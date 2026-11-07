//! "Night Drive", the demo song. Drums, bass and keys are MIDI played by the
//! built-in Drum Rack, Synth and Electric Piano; the pad is rendered to audio
//! so the audio editor has something to work on out of the box.

use soniq_engine::demo::{DemoNote, Voice, render_part};
use soniq_project::{Asset, Clip, Note, Project, SCHEMA_VERSION, Track, TrackKind, catalog, default_returns, new_id};

use crate::session::{Pool, PoolAsset, Session};

const BPM: f64 = 112.0;
const BARS: usize = 24;

// A minor: Am – F – C – G, one bar each.
const ROOTS: [u8; 4] = [45, 41, 48, 43];
const CHORDS: [[u8; 3]; 4] = [[57, 60, 64], [53, 57, 60], [55, 60, 64], [55, 59, 62]];

fn note(start: f64, length: f64, pitch: u8, velocity: f32) -> DemoNote {
    DemoNote { start, length, pitch, velocity }
}

fn drum_notes() -> Vec<DemoNote> {
    let mut notes = Vec::new();
    for bar in 0..BARS {
        let o = bar as f64 * 4.0;
        let intro = bar < 4;
        let kicks: &[f64] = if intro { &[0.0, 2.0] } else { &[0.0, 1.5, 2.0, 3.5] };
        notes.extend(kicks.iter().map(|k| note(o + k, 0.25, 36, 0.95)));
        if !intro {
            notes.extend([1.0, 3.0].iter().map(|s| note(o + s, 0.25, 38, 0.85)));
        }
        notes.extend((0..8).map(|h| note(o + h as f64 * 0.5, 0.2, 42, if h % 2 == 1 { 0.45 } else { 0.7 })));
        if bar % 4 == 3 && !intro {
            notes.push(note(o + 3.5, 0.5, 46, 0.6));
        }
    }
    notes
}

fn bass_notes(bars: usize) -> Vec<DemoNote> {
    (0..bars)
        .flat_map(|bar| {
            let o = bar as f64 * 4.0;
            let root = ROOTS[bar % 4];
            [
                note(o, 0.75, root, 0.9),
                note(o + 0.75, 0.25, root + 12, 0.6),
                note(o + 1.5, 0.5, root, 0.75),
                note(o + 2.5, 0.5, root + 7, 0.7),
                note(o + 3.0, 0.75, root, 0.8),
            ]
        })
        .collect()
}

fn pad_notes(bars: usize) -> Vec<DemoNote> {
    (0..bars).flat_map(|bar| CHORDS[bar % 4].map(|p| note(bar as f64 * 4.0, 4.0, p - 12, 0.8))).collect()
}

fn chord_notes(bars: usize, lift: u8) -> Vec<Note> {
    (0..bars)
        .flat_map(|bar| CHORDS[bar % 4].map(|p| Note { start: bar as f64 * 4.0, length: 3.75, pitch: p + lift, velocity: 0.7 }))
        .collect()
}

struct Rendered {
    asset: Asset,
    pooled: PoolAsset,
}

fn render(name: &str, notes: &[DemoNote], voice: Voice, rate: u32, beats: f64) -> Rendered {
    let buffer = render_part(notes, voice, rate, BPM, beats);
    let asset = Asset {
        id: new_id("a"),
        name: name.into(),
        channels: buffer.channel_count() as u16,
        sample_rate: buffer.sample_rate,
        frames: buffer.frames() as u64,
        path: None,
        missing: false,
    };
    Rendered { asset, pooled: PoolAsset::prepare(buffer, rate) }
}

fn audio_clip(name: &str, asset: &Asset, start_beat: f64, offset_beats: f64, beats: f64) -> Clip {
    let sec = 60.0 / BPM;
    Clip::audio(name, &asset.id, start_beat, offset_beats * sec, beats * sec)
}

fn midi_clip(name: &str, start: f64, length: f64, notes: Vec<Note>) -> Clip {
    Clip::midi(name, start, length, notes)
}

/// The notes of `part` between `from` and `to` beats, moved to start at 0.
fn notes_between(part: &[DemoNote], from: f64, to: f64) -> Vec<Note> {
    part.iter()
        .filter(|n| n.start >= from && n.start < to)
        .map(|n| Note { start: n.start - from, length: n.length, pitch: n.pitch, velocity: f64::from(n.velocity) })
        .collect()
}

fn slots(names: [Option<&str>; 5]) -> Vec<Option<String>> {
    names.iter().map(|n| n.map(str::to_string)).collect()
}

pub fn session(rate: u32) -> Session {
    let total = BARS as f64 * 4.0;
    let pad = render("Warm pad", &pad_notes(BARS), Voice::Pad, rate, total);

    let drums = drum_notes();
    let mut drum_track = Track::new(new_id("t"), "Drums", TrackKind::Midi, 2);
    drum_track.volume = -6.0;
    drum_track.clips = vec![
        midi_clip("Intro", 0.0, 16.0, notes_between(&drums, 0.0, 16.0)),
        midi_clip("Groove", 16.0, total - 16.0, notes_between(&drums, 16.0, total)),
    ];
    drum_track.devices = [catalog::device("drum-rack", &[]), catalog::device("comp", &[("thresh", -14.0), ("ratio", 3.0)])].into_iter().flatten().collect();
    drum_track.sends = [0.12, 0.0];
    drum_track.slots = slots([Some("Intro"), Some("Groove"), Some("Groove"), None, None]);

    // A plucky mono bass: saw and a square an octave down, a closing filter.
    let bass_sound = [
        ("osc2", 1.0),
        ("osc2Semi", -12.0),
        ("detune", 0.0),
        ("mix", 35.0),
        ("sub", 30.0),
        ("cutoff", 420.0),
        ("res", 28.0),
        ("envAmt", 55.0),
        ("fDecay", 180.0),
        ("fSustain", 10.0),
        ("attack", 2.0),
        ("sustain", 70.0),
        ("release", 60.0),
        ("mode", 1.0),
        ("volume", 4.0),
    ];
    let mut bass_track = Track::new(new_id("t"), "Bass", TrackKind::Midi, 1);
    bass_track.volume = -4.0;
    bass_track.clips = vec![midi_clip("Bassline", 16.0, total - 16.0, notes_between(&bass_notes(BARS - 4), 0.0, total))];
    bass_track.devices = [catalog::device("synth", &bass_sound), catalog::device("saturator", &[("drive", 9.0)])].into_iter().flatten().collect();
    bass_track.slots = slots([None, Some("Bassline"), Some("Bassline"), None, None]);

    let mut keys = Track::new(new_id("t"), "Keys", TrackKind::Midi, 6);
    keys.volume = -8.0;
    keys.pan = -0.2;
    keys.clips = vec![
        midi_clip("Chords", 0.0, 32.0, chord_notes(8, 0)),
        midi_clip("Chords", 32.0, 32.0, chord_notes(8, 0)),
        midi_clip("Chords high", 64.0, 32.0, chord_notes(8, 12)),
    ];
    keys.devices = [catalog::device("epiano", &[]), catalog::device("reverb", &[("mix", 18.0)])].into_iter().flatten().collect();
    keys.sends = [0.3, 0.15];
    keys.slots = slots([Some("Chords"), Some("Chords"), Some("Chords high"), None, None]);

    let mut pad_track = Track::new(new_id("t"), "Pad", TrackKind::Audio, 3);
    pad_track.volume = -9.0;
    pad_track.clips = vec![audio_clip("Warm pad", &pad.asset, 0.0, 0.0, total)];
    pad_track.devices = [catalog::device("eq", &[("b2freq", 150.0), ("b2gain", -6.0)]), catalog::device("reverb", &[("size", 80.0), ("mix", 40.0)])]
        .into_iter()
        .flatten()
        .collect();
    pad_track.sends = [0.25, 0.0];
    pad_track.slots = slots([Some("Warm pad"), Some("Warm pad"), None, None, Some("Warm pad")]);

    let mut vocals = Track::new(new_id("t"), "Vocals", TrackKind::Audio, 5);
    let vocal_eq = [("b1on", 1.0), ("b1freq", 80.0), ("b2gain", -6.0), ("b7gain", 2.0)];
    vocals.devices = [catalog::device("eq", &vocal_eq), catalog::device("comp", &[])].into_iter().flatten().collect();

    let mut project = Project {
        mixes: Vec::new(),
        other_mix: None,
        version: SCHEMA_VERSION,
        id: new_id("p"),
        name: "Night Drive (demo)".into(),
        bpm: BPM,
        beats_per_bar: 4,
        key: "A minor".into(),
        tracks: vec![drum_track, bass_track, keys, pad_track, vocals],
        returns: default_returns(),
        scenes: ["Intro", "Verse", "Hook", "Break", "Outro"].iter().map(|s| s.to_string()).collect(),
        master_volume: 0.0,
        assets: vec![pad.asset.clone()],
        markers: Vec::new(),
        mappings: Vec::new(),
        master_devices: Vec::new(),
        launch_quantize: 4.0,
        tempo: Vec::new(),
    };
    // The session's slots hold copies of the arrangement's clips.
    project.migrate_slots();

    let mut pool = Pool::default();
    pool.insert(pad.asset.id, pad.pooled);
    Session::new(project, pool, None)
}
