//! Drum Rack: 16 pads on notes 36–51, the General MIDI drum map. A pad plays
//! its sample, or a built-in synthesized drum while it has none, with its own
//! tune, level, pan, decay and choke group (an open hi-hat is cut short by the
//! closed one).

use std::sync::Arc;

use soniq_io::AudioBuffer;

use crate::common::{Noise, db_gain, hermite, one_pole, pan_gains, pick_voice, sin_turns};
use crate::{NoteEvent, Params, SampleSlot};

pub const PADS: usize = 16;
pub const FIRST_PAD: u8 = 36;
pub const PAD_NAMES: [&str; PADS] = [
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
];

const VOICES: usize = 32;
/// A choked voice fades out over this long instead of stopping dead.
const CHOKE_SECONDS: f32 = 0.006;
/// Inharmonic square-wave partials of the classic analog hi-hat circuit.
const METAL: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];

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
}

const MODELS: [Model; PADS] = [
    Model::Kick,
    Model::Rim,
    Model::Snare,
    Model::Clap,
    Model::Snare2,
    Model::Tom(82.0),
    Model::ClosedHat,
    Model::Tom(98.0),
    Model::PedalHat,
    Model::Tom(116.0),
    Model::OpenHat,
    Model::Tom(132.0),
    Model::Tom(155.0),
    Model::Crash,
    Model::Tom(180.0),
    Model::Ride,
];

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
        }
    }

    /// Output level, so the kit sits together: kick near 0.9 peak, hats and
    /// cymbals lower (they sound louder than their peaks suggest).
    fn level(self) -> f32 {
        match self {
            Self::Kick => 0.82,
            Self::Rim => 0.6,
            Self::Snare => 0.69,
            Self::Clap | Self::Snare2 | Self::Tom(_) => 0.7,
            Self::ClosedHat => 1.73,
            Self::PedalHat => 2.0,
            Self::OpenHat => 1.85,
            Self::Crash => 1.26,
            Self::Ride => 1.55,
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
    sample: Option<Arc<AudioBuffer>>,
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
    /// Each pad's parameter ids (tune, vol, pan, decay, choke), made once so
    /// `configure` never formats strings on the audio thread.
    ids: Vec<[String; 5]>,
    coefs: Coefs,
    gain: f32,
    voices: [Voice; VOICES],
    clock: u64,
}

const PAD_PARAMS: [&str; 5] = ["tune", "vol", "pan", "decay", "choke"];

impl DrumRack {
    pub fn new(sr: f32, p: &dyn Params, samples: &[SampleSlot]) -> Self {
        let notes = (0..PADS).map(|i| FIRST_PAD + i as u8);
        let pads = notes
            .clone()
            .map(|note| Pad { tune: 1.0, gain: 1.0, pan: (1.0, 1.0), decay: 1.0, choke: 0, sample: samples.iter().find(|s| s.key == note).map(|s| Arc::clone(&s.buffer)) })
            .collect();
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
        };
        rack.configure(p);
        rack
    }

    /// Takes new parameter values; playing hits carry on.
    pub fn configure(&mut self, p: &dyn Params) {
        let global_tune = p.or("tune", 0.0);
        for (pad, [tune, vol, pan, decay, choke]) in self.pads.iter_mut().zip(&self.ids) {
            pad.tune = ((global_tune + p.or(tune, 0.0)) / 12.0).exp2() as f32;
            pad.gain = db_gain(p.or(vol, 0.0));
            pad.pan = pan_gains((p.or(pan, 0.0) / 100.0) as f32);
            pad.decay = (p.or(decay, 100.0) / 100.0).clamp(0.05, 4.0) as f32;
            pad.choke = p.or(choke, 0.0).round().clamp(0.0, 8.0) as u8;
        }
        self.gain = db_gain(p.or("volume", 0.0));
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
        self.clock += 1;
        let p = &self.pads[pad as usize];
        if p.choke > 0 {
            for v in self.voices.iter_mut().filter(|v| v.active && v.choke == p.choke) {
                v.choked = true;
            }
        }
        let len = match &p.sample {
            Some(s) => s.frames() as f32 / p.tune * p.decay.min(1.0),
            None => MODELS[pad as usize].length() * p.decay * self.sr,
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

    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let choke_step = 1.0 / (CHOKE_SECONDS * self.sr);
        for v in self.voices.iter_mut().filter(|v| v.active) {
            let pad = &self.pads[v.pad as usize];
            let level = v.velocity * pad.gain * self.gain;
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                if v.t >= v.len {
                    v.active = false;
                    break;
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
                        let model = MODELS[v.pad as usize];
                        model_sample(model, v, &self.coefs, self.sr, pad.tune, pad.decay) * model.level()
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
                        break;
                    }
                }
                let out = s * level * v.fade;
                *l += out * pad.pan.0;
                *r += out * pad.pan.1;
                v.t += 1;
            }
        }
    }

    pub fn adopt(&mut self, old: &mut DrumRack) {
        std::mem::swap(&mut self.voices, &mut old.voices);
        self.clock = old.clock;
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
    }

    pub fn is_silent(&self) -> bool {
        !self.voices.iter().any(|v| v.active)
    }
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
        Model::ClosedHat | Model::PedalHat | Model::OpenHat | Model::Crash | Model::Ride => {
            let (ratio, speed, cutoff, noise, gain) = match model {
                Model::ClosedHat => (1.7, 60.0, c.lp7k, 0.6, 1.4),
                Model::PedalHat => (1.7, 30.0, c.lp7k, 0.5, 1.1),
                Model::OpenHat => (1.7, 6.0, c.lp7k, 0.6, 1.2),
                Model::Crash => (1.25, 2.2, c.lp5k, 0.8, 1.0),
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
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use soniq_io::AudioBuffer;

    use super::{FIRST_PAD, PAD_NAMES, PADS};
    use crate::test_util::*;
    use crate::{Instrument, NoteEvent, SampleSlot};

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
        assert_eq!(levels.len(), PADS);
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
        r.handle(NoteEvent::On { pitch: 60, velocity: 1.0 });
        assert!(r.is_silent());
    }
}
