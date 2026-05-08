/// iOS logger that routes Rust logs into the device log stream.
///
/// `log` crate lines are formatted as `[I dev.zedra.app::module] message`.
/// `tracing` lines use the compact subscriber format.
use log::{Level, Log, Metadata, Record};
use std::ffi::CString;
use std::io::{self, Write};
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

        install_tracing_logger(level);
    }
}

fn install_tracing_logger(level: log::LevelFilter) {
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

fn ios_tracing_filter(level: log::LevelFilter) -> EnvFilter {
    let level = match level {
        log::LevelFilter::Off => "off",
        log::LevelFilter::Error => "error",
        log::LevelFilter::Warn => "warn",
        log::LevelFilter::Info => "info",
        log::LevelFilter::Debug => "debug",
        log::LevelFilter::Trace => "trace",
    };

    if cfg!(feature = "debug-logs") {
        EnvFilter::new(level)
    } else {
        EnvFilter::new(format!(
            "{level},iroh=warn,iroh_quinn=warn,iroh_relay=warn,quinn=warn"
        ))
    }
}

struct IosTracingMakeWriter;

impl<'a> MakeWriter<'a> for IosTracingMakeWriter {
    type Writer = IosTracingWriter;

    fn make_writer(&'a self) -> Self::Writer {
        IosTracingWriter { buffer: Vec::new() }
    }
}

struct IosTracingWriter {
    buffer: Vec<u8>,
}

impl Write for IosTracingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

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
        // Without debug-logs feature, suppress noisy iroh/quinn below warn
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
