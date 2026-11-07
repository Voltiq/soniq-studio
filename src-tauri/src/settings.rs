//! The user's audio device choice, MIDI inputs and saved macros, kept in the
//! app config folder.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use soniq_engine::EngineConfig;
use soniq_io::Effect;

const FILE: &str = "audio.json";
const MACROS: &str = "macros.json";
const MIDI: &str = "midi.json";
const RECENT: &str = "recent.json";
/// Songs the recent list remembers.
const RECENT_LIMIT: usize = 12;

/// A song opened or saved lately.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    pub path: String,
    pub name: String,
    /// Seconds since 1970 when it was last opened or saved.
    pub opened: u64,
}

pub fn load_recent(config_dir: &Path) -> Vec<RecentProject> {
    fs::read_to_string(config_dir.join(RECENT)).ok().and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
}

/// Puts a song at the top of the recent list.
pub fn remember_recent(config_dir: &Path, path: &Path, name: &str) {
    let path = path.to_string_lossy().into_owned();
    let mut recent = load_recent(config_dir);
    recent.retain(|r| !r.path.eq_ignore_ascii_case(&path));
    let opened = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    recent.insert(0, RecentProject { path, name: name.to_string(), opened });
    recent.truncate(RECENT_LIMIT);
    let _ = write_json(config_dir, RECENT, &recent);
}

pub fn forget_recent(config_dir: &Path, path: &str) -> Result<(), String> {
    let mut recent = load_recent(config_dir);
    recent.retain(|r| !r.path.eq_ignore_ascii_case(path));
    write_json(config_dir, RECENT, &recent)
}

/// MIDI input ports the user switched off (all others are used).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MidiSettings {
    pub disabled_inputs: Vec<String>,
}

pub fn load_midi(config_dir: &Path) -> MidiSettings {
    fs::read_to_string(config_dir.join(MIDI)).ok().and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
}

pub fn save_midi(config_dir: &Path, settings: &MidiSettings) -> Result<(), String> {
    write_json(config_dir, MIDI, settings)
}

pub fn load(config_dir: &Path) -> EngineConfig {
    fs::read_to_string(config_dir.join(FILE)).ok().and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
}

pub fn save(config_dir: &Path, config: &EngineConfig) -> Result<(), String> {
    write_json(config_dir, FILE, config)
}

/// A named chain of effects, applied to clips or to a folder of files.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Macro {
    pub id: String,
    pub name: String,
    pub steps: Vec<Effect>,
}

pub fn load_macros(config_dir: &Path) -> Vec<Macro> {
    fs::read_to_string(config_dir.join(MACROS)).ok().and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
}

pub fn save_macros(config_dir: &Path, macros: &[Macro]) -> Result<(), String> {
    write_json(config_dir, MACROS, &macros)
}

pub(crate) fn write_json(config_dir: &Path, file: &str, value: &impl Serialize) -> Result<(), String> {
    fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    fs::write(config_dir.join(file), json).map_err(|e| e.to_string())
}
