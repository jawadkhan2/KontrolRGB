//! Tiny rolling file logger.
//!
//! Release builds are a GUI-subsystem exe, so stderr goes nowhere and every
//! diagnostic used to vanish — which is why "the fans went rainbow overnight"
//! was impossible to diagnose after the fact. `klog!` writes each line to
//! stderr (useful under `tauri dev`) AND appends it, timestamped, to
//! `%LOCALAPPDATA%\com.jawad.kontrolrgb\logs\kontrolrgb.log`. When the file
//! passes `MAX_BYTES` it is rotated to `kontrolrgb.1.log` (one generation kept),
//! so the log can never grow without bound.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

use parking_lot::Mutex;

const MAX_BYTES: u64 = 2 * 1024 * 1024;
const APP_ID: &str = "com.jawad.kontrolrgb";

struct Sink {
    path: PathBuf,
    file: Option<File>,
    written: u64,
}

static SINK: OnceLock<Mutex<Sink>> = OnceLock::new();

/// Directory holding the log files (created on first use).
pub fn log_dir() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join(APP_ID).join("logs"))
}

/// Open the log file. Safe to call more than once; only the first call counts.
/// Called at the very top of `run()` so even pre-setup messages are captured.
pub fn init() {
    let Some(dir) = log_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("kontrolrgb.log");
    let file = OpenOptions::new().create(true).append(true).open(&path).ok();
    let written = file
        .as_ref()
        .and_then(|f| f.metadata().ok())
        .map_or(0, |m| m.len());
    let _ = SINK.set(Mutex::new(Sink {
        path,
        file,
        written,
    }));
    write_line(&format!(
        "===== KontrolRGB {} starting (pid {}) =====",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    ));
}

/// Write one already-formatted line. Use the `klog!` macro instead.
pub fn write_line(msg: &str) {
    eprintln!("{msg}");
    let Some(sink) = SINK.get() else { return };
    let mut s = sink.lock();
    let line = format!("{} {msg}\n", timestamp());
    if s.written + line.len() as u64 > MAX_BYTES {
        rotate(&mut s);
    }
    if let Some(f) = s.file.as_mut() {
        if f.write_all(line.as_bytes()).is_ok() {
            s.written += line.len() as u64;
        }
    }
}

fn rotate(s: &mut Sink) {
    s.file = None; // close before renaming (Windows can't rename an open file)
    let old = s.path.with_file_name("kontrolrgb.1.log");
    let _ = std::fs::remove_file(&old);
    let _ = std::fs::rename(&s.path, &old);
    s.file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&s.path)
        .ok();
    s.written = 0;
}

#[cfg(windows)]
fn timestamp() -> String {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe {
        let mut t = std::mem::zeroed();
        GetLocalTime(&mut t);
        t
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}

#[cfg(not(windows))]
fn timestamp() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}", d.as_secs(), d.subsec_millis())
}

/// `eprintln!` replacement that also lands in the rolling log file.
#[macro_export]
macro_rules! klog {
    ($($arg:tt)*) => {
        $crate::logging::write_line(&format!($($arg)*))
    };
}
