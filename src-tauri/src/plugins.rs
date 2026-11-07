//! Plugins in the app: finding them on disk (each file scanned in a
//! sandbox, the results cached), one sandbox per plugin device while the
//! song is open, their states saved with the song, and what a plugin does
//! by itself (its own window turning a knob, a crash) reaching the song.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};
use soniq_dsp::External;
use soniq_host::{Descriptor, Gesture, Message, ParamInfo, Plugin, SandboxCommand};
use soniq_project::{Device, DeviceParam, DeviceRole, MASTER, Op, PLUGIN_TYPE, PluginRef, Project};
use tauri::{AppHandle, Manager, State};

use crate::session::{AppState, ProjectReply, lock};

type CmdResult<T> = Result<T, String>;

/// The flag that starts this program as a plugin sandbox instead of the app.
pub const SANDBOX_FLAG: &str = "--plugin-sandbox";
const SETTINGS: &str = "plugins.json";
const CACHE: &str = "plugin-cache.json";
/// How long a plugin file may take to load when scanned.
const SCAN_WAIT: Duration = Duration::from_secs(20);
/// Controls a new plugin device shows before you pick others.
const FIRST_CONTROLS: usize = 8;

/// Folders the user added to the standard ones.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Settings {
    folders: Vec<String>,
}

/// What scanning a file found, remembered until the file changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Scanned {
    path: String,
    size: u64,
    modified: u64,
    plugins: Vec<Descriptor>,
    /// Why it could not be used (it crashed or hung while loading): skipped until a rescan.
    error: Option<String>,
}

/// A plugin the browser lists.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginEntry {
    /// "clap" or "vst3".
    pub format: &'static str,
    pub path: String,
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub description: String,
    pub instrument: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginList {
    pub plugins: Vec<PluginEntry>,
    /// Files that could not be loaded, and why.
    pub failed: Vec<(String, String)>,
    /// Where Soniq looks: the standard CLAP folders, then the user's.
    pub standard: Vec<String>,
    pub folders: Vec<String>,
}

/// How a plugin device's plugin is doing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginStatus {
    /// "starting", "running", "crashed" or "failed".
    pub state: &'static str,
    pub message: Option<String>,
    pub latency: u32,
    pub gui: bool,
    pub window: bool,
}

/// A device's id, filled in once the device exists.
type DeviceKey = Arc<Mutex<String>>;

pub struct Plugins {
    sandbox: SandboxCommand,
    app: OnceLock<AppHandle>,
    /// What plugins do by themselves goes to one worker, so a plugin's
    /// reply reader never waits on the song (which may be waiting on it).
    notices: OnceLock<Sender<(DeviceKey, Message)>>,
    /// Running plugins by device id. They stay while the song is open, so
    /// undoing a removal brings back the same plugin, as it was.
    running: Mutex<HashMap<String, Arc<Plugin>>>,
    starting: Mutex<HashSet<String>>,
    failed: Mutex<HashMap<String, String>>,
    windows: Mutex<HashSet<String>>,
    /// Gestures on a plugin's own window: (device, parameter) and the value before it.
    gestures: Mutex<HashMap<(String, String), f64>>,
    /// Bumped when the song is replaced, so plugins still starting for the old one are dropped.
    generation: AtomicU64,
}

/// Every standard CLAP folder that applies here (CLAP_PATH first).
fn standard_folders() -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = std::env::var_os("CLAP_PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if cfg!(windows) {
        for format in ["CLAP", "VST3"] {
            if let Some(common) = std::env::var_os("COMMONPROGRAMFILES") {
                folders.push(PathBuf::from(common).join(format));
            }
            if let Some(local) = std::env::var_os("LOCALAPPDATA") {
                folders.push(PathBuf::from(local).join("Programs").join("Common").join(format));
            }
        }
    }
    folders
}

fn has_extension(path: &Path, ext: &str) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// Plugin files under `dir`, a few folders deep: `.clap` files, and `.vst3`
/// bundles (folders) or files.
fn plugin_files(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if has_extension(&path, "vst3") || (has_extension(&path, "clap") && path.is_file()) {
            out.push(path);
        } else if path.is_dir() && depth > 0 {
            plugin_files(&path, depth - 1, out);
        }
    }
}

fn format_of(path: &str) -> &'static str {
    if has_extension(Path::new(path), "vst3") { "vst3" } else { "clap" }
}

fn stamp(path: &Path) -> (u64, u64) {
    let meta = fs::metadata(path).ok();
    let modified = meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    (meta.map_or(0, |m| m.len()), modified)
}

/// A plugin parameter as a device control.
pub fn control(p: &ParamInfo) -> DeviceParam {
    DeviceParam {
        id: p.id.to_string(),
        label: p.name.clone(),
        value: p.value,
        min: p.min,
        max: p.max.max(p.min),
        default: p.default,
        unit: String::new(),
        log: None,
        bipolar: None,
        // VST3 values run 0–1, so a stepped one moves by 1/steps.
        step: p.stepped.then(|| if p.steps > 0 { (p.max - p.min) / f64::from(p.steps) } else { 1.0 }),
        options: None,
        group: (!p.module.is_empty()).then(|| p.module.clone()),
    }
}

/// The controls a new device shows: the first parameters automation can move.
fn first_controls(params: &[ParamInfo]) -> Vec<DeviceParam> {
    params.iter().filter(|p| p.automatable && !p.hidden && !p.readonly).take(FIRST_CONTROLS).map(control).collect()
}

/// Every plugin device in the song, with the chain it sits in.
fn plugin_devices(project: &Project) -> impl Iterator<Item = (&str, &Device)> {
    let tracks = project.tracks.iter().flat_map(|t| t.devices.iter().map(move |d| (t.id.as_str(), d)));
    let returns = project.returns.iter().flat_map(|r| r.devices.iter().map(move |d| (r.id.as_str(), d)));
    let master = project.master_devices.iter().map(|d| (MASTER, d));
    tracks.chain(returns).chain(master).filter(|(_, d)| d.kind == PLUGIN_TYPE && d.plugin.is_some())
}

fn find_device_mut<'a>(project: &'a mut Project, id: &str) -> Option<&'a mut Device> {
    let tracks = project.tracks.iter_mut().flat_map(|t| t.devices.iter_mut());
    let returns = project.returns.iter_mut().flat_map(|r| r.devices.iter_mut());
    tracks.chain(returns).chain(project.master_devices.iter_mut()).find(|d| d.id == id)
}

impl Plugins {
    pub fn new() -> Self {
        let program = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("soniq-studio"));
        Self {
            sandbox: SandboxCommand { program, args: vec![SANDBOX_FLAG.into()] },
            app: OnceLock::new(),
            notices: OnceLock::new(),
            running: Mutex::default(),
            starting: Mutex::default(),
            failed: Mutex::default(),
            windows: Mutex::default(),
            gestures: Mutex::default(),
            generation: AtomicU64::new(0),
        }
    }

    pub fn set_app(&self, app: AppHandle) {
        let (tx, rx) = channel::<(DeviceKey, Message)>();
        let worker = app.clone();
        std::thread::Builder::new()
            .name("plugin notices".into())
            .spawn(move || {
                for (key, message) in rx {
                    let id = lock(&key).clone();
                    notice(&worker, &id, message);
                }
            })
            .expect("failed to start the plugin notice worker");
        let _ = self.notices.set(tx);
        let _ = self.app.set(app);
    }

    /// Hands what a plugin does by itself to the notice worker.
    fn notices(&self, key: DeviceKey) -> impl Fn(Message) + Send + 'static {
        let tx = self.notices.get().cloned();
        move |message| {
            if let Some(tx) = &tx {
                let _ = tx.send((Arc::clone(&key), message));
            }
        }
    }

    pub fn get(&self, device: &str) -> Option<Arc<Plugin>> {
        lock(&self.running).get(device).cloned()
    }

    /// The node the engine runs for a plugin device, when its plugin is
    /// running. It starts from the device's control values; automation and
    /// modulators move them from there.
    pub fn processor(&self, device: &Device, instrument: bool, live: bool) -> Option<Box<dyn External>> {
        let plugin = self.get(&device.id).filter(|p| !p.crashed())?;
        let params: Vec<(String, u32)> = device.params.iter().filter_map(|p| Some((p.id.clone(), p.id.parse().ok()?))).collect();
        let mut node = plugin.processor(&params, instrument, live);
        node.configure(&crate::build::bank(device));
        Some(Box::new(node))
    }

    /// Starts the plugins of every plugin device that has none yet.
    pub fn ensure(&self, project: &Project, rate: f64) {
        let wanted: Vec<Device> = {
            let (running, starting, failed) = (lock(&self.running), lock(&self.starting), lock(&self.failed));
            plugin_devices(project).map(|(_, d)| d).filter(|d| !running.contains_key(&d.id) && !starting.contains(&d.id) && !failed.contains_key(&d.id)).cloned().collect()
        };
        for device in wanted {
            self.start(device, rate);
        }
    }

    fn start(&self, device: Device, rate: f64) {
        let (Some(app), Some(plugin)) = (self.app.get().cloned(), device.plugin.clone()) else { return };
        lock(&self.starting).insert(device.id.clone());
        let generation = self.generation.load(Relaxed);
        let sandbox = self.sandbox.clone();
        std::thread::spawn(move || {
            let state = app.state::<AppState>();
            let plugins = &state.plugins;
            let id = device.id.clone();
            let notices = plugins.notices(Arc::new(Mutex::new(id.clone())));
            let result = Plugin::spawn(&sandbox, Path::new(&plugin.path), &plugin.id, rate, notices).and_then(|p| {
                if let Some(saved) = device.state.as_deref().and_then(|s| BASE64.decode(s).ok()) {
                    p.load_state(&saved).map_err(|e| format!("It started, but could not take its saved settings: {e}"))?;
                }
                Ok(p)
            });
            if plugins.generation.load(Relaxed) != generation {
                return;
            }
            lock(&plugins.starting).remove(&id);
            match result {
                Ok(p) => {
                    lock(&plugins.running).insert(id, Arc::new(p));
                }
                Err(message) => {
                    lock(&plugins.failed).insert(id, message);
                }
            }
            let session = lock(&state.session);
            state.sync_engine(&session);
            state.revision.fetch_add(1, Relaxed);
        });
    }

    /// Stops every plugin (a new song replaces this one).
    pub fn clear(&self) {
        self.generation.fetch_add(1, Relaxed);
        let running = std::mem::take(&mut *lock(&self.running));
        lock(&self.starting).clear();
        lock(&self.failed).clear();
        lock(&self.windows).clear();
        lock(&self.gestures).clear();
        // Plugins quit on their own threads, so a slow one does not hold the app up.
        std::thread::spawn(move || drop(running));
    }

    /// Lets a crashed or failed plugin start again (from its last saved state).
    pub fn forget(&self, device: &str) {
        let gone = lock(&self.running).remove(device);
        lock(&self.failed).remove(device);
        lock(&self.windows).remove(device);
        std::thread::spawn(move || drop(gone));
    }

    pub fn insert(&self, device: String, plugin: Plugin) {
        lock(&self.failed).remove(&device);
        lock(&self.running).insert(device, Arc::new(plugin));
    }

    /// Restarts every plugin at a new sample rate.
    pub fn set_rate(&self, rate: f64) {
        for plugin in lock(&self.running).values() {
            if let Err(e) = plugin.activate(rate) {
                eprintln!("a plugin could not change its sample rate: {e}");
            }
        }
    }

    pub fn statuses(&self, project: &Project) -> HashMap<String, PluginStatus> {
        let (running, starting, failed, windows) = (lock(&self.running), lock(&self.starting), lock(&self.failed), lock(&self.windows));
        plugin_devices(project)
            .map(|(_, d)| {
                let status = if let Some(p) = running.get(&d.id) {
                    let crashed = p.crashed();
                    PluginStatus {
                        state: if crashed { "crashed" } else { "running" },
                        message: crashed.then(|| "The plugin crashed. Soniq kept going without it.".to_string()),
                        latency: p.latency(),
                        gui: p.info.gui,
                        window: !crashed && windows.contains(&d.id),
                    }
                } else if starting.contains(&d.id) {
                    PluginStatus { state: "starting", message: None, latency: 0, gui: false, window: false }
                } else {
                    let message = failed.get(&d.id).cloned().unwrap_or_else(|| "Not started.".into());
                    PluginStatus { state: "failed", message: Some(message), latency: 0, gui: false, window: false }
                };
                (d.id.clone(), status)
            })
            .collect()
    }

    /// Writes each running plugin's state into its device, for saving.
    pub fn capture(&self, project: &mut Project) {
        let ids: Vec<String> = plugin_devices(project).map(|(_, d)| d.id.clone()).collect();
        for id in ids {
            let Some(plugin) = self.get(&id).filter(|p| !p.crashed()) else { continue };
            match plugin.save_state() {
                Ok(state) => {
                    if let Some(d) = find_device_mut(project, &id) {
                        d.state = Some(BASE64.encode(state));
                    }
                }
                Err(e) => eprintln!("could not save a plugin's state: {e}"),
            }
        }
    }

    fn settings(dirs: &crate::session::Dirs) -> Settings {
        fs::read_to_string(dirs.config.join(SETTINGS)).ok().and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default()
    }

    /// Finds the plugins in the standard folders and the user's, scanning
    /// files that are new or changed (all of them on `rescan`).
    fn list(&self, dirs: &crate::session::Dirs, rescan: bool) -> PluginList {
        let settings = Self::settings(dirs);
        let standard = standard_folders();
        let mut files = Vec::new();
        for folder in standard.iter().cloned().chain(settings.folders.iter().map(PathBuf::from)) {
            plugin_files(&folder, 4, &mut files);
        }
        files.sort();
        files.dedup();
        let cache_file = dirs.data.join(CACHE);
        let cached: Vec<Scanned> = fs::read_to_string(&cache_file).ok().and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
        let scanned: Vec<Scanned> = files
            .iter()
            .map(|file| {
                let path = file.to_string_lossy().into_owned();
                let (size, modified) = stamp(file);
                if !rescan && let Some(hit) = cached.iter().find(|c| c.path == path && c.size == size && c.modified == modified) {
                    return hit.clone();
                }
                match soniq_host::scan(&self.sandbox, file, SCAN_WAIT) {
                    Ok(plugins) => Scanned { path, size, modified, plugins, error: None },
                    Err(error) => Scanned { path, size, modified, plugins: Vec::new(), error: Some(error) },
                }
            })
            .collect();
        if let Ok(json) = serde_json::to_string(&scanned) {
            let _ = fs::write(&cache_file, json);
        }
        let mut plugins: Vec<PluginEntry> = scanned
            .iter()
            .flat_map(|s| {
                s.plugins.iter().filter(|d| !d.is_note_effect()).map(|d| PluginEntry {
                    format: format_of(&s.path),
                    path: s.path.clone(),
                    id: d.id.clone(),
                    name: d.name.clone(),
                    vendor: d.vendor.clone(),
                    version: d.version.clone(),
                    description: d.description.clone(),
                    instrument: d.is_instrument(),
                })
            })
            .collect();
        plugins.sort_by_key(|p| p.name.to_lowercase());
        PluginList {
            plugins,
            failed: scanned.iter().filter_map(|s| Some((s.path.clone(), s.error.clone()?))).collect(),
            standard: standard.iter().map(|p| p.display().to_string()).collect(),
            folders: settings.folders,
        }
    }
}

/// What a plugin did by itself, applied to the song (on the notice worker).
fn notice(app: &AppHandle, id: &str, message: Message) {
    let id = id.to_string();
    let state = app.state::<AppState>();
    let plugins = &state.plugins;
    match message {
        Message::ParamChanged { id: param, value, gesture } => {
            let param = param.to_string();
            let mut session = lock(&state.session);
            let session = &mut *session;
            let Some(d) = find_device_mut(&mut session.project, &id) else { return };
            let Some(p) = d.params.iter_mut().find(|p| p.id == param) else { return };
            let key = (id.clone(), param.clone());
            match (gesture, value) {
                (Gesture::Begin, _) => {
                    lock(&plugins.gestures).insert(key, p.value);
                    return;
                }
                (_, Some(v)) => {
                    // The control follows the plugin's window as it moves.
                    p.value = v.clamp(p.min, p.max);
                }
                (Gesture::End, None) => {
                    // Let go: one undo step from where the gesture began.
                    let Some(before) = lock(&plugins.gestures).remove(&key) else { return };
                    let after = p.value;
                    p.value = before;
                    let owner = plugin_devices(&session.project).find(|(_, d)| d.id == id).map(|(o, _)| o.to_string());
                    if let Some(track) = owner {
                        let _ = soniq_project::apply(&mut session.project, &mut session.history, Op::SetParam { track, device: id.clone(), param, value: after });
                    }
                }
                (Gesture::None, None) => return,
            }
            session.mark_changed();
            state.sync_engine(session);
        }
        Message::GuiClosed => {
            lock(&plugins.windows).remove(&id);
        }
        Message::StateChanged => lock(&state.session).mark_changed(),
        // Its latency or its parameters changed: the graph is built again around it.
        Message::LatencyChanged { .. } | Message::ParamsChanged => {
            let session = lock(&state.session);
            state.sync_engine(&session);
        }
        Message::Exited => {
            lock(&plugins.windows).remove(&id);
        }
        _ => {}
    }
    state.revision.fetch_add(1, Relaxed);
}

#[tauri::command]
pub async fn plugins_list(state: State<'_, AppState>, rescan: bool) -> CmdResult<PluginList> {
    Ok(state.plugins.list(&state.dirs, rescan))
}

#[tauri::command]
pub async fn plugins_set_folders(state: State<'_, AppState>, folders: Vec<String>) -> CmdResult<PluginList> {
    crate::settings::write_json(&state.dirs.config, SETTINGS, &Settings { folders })?;
    Ok(state.plugins.list(&state.dirs, false))
}

/// Adds a plugin to a track's (or a return's, or the master's) chain.
#[tauri::command]
pub async fn plugin_add(state: State<'_, AppState>, track: String, path: String, id: String) -> CmdResult<ProjectReply> {
    let file = PathBuf::from(&path);
    let descriptor = soniq_host::scan(&state.plugins.sandbox, &file, SCAN_WAIT)?.into_iter().find(|d| d.id == id).ok_or("That plugin is no longer in its file.")?;
    if descriptor.is_note_effect() {
        return Err("Plugins that only change notes are not supported yet.".into());
    }
    let key = Arc::new(Mutex::new(String::new()));
    let rate = f64::from(state.engine.sample_rate());
    let plugin = Plugin::spawn(&state.plugins.sandbox, &file, &id, rate, state.plugins.notices(Arc::clone(&key)))?;
    let params = plugin.params()?;
    let device = Device {
        id: soniq_project::new_id("dev"),
        kind: PLUGIN_TYPE.into(),
        name: descriptor.name.clone(),
        role: if descriptor.is_instrument() { DeviceRole::Instrument } else { DeviceRole::Effect },
        on: true,
        params: first_controls(&params),
        samples: Vec::new(),
        chains: Vec::new(),
        macros: Vec::new(),
        sidechain: None,
        modulates: None,
        plugin: Some(PluginRef { format: format_of(&path).into(), path, id, vendor: descriptor.vendor.clone() }),
        state: None,
        pad_chains: Vec::new(),
    };

    let mut session = lock(&state.session);
    let session = &mut *session;
    let before: HashSet<String> = plugin_devices(&session.project).map(|(_, d)| d.id.clone()).collect();
    soniq_project::apply(&mut session.project, &mut session.history, Op::AddDevice { track, device }).map_err(|e| e.to_string())?;
    let added = plugin_devices(&session.project).map(|(_, d)| d.id.clone()).find(|d| !before.contains(d)).ok_or("The plugin was not added.")?;
    *lock(&key) = added.clone();
    state.plugins.insert(added.clone(), plugin);
    session.mark_changed();
    state.sync_engine(session);
    let mut reply = state.reply(session);
    reply.created = Some(added);
    Ok(reply)
}

/// Opens (or brings forward) a plugin's own window, or closes it.
#[tauri::command]
pub async fn plugin_window(state: State<'_, AppState>, device: String, show: bool) -> CmdResult<ProjectReply> {
    let plugin = state.plugins.get(&device).ok_or("The plugin is not running.")?;
    if show {
        let title = {
            let session = lock(&state.session);
            let owner = plugin_devices(&session.project).find(|(_, d)| d.id == device).map(|(o, d)| (o.to_string(), d.name.clone()));
            match owner {
                Some((o, name)) if o == MASTER => format!("{name} · Master"),
                Some((o, name)) => {
                    let track = session.project.tracks.iter().find(|t| t.id == o).map(|t| t.name.clone());
                    let ret = session.project.returns.iter().find(|r| r.id == o).map(|r| r.name.clone());
                    format!("{name} · {}", track.or(ret).unwrap_or_default())
                }
                None => plugin.info.name.clone(),
            }
        };
        plugin.show_gui(&title)?;
        lock(&state.plugins.windows).insert(device);
    } else {
        plugin.hide_gui()?;
        lock(&state.plugins.windows).remove(&device);
    }
    Ok(state.reply(&lock(&state.session)))
}

/// Every parameter of a device's plugin, to pick the controls it shows.
#[tauri::command]
pub async fn plugin_params(state: State<'_, AppState>, device: String) -> CmdResult<Vec<ParamInfo>> {
    state.plugins.get(&device).ok_or("The plugin is not running.")?.params()
}

/// What the plugin writes for each control's value ("-6.0 dB").
#[tauri::command]
pub async fn plugin_texts(state: State<'_, AppState>, device: String, ids: Vec<u32>) -> CmdResult<Vec<(u32, f64, String)>> {
    state.plugins.get(&device).ok_or("The plugin is not running.")?.texts(&ids)
}

/// Starts a crashed or failed plugin again, from the state saved with the song.
#[tauri::command]
pub async fn plugin_reload(state: State<'_, AppState>, device: String) -> CmdResult<ProjectReply> {
    state.plugins.forget(&device);
    let session = lock(&state.session);
    state.sync_engine(&session);
    Ok(state.reply(&session))
}
