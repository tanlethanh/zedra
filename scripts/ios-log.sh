#!/bin/bash
# Unified iOS log tooling — one script, subcommands, no Python.
#
# Usage:
#   ./scripts/ios-log.sh tail [--filter <pattern>] [--with-native] [--select-device] [--simulator]
#   ./scripts/ios-log.sh daemon start [--filter <pattern>] [--select-device] [--tag <name>]
#   ./scripts/ios-log.sh daemon status [--tag <name>]
#   ./scripts/ios-log.sh daemon stop [--tag <name>]
#   ./scripts/ios-log.sh query [--since SPEC] [--until SPEC] [--filter PATTERN] [--tag <name>]
#   ./scripts/ios-log.sh wait <pattern> [--timeout Ns] [--poll-interval Ns] [--tag <name>]
#
# tail — Stream logs live from a USB-connected iOS device or running
#   simulator. Device preference is saved in /tmp/zedra-ios-device (one
#   global target per repo checkout, shared with run-ios.sh/devtool.sh/this
#   script's own `daemon` subcommand). Physical device requires
#   libimobiledevice (brew install libimobiledevice) + USB pairing.
#
# daemon — Background capture so a time range can be queried after the
#   fact, not just tailed live. `devicectl`/`simctl` only stream live —
#   there's no way to ask either for "logs from 10 minutes ago". This wraps
#   whichever applies (device vs simulator) in a background process that
#   keeps a rotating on-disk capture. Both platforms capture the same way,
#   for the same reason: relaunch the app with its console attached
#   (`devicectl device process launch --console` / `simctl launch
#   --console`, Apple's own tools, no sudo) and read raw stdout/stderr,
#   rather than the unified-logging stream (idevicesyslog / `simctl ...
#   log stream`). Unified logging redacts third-party app messages as
#   `<private>` unless marked `%{public}`, and even then a compact-format
#   log entry can fail to locally decode without the binary's dSYM
#   (`<decode: missing data>`) — confirmed on *both* platforms. Console-
#   attached raw stdio sidesteps that decoding entirely; Zedra's iOS logger
#   writes to stderr for exactly this reason (see
#   crates/zedra/src/ios/logger.rs). The tradeoff, same on both platforms:
#   `daemon start` (re)launches the app fresh — neither tool has an "attach
#   console to an already-running process" mode without LLDB.
#   One daemon per repo checkout by default (fixed path under /tmp, mirroring
#   scripts/perf-debug.sh's Android capture). Pass --tag to run more than one
#   concurrently (rare). Query with this script's `query` subcommand — do
#   not grep the directory directly, since the active file rotates.
#   Env: ZEDRA_LOG_MAX_BYTES (rotate threshold, default 20MB),
#        ZEDRA_LOG_KEEP_FILES (rotated files retained, default 4).
#
# query — Search the daemon's capture across any time range + filter,
#   across the active capture.log plus any rotated capture.log.1..N.
#   SPEC: "now" | "<N>s|m|h|d" (relative, e.g. "10m") | "HH:MM[:SS]" (today)
#   | "YYYY-MM-DD HH:MM[:SS]" (absolute). Default --since is unbounded,
#   default --until is "now".
#
# wait — Poll `query` every --poll-interval seconds (default 0.5) until
#   <pattern> matches a line logged since this call started, or --timeout
#   (default 10s) elapses. Removes the sleep-N-then-query guessing game
#   from a debug loop. Exit 0 and print the first matching line if found;
#   exit 1 (message on stderr) if it never appears — itself a finding
#   ("expected log line never fired"), not a script error.
#
# --tag (daemon/query/wait) must match whatever --tag `daemon start` was
# given (only needed for more than one concurrent capture); omit for the
# default daemon.

set -euo pipefail

cd "$(dirname "$0")/.."

PREF_FILE="/tmp/zedra-ios-device"
# "devtool" catches gpui_devtool's own log lines (e.g. "devtool: token: ...",
# the physical-device fallback devtool.sh's read_token() greps for since a
# device's /tmp isn't host-visible) — neither "Zedra" nor "zedra" appears in
# them, so without this a fresh default-filter capture silently never sees
# the token line at all, producing a persistent 401 with no obvious cause.
DEFAULT_FILTER='Zedra|zedra|devtool|panic|PANIC|crash|CRASH|fault|error'
# Console-attaching (either platform) also echoes each NSLog call's raw
# format-string template (a harmless duplicate of the real line our stderr
# sink already printed) — drop it so the capture isn't doubled.
NOISE_FILTER='^[0-9-]+ [0-9:.]+ Zedra\[[0-9:]+\] \{public\}s$'
BUNDLE_ID_DEBUG="dev.zedra.app.debug"
MAX_BYTES="${ZEDRA_LOG_MAX_BYTES:-20971520}"
KEEP_FILES="${ZEDRA_LOG_KEEP_FILES:-4}"

find_tool() {
    local name="$1"
    if command -v "$name" >/dev/null 2>&1; then
        command -v "$name"
    elif [[ -x "/opt/homebrew/bin/$name" ]]; then
        echo "/opt/homebrew/bin/$name"
    elif [[ -x "/usr/local/bin/$name" ]]; then
        echo "/usr/local/bin/$name"
    else
        return 1
    fi
}

# ---------------------------------------------------------------------------
# tail — device selection + live stream
# ---------------------------------------------------------------------------

write_saved_device_pref() {
    printf '%s|%s|%s\n' "$1" "$2" "$3" > "$PREF_FILE"
}

read_saved_device_pref() {
    local first="" second="" third=""
    IFS='|' read -r first second third < "$PREF_FILE" || return 1
    if [[ "$first" == "dev" || "$first" == "sim" ]]; then
        DEVICE_TYPE="$first"
        DEVICE_UDID="${second:-}"
        DEVICE_NAME="${third:-}"
    elif [[ -n "$first" && -n "$second" ]]; then
        # Older run-ios.sh builds saved physical devices as "udid|name".
        DEVICE_TYPE="dev"
        DEVICE_UDID="$first"
        DEVICE_NAME="$second"
        write_saved_device_pref "$DEVICE_TYPE" "$DEVICE_UDID" "$DEVICE_NAME"
    else
        return 1
    fi
    [[ -n "$DEVICE_UDID" ]]
}

visible_physical_device_lines() {
    local idevice_id="$1"
    local udids="" xctrace_lines="" udid="" line=""
    udids=$("$idevice_id" -l 2>/dev/null || true)
    [[ -n "$udids" ]] || return 1
    xctrace_lines=$(xcrun xctrace list devices 2>&1 || true)
    while IFS= read -r udid; do
        [[ -n "$udid" ]] || continue
        line=$(printf '%s\n' "$xctrace_lines" | grep -F "($udid)" | head -1 || true)
        if [[ -n "$line" ]]; then
            printf '%s\n' "$line"
        else
            printf 'iOS Device (unknown) (%s)\n' "$udid"
        fi
    done <<< "$udids"
}

# Populates DEVICE_TYPE/DEVICE_UDID/DEVICE_NAME. Full interactive selection
# (simulator or physical), unlike daemon's lighter resolve_target below —
# `tail` is meant for a human at a terminal.
resolve_tail_device() {
    local select_device="$1" want_simulator="$2"
    DEVICE_UDID="" DEVICE_NAME="" DEVICE_TYPE=""

    if [[ "$select_device" == false && -f "$PREF_FILE" ]] && read_saved_device_pref; then
        if [[ "$want_simulator" == true && "$DEVICE_TYPE" != "sim" ]] || \
           [[ "$want_simulator" == false && "$DEVICE_TYPE" == "sim" ]]; then
            DEVICE_UDID="" DEVICE_NAME="" DEVICE_TYPE=""
        else
            echo "==> Using saved device: $DEVICE_NAME ($DEVICE_UDID)"
            return
        fi
    fi

    if [[ "$want_simulator" == true ]]; then
        echo "==> Enumerating booted simulators..."
        local sim_lines
        sim_lines=$(xcrun simctl list devices booted 2>/dev/null \
            | grep -E '\(Booted\)' \
            | sed 's/^[[:space:]]*//')
        if [[ -z "$sim_lines" ]]; then
            echo "Error: No booted simulators found. Launch a simulator first (Xcode -> Simulator)." >&2
            exit 1
        fi
        echo ""
        echo "Booted simulators:"
        local i=1
        while IFS= read -r line; do
            echo "  $i. $line"
            i=$((i + 1))
        done <<< "$sim_lines"
        echo ""
        local count choice
        count=$(echo "$sim_lines" | wc -l | tr -d ' ')
        if [[ "$count" -eq 1 ]]; then
            choice=1
            echo "==> Auto-selecting only simulator."
        else
            read -rp "Select simulator [1-$count]: " choice
        fi
        local selected
        selected=$(echo "$sim_lines" | sed -n "${choice}p")
        if [[ -z "$selected" ]]; then
            echo "Error: Invalid selection." >&2
            exit 1
        fi
        DEVICE_UDID=$(echo "$selected" | grep -oE '[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}' | head -1)
        DEVICE_NAME=$(echo "$selected" | sed 's/ ([^ ]*) (Booted)$//' | sed 's/ (Booted)$//')
        DEVICE_TYPE="sim"
        if [[ -z "$DEVICE_UDID" ]]; then
            echo "Error: Could not parse UDID from: $selected" >&2
            exit 1
        fi
        write_saved_device_pref "sim" "$DEVICE_UDID" "$DEVICE_NAME"
        echo "==> Selected: $DEVICE_NAME ($DEVICE_UDID)"
    else
        local idevice_id
        idevice_id=$(find_tool idevice_id || true)
        if [[ -z "$idevice_id" ]]; then
            echo "Error: idevice_id not found. Install with: brew install libimobiledevice" >&2
            exit 1
        fi
        echo "==> Enumerating connected devices..."
        local device_lines
        device_lines=$(visible_physical_device_lines "$idevice_id" || true)
        if [[ -z "$device_lines" ]]; then
            echo "Error: No USB-visible iOS devices found. Connect and trust a device, or use --simulator." >&2
            exit 1
        fi
        echo ""
        echo "Connected iOS devices:"
        local i=1
        while IFS= read -r line; do
            echo "  $i. $line"
            i=$((i + 1))
        done <<< "$device_lines"
        echo ""
        local count choice
        count=$(echo "$device_lines" | wc -l | tr -d ' ')
        if [[ "$count" -eq 1 ]]; then
            choice=1
            echo "==> Auto-selecting only device."
        else
            read -rp "Select device [1-$count]: " choice
        fi
        local selected
        selected=$(echo "$device_lines" | sed -n "${choice}p")
        if [[ -z "$selected" ]]; then
            echo "Error: Invalid selection." >&2
            exit 1
        fi
        DEVICE_UDID=$(echo "$selected" | grep -oE '\([A-F0-9a-f-]{25,}\)' | tail -1 | tr -d '()')
        DEVICE_NAME=$(echo "$selected" | sed 's/ ([^)]*) ([^)]*)$//' | sed 's/ ([^)]*)$//')
        DEVICE_TYPE="dev"
        if [[ -z "$DEVICE_UDID" ]]; then
            echo "Error: Could not parse UDID from: $selected" >&2
            exit 1
        fi
        write_saved_device_pref "dev" "$DEVICE_UDID" "$DEVICE_NAME"
        echo "==> Selected: $DEVICE_NAME ($DEVICE_UDID)"
    fi
}

cmd_tail() {
    local filter="" with_native=false select_device=false simulator=false
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --filter) filter="$2"; shift 2 ;;
            --with-native) with_native=true; shift ;;
            --select-device) select_device=true; shift ;;
            --simulator) simulator=true; shift ;;
            *) echo "Error: unknown flag '$1'." >&2; exit 1 ;;
        esac
    done

    local idevice_id idevicesyslog
    idevice_id=$(find_tool idevice_id || true)
    idevicesyslog=$(find_tool idevicesyslog || true)

    resolve_tail_device "$select_device" "$simulator"

    local grep_pattern='\[I |\[W |\[E |\[D | INFO | WARN | ERROR | DEBUG | TRACE |panic|PANIC|crash|CRASH|NSException|Terminating'
    if [[ -n "$filter" ]]; then
        grep_pattern="$grep_pattern|$filter"
    fi
    if [[ "$with_native" == true ]]; then
        echo "==> Including native logs"
        grep_pattern="$grep_pattern|<I|<W|<E|<D"
    fi

    echo "==> Streaming logs from $DEVICE_NAME (Ctrl-C to stop)..."
    echo "    Level prefix: [I]=Info [W]=Warn [E]=Error [D]=Debug [T]=Trace"
    echo ""

    if [[ "$DEVICE_TYPE" == "sim" ]]; then
        local predicate='process == "Zedra"'
        xcrun simctl spawn "$DEVICE_UDID" log stream \
            --predicate "$predicate" \
            --level debug \
            --color none \
            2>/dev/null \
            | grep -E "$grep_pattern" --line-buffered --color=auto
    else
        if [[ -z "$idevicesyslog" ]]; then
            echo "Error: idevicesyslog not found. Install with: brew install libimobiledevice" >&2
            exit 1
        fi
        if [[ -z "$idevice_id" ]]; then
            echo "Error: idevice_id not found. Install with: brew install libimobiledevice" >&2
            exit 1
        fi
        if ! "$idevice_id" -l 2>/dev/null | grep -F -q "$DEVICE_UDID"; then
            echo "Error: Device $DEVICE_UDID not visible to idevicesyslog." >&2
            echo "idevicesyslog requires a USB-connected (paired) device." >&2
            exit 1
        fi
        # idevicesyslog block-buffers when stdout is a pipe, which can make
        # filtered logs appear stuck. `script -q /dev/null` gives it a real
        # pty (discarding the session-transcript copy `script` normally
        # writes) so it stays line-buffered — same fix a PTY-fork would give,
        # no Python needed.
        script -q /dev/null "$idevicesyslog" --no-colors -u "$DEVICE_UDID" -p Zedra 2>/dev/null \
            | grep -E "$grep_pattern" --line-buffered --color=auto
    fi
}

# ---------------------------------------------------------------------------
# daemon — background capture
# ---------------------------------------------------------------------------

TAG=""
# Extract --tag up front (shared by daemon/query/wait) so LOG_DIR is fixed
# before dispatch; the per-command arg loops below still see --tag and
# simply skip it (already consumed here).
args=("$@")
i=0
while [ $i -lt ${#args[@]} ]; do
    if [ "${args[$i]}" = "--tag" ]; then
        i=$((i + 1))
        TAG="${args[$i]:-}"
    fi
    i=$((i + 1))
done
LOG_DIR="/tmp/zedra-ios-log-daemon${TAG:+-$TAG}"
PID_FILE="$LOG_DIR/daemon.pid"
LOG_FILE="$LOG_DIR/capture.log"

# Resolves to "sim <udid> <name>" or "dev <udid> <name>" on one line. Lighter
# than resolve_tail_device: trusts the pref file (already established by
# run-ios.sh) or falls back to physical-device enumeration only — no
# simulator prompt, since `daemon` is meant to run unattended in a debug
# loop, not block on interactive input.
resolve_daemon_target() {
    local select_device="$1"
    if [ "$select_device" = false ] && [ -f "$PREF_FILE" ]; then
        local kind="" id="" name=""
        IFS='|' read -r kind id name < "$PREF_FILE" || true
        if { [ "$kind" = "dev" ] || [ "$kind" = "sim" ]; } && [ -n "$id" ]; then
            echo "$kind $id $name"
            return
        fi
        if [ -n "$kind" ] && [ -z "$id" ]; then
            echo "dev $kind"
            return
        fi
    fi

    local devices
    devices="$(idevice_id -l 2>/dev/null || true)"
    if [ -z "$devices" ]; then
        echo "Error: no saved device preference and no connected physical device (idevice_id -l)." >&2
        echo "Run ./scripts/run-ios.sh sim|device --devtool first, or pass --select-device with a device connected." >&2
        exit 1
    fi

    local count udid
    count="$(echo "$devices" | wc -l | tr -d ' ')"
    if [ "$count" -eq 1 ]; then
        udid="$devices"
    else
        echo "Connected iOS devices:" >&2
        echo "$devices" | nl -ba >&2
        read -rp "Select device [1-$count]: " choice
        udid="$(echo "$devices" | sed -n "${choice}p")"
    fi
    echo "dev $udid"
}

validate_dev_udid() {
    local udid="$1"
    if ! idevice_id -l 2>/dev/null | grep -qx "$udid"; then
        echo "Error: device $udid is not currently connected (checked via idevice_id -l)." >&2
        exit 1
    fi
}

validate_sim_udid() {
    local udid="$1"
    if ! xcrun simctl list devices booted 2>/dev/null | grep -q "$udid"; then
        echo "Error: simulator $udid is not currently booted." >&2
        exit 1
    fi
}

# Line-buffered rotating sink, reading stdin and appending to $1, rotating
# when it exceeds $2 bytes (keeping $3 rotated files). Pure bash/coreutils —
# no Python. Single-writer, so rotation never races a concurrent reader:
# renames the file out from under itself and starts a fresh one at the
# original path.
rotate_sink() {
    local log_file="$1" max_bytes="$2" keep_files="$3"
    local size=0
    if [ -f "$log_file" ]; then
        size=$(wc -c < "$log_file" | tr -d ' ')
    fi
    while IFS= read -r line || [ -n "$line" ]; do
        printf '%s\n' "$line" >> "$log_file"
        size=$((size + ${#line} + 1))
        if [ "$size" -ge "$max_bytes" ]; then
            local i
            for ((i = keep_files; i >= 1; i--)); do
                local src="$log_file.$i" dst="$log_file.$((i + 1))"
                if [ -f "$src" ]; then
                    if [ "$i" -eq "$keep_files" ]; then
                        rm -f "$src"
                    else
                        mv -f "$src" "$dst"
                    fi
                fi
            done
            mv -f "$log_file" "$log_file.1"
            size=0
        fi
    done
}

cmd_daemon_start() {
    local filter="$DEFAULT_FILTER" select_device=false
    while [ $# -gt 0 ]; do
        case "$1" in
            --filter) filter="$2"; shift 2 ;;
            --select-device) select_device=true; shift ;;
            --tag) shift 2 ;;
            *) echo "Error: unknown flag '$1'." >&2; exit 1 ;;
        esac
    done

    mkdir -p "$LOG_DIR"

    if [ -f "$PID_FILE" ] && kill -0 "-$(cat "$PID_FILE")" 2>/dev/null; then
        echo "==> Daemon already running (pgid $(cat "$PID_FILE")), log: $LOG_FILE"
        return
    fi

    local kind id name
    read -r kind id name <<< "$(resolve_daemon_target "$select_device")"

    # `set -m` puts the whole backgrounded pipeline into its own process
    # group, so `stop` can tear down every stage at once with
    # `kill -TERM -<pgid>`. Without this, killing only the tail leaves the
    # capture command orphaned until it happens to write into the closed
    # pipe and get SIGPIPE — which may never happen if the filter rarely
    # matches.
    set -m
    if [ "$kind" = "sim" ]; then
        validate_sim_udid "$id"
        echo "==> Capturing from simulator $id ${name:+($name) }via simctl console (relaunches the app), filter: $filter"
        xcrun simctl launch --console --terminate-running-process "$id" "$BUNDLE_ID_DEBUG" 2>&1 \
            | grep --line-buffered -Ev "$NOISE_FILTER" \
            | grep --line-buffered -E "$filter" \
            | rotate_sink "$LOG_FILE" "$MAX_BYTES" "$KEEP_FILES" &
    else
        validate_dev_udid "$id"
        echo "==> Capturing from device $id ${name:+($name) }via devicectl console (relaunches the app), filter: $filter"
        xcrun devicectl device process launch --console --device "$id" "$BUNDLE_ID_DEBUG" 2>&1 \
            | grep --line-buffered -Ev "$NOISE_FILTER" \
            | grep --line-buffered -E "$filter" \
            | rotate_sink "$LOG_FILE" "$MAX_BYTES" "$KEEP_FILES" &
    fi
    local pipeline_pid=$!
    set +m
    local pgid
    pgid="$(ps -o pgid= -p "$pipeline_pid" | tr -d ' ')"
    echo "$pgid" > "$PID_FILE"
    disown

    sleep 0.5
    if ! kill -0 "-$pgid" 2>/dev/null; then
        echo "Error: daemon failed to start. Check 'xcrun devicectl' (device) or 'xcrun simctl' (simulator) work standalone." >&2
        exit 1
    fi
    echo "==> Daemon started (pgid $pgid), log: $LOG_FILE"
}

cmd_daemon_status() {
    if [ ! -f "$PID_FILE" ] || ! kill -0 "-$(cat "$PID_FILE")" 2>/dev/null; then
        echo "not running"
        return 1
    fi
    local size lines newest
    size="$(du -h "$LOG_FILE" 2>/dev/null | cut -f1)"
    lines="$(wc -l < "$LOG_FILE" 2>/dev/null | tr -d ' ')"
    newest="$(tail -1 "$LOG_FILE" 2>/dev/null | cut -c1-15)"
    echo "running (pgid $(cat "$PID_FILE"))"
    echo "log: $LOG_FILE ($size, $lines lines)"
    echo "newest line starts: $newest"
}

cmd_daemon_stop() {
    if [ ! -f "$PID_FILE" ]; then
        echo "not running"
        return
    fi
    local pgid
    pgid="$(cat "$PID_FILE")"
    if kill -0 "-$pgid" 2>/dev/null; then
        kill -TERM "-$pgid" 2>/dev/null || true
        echo "==> Stopped daemon (pgid $pgid)"
    else
        echo "==> Daemon was not running"
    fi
    rm -f "$PID_FILE"
}

cmd_daemon() {
    local sub="${1:-}"
    shift || true
    case "$sub" in
        start)  cmd_daemon_start "$@" ;;
        status) cmd_daemon_status "$@" ;;
        stop)   cmd_daemon_stop "$@" ;;
        *)
            echo "Error: unknown daemon subcommand '$sub'. Expected start, status, or stop." >&2
            exit 1
            ;;
    esac
}

# ---------------------------------------------------------------------------
# query — historical search across the daemon's capture
# ---------------------------------------------------------------------------

# idevicesyslog lines start with classic syslog timestamps, e.g.
# "Mar  4 17:42:55". Prints the epoch seconds for a matching line, or
# nothing if it doesn't start with one (a wrapped continuation line).
_CACHED_DAY_KEY=""
_CACHED_DAY_EPOCH=""

# Epoch for midnight on a given "Mon Day" — memoized (single-slot: a
# capture file's lines are chronological, so consecutive lines almost
# always share the same day; a plain scalar cache is enough and stays
# compatible with macOS's system /bin/bash 3.2, which has no associative
# arrays). Spawning `date` per *line* (as a straight port of the original
# per-line datetime parse would) turned a query over a large capture into a
# 30s+ stall; caching by day cuts that to one `date` call per unique day.
# Sets $_DAY_EPOCH_RESULT rather than echoing — even a cache-hit path still
# forks a subshell under `$(...)` capture, and that fork-per-line cost alone
# (not just the avoided `date` calls) was still enough to stall a large
# query; a plain global write has no subshell at all.
day_epoch() {
    local mon="$1" day="$2" reference_year="$3"
    local key="$mon $day"
    if [ "$key" != "$_CACHED_DAY_KEY" ]; then
        _CACHED_DAY_KEY="$key"
        # BSD `date -j` defaults any field missing from the format string to
        # *now*'s value, not zero — must spell out 00:00:00 explicitly or
        # this silently returns "today at the current time" instead of
        # midnight, corrupting every downstream comparison.
        _CACHED_DAY_EPOCH=$(date -j -f "%Y %b %d %H:%M:%S" "$reference_year $mon $day 00:00:00" "+%s" 2>/dev/null)
    fi
    _DAY_EPOCH_RESULT="$_CACHED_DAY_EPOCH"
}

# Sets $_LINE_EPOCH_RESULT (empty if the line doesn't start with a syslog
# timestamp) — same no-subshell reasoning as `day_epoch`.
line_epoch() {
    local line="$1" reference_year="$2"
    _LINE_EPOCH_RESULT=""
    if [[ "$line" =~ ^([A-Z][a-z]{2})[[:space:]]+([0-9]{1,2})[[:space:]]([0-9]{2}):([0-9]{2}):([0-9]{2}) ]]; then
        local mon="${BASH_REMATCH[1]}" day="${BASH_REMATCH[2]}"
        local hh="${BASH_REMATCH[3]}" mm="${BASH_REMATCH[4]}" ss="${BASH_REMATCH[5]}"
        day_epoch "$mon" "$day" "$reference_year"
        [ -n "$_DAY_EPOCH_RESULT" ] || return
        _LINE_EPOCH_RESULT=$((_DAY_EPOCH_RESULT + 10#$hh * 3600 + 10#$mm * 60 + 10#$ss))
    fi
}

# Converts a --since/--until SPEC into epoch seconds.
parse_time_spec() {
    local spec="$1" now="$2"
    if [[ "$spec" == "now" ]]; then
        echo "$now"
        return
    fi
    if [[ "$spec" =~ ^([0-9]+)([smhd])$ ]]; then
        local amount="${BASH_REMATCH[1]}" unit="${BASH_REMATCH[2]}" seconds
        case "$unit" in
            s) seconds=1 ;;
            m) seconds=60 ;;
            h) seconds=3600 ;;
            d) seconds=86400 ;;
        esac
        echo $((now - amount * seconds))
        return
    fi
    # BSD `date -j` defaults any field missing from the format string to
    # *now*'s value, not zero (e.g. omitted seconds silently becomes "this
    # instant's seconds", not :00) — normalize to always include seconds
    # and always parse with the same fully-specified format, rather than
    # branching on whether the spec included them.
    if [[ "$spec" =~ ^([0-9]{4}-[0-9]{2}-[0-9]{2})[\ T]([0-9]{2}:[0-9]{2})(:([0-9]{2}))?$ ]]; then
        local date_part="${BASH_REMATCH[1]}" time_part="${BASH_REMATCH[2]}"
        local secs="${BASH_REMATCH[4]:-00}"
        date -j -f "%Y-%m-%d %H:%M:%S" "$date_part $time_part:$secs" "+%s" 2>/dev/null && return
        echo "Error: could not parse time spec '$spec'." >&2
        exit 1
    fi
    if [[ "$spec" =~ ^([0-9]{2}:[0-9]{2})(:([0-9]{2}))?$ ]]; then
        local today time_part="${BASH_REMATCH[1]}" secs="${BASH_REMATCH[3]:-00}"
        today="$(date '+%Y-%m-%d')"
        date -j -f "%Y-%m-%d %H:%M:%S" "$today $time_part:$secs" "+%s" 2>/dev/null && return
        echo "Error: could not parse time spec '$spec'." >&2
        exit 1
    fi
    echo "Error: unrecognized time spec '$spec'." >&2
    exit 1
}

capture_files() {
    # Rotated files are named capture.log.1 (most recent rotation) up to
    # capture.log.N (oldest surviving). Sort by that trailing number,
    # descending, so oldest content comes first — matches reading order:
    # oldest rotated file, ..., newest rotated file, then the active file.
    local f
    for f in "$LOG_FILE".[0-9]*; do
        [ -f "$f" ] || continue
        printf '%s %s\n' "${f##*.}" "$f"
    done | sort -rn | cut -d' ' -f2-
    [ -f "$LOG_FILE" ] && echo "$LOG_FILE"
    return 0
}

cmd_query() {
    local since="" until="now" filter=""
    while [ $# -gt 0 ]; do
        case "$1" in
            --since) since="$2"; shift 2 ;;
            --until) until="$2"; shift 2 ;;
            --filter) filter="$2"; shift 2 ;;
            --tag) shift 2 ;;
            *) echo "Error: unknown flag '$1'." >&2; exit 1 ;;
        esac
    done

    local now since_ts until_ts reference_year
    now=$(date +%s)
    until_ts=$(parse_time_spec "$until" "$now")
    since_ts=""
    [ -n "$since" ] && since_ts=$(parse_time_spec "$since" "$now")
    reference_year=$(date '+%Y')

    local files
    files=$(capture_files)
    if [ -z "$files" ]; then
        echo "No capture files found in $LOG_DIR — is the daemon running?" >&2
        exit 1
    fi

    local matched=0 last_ts=""
    while IFS= read -r path; do
        [ -f "$path" ] || continue
        while IFS= read -r line || [ -n "$line" ]; do
            if [[ "$line" =~ $NOISE_FILTER ]]; then
                continue
            fi
            local ts effective_ts
            line_epoch "$line" "$reference_year"
            ts="$_LINE_EPOCH_RESULT"
            if [ -n "$ts" ]; then
                last_ts="$ts"
                effective_ts="$ts"
            else
                effective_ts="$last_ts"
            fi
            if [ -n "$effective_ts" ]; then
                if [ -n "$since_ts" ] && [ "$effective_ts" -lt "$since_ts" ]; then
                    continue
                fi
                if [ "$effective_ts" -gt "$until_ts" ]; then
                    continue
                fi
            fi
            if [ -n "$filter" ] && ! [[ "$line" =~ $filter ]]; then
                continue
            fi
            printf '%s\n' "$line"
            matched=$((matched + 1))
        done < "$path"
    done <<< "$files"

    if [ "$matched" -eq 0 ]; then
        echo "(no matching lines)" >&2
    fi
}

# ---------------------------------------------------------------------------
# wait — poll query until a pattern appears
# ---------------------------------------------------------------------------

cmd_wait() {
    local pattern="" timeout=10 poll_interval=0.5 tag_args=()
    while [ $# -gt 0 ]; do
        case "$1" in
            --timeout) timeout="${2%s}"; shift 2 ;;
            --poll-interval) poll_interval="$2"; shift 2 ;;
            --tag) tag_args=(--tag "$2"); shift 2 ;;
            *)
                if [ -z "$pattern" ]; then
                    pattern="$1"
                else
                    echo "Error: unexpected argument '$1'." >&2
                    exit 1
                fi
                shift
                ;;
        esac
    done
    if [ -z "$pattern" ]; then
        echo "Usage: $0 wait <pattern> [--timeout Ns] [--poll-interval Ns] [--tag <name>]" >&2
        exit 1
    fi

    local start_time deadline
    start_time="$(date '+%H:%M:%S')"
    deadline=$(($(date +%s) + timeout))

    while [ "$(date +%s)" -lt "$deadline" ]; do
        local match
        match="$(cmd_query --since "$start_time" --filter "$pattern" "${tag_args[@]+"${tag_args[@]}"}" 2>/dev/null | head -1)"
        if [ -n "$match" ]; then
            echo "$match"
            return 0
        fi
        sleep "$poll_interval"
    done

    echo "Error: pattern '$pattern' did not appear within ${timeout}s." >&2
    exit 1
}

# ---------------------------------------------------------------------------
# Dispatch
# ---------------------------------------------------------------------------

sub="${1:-}"
shift || true
case "$sub" in
    tail)   cmd_tail "$@" ;;
    daemon) cmd_daemon "$@" ;;
    query)  cmd_query "$@" ;;
    wait)   cmd_wait "$@" ;;
    ""|-h|--help)
        sed -n '2,60p' "$0" | sed 's/^# \{0,1\}//'
        ;;
    *)
        echo "Error: unknown subcommand '$sub'. Expected tail, daemon, query, or wait." >&2
        exit 1
        ;;
esac
