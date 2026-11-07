//! Opt-in crash reports that stay on this computer. When the user has
//! switched them on, a panic anywhere in the app (or an error the interface
//! could not handle) is written to a text file in the app's data folder:
//! what happened, where, the version and the system, and nothing from the
//! user's songs or audio. The next launch offers to show it, so the user can
//! send it on if they like. Nothing is uploaded.

use std::fs;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::session::AppState;
use crate::settings;

type CmdResult<T> = Result<T, String>;

const SETTINGS: &str = "crash.json";
/// Interface errors in one run that are written down, at most.
const UI_LIMIT: usize = 5;

static ENABLED: AtomicBool = AtomicBool::new(false);
static DIR: OnceLock<PathBuf> = OnceLock::new();
static UI_REPORTS: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CrashSettings {
    /// The user chose to keep crash reports.
    pub enabled: bool,
    /// The user has been asked once.
    pub asked: bool,
    /// Reports older than this (seconds since 1970) have been seen.
    pub seen_until: u64,
}

pub fn load(config_dir: &Path) -> CrashSettings {
    fs::read_to_string(config_dir.join(SETTINGS)).ok().and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// `YYYY-MM-DD HH:MM:SS` in UTC, from seconds since 1970 (no date library needed).
fn utc(secs: u64) -> (String, String) {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let (h, mi, s) = (rest / 3600, rest / 60 % 60, rest % 60);
    (format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02} UTC"), format!("{y:04}{m:02}{d:02}-{h:02}{mi:02}{s:02}"))
}

/// The text of a report.
fn report(kind: &str, what: &str, place: &str, trace: &str) -> String {
    let (when, _) = utc(now());
    format!(
        "Soniq Studio crash report\n\
         Kind: {kind}\n\
         Version: {} ({} {})\n\
         When: {when}\n\
         Where: {place}\n\
         What: {what}\n\n\
         Trace:\n{trace}\n\n\
         Nothing from your songs or audio is in this report.\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}

/// Writes a report into `dir`; the file name carries the time.
fn write(dir: &Path, text: &str) -> std::io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let (_, stamp) = utc(now());
    let mut path = dir.join(format!("crash-{stamp}.txt"));
    for n in 2.. {
        if !path.exists() {
            break;
        }
        path = dir.join(format!("crash-{stamp}-{n}.txt"));
    }
    fs::write(&path, text)?;
    Ok(path)
}

fn panic_text(info: &PanicHookInfo) -> String {
    let what = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic without a message".into());
    let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
    let place = info.location().map_or_else(|| format!("thread '{thread}'"), |l| format!("thread '{thread}' at {}:{}:{}", l.file(), l.line(), l.column()));
    report("panic", &what, &place, &std::backtrace::Backtrace::force_capture().to_string())
}

/// Puts the panic hook in place; it writes a report only while reports are on.
pub fn install(dir: PathBuf, enabled: bool) {
    let _ = DIR.set(dir);
    ENABLED.store(enabled, Ordering::Relaxed);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if ENABLED.load(Ordering::Relaxed)
            && let Some(dir) = DIR.get()
        {
            let _ = write(dir, &panic_text(info));
        }
        previous(info);
    }));
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashReport {
    pub file: String,
    /// Seconds since 1970.
    pub time: u64,
    /// The "What:" line.
    pub summary: String,
    pub seen: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashStatus {
    pub enabled: bool,
    pub asked: bool,
    pub folder: String,
    pub reports: Vec<CrashReport>,
}

fn list(dir: &Path, seen_until: u64) -> Vec<CrashReport> {
    let mut reports: Vec<CrashReport> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?.to_string();
            (name.starts_with("crash-") && name.ends_with(".txt")).then_some(())?;
            let time = entry.metadata().ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
            let text = fs::read_to_string(&path).unwrap_or_default();
            let summary = text.lines().find_map(|l| l.strip_prefix("What: ")).unwrap_or("").chars().take(160).collect();
            Some(CrashReport { file: name, time, summary, seen: time <= seen_until })
        })
        .collect();
    reports.sort_by_key(|r| std::cmp::Reverse(r.time));
    reports
}

/// Only a report's own file name, inside the crash folder.
fn report_path(dir: &Path, file: &str) -> CmdResult<PathBuf> {
    let ok = Path::new(file).file_name().is_some_and(|n| n == file) && file.starts_with("crash-") && file.ends_with(".txt");
    ok.then(|| dir.join(file)).ok_or_else(|| "There is no such crash report.".into())
}

#[tauri::command]
pub fn crash_status(state: State<'_, AppState>) -> CrashStatus {
    let s = load(&state.dirs.config);
    let dir = state.dirs.crashes();
    CrashStatus { enabled: s.enabled, asked: s.asked, folder: dir.display().to_string(), reports: list(&dir, s.seen_until) }
}

/// Switches reports on or off (and remembers that the user has been asked).
#[tauri::command]
pub fn crash_enable(state: State<'_, AppState>, on: bool) -> CmdResult<()> {
    let mut s = load(&state.dirs.config);
    s.enabled = on;
    s.asked = true;
    ENABLED.store(on, Ordering::Relaxed);
    settings::write_json(&state.dirs.config, SETTINGS, &s)
}

/// Every report so far has been seen: the next launch says nothing about them.
#[tauri::command]
pub fn crash_seen(state: State<'_, AppState>) -> CmdResult<()> {
    let mut s = load(&state.dirs.config);
    s.seen_until = now();
    settings::write_json(&state.dirs.config, SETTINGS, &s)
}

#[tauri::command]
pub fn crash_read(state: State<'_, AppState>, file: String) -> CmdResult<String> {
    let path = report_path(&state.dirs.crashes(), &file)?;
    fs::read_to_string(&path).map_err(|e| format!("Could not read {}: {e}", path.display()))
}

#[tauri::command]
pub fn crash_delete(state: State<'_, AppState>, file: String) -> CmdResult<()> {
    let path = report_path(&state.dirs.crashes(), &file)?;
    fs::remove_file(&path).map_err(|e| format!("Could not delete {}: {e}", path.display()))
}

/// An error the interface could not handle: kept like a crash (when reports
/// are on), a few per run at most.
#[tauri::command]
pub fn crash_ui(state: State<'_, AppState>, message: String, stack: Option<String>) -> Option<String> {
    if !ENABLED.load(Ordering::Relaxed) || UI_REPORTS.fetch_add(1, Ordering::Relaxed) >= UI_LIMIT {
        return None;
    }
    let what: String = message.chars().take(500).collect();
    let trace: String = stack.unwrap_or_default().chars().take(6000).collect();
    write(&state.dirs.crashes(), &report("interface error", &what, "the interface", &trace)).ok().map(|p| p.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_come_out_right() {
        assert_eq!(utc(0).0, "1970-01-01 00:00:00 UTC");
        assert_eq!(utc(1_794_052_800).1, "20261107-120000");
        assert_eq!(utc(951_782_400).0, "2000-02-29 00:00:00 UTC");
    }

    #[test]
    fn a_panic_is_written_down_only_while_reports_are_on() {
        let dir = std::env::temp_dir().join(format!("soniq-crash-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        install(dir.clone(), false);
        let _ = std::thread::Builder::new().name("quiet".into()).spawn(|| panic!("not kept")).unwrap().join();
        assert!(list(&dir, 0).is_empty(), "off: nothing written");
        ENABLED.store(true, Ordering::Relaxed);
        let _ = std::thread::Builder::new().name("audio".into()).spawn(|| panic!("the mixer fell over")).unwrap().join();
        ENABLED.store(false, Ordering::Relaxed);
        let reports = list(&dir, 0);
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].summary, "the mixer fell over");
        let text = fs::read_to_string(dir.join(&reports[0].file)).unwrap();
        assert!(text.contains("thread 'audio' at") && text.contains("crash.rs") && text.contains(env!("CARGO_PKG_VERSION")), "{text}");
        assert!(list(&dir, u64::MAX)[0].seen);
        assert!(report_path(&dir, "../audio.json").is_err() && report_path(&dir, "notes.txt").is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
