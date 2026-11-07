//! The browser's library: folders of samples and presets the user adds,
//! indexed in the background (what each sample is: loop or one-shot, tempo,
//! key, tags from its name), with favourites, the user's own tags and the
//! items used lately. Samples can be auditioned in time with the song, and
//! any device saved as a preset file.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use soniq_engine::PreviewMode;
use soniq_io::{AudioBuffer, KeyGuess, SUPPORTED_EXTENSIONS, SampleInfo, SampleKind};
use soniq_project::{Device, catalog, new_id};
use tauri::State;

use crate::session::{AppState, lock};

type CmdResult<T> = Result<T, String>;

const FILE: &str = "library.json";
pub const PRESET_EXTENSION: &str = "soniqpreset";
/// Files larger than this are listed but not listened to.
const MAX_ANALYSE_BYTES: u64 = 64 * 1024 * 1024;
/// The most files one scan indexes.
const MAX_FILES: usize = 50_000;
const RECENT_LIMIT: usize = 40;
/// An audition plays at most this much of a long file.
const MAX_AUDITION_SECONDS: f64 = 60.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    Sample,
    Preset,
}

/// A file in a library folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub path: String,
    /// The file name without its extension.
    pub name: String,
    /// The library folder it was found in.
    pub folder: String,
    pub kind: EntryKind,
    pub size: u64,
    /// Seconds since 1970.
    pub modified: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<SampleInfo>,
    /// Tags from its name and folders, and loop or one-shot.
    #[serde(default)]
    pub auto_tags: Vec<String>,
    /// A preset's device type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// Why it could not be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What is kept in `library.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LibraryData {
    pub folders: Vec<String>,
    pub entries: Vec<Entry>,
    pub favourites: Vec<String>,
    /// The user's own tags, by path.
    pub tags: BTreeMap<String, Vec<String>>,
    /// Paths used lately, newest first.
    pub recent: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Library {
    pub data: LibraryData,
    /// While indexing: files done and files to do.
    pub scan: Option<(usize, usize)>,
    /// Another scan was asked for while one ran.
    pending: bool,
    /// Where presets are saved; always indexed.
    pub presets: PathBuf,
    config: PathBuf,
    /// The last sample auditioned, decoded.
    heard: Option<(String, Arc<AudioBuffer>)>,
}

impl Library {
    pub fn load(config: &Path, presets: PathBuf) -> Self {
        let data = fs::read_to_string(config.join(FILE)).ok().and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default();
        Self { data, presets, config: config.to_path_buf(), ..Self::default() }
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_string(&self.data) {
            let _ = fs::create_dir_all(&self.config);
            let _ = fs::write(self.config.join(FILE), json);
        }
    }

    /// The folders a scan looks in: the user's, and the presets folder if it exists.
    fn roots(&self) -> Vec<String> {
        let mut roots = self.data.folders.clone();
        let presets = self.presets.to_string_lossy().into_owned();
        if self.presets.is_dir() && !roots.iter().any(|f| same_path(f, &presets)) {
            roots.push(presets);
        }
        roots
    }

    fn view(&self) -> LibraryView {
        LibraryView {
            folders: self.data.folders.clone(),
            entries: self.data.entries.clone(),
            favourites: self.data.favourites.clone(),
            tags: self.data.tags.clone(),
            recent: self.data.recent.clone(),
            scanning: self.scan,
            presets: self.presets.to_string_lossy().into_owned(),
        }
    }
}

fn same_path(a: &str, b: &str) -> bool {
    let norm = |p: &str| p.trim_end_matches(['/', '\\']).replace('\\', "/").to_lowercase();
    norm(a) == norm(b)
}

/// The library as the browser shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryView {
    pub folders: Vec<String>,
    pub entries: Vec<Entry>,
    pub favourites: Vec<String>,
    pub tags: BTreeMap<String, Vec<String>>,
    pub recent: Vec<String>,
    pub scanning: Option<(usize, usize)>,
    pub presets: String,
}

/// Every sample and preset file under `root` (hidden files and folders skipped).
fn walk(root: &Path, out: &mut Vec<PathBuf>) {
    let mut stack = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(items) = fs::read_dir(&dir) else { continue };
        for item in items.flatten() {
            if out.len() >= MAX_FILES {
                return;
            }
            let path = item.path();
            let hidden = path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'));
            let Ok(kind) = item.file_type() else { continue };
            if hidden {
                continue;
            }
            if kind.is_dir() && depth < 12 {
                stack.push((path, depth + 1));
            } else if kind.is_file() && entry_kind(&path).is_some() {
                out.push(path);
            }
        }
    }
}

fn entry_kind(path: &Path) -> Option<EntryKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if ext == PRESET_EXTENSION {
        Some(EntryKind::Preset)
    } else {
        SUPPORTED_EXTENSIONS.contains(&ext.as_str()).then_some(EntryKind::Sample)
    }
}

fn stamp(path: &Path) -> (u64, u64) {
    let meta = fs::metadata(path).ok();
    let size = meta.as_ref().map_or(0, fs::Metadata::len);
    let modified = meta.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    (size, modified)
}

/// Reads one file for the index.
fn index_file(path: &Path, folder: &str) -> Entry {
    let (size, modified) = stamp(path);
    let kind = entry_kind(path).unwrap_or(EntryKind::Sample);
    let mut entry = Entry {
        path: path.to_string_lossy().into_owned(),
        name: path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        folder: folder.to_string(),
        kind,
        size,
        modified,
        info: None,
        auto_tags: Vec::new(),
        device: None,
        error: None,
    };
    // Folder names below the library folder count too ("Drums/Kicks/…").
    let relative = path.strip_prefix(folder).map_or_else(|_| entry.name.clone(), |p| p.to_string_lossy().into_owned());
    entry.auto_tags = soniq_io::name_tags(&relative);
    match kind {
        EntryKind::Preset => match read_preset(path) {
            Ok(device) => {
                entry.auto_tags.insert(0, "preset".into());
                entry.device = Some(device.kind);
            }
            Err(err) => entry.error = Some(err),
        },
        EntryKind::Sample if size > MAX_ANALYSE_BYTES => entry.error = Some("Too large to listen to while indexing".into()),
        EntryKind::Sample => match soniq_io::decode_file(path) {
            Ok(buffer) => {
                let info = soniq_io::describe(&buffer, &relative);
                let kind = if info.kind == SampleKind::Loop { "loop" } else { "one-shot" };
                entry.auto_tags.retain(|t| t != "loop" && t != "one-shot");
                entry.auto_tags.insert(0, kind.into());
                entry.info = Some(info);
            }
            Err(err) => entry.error = Some(err.to_string()),
        },
    }
    entry
}

/// Indexes the library folders in the background: files already indexed
/// and unchanged are kept, new and changed ones are listened to, on every core.
fn start_scan(library: Arc<Mutex<Library>>) {
    let roots = {
        let mut lib = lock(&library);
        if lib.scan.is_some() {
            lib.pending = true;
            return;
        }
        lib.scan = Some((0, 0));
        lib.roots()
    };
    std::thread::spawn(move || {
        let mut files: Vec<(PathBuf, String)> = Vec::new();
        for root in &roots {
            let mut found = Vec::new();
            walk(Path::new(root), &mut found);
            files.extend(found.into_iter().map(|p| (p, root.clone())));
        }
        let known: BTreeMap<String, Entry> = lock(&library).data.entries.iter().map(|e| (e.path.clone(), e.clone())).collect();
        let mut kept = Vec::new();
        let mut todo = Vec::new();
        for (path, root) in files {
            let key = path.to_string_lossy().into_owned();
            match known.get(&key) {
                Some(e) if (e.size, e.modified) == stamp(&path) && e.folder == root => kept.push(e.clone()),
                _ => todo.push((path, root)),
            }
        }
        lock(&library).scan = Some((0, todo.len()));
        let next = AtomicUsize::new(0);
        let done = AtomicUsize::new(0);
        let results = Mutex::new(Vec::with_capacity(todo.len()));
        let workers = std::thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(1).max(1));
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some((path, root)) = todo.get(i) else { break };
                        // A file that makes the decoder panic is listed as unreadable.
                        let entry = std::panic::catch_unwind(|| index_file(path, root)).unwrap_or_else(|_| Entry {
                            path: path.to_string_lossy().into_owned(),
                            name: path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
                            folder: root.clone(),
                            kind: EntryKind::Sample,
                            size: 0,
                            modified: 0,
                            info: None,
                            auto_tags: Vec::new(),
                            device: None,
                            error: Some("Could not be read".into()),
                        });
                        lock(&results).push(entry);
                        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                        if let Some(scan) = lock(&library).scan.as_mut() {
                            scan.0 = n;
                        }
                    }
                });
            }
        });
        kept.extend(results.into_inner().unwrap_or_default());
        kept.sort_by_key(|e| e.path.to_lowercase());
        let again = {
            let mut lib = lock(&library);
            lib.data.entries = kept;
            lib.scan = None;
            lib.save();
            std::mem::take(&mut lib.pending)
        };
        if again {
            start_scan(library);
        }
    });
}

#[tauri::command]
pub fn library_get(state: State<'_, AppState>) -> LibraryView {
    lock(&state.library).view()
}

/// Indexes a new folder (and everything in it).
#[tauri::command]
pub fn library_add_folder(state: State<'_, AppState>, path: String) -> CmdResult<LibraryView> {
    if !Path::new(&path).is_dir() {
        return Err("That folder cannot be opened.".into());
    }
    {
        let mut lib = lock(&state.library);
        if !lib.data.folders.iter().any(|f| same_path(f, &path)) {
            lib.data.folders.push(path);
            lib.save();
        }
    }
    start_scan(Arc::clone(&state.library));
    Ok(lock(&state.library).view())
}

/// Stops indexing a folder. Its files stay where they are.
#[tauri::command]
pub fn library_remove_folder(state: State<'_, AppState>, path: String) -> LibraryView {
    let mut lib = lock(&state.library);
    lib.data.folders.retain(|f| !same_path(f, &path));
    lib.data.entries.retain(|e| !same_path(&e.folder, &path));
    lib.save();
    lib.view()
}

/// Looks through the folders again for new, changed and removed files.
#[tauri::command]
pub fn library_rescan(state: State<'_, AppState>) -> LibraryView {
    start_scan(Arc::clone(&state.library));
    lock(&state.library).view()
}

#[tauri::command]
pub fn library_favourite(state: State<'_, AppState>, path: String, on: bool) {
    let mut lib = lock(&state.library);
    lib.data.favourites.retain(|p| p != &path);
    if on {
        lib.data.favourites.push(path);
    }
    lib.save();
}

/// The user's own tags for a file (trimmed, lower case, once each).
#[tauri::command]
pub fn library_tags(state: State<'_, AppState>, path: String, tags: Vec<String>) -> Vec<String> {
    let mut clean: Vec<String> = Vec::new();
    for t in tags.iter().map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty()) {
        if !clean.contains(&t) {
            clean.push(t);
        }
    }
    let mut lib = lock(&state.library);
    if clean.is_empty() {
        lib.data.tags.remove(&path);
    } else {
        lib.data.tags.insert(path, clean.clone());
    }
    lib.save();
    clean
}

/// Puts a file at the top of the recent items (it was added to the song).
#[tauri::command]
pub fn library_used(state: State<'_, AppState>, path: String) {
    let mut lib = lock(&state.library);
    lib.data.recent.retain(|p| p != &path);
    lib.data.recent.insert(0, path);
    lib.data.recent.truncate(RECENT_LIMIT);
    lib.save();
}

/// How an audition plays, for the browser to show.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Audition {
    pub seconds: f64,
    pub looping: bool,
    /// The loop's own tempo, when it was stretched to the song's.
    pub from_bpm: Option<f64>,
    /// Semitones it was moved to the song's key.
    pub semitones: i32,
}

/// The song's key as the library compares keys ("A minor" → A, minor).
fn song_key(name: &str) -> KeyGuess {
    let lower = name.to_lowercase();
    let minor = lower.contains("minor") || lower.contains("aeolian") || lower.contains("dorian") || lower.contains("phrygian");
    let tonic = soniq_io::name_key(&format!("{} major", name.split_whitespace().next().unwrap_or("C"))).map_or(0, |k| k.tonic);
    KeyGuess { tonic, minor, root: false }
}

/// Plays a library sample on top of the mix. In time (`sync`), a loop is
/// stretched to the song's tempo and loops from the next bar; a tonal sample
/// is moved to the song's key; one-shots start on the next beat while the song plays.
#[tauri::command]
pub async fn library_audition(state: State<'_, AppState>, path: String, sync: bool) -> CmdResult<Audition> {
    let cached = lock(&state.library).heard.as_ref().filter(|(p, _)| *p == path).map(|(_, b)| Arc::clone(b));
    let buffer = match cached {
        Some(b) => b,
        None => {
            let mut b = soniq_io::decode_file(Path::new(&path)).map_err(|e| e.to_string())?;
            let most = (MAX_AUDITION_SECONDS * f64::from(b.sample_rate)) as usize;
            b.channels.iter_mut().for_each(|c| c.truncate(most));
            let b = Arc::new(b);
            lock(&state.library).heard = Some((path.clone(), Arc::clone(&b)));
            b
        }
    };
    let entry = lock(&state.library).data.entries.iter().find(|e| e.path == path).cloned();
    let name = entry.as_ref().map_or_else(|| path.clone(), |e| e.name.clone());
    let info = entry.and_then(|e| e.info).unwrap_or_else(|| soniq_io::describe(&buffer, &name));
    let (bpm, beats_per_bar, key) = {
        let session = lock(&state.session);
        (session.project.bpm, f64::from(session.project.beats_per_bar), song_key(&session.project.key))
    };
    let looping = sync && info.kind == SampleKind::Loop;
    let mut audio: AudioBuffer = (*buffer).clone();
    let mut from_bpm = None;
    let mut semitones = 0;
    if sync {
        if let Some(own) = info.bpm.filter(|_| looping) {
            audio = soniq_io::time_stretch(&audio, own / bpm);
            from_bpm = Some(own);
        }
        if let Some(k) = info.key {
            semitones = k.shift_to(key);
            if semitones != 0 {
                audio = soniq_io::change_pitch(&audio, f64::from(semitones));
            }
        }
    }
    let audio = soniq_io::resample(&audio, state.engine.sample_rate());
    let seconds = audio.duration();
    let grid = match (sync, looping) {
        (false, _) => 0.0,
        (true, true) => beats_per_bar,
        (true, false) => 1.0,
    };
    state.engine.audition(Arc::new(audio), PreviewMode { looping, grid })?;
    Ok(Audition { seconds, looping, from_bpm, semitones })
}

#[tauri::command]
pub fn library_stop(state: State<'_, AppState>) {
    state.engine.stop_preview();
}

/// A preset file: a device with its settings (and any pad effects), no samples.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PresetFile {
    format: String,
    version: u32,
    device: Device,
}

fn read_preset(path: &Path) -> Result<Device, String> {
    let json = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let file: PresetFile = serde_json::from_str(&json).map_err(|_| "Not a Soniq preset".to_string())?;
    if file.format != "soniq-preset" {
        return Err("Not a Soniq preset".into());
    }
    Ok(file.device)
}

/// A file name without characters Windows refuses.
fn safe_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c }).collect();
    let cleaned = cleaned.trim().trim_end_matches('.').to_string();
    if cleaned.is_empty() { "Preset".into() } else { cleaned }
}

/// Saves a device (on `owner`'s chain) as a preset in the presets folder,
/// under the device's name; returns the file's path.
#[tauri::command]
pub fn preset_save(state: State<'_, AppState>, owner: String, device: String, name: String) -> CmdResult<String> {
    let mut d = {
        let session = lock(&state.session);
        let chain = session.project.chain(&owner).ok_or("That track no longer exists.")?;
        chain.iter().flat_map(Device::with_inner).find(|d| d.id == device).cloned().ok_or("That device no longer exists.")?
    };
    if d.kind == soniq_project::PLUGIN_TYPE {
        return Err("Plugin presets are not supported yet.".into());
    }
    // Samples belong to the song; the preset keeps the settings.
    d.samples.clear();
    for chain in &mut d.chains {
        chain.device.samples.clear();
    }
    let presets = lock(&state.library).presets.clone();
    let folder = presets.join(safe_name(&d.name));
    fs::create_dir_all(&folder).map_err(|e| format!("Could not make the presets folder: {e}"))?;
    let path = folder.join(format!("{}.{PRESET_EXTENSION}", safe_name(&name)));
    let file = PresetFile { format: "soniq-preset".into(), version: 1, device: d };
    let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| format!("Could not save the preset: {e}"))?;
    let root = presets.to_string_lossy().into_owned();
    let entry = index_file(&path, &root);
    let mut lib = lock(&state.library);
    lib.data.entries.retain(|e| e.path != entry.path);
    lib.data.entries.push(entry);
    lib.data.entries.sort_by_key(|e| e.path.to_lowercase());
    lib.save();
    Ok(path.to_string_lossy().into_owned())
}

/// A preset's device, ready to add (fresh ids, parameters brought up to date).
#[tauri::command]
pub fn preset_load(path: String) -> CmdResult<Device> {
    let mut device = read_preset(Path::new(&path))?;
    device.id = new_id("dev");
    catalog::refresh(&mut device);
    for chain in &mut device.chains {
        let id = new_id("ch");
        for m in device.macros.iter_mut().filter(|m| m.chain == chain.id) {
            m.chain = id.clone();
        }
        chain.id = id;
        chain.device.id = new_id("dev");
        catalog::refresh(&mut chain.device);
    }
    for d in device.pad_chains.iter_mut().flat_map(|c| c.devices.iter_mut()) {
        d.id = new_id("dev");
        catalog::refresh(d);
    }
    device.macros.retain(|m| device.chains.iter().any(|c| c.id == m.chain));
    Ok(device)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn song_keys_read_as_tonic_and_mode() {
        assert_eq!(song_key("A minor"), KeyGuess { tonic: 9, minor: true, root: false });
        assert_eq!(song_key("F# major"), KeyGuess { tonic: 6, minor: false, root: false });
        assert_eq!(song_key("Eb dorian"), KeyGuess { tonic: 3, minor: true, root: false });
    }

    #[test]
    fn preset_names_are_safe_file_names() {
        assert_eq!(safe_name("Big: room / verb?"), "Big_ room _ verb_");
        assert_eq!(safe_name("  ..."), "Preset");
    }
}
