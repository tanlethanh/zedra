/// iOS logger. Every message goes out on two independent, intentionally
/// duplicate channels — deliberate, not something to consolidate to one:
///
/// | Channel | Written by                | Read by                          |
/// |---------|----------------------------|-----------------------------------|
/// | NSLog (unified logging) | `zedra_nslog` FFI → `nslog_bridge.m` | Console.app (has local dSYM, decodes fine) |
/// | stderr (raw stdio)      | `eprintln!` below, timestamped       | `xcrun devicectl device process launch --console` — what `scripts/ios-log.sh daemon` actually uses |
///
/// Neither channel subsumes the other. `idevicesyslog` reads the NSLog
/// channel but can't locally decode a third-party binary's compact log
/// entries without its dSYM (shows `<decode: missing data>` regardless of
/// the `%{public}s` marker in `nslog_bridge.m`) — that's *why* the stderr
/// channel exists, not a redundant afterthought. Console.app has no CLI/
/// automation equivalent, so it stays on NSLog. See docs/DEVTOOL.md.
///
/// Release builds use only this path. `tracing` macros are not subscribed unless
/// the `debug-logs` feature is enabled — matching v0.2.4 behavior and avoiding
/// synchronous NSLog/os_log storms during iroh connect.
use log::{Level, Log, Metadata, Record};
use std::ffi::CString;

#[cfg(feature = "debug-logs")]
use std::io::{self, Write};
#[cfg(feature = "debug-logs")]
use tracing_subscriber::{EnvFilter, fmt::MakeWriter};

unsafe extern "C" {
    fn zedra_nslog(msg: *const std::ffi::c_char);
}

pub struct IosLogger;

impl IosLogger {
    pub fn init(level: log::LevelFilter) {
        match log::set_boxed_logger(Box::new(IosLogger)) {
            Ok(()) => log::set_max_level(level),
            Err(_) => log::set_max_level(level),
        }

        #[cfg(feature = "debug-logs")]
        install_tracing_subscriber(level);
    }
}

#[cfg(feature = "debug-logs")]
fn install_tracing_subscriber(level: log::LevelFilter) {
    if level == log::LevelFilter::Off {
        return;
    }

    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(ios_tracing_filter(level))
        .with_level(true)
        .with_target(true)
        .without_time()
        .with_writer(IosTracingMakeWriter)
        .finish();

    let _ = tracing::subscriber::set_global_default(subscriber);
}

#[cfg(feature = "debug-logs")]
fn ios_tracing_filter(level: log::LevelFilter) -> EnvFilter {
    let level = match level {
        log::LevelFilter::Off => "off",
        log::LevelFilter::Error => "error",
        log::LevelFilter::Warn => "warn",
        log::LevelFilter::Info => "info",
        log::LevelFilter::Debug => "debug",
        log::LevelFilter::Trace => "trace",
    };

    EnvFilter::new(format!(
        "{level},iroh=warn,iroh_quinn=warn,iroh_quinn_proto=warn,\
         iroh_relay=warn,iroh_net_report=warn,quinn=warn"
    ))
}

#[cfg(feature = "debug-logs")]
struct IosTracingMakeWriter;

#[cfg(feature = "debug-logs")]
impl<'a> MakeWriter<'a> for IosTracingMakeWriter {
    type Writer = IosTracingWriter;

    fn make_writer(&'a self) -> Self::Writer {
        IosTracingWriter { buffer: Vec::new() }
    }
}

#[cfg(feature = "debug-logs")]
struct IosTracingWriter {
    buffer: Vec<u8>,
}

#[cfg(feature = "debug-logs")]
impl Write for IosTracingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(feature = "debug-logs")]
impl Drop for IosTracingWriter {
    fn drop(&mut self) {
        if self.buffer.is_empty() {
            return;
        }

        let message = String::from_utf8_lossy(&self.buffer);
        write_ios_log(message.trim_end());
    }
}

fn write_ios_log(message: &str) {
    // Classic syslog timestamp ("Jul  6 17:42:55"), matching idevicesyslog's own
    // format — devicectl's --console capture has no per-line timestamp otherwise,
    // which would break ios-log.sh's `query` time-range filtering.
    let timestamp = chrono::Local::now().format("%b %e %H:%M:%S");
    eprintln!("{timestamp} {message}");
    if let Ok(c) = CString::new(message) {
        unsafe { zedra_nslog(c.as_ptr()) };
    }
}

impl Log for IosLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        let target = metadata.target();
        if matches!(target, "tracing::span" | "tracing::span::active") {
            return false;
        }
        if !cfg!(feature = "debug-logs")
            && metadata.level() > Level::Warn
            && (target.starts_with("iroh") || target.starts_with("quinn"))
        {
            return false;
        }
        true
    }

    fn log(&self, record: &Record) {
        let level = match record.level() {
            Level::Error => 'E',
            Level::Warn => 'W',
            Level::Info => 'I',
            Level::Debug => 'D',
            Level::Trace => 'T',
        };
        let msg = format!("[{} {}] {}", level, record.target(), record.args());
        write_ios_log(&msg);
    }

    fn flush(&self) {}
}
