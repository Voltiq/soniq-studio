//! MIDI commands: live notes from the computer keyboard and the piano roll,
//! hardware inputs, MIDI files, MIDI recording, and samples for the sampler
//! and drum rack.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use soniq_engine::{ALL_TRACKS, NoteEvent, track_key};
use serde::Deserialize;
use soniq_midi::{ExportPart, MidiEvent, MidiPortStatus, read_midi, write_midi};
use soniq_project::{Clip, ClipContent, MappingTarget, Note, Project, SampleRef, TempoMap, Track, TrackKind, catalog, new_id, tidy_notes};
use tauri::{AppHandle, Manager, State};

use crate::files;
use crate::session::{AppState, HeardNote, MidiTake, PoolAsset, ProjectReply, lock};
use crate::settings;

type CmdResult<T> = Result<T, String>;

/// A hardware MIDI message as the engine's note event (every channel plays);
/// `None` for a controller, which is not a note.
pub fn note_event(event: MidiEvent) -> Option<NoteEvent> {
    Some(match event {
        MidiEvent::NoteOn { pitch, velocity, .. } => NoteEvent::On { pitch, velocity },
        MidiEvent::NoteOff { pitch, .. } => NoteEvent::Off { pitch },
        MidiEvent::Sustain { down, .. } => NoteEvent::Sustain(down),
        MidiEvent::AllNotesOff { .. } => NoteEvent::AllNotesOff,
        MidiEvent::Control { .. } => return None,
    })
}

/// A note from the UI. With `track` it plays only there (auditioning in the
/// piano roll or on a drum pad); without, it goes where a MIDI keyboard's
/// notes go and is recorded.
#[tauri::command]
pub fn note_play(state: State<'_, AppState>, track: Option<String>, pitch: u8, velocity: f32, on: bool) {
    let pitch = pitch.min(127);
    let event = if on { NoteEvent::On { pitch, velocity: velocity.clamp(0.01, 1.0) } } else { NoteEvent::Off { pitch } };
    match track {
        Some(id) => {
            state.engine.play_note(track_key(&id), event);
        }
        None => state.route_note(event),
    }
}

#[tauri::command]
pub fn note_sustain(state: State<'_, AppState>, down: bool) {
    state.route_note(NoteEvent::Sustain(down));
}

/// Silences every instrument at once.
#[tauri::command]
pub fn note_panic(state: State<'_, AppState>) {
    state.engine.play_note(ALL_TRACKS, NoteEvent::AllSoundOff);
}

/// The UI's selected track: it plays live notes while no MIDI track is armed.
#[tauri::command]
pub fn midi_select(state: State<'_, AppState>, track: Option<String>) {
    state.select_track(track.as_deref());
}

#[tauri::command]
pub fn midi_activity(state: State<'_, AppState>) -> u64 {
    state.midi_activity.load(std::sync::atomic::Ordering::Relaxed)
}

/// Maps the next MIDI controller that moves to `target`; `None` stops learning.
#[tauri::command]
pub fn midi_learn(state: State<'_, AppState>, target: Option<MappingTarget>) {
    *lock(&state.learning) = target;
}

/// Feeds raw MIDI bytes in as if a controller had sent them: for testing
/// without hardware, in debug builds only.
#[tauri::command]
pub fn midi_simulate(state: State<'_, AppState>, bytes: Vec<u8>) -> CmdResult<()> {
    if !cfg!(debug_assertions) {
        return Err("MIDI simulation is only in debug builds.".into());
    }
    let event = soniq_midi::parse(&bytes).ok_or("Soniq ignores that MIDI message.")?;
    state.midi_in(event);
    Ok(())
}

/// Applies MIDI controller moves in batches: every few milliseconds, the
/// latest value of each control, so a fast knob makes one change per batch
/// instead of one per message.
pub fn spawn_control_worker(handle: AppHandle, moves: Receiver<(u8, u8, u8)>) {
    let worker = std::thread::Builder::new().name("midi-controls".into()).spawn(move || {
        while let Ok(first) = moves.recv() {
            std::thread::sleep(Duration::from_millis(15));
            let mut latest = HashMap::new();
            for (channel, cc, value) in std::iter::once(first).chain(moves.try_iter()) {
                latest.insert((channel, cc), value);
            }
            handle.state::<AppState>().apply_controls(&latest);
        }
    });
    if let Err(err) = worker {
        eprintln!("MIDI controllers will not work: {err}");
    }
}

#[tauri::command]
pub async fn midi_inputs(state: State<'_, AppState>) -> CmdResult<Vec<MidiPortStatus>> {
    let inputs = lock(&state.midi).clone();
    Ok(inputs.map(|m| {
        m.rescan();
        m.status()
    })
    .unwrap_or_default())
}

#[tauri::command]
pub async fn midi_input_enable(state: State<'_, AppState>, name: String, enabled: bool) -> CmdResult<Vec<MidiPortStatus>> {
    let inputs = lock(&state.midi).clone().ok_or("MIDI input is not available.")?;
    inputs.set_enabled(&name, enabled);
    settings::save_midi(&state.dirs.config, &settings::MidiSettings { disabled_inputs: inputs.disabled() })?;
    Ok(inputs.status())
}

/// Beats rounded up to whole bars.
fn whole_bars(beats: f64, beats_per_bar: u32) -> f64 {
    let bar = f64::from(beats_per_bar.max(1));
    ((beats / bar).ceil() * bar).max(bar)
}

/// A MIDI track for an imported part: drums get the Drum Rack, pianos the
/// Electric Piano, everything else the synth.
fn track_for_part(name: &str, drums: bool, program: Option<u8>, color: u8) -> Track {
    let mut track = Track::new(new_id("t"), name, TrackKind::Midi, color);
    let kind = if drums {
        "drum-rack"
    } else if program.is_some_and(|p| p < 8) {
        "epiano"
    } else {
        "synth"
    };
    track.devices = catalog::device(kind, &[]).into_iter().collect();
    track
}

/// Reads MIDI files and places their parts at `start` (beats): onto `track`
/// when there is one part and it is a MIDI track, else onto new tracks. An
/// empty song also takes the file's tempo and time signature.
#[tauri::command]
pub async fn midi_import(state: State<'_, AppState>, paths: Vec<String>, track: Option<String>, start: Option<f64>) -> CmdResult<ProjectReply> {
    let mut warnings = Vec::new();
    let mut songs = Vec::new();
    for path in &paths {
        let name = PathBuf::from(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone());
        match std::fs::read(path).map_err(|e| e.to_string()).and_then(|bytes| read_midi(&bytes)) {
            Ok(song) if song.parts.is_empty() => warnings.push(format!("{name} has no notes.")),
            Ok(song) => songs.push(song),
            Err(err) => warnings.push(format!("{name}: {err}")),
        }
    }

    let mut session = lock(&state.session);
    let session = &mut *session;
    let before = session.project.clone();
    let project = &mut session.project;
    let start = start.unwrap_or(0.0).max(0.0);
    let mut note = None;
    if project.tracks.iter().all(|t| t.clips.is_empty())
        && let Some(song) = songs.first()
    {
        if let Some(bpm) = song.bpm.filter(|b| (20.0..=999.0).contains(b)) {
            project.bpm = bpm;
            note = Some(format!("Tempo set to {bpm} BPM from the file"));
        }
        if let Some(bpb) = song.beats_per_bar {
            project.beats_per_bar = bpb;
        }
    }
    let parts: Vec<_> = songs.into_iter().flat_map(|s| s.parts).collect();
    let mut target = track.filter(|id| parts.len() == 1 && project.track(id).is_some_and(|t| t.kind == TrackKind::Midi));
    let mut created = None;
    for part in &parts {
        let end = part.notes.iter().map(|n| n.start + n.length).fold(0.0, f64::max);
        let clip = Clip::midi(part.name.clone(), start, whole_bars(end, project.beats_per_bar), part.notes.clone());
        let id = match target.take() {
            Some(id) => id,
            None => {
                let t = track_for_part(&part.name, part.is_drums(), part.program, project.next_color());
                let id = t.id.clone();
                project.tracks.push(t);
                id
            }
        };
        created.get_or_insert(clip.id.clone());
        if let Some(t) = project.track_mut(&id) {
            t.clips.push(clip);
            t.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
        }
    }
    if !parts.is_empty() {
        let label = if parts.len() == 1 { "Import MIDI".to_string() } else { format!("Import {} MIDI parts", parts.len()) };
        session.history.record(before, &label, None);
        session.mark_changed();
        state.sync_engine(session);
    }
    let mut reply = state.reply(session);
    reply.warnings = warnings;
    reply.created = created;
    reply.note = note;
    Ok(reply)
}

/// Writes one clip (from its own start) or every MIDI track (from the start
/// of the song) as a Standard MIDI File.
#[tauri::command]
pub async fn midi_export(state: State<'_, AppState>, path: String, clip: Option<String>) -> CmdResult<String> {
    let (bytes, count) = {
        let session = lock(&state.session);
        let project = &session.project;
        let is_drums = |t: &Track| t.devices.iter().any(|d| d.kind == "drum-rack" && d.role == soniq_project::DeviceRole::Instrument);
        // The General MIDI sound closest to the track's instrument (a rack's first chain).
        fn instrument(t: &Track) -> Option<&str> {
            let d = t.devices.iter().find(|d| d.role == soniq_project::DeviceRole::Instrument)?;
            Some(if d.kind == "rack" { d.chains.first()?.device.kind.as_str() } else { d.kind.as_str() })
        }
        let program = |t: &Track| match instrument(t) {
            Some("epiano") => Some(4),
            Some("synth") => Some(81),
            _ => None,
        };
        let clip_notes = |c: &Clip, offset: f64| -> Vec<Note> {
            let ClipContent::Midi { length, notes, .. } = &c.content else { return Vec::new() };
            notes
                .iter()
                .filter(|n| n.start < *length)
                .map(|n| Note { start: n.start + offset, length: n.length.min(length - n.start), ..n.clone() })
                .collect()
        };
        let parts: Vec<ExportPart> = match &clip {
            Some(id) => {
                let (ti, ci) = project.find_clip(id).ok_or("That clip no longer exists.")?;
                let (t, c) = (&project.tracks[ti], &project.tracks[ti].clips[ci]);
                vec![ExportPart { name: c.name.clone(), channel: if is_drums(t) { 9 } else { 0 }, program: program(t), notes: clip_notes(c, 0.0) }]
            }
            None => {
                let mut channel = 0;
                project
                    .tracks
                    .iter()
                    .filter(|t| t.kind == TrackKind::Midi && t.timeline().next().is_some())
                    .map(|t| {
                        let mut notes: Vec<Note> = t.timeline().flat_map(|c| clip_notes(c, c.start)).collect();
                        tidy_notes(&mut notes);
                        let ch = if is_drums(t) {
                            9
                        } else {
                            // Melodic tracks take channels 1–16, skipping the drum channel.
                            let c = channel;
                            channel = (channel + 1) % 16;
                            if channel == 9 {
                                channel = 10;
                            }
                            c
                        };
                        ExportPart { name: t.name.clone(), channel: ch, program: program(t), notes }
                    })
                    .collect()
            }
        };
        if parts.iter().all(|p| p.notes.is_empty()) {
            return Err("There are no notes to export.".into());
        }
        (write_midi(&project.name, &parts, project.bpm, project.beats_per_bar)?, parts.len())
    };
    let path = PathBuf::from(path);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    }
    std::fs::write(&path, bytes).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    Ok(if count == 1 { "1 track".into() } else { format!("{count} tracks") })
}

/// Loads an audio file into a sampler (`key` 0) or a drum pad (`key` is its note).
#[tauri::command]
pub async fn device_load_sample(state: State<'_, AppState>, track: String, device: String, key: u8, path: String) -> CmdResult<ProjectReply> {
    let (asset, pooled): (_, PoolAsset) = files::import_file(&PathBuf::from(&path), state.engine.sample_rate())?;
    let mut session = lock(&state.session);
    let session = &mut *session;
    let before = session.project.clone();
    let t = session.project.track_mut(&track).ok_or("That track no longer exists.")?;
    let d = t.device_mut(&device).ok_or("That device no longer exists.")?;
    if d.kind != "sampler" && d.kind != "drum-rack" {
        return Err(format!("{} does not play samples.", d.name));
    }
    let label = format!("Load “{}” into {}", asset.name, d.name);
    d.samples.retain(|s| s.key != key);
    d.samples.push(SampleRef { key, asset: asset.id.clone() });
    d.samples.sort_by_key(|s| s.key);
    session.pool.insert(asset.id.clone(), pooled);
    session.project.assets.push(asset);
    session.project.prune_assets();
    session.history.record(before, &label, None);
    session.mark_changed();
    session.collect_garbage();
    state.sync_engine(session);
    Ok(state.reply(session))
}

/// Where the hits in a sample start, as fractions of its length, so the
/// sampler can draw the slices it plays when slicing by transients.
#[tauri::command]
pub fn sample_onsets(state: State<'_, AppState>, asset: String, sensitivity: f64) -> CmdResult<Vec<f64>> {
    let session = lock(&state.session);
    let found = session.pool.onsets(&asset, sensitivity.clamp(0.0, 100.0).round() as u32).ok_or("That sample is not loaded.")?;
    let frames = session.pool.get(&asset).map_or(1, |a| a.playback.frames().max(1)) as f64;
    Ok(found.iter().map(|&at| at as f64 / frames).collect())
}

/// How a MIDI recording lands where a track already has a clip.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MidiRecordMode {
    /// A new clip, layered over what is there.
    #[default]
    New,
    /// Into the clip that is there, keeping its notes.
    Overdub,
    /// Into the clip that is there, replacing its notes in the recorded range.
    Replace,
}

/// What a finished MIDI take becomes: (start, length, notes) in beats, from
/// where recording started (or the punch-in) to where it stopped (or the
/// punch-out). Recording over a loop keeps every pass inside the loop. Notes
/// the sustain pedal held last until it lifted. `None` when nothing was
/// played in that range.
pub(crate) fn midi_take_notes(take: &MidiTake, stop: f64, tempo: &TempoMap, punch: Option<(f64, f64)>) -> Option<(f64, f64, Vec<Note>)> {
    // The playhead went round the loop at least once.
    let wrapped = take.loop_range.is_some()
        && (stop < take.start - 1e-6 || take.events.windows(2).any(|w| w[1].0 + 1e-6 < w[0].0) || take.events.iter().any(|e| e.0 > stop + 1e-6));
    let (from, to) = match (punch, take.loop_range) {
        (Some((a, b)), _) => (a, b.min(stop)),
        (None, Some((loop_start, loop_end))) if wrapped => (take.start.min(loop_start), loop_end),
        _ => (take.start, stop),
    };
    if to - from < 0.01 {
        return None;
    }
    let beats = |seconds: f64| tempo.beats_at(seconds);
    let notes: Vec<Note> = notes_from_events(&take.events, if wrapped { to } else { stop })
        .into_iter()
        // A few milliseconds early still counts (the latency correction can land a note just before the start).
        .filter(|(start, ..)| *start >= from - 0.05 && *start < to)
        .map(|(start, end, pitch, velocity)| {
            let start = start.max(from);
            Note {
                start: beats(start) - beats(from),
                length: (beats(end.min(to)) - beats(start)).max(soniq_project::MIN_NOTE),
                pitch,
                velocity: f64::from(velocity),
            }
        })
        .collect();
    (!notes.is_empty()).then(|| (beats(from), beats(to) - beats(from), notes))
}

/// Puts a recorded or captured take on a track: as a new clip, or into the
/// clip already at `start` (overdub or replace, which also reach its linked clips).
pub(crate) fn place_midi_take(project: &mut Project, track_id: &str, start: f64, length: f64, notes: Vec<Note>, mode: MidiRecordMode) -> Option<String> {
    let tempo = project.tempo_map();
    let track = project.track_mut(track_id)?;
    let existing = (mode != MidiRecordMode::New)
        .then(|| track.clips.iter().position(|c| c.slot.is_none() && matches!(c.content, ClipContent::Midi { .. }) && c.start <= start + 1e-9 && start < c.end(&tempo) - 1e-9))
        .flatten();
    let Some(ci) = existing else {
        let clip = Clip::midi(format!("{} take {}", track.name, track.clips.len() + 1), start, length, notes);
        let id = clip.id.clone();
        track.clips.push(clip);
        track.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
        return Some(id);
    };
    let clip = &mut track.clips[ci];
    let offset = start - clip.start;
    let id = clip.id.clone();
    let ClipContent::Midi { length: clip_length, notes: current, .. } = &mut clip.content else { return None };
    let replaced = |n: &Note| mode == MidiRecordMode::Replace && n.start >= offset - 1e-9 && n.start < offset + length;
    let mut merged: Vec<Note> = current.iter().filter(|n| !replaced(n)).cloned().collect();
    merged.extend(notes.into_iter().map(|n| Note { start: n.start + offset, ..n }));
    *clip_length = clip_length.max(offset + length);
    project.set_clip_notes(&id, merged);
    Some(id)
}

/// After a pause this long, earlier playing belongs to another idea.
const CAPTURE_GAP: f64 = 8.0;

/// Capture MIDI: turns what was just played on a keyboard into a clip on the
/// armed MIDI tracks (or the selected one), although nothing was recording.
/// Played along with the song, the notes keep their place in it; played with
/// the song stopped, they start at the bar of the playhead.
#[tauri::command]
pub fn midi_capture(state: State<'_, AppState>) -> CmdResult<ProjectReply> {
    let phrase: Vec<HeardNote> = {
        let mut history = lock(&state.midi_history);
        let mut first = history.len().saturating_sub(1);
        while first > 0 && history[first].wall - history[first - 1].wall <= CAPTURE_GAP {
            first -= 1;
        }
        let phrase = history.range(first..).copied().collect();
        history.clear();
        phrase
    };
    let ons: Vec<&HeardNote> = phrase.iter().filter(|n| matches!(n.event, NoteEvent::On { .. })).collect();
    if ons.is_empty() {
        return Err("Play something first: Capture MIDI turns what you just played into a clip, without recording.".into());
    }
    let targets: Vec<String> = {
        let routing = lock(&state.midi_routing);
        if routing.armed.is_empty() { routing.selected.iter().map(|(id, _)| id.clone()).collect() } else { routing.armed.iter().map(|(id, _)| id.clone()).collect() }
    };
    if targets.is_empty() {
        return Err("Select or arm a MIDI track to capture into.".into());
    }
    let with_song = ons.iter().filter(|n| n.playing).count() * 2 >= ons.len();
    let first_wall = ons[0].wall;
    let events: Vec<(f64, NoteEvent)> = phrase.iter().map(|n| (if with_song { n.at } else { n.wall - first_wall }, n.event)).collect();
    let end = events.iter().map(|e| e.0).fold(0.0, f64::max) + 0.25;

    let mut session = lock(&state.session);
    let session = &mut *session;
    let (tempo, bar) = (session.project.tempo_map(), f64::from(session.project.beats_per_bar.max(1)));
    let playhead = tempo.beats_at(state.engine.transport().position_seconds());
    // Played with the song, times are the song's; played alone, they count
    // from the first note at the tempo where the playhead is.
    let beats = |seconds: f64| if with_song { tempo.beats_at(seconds) } else { seconds * tempo.bpm_at(playhead) / 60.0 };
    let found = notes_from_events(&events, end);
    let first = found.iter().map(|n| beats(n.0)).fold(f64::INFINITY, f64::min);
    let start = if with_song { (first / bar).floor() * bar } else { (playhead / bar).floor() * bar };
    let offset = if with_song { 0.0 } else { start };
    let notes: Vec<Note> = found
        .iter()
        .map(|&(s, e, pitch, velocity)| Note {
            start: (beats(s) + offset - start).max(0.0),
            length: (beats(e) - beats(s)).max(soniq_project::MIN_NOTE),
            pitch,
            velocity: f64::from(velocity),
        })
        .collect();
    let last = notes.iter().map(|n| n.start + n.length).fold(0.0, f64::max);
    let length = ((last / bar).ceil() * bar).max(bar);

    let before = session.project.clone();
    let mut created = None;
    for track in &targets {
        let id = place_midi_take(&mut session.project, track, start, length, notes.clone(), MidiRecordMode::New);
        created = created.or(id);
    }
    session.history.record(before, "Capture MIDI", None);
    session.mark_changed();
    state.sync_engine(session);
    let mut reply = state.reply(session);
    reply.created = created;
    reply.note = Some(format!("{} notes{}", notes.len(), if with_song { ", where you played them" } else { ", from the playhead's bar" }));
    Ok(reply)
}

/// Notes (in seconds) from note events: (start, end, pitch, velocity). A
/// note-off that comes back round a loop (earlier than its note-on) ends
/// the note at `end`.
pub(crate) fn notes_from_events(events: &[(f64, NoteEvent)], end: f64) -> Vec<(f64, f64, u8, f32)> {
    let mut open: [Option<(f64, f32)>; 128] = [None; 128];
    let mut pedal = false;
    let mut pending: Vec<u8> = Vec::new();
    let mut out = Vec::new();
    let close = |out: &mut Vec<(f64, f64, u8, f32)>, (start, vel): (f64, f32), at: f64, pitch: u8| {
        out.push((start, if at < start { end } else { at }, pitch, vel));
    };
    for &(at, event) in events {
        match event {
            NoteEvent::On { pitch, velocity } => {
                // A key struck again while still sounding ends the old note.
                if let Some(note) = open[pitch as usize].take() {
                    close(&mut out, note, at, pitch);
                }
                pending.retain(|p| *p != pitch);
                open[pitch as usize] = Some((at, velocity));
            }
            NoteEvent::Off { pitch } => {
                if pedal {
                    if open[pitch as usize].is_some() && !pending.contains(&pitch) {
                        pending.push(pitch);
                    }
                } else if let Some(note) = open[pitch as usize].take() {
                    close(&mut out, note, at, pitch);
                }
            }
            NoteEvent::Sustain(down) => {
                pedal = down;
                if !down {
                    for pitch in pending.drain(..) {
                        if let Some(note) = open[pitch as usize].take() {
                            close(&mut out, note, at, pitch);
                        }
                    }
                }
            }
            NoteEvent::AllNotesOff | NoteEvent::AllSoundOff => {
                pending.clear();
                for (pitch, slot) in open.iter_mut().enumerate() {
                    if let Some(note) = slot.take() {
                        close(&mut out, note, at, pitch as u8);
                    }
                }
            }
        }
    }
    for (pitch, slot) in open.iter().enumerate() {
        if let Some((start, vel)) = slot {
            out.push((*start, end, pitch as u8, *vel));
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pedal_holds_notes_until_it_lifts() {
        let on = |pitch| NoteEvent::On { pitch, velocity: 0.5 };
        let off = |pitch| NoteEvent::Off { pitch };
        let events = [
            (0.0, on(60)),
            (0.5, NoteEvent::Sustain(true)),
            (1.0, off(60)),
            (1.5, on(64)),
            (2.0, off(64)),
            (3.0, NoteEvent::Sustain(false)),
            (3.5, on(67)),
        ];
        let notes = notes_from_events(&events, 4.0);
        assert_eq!(notes, vec![(0.0, 3.0, 60, 0.5), (1.5, 3.0, 64, 0.5), (3.5, 4.0, 67, 0.5)]);
    }

    #[test]
    fn a_punched_take_keeps_only_notes_inside_the_range() {
        // 120 bpm: a beat is half a second. Pre-roll from 0 s, punch 2–4 s.
        let take = MidiTake {
            start: 0.0,
            tracks: vec![],
            events: vec![
                (1.0, NoteEvent::On { pitch: 60, velocity: 0.8 }),
                (2.5, NoteEvent::On { pitch: 62, velocity: 0.8 }),
                (3.0, NoteEvent::Off { pitch: 62 }),
                (3.5, NoteEvent::On { pitch: 64, velocity: 0.8 }),
            ],
            loop_range: None,
            mode: MidiRecordMode::New,
        };
        let (start, length, notes) = midi_take_notes(&take, 5.0, &TempoMap::constant(120.0), Some((2.0, 4.0))).unwrap();
        assert_eq!((start, length), (4.0, 4.0));
        assert_eq!(notes.iter().map(|n| (n.start, n.length, n.pitch)).collect::<Vec<_>>(), vec![(1.0, 1.0, 62), (3.0, 1.0, 64)]);
    }

    #[test]
    fn loop_recording_keeps_every_pass_inside_the_loop() {
        // 120 bpm, loop 2–4 s. First pass plays C at 2.5 s, the second pass E at 2.5 s
        // (the playhead came round), and a note held across the wrap ends at the loop end.
        let on = |pitch| NoteEvent::On { pitch, velocity: 0.8 };
        let take = MidiTake {
            start: 2.0,
            tracks: vec![],
            events: vec![(2.5, on(60)), (3.0, NoteEvent::Off { pitch: 60 }), (3.5, on(67)), (2.2, NoteEvent::Off { pitch: 67 }), (2.5, on(64)), (3.0, NoteEvent::Off { pitch: 64 })],
            loop_range: Some((2.0, 4.0)),
            mode: MidiRecordMode::Overdub,
        };
        let (start, length, notes) = midi_take_notes(&take, 3.2, &TempoMap::constant(120.0), None).unwrap();
        assert_eq!((start, length), (4.0, 4.0));
        let got: Vec<_> = notes.iter().map(|n| (n.start, n.length, n.pitch)).collect();
        assert_eq!(got, vec![(1.0, 1.0, 60), (1.0, 1.0, 64), (3.0, 1.0, 67)]);
    }

    #[test]
    fn overdub_merges_and_replace_clears_the_range() {
        let mut project = Project::empty("Song");
        let track = project.tracks[2].id.clone();
        let old = vec![Note { start: 0.0, length: 1.0, pitch: 60, velocity: 0.5 }, Note { start: 2.0, length: 1.0, pitch: 62, velocity: 0.5 }];
        let clip = place_midi_take(&mut project, &track, 4.0, 4.0, old, MidiRecordMode::New).unwrap();
        let new = vec![Note { start: 0.0, length: 0.5, pitch: 67, velocity: 0.9 }];
        // Overdub at beat 6 (2 beats into the clip): both old notes stay.
        place_midi_take(&mut project, &track, 6.0, 1.0, new.clone(), MidiRecordMode::Overdub);
        let pitches = |p: &Project| {
            let (ti, ci) = p.find_clip(&clip).unwrap();
            let ClipContent::Midi { notes, .. } = &p.tracks[ti].clips[ci].content else { panic!() };
            notes.iter().map(|n| n.pitch).collect::<Vec<_>>()
        };
        assert_eq!(pitches(&project), vec![60, 62, 67]);
        // Replace over the same beat: the note that started there (62) goes.
        place_midi_take(&mut project, &track, 6.0, 1.0, new, MidiRecordMode::Replace);
        assert_eq!(pitches(&project), vec![60, 67]);
        assert_eq!(project.tracks[2].clips.len(), 1, "no new clips");
    }
}
