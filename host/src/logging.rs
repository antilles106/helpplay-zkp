//! Log output through a replaceable handler.
//!
//! All messages of the CLI go through the global handler, which writes to stdout unless
//! another one is installed with [`set_handler`]. Use the `log_debug!`, `log_info!`,
//! `log_warn!` and `log_error!` macros to write messages.
//!
//! Messages below the global level (see [`set_level`], default `Info`) are dropped before
//! they reach the handler, so debug messages are hidden unless the level is lowered.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// Severity of a message, from least to most severe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl std::str::FromStr for Level {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "debug" => Ok(Level::Debug),
            "info" => Ok(Level::Info),
            "warn" | "warning" => Ok(Level::Warn),
            "error" => Ok(Level::Error),
            _ => Err(format!("unknown log level {s} (expected debug, info, warn or error)")),
        }
    }
}

/// Formats a message as one line of output (without the trailing newline).
/// Info messages are written as they are; the other levels get a prefix.
pub fn format_line(level: Level, msg: &str) -> String {
    match level {
        Level::Debug => format!("Debug: {msg}"),
        Level::Info => msg.to_string(),
        Level::Warn => format!("Warning: {msg}"),
        Level::Error => format!("Error: {msg}"),
    }
}

/// Receives log messages.
pub trait LogHandler: Send + Sync {
    fn log(&self, level: Level, msg: &str);
}

/// Writes to standard output (the default).
pub struct StdoutHandler;

impl LogHandler for StdoutHandler {
    fn log(&self, level: Level, msg: &str) {
        println!("{}", format_line(level, msg));
    }
}

/// Writes to standard error.
pub struct StderrHandler;

impl LogHandler for StderrHandler {
    fn log(&self, level: Level, msg: &str) {
        eprintln!("{}", format_line(level, msg));
    }
}

/// Discards all messages.
pub struct NullHandler;

impl LogHandler for NullHandler {
    fn log(&self, _level: Level, _msg: &str) {}
}

/// Appends to a file.
pub struct FileHandler {
    file: Mutex<File>,
}

impl FileHandler {
    /// Opens `path` for appending, creating it if needed.
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self { file: Mutex::new(file) })
    }
}

impl LogHandler for FileHandler {
    fn log(&self, level: Level, msg: &str) {
        if let Ok(mut f) = self.file.lock() {
            // A failed write must not stop the program; there is nowhere left to report it.
            let _ = writeln!(f, "{}", format_line(level, msg));
        }
    }
}

/// Keeps messages in memory (for tests, or for programs that embed this library).
#[derive(Clone, Default)]
pub struct MemoryHandler {
    lines: Arc<Mutex<Vec<(Level, String)>>>,
}

impl MemoryHandler {
    pub fn new() -> Self {
        Self::default()
    }

    /// The messages received so far.
    pub fn lines(&self) -> Vec<(Level, String)> {
        self.lines.lock().map(|l| l.clone()).unwrap_or_default()
    }
}

impl LogHandler for MemoryHandler {
    fn log(&self, level: Level, msg: &str) {
        if let Ok(mut l) = self.lines.lock() {
            l.push((level, msg.to_string()));
        }
    }
}

/// Builds a handler from a destination name: `stdout`, `stderr`, `none`, or a file path.
pub fn handler_for(dest: &str) -> std::io::Result<Box<dyn LogHandler>> {
    Ok(match dest {
        "stdout" => Box::new(StdoutHandler),
        "stderr" => Box::new(StderrHandler),
        "none" => Box::new(NullHandler),
        path => Box::new(FileHandler::open(path)?),
    })
}

/// The installed handler. `None` means the default (stdout).
static HANDLER: RwLock<Option<Box<dyn LogHandler>>> = RwLock::new(None);

/// Replaces the global handler.
pub fn set_handler(handler: Box<dyn LogHandler>) {
    if let Ok(mut h) = HANDLER.write() {
        *h = Some(handler);
    }
}

/// Restores the default handler (stdout).
pub fn reset_handler() {
    if let Ok(mut h) = HANDLER.write() {
        *h = None;
    }
}

/// The lowest level that is written, stored as `Level as u8`. Defaults to `Info`.
static MIN_LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);

/// Sets the lowest level that is written.
pub fn set_level(level: Level) {
    MIN_LEVEL.store(level as u8, Ordering::Relaxed);
}

/// Whether messages of `level` are written.
pub fn enabled(level: Level) -> bool {
    level as u8 >= MIN_LEVEL.load(Ordering::Relaxed)
}

/// Sends a message to the global handler, unless its level is below the global level.
pub fn log(level: Level, msg: &str) {
    if !enabled(level) {
        return;
    }
    match HANDLER.read() {
        Ok(h) => match h.as_ref() {
            Some(handler) => handler.log(level, msg),
            None => StdoutHandler.log(level, msg),
        },
        // A panic while logging poisoned the lock; fall back to stdout.
        Err(_) => StdoutHandler.log(level, msg),
    }
}

/// Formats a duration for logs, e.g. "1.234s" or "12.346ms".
pub fn format_duration(d: Duration) -> String {
    format!("{d:.3?}")
}

/// Runs `f` and logs how long it took at debug level, as "time: <label>: <duration>".
pub fn timed<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let out = f();
    log(Level::Debug, &format!("time: {label}: {}", format_duration(start.elapsed())));
    out
}

/// Formats the message only if debug messages are enabled.
#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        if $crate::logging::enabled($crate::logging::Level::Debug) {
            $crate::logging::log($crate::logging::Level::Debug, &format!($($arg)*))
        }
    };
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => { $crate::logging::log($crate::logging::Level::Info, &format!($($arg)*)) };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => { $crate::logging::log($crate::logging::Level::Warn, &format!($($arg)*)) };
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => { $crate::logging::log($crate::logging::Level::Error, &format!($($arg)*)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_adds_prefix_except_to_info() {
        assert_eq!(format_line(Level::Debug, "x"), "Debug: x");
        assert_eq!(format_line(Level::Info, "OK"), "OK");
        assert_eq!(format_line(Level::Warn, "careful"), "Warning: careful");
        assert_eq!(format_line(Level::Error, "failed"), "Error: failed");
    }

    #[test]
    fn levels_parse_and_order() {
        assert_eq!("debug".parse::<Level>(), Ok(Level::Debug));
        assert_eq!("INFO".parse::<Level>(), Ok(Level::Info));
        assert_eq!("warning".parse::<Level>(), Ok(Level::Warn));
        assert!("verbose".parse::<Level>().is_err());
        assert!(Level::Debug < Level::Info && Level::Info < Level::Warn && Level::Warn < Level::Error);
    }

    #[test]
    fn duration_format() {
        assert_eq!(format_duration(Duration::from_millis(1234)), "1.234s");
        assert_eq!(format_duration(Duration::from_micros(12346)), "12.346ms");
    }

    // The only test that touches the global handler and level, so parallel tests do not clash.
    #[test]
    fn global_handler_and_level() {
        let mem = MemoryHandler::new();
        set_handler(Box::new(mem.clone()));

        // Default level: debug is dropped
        crate::log_debug!("hidden");
        crate::log_info!("hello {}", 1);
        crate::log_error!("bad");

        // Lowered level: debug and timings are written
        set_level(Level::Debug);
        crate::log_debug!("shown");
        let v = timed("step", || 42);
        assert_eq!(v, 42);

        // Raised level: info is dropped
        set_level(Level::Warn);
        crate::log_info!("dropped");
        crate::log_warn!("kept");

        set_level(Level::Info);
        reset_handler();
        crate::log_info!("not captured");

        let lines = mem.lines();
        assert_eq!(lines.len(), 5, "{lines:?}");
        assert_eq!(lines[0], (Level::Info, "hello 1".to_string()));
        assert_eq!(lines[1], (Level::Error, "bad".to_string()));
        assert_eq!(lines[2], (Level::Debug, "shown".to_string()));
        assert_eq!(lines[3].0, Level::Debug);
        assert!(lines[3].1.starts_with("time: step: "), "{}", lines[3].1);
        assert_eq!(lines[4], (Level::Warn, "kept".to_string()));
    }

    #[test]
    fn file_handler_appends_lines() {
        let path = std::env::temp_dir().join(format!("helpmate-log-test-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let h = handler_for(path.to_str().unwrap()).unwrap();
            h.log(Level::Info, "first");
            h.log(Level::Warn, "second");
        }
        {
            let h = handler_for(path.to_str().unwrap()).unwrap();
            h.log(Level::Info, "third");
        }
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(text, "first\nWarning: second\nthird\n");
    }
}
