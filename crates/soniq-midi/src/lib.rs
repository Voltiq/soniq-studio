//! MIDI for Soniq Studio: hardware inputs (midir) and Standard MIDI Files
//! (midly).

mod file;
mod input;

pub use file::{ExportPart, MidiPart, MidiSong, read_midi, write_midi};
pub use input::{MidiInputs, MidiPortStatus};

/// The channel-voice messages Soniq acts on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MidiEvent {
    /// Velocity 0–1.
    NoteOn { channel: u8, pitch: u8, velocity: f32 },
    NoteOff { channel: u8, pitch: u8 },
    /// The sustain pedal (CC 64).
    Sustain { channel: u8, down: bool },
    /// CC 123 (all notes off) or CC 120 (all sound off).
    AllNotesOff { channel: u8 },
    /// Any other controller: a knob, fader or button, value 0–127.
    Control { channel: u8, cc: u8, value: u8 },
}

/// Parses one raw MIDI message; `None` for anything Soniq ignores.
pub fn parse(bytes: &[u8]) -> Option<MidiEvent> {
    let (&status, data) = bytes.split_first()?;
    let channel = status & 0x0f;
    let at = |i: usize| data.get(i).copied().map(|b| b & 0x7f);
    match status & 0xf0 {
        // A note-on with velocity 0 is a note-off (running-status keyboards send these).
        0x90 if at(1)? > 0 => Some(MidiEvent::NoteOn { channel, pitch: at(0)?, velocity: f32::from(at(1)?) / 127.0 }),
        0x80 | 0x90 => Some(MidiEvent::NoteOff { channel, pitch: at(0)? }),
        0xb0 => match at(0)? {
            64 => Some(MidiEvent::Sustain { channel, down: at(1)? >= 64 }),
            120 | 123 => Some(MidiEvent::AllNotesOff { channel }),
            // Channel-mode messages (121–127) are not controls.
            cc if cc < 120 => Some(MidiEvent::Control { channel, cc, value: at(1)? }),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_notes_pedal_and_panic() {
        assert_eq!(parse(&[0x91, 60, 127]), Some(MidiEvent::NoteOn { channel: 1, pitch: 60, velocity: 1.0 }));
        assert_eq!(parse(&[0x90, 60, 0]), Some(MidiEvent::NoteOff { channel: 0, pitch: 60 }));
        assert_eq!(parse(&[0xb0, 64, 100]), Some(MidiEvent::Sustain { channel: 0, down: true }));
        assert_eq!(parse(&[0xb0, 123, 0]), Some(MidiEvent::AllNotesOff { channel: 0 }));
        assert_eq!(parse(&[0xb2, 21, 90]), Some(MidiEvent::Control { channel: 2, cc: 21, value: 90 }));
        assert_eq!(parse(&[0xb0, 121, 0]), None, "reset all controllers is not a control");
        assert_eq!(parse(&[0xf8]), None, "clock ticks are ignored");
        assert_eq!(parse(&[0x90, 60]), None, "truncated");
    }
}
