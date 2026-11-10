//! The app's log: everything the backend and the frontend log goes to stderr
//! and to `qpxui.log` in the app's log folder, so a user can look at it and
//! share it with the developers (Settings → Diagnostics).
//!
//! Two levels: normal (info for the app, warnings for libraries) and
//! detailed (debug for the app: every query, every streamed range).

use log::{Level, LevelFilter, Log, Metadata, Record};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// A log file over this size at startup is moved to `qpxui.old.log`.
const ROTATE_BYTES: u64 = 5 << 20;
/// Most bytes of the log the Diagnostics tab reads back.
const TAIL_MAX: u64 = 2 << 20;

struct Logger {
    file: Mutex<Option<File>>,
    path: Mutex<Option<PathBuf>>,
}

static LOGGER: Logger = Logger {
    file: Mutex::new(None),
    path: Mutex::new(None),
};
static INIT: OnceLock<()> = OnceLock::new();

/// Whether a record from `target` at `level` is kept, with the app's level
/// set to `app`. Libraries (hyper, reqwest, the AWS SDK, …) log a lot at
/// info and debug, so only their warnings are kept.
fn keep(target: &str, level: Level, app: LevelFilter) -> bool {
    if target == "qpxui" || target.starts_with("qpxui") {
        level <= app
    } else {
        level <= Level::Warn
    }
}

impl Log for Logger {
    fn enabled(&self, m: &Metadata) -> bool {
        keep(m.target(), m.level(), log::max_level())
    }

    fn log(&self, r: &Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let line = format!(
            "{} {:<5} {}: {}\n",
            httpdate::fmt_http_date(std::time::SystemTime::now()),
            r.level(),
            r.target(),
            r.args()
        );
        eprint!("{line}");
        if let Some(f) = self.file.lock().unwrap().as_mut() {
            let _ = f.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {
        if let Some(f) = self.file.lock().unwrap().as_mut() {
            let _ = f.flush();
        }
    }
}

/// Start logging to stderr and to `dir/qpxui.log`. Without a usable folder
/// the log goes to stderr only. Safe to call more than once.
pub fn init(dir: Option<PathBuf>, detailed: bool) {
    if INIT.set(()).is_ok() {
        // Another logger already installed (tests): keep it.
        if log::set_logger(&LOGGER).is_err() {
            return;
        }
    }
    set_detailed(detailed);
    if let Some(dir) = dir {
        match open(&dir) {
            Ok((file, path)) => {
                *LOGGER.file.lock().unwrap() = Some(file);
                *LOGGER.path.lock().unwrap() = Some(path);
            }
            Err(e) => eprintln!("log file in {}: {e}", dir.display()),
        }
    }
    log::info!(
        target: "qpxui",
        "QPX UI {} started on {} {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
}

/// Continue the log in `dir` (after the user moved it in Settings).
pub fn reopen(dir: Option<PathBuf>) -> Result<(), String> {
    let Some(dir) = dir else {
        return Err("no log folder available".into());
    };
    let (file, path) = open(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    LOGGER.flush();
    *LOGGER.file.lock().unwrap() = Some(file);
    *LOGGER.path.lock().unwrap() = Some(path.clone());
    log::info!(
        target: "qpxui",
        "QPX UI {} log continues in {}",
        env!("CARGO_PKG_VERSION"),
        path.display()
    );
    Ok(())
}

/// Open `dir/qpxui.log` for appending, moving a large one aside first.
fn open(dir: &Path) -> std::io::Result<(File, PathBuf)> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("qpxui.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > ROTATE_BYTES) {
        std::fs::rename(&path, dir.join("qpxui.old.log"))?;
    }
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    Ok((file, path))
}

/// Switch between normal and detailed logging.
pub fn set_detailed(detailed: bool) {
    log::set_max_level(if detailed {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    });
}

/// The last `max` bytes of `path` (whole lines only), or "" if it can't be read.
fn tail(path: &Path, max: u64) -> String {
    let Ok(mut f) = File::open(path) else {
        return String::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let from = len.saturating_sub(max);
    if f.seek(SeekFrom::Start(from)).is_err() {
        return String::new();
    }
    let mut buf = Vec::new();
    let _ = f.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf).into_owned();
    if from > 0 {
        // Drop the partial first line.
        text.split_once('\n')
            .map_or(text.clone(), |(_, rest)| rest.to_string())
    } else {
        text
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogInfo {
    /// The log file, if the app could open one.
    path: Option<String>,
    detailed: bool,
    /// The end of the log (up to 2 MB).
    text: String,
}

/// Tauri command: where the log is, its level and its last lines.
#[tauri::command]
pub fn log_read() -> LogInfo {
    LOGGER.flush();
    let path = LOGGER.path.lock().unwrap().clone();
    LogInfo {
        text: path
            .as_deref()
            .map(|p| tail(p, TAIL_MAX))
            .unwrap_or_default(),
        path: path.map(|p| p.to_string_lossy().into_owned()),
        detailed: log::max_level() >= LevelFilter::Debug,
    }
}

/// Tauri command: turn detailed logging on or off.
#[tauri::command]
pub fn log_set_detailed(detailed: bool) {
    set_detailed(detailed);
    log::info!(target: "qpxui", "detailed logging {}", if detailed { "on" } else { "off" });
}

/// Tauri command: a line from the frontend (`level` is error, warn, info or
/// debug).
#[tauri::command]
pub fn log_write(level: String, message: String) {
    let level = match level.as_str() {
        "error" => Level::Error,
        "warn" => Level::Warn,
        "debug" => Level::Debug,
        _ => Level::Info,
    };
    // Keep one line per record, and a runaway message short.
    let mut message = message.replace('\n', " ⏎ ");
    if message.len() > 8000 {
        let cut = (0..=8000)
            .rev()
            .find(|&i| message.is_char_boundary(i))
            .unwrap_or(0);
        message.truncate(cut);
        message.push('…');
    }
    log::log!(target: "qpxui::ui", level, "{message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libraries_log_only_warnings() {
        assert!(keep("qpxui::ui", Level::Info, LevelFilter::Info));
        assert!(!keep("qpxui::ui", Level::Debug, LevelFilter::Info));
        assert!(keep("qpxui::stream", Level::Debug, LevelFilter::Debug));
        assert!(!keep("hyper::proto", Level::Info, LevelFilter::Debug));
        assert!(keep("reqwest", Level::Warn, LevelFilter::Info));
    }

    #[test]
    fn tail_keeps_whole_lines() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.log");
        std::fs::write(&p, "first line\nsecond line\nthird\n").unwrap();
        assert_eq!(tail(&p, 1000), "first line\nsecond line\nthird\n");
        assert_eq!(tail(&p, 10), "third\n");
        assert_eq!(tail(&dir.path().join("none"), 10), "");
    }

    #[test]
    fn large_logs_are_rotated() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("qpxui.log"),
            vec![b'x'; ROTATE_BYTES as usize + 1],
        )
        .unwrap();
        let (_, path) = open(dir.path()).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        assert!(dir.path().join("qpxui.old.log").exists());
    }
}
