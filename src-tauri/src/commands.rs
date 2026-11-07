//! Tauri commands: the UI's only way into the project and the engine.
//!
//! Anything slow (decoding, rendering, saving) is an async command and does
//! the heavy work before taking the session lock, so faders stay responsive.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use soniq_engine::{ClipState, DeviceInfo, DeviceOptions, EngineConfig, EngineStatus, InputStatus};
use soniq_io::{AudioBuffer, EditOp, Encoding, PEAK_STEP};
use soniq_project::{Asset, Clip, ClipContent, Op, Track, TrackKind, new_id};
use tauri::ipc::{Channel, Response};
use tauri::{AppHandle, Manager, State};

use crate::files::{self, AUTOSAVE_FILE, SaveMode};
use crate::session::{AppState, MidiTake, Pool, PoolAsset, ProjectReply, Session, lock};
use crate::{demo, settings, templates};

type CmdResult<T> = Result<T, String>;

// ---------- Project ----------

#[tauri::command]
pub fn project_get(state: State<'_, AppState>) -> ProjectReply {
    state.reply(&lock(&state.session))
}

#[tauri::command]
pub fn project_apply(state: State<'_, AppState>, op: Op) -> CmdResult<ProjectReply> {
    let mut session = lock(&state.session);
    let session = &mut *session;
    let applied = soniq_project::apply(&mut session.project, &mut session.history, op).map_err(|e| e.to_string())?;
    session.mark_changed();
    session.collect_garbage();
    state.sync_engine(session);
    let mut reply = state.reply(session);
    reply.created = applied.created;
    Ok(reply)
}

#[tauri::command]
pub fn project_undo(state: State<'_, AppState>) -> ProjectReply {
    let mut session = lock(&state.session);
    let session = &mut *session;
    if session.history.undo(&mut session.project).is_some() {
        session.mark_changed();
        state.sync_engine(session);
    }
    state.reply(session)
}

#[tauri::command]
pub fn project_redo(state: State<'_, AppState>) -> ProjectReply {
    let mut session = lock(&state.session);
    let session = &mut *session;
    if session.history.redo(&mut session.project).is_some() {
        session.mark_changed();
        state.sync_engine(session);
    }
    state.reply(session)
}

/// A new song from a template (`empty` without one).
#[tauri::command]
pub fn project_new(state: State<'_, AppState>, template: Option<String>) -> CmdResult<ProjectReply> {
    let project = templates::build(template.as_deref().unwrap_or("empty"), &state.dirs.templates())?;
    Ok(state.replace_session(Session::new(project, Pool::default(), None)))
}

#[tauri::command]
pub fn templates_list(state: State<'_, AppState>) -> Vec<templates::TemplateInfo> {
    templates::list(&state.dirs.templates())
}

/// Saves the song's setup (tracks, devices, routing, tempo; no clips or audio) as a template.
#[tauri::command]
pub fn template_save(state: State<'_, AppState>, name: String) -> CmdResult<templates::TemplateInfo> {
    let mut project = lock(&state.session).project.clone();
    state.plugins.capture(&mut project);
    templates::save(&state.dirs.templates(), &name, &project)
}

#[tauri::command]
pub fn template_delete(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    templates::delete(&state.dirs.templates(), &id)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentEntry {
    #[serde(flatten)]
    pub project: settings::RecentProject,
    /// The file is still there.
    pub exists: bool,
}

/// The songs opened or saved lately, newest first.
#[tauri::command]
pub fn recent_projects(state: State<'_, AppState>) -> Vec<RecentEntry> {
    settings::load_recent(&state.dirs.config)
        .into_iter()
        .map(|project| RecentEntry { exists: Path::new(&project.path).is_file(), project })
        .collect()
}

#[tauri::command]
pub fn recent_forget(state: State<'_, AppState>, path: String) -> CmdResult<Vec<RecentEntry>> {
    settings::forget_recent(&state.dirs.config, &path)?;
    Ok(recent_projects(state))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryList {
    /// Steps that can be undone, oldest first.
    pub past: Vec<String>,
    /// Steps that can be redone, next first.
    pub future: Vec<String>,
}

#[tauri::command]
pub fn history_list(state: State<'_, AppState>) -> HistoryList {
    let (past, future) = lock(&state.session).history.labels();
    HistoryList { past, future }
}

/// Undoes (`steps` below zero) or redoes that many steps at once.
#[tauri::command]
pub fn history_jump(state: State<'_, AppState>, steps: i64) -> ProjectReply {
    let mut session = lock(&state.session);
    let session = &mut *session;
    let mut moved = false;
    for _ in 0..steps.unsigned_abs() {
        let step = if steps < 0 { session.history.undo(&mut session.project) } else { session.history.redo(&mut session.project) };
        if step.is_none() {
            break;
        }
        moved = true;
    }
    if moved {
        session.mark_changed();
        state.sync_engine(session);
    }
    state.reply(session)
}

/// Looks through `folder` and the folders in it for the song's missing
/// audio, by file name, and plays what it finds.
#[tauri::command]
pub async fn assets_find(state: State<'_, AppState>, folder: String) -> CmdResult<ProjectReply> {
    let wanted: Vec<(String, String)> = {
        let session = lock(&state.session);
        session
            .project
            .assets
            .iter()
            .filter(|a| a.missing)
            .filter_map(|a| Some((a.id.clone(), Path::new(a.path.as_deref()?).file_name()?.to_string_lossy().to_lowercase())))
            .collect()
    };
    if wanted.is_empty() {
        return Err("No audio is missing from this song.".into());
    }
    let found = files::find_by_name(Path::new(&folder), wanted.iter().map(|(_, name)| name.as_str()));
    let mut located = Vec::new();
    for (id, name) in &wanted {
        if let Some(path) = found.get(name) {
            located.push((id.clone(), path.clone(), files::import_file(path, state.engine.sample_rate())));
        }
    }
    let count = located.iter().filter(|(.., r)| r.is_ok()).count();
    let mut reply = relink(&state, located);
    reply.note = Some(format!("Found {count} of {} missing {}", wanted.len(), if wanted.len() == 1 { "file" } else { "files" }));
    Ok(reply)
}

/// Points one missing (or any) asset at a file the user picked.
#[tauri::command]
pub async fn asset_locate(state: State<'_, AppState>, asset: String, path: String) -> CmdResult<ProjectReply> {
    let path = PathBuf::from(path);
    let result = files::import_file(&path, state.engine.sample_rate());
    if let Err(err) = &result {
        return Err(err.clone());
    }
    Ok(relink(&state, vec![(asset, path, result)]))
}

/// A missing asset's id, the file found for it, and that file decoded.
type Located = (String, PathBuf, Result<(Asset, PoolAsset), String>);

/// Puts found audio in place of missing assets: their files, and the pool.
fn relink(state: &AppState, located: Vec<Located>) -> ProjectReply {
    let mut session = lock(&state.session);
    let session = &mut *session;
    let mut warnings = Vec::new();
    for (id, path, result) in located {
        let Ok((found, pooled)) = result else { continue };
        let Some(asset) = session.project.assets.iter_mut().find(|a| a.id == id) else { continue };
        if found.frames != asset.frames && asset.frames > 0 {
            warnings.push(format!("{} is a different length than the song expects: check where its clips start and end", path.display()));
        }
        (asset.path, asset.missing, asset.frames, asset.sample_rate, asset.channels) =
            (Some(path.to_string_lossy().into_owned()), false, found.frames, found.sample_rate, found.channels);
        session.pool.insert(id, pooled);
    }
    session.mark_changed();
    state.sync_engine(session);
    let mut reply = state.reply(session);
    reply.warnings = warnings;
    reply
}

#[tauri::command]
pub async fn project_open_demo(state: State<'_, AppState>) -> CmdResult<ProjectReply> {
    let session = demo::session(state.engine.sample_rate());
    Ok(state.replace_session(session))
}

#[tauri::command]
pub async fn project_open(state: State<'_, AppState>, path: String) -> CmdResult<ProjectReply> {
    let path = PathBuf::from(path);
    let mut project = files::read_project(&path)?;
    let (pool, warnings) = files::load_assets(&mut project, state.engine.sample_rate());
    settings::remember_recent(&state.dirs.config, &path, &project.name);
    let mut reply = state.replace_session(Session::new(project, pool, Some(path)));
    reply.warnings = warnings;
    Ok(reply)
}

/// Saves to `path`, or to the current file when `path` is omitted.
#[tauri::command]
pub async fn project_save(state: State<'_, AppState>, path: Option<String>) -> CmdResult<ProjectReply> {
    let mut session = lock(&state.session);
    let target = match path.map(PathBuf::from).or_else(|| session.path.clone()) {
        Some(p) => p,
        None => return Err("Choose where to save the project first.".into()),
    };
    if session.path.as_ref() != Some(&target) {
        // Saving under a new name renames the song too.
        if let Some(stem) = target.file_stem().and_then(|s| s.to_str()) {
            session.project.name = stem.to_string();
        }
    }
    // Plugins keep their settings themselves: fetch them into the song first.
    state.plugins.capture(&mut session.project);
    files::save(&mut session, &target, SaveMode::Project)?;
    settings::remember_recent(&state.dirs.config, &target, &session.project.name);
    session.path = Some(target);
    session.dirty = false;
    session.autosave_pending = false;
    // The work is safe on disk now; no autosave needs to survive.
    files::remove_dir(&state.dirs.autosave());
    files::remove_dir(&state.dirs.recovered());
    Ok(state.reply(&session))
}

/// Decodes audio files and places them on the timeline. The first file goes to
/// `track` (if it is an audio track); every other file gets a new track.
#[tauri::command]
pub async fn project_import(
    state: State<'_, AppState>,
    paths: Vec<String>,
    track: Option<String>,
    start: Option<f64>,
) -> CmdResult<ProjectReply> {
    let rate = state.engine.sample_rate();
    let mut warnings = Vec::new();
    let mut imported = Vec::new();
    for path in &paths {
        match files::import_file(&PathBuf::from(path), rate) {
            Ok(result) => imported.push(result),
            Err(err) => warnings.push(err),
        }
    }

    let mut session = lock(&state.session);
    let session = &mut *session;
    if imported.is_empty() {
        let mut reply = state.reply(session);
        reply.warnings = warnings;
        return Ok(reply);
    }

    let before = session.project.clone();
    let start = start.unwrap_or(0.0).max(0.0);
    let mut target = track.filter(|id| session.project.track(id).is_some_and(|t| t.kind == TrackKind::Audio));
    let count = imported.len();
    let mut created = None;

    for (asset, pooled) in imported {
        let track_id = match target.take() {
            Some(id) => id,
            None => {
                let id = new_id("t");
                let project = &mut session.project;
                let track = Track::new(id.clone(), asset.name.clone(), TrackKind::Audio, project.next_color());
                project.tracks.push(track);
                created.get_or_insert(id.clone());
                id
            }
        };
        let clip = Clip::audio(asset.name.clone(), asset.id.clone(), start, 0.0, asset.duration());
        if let Some(track) = session.project.track_mut(&track_id) {
            track.clips.push(clip);
        }
        session.pool.insert(asset.id.clone(), pooled);
        session.project.assets.push(asset);
    }

    let label = if count == 1 { "Import audio".to_string() } else { format!("Import {count} files") };
    session.history.record(before, &label, None);
    session.mark_changed();
    state.sync_engine(session);
    let mut reply = state.reply(session);
    reply.warnings = warnings;
    reply.created = created;
    Ok(reply)
}

/// The audio behind one clip, read under the lock and used without it.
pub(crate) struct ClipAudio {
    pub native: Arc<AudioBuffer>,
    pub asset: Asset,
    pub offset: f64,
    pub duration: f64,
}

pub(crate) fn clip_audio(state: &AppState, clip: &str) -> CmdResult<ClipAudio> {
    let session = lock(&state.session);
    let (ti, ci) = session.project.find_clip(clip).ok_or("That clip no longer exists.")?;
    let ClipContent::Audio { asset, offset, duration, .. } = &session.project.tracks[ti].clips[ci].content else {
        return Err("Only audio clips can be edited this way.".into());
    };
    let pooled = session.pool.get(asset).ok_or("The audio for this clip is missing.")?;
    let meta = session.project.asset(asset).cloned().ok_or("The audio for this clip is missing.")?;
    Ok(ClipAudio { native: Arc::clone(&pooled.native), asset: meta, offset: *offset, duration: *duration })
}

impl ClipAudio {
    /// Frames of the asset for `start..end` seconds from the clip start
    /// (the whole clip when omitted), optionally snapped to zero crossings.
    pub fn range(&self, start: Option<f64>, end: Option<f64>, zero_cross: bool) -> Range<usize> {
        let rate = f64::from(self.native.sample_rate);
        let from = start.unwrap_or(0.0).clamp(0.0, self.duration);
        let to = end.unwrap_or(self.duration).clamp(from, self.duration);
        let frame = |sec: f64| ((self.offset + sec) * rate).round() as usize;
        let (mut a, mut b) = (frame(from), frame(to));
        if zero_cross {
            let radius = (rate * 0.01) as usize;
            a = soniq_io::snap_to_zero_crossing(&self.native, a, radius);
            b = soniq_io::snap_to_zero_crossing(&self.native, b, radius).max(a);
        }
        a..b
    }

    pub fn seconds(&self, frames: usize) -> f64 {
        frames as f64 / f64::from(self.native.sample_rate.max(1))
    }
}

/// Points the clip at freshly edited audio (a new asset, so undo restores the
/// old one), optionally with a new duration.
pub(crate) fn replace_clip_audio(
    state: &AppState,
    clip: &str,
    source: &ClipAudio,
    edited: AudioBuffer,
    new_duration: Option<f64>,
    label: &str,
) -> CmdResult<ProjectReply> {
    let mut new_asset = source.asset.clone();
    new_asset.id = new_id("a");
    new_asset.path = None;
    new_asset.frames = edited.frames() as u64;
    let pooled = PoolAsset::prepare(edited, state.engine.sample_rate());

    let mut session = lock(&state.session);
    let session = &mut *session;
    let (ti, ci) = session.project.find_clip(clip).ok_or("That clip no longer exists.")?;
    let before = session.project.clone();
    if let ClipContent::Audio { asset, duration, fade_in, fade_out, .. } = &mut session.project.tracks[ti].clips[ci].content {
        if *asset != source.asset.id {
            return Err("The clip changed while editing; try again.".into());
        }
        *asset = new_asset.id.clone();
        if let Some(d) = new_duration {
            *duration = d.max(0.001);
            let total = *fade_in + *fade_out;
            if total > *duration {
                *fade_in *= *duration / total;
                *fade_out *= *duration / total;
            }
        }
    }
    session.pool.insert(new_asset.id.clone(), pooled);
    session.project.assets.push(new_asset);
    session.project.prune_assets();
    session.history.record(before, label, None);
    session.mark_changed();
    session.collect_garbage();
    state.sync_engine(session);
    Ok(state.reply(session))
}

/// Applies an Audacity-style edit to an audio clip. `start`/`end` are seconds
/// from the clip start; without them the whole clip is edited.
#[tauri::command]
pub async fn clip_edit(
    state: State<'_, AppState>,
    clip: String,
    op: EditOp,
    start: Option<f64>,
    end: Option<f64>,
    zero_cross: Option<bool>,
) -> CmdResult<ProjectReply> {
    let source = clip_audio(&state, &clip)?;
    let range = source.range(start, end, zero_cross.unwrap_or(false));
    if let EditOp::Loudness(_) = op
        && soniq_io::integrated_loudness(&soniq_io::extract(&source.native, range.clone())).is_none()
    {
        return Err("Too short or too quiet to measure loudness: select at least half a second of sound.".into());
    }
    let (edited, clipped) = soniq_io::apply_edit(&source.native, op, range);
    let mut reply = replace_clip_audio(&state, &clip, &source, edited, None, op.label())?;
    reply.clipped = clipped;
    Ok(reply)
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipboardAction {
    Copy,
    Cut,
    Delete,
    Paste,
}

/// Cut, copy, delete or paste audio inside a clip. Delete and cut shorten the
/// clip; paste lengthens it, replacing the selection if there is one.
#[tauri::command]
pub async fn clip_clipboard(
    state: State<'_, AppState>,
    clip: String,
    action: ClipboardAction,
    start: Option<f64>,
    end: Option<f64>,
    zero_cross: Option<bool>,
) -> CmdResult<ProjectReply> {
    let source = clip_audio(&state, &clip)?;
    let range = source.range(start, end, zero_cross.unwrap_or(false));
    let removed = source.seconds(range.len());

    if matches!(action, ClipboardAction::Copy | ClipboardAction::Cut) {
        if range.is_empty() {
            return Err("Select some audio first.".into());
        }
        lock(&state.session).clipboard = Some(Arc::new(soniq_io::extract(&source.native, range.clone())));
    }
    match action {
        ClipboardAction::Copy => Ok(state.reply(&lock(&state.session))),
        ClipboardAction::Cut | ClipboardAction::Delete => {
            if range.is_empty() {
                return Err("Select some audio first.".into());
            }
            if removed >= source.duration - 0.001 {
                return Err("That would leave the clip empty; delete the clip instead.".into());
            }
            let edited = soniq_io::delete_range(&source.native, range);
            let label = if matches!(action, ClipboardAction::Cut) { "Cut" } else { "Delete audio" };
            replace_clip_audio(&state, &clip, &source, edited, Some(source.duration - removed), label)
        }
        ClipboardAction::Paste => {
            let pasted = lock(&state.session).clipboard.clone().ok_or("Nothing to paste: copy or cut some audio first.")?;
            let edited = soniq_io::insert(&source.native, range.start, range.len(), &pasted);
            let added = pasted.duration();
            replace_clip_audio(&state, &clip, &source, edited, Some(source.duration - removed + added), "Paste")
        }
    }
}

/// Waveform overview of an asset as raw little-endian bytes:
/// `[channels, step, sample_rate, peaks_per_channel]` as u32, then f32 peaks
/// channel after channel.
#[tauri::command]
pub fn asset_peaks(state: State<'_, AppState>, id: String) -> CmdResult<Response> {
    let session = lock(&state.session);
    let pooled = session.pool.get(&id).ok_or("No audio with that id.")?;
    let peaks = std::sync::Arc::clone(&pooled.peaks);
    let rate = pooled.native.sample_rate;
    drop(session);

    let count = peaks.first().map_or(0, Vec::len);
    let mut bytes = Vec::with_capacity(16 + peaks.len() * count * 4);
    for v in [peaks.len() as u32, PEAK_STEP as u32, rate, count as u32] {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    for ch in peaks.iter() {
        for p in ch {
            bytes.extend_from_slice(&p.to_le_bytes());
        }
    }
    Ok(Response::new(bytes))
}

// ---------- Recovery ----------

#[tauri::command]
pub fn recovery_available(state: State<'_, AppState>) -> bool {
    state.recovery_available.load(Ordering::Relaxed)
}

#[tauri::command]
pub async fn recovery_restore(state: State<'_, AppState>) -> CmdResult<ProjectReply> {
    let file = state.dirs.recovered().join(AUTOSAVE_FILE);
    let mut project = files::read_project(&file)?;
    let (pool, warnings) = files::load_assets(&mut project, state.engine.sample_rate());
    let mut session = Session::new(project, pool, None);
    // Restored work is unsaved until the user saves it somewhere.
    session.mark_changed();
    let mut reply = state.replace_session(session);
    reply.warnings = warnings;
    state.recovery_available.store(false, Ordering::Relaxed);
    Ok(reply)
}

#[tauri::command]
pub fn recovery_discard(state: State<'_, AppState>) {
    files::remove_dir(&state.dirs.recovered());
    state.recovery_available.store(false, Ordering::Relaxed);
}

/// Called by the UI right before the window closes on purpose, after the user
/// saved or chose to discard their changes.
#[tauri::command]
pub fn app_closing(state: State<'_, AppState>) {
    files::remove_dir(&state.dirs.autosave());
    files::remove_dir(&state.dirs.recovered());
}

/// Every 30 s, writes unsaved changes to the app data folder.
pub fn spawn_autosave(app: AppHandle) {
    std::thread::Builder::new()
        .name("soniq-autosave".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(30));
                let state = app.state::<AppState>();
                let mut session = lock(&state.session);
                if !session.autosave_pending {
                    continue;
                }
                let dir = state.dirs.autosave();
                state.plugins.capture(&mut session.project);
                let result = std::fs::create_dir_all(&dir)
                    .map_err(|e| e.to_string())
                    .and_then(|_| files::save(&mut session, &dir.join(AUTOSAVE_FILE), SaveMode::Autosave));
                match result {
                    Ok(()) => session.autosave_pending = false,
                    Err(err) => eprintln!("autosave failed: {err}"),
                }
            }
        })
        .expect("failed to start the autosave thread");
}

// ---------- Engine ----------

#[tauri::command]
pub fn engine_status(state: State<'_, AppState>) -> EngineStatus {
    state.engine.status()
}

#[tauri::command]
pub fn engine_config(state: State<'_, AppState>) -> EngineConfig {
    lock(&state.engine_config).clone()
}

/// Device enumeration can take a moment on some drivers, so it runs off the main thread.
#[tauri::command]
pub async fn audio_devices() -> Vec<DeviceInfo> {
    soniq_engine::list_devices()
}

#[tauri::command]
pub async fn audio_device_options(host: Option<String>, output: Option<String>) -> CmdResult<DeviceOptions> {
    soniq_engine::device_options(host.as_deref(), output.as_deref())
}

/// Reopens the audio device with new settings and remembers them.
#[tauri::command]
pub async fn engine_configure(state: State<'_, AppState>, config: EngineConfig) -> CmdResult<EngineStatus> {
    let previous = std::mem::replace(&mut *lock(&state.engine_config), config.clone());
    match apply_engine(&state, &config) {
        Ok(status) => {
            settings::save(&state.dirs.config, &config)?;
            Ok(status)
        }
        Err(err) => {
            // Fall back to what worked before so the user is never left silent.
            *lock(&state.engine_config) = previous.clone();
            let _ = apply_engine(&state, &previous);
            Err(err)
        }
    }
}

/// A live spectrum of what `device` puts out (its panel is open), or none.
#[tauri::command]
pub fn device_spectrum(state: State<'_, AppState>, device: Option<String>) {
    let consumer = state.engine.set_tap(device.clone());
    *lock(&state.tap) = device.zip(consumer).map(|(id, c)| (id, c, soniq_io::SpectrumMeter::new(state.engine.sample_rate())));
}

/// The safety limiter before the speakers, on or off (at once, and remembered).
#[tauri::command]
pub fn engine_safety(state: State<'_, AppState>, on: bool) -> CmdResult<()> {
    state.engine.set_safety(on);
    let mut config = lock(&state.engine_config);
    config.safety_limiter_off = !on;
    settings::save(&state.dirs.config, &config)
}

/// (Re)opens the output for `config` (the chosen device, or the default if it
/// is missing) and brings everything along: the input and its monitoring,
/// the plugins' and the pool's sample rate, and the song.
pub(crate) fn apply_engine(state: &AppState, config: &EngineConfig) -> CmdResult<EngineStatus> {
    let old_rate = state.engine.sample_rate();
    let status = state.engine.start(config)?;
    // A new stream: the live spectrum (if any) needs its queue again.
    let watched = lock(&state.tap).as_ref().map(|(id, ..)| id.clone());
    if let Some(id) = watched {
        let consumer = state.engine.set_tap(Some(id.clone()));
        *lock(&state.tap) = consumer.map(|c| (id, c, soniq_io::SpectrumMeter::new(status.sample_rate)));
    }
    // An open input follows the new settings (unless a take is running); the
    // new output stream also needs a fresh monitoring feed.
    if state.recorder.is_open() && !state.recorder.is_recording() {
        state.close_input();
        if let Err(err) = state.open_input() {
            eprintln!("could not reopen the input: {err}");
        }
    }

    if status.sample_rate != old_rate {
        state.plugins.set_rate(f64::from(status.sample_rate));
    }
    let mut session = lock(&state.session);
    if status.sample_rate != old_rate {
        session.pool.resample_all(status.sample_rate);
    }
    state.sync_engine(&session);
    Ok(status)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransportState {
    pub playing: bool,
    pub position_seconds: f64,
    pub bpm: f64,
    pub beats_per_bar: u32,
    pub metronome: bool,
}

#[tauri::command]
pub fn transport_state(state: State<'_, AppState>) -> TransportState {
    let t = state.engine.transport();
    TransportState {
        playing: t.is_playing(),
        position_seconds: t.position_seconds(),
        bpm: t.bpm(),
        beats_per_bar: t.beats_per_bar(),
        metronome: t.metronome_enabled(),
    }
}

#[tauri::command]
pub fn transport_play(state: State<'_, AppState>, playing: bool) {
    state.engine.transport().set_playing(playing);
}

#[tauri::command]
pub fn transport_seek(state: State<'_, AppState>, seconds: f64) {
    state.engine.transport().seek_seconds(seconds);
}

#[tauri::command]
pub fn transport_set_metronome(state: State<'_, AppState>, enabled: bool) {
    state.engine.transport().set_metronome(enabled);
}

#[tauri::command]
pub fn transport_set_loop(state: State<'_, AppState>, enabled: bool, start: f64, end: f64) {
    state.engine.transport().set_loop(enabled, start, end);
}

// ---------- Recording ----------

#[tauri::command]
pub fn input_status(state: State<'_, AppState>) -> InputStatus {
    state.recorder.status()
}

/// Starts recording onto every armed track from the playhead: audio from the
/// input, MIDI from the keyboards, and the session's clips into the
/// arrangement. With a count-in, one bar of clicks plays before the playhead
/// moves. With a punch range (seconds), only what is played inside it is kept.
#[tauri::command]
pub fn record_start(
    state: State<'_, AppState>,
    count_in: bool,
    punch: Option<(f64, f64)>,
    midi_mode: Option<crate::midi::MidiRecordMode>,
    pass: Option<(f64, f64)>,
) -> CmdResult<()> {
    let (audio, midi, bpm, beats_per_bar, session_clips) = {
        let session = lock(&state.session);
        let project = &session.project;
        let armed = |kind: TrackKind| -> Vec<String> { project.tracks.iter().filter(|t| t.arm && t.kind == kind).map(|t| t.id.clone()).collect() };
        let session_clips = project.tracks.iter().any(|t| t.clips.iter().any(|c| c.slot.is_some()));
        // The count-in plays at the tempo where recording starts.
        let tempo = project.tempo_map();
        let bpm = tempo.bpm_at(tempo.beats_at(state.engine.transport().position_seconds()));
        (armed(TrackKind::Audio), armed(TrackKind::Midi), bpm, project.beats_per_bar, session_clips)
    };
    if audio.is_empty() && midi.is_empty() && !session_clips {
        return Err("Arm a track first: press the record dot on its header.".into());
    }
    if !audio.is_empty() {
        if !state.recorder.is_open() {
            state.open_input()?;
        }
        state.recorder.start(&state.dirs.recordings())?;
    }
    *lock(&state.record_tracks) = audio;
    let pass = pass.filter(|(a, b)| b > a);
    *lock(&state.punch) = punch.filter(|(a, b)| b > a && pass.is_none());
    *lock(&state.pass) = pass;
    let transport = state.engine.transport();
    // With the loop on (and no punch), every pass round the loop is recorded into it.
    let (looping, loop_start, loop_end) = transport.loop_range();
    let loop_range = (looping && punch.is_none() && loop_end > loop_start).then_some((loop_start, loop_end));
    *lock(&state.midi_take) = (!midi.is_empty()).then(|| MidiTake {
        start: transport.position_seconds(),
        tracks: midi,
        events: Vec::new(),
        loop_range,
        mode: midi_mode.unwrap_or_default(),
    });
    state.begin_session_take();

    if count_in && !transport.is_playing() {
        let samples = f64::from(beats_per_bar) * 60.0 / bpm * f64::from(state.engine.sample_rate());
        transport.start_count_in(samples.round() as u64);
    }
    transport.set_playing(true);
    Ok(())
}

/// Picks a track's input channels ("1", "2", "1/2", …) out of a take.
fn input_channels(take: &AudioBuffer, input: &str) -> Vec<Vec<f32>> {
    let pick = |i: usize| take.channels.get(i).or(take.channels.first()).cloned().unwrap_or_default();
    let wanted: Vec<usize> = input.split('/').filter_map(|s| s.trim().parse::<usize>().ok()).map(|n| n.saturating_sub(1)).collect();
    if wanted.is_empty() { vec![pick(0)] } else { wanted.into_iter().map(pick).collect() }
}

/// Where a take belongs on the timeline: (start, seconds skipped at the front
/// of the take, seconds kept). The input hears the speakers late by the
/// round-trip latency, so the take moves earlier by that much; a punch range
/// keeps only its inside. `None` if nothing is left.
pub(crate) fn place_take(start: f64, length: f64, latency: f64, punch: Option<(f64, f64)>) -> Option<(f64, f64, f64)> {
    let mut start = start - latency;
    let mut skip = 0.0;
    let mut keep = length;
    if start < 0.0 {
        skip = -start;
        keep -= skip;
        start = 0.0;
    }
    if let Some((from, to)) = punch {
        if start < from {
            skip += from - start;
            keep -= from - start;
            start = from;
        }
        keep = keep.min(to - start);
    }
    (keep > 0.01).then_some((start, skip, keep))
}

/// Sketch mode: keeps one pass round the loop `a..b` (seconds). The
/// transport jumps from `b` back to `a` while it records, so the pass starts
/// at once if recording began at the loop's start (after a count-in), or at
/// the first jump back if it began part way round. A pass cut short keeps
/// what there is. `None` if nothing of a pass was recorded.
pub(crate) fn place_pass(start: f64, length: f64, latency: f64, (a, b): (f64, f64)) -> Option<(f64, f64, f64)> {
    let heard = start - latency;
    let skip = if heard <= a + 0.005 { a - heard } else { b - heard }.max(0.0);
    let keep = (b - a).min(length - skip);
    (keep > 0.05).then_some((a, skip, keep))
}

/// Stops recording and turns the takes into one clip per armed track.
#[tauri::command]
pub async fn record_stop(state: State<'_, AppState>, keep_playing: bool) -> CmdResult<ProjectReply> {
    let transport = state.engine.transport();
    let take = state.recorder.stop(transport)?;
    let midi_take = lock(&state.midi_take).take();
    let stop_seconds = transport.position_seconds();
    let stop_sample = transport.position_samples();
    if !keep_playing {
        transport.set_playing(false);
    }
    let track_ids = std::mem::take(&mut *lock(&state.record_tracks));
    let punch = lock(&state.punch).take();
    let pass = lock(&state.pass).take();
    let tempo = lock(&state.session).project.tempo_map();

    let mut warnings = Vec::new();
    let audio = match take {
        Some(take) => audio_take_clips(&state, take, &track_ids, punch, pass, &mut warnings)?,
        None => Vec::new(),
    };
    let midi = midi_take.as_ref().and_then(|t| crate::midi::midi_take_notes(t, stop_seconds, &tempo, punch).map(|found| (t.tracks.clone(), t.mode, found)));

    let mut session = lock(&state.session);
    let session = &mut *session;
    let before = session.project.clone();
    // The session's clips go in first; recorded takes land on top.
    let placed = state.end_session_take(&mut session.project, stop_sample);
    if audio.is_empty() && midi.is_none() && placed == 0 {
        let mut reply = state.reply(session);
        reply.warnings = warnings;
        return Ok(reply);
    }
    for (track_id, asset, pooled, clip) in audio {
        if let Some(track) = session.project.track_mut(&track_id) {
            track.clips.push(clip);
        }
        session.pool.insert(asset.id.clone(), pooled);
        session.project.assets.push(asset);
    }
    if let Some((tracks, mode, (start, length, notes))) = midi {
        for id in tracks {
            crate::midi::place_midi_take(&mut session.project, &id, start, length, notes.clone(), mode);
        }
    }
    session.history.record(before, "Record", None);
    session.mark_changed();
    state.sync_engine(session);
    let mut reply = state.reply(session);
    reply.warnings = warnings;
    Ok(reply)
}

/// Splits an audio take into one file and clip per armed track.
fn audio_take_clips(
    state: &AppState,
    take: soniq_engine::Take,
    track_ids: &[String],
    punch: Option<(f64, f64)>,
    pass: Option<(f64, f64)>,
    warnings: &mut Vec<String>,
) -> CmdResult<Vec<(String, Asset, PoolAsset, Clip)>> {
    let raw = soniq_io::decode_file(&take.path).map_err(|e| e.to_string())?;
    // What was heard came late by the device and by the plugins' compensation too.
    let latency = state.recorder.latency() + state.engine.output_latency() + state.engine.plugin_latency() + lock(&state.engine_config).record_offset_ms / 1000.0;
    let placed = match pass {
        Some(range) => place_pass(take.start_seconds, raw.duration(), latency, range),
        None => place_take(take.start_seconds, raw.duration(), latency, punch),
    };
    let Some((start_sec, skip, keep)) = placed else {
        let _ = std::fs::remove_file(&take.path);
        warnings.push(if pass.is_some() { "The recording stopped before the loop came round to its start." } else { "Nothing was recorded inside the punch range." }.into());
        return Ok(Vec::new());
    };

    // Where the per-track files go: the project's Samples folder once it has one.
    let (dir, plan, tempo) = {
        let session = lock(&state.session);
        let dir = session.path.as_ref().and_then(|p| p.parent()).map_or_else(|| state.dirs.recordings(), |p| p.join("Samples"));
        let plan: Vec<(String, String, String, usize)> = track_ids
            .iter()
            .filter_map(|id| session.project.track(id))
            .map(|t| (t.id.clone(), t.name.clone(), t.input.clone(), t.clips.len() + 1))
            .collect();
        (dir, plan, session.project.tempo_map())
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;

    let rate = state.engine.sample_rate();
    let mut made = Vec::new();
    for (track_id, track_name, input, number) in plan {
        let buffer = AudioBuffer::new(raw.sample_rate, input_channels(&raw, &input));
        let name = format!("{track_name} take {number}");
        let asset_id = new_id("a");
        let file = dir.join(format!("{}-{}.wav", name.replace(|c: char| !c.is_alphanumeric() && c != ' ', "_"), &asset_id[asset_id.len() - 4..]));
        soniq_io::write_wav(&file, &buffer).map_err(|e| e.to_string())?;
        let asset = Asset {
            id: asset_id,
            name,
            channels: buffer.channel_count() as u16,
            sample_rate: buffer.sample_rate,
            frames: buffer.frames() as u64,
            path: Some(file.to_string_lossy().into_owned()),
            missing: false,
        };
        made.push((track_id, asset, PoolAsset::prepare(buffer, rate)));
    }
    // The per-track files hold everything now; the raw take is no longer needed.
    let _ = std::fs::remove_file(&take.path);

    // The files keep the whole take; the clip shows the part that belongs
    // (drag its edges to reveal the rest).
    let start = tempo.beats_at(start_sec);
    Ok(made
        .into_iter()
        .map(|(track_id, asset, pooled)| {
            let clip = Clip::audio(asset.name.clone(), asset.id.clone(), start, skip, keep.min(asset.duration() - skip));
            (track_id, asset, pooled, clip)
        })
        .collect())
}

fn orphaned_takes(state: &AppState) -> Vec<PathBuf> {
    std::fs::read_dir(state.dirs.recordings())
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.to_string_lossy().ends_with(soniq_engine::TAKE_SUFFIX))
                .collect()
        })
        .unwrap_or_default()
}

/// Takes left behind when the app stopped mid-recording.
#[tauri::command]
pub fn recordings_orphaned(state: State<'_, AppState>) -> Vec<String> {
    if state.recorder.is_recording() {
        return Vec::new();
    }
    orphaned_takes(&state).iter().map(|p| p.display().to_string()).collect()
}

/// Renames orphaned takes to ordinary WAV files and returns their paths for import.
#[tauri::command]
pub fn recordings_recover(state: State<'_, AppState>) -> Vec<String> {
    orphaned_takes(&state)
        .into_iter()
        .filter_map(|p| {
            let name = p.file_name()?.to_string_lossy().replace(soniq_engine::TAKE_SUFFIX, " (recovered).wav");
            let target = p.with_file_name(name);
            std::fs::rename(&p, &target).ok()?;
            Some(target.display().to_string())
        })
        .collect()
}

#[tauri::command]
pub fn recordings_discard(state: State<'_, AppState>) {
    for p in orphaned_takes(&state) {
        let _ = std::fs::remove_file(p);
    }
}

// ---------- Export ----------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    pub path: String,
    /// Format, bit depth or bitrate, and tags.
    #[serde(flatten)]
    pub encoding: Encoding,
    /// `None` keeps the engine rate.
    pub sample_rate: Option<u32>,
    /// Range in seconds.
    pub start: f64,
    pub end: f64,
    /// Integrated loudness target in LUFS, if any.
    pub loudness: Option<f64>,
    /// Export only this track (a stem), at its own volume and pan.
    #[serde(default)]
    pub track: Option<String>,
    /// Chapters from the markers, written into the file.
    #[serde(default)]
    pub chapters: bool,
    /// Also a chapters file next to it.
    #[serde(default)]
    pub chapters_file: Option<crate::podcast::ChaptersFile>,
    /// Reach the loudness target with a limiter (peaks held at −1 dBFS)
    /// instead of stopping short of it.
    #[serde(default)]
    pub limit: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub path: String,
    pub seconds: f64,
    /// Integrated loudness of the file as written.
    pub loudness: Option<f64>,
    pub peak_db: f64,
    pub gain_db: f64,
    /// The loudness target was not reached, to keep peaks below −1 dBFS.
    pub held_back: bool,
    /// The limiter held the peaks so the target could be reached.
    pub limited: bool,
    pub chapters: usize,
    /// Where the chapters file went, if one was asked for.
    pub chapters_file: Option<String>,
}

/// Renders the mix faster than real time and writes it to disk.
#[tauri::command]
pub async fn export_mix(state: State<'_, AppState>, options: ExportOptions) -> CmdResult<ExportReport> {
    if options.end <= options.start {
        return Err("There is nothing to export in that range.".into());
    }
    let (graph, bpm, beats_per_bar, chapters, title) = {
        let session = lock(&state.session);
        let chapters = if options.chapters { crate::podcast::chapters(&session.project, options.start, options.end) } else { Vec::new() };
        let title = if options.encoding.metadata.title.trim().is_empty() { session.project.name.clone() } else { options.encoding.metadata.title.clone() };
        let mut graph = state.build_graph(&session, false);
        if let Some(id) = &options.track {
            let tracks = &session.project.tracks;
            let only = tracks.iter().position(|t| &t.id == id).ok_or("That track no longer exists.")?;
            // A stem plays its track alone, whatever is muted or soloed,
            // through its own effects but dry of the returns and the master's chain.
            for (i, node) in graph.tracks.iter_mut().enumerate() {
                node.gain = if i == only { soniq_engine::track_gains(tracks[i].volume, tracks[i].pan, true) } else { [0.0, 0.0] };
                node.sends.fill(0.0);
            }
            graph.master_fx.clear();
        }
        (graph, session.project.bpm, session.project.beats_per_bar, chapters, title)
    };
    let rate = state.engine.sample_rate();
    let mut mix = soniq_engine::render_offline(graph, rate, bpm, beats_per_bar, options.start, options.end);
    if let Some(target) = options.sample_rate.filter(|r| *r != rate) {
        mix = soniq_io::resample(&mix, target);
    }

    // Loudness target first, then keep true-ish peaks below −1 dBFS.
    const CEILING_DB: f64 = -1.0;
    let measured = soniq_io::integrated_loudness(&mix);
    let mut gain_db = match (options.loudness, measured) {
        (Some(target), Some(lufs)) => target - lufs,
        _ => 0.0,
    };
    let peak_db = 20.0 * f64::from(mix.peak().max(1e-9)).log10();
    let (mut held_back, mut limited) = (false, false);
    if options.loudness.is_some() && peak_db + gain_db > CEILING_DB {
        if options.limit {
            limited = true;
        } else {
            gain_db = CEILING_DB - peak_db;
            held_back = true;
        }
    }
    if gain_db.abs() > 0.001 {
        let g = 10f64.powf(gain_db / 20.0) as f32;
        mix.channels.iter_mut().flatten().for_each(|s| *s *= g);
    }
    if limited {
        mix = soniq_io::apply_effect(&mix, &soniq_io::Effect::Limiter { ceiling: CEILING_DB, release: 50.0 }, &Default::default())?.buffer;
    }

    let path = PathBuf::from(&options.path);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    }
    let mut encoding = options.encoding.clone();
    encoding.metadata.chapters = chapters.clone();
    soniq_io::write_export(&path, &mix, &encoding).map_err(|e| e.to_string())?;
    let chapters_file = match options.chapters_file {
        Some(format) if !chapters.is_empty() => Some(crate::podcast::write_chapters_file(&path, &chapters, format, &title)?),
        _ => None,
    };
    Ok(ExportReport {
        path: options.path,
        seconds: mix.duration(),
        loudness: if limited { soniq_io::integrated_loudness(&mix) } else { measured.map(|l| l + gain_db) },
        peak_db: if limited { 20.0 * f64::from(mix.peak().max(1e-9)).log10() } else { peak_db + gain_db },
        gain_db,
        held_back,
        limited,
        chapters: chapters.len(),
        chapters_file,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeterFrame {
    pub position: f64,
    pub playing: bool,
    pub tracks: Vec<(String, [f32; 2])>,
    pub master: [f32; 2],
    /// The audio input, while a track is armed.
    pub input: [f32; 2],
    /// Count-in still running.
    pub counting_in: bool,
    /// An effect preview is playing.
    pub previewing: bool,
    /// Counts live notes; a change means MIDI came in.
    pub midi: u64,
    /// Changes when a MIDI controller changed the project: time to fetch it.
    pub revision: u64,
    /// What each track's session plays, for tracks where that is anything.
    pub clips: Vec<(String, ClipState)>,
    /// Loudness, peaks and the spectrum of the master, while asked for.
    pub analysis: Option<soniq_io::Reading>,
    /// How far the safety limiter turned the master down since the last frame, in dB.
    pub limiter: f32,
    /// Gain reduction of dynamics devices (dB), for those turning anything down.
    pub devices: Vec<(String, f32)>,
    /// The live spectrum of the device whose panel asked for it.
    pub tap: Option<(String, Vec<f32>)>,
    /// A block of NaN or infinity (a broken plugin) was silenced.
    pub silenced: bool,
}

#[cfg(test)]
mod tests {
    use super::{place_pass, place_take};

    #[test]
    fn a_sketch_keeps_one_pass_round_the_loop() {
        // From the loop's start after a count-in: the pass starts at once, 20 ms in.
        let (start, skip, keep) = place_pass(0.0, 9.0, 0.02, (0.0, 8.0)).unwrap();
        assert!(start == 0.0 && (skip - 0.02).abs() < 1e-9 && keep == 8.0);
        // Part way round (at 3 s): it starts where the loop came back to 0, 5 s in.
        let (start, skip, keep) = place_pass(3.0, 14.0, 0.0, (0.0, 8.0)).unwrap();
        assert_eq!((start, skip, keep), (0.0, 5.0, 8.0));
        // Stopped half way through the pass: half a layer.
        assert_eq!(place_pass(3.0, 9.0, 0.0, (0.0, 8.0)), Some((0.0, 5.0, 4.0)));
        // Stopped before the loop came round.
        assert_eq!(place_pass(3.0, 4.0, 0.0, (0.0, 8.0)), None);
    }

    #[test]
    fn takes_move_earlier_by_the_latency() {
        let (start, skip, keep) = place_take(10.0, 5.0, 0.02, None).unwrap();
        assert!((start - 9.98).abs() < 1e-9 && skip == 0.0 && keep == 5.0);
        // A take that started at zero loses its first few milliseconds instead.
        let (start, skip, keep) = place_take(0.0, 5.0, 0.02, None).unwrap();
        assert_eq!(start, 0.0);
        assert!((skip - 0.02).abs() < 1e-9 && (keep - 4.98).abs() < 1e-9);
    }

    #[test]
    fn punch_keeps_only_the_inside() {
        // Recorded 2 s of pre-roll before a punch range of 4–6 s.
        let (start, skip, keep) = place_take(2.0, 6.0, 0.0, Some((4.0, 6.0))).unwrap();
        assert_eq!((start, skip, keep), (4.0, 2.0, 2.0));
        assert_eq!(place_take(7.0, 1.0, 0.0, Some((4.0, 6.0))), None);
    }
}

/// Starts or stops analysing the master (loudness, true peak, RMS,
/// correlation and the spectrum), which then comes with every meter frame.
#[tauri::command]
pub fn analysis_enable(state: State<'_, AppState>, on: bool) {
    let mut analyzer = lock(&state.analyzer);
    if on && analyzer.is_none() {
        *analyzer = Some(soniq_io::Analyzer::new(state.engine.sample_rate()));
        // Nothing stale from an earlier run.
        state.engine.drain_analysis(|_, _| {});
    } else if !on {
        *analyzer = None;
    }
    state.engine.set_analysing(on);
}

/// Starts the integrated loudness, the loudness range and the highest true peak over.
#[tauri::command]
pub fn analysis_reset(state: State<'_, AppState>) {
    if let Some(analyzer) = lock(&state.analyzer).as_mut() {
        analyzer.reset();
    }
}

/// Streams meter peaks and the playhead to the UI about 30 times a second.
#[tauri::command]
pub fn meters_subscribe(app: AppHandle, state: State<'_, AppState>, channel: Channel<MeterFrame>) {
    let generation = state.meter_generation.fetch_add(1, Ordering::Relaxed) + 1;
    std::thread::Builder::new()
        .name("soniq-meters".into())
        .spawn(move || {
            let state = app.state::<AppState>();
            while state.meter_generation.load(Ordering::Relaxed) == generation {
                std::thread::sleep(Duration::from_millis(33));
                state.engine.flush();
                state.collect_launched();
                let (tracks, master) = state.engine.meters().take_all();
                let t = state.engine.transport();
                let (limiter, silenced) = state.engine.take_safety();
                let tap = lock(&state.tap).as_mut().map(|(id, consumer, meter)| {
                    let ready = consumer.slots();
                    if let Ok(chunk) = consumer.read_chunk(ready) {
                        let (a, b) = chunk.as_slices();
                        meter.push(a);
                        meter.push(b);
                        chunk.commit_all();
                    }
                    (id.clone(), meter.bands())
                });
                let frame = MeterFrame {
                    devices: state.engine.meters().take_devices(),
                    tap,
                    limiter,
                    silenced,
                    position: t.position_seconds(),
                    playing: t.is_playing(),
                    tracks,
                    master,
                    input: state.recorder.meter().take(),
                    counting_in: t.count_in().1 > 0,
                    previewing: state.engine.is_previewing(),
                    midi: state.midi_activity.load(Ordering::Relaxed),
                    revision: state.revision.load(Ordering::Relaxed),
                    clips: state.engine.meters().clip_states(),
                    analysis: state.analyse(),
                };
                if channel.send(frame).is_err() {
                    break;
                }
            }
        })
        .ok();
}
