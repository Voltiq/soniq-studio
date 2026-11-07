//! Drum Rack: 64 pads on notes 36–99, in four pages of 16. The first page is
//! the General MIDI drum kit, the second and third its percussion (to note
//! 81); the rest start empty. A pad plays its sample, or its built-in
//! synthesized sound while it has none, with its own tune, level, pan, decay
//! and choke group (an open hi-hat is cut short by the closed one).
//!
//! Each pad can have its own audio effects, and sends to the rack's two
//! returns (A and B), which have effects of their own. Pads without effects
//! or sends cost nothing extra.

use std::sync::Arc;

use soniq_io::AudioBuffer;

use crate::common::{Noise, db_gain, hermite, one_pole, pan_gains, pick_voice, sin_turns};
use crate::{AudioFx, FxContext, NoteEvent, Params, SampleSlot};

pub const PADS: usize = 64;
pub const FIRST_PAD: u8 = 36;
/// Pads with a built-in sound (notes 36–81); the rest play only samples.
pub const BUILT_IN: usize = 46;
pub const PAD_NAMES: [&str; BUILT_IN] = [
    "Kick",
    "Rim",
    "Snare",
    "Clap",
    "Snare 2",
    "Floor tom",
    "Closed hat",
    "Low tom",
    "Pedal hat",
    "Mid tom",
    "Open hat",
    "Mid tom 2",
    "High tom",
    "Crash",
    "High tom 2",
    "Ride",
    "China",
    "Ride bell",
    "Tambourine",
    "Splash",
    "Cowbell",
    "Crash 2",
    "Vibraslap",
    "Ride 2",
    "Hi bongo",
    "Low bongo",
    "Mute conga",
    "Open conga",
    "Low conga",
    "Hi timbale",
    "Low timbale",
    "Hi agogo",
    "Low agogo",
    "Cabasa",
    "Maracas",
    "Short whistle",
    "Long whistle",
    "Short guiro",
    "Long guiro",
    "Claves",
    "Hi wood block",
    "Low wood block",
    "Mute cuica",
    "Open cuica",
    "Mute triangle",
    "Open triangle",
];

const VOICES: usize = 32;
/// A choked voice fades out over this long instead of stopping dead.
const CHOKE_SECONDS: f32 = 0.006;
/// Inharmonic square-wave partials of the classic analog hi-hat circuit.
const METAL: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];
/// Pad and return effects work through the block in pieces this long.
const CHUNK: usize = 256;
/// Effects keep running this long after the last hit, so reverbs and delays ring out.
const TAIL_SECONDS: f32 = 8.0;
/// Sends at or below this (dB) are off.
const SEND_OFF: f64 = -59.9;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Model {
    Kick,
    Rim,
    Snare,
    Clap,
    Snare2,
    Tom(f32),
    ClosedHat,
    PedalHat,
    OpenHat,
    Crash,
    Ride,
    /// Cymbals and jingles from the hi-hat circuit: how its partials are
    /// spread, how fast it dies (per second), how much is noise, gain, a
    /// brighter filter, length (s) and output level.
    Metal { ratio: f32, speed: f32, noise: f32, gain: f32, bright: bool, len: f32, level: f32 },
    /// Struck metal or wood: two partials (the second at half level, 0 for
    /// none), squares for a cowbell, decay per second, length (s), level.
    Bell { hz: [f32; 2], square: bool, decay: f32, len: f32, level: f32 },
    /// Hand drums: pitch (Hz) and length (s).
    Hand(f32, f32),
    /// Shakers and scrapers: decay per second, a scrape rate (Hz, 0 for a
    /// plain shake), length (s), level.
    Shaker { decay: f32, scrape: f32, len: f32, level: f32 },
    /// A whistle: pitch (Hz) and length (s).
    Whistle(f32, f32),
    /// A cuíca: pitch (Hz) it squeaks up from, and length (s).
    Cuica(f32, f32),
    /// Plays nothing until a sample is dropped on it.
    Empty,
}

fn model(pad: usize) -> Model {
    use Model::*;
    let metal = |ratio, speed, noise, gain, bright, len, level| Metal { ratio, speed, noise, gain, bright, len, level };
    let bell = |a, b, decay, len, level| Bell { hz: [a, b], square: false, decay, len, level };
    match pad {
        0 => Kick,
        1 => Rim,
        2 => Snare,
        3 => Clap,
        4 => Snare2,
        5 => Tom(82.0),
        6 => ClosedHat,
        7 => Tom(98.0),
        8 => PedalHat,
        9 => Tom(116.0),
        10 => OpenHat,
        11 => Tom(132.0),
        12 => Tom(155.0),
        13 => Crash,
        14 => Tom(180.0),
        15 => Ride,
        16 => metal(1.4, 2.8, 0.85, 1.0, false, 2.0, 1.2),
        17 => bell(3100.0, 3720.0, 3.5, 1.6, 0.35),
        18 => metal(3.2, 9.0, 0.7, 1.2, true, 0.5, 1.6),
        19 => metal(1.5, 5.0, 0.8, 1.0, true, 1.2, 1.7),
        20 => Bell { hz: [587.0, 845.0], square: true, decay: 12.0, len: 0.4, level: 0.3 },
        21 => metal(1.32, 2.0, 0.8, 1.0, false, 2.6, 1.26),
        22 => Shaker { decay: 3.5, scrape: 28.0, len: 1.0, level: 0.55 },
        23 => metal(2.6, 2.4, 0.25, 0.8, false, 2.0, 2.3),
        24 => Hand(420.0, 0.2),
        25 => Hand(320.0, 0.25),
        26 => Hand(360.0, 0.12),
        27 => Hand(300.0, 0.4),
        28 => Hand(220.0, 0.45),
        29 => Hand(520.0, 0.5),
        30 => Hand(400.0, 0.55),
        31 => bell(920.0, 2480.0, 10.0, 0.5, 0.6),
        32 => bell(690.0, 1860.0, 10.0, 0.5, 0.6),
        33 => Shaker { decay: 22.0, scrape: 0.0, len: 0.2, level: 0.5 },
        34 => Shaker { decay: 40.0, scrape: 0.0, len: 0.12, level: 0.5 },
        35 => Whistle(2500.0, 0.15),
        36 => Whistle(2500.0, 0.6),
        37 => Shaker { decay: 8.0, scrape: 45.0, len: 0.2, level: 0.5 },
        38 => Shaker { decay: 3.0, scrape: 45.0, len: 0.6, level: 0.5 },
        39 => bell(2500.0, 0.0, 40.0, 0.15, 0.7),
        40 => bell(1200.0, 3100.0, 35.0, 0.15, 0.6),
        41 => bell(800.0, 2100.0, 30.0, 0.18, 0.6),
        42 => Cuica(500.0, 0.15),
        43 => Cuica(400.0, 0.45),
        44 => bell(4600.0, 7250.0, 25.0, 0.2, 0.45),
        45 => bell(4600.0, 7250.0, 1.8, 2.0, 0.45),
        _ => Empty,
    }
}

impl Model {
    /// Seconds the drum lasts at 100 % decay.
    fn length(self) -> f32 {
        match self {
            Self::Kick => 0.6,
            Self::Rim => 0.12,
            Self::Snare => 0.35,
            Self::Clap => 0.45,
            Self::Snare2 => 0.25,
            Self::Tom(_) => 0.7,
            Self::ClosedHat => 0.12,
            Self::PedalHat => 0.25,
            Self::OpenHat => 0.9,
            Self::Crash => 2.5,
            Self::Ride => 2.0,
            Self::Metal { len, .. } | Self::Bell { len, .. } | Self::Shaker { len, .. } | Self::Hand(_, len) | Self::Whistle(_, len) | Self::Cuica(_, len) => len,
            Self::Empty => 0.0,
        }
    }

    /// Output level, so the kit sits together: kick near 0.9 peak, hats and
    /// cymbals lower (they sound louder than their peaks suggest).
    fn level(self) -> f32 {
        match self {
            Self::Kick => 0.82,
            Self::Rim => 0.6,
            Self::Snare => 0.69,
            Self::Clap | Self::Snare2 | Self::Tom(_) | Self::Hand(..) => 0.7,
            Self::ClosedHat => 1.73,
            Self::PedalHat => 2.0,
            Self::OpenHat => 1.85,
            Self::Crash => 1.26,
            Self::Ride => 1.55,
            Self::Metal { level, .. } | Self::Bell { level, .. } | Self::Shaker { level, .. } => level,
            Self::Whistle(..) => 0.5,
            Self::Cuica(..) => 0.6,
            Self::Empty => 0.0,
        }
    }
}

struct Pad {
    /// Playback-speed (and pitch) ratio.
    tune: f32,
    gain: f32,
    pan: (f32, f32),
    decay: f32,
    choke: u8,
    /// Gains of Send A and Send B (0 when off).
    sends: [f32; 2],
    sample: Option<Arc<AudioBuffer>>,
}

/// One effect in a pad's or a return's chain, built by the host.
pub struct PadFx {
    /// The device's id, so its state carries over when the song changes.
    pub id: String,
    pub fx: AudioFx,
}

/// The effects of a Drum Rack's pads (by note) and of its two returns.
#[derive(Default)]
pub struct PadChains {
    pub pads: Vec<(u8, Vec<PadFx>)>,
    pub returns: [Vec<PadFx>; 2],
}

/// A pad's effects and the buffer its hits gather in.
struct Chained {
    pad: usize,
    fx: Vec<PadFx>,
    buf: [Vec<f32>; 2],
}

/// One-pole coefficients at the engine rate.
struct Coefs {
    lp1k: f32,
    lp2k5: f32,
    lp800: f32,
    lp5k: f32,
    lp7k: f32,
}

#[derive(Debug, Clone, Copy, Default)]
struct Voice {
    active: bool,
    pad: u8,
    choke: u8,
    velocity: f32,
    age: u64,
    /// Samples played, and how many the voice lasts.
    t: u32,
    len: u32,
    phases: [f32; 6],
    lp: [f32; 3],
    noise: u32,
    choked: bool,
    fade: f32,
    /// Sample playback.
    pos: f64,
    speed: f64,
}

pub struct DrumRack {
    sr: f32,
    pads: Vec<Pad>,
    /// Each pad's parameter ids (tune, vol, pan, decay, choke, sendA, sendB),
    /// made once so `configure` never formats strings on the audio thread.
    ids: Vec<[String; 7]>,
    coefs: Coefs,
    gain: f32,
    voices: [Voice; VOICES],
    clock: u64,
    /// Pads with effects, and which of them each pad is (if any).
    chains: Vec<Chained>,
    routed: [Option<u8>; PADS],
    returns: [Vec<PadFx>; 2],
    ret_buf: [[Vec<f32>; 2]; 2],
    /// Where the block is, for tempo-synced effects.
    ctx: FxContext,
    /// Samples the effects still ring after the last hit.
    tail: u32,
}

const PAD_PARAMS: [&str; 7] = ["tune", "vol", "pan", "decay", "choke", "sendA", "sendB"];

impl DrumRack {
    pub fn new(sr: f32, p: &dyn Params, samples: &[SampleSlot]) -> Self {
        let notes = (0..PADS).map(|i| FIRST_PAD + i as u8);
        let pads = notes
            .clone()
            .map(|note| Pad {
                tune: 1.0,
                gain: 1.0,
                pan: (1.0, 1.0),
                decay: 1.0,
                choke: 0,
                sends: [0.0; 2],
                sample: samples.iter().find(|s| s.key == note).map(|s| Arc::clone(&s.buffer)),
            })
            .collect();
        let scratch = || [vec![0.0; CHUNK], vec![0.0; CHUNK]];
        let mut rack = Self {
            sr,
            pads,
            ids: notes.map(|note| PAD_PARAMS.map(|name| format!("p{note}.{name}"))).collect(),
            coefs: Coefs {
                lp1k: one_pole(1000.0, sr),
                lp2k5: one_pole(2500.0, sr),
                lp800: one_pole(800.0, sr),
                lp5k: one_pole(5000.0, sr),
                lp7k: one_pole(7000.0, sr),
            },
            gain: 1.0,
            voices: [Voice::default(); VOICES],
            clock: 0,
            chains: Vec::new(),
            routed: [None; PADS],
            returns: [Vec::new(), Vec::new()],
            ret_buf: [scratch(), scratch()],
            ctx: FxContext { playing: false, pos: 0, beat: 0.0, samples_per_beat: f64::from(sr) * 0.5, sample_rate: sr },
            tail: 0,
        };
        rack.configure(p);
        rack
    }

    /// Takes new parameter values; playing hits carry on.
    pub fn configure(&mut self, p: &dyn Params) {
        let global_tune = p.or("tune", 0.0);
        let send = |db: f64| if db <= SEND_OFF { 0.0 } else { db_gain(db) };
        for (pad, [tune, vol, pan, decay, choke, send_a, send_b]) in self.pads.iter_mut().zip(&self.ids) {
            pad.tune = ((global_tune + p.or(tune, 0.0)) / 12.0).exp2() as f32;
            pad.gain = db_gain(p.or(vol, 0.0));
            pad.pan = pan_gains((p.or(pan, 0.0) / 100.0) as f32);
            pad.decay = (p.or(decay, 100.0) / 100.0).clamp(0.05, 4.0) as f32;
            pad.choke = p.or(choke, 0.0).round().clamp(0.0, 8.0) as u8;
            pad.sends = [send(p.or(send_a, -60.0)), send(p.or(send_b, -60.0))];
        }
        self.gain = db_gain(p.or("volume", 0.0));
    }

    /// Gives pads and returns their effects (pads not listed have none).
    pub fn set_chains(&mut self, chains: PadChains) {
        self.routed = [None; PADS];
        self.chains = chains
            .pads
            .into_iter()
            .filter_map(|(note, fx)| {
                let pad = usize::from(note.checked_sub(FIRST_PAD)?);
                (pad < PADS && !fx.is_empty()).then(|| Chained { pad, fx, buf: [vec![0.0; CHUNK], vec![0.0; CHUNK]] })
            })
            .collect();
        for (i, c) in self.chains.iter().enumerate() {
            self.routed[c.pad] = Some(i as u8);
        }
        self.returns = chains.returns;
    }

    /// Where the coming block sits in the song (for synced effects).
    pub fn set_context(&mut self, ctx: &FxContext) {
        self.ctx = *ctx;
    }

    pub fn handle(&mut self, event: NoteEvent) {
        match event {
            NoteEvent::On { pitch, velocity } => {
                if let Some(pad) = pitch.checked_sub(FIRST_PAD).filter(|&p| (p as usize) < PADS) {
                    self.trigger(pad, velocity);
                }
            }
            // Drums ring out whatever the keys do.
            NoteEvent::Off { .. } | NoteEvent::Sustain(_) | NoteEvent::AllNotesOff => {}
            NoteEvent::AllSoundOff => self.voices = [Voice::default(); VOICES],
        }
    }

    fn trigger(&mut self, pad: u8, velocity: f32) {
        let p = &self.pads[pad as usize];
        let built_in = model(pad as usize);
        if p.sample.is_none() && built_in == Model::Empty {
            return;
        }
        self.clock += 1;
        if p.choke > 0 {
            for v in self.voices.iter_mut().filter(|v| v.active && v.choke == p.choke) {
                v.choked = true;
            }
        }
        let len = match &p.sample {
            Some(s) => s.frames() as f32 / p.tune * p.decay.min(1.0),
            None => built_in.length() * p.decay * self.sr,
        };
        let i = pick_voice(self.voices.iter().map(|v| (v.active, v.active, v.age)));
        self.voices[i] = Voice {
            active: true,
            pad,
            choke: p.choke,
            velocity,
            age: self.clock,
            len: len.max(1.0) as u32,
            noise: 0x9e37_79b9 ^ (self.clock as u32).wrapping_mul(0x85eb_ca6b),
            fade: 1.0,
            speed: f64::from(p.tune),
            ..Voice::default()
        };
    }

    /// Whether anything goes through effects or returns (otherwise hits go straight out).
    fn routing(&self) -> bool {
        !self.chains.is_empty() || self.returns.iter().any(|r| !r.is_empty()) || self.pads.iter().any(|p| p.sends != [0.0; 2])
    }

    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        if !self.routing() {
            self.voices_into(left, right);
            return;
        }
        for (l, r) in left.chunks_mut(CHUNK).zip(right.chunks_mut(CHUNK)) {
            self.render_routed(l, r);
        }
    }

    /// Every hit straight into the output: no pad has effects or sends.
    fn voices_into(&mut self, left: &mut [f32], right: &mut [f32]) {
        let choke_step = 1.0 / (CHOKE_SECONDS * self.sr);
        for v in self.voices.iter_mut().filter(|v| v.active) {
            let pad = &self.pads[v.pad as usize];
            let level = v.velocity * pad.gain * self.gain;
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                let Some(out) = voice_sample(v, pad, &self.coefs, self.sr, choke_step) else { break };
                *l += out * level * pad.pan.0;
                *r += out * level * pad.pan.1;
            }
        }
    }

    /// One chunk (≤ CHUNK) with pad effects, sends and returns.
    fn render_routed(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len()).min(CHUNK);
        let choke_step = 1.0 / (CHOKE_SECONDS * self.sr);
        let Self { voices, pads, chains, routed, returns, ret_buf, coefs, sr, gain, ctx, tail, .. } = self;
        for c in chains.iter_mut() {
            c.buf.iter_mut().for_each(|b| b[..n].fill(0.0));
        }
        ret_buf.iter_mut().flatten().for_each(|b| b[..n].fill(0.0));
        let mut sounding = false;
        for v in voices.iter_mut().filter(|v| v.active) {
            sounding = true;
            let pad = &pads[v.pad as usize];
            let level = v.velocity * pad.gain * *gain;
            match routed[v.pad as usize] {
                // Into the pad's effects; its sends are taken after them.
                Some(c) => {
                    let [bl, br] = &mut chains[c as usize].buf;
                    for i in 0..n {
                        let Some(out) = voice_sample(v, pad, coefs, *sr, choke_step) else { break };
                        bl[i] += out * level * pad.pan.0;
                        br[i] += out * level * pad.pan.1;
                    }
                }
                None => {
                    for i in 0..n {
                        let Some(out) = voice_sample(v, pad, coefs, *sr, choke_step) else { break };
                        let (a, b) = (out * level * pad.pan.0, out * level * pad.pan.1);
                        left[i] += a;
                        right[i] += b;
                        for (k, &send) in pad.sends.iter().enumerate().filter(|(_, s)| **s > 0.0) {
                            ret_buf[k][0][i] += a * send;
                            ret_buf[k][1][i] += b * send;
                        }
                    }
                }
            }
        }
        if sounding {
            *tail = (TAIL_SECONDS * *sr) as u32;
        } else {
            *tail = tail.saturating_sub(n as u32);
        }
        for c in chains.iter_mut() {
            let [bl, br] = &mut c.buf;
            for fx in &mut c.fx {
                fx.fx.process(&mut bl[..n], &mut br[..n], None, ctx);
            }
            let sends = pads[c.pad].sends;
            for i in 0..n {
                left[i] += bl[i];
                right[i] += br[i];
                for (k, &send) in sends.iter().enumerate().filter(|(_, s)| **s > 0.0) {
                    ret_buf[k][0][i] += bl[i] * send;
                    ret_buf[k][1][i] += br[i] * send;
                }
            }
        }
        for (chain, [bl, br]) in returns.iter_mut().zip(ret_buf.iter_mut()) {
            for fx in chain.iter_mut() {
                fx.fx.process(&mut bl[..n], &mut br[..n], None, ctx);
            }
            for i in 0..n {
                left[i] += bl[i];
                right[i] += br[i];
            }
        }
    }

    pub fn adopt(&mut self, old: &mut DrumRack) {
        std::mem::swap(&mut self.voices, &mut old.voices);
        self.clock = old.clock;
        self.tail = old.tail;
        // A pad whose sample changed drops its playing hits.
        for v in self.voices.iter_mut().filter(|v| v.active) {
            let (new, was) = (&self.pads[v.pad as usize].sample, &old.pads[v.pad as usize].sample);
            let same = match (new, was) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            };
            if !same {
                v.active = false;
            }
        }
        // Effects carry on where they were (a reverb's tail, a compressor's level).
        let mut old_fx: Vec<&mut PadFx> = old.chains.iter_mut().flat_map(|c| c.fx.iter_mut()).chain(old.returns.iter_mut().flatten()).collect();
        for fx in self.chains.iter_mut().flat_map(|c| c.fx.iter_mut()).chain(self.returns.iter_mut().flatten()) {
            if let Some(was) = old_fx.iter_mut().find(|o| o.id == fx.id) {
                fx.fx.adopt(&mut was.fx);
            }
        }
    }

    pub fn is_silent(&self) -> bool {
        !self.voices.iter().any(|v| v.active) && (self.tail == 0 || !self.routing())
    }
}

/// The next sample of a voice (before level and pan), or `None` once it has ended.
fn voice_sample(v: &mut Voice, pad: &Pad, coefs: &Coefs, sr: f32, choke_step: f32) -> Option<f32> {
    if v.t >= v.len {
        v.active = false;
        return None;
    }
    let mut s = match &pad.sample {
        // Pads are mono hits, placed by the pad's pan.
        Some(sample) => {
            let at = v.pos;
            v.pos += v.speed;
            let sum: f32 = sample.channels.iter().map(|ch| hermite(ch, at)).sum();
            sum / sample.channels.len().max(1) as f32
        }
        None => {
            let model = model(v.pad as usize);
            model_sample(model, v, coefs, sr, pad.tune, pad.decay) * model.level()
        }
    };
    // The last tenth fades out, so a cut-short tail never clicks.
    let tail = (v.len - v.t) as f32 / (v.len as f32 * 0.1);
    if tail < 1.0 {
        s *= tail;
    }
    if v.choked {
        v.fade -= choke_step;
        if v.fade <= 0.0 {
            v.active = false;
            return None;
        }
    }
    v.t += 1;
    Some(s * v.fade)
}

/// One sample of a built-in drum.
fn model_sample(model: Model, v: &mut Voice, c: &Coefs, sr: f32, tune: f32, decay: f32) -> f32 {
    let t = v.t as f32 / sr;
    let d = decay.max(0.05);
    let mut rng = Noise(v.noise);
    let n = rng.next();
    v.noise = rng.0;
    let mut step = |i: usize, hz: f32| {
        v.phases[i] = (v.phases[i] + hz / sr).fract();
        v.phases[i]
    };
    match model {
        Model::Kick => {
            let ph = step(0, (45.0 + 110.0 * (-t * 28.0).exp()) * tune);
            let click = if t < 0.003 { n * 0.25 * (1.0 - t / 0.003) } else { 0.0 };
            sin_turns(ph) * (-t * 7.0 / d).exp() + click
        }
        Model::Rim => {
            let a = sin_turns(step(0, 1700.0 * tune));
            let b = sin_turns(step(1, 520.0 * tune));
            (a + b) * 0.45 * (-t * 55.0 / d).exp() + n * 0.4 * (-t * 300.0).exp()
        }
        Model::Snare | Model::Snare2 => {
            let (tone, rattle_decay, tone_decay) = if model == Model::Snare { (185.0, 16.0, 25.0) } else { (240.0, 26.0, 32.0) };
            let body = sin_turns(step(0, tone * tune)) * 0.45 * (-t * tone_decay / d).exp();
            v.lp[0] += c.lp7k * (n - v.lp[0]);
            v.lp[1] += c.lp1k * (v.lp[0] - v.lp[1]);
            body + (v.lp[0] - v.lp[1]) * 1.2 * (-t * rattle_decay / d).exp()
        }
        Model::Clap => {
            v.lp[0] += c.lp2k5 * (n - v.lp[0]);
            v.lp[1] += c.lp800 * (v.lp[0] - v.lp[1]);
            // Three quick hand claps, then the room.
            let mut env = 0.0_f32;
            for burst in [0.0, 0.011, 0.022] {
                if t >= burst {
                    env = env.max((-(t - burst) * 320.0).exp());
                }
            }
            if t >= 0.031 {
                env = env.max(0.8 * (-(t - 0.031) * 13.0 / d).exp());
            }
            (v.lp[0] - v.lp[1]) * 3.0 * env
        }
        Model::Tom(hz) => {
            let ph = step(0, hz * tune * (1.0 + 0.6 * (-t * 25.0).exp()));
            v.lp[0] += c.lp1k * (n - v.lp[0]);
            sin_turns(ph) * (-t * 7.0 / d).exp() + v.lp[0] * 0.3 * (-t * 40.0).exp()
        }
        Model::Hand(hz, len) => {
            // A tighter skin than a tom: less pitch drop, a slap of noise.
            let ph = step(0, hz * tune * (1.0 + 0.25 * (-t * 40.0).exp()));
            v.lp[0] += c.lp2k5 * (n - v.lp[0]);
            sin_turns(ph) * (-t * 4.0 / (len * d)).exp() + v.lp[0] * 0.5 * (-t * 120.0).exp()
        }
        Model::ClosedHat | Model::PedalHat | Model::OpenHat | Model::Crash | Model::Ride | Model::Metal { .. } => {
            let (ratio, speed, cutoff, noise, gain) = match model {
                Model::ClosedHat => (1.7, 60.0, c.lp7k, 0.6, 1.4),
                Model::PedalHat => (1.7, 30.0, c.lp7k, 0.5, 1.1),
                Model::OpenHat => (1.7, 6.0, c.lp7k, 0.6, 1.2),
                Model::Crash => (1.25, 2.2, c.lp5k, 0.8, 1.0),
                Model::Metal { ratio, speed, noise, gain, bright, .. } => (ratio, speed, if bright { c.lp7k } else { c.lp5k }, noise, gain),
                _ => (2.4, 2.6, c.lp5k, 0.25, 0.8),
            };
            let mut metal = 0.0;
            for (i, hz) in METAL.iter().enumerate() {
                metal += if step(i, hz * ratio * tune) < 0.5 { 1.0 } else { -1.0 };
            }
            let x = metal / 6.0 * (1.0 - noise) + n * noise;
            // Two one-pole high-passes: only the shimmer is left.
            v.lp[0] += cutoff * (x - v.lp[0]);
            let h1 = x - v.lp[0];
            v.lp[1] += cutoff * (h1 - v.lp[1]);
            let mut s = (h1 - v.lp[1]) * gain * (-t * speed / d).exp();
            if model == Model::Ride {
                // The ride's bell.
                v.lp[2] = (v.lp[2] + 3100.0 * tune / sr).fract();
                s += sin_turns(v.lp[2]) * 0.12 * (-t * 4.0 / d).exp();
            }
            s
        }
        Model::Bell { hz, square, decay, .. } => {
            let wave = |ph: f32| if square { if ph < 0.5 { 1.0 } else { -1.0 } } else { sin_turns(ph) };
            let a = wave(step(0, hz[0] * tune));
            let b = if hz[1] > 0.0 { 0.5 * wave(step(1, hz[1] * tune)) } else { 0.0 };
            let mut s = a + b;
            if square {
                // A cowbell's squares, softened.
                v.lp[0] += c.lp2k5 * (s - v.lp[0]);
                s = v.lp[0] * 1.6;
            }
            let attack = (t / 0.0015).min(1.0);
            s * attack * (-t * decay / d).exp()
        }
        Model::Shaker { decay, scrape, .. } => {
            v.lp[0] += c.lp5k * (n - v.lp[0]);
            let hiss = n - v.lp[0];
            let attack = (t / 0.005).min(1.0);
            let mut env = attack * (-t * decay / d).exp();
            if scrape > 0.0 {
                let ridge = 0.5 + 0.5 * sin_turns((t * scrape * tune).fract());
                env *= ridge * ridge;
            }
            hiss * 1.6 * env
        }
        Model::Whistle(hz, _) => {
            // A pea whistle's trill.
            let trill = 1.0 + 0.015 * sin_turns((t * 25.0).fract());
            let attack = (t / 0.01).min(1.0);
            sin_turns(step(0, hz * tune * trill)) * attack
        }
        Model::Cuica(hz, _) => {
            let rise = 1.0 + 0.8 * (1.0 - (-t * 12.0).exp());
            sin_turns(step(0, hz * tune * rise)) * (t / 0.004).min(1.0) * (-t * 6.0 / d).exp()
        }
        Model::Empty => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use soniq_io::AudioBuffer;

    use super::{BUILT_IN, FIRST_PAD, PAD_NAMES, PADS, PadChains, PadFx};
    use crate::fx::test_util::params;
    use crate::test_util::*;
    use crate::{AudioFx, Instrument, NoteEvent, SampleSlot};

    fn rack(params: &[(&str, f64)], samples: &[SampleSlot]) -> Instrument {
        let owned: Vec<(String, f64)> = params.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        let lookup = move |id: &str| owned.iter().find(|(k, _)| k == id).map(|(_, v)| *v);
        Instrument::build("drum-rack", SR, &lookup, samples).unwrap()
    }

    #[test]
    fn every_built_in_pad_makes_a_sound_at_a_sensible_level() {
        let mut levels = Vec::new();
        for (i, name) in PAD_NAMES.iter().enumerate() {
            let mut r = rack(&[], &[]);
            r.handle(NoteEvent::On { pitch: FIRST_PAD + i as u8, velocity: 1.0 });
            let (l, _) = render(&mut r, SR as usize * 3);
            assert!(r.is_silent(), "{name} ends");
            levels.push((name, peak(&l)));
        }
        let all = format!("{levels:?}");
        assert!(levels.iter().all(|(_, p)| (0.3..1.0).contains(p)), "{all}");
        assert_eq!(levels.len(), BUILT_IN);
    }

    #[test]
    fn pads_past_the_kit_play_only_samples() {
        let mut r = rack(&[], &[]);
        r.handle(NoteEvent::On { pitch: 90, velocity: 1.0 });
        assert!(r.is_silent());
        let buffer = Arc::new(AudioBuffer::new(SR, vec![vec![0.5; 1000]]));
        let mut r = rack(&[], &[SampleSlot { key: 99, buffer, onsets: None }]);
        r.handle(NoteEvent::On { pitch: FIRST_PAD + PADS as u8 - 1, velocity: 1.0 });
        let (l, _) = render(&mut r, 2000);
        assert!((l[100] - 0.5).abs() < 1e-4, "{}", l[100]);
    }

    #[test]
    fn the_closed_hat_chokes_the_open_one() {
        let mut r = rack(&[("p42.choke", 1.0), ("p46.choke", 1.0)], &[]);
        r.handle(NoteEvent::On { pitch: 46, velocity: 1.0 });
        render(&mut r, 2400);
        r.handle(NoteEvent::On { pitch: 42, velocity: 1.0 });
        // Closed hat (0.12 s) plus the choke fade: all quiet well before the open hat would end.
        render(&mut r, 9600);
        assert!(r.is_silent());
    }

    #[test]
    fn a_pad_with_a_sample_plays_it() {
        let buffer = Arc::new(AudioBuffer::new(SR, vec![vec![0.5; 1000]]));
        let mut r = rack(&[], &[SampleSlot { key: 38, buffer, onsets: None }]);
        r.handle(NoteEvent::On { pitch: 38, velocity: 1.0 });
        let (l, _) = render(&mut r, 2000);
        assert!((l[100] - 0.5).abs() < 1e-4, "{}", l[100]);
        assert!(peak(&l[1000..]) < 1e-6);
    }

    #[test]
    fn notes_outside_the_pads_are_ignored() {
        let mut r = rack(&[], &[]);
        r.handle(NoteEvent::On { pitch: 20, velocity: 1.0 });
        r.handle(NoteEvent::On { pitch: 100, velocity: 1.0 });
        assert!(r.is_silent());
    }

    fn utility(gain_db: f64) -> AudioFx {
        AudioFx::build("utility", SR, &params(&[("gain", gain_db)])).unwrap()
    }

    #[test]
    fn a_pads_effects_change_only_that_pad() {
        let buffer = Arc::new(AudioBuffer::new(SR, vec![vec![0.5; 4000]]));
        let slots = [SampleSlot { key: 38, buffer: Arc::clone(&buffer), onsets: None }, SampleSlot { key: 40, buffer, onsets: None }];
        let mut r = rack(&[], &slots);
        let Instrument::Drums(d) = &mut r else { unreachable!() };
        d.set_chains(PadChains { pads: vec![(38, vec![PadFx { id: "u".into(), fx: utility(-6.0) }])], returns: Default::default() });
        r.handle(NoteEvent::On { pitch: 38, velocity: 1.0 });
        let (l, _) = render(&mut r, 3000);
        // Half as loud (−6 dB) through its utility.
        assert!((l[2000] - 0.25).abs() < 0.01, "{}", l[2000]);
        let mut r2 = rack(&[], &slots);
        let Instrument::Drums(d) = &mut r2 else { unreachable!() };
        d.set_chains(PadChains { pads: vec![(38, vec![PadFx { id: "u".into(), fx: utility(-6.0) }])], returns: Default::default() });
        r2.handle(NoteEvent::On { pitch: 40, velocity: 1.0 });
        let (l, _) = render(&mut r2, 3000);
        assert!((l[2000] - 0.5).abs() < 1e-3, "the other pad is untouched: {}", l[2000]);
    }

    #[test]
    fn sends_feed_the_returns_and_their_effects() {
        let buffer = Arc::new(AudioBuffer::new(SR, vec![vec![0.5; 4000]]));
        let slots = [SampleSlot { key: 36, buffer, onsets: None }];
        // Send A at 0 dB into a return that turns it down 6 dB: dry 0.5 plus 0.25.
        let mut r = rack(&[("p36.sendA", 0.0)], &slots);
        let Instrument::Drums(d) = &mut r else { unreachable!() };
        d.set_chains(PadChains { pads: Vec::new(), returns: [vec![PadFx { id: "ra".into(), fx: utility(-6.0) }], Vec::new()] });
        r.handle(NoteEvent::On { pitch: 36, velocity: 1.0 });
        let (l, _) = render(&mut r, 3000);
        assert!((l[2000] - 0.75).abs() < 0.01, "{}", l[2000]);
        // Off (−60 dB) sends nothing.
        let mut r = rack(&[("p36.sendA", -60.0)], &slots);
        let Instrument::Drums(d) = &mut r else { unreachable!() };
        d.set_chains(PadChains { pads: Vec::new(), returns: [vec![PadFx { id: "ra".into(), fx: utility(-6.0) }], Vec::new()] });
        r.handle(NoteEvent::On { pitch: 36, velocity: 1.0 });
        let (l, _) = render(&mut r, 3000);
        assert!((l[2000] - 0.5).abs() < 1e-3, "{}", l[2000]);
    }

    #[test]
    fn a_reverb_on_a_pad_rings_on_after_the_hit() {
        let mut r = rack(&[], &[]);
        let Instrument::Drums(d) = &mut r else { unreachable!() };
        let reverb = AudioFx::build("reverb", SR, &params(&[("mix", 50.0), ("size", 80.0)])).unwrap();
        d.set_chains(PadChains { pads: vec![(37, vec![PadFx { id: "rv".into(), fx: reverb }])], returns: Default::default() });
        r.handle(NoteEvent::On { pitch: 37, velocity: 1.0 });
        // The rim lasts 0.12 s; the reverb is still heard at 0.5 s.
        let (l, _) = render(&mut r, SR as usize);
        assert!(!r.is_silent());
        assert!(peak(&l[SR as usize / 2..]) > 1e-3, "{}", peak(&l[SR as usize / 2..]));
    }
}
