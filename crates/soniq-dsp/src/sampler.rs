//! Sampler: one sample across the keyboard, pitched from its root note.
//! Classic mode follows the keys with an envelope, one-shot plays the whole
//! sample whatever the key does, loop mode repeats between the start and end
//! points while the key is held, and slice mode cuts the sample into pieces
//! (equal, or at its transients) with one key per piece from the root up.
//! An LFO can move the pitch, the filter or the volume.

use std::sync::Arc;

use soniq_io::AudioBuffer;

use crate::common::{Adsr, Env, Svf, SvfCoef, db_gain, hermite, pick_voice, sin_turns};
use crate::{NoteEvent, Params};

const VOICES: usize = 16;
/// Frames faded out before the end of the sample (or slice) so it never clicks.
const END_FADE: f64 = 64.0;
/// Samples between LFO updates.
const CONTROL: usize = 32;
pub const MAX_SLICES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Classic,
    OneShot,
    Loop,
    Slice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LfoTarget {
    Pitch,
    Filter,
    Volume,
}

#[derive(Debug, Clone, Copy, Default)]
struct Voice {
    active: bool,
    held: bool,
    sustained: bool,
    pitch: u8,
    velocity: f32,
    age: u64,
    /// Read position in frames, frames advanced per output frame, and where the voice ends.
    pos: f64,
    speed: f64,
    end: f64,
    env: Env,
    svf: [Svf; 2],
}

pub struct Sampler {
    sample: Option<Arc<AudioBuffer>>,
    /// Transients in the sample (frames), for slicing by them.
    onsets: Option<Arc<Vec<usize>>>,
    mode: Mode,
    root: f64,
    tune: f64,
    start: f64,
    end: f64,
    /// Slice start frames (slice mode).
    slices: [f64; MAX_SLICES],
    slice_count: usize,
    amp: Adsr,
    cutoff: f32,
    res: f32,
    /// `None` while the filter is wide open and nothing moves it.
    filter: Option<SvfCoef>,
    vel_sens: f32,
    gain: f32,
    lfo_step: f32,
    lfo_amount: f32,
    lfo_target: LfoTarget,
    lfo_phase: f32,
    sr: f32,
    voices: [Voice; VOICES],
    clock: u64,
    pedal: bool,
}

impl Sampler {
    /// `onsets` are transient positions (frames) found in the sample, for slicing by transients.
    pub fn new(sr: f32, p: &dyn Params, sample: Option<Arc<AudioBuffer>>, onsets: Option<Arc<Vec<usize>>>) -> Self {
        let mut sampler = Self {
            sample,
            onsets,
            mode: Mode::Classic,
            root: 60.0,
            tune: 0.0,
            start: 0.0,
            end: 0.0,
            slices: [0.0; MAX_SLICES],
            slice_count: 1,
            amp: Adsr::new(0.001, 0.5, 1.0, 0.12, sr),
            cutoff: 18_000.0,
            res: 0.0,
            filter: None,
            vel_sens: 0.6,
            gain: 1.0,
            lfo_step: 0.0,
            lfo_amount: 0.0,
            lfo_target: LfoTarget::Pitch,
            lfo_phase: 0.0,
            sr,
            voices: [Voice::default(); VOICES],
            clock: 0,
            pedal: false,
        };
        sampler.configure(p);
        sampler
    }

    /// Takes new parameter values; playing voices carry on.
    pub fn configure(&mut self, p: &dyn Params) {
        let sr = self.sr;
        let sec = |id: &str, d: f64| (p.or(id, d) / 1000.0) as f32;
        self.mode = match p.or("mode", 0.0).round() as i64 {
            1 => Mode::OneShot,
            2 => Mode::Loop,
            3 => Mode::Slice,
            _ => Mode::Classic,
        };
        let frames = self.sample.as_ref().map_or(0, |s| s.frames()) as f64;
        let start = (p.or("start", 0.0) / 100.0).clamp(0.0, 1.0) * frames;
        let end = ((p.or("end", 100.0) / 100.0).clamp(0.0, 1.0) * frames).max((start + END_FADE * 2.0).min(frames));
        (self.start, self.end) = (start, end);

        // Slices: equal pieces of start..end, or the transients inside it.
        let wanted = (p.or("slices", 8.0).round() as usize).clamp(1, MAX_SLICES);
        self.slice_count = 0;
        match (p.or("sliceBy", 0.0).round() as i64, self.onsets.as_deref()) {
            (1, Some(points)) => {
                for &at in points.iter().filter(|&&at| (at as f64) >= start && (at as f64) < end).take(wanted) {
                    self.slices[self.slice_count] = at as f64;
                    self.slice_count += 1;
                }
            }
            _ => {
                for (k, s) in self.slices.iter_mut().enumerate().take(wanted) {
                    *s = start + (end - start) * k as f64 / wanted as f64;
                }
                self.slice_count = wanted;
            }
        }
        if self.slice_count == 0 {
            self.slice_count = 1;
            self.slices[0] = start;
        }

        // One-shots and slices ignore the key: full level until the sample ends.
        let sustain = if matches!(self.mode, Mode::OneShot | Mode::Slice) { 1.0 } else { (p.or("sustain", 100.0) / 100.0) as f32 };
        self.cutoff = p.or("cutoff", 18000.0) as f32;
        self.res = (p.or("res", 0.0) / 100.0) as f32;
        self.lfo_amount = (p.or("lfoAmt", 0.0) / 100.0) as f32;
        self.lfo_target = match p.or("lfoDest", 0.0).round() as i64 {
            1 => LfoTarget::Filter,
            2 => LfoTarget::Volume,
            _ => LfoTarget::Pitch,
        };
        let filtered = self.cutoff < 17_999.0 || self.res > 0.0 || (self.lfo_amount > 0.0 && self.lfo_target == LfoTarget::Filter);
        self.filter = filtered.then(|| SvfCoef::new(self.cutoff, self.res, sr));
        self.root = p.or("root", 60.0);
        self.tune = p.or("tune", 0.0);
        self.amp = Adsr::new(sec("attack", 1.0), sec("decay", 500.0), sustain, sec("release", 120.0), sr);
        self.vel_sens = (p.or("velSens", 60.0) / 100.0) as f32;
        self.gain = db_gain(p.or("volume", 0.0));
        self.lfo_step = p.or("lfoRate", 4.0) as f32 / sr;
    }

    pub fn handle(&mut self, event: NoteEvent) {
        match event {
            NoteEvent::On { pitch, velocity } => {
                if self.sample.is_none() {
                    return;
                }
                let (from, to, speed) = if self.mode == Mode::Slice {
                    // One key per slice, from the root up, at the sample's own pitch.
                    let k = f64::from(pitch) - self.root;
                    if k < 0.0 || k >= self.slice_count as f64 {
                        return;
                    }
                    let k = k as usize;
                    let to = if k + 1 < self.slice_count { self.slices[k + 1] } else { self.end };
                    (self.slices[k], to, (self.tune / 12.0).exp2())
                } else {
                    (self.start, self.end, ((f64::from(pitch) - self.root + self.tune) / 12.0).exp2())
                };
                self.clock += 1;
                let i = pick_voice(self.voices.iter().map(|v| (v.active, v.held, v.age)));
                let v = &mut self.voices[i];
                // A stolen voice keeps its level and fades into the new note.
                let env = if v.active { v.env } else { Env::default() };
                *v = Voice { active: true, held: true, pitch, velocity, age: self.clock, pos: from, speed, end: to, env, ..Voice::default() };
                v.env.start();
            }
            NoteEvent::Off { pitch } => {
                let (pedal, ignores_keys) = (self.pedal, matches!(self.mode, Mode::OneShot | Mode::Slice));
                for v in self.voices.iter_mut().filter(|v| v.active && v.held && v.pitch == pitch) {
                    v.held = false;
                    if ignores_keys {
                        continue;
                    }
                    if pedal { v.sustained = true } else { v.env.release() }
                }
            }
            NoteEvent::Sustain(down) => {
                self.pedal = down;
                if !down {
                    for v in self.voices.iter_mut().filter(|v| v.sustained) {
                        v.sustained = false;
                        v.env.release();
                    }
                }
            }
            NoteEvent::AllNotesOff => {
                self.pedal = false;
                for v in self.voices.iter_mut().filter(|v| v.active) {
                    v.held = false;
                    v.sustained = false;
                    v.env.release();
                }
            }
            NoteEvent::AllSoundOff => {
                self.pedal = false;
                self.voices = [Voice::default(); VOICES];
            }
        }
    }

    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let Some(sample) = self.sample.clone() else { return };
        let Some(data_l) = sample.channels.first() else { return };
        let data_r = sample.channels.get(1).unwrap_or(data_l);
        let looping = self.mode == Mode::Loop && self.end - self.start > 1.0;
        let mut done = 0;
        while done < left.len() {
            let len = (left.len() - done).min(CONTROL);
            // The LFO moves at control rate, shared by every voice.
            let lfo = if self.lfo_amount > 0.0 { sin_turns(self.lfo_phase) * self.lfo_amount } else { 0.0 };
            self.lfo_phase = (self.lfo_phase + self.lfo_step * len as f32).fract();
            // Pitch swings up to an octave either way; volume dips by up to the amount.
            let (pitch_mod, gain_mod) = match self.lfo_target {
                LfoTarget::Pitch => (f64::from(lfo).exp2(), 1.0),
                LfoTarget::Volume => (1.0, 1.0 - 0.5 * (self.lfo_amount - lfo)),
                LfoTarget::Filter => (1.0, 1.0),
            };
            let coef = match (self.lfo_target, self.filter) {
                (LfoTarget::Filter, Some(_)) if lfo != 0.0 => Some(SvfCoef::new(self.cutoff * (lfo * 3.0).exp2(), self.res, self.sr)),
                (_, coef) => coef,
            };
            let (out_l, out_r) = (&mut left[done..done + len], &mut right[done..done + len]);
            for v in self.voices.iter_mut().filter(|v| v.active) {
                let vel = 1.0 - self.vel_sens + self.vel_sens * v.velocity;
                for (l, r) in out_l.iter_mut().zip(out_r.iter_mut()) {
                    if v.pos >= v.end {
                        // Loops keep cycling, through the release too.
                        if looping {
                            v.pos = self.start + (v.pos - v.end) % (v.end - self.start);
                        } else {
                            v.active = false;
                            break;
                        }
                    }
                    let (mut a, mut b) = (hermite(data_l, v.pos), hermite(data_r, v.pos));
                    if !looping {
                        let left_frames = (v.end - v.pos) / v.speed;
                        if left_frames < END_FADE {
                            let g = (left_frames / END_FADE) as f32;
                            a *= g;
                            b *= g;
                        }
                    }
                    if let Some(c) = &coef {
                        a = v.svf[0].tick(c, a).0;
                        b = v.svf[1].tick(c, b).0;
                    }
                    let amp = v.env.next(&self.amp) * vel * self.gain * gain_mod;
                    *l += a * amp;
                    *r += b * amp;
                    v.pos += v.speed * pitch_mod;
                }
                if v.env.is_idle() {
                    v.active = false;
                }
            }
            done += len;
        }
    }

    pub fn adopt(&mut self, old: &mut Sampler) {
        std::mem::swap(&mut self.voices, &mut old.voices);
        self.clock = old.clock;
        self.pedal = old.pedal;
        self.lfo_phase = old.lfo_phase;
        // A different sample (or none) leaves nothing to continue.
        let same = match (&self.sample, &old.sample) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        if !same {
            self.voices = [Voice::default(); VOICES];
        }
    }

    pub fn is_silent(&self) -> bool {
        !self.voices.iter().any(|v| v.active)
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;
    use std::sync::Arc;

    use soniq_io::AudioBuffer;

    use crate::test_util::*;
    use crate::{Instrument, NoteEvent, SampleSlot};

    fn sine(hz: f32, frames: usize) -> Arc<AudioBuffer> {
        Arc::new(AudioBuffer::new(SR, vec![(0..frames).map(|i| (i as f32 * hz / SR as f32 * TAU).sin() * 0.5).collect()]))
    }

    fn sampler(params: &[(&str, f64)], buffer: Arc<AudioBuffer>) -> Instrument {
        let owned: Vec<(String, f64)> = params.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        let lookup = move |id: &str| owned.iter().find(|(k, _)| k == id).map(|(_, v)| *v);
        Instrument::build("sampler", SR, &lookup, &[SampleSlot { key: 0, buffer, onsets: None }]).unwrap()
    }

    #[test]
    fn an_octave_up_plays_twice_as_fast() {
        let mut s = sampler(&[("root", 69.0)], sine(440.0, SR as usize));
        s.handle(NoteEvent::On { pitch: 81, velocity: 1.0 });
        let (l, _) = render(&mut s, SR as usize);
        let f = frequency(&l[1000..20_000], SR);
        assert!((f - 880.0).abs() < 2.0, "{f}");
        // A one-second sample at double speed is over after half a second.
        assert!(peak(&l[24_100..]) < 1e-6);
        assert!(s.is_silent());
    }

    #[test]
    fn one_shots_ignore_the_key_and_loops_keep_going() {
        let mut one = sampler(&[("mode", 1.0)], sine(220.0, 4800));
        one.handle(NoteEvent::On { pitch: 60, velocity: 1.0 });
        one.handle(NoteEvent::Off { pitch: 60 });
        let (l, _) = render(&mut one, 4000);
        assert!(peak(&l[3000..]) > 0.3, "still playing after the key went up");

        let mut looped = sampler(&[("mode", 2.0)], sine(220.0, 4800));
        looped.handle(NoteEvent::On { pitch: 60, velocity: 1.0 });
        let (l, _) = render(&mut looped, 20_000);
        assert!(peak(&l[15_000..]) > 0.3, "loops past the end of the sample");
    }

    #[test]
    fn slices_play_one_piece_per_key() {
        // Four equal slices of a ramp 0…1: the third key plays the third quarter.
        let ramp: Vec<f32> = (0..4800).map(|i| i as f32 / 4800.0).collect();
        let buffer = Arc::new(AudioBuffer::new(SR, vec![ramp]));
        let mut s = sampler(&[("mode", 3.0), ("slices", 4.0), ("root", 36.0), ("velSens", 0.0)], buffer);
        s.handle(NoteEvent::On { pitch: 38, velocity: 1.0 });
        let (l, _) = render(&mut s, 2400);
        assert!((l[100] - (2400.0 + 100.0) / 4800.0).abs() < 0.01, "{}", l[100]);
        assert!(peak(&l[1250..]) < 1e-6, "the slice ends at the next one");
        s.handle(NoteEvent::On { pitch: 40, velocity: 1.0 });
        assert!(s.is_silent(), "no fifth slice");
    }

    #[test]
    fn the_lfo_moves_volume_and_pitch() {
        // Tremolo: at 4 Hz the level swings between full and silent every 125 ms.
        let mut s = sampler(&[("mode", 2.0), ("lfoAmt", 100.0), ("lfoDest", 2.0), ("lfoRate", 4.0), ("velSens", 0.0)], sine(220.0, 48_000));
        s.handle(NoteEvent::On { pitch: 60, velocity: 1.0 });
        let (l, _) = render(&mut s, 24_000);
        let levels: Vec<f32> = l.chunks(1200).map(peak).collect();
        let (low, high) = levels.iter().fold((f32::MAX, 0.0_f32), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        assert!(high > 0.45 && low < 0.1, "{low} … {high}");

        // Vibrato: the pitch leaves 220 Hz and comes back.
        let mut s = sampler(&[("mode", 2.0), ("lfoAmt", 10.0), ("lfoDest", 0.0), ("lfoRate", 2.0)], sine(220.0, 48_000));
        s.handle(NoteEvent::On { pitch: 60, velocity: 1.0 });
        let (l, _) = render(&mut s, 24_000);
        let (up, down) = (frequency(&l[2_000..10_000], SR), frequency(&l[14_000..22_000], SR));
        assert!(up > 225.0 && down < 215.0, "{up} {down}");
    }

    #[test]
    fn slices_follow_the_transients_when_asked() {
        let buffer = sine(220.0, 48_000);
        let lookup = |id: &str| match id {
            "mode" => Some(3.0),
            "sliceBy" => Some(1.0),
            "root" => Some(60.0),
            _ => None,
        };
        let onsets = Arc::new(vec![0, 30_000]);
        let mut s = Instrument::build("sampler", SR, &lookup, &[SampleSlot { key: 0, buffer, onsets: Some(onsets) }]).unwrap();
        s.handle(NoteEvent::On { pitch: 61, velocity: 1.0 });
        let (l, _) = render(&mut s, 20_000);
        // The second slice runs from 30 000 to the end: 18 000 frames.
        assert!(peak(&l[17_000..17_900]) > 0.1 && peak(&l[18_100..]) < 1e-6);
    }
}
