//! Standard MIDI Files: reading parts out of `.mid` files and writing MIDI
//! tracks back out.

use std::collections::{BTreeMap, HashMap, VecDeque};

use midly::num::{u4, u7, u15, u24, u28};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};
use soniq_project::{Note, tidy_notes};

/// Ticks per beat in the files Soniq writes.
const PPQ: u16 = 480;
/// The General MIDI drum channel (10, counted from 1).
pub const DRUM_CHANNEL: u8 = 9;

/// The notes one channel of one track plays.
#[derive(Debug, Clone, PartialEq)]
pub struct MidiPart {
    pub name: String,
    /// 0–15; 9 is the General MIDI drum channel.
    pub channel: u8,
    /// The first program change, if any.
    pub program: Option<u8>,
    /// Positions in beats from the start of the song.
    pub notes: Vec<Note>,
}

impl MidiPart {
    pub fn is_drums(&self) -> bool {
        self.channel == DRUM_CHANNEL
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MidiSong {
    /// The first tempo in the file.
    pub bpm: Option<f64>,
    /// The first time signature, in quarter-note beats per bar.
    pub beats_per_bar: Option<u32>,
    pub parts: Vec<MidiPart>,
}

/// Reads every part with notes out of a Standard MIDI File.
pub fn read_midi(bytes: &[u8]) -> Result<MidiSong, String> {
    let smf = Smf::parse(bytes).map_err(|e| format!("not a MIDI file ({e})"))?;
    let mut bpm = None;
    let mut beats_per_bar = None;
    // Ticks to beats. SMPTE-timed files are rare; they assume 120 bpm.
    let ticks_per_beat = match smf.header.timing {
        Timing::Metrical(ppq) => f64::from(ppq.as_int().max(1)),
        Timing::Timecode(fps, sub) => f64::from(fps.as_f32()) * f64::from(sub.max(1)) * 0.5,
    };

    let mut parts = Vec::new();
    for (index, track) in smf.tracks.iter().enumerate() {
        let mut tick: u64 = 0;
        let mut name = None;
        let mut programs: HashMap<u8, u8> = HashMap::new();
        let mut open: HashMap<(u8, u8), VecDeque<(u64, u8)>> = HashMap::new();
        let mut notes: BTreeMap<u8, Vec<Note>> = BTreeMap::new();
        let close = |channel: u8, start: u64, end: u64, key: u8, vel: u8, notes: &mut BTreeMap<u8, Vec<Note>>| {
            notes.entry(channel).or_default().push(Note {
                start: start as f64 / ticks_per_beat,
                length: (end.saturating_sub(start)) as f64 / ticks_per_beat,
                pitch: key,
                velocity: f64::from(vel) / 127.0,
            });
        };
        for event in track {
            tick += u64::from(event.delta.as_int());
            match event.kind {
                TrackEventKind::Midi { channel, message } => {
                    let channel = channel.as_int();
                    match message {
                        MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                            open.entry((channel, key.as_int())).or_default().push_back((tick, vel.as_int()));
                        }
                        MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                            // The earliest open note of that key ends first.
                            if let Some((start, vel)) = open.get_mut(&(channel, key.as_int())).and_then(VecDeque::pop_front) {
                                close(channel, start, tick, key.as_int(), vel, &mut notes);
                            }
                        }
                        MidiMessage::ProgramChange { program } => {
                            programs.entry(channel).or_insert(program.as_int());
                        }
                        _ => {}
                    }
                }
                TrackEventKind::Meta(MetaMessage::Tempo(us)) if bpm.is_none() => {
                    bpm = Some((60_000_000.0 / f64::from(us.as_int().max(1)) * 100.0).round() / 100.0);
                }
                TrackEventKind::Meta(MetaMessage::TimeSignature(num, den_pow, _, _)) if beats_per_bar.is_none() => {
                    let quarters = f64::from(num) * 4.0 / 2f64.powi(i32::from(den_pow));
                    beats_per_bar = Some((quarters.round() as u32).clamp(1, 32));
                }
                TrackEventKind::Meta(MetaMessage::TrackName(bytes)) if name.is_none() => {
                    let text = String::from_utf8_lossy(bytes).trim().to_string();
                    name = (!text.is_empty()).then_some(text);
                }
                _ => {}
            }
        }
        // Notes still held at the end of the track end there.
        for ((channel, key), starts) in open {
            for (start, vel) in starts {
                close(channel, start, tick, key, vel, &mut notes);
            }
        }
        let channels = notes.len();
        for (channel, mut list) in notes {
            tidy_notes(&mut list);
            let base = name.clone().unwrap_or_else(|| if channel == DRUM_CHANNEL { "Drums".into() } else { format!("Track {}", index + 1) });
            let name = if channels > 1 { format!("{base} (ch {})", channel + 1) } else { base };
            parts.push(MidiPart { name, channel, program: programs.get(&channel).copied(), notes: list });
        }
    }
    Ok(MidiSong { bpm, beats_per_bar, parts })
}

/// One track to write: notes in beats from the start of the song.
#[derive(Debug, Clone)]
pub struct ExportPart {
    pub name: String,
    pub channel: u8,
    /// A General MIDI program, so other apps (and Soniq) pick a fitting sound.
    pub program: Option<u8>,
    pub notes: Vec<Note>,
}

/// A type-1 MIDI file: a tempo track, then one track per part.
pub fn write_midi(name: &str, parts: &[ExportPart], bpm: f64, beats_per_bar: u32) -> Result<Vec<u8>, String> {
    let ticks = |beats: f64| (beats.max(0.0) * f64::from(PPQ)).round() as u64;
    let delta = |d: u64| u28::new(d.min(u64::from(u28::max_value().as_int())) as u32);

    let mut tracks: Vec<Vec<TrackEvent>> = Vec::with_capacity(parts.len() + 1);
    let tempo = (60_000_000.0 / bpm.max(1.0)).round() as u32;
    tracks.push(vec![
        TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::TrackName(name.as_bytes())) },
        TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new(tempo.min(0xff_ffff)))) },
        TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::TimeSignature(beats_per_bar.clamp(1, 255) as u8, 2, 24, 8)) },
        TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::EndOfTrack) },
    ]);

    for part in parts {
        let channel = u4::new(part.channel.min(15));
        // (tick, is_on, key, velocity), offs before ons at the same tick.
        let mut events: Vec<(u64, bool, u8, u8)> = Vec::with_capacity(part.notes.len() * 2);
        for n in &part.notes {
            let (start, end) = (ticks(n.start), ticks(n.start + n.length));
            let vel = (n.velocity * 127.0).round().clamp(1.0, 127.0) as u8;
            events.push((start, true, n.pitch.min(127), vel));
            events.push((end.max(start + 1), false, n.pitch.min(127), 0));
        }
        events.sort_by_key(|e| (e.0, e.1));
        let mut track = vec![TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::TrackName(part.name.as_bytes())) }];
        if let Some(program) = part.program {
            let message = MidiMessage::ProgramChange { program: u7::new(program.min(127)) };
            track.push(TrackEvent { delta: u28::new(0), kind: TrackEventKind::Midi { channel, message } });
        }
        let mut last = 0;
        for (tick, on, key, vel) in events {
            let message = if on {
                MidiMessage::NoteOn { key: u7::new(key), vel: u7::new(vel) }
            } else {
                MidiMessage::NoteOff { key: u7::new(key), vel: u7::new(64) }
            };
            track.push(TrackEvent { delta: delta(tick - last), kind: TrackEventKind::Midi { channel, message } });
            last = tick;
        }
        track.push(TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::EndOfTrack) });
        tracks.push(track);
    }

    let smf = Smf { header: Header::new(Format::Parallel, Timing::Metrical(u15::new(PPQ))), tracks };
    let mut out = Vec::new();
    smf.write_std(&mut out).map_err(|e| format!("Could not write the MIDI file: {e}"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start: f64, length: f64, pitch: u8, velocity: f64) -> Note {
        Note { start, length, pitch, velocity }
    }

    #[test]
    fn written_files_read_back_the_same() {
        let keys = vec![note(0.0, 1.0, 60, 100.0 / 127.0), note(0.0, 1.0, 64, 0.5), note(1.5, 0.25, 67, 1.0)];
        let drums = vec![note(0.0, 0.25, 36, 1.0), note(1.0, 0.25, 38, 0.8)];
        let parts = [
            ExportPart { name: "Keys".into(), channel: 0, program: Some(4), notes: keys.clone() },
            ExportPart { name: "Drums".into(), channel: DRUM_CHANNEL, program: None, notes: drums },
        ];
        let bytes = write_midi("Song", &parts, 112.0, 3).unwrap();
        let song = read_midi(&bytes).unwrap();
        assert_eq!(song.bpm, Some(112.0));
        assert_eq!(song.beats_per_bar, Some(3));
        assert_eq!(song.parts.len(), 2);
        assert_eq!(song.parts[0].name, "Keys");
        assert_eq!(song.parts[0].program, Some(4));
        assert!(song.parts[1].is_drums());
        for (a, b) in song.parts[0].notes.iter().zip(&keys) {
            assert_eq!((a.start, a.length, a.pitch), (b.start, b.length, b.pitch));
            assert!((a.velocity - b.velocity).abs() < 0.005);
        }
    }

    #[test]
    fn one_track_with_two_channels_becomes_two_parts() {
        // A type-0 file: everything in one track, the drums on channel 10.
        let track = vec![
            TrackEvent { delta: u28::new(0), kind: TrackEventKind::Midi { channel: u4::new(0), message: MidiMessage::NoteOn { key: u7::new(60), vel: u7::new(90) } } },
            TrackEvent { delta: u28::new(0), kind: TrackEventKind::Midi { channel: u4::new(9), message: MidiMessage::NoteOn { key: u7::new(36), vel: u7::new(90) } } },
            TrackEvent { delta: u28::new(480), kind: TrackEventKind::Midi { channel: u4::new(0), message: MidiMessage::NoteOn { key: u7::new(60), vel: u7::new(0) } } },
            TrackEvent { delta: u28::new(0), kind: TrackEventKind::Midi { channel: u4::new(9), message: MidiMessage::NoteOff { key: u7::new(36), vel: u7::new(0) } } },
            TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::EndOfTrack) },
        ];
        let smf = Smf { header: Header::new(Format::SingleTrack, Timing::Metrical(u15::new(480))), tracks: vec![track] };
        let mut bytes = Vec::new();
        smf.write_std(&mut bytes).unwrap();
        let song = read_midi(&bytes).unwrap();
        assert_eq!(song.parts.len(), 2);
        assert_eq!(song.parts[0].notes, vec![note(0.0, 1.0, 60, 90.0 / 127.0)]);
        assert!(song.parts[1].is_drums());
    }

    #[test]
    fn garbage_is_refused() {
        assert!(read_midi(b"RIFF....WAVE").is_err());
    }
}
