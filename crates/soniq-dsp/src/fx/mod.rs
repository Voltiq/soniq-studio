//! Audio effects for the real-time engine: inserts on tracks, returns and
//! the master. Each is built on the control thread from a device's
//! parameters (delay lines and other buffers are allocated there) and then
//! processes blocks in place on the audio thread without allocating. A
//! rebuilt effect adopts the old one's state (filter memories, delay lines,
//! envelopes, LFO phases), so turning a knob never clicks or cuts a tail;
//! automation and modulators retune a running effect with `configure`.

mod bus;
mod character;
mod convolution;
mod dynamics;
mod eq;
mod modulation;
mod space;
mod tone;
mod vocoder;

pub use bus::{DeEsser, Glue, Multiband, TransientShaper};
pub use character::{AutoPan, PitchShifter, Redux};
pub use convolution::Convolution;
pub use dynamics::{Compressor, Gate, Limiter};
pub use eq::{BAND_TYPES, BANDS, Eq};
pub use modulation::{AutoFilter, Chorus, Phaser};
pub use space::{Delay, Reverb};
pub use tone::{Saturator, Utility};
pub use vocoder::Vocoder;

use crate::{External, FxContext, Params};

/// A sidechain key for one block: the left and right signal of the track it listens to.
pub type Sidechain<'a> = Option<(&'a [f32], &'a [f32])>;

// The engine keeps each effect in place inside its chain, so variant sizes do not matter.
#[allow(clippy::large_enum_variant)]
pub enum AudioFx {
    Eq(Eq),
    Compressor(Compressor),
    Gate(Gate),
    Limiter(Limiter),
    Reverb(Reverb),
    Delay(Delay),
    Chorus(Chorus),
    Phaser(Phaser),
    AutoFilter(AutoFilter),
    Saturator(Saturator),
    Utility(Utility),
    Glue(Glue),
    Multiband(Multiband),
    DeEsser(DeEsser),
    TransientShaper(TransientShaper),
    AutoPan(AutoPan),
    Redux(Redux),
    PitchShifter(PitchShifter),
    Convolution(Convolution),
    Vocoder(Vocoder),
    /// A plugin running in its own process.
    External(Box<dyn External>),
}

/// Device types that are audio effects the engine can run.
pub const AUDIO_FX_TYPES: [&str; 20] = [
    "eq",
    "comp",
    "gate",
    "limiter",
    "reverb",
    "delay",
    "chorus",
    "phaser",
    "autofilter",
    "saturator",
    "utility",
    "glue",
    "multiband",
    "deesser",
    "transient",
    "autopan",
    "redux",
    "pitch",
    "convolution",
    "vocoder",
];

impl AudioFx {
    /// The effect for a device type, or `None` if the type is not one.
    pub fn build(kind: &str, sample_rate: u32, p: &dyn Params) -> Option<Self> {
        let sr = sample_rate.max(1) as f32;
        Some(match kind {
            "eq" => Self::Eq(Eq::new(sr, p)),
            "comp" => Self::Compressor(Compressor::new(sr, p)),
            "gate" => Self::Gate(Gate::new(sr, p)),
            "limiter" => Self::Limiter(Limiter::new(sr, p)),
            "reverb" => Self::Reverb(Reverb::new(sr, p)),
            "delay" => Self::Delay(Delay::new(sr, p)),
            "chorus" => Self::Chorus(Chorus::new(sr, p)),
            "phaser" => Self::Phaser(Phaser::new(sr, p)),
            "autofilter" => Self::AutoFilter(AutoFilter::new(sr, p)),
            "saturator" => Self::Saturator(Saturator::new(sr, p)),
            "utility" => Self::Utility(Utility::new(sr, p)),
            "glue" => Self::Glue(Glue::new(sr, p)),
            "multiband" => Self::Multiband(Multiband::new(sr, p)),
            "deesser" => Self::DeEsser(DeEsser::new(sr, p)),
            "transient" => Self::TransientShaper(TransientShaper::new(sr, p)),
            "autopan" => Self::AutoPan(AutoPan::new(sr, p)),
            "redux" => Self::Redux(Redux::new(sr, p)),
            "pitch" => Self::PitchShifter(PitchShifter::new(sr, p)),
            "convolution" => Self::Convolution(Convolution::new(sr, p)),
            "vocoder" => Self::Vocoder(Vocoder::new(sr, p)),
            _ => return None,
        })
    }

    /// Processes one block in place. `sidechain` is the key signal for the
    /// effects that listen to another track (compressor, gate, the vocoder's
    /// carrier); the others ignore it.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32], sidechain: Sidechain<'_>, ctx: &FxContext) {
        let n = left.len().min(right.len());
        let (left, right) = (&mut left[..n], &mut right[..n]);
        let sidechain = sidechain.map(|(l, r)| (&l[..n.min(l.len())], &r[..n.min(r.len())])).filter(|(l, r)| l.len() == n && r.len() == n);
        match self {
            Self::Eq(fx) => fx.process(left, right),
            Self::Compressor(fx) => fx.process(left, right, sidechain),
            Self::Gate(fx) => fx.process(left, right, sidechain),
            Self::Limiter(fx) => fx.process(left, right),
            Self::Reverb(fx) => fx.process(left, right),
            Self::Delay(fx) => fx.process(left, right, ctx),
            Self::Chorus(fx) => fx.process(left, right),
            Self::Phaser(fx) => fx.process(left, right),
            Self::AutoFilter(fx) => fx.process(left, right),
            Self::Saturator(fx) => fx.process(left, right),
            Self::Utility(fx) => fx.process(left, right),
            Self::Glue(fx) => fx.process(left, right),
            Self::Multiband(fx) => fx.process(left, right),
            Self::DeEsser(fx) => fx.process(left, right),
            Self::TransientShaper(fx) => fx.process(left, right),
            Self::AutoPan(fx) => fx.process(left, right, ctx),
            Self::Redux(fx) => fx.process(left, right),
            Self::PitchShifter(fx) => fx.process(left, right),
            Self::Convolution(fx) => fx.process(left, right),
            Self::Vocoder(fx) => fx.process(left, right, sidechain),
            Self::External(fx) => fx.process(ctx, left, right, &[]),
        }
    }

    /// Takes new parameter values (automation, modulators) without
    /// allocating: delay lines, envelopes and filter memories carry on.
    pub fn configure(&mut self, p: &dyn Params) {
        match self {
            Self::Eq(fx) => fx.configure(p),
            Self::Compressor(fx) => fx.configure(p),
            Self::Gate(fx) => fx.configure(p),
            Self::Limiter(fx) => fx.configure(p),
            Self::Reverb(fx) => fx.configure(p),
            Self::Delay(fx) => fx.configure(p),
            Self::Chorus(fx) => fx.configure(p),
            Self::Phaser(fx) => fx.configure(p),
            Self::AutoFilter(fx) => fx.configure(p),
            Self::Saturator(fx) => fx.configure(p),
            Self::Utility(fx) => fx.configure(p),
            Self::Glue(fx) => fx.configure(p),
            Self::Multiband(fx) => fx.configure(p),
            Self::DeEsser(fx) => fx.configure(p),
            Self::TransientShaper(fx) => fx.configure(p),
            Self::AutoPan(fx) => fx.configure(p),
            Self::Redux(fx) => fx.configure(p),
            Self::PitchShifter(fx) => fx.configure(p),
            Self::Convolution(fx) => fx.configure(p),
            Self::Vocoder(fx) => fx.configure(p),
            Self::External(fx) => fx.configure(p),
        }
    }

    /// Samples its output comes late by (only plugins report any; the
    /// convolution reverb's late wet sound is part of its sound).
    pub fn latency(&self) -> u32 {
        match self {
            Self::External(fx) => fx.latency(),
            _ => 0,
        }
    }

    /// Carries on from the effect this one replaces: different kinds share nothing.
    pub fn adopt(&mut self, old: &mut AudioFx) {
        match (self, old) {
            (Self::Eq(new), Self::Eq(old)) => new.adopt(old),
            (Self::Compressor(new), Self::Compressor(old)) => new.adopt(old),
            (Self::Gate(new), Self::Gate(old)) => new.adopt(old),
            (Self::Limiter(new), Self::Limiter(old)) => new.adopt(old),
            (Self::Reverb(new), Self::Reverb(old)) => new.adopt(old),
            (Self::Delay(new), Self::Delay(old)) => new.adopt(old),
            (Self::Chorus(new), Self::Chorus(old)) => new.adopt(old),
            (Self::Phaser(new), Self::Phaser(old)) => new.adopt(old),
            (Self::AutoFilter(new), Self::AutoFilter(old)) => new.adopt(old),
            (Self::Saturator(new), Self::Saturator(old)) => new.adopt(old),
            (Self::Utility(new), Self::Utility(old)) => new.adopt(old),
            (Self::Glue(new), Self::Glue(old)) => new.adopt(old),
            (Self::Multiband(new), Self::Multiband(old)) => new.adopt(old),
            (Self::DeEsser(new), Self::DeEsser(old)) => new.adopt(old),
            (Self::TransientShaper(new), Self::TransientShaper(old)) => new.adopt(old),
            (Self::AutoPan(new), Self::AutoPan(old)) => new.adopt(old),
            (Self::Redux(new), Self::Redux(old)) => new.adopt(old),
            (Self::PitchShifter(new), Self::PitchShifter(old)) => new.adopt(old),
            (Self::Convolution(new), Self::Convolution(old)) => new.adopt(old),
            (Self::Vocoder(new), Self::Vocoder(old)) => new.adopt(old),
            _ => {}
        }
    }
}

/// Moves toward a target a little every sample, so level changes never click.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Smooth {
    value: f32,
    target: f32,
    coef: f32,
}

impl Smooth {
    /// Settles in about 20 ms.
    pub fn new(target: f32, sr: f32) -> Self {
        Self { value: target, target, coef: 1.0 - (-1.0 / (0.02 * sr)).exp() }
    }

    pub fn next(&mut self) -> f32 {
        self.value += (self.target - self.value) * self.coef;
        self.value
    }

    /// Heads for a new target from wherever it is.
    pub fn set(&mut self, target: f32) {
        self.target = target;
    }

    /// Starts from where the old one had got to.
    pub fn adopt(&mut self, old: &Smooth) {
        self.value = old.value;
    }
}

/// Per-sample factor of a one-pole smoother with time constant `ms`.
pub(crate) fn time_coef(ms: f64, sr: f32) -> f32 {
    (-1.0 / (ms.max(0.01) as f32 * 0.001 * sr)).exp()
}

/// Peak level of a stereo sample in dB (−120 for silence).
pub(crate) fn level_db(l: f32, r: f32) -> f32 {
    20.0 * l.abs().max(r.abs()).max(1e-6).log10()
}

/// The option index of a parameter.
pub(crate) fn choice(p: &dyn Params, id: &str, default: f64) -> usize {
    p.or(id, default).round().max(0.0) as usize
}

/// A delay line with fractional reads, for echoes and modulated delays.
#[derive(Debug, Clone, Default)]
pub(crate) struct DelayLine {
    data: Vec<f32>,
    write: usize,
}

impl DelayLine {
    /// Holds up to `seconds` (allocates: build time only).
    pub fn new(seconds: f32, sr: f32) -> Self {
        Self { data: vec![0.0; ((seconds * sr) as usize).max(4)], write: 0 }
    }

    pub fn push(&mut self, x: f32) {
        self.data[self.write] = x;
        self.write = (self.write + 1) % self.data.len();
    }

    /// The sample `delay` samples before the last one pushed (linear interpolation).
    pub fn read(&self, delay: f32) -> f32 {
        let len = self.data.len();
        let delay = delay.clamp(1.0, (len - 2) as f32);
        let whole = delay.floor();
        let frac = delay - whole;
        let i = (self.write + len - 1 - whole as usize) % len;
        let j = (i + len - 1) % len;
        self.data[i] + (self.data[j] - self.data[i]) * frac
    }

    /// Takes the old line's contents when it is the same size.
    pub fn adopt(&mut self, old: &mut DelayLine) {
        if self.data.len() == old.data.len() {
            std::mem::swap(&mut self.data, &mut old.data);
            self.write = old.write;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::*;
    use super::{AUDIO_FX_TYPES, AudioFx};

    #[test]
    fn a_retuned_effect_sounds_like_a_fresh_one() {
        // Settings away from every default, for whichever ids each effect reads.
        let changed = [
            ("gain", 6.0),
            ("mix", 60.0),
            ("drive", 12.0),
            ("thresh", -30.0),
            ("ratio", 6.0),
            ("cutoff", 800.0),
            ("rate", 2.0),
            ("depth", 80.0),
            ("fb", 50.0),
            ("time", 120.0),
            ("sync", 0.0),
            ("size", 80.0),
            ("decay", 1.2),
            ("b4gain", 6.0),
            ("b4freq", 1500.0),
            ("output", -3.0),
            ("width", 150.0),
            ("pan", 20.0),
            ("ceiling", -6.0),
            ("range", -40.0),
            ("release", 200.0),
            ("type", 1.0),
            ("mode", 1.0),
            ("tone", 30.0),
        ];
        let input = sine(330.0, 0.4, 24_000);
        for kind in AUDIO_FX_TYPES {
            let mut retuned = AudioFx::build(kind, SR, &params(&[])).unwrap();
            retuned.configure(&params(&changed));
            let mut fresh = AudioFx::build(kind, SR, &params(&changed)).unwrap();
            let (a, _) = run(&mut retuned, &input);
            let (b, _) = run(&mut fresh, &input);
            // Smoothed levels glide over the first moments; after that the two agree.
            let worst = a[12_000..].iter().zip(&b[12_000..]).fold(0.0_f32, |m, (x, y)| m.max((x - y).abs()));
            assert!(worst < 1e-3, "{kind}: {worst}");
        }
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use std::f32::consts::TAU;

    use crate::FxContext;

    pub const SR: u32 = 48_000;

    pub fn ctx() -> FxContext {
        FxContext { playing: true, pos: 0, beat: 0.0, samples_per_beat: 24_000.0, sample_rate: SR as f32 }
    }

    pub fn params<'a>(list: &'a [(&'a str, f64)]) -> impl Fn(&str) -> Option<f64> + 'a {
        move |id: &str| list.iter().find(|(k, _)| *k == id).map(|(_, v)| *v)
    }

    pub fn sine(hz: f32, amp: f32, frames: usize) -> Vec<f32> {
        (0..frames).map(|i| (i as f32 * hz / SR as f32 * TAU).sin() * amp).collect()
    }

    /// Runs an effect over a stereo copy of `input`, 512 frames at a time.
    pub fn run(fx: &mut super::AudioFx, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
        run_keyed(fx, input, None)
    }

    pub fn run_keyed(fx: &mut super::AudioFx, input: &[f32], key: Option<&[f32]>) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (input.to_vec(), input.to_vec());
        let mut at = 0;
        while at < l.len() {
            let end = (at + 512).min(l.len());
            let key = key.map(|k| (&k[at..end], &k[at..end]));
            let c = FxContext { pos: at as u64, beat: at as f64 / 24_000.0, ..ctx() };
            fx.process(&mut l[at..end], &mut r[at..end], key, &c);
            at = end;
        }
        (l, r)
    }

    pub fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
    }

    pub fn db(x: f32) -> f32 {
        20.0 * x.max(1e-9).log10()
    }

    pub fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }
}
