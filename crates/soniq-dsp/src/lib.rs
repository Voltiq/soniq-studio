//! Built-in instruments, MIDI effects and audio effects for the real-time engine.
//!
//! An instrument is built on the control thread from a device's parameters
//! and handed to the audio thread inside a graph. From then on nothing it
//! does allocates, locks or blocks: voices live in fixed arrays. When a new
//! graph replaces the old one (any project change), the new instrument
//! [adopts](Instrument::adopt) the old one's voices, so turning a knob never
//! cuts off a note.

mod common;
mod drums;
mod epiano;
mod external;
mod fx;
mod midifx;
mod rack;
mod sampler;
mod synth;
mod warp;

use std::collections::HashMap;
use std::sync::Arc;

use soniq_io::AudioBuffer;

pub use common::Wave;
pub use drums::{BUILT_IN, DrumRack, FIRST_PAD, PAD_NAMES, PADS, PadChains, PadFx};
pub use epiano::EPiano;
pub use external::External;
pub use fx::{AUDIO_FX_TYPES, AudioFx, BAND_TYPES, BANDS, Compressor, Limiter, Sidechain};
pub use midifx::{BlockEvent, FxContext, Key, MIDI_FX_TYPES, MidiFx, push_event, sort_events};
pub use rack::{MacroLink, Rack, RackLayer};
pub use sampler::{MAX_SLICES, Sampler};
pub use synth::Synth;
pub use warp::{WarpClock, WarpMode, Warper};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NoteEvent {
    /// Velocity 0–1.
    On { pitch: u8, velocity: f32 },
    Off { pitch: u8 },
    /// The sustain pedal (MIDI CC 64).
    Sustain(bool),
    /// Releases every note, letting it ring out: the transport stopped or jumped.
    AllNotesOff,
    /// Silences everything at once.
    AllSoundOff,
}

/// A device's parameter values by id.
pub trait Params {
    fn get(&self, id: &str) -> Option<f64>;

    fn or(&self, id: &str, default: f64) -> f64 {
        self.get(id).filter(|v| v.is_finite()).unwrap_or(default)
    }
}

impl<F: Fn(&str) -> Option<f64>> Params for F {
    fn get(&self, id: &str) -> Option<f64> {
        self(id)
    }
}

/// A device's parameter values, held by the engine so automation and
/// modulators can change them while it plays. Built on the control thread;
/// reading and setting never allocate.
#[derive(Debug, Clone, Default)]
pub struct ParamBank {
    values: Vec<f64>,
    index: HashMap<String, usize>,
}

impl ParamBank {
    pub fn new(params: impl IntoIterator<Item = (String, f64)>) -> Self {
        let mut bank = Self::default();
        for (id, value) in params {
            bank.index.insert(id, bank.values.len());
            bank.values.push(value);
        }
        bank
    }

    /// Where a parameter sits, to set it by index later.
    pub fn find(&self, id: &str) -> Option<usize> {
        self.index.get(id).copied()
    }

    pub fn value(&self, i: usize) -> f64 {
        self.values.get(i).copied().unwrap_or(0.0)
    }

    /// Sets a value; whether it changed.
    pub fn set(&mut self, i: usize, value: f64) -> bool {
        match self.values.get_mut(i) {
            Some(v) if *v != value => {
                *v = value;
                true
            }
            _ => false,
        }
    }
}

impl Params for ParamBank {
    fn get(&self, id: &str) -> Option<f64> {
        self.index.get(id).map(|&i| self.values[i])
    }
}

/// A sample loaded into an instrument: the key it belongs to (a drum pad's
/// note; ignored by the sampler), its audio at the engine rate and, for a
/// sampler slicing by transients, where its hits start (frames).
#[derive(Debug, Clone)]
pub struct SampleSlot {
    pub key: u8,
    pub buffer: Arc<AudioBuffer>,
    pub onsets: Option<Arc<Vec<usize>>>,
}

// The graph boxes each instrument as a whole, so variant sizes do not matter.
#[allow(clippy::large_enum_variant)]
pub enum Instrument {
    Synth(Synth),
    EPiano(EPiano),
    Sampler(Sampler),
    Drums(DrumRack),
    Rack(Rack),
    /// A plugin: the graph's instrument node hands it whole blocks with their notes.
    External(Box<dyn External>),
}

/// Device types that make sound from notes.
pub const INSTRUMENT_TYPES: [&str; 5] = ["synth", "epiano", "sampler", "drum-rack", "rack"];

impl Instrument {
    /// The instrument for a device type, or `None` if the type is not one.
    /// A rack is built from its chains with [`Rack::new`] instead.
    pub fn build(kind: &str, sample_rate: u32, params: &dyn Params, samples: &[SampleSlot]) -> Option<Self> {
        let sr = sample_rate.max(1) as f32;
        Some(match kind {
            "synth" => Self::Synth(Synth::new(sr, params)),
            "epiano" => Self::EPiano(EPiano::new(sr, params)),
            "sampler" => {
                let slot = samples.first();
                Self::Sampler(Sampler::new(sr, params, slot.map(|s| Arc::clone(&s.buffer)), slot.and_then(|s| s.onsets.clone())))
            }
            "drum-rack" => Self::Drums(DrumRack::new(sr, params, samples)),
            _ => return None,
        })
    }

    /// Takes new parameter values (automation, modulators) without
    /// allocating; sounding voices carry on.
    pub fn configure(&mut self, params: &dyn Params) {
        match self {
            Self::Synth(i) => i.configure(params),
            Self::EPiano(i) => i.configure(params),
            Self::Sampler(i) => i.configure(params),
            Self::Drums(i) => i.configure(params),
            Self::Rack(i) => i.configure(params),
            Self::External(i) => i.configure(params),
        }
    }

    pub fn handle(&mut self, event: NoteEvent) {
        match self {
            Self::Synth(i) => i.handle(event),
            Self::EPiano(i) => i.handle(event),
            Self::Sampler(i) => i.handle(event),
            Self::Drums(i) => i.handle(event),
            Self::Rack(i) => i.handle(event),
            // Its notes go with the block instead (see `InstrumentNode::render`).
            Self::External(_) => {}
        }
    }

    /// Adds `left.len()` frames of output to `left` and `right`.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        let (left, right) = (&mut left[..n], &mut right[..n]);
        match self {
            Self::Synth(i) => i.render(left, right),
            Self::EPiano(i) => i.render(left, right),
            Self::Sampler(i) => i.render(left, right),
            Self::Drums(i) => i.render(left, right),
            Self::Rack(i) => i.render(left, right),
            Self::External(_) => {}
        }
    }

    /// Where the coming block sits in the song, for effects inside the
    /// instrument that follow the tempo (a Drum Rack's pad effects).
    pub fn set_context(&mut self, ctx: &FxContext) {
        if let Self::Drums(i) = self {
            i.set_context(ctx);
        }
    }

    /// Takes over the voices of the instrument this one replaces. Different
    /// kinds of instrument share nothing; the old one is simply silenced.
    pub fn adopt(&mut self, old: &mut Instrument) {
        match (self, old) {
            (Self::Synth(new), Self::Synth(old)) => new.adopt(old),
            (Self::EPiano(new), Self::EPiano(old)) => new.adopt(old),
            (Self::Sampler(new), Self::Sampler(old)) => new.adopt(old),
            (Self::Drums(new), Self::Drums(old)) => new.adopt(old),
            (Self::Rack(new), Self::Rack(old)) => new.adopt(old),
            _ => {}
        }
    }

    /// Samples its output comes late by (only plugins report any).
    pub fn latency(&self) -> u32 {
        match self {
            Self::External(i) => i.latency(),
            _ => 0,
        }
    }

    /// Nothing is sounding, so rendering can be skipped.
    pub fn is_silent(&self) -> bool {
        match self {
            Self::Synth(i) => i.is_silent(),
            Self::EPiano(i) => i.is_silent(),
            Self::Sampler(i) => i.is_silent(),
            Self::Drums(i) => i.is_silent(),
            Self::Rack(i) => i.is_silent(),
            Self::External(_) => false,
        }
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use super::*;

    pub const SR: u32 = 48_000;

    pub fn defaults(_: &str) -> Option<f64> {
        None
    }

    /// Renders `frames` of an instrument into fresh stereo buffers.
    pub fn render(inst: &mut Instrument, frames: usize) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
        for (cl, cr) in l.chunks_mut(256).zip(r.chunks_mut(256)) {
            inst.render(cl, cr);
        }
        (l, r)
    }

    pub fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
    }

    /// Frequency estimated from upward zero crossings.
    pub fn frequency(x: &[f32], sr: u32) -> f32 {
        let crossings: Vec<usize> = (1..x.len()).filter(|&i| x[i - 1] <= 0.0 && x[i] > 0.0).collect();
        if crossings.len() < 2 {
            return 0.0;
        }
        let span = (crossings[crossings.len() - 1] - crossings[0]) as f32;
        (crossings.len() - 1) as f32 * sr as f32 / span
    }
}
