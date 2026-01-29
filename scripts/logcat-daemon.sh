#!/bin/bash
# Background logcat monitor with smart filtering
# Usage: ./scripts/logcat-daemon.sh {start|stop|tail|restart}

LOG_DIR="/tmp/zedra-logs"
mkdir -p "$LOG_DIR"

LOG_FILE="$LOG_DIR/zedra-logcat-$(date +%Y%m%d-%H%M%S).log"
PID_FILE="/tmp/zedra-logcat.pid"
CURRENT_LOG_LINK="$LOG_DIR/current.log"

# Color codes for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Start daemon
start() {
  if [ -f "$PID_FILE" ]; then
    echo -e "${YELLOW}Daemon already running (PID: $(cat $PID_FILE))${NC}"
    exit 1
  fi

  # Check if device is connected
  if ! adb devices | grep -q "device$"; then
    echo -e "${RED}Error: No Android device connected${NC}"
    exit 1
  fi

  echo -e "${GREEN}Starting logcat daemon...${NC}"

  # Clear existing logs
  adb logcat -c

  # Start filtered logcat in background
  # Filter for: zedra logs, FATAL errors, VK_ERROR, JNI errors, panics
  adb logcat -v threadtime | grep --line-buffered -E "zedra|FATAL|VK_ERROR|JNI ERROR|panicked at|Surface" > "$LOG_FILE" &

  echo $! > "$PID_FILE"
  ln -sf "$LOG_FILE" "$CURRENT_LOG_LINK"

  echo -e "${GREEN}✓ Logcat daemon started${NC}"
  echo "  PID: $(cat $PID_FILE)"
  echo "  Log: $LOG_FILE"
  echo "  Link: $CURRENT_LOG_LINK"
  echo ""
  echo "Use './scripts/logcat-daemon.sh tail' to view logs"
}

# Stop daemon
stop() {
  if [ ! -f "$PID_FILE" ]; then
    echo -e "${YELLOW}Daemon not running${NC}"
    exit 0
  fi

  PID=$(cat "$PID_FILE")
  echo -e "${GREEN}Stopping logcat daemon (PID: $PID)...${NC}"

  kill "$PID" 2>/dev/null
  rm "$PID_FILE"

  echo -e "${GREEN}✓ Daemon stopped${NC}"
}

# Show recent logs
tail_logs() {
  if [ ! -f "$CURRENT_LOG_LINK" ]; then
    echo -e "${YELLOW}No active log file. Start daemon first.${NC}"
    exit 1
  fi

  echo -e "${GREEN}Monitoring logs (Ctrl+C to stop)...${NC}"
  echo ""
  tail -f "$CURRENT_LOG_LINK"
}

# Show status
status() {
  if [ -f "$PID_FILE" ]; then
    PID=$(cat "$PID_FILE")
    if ps -p "$PID" > /dev/null; then
      echo -e "${GREEN}✓ Daemon running (PID: $PID)${NC}"
      echo "  Log: $(readlink $CURRENT_LOG_LINK)"
      echo "  Size: $(du -h $(readlink $CURRENT_LOG_LINK) | cut -f1)"
    else
      echo -e "${RED}✗ Daemon not running (stale PID file)${NC}"
      rm "$PID_FILE"
    fi
  else
    echo -e "${YELLOW}Daemon not running${NC}"
  fi
}

# Main command dispatcher
case "$1" in
  start)
    start
    ;;
  stop)
    stop
    ;;
  tail)
    tail_logs
    ;;
  restart)
    stop
    sleep 1
    start
    ;;
  status)
    status
    ;;
  *)
    echo "Usage: $0 {start|stop|tail|restart|status}"
    echo ""
    echo "Commands:"
    echo "  start   - Start background logcat monitoring"
    echo "  stop    - Stop background monitoring"
    echo "  tail    - View live logs from current session"
    echo "  restart - Restart the daemon"
    echo "  status  - Check daemon status"
    exit 1
    ;;
esac
