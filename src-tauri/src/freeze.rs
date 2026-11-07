//! Freezing tracks and rendering clips to audio: a track's clips played
//! through its instrument and effects, rendered faster than real time.

use soniq_engine::Graph;
use soniq_io::AudioBuffer;
use soniq_project::{Asset, AutoTarget, Clip, Op, Project, Track, TrackKind, new_id};
use tauri::State;

use crate::session::{AppState, PoolAsset, ProjectReply, lock};

type CmdResult<T> = Result<T, String>;

/// How long a release or a reverb may ring on past the last clip, at most.
const TAIL_SECONDS: f64 = 10.0;
/// Quieter than this (about −90 dBFS) the tail has died away.
const SILENCE: f32 = 3.2e-5;

/// The song with only `track` heard, straight into a bare master: its clips
/// and devices before its fader. With `clip`, that clip is all it plays.
pub(crate) fn alone(project: &Project, track: &str, clip: Option<&str>) -> Project {
    let mut song = project.clone();
    for t in &mut song.tracks {
        t.solo = false;
        if t.id != track {
            // Muted, not taken out: one may still key a sidechain.
            t.mute = true;
            continue;
        }
        t.mute = false;
        t.volume = 0.0;
        t.pan = 0.0;
        t.sends = [0.0; 2];
        t.group = None;
        t.automation.retain(|lane| matches!(lane.target, AutoTarget::Param { .. }));
        if let Some(clip) = clip {
            t.clips.retain(|c| c.id == clip);
        }
    }
    for r in &mut song.returns {
        r.mute = true;
    }
    song.master_devices.clear();
    song.master_volume = 0.0;
    song
}

/// `start..end` seconds, and on for as long as the sound rings (up to `TAIL_SECONDS`).
fn render(graph: Graph, rate: u32, bpm: f64, beats_per_bar: u32, start: f64, end: f64) -> AudioBuffer {
    let mut audio = soniq_engine::render_offline(graph, rate, bpm, beats_per_bar, start, end + TAIL_SECONDS);
    let keep = ((end - start) * f64::from(rate)).round() as usize;
    let rings = (0..audio.frames()).rev().find(|&i| audio.channels.iter().any(|c| c[i].abs() > SILENCE)).map_or(0, |i| i + 1);
    let frames = keep.max(rings).min(audio.frames());
    for channel in &mut audio.channels {
        channel.truncate(frames);
    }
    audio
}

fn rendered_asset(name: String, audio: &AudioBuffer) -> Asset {
    Asset { id: new_id("a"), name, channels: 2, sample_rate: audio.sample_rate, frames: audio.frames() as u64, path: None, missing: false }
}

/// Renders a track's clips through its instrument and effects, from the
/// start of the song, for it to play instead while its devices rest.
#[tauri::command]
pub async fn track_freeze(state: State<'_, AppState>, track: String) -> CmdResult<ProjectReply> {
    let (graph, bpm, beats_per_bar, end, name) = {
        let session = lock(&state.session);
        let project = &session.project;
        let t = project.track(&track).ok_or("That track no longer exists.")?;
        if t.frozen.is_some() {
            return Err(format!("{} is frozen already.", t.name));
        }
        if t.kind == TrackKind::Group {
            return Err("Group tracks play their tracks: freeze those instead.".into());
        }
        let tempo = project.tempo_map();
        let end = t.timeline().map(|c| tempo.seconds_at(c.end(&tempo))).fold(0.0, f64::max);
        if end <= 0.0 {
            return Err(format!("{} has no clips on the timeline to freeze.", t.name));
        }
        let graph = state.graph_of(&alone(project, &track, None), &session.pool, false);
        (graph, project.bpm, project.beats_per_bar, end, t.name.clone())
    };
    let rate = state.engine.sample_rate();
    let audio = render(graph, rate, bpm, beats_per_bar, 0.0, end);
    let asset = rendered_asset(format!("{name} (frozen)"), &audio);
    let pooled = PoolAsset::prepare(audio, rate);

    let mut session = lock(&state.session);
    let session = &mut *session;
    session.pool.insert(asset.id.clone(), pooled);
    let applied = soniq_project::apply(&mut session.project, &mut session.history, Op::FreezeTrack { track, asset });
    session.collect_garbage();
    applied.map_err(|e| e.to_string())?;
    session.mark_changed();
    state.sync_engine(session);
    Ok(state.reply(session))
}

/// Renders one clip through its track's instrument and effects onto a new
/// audio track just below, and mutes the track it came from.
#[tauri::command]
pub async fn clip_render(state: State<'_, AppState>, clip: String) -> CmdResult<ProjectReply> {
    let (graph, bpm, beats_per_bar, start, end, name) = {
        let session = lock(&state.session);
        let project = &session.project;
        let (ti, ci) = project.find_clip(&clip).ok_or("That clip no longer exists.")?;
        let (t, c) = (&project.tracks[ti], &project.tracks[ti].clips[ci]);
        if c.slot.is_some() {
            return Err("Only clips on the timeline render to a track.".into());
        }
        if t.frozen.is_some() {
            return Err(format!("Unfreeze {} to render its clips.", t.name));
        }
        let tempo = project.tempo_map();
        let graph = state.graph_of(&alone(project, &t.id, Some(&clip)), &session.pool, false);
        (graph, project.bpm, project.beats_per_bar, tempo.seconds_at(c.start), tempo.seconds_at(c.end(&tempo)), c.name.clone())
    };
    let rate = state.engine.sample_rate();
    let audio = render(graph, rate, bpm, beats_per_bar, start, end);
    if audio.peak() == 0.0 {
        return Err("That clip makes no sound to render.".into());
    }
    let asset = rendered_asset(format!("{name} (rendered)"), &audio);
    let pooled = PoolAsset::prepare(audio, rate);

    let mut session = lock(&state.session);
    let session = &mut *session;
    let before = session.project.clone();
    let project = &mut session.project;
    let (ti, ci) = project.find_clip(&clip).ok_or("That clip no longer exists.")?;
    let at = project.tracks[ti].clips[ci].start;

    // The new track carries on where the render stopped: the source's fader, sends and group.
    let source = &mut project.tracks[ti];
    let mut track = Track::new(new_id("t"), format!("{name} (rendered)"), TrackKind::Audio, source.color);
    track.volume = source.volume;
    track.pan = source.pan;
    track.sends = source.sends;
    track.group = source.group.clone();
    track.automation = source.automation.iter().filter(|lane| !matches!(lane.target, AutoTarget::Param { .. })).cloned().collect();
    let rendered = Clip::audio(name.clone(), asset.id.clone(), at, 0.0, asset.duration());
    let created = rendered.id.clone();
    track.clips.push(rendered);
    source.mute = true;
    let source_name = source.name.clone();
    project.tracks.insert(ti + 1, track);

    session.pool.insert(asset.id.clone(), pooled);
    session.project.assets.push(asset);
    session.history.record(before, &format!("Render {name} to audio"), None);
    session.mark_changed();
    state.sync_engine(session);
    let mut reply = state.reply(session);
    reply.created = Some(created);
    reply.note = Some(format!("{source_name} is muted: {name} plays from the track below it"));
    Ok(reply)
}
