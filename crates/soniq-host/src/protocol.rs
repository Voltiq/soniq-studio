//! What the app and a plugin's sandbox process say to each other. Commands
//! go down the sandbox's stdin; replies (one per command, in order) and
//! notices (any time) come back on its stdout. Each message is a frame: a
//! JSON header and a binary payload (a plugin's saved state), each preceded
//! by its length.

use std::io::{self, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Largest frame part accepted, so a corrupt length cannot exhaust memory.
const MAX_PART: usize = 1 << 30;

/// A plugin a `.clap` file offers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub description: String,
    /// CLAP features: "instrument", "audio-effect", "reverb"…
    pub features: Vec<String>,
}

impl Descriptor {
    /// It makes sound from notes rather than changing a track's audio.
    pub fn is_instrument(&self) -> bool {
        self.features.iter().any(|f| f == "instrument")
    }

    /// It only changes notes: not hosted yet.
    pub fn is_note_effect(&self) -> bool {
        !self.is_instrument() && !self.features.iter().any(|f| f == "audio-effect" || f == "analyzer") && self.features.iter().any(|f| f == "note-effect")
    }
}

/// One of a plugin's parameters, with its value now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamInfo {
    pub id: u32,
    pub name: String,
    /// Where the plugin files it ("Filter/Envelope"), or empty.
    pub module: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub value: f64,
    /// The value as the plugin writes it ("-6.0 dB").
    pub text: String,
    pub stepped: bool,
    /// How many steps a stepped value takes from `min` to `max` (0: one per unit).
    #[serde(default)]
    pub steps: u32,
    pub automatable: bool,
    pub hidden: bool,
    pub readonly: bool,
}

/// What a loaded plugin turned out to be.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Loaded {
    pub name: String,
    /// Channels of its main audio input and output (0: none).
    pub inputs: u32,
    pub outputs: u32,
    /// It takes notes.
    pub notes: bool,
    /// It has a window of its own.
    pub gui: bool,
    /// Samples it delays its output by.
    pub latency: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Command {
    /// Loads the plugin `id` from `path`, opens the shared memory `shm` and
    /// starts at `rate`. Replies `Loaded`.
    Load { path: String, id: String, shm: String, rate: f64 },
    /// The engine's sample rate changed.
    Activate { rate: f64 },
    /// Replies `Params` with every parameter.
    Params,
    /// Replies `Texts` with the value and text of these parameters.
    Texts { ids: Vec<u32> },
    /// Replies `State`, the state in the payload.
    SaveState,
    /// The state is in the payload.
    LoadState,
    ShowGui { title: String },
    HideGui,
    Quit,
}

/// Which part of a gesture on the plugin's own window a parameter change is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Gesture {
    /// A change on its own (a preset, a click).
    None,
    Begin,
    /// The knob was let go: the value is final.
    End,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Message {
    // Replies.
    Done,
    Failed { message: String },
    Loaded { info: Loaded },
    Params { params: Vec<ParamInfo> },
    Texts { texts: Vec<(u32, f64, String)> },
    /// The state is in the payload.
    State,
    Scanned { plugins: Vec<Descriptor> },
    // Notices.
    /// The plugin changed a parameter itself (its window, a preset). The
    /// start and end of a gesture carry no value.
    ParamChanged {
        id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<f64>,
        gesture: Gesture,
    },
    GuiClosed,
    /// Its state changed in a way parameters do not show.
    StateChanged,
    LatencyChanged { latency: u32 },
    /// Its parameters changed (names, ranges, how many).
    ParamsChanged,
    /// The sandbox process ended (made up on the app's side when its pipe closes).
    Exited,
}

impl Message {
    /// Not a reply to a command.
    pub fn is_notice(&self) -> bool {
        matches!(self, Self::ParamChanged { .. } | Self::GuiClosed | Self::StateChanged | Self::LatencyChanged { .. } | Self::ParamsChanged | Self::Exited)
    }
}

pub fn write_frame(out: &mut impl Write, header: &impl Serialize, payload: &[u8]) -> io::Result<()> {
    let json = serde_json::to_vec(header).map_err(io::Error::other)?;
    let mut frame = Vec::with_capacity(8 + json.len() + payload.len());
    frame.extend_from_slice(&(json.len() as u32).to_le_bytes());
    frame.extend_from_slice(&json);
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);
    out.write_all(&frame)?;
    out.flush()
}

fn read_part(input: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    input.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_PART {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut part = vec![0u8; len];
    input.read_exact(&mut part)?;
    Ok(part)
}

/// Reads one frame. A header that does not parse is `InvalidData`, with the
/// whole frame consumed, so the next one can still be read.
pub fn read_frame<T: DeserializeOwned>(input: &mut impl Read) -> io::Result<(T, Vec<u8>)> {
    let json = read_part(input)?;
    let payload = read_part(input)?;
    let header = serde_json::from_slice(&json).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok((header, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_with_their_payload() {
        let mut wire = Vec::new();
        write_frame(&mut wire, &Command::LoadState, &[1, 2, 3]).unwrap();
        write_frame(&mut wire, &Message::ParamChanged { id: 7, value: None, gesture: Gesture::End }, &[]).unwrap();
        write_frame(&mut wire, &"not a message", &[]).unwrap();
        let mut input = wire.as_slice();
        assert_eq!(read_frame::<Command>(&mut input).unwrap(), (Command::LoadState, vec![1, 2, 3]));
        let (message, payload) = read_frame::<Message>(&mut input).unwrap();
        assert!(message.is_notice() && payload.is_empty());
        // A frame that does not parse is skipped whole.
        assert_eq!(read_frame::<Message>(&mut input).unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(read_frame::<Message>(&mut input).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }
}
