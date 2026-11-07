//! Commands that render new audio from old: effects and their previews,
//! noise profiles, spectral edits, the spectrogram, sample drawing,
//! generators, macros and batch processing.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use soniq_io::{AudioBuffer, Effect, EffectContext, Encoding, FrequencyScale, Generator, Processed};
use soniq_project::{Asset, Clip, Track, TrackKind, new_id};
use tauri::State;
use tauri::ipc::{Channel, Response};

use crate::commands::{ClipAudio, clip_audio, replace_clip_audio};
use crate::session::{AppState, PoolAsset, ProjectReply, lock};
use crate::settings::{self, Macro};

type CmdResult<T> = Result<T, String>;

/// The longest stretch an effect preview plays, in seconds.
const PREVIEW_SECONDS: f64 = 10.0;

fn run_effects(state: &AppState, input: &AudioBuffer, effects: &[Effect], control: Option<&AudioBuffer>) -> CmdResult<Processed> {
    let profile = lock(&state.noise_profile).clone();
    soniq_io::apply_effects(input, effects, &EffectContext { noise_profile: profile.as_ref(), control })
}

/// What Auto Duck listens to: the other track over the same stretch of the
/// song as `range` of the clip, through its devices, at the clip's rate.
fn duck_control(state: &AppState, clip: &str, effects: &[Effect], source: &ClipAudio, range: &Range<usize>) -> CmdResult<Option<AudioBuffer>> {
    let Some(track) = effects.iter().find_map(|e| match e {
        Effect::AutoDuck { track, .. } => Some(track.clone()),
        _ => None,
    }) else {
        return Ok(None);
    };
    let (graph, bpm, beats_per_bar, from, to) = {
        let session = lock(&state.session);
        let project = &session.project;
        let (ti, ci) = project.find_clip(clip).ok_or("That clip no longer exists.")?;
        if project.tracks[ti].id == track {
            return Err("Auto Duck listens to another track: pick one other than this clip's.".into());
        }
        project.track(&track).ok_or("The track to listen to no longer exists.")?;
        let c = &project.tracks[ti].clips[ci];
        let tempo = project.tempo_map();
        let rate = f64::from(source.native.sample_rate);
        let at = |frame: usize| tempo.seconds_at(c.beat_after(&tempo, c.start, frame as f64 / rate - source.offset));
        let graph = state.graph_of(&crate::freeze::alone(project, &track, None), &session.pool, false);
        (graph, project.bpm, project.beats_per_bar, at(range.start), at(range.end))
    };
    let mut audio = soniq_engine::render_offline(graph, state.engine.sample_rate(), bpm, beats_per_bar, from, to.max(from));
    if audio.sample_rate != source.native.sample_rate {
        audio = soniq_io::resample(&audio, source.native.sample_rate);
    }
    Ok(Some(audio))
}

/// Redraws a short damaged stretch (a click, a dropout) from the sound around it.
#[tauri::command]
pub async fn clip_repair(state: State<'_, AppState>, clip: String, start: f64, end: f64) -> CmdResult<ProjectReply> {
    let source = clip_audio(&state, &clip)?;
    let range = source.range(Some(start), Some(end), false);
    let edited = soniq_io::repair(&source.native, range.start, range.end)?;
    replace_clip_audio(&state, &clip, &source, edited, None, "Repair")
}

/// The pooled audio of an asset at its own rate.
fn native_audio(state: &AppState, id: &str) -> CmdResult<Arc<AudioBuffer>> {
    let session = lock(&state.session);
    session.pool.get(id).map(|p| Arc::clone(&p.native)).ok_or_else(|| "No audio with that id.".into())
}

fn header(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

// ---------- Effects ----------

/// Runs a chain of effects (one effect, or a macro) over the selection, or
/// over the whole clip without one. Effects that change the length (speed,
/// tempo, truncate silence) lengthen or shorten the clip to match.
#[tauri::command]
pub async fn clip_effect(
    state: State<'_, AppState>,
    clip: String,
    effects: Vec<Effect>,
    start: Option<f64>,
    end: Option<f64>,
    zero_cross: Option<bool>,
    label: Option<String>,
) -> CmdResult<ProjectReply> {
    let first = effects.first().ok_or("Pick an effect first.")?;
    let source = clip_audio(&state, &clip)?;
    let range = source.range(start, end, zero_cross.unwrap_or(false));
    if range.is_empty() {
        return Err("Select some audio first.".into());
    }
    let control = duck_control(&state, &clip, &effects, &source, &range)?;
    let processed = run_effects(&state, &soniq_io::extract(&source.native, range.clone()), &effects, control.as_ref())?;
    let changed = processed.buffer.frames() != range.len();
    let new_duration = changed.then(|| source.duration - source.seconds(range.len()) + processed.buffer.duration());
    let edited = soniq_io::insert(&source.native, range.start, range.len(), &processed.buffer);
    let label = label.unwrap_or_else(|| first.label().to_string());
    let mut reply = replace_clip_audio(&state, &clip, &source, edited, new_duration, &label)?;
    reply.clipped = processed.buffer.peak() > 1.0;
    reply.note = processed.note;
    Ok(reply)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FixAnalysis {
    pub diagnosis: soniq_io::Diagnosis,
    pub steps: Vec<soniq_io::FixStep>,
}

/// Listens to the selection (or the whole clip) and says what "Fix my
/// audio" would do to it, ending at `target` LUFS.
#[tauri::command]
pub async fn clip_fix_analyse(state: State<'_, AppState>, clip: String, start: Option<f64>, end: Option<f64>, target: f64) -> CmdResult<FixAnalysis> {
    let source = clip_audio(&state, &clip)?;
    let range = source.range(start, end, false);
    if range.is_empty() {
        return Err("Select some audio first.".into());
    }
    let diagnosis = soniq_io::diagnose(&soniq_io::extract(&source.native, range));
    let steps = soniq_io::fix_plan(&diagnosis, target.clamp(-30.0, -8.0));
    Ok(FixAnalysis { diagnosis, steps })
}

/// Plays the effect on (up to 10 s of) the selection, without changing
/// anything. Returns how long the preview is, in seconds.
#[tauri::command]
pub async fn clip_effect_preview(state: State<'_, AppState>, clip: String, effects: Vec<Effect>, start: Option<f64>, end: Option<f64>) -> CmdResult<f64> {
    let source = clip_audio(&state, &clip)?;
    let mut range = source.range(start, end, false);
    let most = (PREVIEW_SECONDS * f64::from(source.native.sample_rate)) as usize;
    range.end = range.end.min(range.start + most);
    let control = duck_control(&state, &clip, &effects, &source, &range)?;
    let selection = soniq_io::extract(&source.native, range);
    let processed = if effects.is_empty() { selection } else { run_effects(&state, &selection, &effects, control.as_ref())?.buffer };
    let rate = state.engine.sample_rate();
    let audio = if processed.sample_rate == rate { processed } else { soniq_io::resample(&processed, rate) };
    let seconds = audio.duration();
    state.engine.transport().set_playing(false);
    state.engine.preview(Arc::new(audio))?;
    Ok(seconds)
}

#[tauri::command]
pub fn preview_stop(state: State<'_, AppState>) {
    state.engine.stop_preview();
}

/// Learns the noise floor from the selection. Returns its length in seconds.
#[tauri::command]
pub async fn noise_profile_capture(state: State<'_, AppState>, clip: String, start: Option<f64>, end: Option<f64>) -> CmdResult<f64> {
    let source = clip_audio(&state, &clip)?;
    let range = source.range(start, end, false);
    let profile = soniq_io::noise_profile(&soniq_io::extract(&source.native, range))?;
    let seconds = profile.seconds;
    *lock(&state.noise_profile) = Some(profile);
    Ok(seconds)
}

/// Seconds of noise in the current profile, if there is one.
#[tauri::command]
pub fn noise_profile_status(state: State<'_, AppState>) -> Option<f64> {
    lock(&state.noise_profile).as_ref().map(|p| p.seconds)
}

// ---------- Spectral editing ----------

/// Changes the level of a frequency band over a time range (seconds from the
/// clip start). `gain_db` of −100 or less removes the band; `invert` changes
/// everything outside the band instead, keeping only the band.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn clip_spectral(
    state: State<'_, AppState>,
    clip: String,
    start: f64,
    end: f64,
    low_hz: f64,
    high_hz: f64,
    gain_db: f64,
    invert: Option<bool>,
) -> CmdResult<ProjectReply> {
    let source = clip_audio(&state, &clip)?;
    let range = source.range(Some(start), Some(end), false);
    if range.is_empty() {
        return Err("Select a time range and a frequency band first.".into());
    }
    let gain = if gain_db <= -100.0 { 0.0 } else { 10f64.powf(gain_db / 20.0) as f32 };
    let (lo, hi) = (low_hz.min(high_hz).max(0.0), low_hz.max(high_hz));
    let nyquist = f64::from(source.native.sample_rate) / 2.0;
    let edited = if invert.unwrap_or(false) {
        let below = soniq_io::spectral_gain(&source.native, range.start, range.end, 0.0, lo, gain);
        soniq_io::spectral_gain(&below, range.start, range.end, hi, nyquist + 1.0, gain)
    } else {
        soniq_io::spectral_gain(&source.native, range.start, range.end, lo, hi, gain)
    };
    let label = match (invert.unwrap_or(false), gain) {
        (true, _) => "Keep frequency band",
        (false, 0.0) => "Delete frequency band",
        (false, g) if g < 1.0 => "Attenuate frequency band",
        _ => "Boost frequency band",
    };
    replace_clip_audio(&state, &clip, &source, edited, None, label)
}

/// Spectrogram of `start..end` seconds of an asset as raw bytes:
/// `[channels, columns, rows, sample_rate]` as u32, then `columns × rows`
/// bytes per channel, row-major from the top row (0 = −100 dB, 255 = 0 dB).
#[tauri::command]
pub async fn asset_spectrogram(
    state: State<'_, AppState>,
    id: String,
    start: f64,
    end: f64,
    columns: u32,
    rows: u32,
    scale: FrequencyScale,
) -> CmdResult<Response> {
    let native = native_audio(&state, &id)?;
    let rate = f64::from(native.sample_rate);
    let (columns, rows) = (columns.clamp(1, 4096), rows.clamp(8, 2048));
    let (a, b) = ((start.max(0.0) * rate) as usize, (end.max(0.0) * rate) as usize);
    let images = soniq_io::spectrogram(&native, a, b, columns as usize, rows as usize, scale);
    let mut bytes = header(&[images.len() as u32, columns, rows, native.sample_rate]);
    for image in images {
        bytes.extend_from_slice(&image);
    }
    Ok(Response::new(bytes))
}

/// Waveform detail for `start..end` seconds of an asset, as raw bytes:
/// `[channels, count, raw, sample_rate, first_frame]` as u32, then per
/// channel either `count` samples (raw = 1, when zoomed in to single
/// samples) or `count` (min, max) pairs, one per column.
#[tauri::command]
pub async fn asset_window(state: State<'_, AppState>, id: String, start: f64, end: f64, columns: u32) -> CmdResult<Response> {
    let native = native_audio(&state, &id)?;
    let rate = f64::from(native.sample_rate);
    let frames = native.frames();
    let a = ((start.max(0.0) * rate).floor() as usize).min(frames);
    let b = ((end.max(0.0) * rate).ceil() as usize).clamp(a, frames);
    let columns = columns.clamp(1, 8192) as usize;
    let n = b - a;
    let raw = n <= columns * 2;
    let count = if raw { n } else { columns };
    let mut bytes = header(&[native.channel_count() as u32, count as u32, u32::from(raw), native.sample_rate, a as u32]);
    for ch in &native.channels {
        if raw {
            ch[a..b].iter().for_each(|s| bytes.extend_from_slice(&s.to_le_bytes()));
        } else {
            for c in 0..columns {
                let (from, to) = (a + c * n / columns, (a + (c + 1) * n / columns).max(a + c * n / columns + 1).min(b));
                let (lo, hi) = ch[from..to].iter().fold((f32::MAX, f32::MIN), |(lo, hi), s| (lo.min(*s), hi.max(*s)));
                bytes.extend_from_slice(&lo.to_le_bytes());
                bytes.extend_from_slice(&hi.to_le_bytes());
            }
        }
    }
    Ok(Response::new(bytes))
}

/// Replaces samples drawn by hand, from frame `first` of the clip's audio,
/// on one channel or (`channel` = None) on all of them.
#[tauri::command]
pub async fn clip_draw(state: State<'_, AppState>, clip: String, channel: Option<usize>, first: usize, samples: Vec<f32>) -> CmdResult<ProjectReply> {
    let source = clip_audio(&state, &clip)?;
    if samples.is_empty() {
        return Err("Nothing was drawn.".into());
    }
    let mut edited = (*source.native).clone();
    for (c, ch) in edited.channels.iter_mut().enumerate() {
        if channel.is_some_and(|only| only != c) {
            continue;
        }
        for (slot, value) in ch.iter_mut().skip(first).zip(&samples) {
            *slot = value.clamp(-1.0, 1.0);
        }
    }
    replace_clip_audio(&state, &clip, &source, edited, None, "Draw samples")
}

// ---------- Generators ----------

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum GenerateTarget {
    /// Into a clip at `start` seconds from its start, replacing up to `end`.
    Clip { clip: String, start: f64, end: f64 },
    /// A new clip at `at` beats on `track`, or on a new track.
    Track { track: Option<String>, at: f64 },
}

#[tauri::command]
pub async fn generate_audio(state: State<'_, AppState>, generator: Generator, target: GenerateTarget) -> CmdResult<ProjectReply> {
    let label = format!("Generate {}", generator.label().to_lowercase());
    match target {
        GenerateTarget::Clip { clip, start, end } => {
            let source = clip_audio(&state, &clip)?;
            let range = source.range(Some(start), Some(end), false);
            let audio = soniq_io::generate(&generator, source.native.sample_rate);
            if audio.frames() == 0 {
                return Err("Nothing to generate: give it a length.".into());
            }
            let duration = source.duration - source.seconds(range.len()) + audio.duration();
            let edited = soniq_io::insert(&source.native, range.start, range.len(), &audio);
            replace_clip_audio(&state, &clip, &source, edited, Some(duration), &label)
        }
        GenerateTarget::Track { track, at } => {
            let rate = state.engine.sample_rate();
            let audio = soniq_io::generate(&generator, rate);
            if audio.frames() == 0 {
                return Err("Nothing to generate: give it a length.".into());
            }
            let name = generator.label().to_string();
            let asset = Asset {
                id: new_id("a"),
                name: name.clone(),
                channels: 1,
                sample_rate: rate,
                frames: audio.frames() as u64,
                path: None,
                missing: false,
            };
            let pooled = PoolAsset::prepare(audio, rate);

            let mut session = lock(&state.session);
            let session = &mut *session;
            let before = session.project.clone();
            let project = &mut session.project;
            let track_id = match track.filter(|id| project.track(id).is_some_and(|t| t.kind == TrackKind::Audio)) {
                Some(id) => id,
                None => {
                    let id = new_id("t");
                    let new = Track::new(id.clone(), name.clone(), TrackKind::Audio, project.next_color());
                    project.tracks.push(new);
                    id
                }
            };
            let clip = Clip::audio(name, asset.id.clone(), at.max(0.0), 0.0, asset.duration());
            let clip_id = clip.id.clone();
            if let Some(t) = project.track_mut(&track_id) {
                t.clips.push(clip);
                t.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
            }
            session.pool.insert(asset.id.clone(), pooled);
            session.project.assets.push(asset);
            session.history.record(before, &label, None);
            session.mark_changed();
            state.sync_engine(session);
            let mut reply = state.reply(session);
            reply.created = Some(clip_id);
            Ok(reply)
        }
    }
}

// ---------- Macros and batch processing ----------

#[tauri::command]
pub fn macros_get(state: State<'_, AppState>) -> Vec<Macro> {
    settings::load_macros(&state.dirs.config)
}

#[tauri::command]
pub fn macros_set(state: State<'_, AppState>, macros: Vec<Macro>) -> CmdResult<()> {
    settings::save_macros(&state.dirs.config, &macros)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchOutput {
    pub folder: String,
    /// Added to each file name, before the extension.
    #[serde(default)]
    pub suffix: String,
    #[serde(flatten)]
    pub encoding: Encoding,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchProgress {
    /// Files finished so far.
    pub done: usize,
    pub total: usize,
    /// The file being worked on.
    pub file: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchFailure {
    pub file: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchReport {
    pub written: Vec<String>,
    pub failed: Vec<BatchFailure>,
    pub folder: String,
}

/// Decodes each file, runs the steps and writes the result into `folder`.
/// A failing file is reported and skipped; the rest carry on.
#[tauri::command]
pub async fn batch_process(
    state: State<'_, AppState>,
    files: Vec<String>,
    steps: Vec<Effect>,
    output: BatchOutput,
    progress: Channel<BatchProgress>,
) -> CmdResult<BatchReport> {
    let folder = PathBuf::from(&output.folder);
    std::fs::create_dir_all(&folder).map_err(|e| format!("Could not create {}: {e}", folder.display()))?;
    let total = files.len();
    let mut report = BatchReport { written: Vec::new(), failed: Vec::new(), folder: output.folder.clone() };
    for (done, file) in files.iter().enumerate() {
        let name = Path::new(file).file_name().map_or_else(|| file.clone(), |n| n.to_string_lossy().into_owned());
        let _ = progress.send(BatchProgress { done, total, file: name.clone() });
        match process_file(&state, Path::new(file), &steps, &output, &folder, &report.written) {
            Ok(path) => report.written.push(path.display().to_string()),
            Err(error) => report.failed.push(BatchFailure { file: name, error }),
        }
    }
    let _ = progress.send(BatchProgress { done: total, total, file: String::new() });
    Ok(report)
}

fn process_file(state: &AppState, file: &Path, steps: &[Effect], output: &BatchOutput, folder: &Path, written: &[String]) -> CmdResult<PathBuf> {
    let input = soniq_io::decode_file(file).map_err(|e| e.to_string())?;
    let processed = if steps.is_empty() { input } else { run_effects(state, &input, steps, None)?.buffer };
    let stem = file.file_stem().map_or_else(|| "audio".into(), |s| s.to_string_lossy().into_owned());
    let ext = output.encoding.format.extension();
    // Inputs with the same name (song.wav, song.mp3) must not overwrite each other's results.
    let target = (1..)
        .map(|n| folder.join(if n == 1 { format!("{stem}{}.{ext}", output.suffix) } else { format!("{stem}{} ({n}).{ext}", output.suffix) }))
        .find(|p| !written.iter().any(|w| Path::new(w) == p))
        .unwrap_or_else(|| folder.join(format!("{stem}.{ext}")));
    let same = |a: &Path, b: &Path| a.canonicalize().ok().zip(b.canonicalize().ok()).is_some_and(|(a, b)| a == b);
    if same(&target, file) {
        return Err("This would overwrite the original: add a suffix or pick another folder.".into());
    }
    soniq_io::write_export(&target, &processed, &output.encoding).map_err(|e| e.to_string())?;
    Ok(target)
}
