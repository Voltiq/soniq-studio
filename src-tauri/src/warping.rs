//! Audio clips and the song's tempo: warping a clip to it (finding the
//! clip's own tempo first), and slicing a clip onto a new MIDI track.

use soniq_io::AudioBuffer;
use soniq_project::{Clip, ClipContent, Note, Op, Project, SampleRef, Track, TrackKind, Warp, WarpMode, catalog, new_id};
use tauri::State;

use crate::session::{AppState, ProjectReply, Session, lock};

type CmdResult<T> = Result<T, String>;

/// Most slices a Sampler plays.
const MAX_SLICES: usize = soniq_dsp::MAX_SLICES;
/// How sensitive slicing by transients is (0–100).
const SLICE_SENSITIVITY: u32 = 50;

/// An audio clip on the timeline: its track, asset, offset and duration.
fn audio_clip<'a>(project: &'a Project, id: &str) -> CmdResult<(usize, &'a Clip, &'a str, f64, f64)> {
    let (ti, ci) = project.find_clip(id).ok_or("That clip no longer exists.")?;
    let clip = &project.tracks[ti].clips[ci];
    match &clip.content {
        ClipContent::Audio { asset, offset, duration, .. } => Ok((ti, clip, asset, *offset, *duration)),
        ClipContent::Midi { .. } => Err("Only audio clips do that.".into()),
    }
}

/// The part of the clip's file it plays, as decoded.
fn region(session: &Session, asset: &str, offset: f64, duration: f64) -> CmdResult<AudioBuffer> {
    let native = &session.pool.get(asset).ok_or("The clip's audio is not loaded.")?.native;
    let rate = f64::from(native.sample_rate);
    let (a, b) = ((offset * rate) as usize, ((offset + duration) * rate) as usize);
    let (a, b) = (a.min(native.frames()), b.min(native.frames()));
    Ok(AudioBuffer::new(native.sample_rate, native.channels.iter().map(|c| c[a..b].to_vec()).collect()))
}

/// Makes an audio clip follow the song's tempo: it finds the clip's own
/// tempo (a loop gets the one that makes it whole bars) and keeps its beats
/// on the song's from then on. Without a steady beat it takes the song's
/// tempo where it starts, so nothing moves until the tempo changes.
#[tauri::command]
pub async fn clip_warp(state: State<'_, AppState>, clip: String, mode: Option<WarpMode>) -> CmdResult<ProjectReply> {
    let (audio, fallback) = {
        let session = lock(&state.session);
        let (_, c, asset, offset, duration) = audio_clip(&session.project, &clip)?;
        let tempo = session.project.tempo_map();
        (region(&session, asset, offset, duration)?, tempo.bpm_at(c.start))
    };
    // Finding the tempo reads the whole clip: no lock held meanwhile.
    let found = soniq_io::detect_tempo(&audio);
    let warp = Warp { bpm: found.unwrap_or(fallback), mode: mode.unwrap_or_default() };
    let mut session = lock(&state.session);
    let session = &mut *session;
    soniq_project::apply(&mut session.project, &mut session.history, Op::SetWarp { clip, warp: Some(warp) }).map_err(|e| e.to_string())?;
    session.mark_changed();
    state.sync_engine(session);
    let mut reply = state.reply(session);
    reply.note = Some(match found {
        Some(bpm) => format!("Its own tempo is {bpm} BPM: it follows the song's now"),
        None => "No steady beat found: it follows the song's tempo from where it is".into(),
    });
    Ok(reply)
}

/// A new MIDI track that plays an audio clip in slices: a Sampler holding
/// the clip's audio, cut at its transients (or every `division` beats), and
/// a MIDI clip in the audio clip's place that plays each slice where it was.
/// Each slice then plays at its own speed wherever its note goes.
#[tauri::command]
pub fn slice_to_midi(state: State<'_, AppState>, clip: String, division: Option<f64>) -> CmdResult<ProjectReply> {
    let mut session = lock(&state.session);
    let session = &mut *session;
    let project = &session.project;
    let tempo = project.tempo_map();
    let (ti, source, asset, offset, duration) = audio_clip(project, &clip)?;
    let playback = &session.pool.get(asset).ok_or("The clip's audio is not loaded.")?.playback;
    let (frames, rate) = (playback.frames().max(1) as f64, f64::from(playback.sample_rate));
    let (from, to) = (offset * rate, ((offset + duration) * rate).min(frames));
    let length = source.length_beats(&tempo);

    // Where each slice starts, in seconds into the clip; and how the Sampler finds the same ones.
    let (starts, slicing): (Vec<f64>, [(&str, f64); 3]) = match division.filter(|d| *d > 0.0) {
        Some(d) => {
            let count = ((length / d).round() as usize).clamp(1, MAX_SLICES);
            ((0..count).map(|k| duration * k as f64 / count as f64).collect(), [("sliceBy", 0.0), ("slices", count as f64), ("sensitivity", 50.0)])
        }
        None => {
            let onsets = session.pool.onsets(asset, SLICE_SENSITIVITY).unwrap_or_default();
            let mut starts: Vec<f64> = onsets.iter().map(|&f| f as f64).filter(|f| *f >= from && *f < to).take(MAX_SLICES).map(|f| (f - from) / rate).collect();
            if starts.is_empty() {
                starts.push(0.0);
            }
            (starts, [("sliceBy", 1.0), ("slices", MAX_SLICES as f64), ("sensitivity", f64::from(SLICE_SENSITIVITY))])
        }
    };
    const ROOT: u8 = 60;
    let beat_of = |seconds: f64| source.beat_after(&tempo, source.start, seconds) - source.start;
    let notes: Vec<Note> = starts
        .iter()
        .enumerate()
        .map(|(k, &s)| {
            let end = starts.get(k + 1).copied().unwrap_or(duration);
            Note { start: beat_of(s), length: (beat_of(end) - beat_of(s)).max(soniq_project::MIN_NOTE), pitch: ROOT.saturating_add(k as u8), velocity: 0.8 }
        })
        .collect();

    let mut params = vec![("mode", 3.0), ("root", f64::from(ROOT)), ("start", from / frames * 100.0), ("end", to / frames * 100.0)];
    params.extend(slicing);
    let mut sampler = catalog::device("sampler", &params).ok_or("The Sampler is missing from the device catalog.")?;
    sampler.samples = vec![SampleRef { key: 0, asset: asset.to_string() }];
    let name = format!("{} slices", source.name);
    let midi = Clip::midi(name.clone(), source.start, length, notes);
    let created = midi.id.clone();
    let count = starts.len();

    let before = session.project.clone();
    let mut track = Track::new(new_id("t"), &name, TrackKind::Midi, session.project.next_color());
    track.devices = vec![sampler];
    track.clips = vec![midi];
    // Right under the audio, in its group if it has one.
    track.group = session.project.tracks[ti].group.clone();
    session.project.tracks.insert(ti + 1, track);
    session.history.record(before, "Slice to MIDI", None);
    session.mark_changed();
    state.sync_engine(session);
    let mut reply = state.reply(session);
    reply.created = Some(created);
    reply.note = Some(format!("{count} slices on a new MIDI track"));
    Ok(reply)
}
