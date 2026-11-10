//! Where the app keeps its files: the download cache and the log. Both can
//! be moved in Settings; the choice is saved in `config.json` in the app's
//! config folder, read at startup (the log starts before the UI loads).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    /// Folder holding the download cache (`portal/`, `s3-cache/`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cache_dir: Option<PathBuf>,
    /// Folder holding `qpxui.log`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    log_dir: Option<PathBuf>,
}

struct State {
    /// `config.json`, if the app has a config folder.
    file: Option<PathBuf>,
    /// The platform's default log folder.
    default_log_dir: Option<PathBuf>,
    config: Config,
}

static STATE: OnceLock<RwLock<State>> = OnceLock::new();

fn state() -> &'static RwLock<State> {
    STATE.get_or_init(|| {
        RwLock::new(State {
            file: None,
            default_log_dir: None,
            config: Config::default(),
        })
    })
}

/// Read `config_dir/config.json`. A missing file means defaults; an
/// unreadable one is reported (after the log starts) and ignored.
pub fn init(config_dir: Option<PathBuf>, default_log_dir: Option<PathBuf>) -> Result<(), String> {
    let file = config_dir.map(|d| d.join("config.json"));
    let mut problem = Ok(());
    let config = match file.as_deref().map(std::fs::read_to_string) {
        Some(Ok(text)) => serde_json::from_str(&text).unwrap_or_else(|e| {
            problem = Err(format!("ignoring unreadable settings file: {e}"));
            Config::default()
        }),
        _ => Config::default(),
    };
    *state().write().unwrap() = State {
        file,
        default_log_dir,
        config,
    };
    problem
}

fn save(config: &Config) -> Result<(), String> {
    let Some(file) = state().read().unwrap().file.clone() else {
        return Err("no settings folder available".into());
    };
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &file).map_err(|e| format!("{}: {e}", file.display()))
}

/// The default download cache folder: `<user cache folder>/qpxui`.
fn default_cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("qpxui"))
}

/// The download cache folder in use.
pub fn cache_dir() -> Result<PathBuf, String> {
    state()
        .read()
        .unwrap()
        .config
        .cache_dir
        .clone()
        .or_else(default_cache_dir)
        .ok_or_else(|| "no cache directory available".to_string())
}

/// Whether the download cache is in its default folder (the only place an
/// old cache layout can be, so the only place it is cleaned up).
pub fn cache_dir_is_default() -> bool {
    state().read().unwrap().config.cache_dir.is_none()
}

/// The log folder in use.
pub fn log_dir() -> Option<PathBuf> {
    let s = state().read().unwrap();
    s.config
        .log_dir
        .clone()
        .or_else(|| s.default_log_dir.clone())
}

/// A folder the user picked: absolute, created if missing, and writable.
fn check_folder(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path.trim());
    if !p.is_absolute() {
        return Err(format!("{path}: not an absolute folder path"));
    }
    std::fs::create_dir_all(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    if !p.is_dir() {
        return Err(format!("{}: not a folder", p.display()));
    }
    let probe = p.join(".qpxui-write-test");
    std::fs::write(&probe, b"ok")
        .map_err(|e| format!("{}: cannot write there: {e}", p.display()))?;
    let _ = std::fs::remove_file(&probe);
    Ok(p)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoragePaths {
    cache_dir: Option<String>,
    cache_dir_default: Option<String>,
    log_dir: Option<String>,
    log_dir_default: Option<String>,
}

fn show(p: Option<&Path>) -> Option<String> {
    p.map(|p| p.to_string_lossy().into_owned())
}

/// Tauri command: the folders in use and their defaults.
#[tauri::command]
pub fn storage_paths() -> StoragePaths {
    let s = state().read().unwrap();
    let default_cache = default_cache_dir();
    StoragePaths {
        cache_dir: show(s.config.cache_dir.as_deref().or(default_cache.as_deref())),
        cache_dir_default: show(default_cache.as_deref()),
        log_dir: show(s.config.log_dir.as_deref().or(s.default_log_dir.as_deref())),
        log_dir_default: show(s.default_log_dir.as_deref()),
    }
}

/// Apply a change to the config and save it; nothing changes if saving fails.
fn update(change: impl FnOnce(&mut Config)) -> Result<(), String> {
    let mut next = state().read().unwrap().config.clone();
    change(&mut next);
    save(&next)?;
    state().write().unwrap().config = next;
    Ok(())
}

/// Tauri command: move the download cache to `path` (`None`: the default).
/// Files already downloaded stay where they are.
#[tauri::command]
pub fn set_cache_dir(path: Option<String>) -> Result<StoragePaths, String> {
    let dir = path.as_deref().map(check_folder).transpose()?;
    update(|c| c.cache_dir = dir.clone())?;
    log::info!("download cache folder: {}", cache_dir()?.display());
    Ok(storage_paths())
}

/// Tauri command: move the log to `path` (`None`: the default). Logging
/// continues in the new folder at once.
#[tauri::command]
pub fn set_log_dir(path: Option<String>) -> Result<StoragePaths, String> {
    let dir = path.as_deref().map(check_folder).transpose()?;
    update(|c| c.log_dir = dir.clone())?;
    crate::logging::reopen(log_dir())?;
    Ok(storage_paths())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picked_folders_must_be_absolute_and_writable() {
        assert!(check_folder("relative/dir")
            .unwrap_err()
            .contains("absolute"));
        let tmp = tempfile::tempdir().unwrap();
        let new = tmp.path().join("a").join("b");
        assert_eq!(check_folder(new.to_str().unwrap()).unwrap(), new);
        assert!(new.is_dir() && !new.join(".qpxui-write-test").exists());
        let file = tmp.path().join("f");
        std::fs::write(&file, b"x").unwrap();
        assert!(check_folder(file.to_str().unwrap()).is_err());
    }

    #[test]
    fn config_round_trips_and_ignores_unknown_or_bad_files() {
        let c: Config = serde_json::from_str(r#"{"cacheDir":"/x/cache","other":1}"#).unwrap();
        assert_eq!(c.cache_dir.as_deref(), Some(Path::new("/x/cache")));
        assert!(c.log_dir.is_none());
        assert_eq!(serde_json::to_string(&Config::default()).unwrap(), "{}");
    }
}
