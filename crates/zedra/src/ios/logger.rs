/// iOS logger that routes through NSLog so output appears in idevicesyslog.
///
/// `os_log` (used by the `oslog` crate) goes to Apple's unified logging
/// system at OS_LOG_TYPE_INFO level, which idevicesyslog (libimobiledevice)
/// does not capture. NSLog routes through ASL (Apple System Log), which
/// idevicesyslog reliably surfaces over USB.
///
/// Log lines are formatted as:
///   `[I dev.zedra.app::module] message`
/// so the `grep -E 'Zedra|zedra'` filter in the ios-log skill matches them.
use log::{Level, Log, Metadata, Record};
use std::ffi::CString;

unsafe extern "C" {
    fn zedra_nslog(msg: *const std::ffi::c_char);
}

pub struct IosLogger;

impl IosLogger {
    pub fn init(level: log::LevelFilter) {
        log::set_boxed_logger(Box::new(IosLogger))
            .map(|()| log::set_max_level(level))
            .ok();
    }
}

impl Log for IosLogger {
    fn enabled(&self, _: &Metadata) -> bool {
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
        if let Ok(c) = CString::new(msg) {
            unsafe { zedra_nslog(c.as_ptr()) };
        }
    }

    fn flush(&self) {}
}
