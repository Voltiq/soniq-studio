//! Podcast mode: each speaker evened out to the same loudness, music that
//! dips under speech (as volume automation you can see and move), and
//! chapters from the song's markers.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use soniq_io::{AudioBuffer, Chapter, Effect};
use soniq_project::{Asset, AutoPoint, AutoTarget, AutomationLane, ClipContent, Project, TrackKind, new_id};
use tauri::State;

use crate::session::{AppState, PoolAsset, ProjectReply, lock};

type CmdResult<T> = Result<T, String>;

/// The most a speaker is turned up or down while leveling, in dB.
const MAX_LEVEL_GAIN: f64 = 12.0;
/// Speech closer together than this keeps the music down in between, in seconds.
const DUCK_HOLD: f64 = 2.5;
/// How the music goes down before speech and comes back after it, in seconds.
const DUCK_LEAD: f64 = 0.6;
const DUCK_FALL: f64 = 0.5;
const DUCK_STAY: f64 = 0.4;
const DUCK_RISE: f64 = 1.0;

// ---------- Chapters ----------

/// Chapters from the markers between `start` and `end` (seconds), timed from
/// `start`. Each runs to its region's end, or to the next marker.
pub fn chapters(project: &Project, start: f64, end: f64) -> Vec<Chapter> {
    let tempo = project.tempo_map();
    let mut marks: Vec<(f64, Option<f64>, String)> = project
        .markers
        .iter()
        .map(|m| (tempo.seconds_at(m.start), m.end.map(|e| tempo.seconds_at(e)), m.name.trim().to_string()))
        .filter(|(at, _, _)| *at >= start - 1e-6 && *at < end)
        .collect();
    marks.sort_by(|a, b| a.0.total_cmp(&b.0));
    let ms = |sec: f64| ((sec - start).max(0.0) * 1000.0).round() as u32;
    (0..marks.len())
        .map(|i| {
            let (at, region_end, name) = &marks[i];
            let next = marks.get(i + 1).map_or(end, |m| m.0);
            let stop = region_end.map_or(next, |e| e.min(next)).min(end);
            Chapter { title: if name.is_empty() { format!("Chapter {}", i + 1) } else { name.clone() }, start_ms: ms(*at), end_ms: ms(stop.max(*at)) }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChaptersFile {
    /// Podcasting 2.0 JSON chapters, for podcast hosts.
    Json,
    /// One `00:00 Title` line each, for show notes and video descriptions.
    Txt,
    /// A CUE sheet, for players and CD tools.
    Cue,
}

impl ChaptersFile {
    pub fn extension(self) -> &'static str {
        match self {
            ChaptersFile::Json => "chapters.json",
            ChaptersFile::Txt => "chapters.txt",
            ChaptersFile::Cue => "cue",
        }
    }
}

fn short_time(ms: u32) -> String {
    let s = ms / 1000;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{:02}:{:02}", s / 60, s % 60) }
}

/// The text of a chapters file. `audio` names the audio file a CUE sheet points at.
pub fn chapters_text(chapters: &[Chapter], format: ChaptersFile, title: &str, audio: &str) -> String {
    match format {
        ChaptersFile::Json => {
            let list: Vec<serde_json::Value> = chapters
                .iter()
                .map(|c| serde_json::json!({ "startTime": f64::from(c.start_ms) / 1000.0, "endTime": f64::from(c.end_ms) / 1000.0, "title": c.title }))
                .collect();
            let doc = serde_json::json!({ "version": "1.2.0", "title": title, "chapters": list });
            serde_json::to_string_pretty(&doc).unwrap_or_default()
        }
        ChaptersFile::Txt => chapters.iter().map(|c| format!("{} {}\n", short_time(c.start_ms), c.title)).collect(),
        ChaptersFile::Cue => {
            let quote = |s: &str| s.replace('"', "'");
            let mut out = format!("TITLE \"{}\"\nFILE \"{}\" WAVE\n", quote(title), quote(audio));
            for (i, c) in chapters.iter().enumerate() {
                // CUE times are minutes, seconds and frames of 1/75 s.
                let frames = u64::from(c.start_ms) * 75 / 1000;
                out += &format!(
                    "  TRACK {:02} AUDIO\n    TITLE \"{}\"\n    INDEX 01 {:02}:{:02}:{:02}\n",
                    i + 1,
                    quote(&c.title),
                    frames / 75 / 60,
                    frames / 75 % 60,
                    frames % 75
                );
            }
            out
        }
    }
}

/// Writes a chapters file next to `audio` (same name, the format's extension). Returns its path.
pub fn write_chapters_file(audio: &Path, chapters: &[Chapter], format: ChaptersFile, title: &str) -> CmdResult<String> {
    let stem = audio.file_stem().map_or_else(|| "chapters".into(), |s| s.to_string_lossy().into_owned());
    let path = audio.with_file_name(format!("{stem}.{}", format.extension()));
    let name = audio.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    std::fs::write(&path, chapters_text(chapters, format, title, &name)).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    Ok(path.display().to_string())
}

/// Writes the song's chapters (every marker) to `path` without any audio.
#[tauri::command]
pub fn chapters_export(state: State<'_, AppState>, path: String, format: ChaptersFile) -> CmdResult<usize> {
    let (list, title) = {
        let session = lock(&state.session);
        let project = &session.project;
        let tempo = project.tempo_map();
        // To the end of the last clip (or a little past the last marker).
        let clips = project.tracks.iter().flat_map(|t| t.timeline()).map(|c| c.end(&tempo)).fold(0.0, f64::max);
        let marks = project.markers.iter().map(|m| m.end.unwrap_or(m.start) + 1.0).fold(0.0, f64::max);
        (chapters(project, 0.0, tempo.seconds_at(clips.max(marks))), project.name.clone())
    };
    if list.is_empty() {
        return Err("The song has no markers yet: add one where each chapter starts.".into());
    }
    std::fs::write(&path, chapters_text(&list, format, &title, &format!("{title}.wav"))).map_err(|e| format!("Could not write {path}: {e}"))?;
    Ok(list.len())
}

// ---------- Speakers ----------

struct ClipPart {
    clip: String,
    offset: f64,
    duration: f64,
    gain: f64,
    /// Where it starts on the timeline, in beats.
    start: f64,
}

/// The timeline audio clips of each track, grouped by the audio they play.
fn clips_by_asset(state: &AppState, tracks: &[String]) -> Vec<(String, Arc<AudioBuffer>, Asset, Vec<ClipPart>)> {
    let session = lock(&state.session);
    let mut by_asset: BTreeMap<String, Vec<ClipPart>> = BTreeMap::new();
    for track in session.project.tracks.iter().filter(|t| t.kind == TrackKind::Audio && tracks.contains(&t.id)) {
        for clip in track.timeline() {
            if let ClipContent::Audio { asset, offset, duration, gain, .. } = &clip.content {
                by_asset.entry(asset.clone()).or_default().push(ClipPart { clip: clip.id.clone(), offset: *offset, duration: *duration, gain: *gain, start: clip.start });
            }
        }
    }
    by_asset
        .into_iter()
        .filter_map(|(id, parts)| Some((id.clone(), Arc::clone(&session.pool.get(&id)?.native), session.project.asset(&id)?.clone(), parts)))
        .collect()
}

fn frames_of(native: &AudioBuffer, offset: f64, duration: f64) -> std::ops::Range<usize> {
    let rate = f64::from(native.sample_rate);
    let n = native.frames();
    let a = ((offset * rate).round().max(0.0) as usize).min(n);
    a..(((offset + duration) * rate).round().max(0.0) as usize).clamp(a, n)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerLevel {
    pub track: String,
    /// Integrated loudness of what the track plays (clip gain included).
    pub loudness: Option<f64>,
    pub seconds: f64,
}

/// How loud each track's clips are, to see whether the speakers match.
#[tauri::command]
pub async fn podcast_measure(state: State<'_, AppState>, tracks: Vec<String>) -> CmdResult<Vec<SpeakerLevel>> {
    let owners: Vec<(String, Vec<String>)> = {
        let session = lock(&state.session);
        tracks
            .iter()
            .filter_map(|id| session.project.track(id))
            .map(|t| (t.id.clone(), t.timeline().map(|c| c.id.clone()).collect()))
            .collect()
    };
    let groups = clips_by_asset(&state, &tracks);
    Ok(owners
        .into_iter()
        .map(|(track, clips)| {
            // Energy-weighted mean of each clip's loudness.
            let (mut energy, mut seconds) = (0.0, 0.0);
            for (_, native, _, parts) in &groups {
                for part in parts.iter().filter(|p| clips.contains(&p.clip)) {
                    let piece = soniq_io::extract(native, frames_of(native, part.offset, part.duration));
                    if let Some(lufs) = soniq_io::integrated_loudness(&piece) {
                        energy += part.duration * 10f64.powf((lufs + part.gain) / 10.0);
                        seconds += part.duration;
                    }
                }
            }
            SpeakerLevel { track, loudness: (seconds > 0.0).then(|| 10.0 * (energy / seconds).log10()), seconds }
        })
        .collect())
}

/// Levels every clip on the speakers' tracks to `target` LUFS, in one undo step.
#[tauri::command]
pub async fn podcast_level(state: State<'_, AppState>, tracks: Vec<String>, target: f64) -> CmdResult<ProjectReply> {
    let groups = clips_by_asset(&state, &tracks);
    if groups.is_empty() {
        return Err("The speakers have no audio to level yet: record or import some first.".into());
    }
    let target = target.clamp(-30.0, -10.0);
    let (mut levelled, mut skipped) = (0, 0);
    let mut edited_assets = Vec::new();
    for (_, native, asset, parts) in groups {
        let mut edited = (*native).clone();
        let mut done = Vec::new();
        let mut seen = Vec::new();
        for part in parts {
            let range = frames_of(&native, part.offset, part.duration);
            if seen.contains(&(range.start, range.end)) {
                done.push(part.clip);
                continue;
            }
            seen.push((range.start, range.end));
            let piece = soniq_io::extract(&edited, range.clone());
            match soniq_io::apply_effect(&piece, &Effect::Level { target: target - part.gain, max_gain: MAX_LEVEL_GAIN }, &Default::default()) {
                Ok(out) => {
                    // Leveling keeps the length: write it straight back.
                    for (ch, new) in edited.channels.iter_mut().zip(&out.buffer.channels) {
                        ch[range.clone()].copy_from_slice(new);
                    }
                    levelled += 1;
                    done.push(part.clip);
                }
                Err(_) => skipped += 1,
            }
        }
        if !done.is_empty() {
            edited_assets.push((asset, done, edited));
        }
    }
    if edited_assets.is_empty() {
        return Err("The speakers' clips are too short or too quiet to level (each needs a few seconds of speech).".into());
    }

    let rate = state.engine.sample_rate();
    let mut session = lock(&state.session);
    let session = &mut *session;
    let before = session.project.clone();
    for (old, clips, edited) in edited_assets {
        let mut asset = old.clone();
        asset.id = new_id("a");
        asset.path = None;
        asset.frames = edited.frames() as u64;
        session.pool.insert(asset.id.clone(), PoolAsset::prepare(edited, rate));
        for clip in clips {
            if let Some((ti, ci)) = session.project.find_clip(&clip)
                && let ClipContent::Audio { asset: playing, .. } = &mut session.project.tracks[ti].clips[ci].content
                && *playing == old.id
            {
                *playing = asset.id.clone();
            }
        }
        session.project.assets.push(asset);
    }
    session.project.prune_assets();
    session.history.record(before, "Level speakers", None);
    session.mark_changed();
    session.collect_garbage();
    state.sync_engine(session);
    let mut reply = state.reply(session);
    reply.note = Some(match (levelled, skipped) {
        (1, 0) => format!("1 clip levelled to {target:.0} LUFS"),
        (n, 0) => format!("{n} clips levelled to {target:.0} LUFS"),
        (n, s) => format!("{n} clips levelled to {target:.0} LUFS; {s} too short or quiet to measure"),
    });
    Ok(reply)
}

// ---------- Ducking ----------

/// Where anyone is speaking, as (start, end) seconds on the timeline.
fn speech(state: &AppState, speakers: &[String], project: &Project) -> Vec<(f64, f64)> {
    let tempo = project.tempo_map();
    let window = 0.05;
    let mut regions = Vec::new();
    // Each track's own background and level decide what counts as speech.
    for track in speakers {
        let groups = clips_by_asset(state, std::slice::from_ref(track));
        let mut levels: Vec<(f64, Vec<f64>)> = Vec::new();
        for (_, native, _, parts) in &groups {
            let w = ((window * f64::from(native.sample_rate)) as usize).max(1);
            for part in parts {
                let range = frames_of(native, part.offset, part.duration);
                let channels = native.channel_count().max(1) as f64;
                let dbs: Vec<f64> = range
                    .clone()
                    .step_by(w)
                    .map(|a| {
                        let e = (a + w).min(range.end);
                        let sum: f64 = native.channels.iter().map(|c| c[a..e].iter().map(|s| f64::from(*s).powi(2)).sum::<f64>()).sum();
                        10.0 * (sum / ((e - a).max(1) as f64 * channels)).max(1e-14).log10() + part.gain
                    })
                    .collect();
                levels.push((tempo.seconds_at(part.start), dbs));
            }
        }
        let mut sorted: Vec<f64> = levels.iter().flat_map(|(_, l)| l.iter().copied()).filter(|l| *l > -100.0).collect();
        if sorted.is_empty() {
            continue;
        }
        sorted.sort_by(f64::total_cmp);
        let at = |p: f64| sorted[((sorted.len() - 1) as f64 * p).round() as usize];
        let gate = (at(0.1) + 12.0).max(-60.0).min(at(0.9) - 15.0);
        for (start, dbs) in levels {
            for (i, db) in dbs.iter().enumerate() {
                if *db > gate {
                    regions.push((start + i as f64 * window, start + (i + 1) as f64 * window));
                }
            }
        }
    }
    regions.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Join speech that comes close together; drop stray blips.
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (a, b) in regions {
        match merged.last_mut() {
            Some(last) if a - last.1 < DUCK_HOLD => last.1 = last.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    merged.retain(|(a, b)| b - a >= 0.4);
    merged
}

/// Volume points (in seconds and dB) that hold `base` and dip to `low` under each region.
fn duck_points(regions: &[(f64, f64)], base: f64, low: f64) -> Vec<(f64, f64)> {
    let mut points = Vec::new();
    for &(s, e) in regions {
        let from = (s - DUCK_LEAD).max(points.last().map_or(0.0, |p: &(f64, f64)| p.0));
        let down = (from + DUCK_FALL).min(s.max(from));
        if down > from {
            points.push((from, base));
        }
        points.push((down, low));
        points.push((e + DUCK_STAY, low));
        points.push((e + DUCK_STAY + DUCK_RISE, base));
    }
    points
}

/// Writes a volume lane on each music track that dips by `depth` dB
/// wherever a speaker talks, replacing the lane it had. One undo step.
#[tauri::command]
pub async fn podcast_duck(state: State<'_, AppState>, music: Vec<String>, speakers: Vec<String>, depth: f64) -> CmdResult<ProjectReply> {
    if music.is_empty() {
        return Err("Pick a music track to duck first.".into());
    }
    let project = lock(&state.session).project.clone();
    let regions = speech(&state, &speakers, &project);
    if regions.is_empty() {
        return Err("Found no speech on the speakers' tracks to duck under.".into());
    }
    let tempo = project.tempo_map();
    let depth = depth.clamp(-40.0, -3.0);

    let mut session = lock(&state.session);
    let session = &mut *session;
    let before = session.project.clone();
    let mut ducked = Vec::new();
    for track in session.project.tracks.iter_mut().filter(|t| music.contains(&t.id) && t.kind != TrackKind::Group) {
        let base = track.volume;
        let points = duck_points(&regions, base, (base + depth).max(-60.0))
            .into_iter()
            .map(|(sec, value)| AutoPoint { time: tempo.beats_at(sec), value })
            .collect();
        track.automation.retain(|l| l.target != AutoTarget::Volume);
        track.automation.push(AutomationLane { target: AutoTarget::Volume, points });
        ducked.push(track.name.clone());
    }
    if ducked.is_empty() {
        return Err("Those music tracks no longer exist.".into());
    }
    session.history.record(before, "Duck music under speech", None);
    session.mark_changed();
    state.sync_engine(session);
    let mut reply = state.reply(session);
    let times = if regions.len() == 1 { "once".to_string() } else { format!("{} times", regions.len()) };
    reply.note = Some(format!("{} dips {:.0} dB under speech, {times}. Its volume lane shows each dip.", ducked.join(" and "), -depth));
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soniq_project::Marker;

    #[test]
    fn chapters_come_from_markers_in_the_range() {
        let mut p = Project::empty("Show");
        p.bpm = 120.0; // two beats a second
        let mark = |name: &str, start: f64, end: Option<f64>| Marker { id: new_id("m"), name: name.into(), start, end };
        p.markers = vec![mark("Outro", 100.0, None), mark("Intro", 0.0, None), mark("", 20.0, Some(30.0)), mark("Before", -1.0, None)];
        let list = chapters(&p, 0.0, 60.0);
        let got: Vec<(&str, u32, u32)> = list.iter().map(|c| (c.title.as_str(), c.start_ms, c.end_ms)).collect();
        assert_eq!(got, [("Intro", 0, 10_000), ("Chapter 2", 10_000, 15_000), ("Outro", 50_000, 60_000)]);
        // From a later start, times count from there.
        assert_eq!(chapters(&p, 10.0, 60.0)[0].start_ms, 0);
        let txt = chapters_text(&list, ChaptersFile::Txt, "Show", "show.mp3");
        assert_eq!(txt.lines().collect::<Vec<_>>(), ["00:00 Intro", "00:10 Chapter 2", "00:50 Outro"]);
        let json: serde_json::Value = serde_json::from_str(&chapters_text(&list, ChaptersFile::Json, "Show", "show.mp3")).unwrap();
        assert_eq!(json["chapters"][1]["startTime"], 10.0);
        let cue = chapters_text(&list, ChaptersFile::Cue, "Show", "show.mp3");
        assert!(cue.contains("TRACK 03 AUDIO") && cue.contains("INDEX 01 00:50:00"), "{cue}");
    }

    #[test]
    fn ducking_dips_under_each_stretch_of_speech_and_comes_back() {
        let points = duck_points(&[(5.0, 9.0), (20.0, 21.0)], -10.0, -22.0);
        let times: Vec<f64> = points.iter().map(|p| p.0).collect();
        assert!(times.windows(2).all(|w| w[0] <= w[1]), "in order: {times:?}");
        let near = |p: (f64, f64), t: f64, v: f64| (p.0 - t).abs() < 1e-9 && p.1 == v;
        assert!(near(points[0], 4.4, -10.0), "{points:?}");
        assert!(near(points[1], 4.9, -22.0), "down before the voice starts");
        assert!(near(points[2], 9.4, -22.0) && near(points[3], 10.4, -10.0), "back up after it");
        assert_eq!(points.len(), 8);
        // Speech right at the start: the music begins already down.
        assert_eq!(duck_points(&[(0.0, 2.0)], 0.0, -12.0)[0], (0.0, -12.0));
    }
}
