//! Soniq Synth: two oscillators and a sub, unison, a state-variable filter
//! with drive, amp and filter envelopes, an LFO on the cutoff, vibrato and
//! glide, played poly, mono or legato.

use crate::common::{Adsr, Env, Noise, Svf, SvfCoef, Wave, db_gain, midi_hz, osc, pan_gains, pick_voice, sin_turns};
use crate::{NoteEvent, Params};

const VOICES: usize = 16;
const MAX_UNISON: usize = 5;
/// Samples between modulation updates (cutoff, pitch).
const CONTROL: usize = 32;
/// Keeps a full chord of saws well below clipping.
const VOICE_GAIN: f32 = 0.22;
/// The widest unison detune, in semitones either side.
const MAX_SPREAD: f32 = 0.3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FilterKind {
    Lp24,
    Lp12,
    Bp,
    Hp,
}

impl FilterKind {
    /// The option index of a `filter` parameter.
    pub(crate) fn from_index(v: f64) -> Self {
        match v.round() as i64 {
            1 => Self::Lp12,
            2 => Self::Bp,
            3 => Self::Hp,
            _ => Self::Lp24,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Poly,
    Mono,
    Legato,
}

struct Settings {
    waves: [Wave; 2],
    /// Oscillator 2 against oscillator 1: semitones and detune together.
    osc2_ratio: f32,
    mix: f32,
    sub: f32,
    unison: usize,
    spread: f32,
    filter: FilterKind,
    cutoff: f32,
    res: f32,
    /// Linear drive into the filter; 1 is off.
    drive: f32,
    /// Octaves the cutoff moves at full filter envelope.
    env_amt: f32,
    amp: Adsr,
    fenv: Adsr,
    /// Turns per sample.
    lfo_rate: f32,
    /// Octaves of cutoff movement.
    lfo_amt: f32,
    vib_rate: f32,
    /// Semitones.
    vib_depth: f32,
    /// Fraction of the pitch distance left after each control step; 0 jumps.
    glide: f32,
    mode: Mode,
    gain: f32,
}

impl Settings {
    fn new(sr: f32, p: &dyn Params) -> Self {
        let sec = |id: &str, d: f64| (p.or(id, d) / 1000.0) as f32;
        let pct = |id: &str, d: f64| (p.or(id, d) / 100.0) as f32;
        let glide = sec("glide", 0.0);
        Self {
            waves: [Wave::from_index(p.or("osc1", 0.0)), Wave::from_index(p.or("osc2", 0.0))],
            osc2_ratio: ((p.or("osc2Semi", 0.0) + p.or("detune", 8.0) / 100.0) as f32 / 12.0).exp2(),
            mix: pct("mix", 50.0),
            sub: pct("sub", 0.0),
            unison: (p.or("unison", 1.0).round() as usize).clamp(1, MAX_UNISON),
            spread: pct("spread", 30.0),
            filter: FilterKind::from_index(p.or("filter", 0.0)),
            cutoff: p.or("cutoff", 2200.0) as f32,
            res: pct("res", 22.0),
            drive: db_gain(p.or("drive", 0.0)),
            env_amt: pct("envAmt", 30.0) * 4.0,
            amp: Adsr::new(sec("attack", 4.0), sec("decay", 300.0), pct("sustain", 80.0), sec("release", 280.0), sr),
            fenv: Adsr::new(sec("fAttack", 2.0), sec("fDecay", 400.0), pct("fSustain", 30.0), sec("fRelease", 300.0), sr),
            lfo_rate: p.or("lfoRate", 4.0) as f32 / sr,
            lfo_amt: pct("lfoAmt", 0.0) * 2.0,
            vib_rate: p.or("vibRate", 5.0) as f32 / sr,
            vib_depth: p.or("vibrato", 0.0) as f32 / 100.0,
            // Reaches 99 % of the way in the glide time.
            glide: if glide <= 0.0 { 0.0 } else { (-4.6 * CONTROL as f32 / (glide * sr)).exp() },
            mode: match p.or("mode", 0.0).round() as i64 {
                1 => Mode::Mono,
                2 => Mode::Legato,
                _ => Mode::Poly,
            },
            gain: db_gain(p.or("volume", 0.0)) * VOICE_GAIN,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Voice {
    active: bool,
    /// The key is down.
    held: bool,
    /// The key is up but the sustain pedal holds the note.
    sustained: bool,
    pitch: u8,
    velocity: f32,
    age: u64,
    /// Sounding pitch in semitones; glides towards `target`.
    note: f32,
    target: f32,
    /// Oscillator 1 and 2 phase per unison voice.
    phases: [[f32; 2]; MAX_UNISON],
    sub: f32,
    amp: Env,
    filt: Env,
    /// [left, right][first, second stage]
    svf: [[Svf; 2]; 2],
}

pub struct Synth {
    sr: f32,
    s: Settings,
    voices: [Voice; VOICES],
    clock: u64,
    lfo: f32,
    vib: f32,
    pedal: bool,
    /// Keys held in mono and legato modes, newest last.
    stack: [u8; 16],
    stack_len: usize,
    last_note: Option<f32>,
    rng: Noise,
}

impl Synth {
    pub fn new(sr: f32, params: &dyn Params) -> Self {
        Self {
            sr,
            s: Settings::new(sr, params),
            voices: [Voice::default(); VOICES],
            clock: 0,
            lfo: 0.0,
            vib: 0.0,
            pedal: false,
            stack: [0; 16],
            stack_len: 0,
            last_note: None,
            rng: Noise::default(),
        }
    }

    /// Takes new parameter values; playing voices carry on.
    pub fn configure(&mut self, params: &dyn Params) {
        self.s = Settings::new(self.sr, params);
    }

    pub fn handle(&mut self, event: NoteEvent) {
        match event {
            NoteEvent::On { pitch, velocity } => {
                self.clock += 1;
                if self.s.mode == Mode::Poly { self.poly_on(pitch, velocity) } else { self.mono_on(pitch, velocity) }
            }
            NoteEvent::Off { pitch } => {
                let poly = self.s.mode == Mode::Poly;
                if !poly {
                    self.mono_off(pitch);
                }
                // In mono modes voice 0 is handled above; the rest can still
                // hold notes from before a switch to mono.
                let pedal = self.pedal;
                for v in self.voices.iter_mut().skip(usize::from(!poly)).filter(|v| v.active && v.held && v.pitch == pitch) {
                    v.held = false;
                    if pedal {
                        v.sustained = true;
                    } else {
                        v.amp.release();
                        v.filt.release();
                    }
                }
            }
            NoteEvent::Sustain(down) => {
                self.pedal = down;
                if !down {
                    for v in self.voices.iter_mut().filter(|v| v.sustained) {
                        v.sustained = false;
                        v.amp.release();
                        v.filt.release();
                    }
                }
            }
            NoteEvent::AllNotesOff => {
                self.pedal = false;
                self.stack_len = 0;
                for v in self.voices.iter_mut().filter(|v| v.active) {
                    v.held = false;
                    v.sustained = false;
                    v.amp.release();
                    v.filt.release();
                }
            }
            NoteEvent::AllSoundOff => {
                self.pedal = false;
                self.stack_len = 0;
                self.voices = [Voice::default(); VOICES];
            }
        }
    }

    /// Resets a voice that was silent, with unison phases spread so the
    /// voices do not start in step.
    fn wake(&mut self, i: usize, note: f32) {
        let mut phases = [[0.0; 2]; MAX_UNISON];
        for p in phases.iter_mut().flatten() {
            *p = self.rng.next() * 0.5 + 0.5;
        }
        self.voices[i] = Voice { phases, note, ..Voice::default() };
    }

    fn poly_on(&mut self, pitch: u8, velocity: f32) {
        let i = pick_voice(self.voices.iter().map(|v| (v.active, v.held, v.age)));
        let target = f32::from(pitch);
        let from = if self.s.glide > 0.0 { self.last_note.unwrap_or(target) } else { target };
        if !self.voices[i].active {
            self.wake(i, from);
        }
        let clock = self.clock;
        let v = &mut self.voices[i];
        *v = Voice { active: true, held: true, sustained: false, pitch, velocity, age: clock, note: from, target, ..*v };
        v.amp.start();
        v.filt.start();
        self.last_note = Some(target);
    }

    fn stack_remove(&mut self, pitch: u8) {
        if let Some(at) = self.stack[..self.stack_len].iter().position(|&p| p == pitch) {
            self.stack.copy_within(at + 1..self.stack_len, at);
            self.stack_len -= 1;
        }
    }

    fn mono_on(&mut self, pitch: u8, velocity: f32) {
        self.stack_remove(pitch);
        if self.stack_len == self.stack.len() {
            self.stack.copy_within(1.., 0);
            self.stack_len -= 1;
        }
        self.stack[self.stack_len] = pitch;
        self.stack_len += 1;

        let target = f32::from(pitch);
        let legato = self.s.mode == Mode::Legato && self.voices[0].active && self.voices[0].held;
        if !self.voices[0].active {
            self.wake(0, target);
        }
        let glide = self.s.glide > 0.0;
        let clock = self.clock;
        let v = &mut self.voices[0];
        v.active = true;
        v.held = true;
        v.sustained = false;
        v.pitch = pitch;
        v.target = target;
        v.age = clock;
        if !glide {
            v.note = target;
        }
        if !legato {
            v.velocity = velocity;
            v.amp.start();
            v.filt.start();
        }
        self.last_note = Some(target);
    }

    fn mono_off(&mut self, pitch: u8) {
        self.stack_remove(pitch);
        let glide = self.s.glide > 0.0;
        let pedal = self.pedal;
        let v = &mut self.voices[0];
        if !v.active || !v.held {
            return;
        }
        if self.stack_len > 0 {
            // Back to the newest key still held, without a new attack.
            let top = self.stack[self.stack_len - 1];
            v.pitch = top;
            v.target = f32::from(top);
            if !glide {
                v.note = v.target;
            }
        } else if v.pitch == pitch {
            v.held = false;
            if pedal {
                v.sustained = true;
            } else {
                v.amp.release();
                v.filt.release();
            }
        }
    }

    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let mut done = 0;
        while done < left.len() {
            let len = (left.len() - done).min(CONTROL);
            let lfo = sin_turns(self.lfo);
            let vib = sin_turns(self.vib);
            self.lfo = (self.lfo + self.s.lfo_rate * len as f32).fract();
            self.vib = (self.vib + self.s.vib_rate * len as f32).fract();
            for v in self.voices.iter_mut().filter(|v| v.active) {
                render_voice(v, &self.s, self.sr, lfo, vib, &mut left[done..done + len], &mut right[done..done + len]);
            }
            done += len;
        }
    }

    pub fn adopt(&mut self, old: &mut Synth) {
        std::mem::swap(&mut self.voices, &mut old.voices);
        self.clock = old.clock;
        self.lfo = old.lfo;
        self.vib = old.vib;
        self.pedal = old.pedal;
        self.stack = old.stack;
        self.stack_len = old.stack_len;
        self.last_note = old.last_note;
    }

    pub fn is_silent(&self) -> bool {
        !self.voices.iter().any(|v| v.active)
    }
}

pub(crate) fn filter(stages: &mut [Svf; 2], c: &SvfCoef, c2: &SvfCoef, kind: FilterKind, x: f32) -> f32 {
    match kind {
        FilterKind::Lp24 => {
            let (low, _, _) = stages[0].tick(c, x);
            stages[1].tick(c2, low).0
        }
        FilterKind::Lp12 => stages[0].tick(c, x).0,
        FilterKind::Bp => stages[0].tick(c, x).1,
        FilterKind::Hp => stages[0].tick(c, x).2,
    }
}

fn render_voice(v: &mut Voice, s: &Settings, sr: f32, lfo: f32, vib: f32, left: &mut [f32], right: &mut [f32]) {
    v.note = if s.glide > 0.0 { v.target + (v.note - v.target) * s.glide } else { v.target };
    let base = midi_hz(v.note + vib * s.vib_depth) / sr;
    // The cutoff follows the filter envelope and the LFO at control rate.
    let cutoff = s.cutoff * (v.filt.level * s.env_amt + lfo * s.lfo_amt).exp2();
    let coef = SvfCoef::new(cutoff, s.res, sr);
    // In a 24 dB filter only the first stage resonates, so the peak stays single.
    let coef2 = SvfCoef::new(cutoff, 0.0, sr);

    let unison = s.unison;
    let mut steps = [[0.0_f32; 2]; MAX_UNISON];
    let mut pans = [(1.0_f32, 1.0_f32); MAX_UNISON];
    for u in 0..unison {
        let offset = if unison > 1 { u as f32 / (unison - 1) as f32 * 2.0 - 1.0 } else { 0.0 };
        let ratio = (offset * s.spread * MAX_SPREAD / 12.0).exp2();
        steps[u] = [(base * ratio).min(0.45), (base * ratio * s.osc2_ratio).min(0.45)];
        if unison > 1 {
            pans[u] = pan_gains(offset * s.spread);
        }
    }
    let norm = 1.0 / (unison as f32).sqrt();
    let sub_step = (base * 0.5).min(0.45);

    for (l_out, r_out) in left.iter_mut().zip(right.iter_mut()) {
        let (mut l, mut r) = (0.0, 0.0);
        for u in 0..unison {
            let ph = &mut v.phases[u];
            let x = osc(s.waves[0], ph[0], steps[u][0]) * (1.0 - s.mix) + osc(s.waves[1], ph[1], steps[u][1]) * s.mix;
            for (p, step) in ph.iter_mut().zip(steps[u]) {
                *p += step;
                if *p >= 1.0 {
                    *p -= 1.0;
                }
            }
            l += x * pans[u].0;
            r += x * pans[u].1;
        }
        l *= norm;
        r *= norm;
        if s.sub > 0.0 {
            let sq = osc(Wave::Square, v.sub, sub_step) * s.sub;
            l += sq;
            r += sq;
        }
        v.sub += sub_step;
        if v.sub >= 1.0 {
            v.sub -= 1.0;
        }
        if s.drive > 1.001 {
            l = (l * s.drive).tanh();
            r = (r * s.drive).tanh();
        }
        let yl = filter(&mut v.svf[0], &coef, &coef2, s.filter, l);
        let yr = filter(&mut v.svf[1], &coef, &coef2, s.filter, r);
        v.filt.next(&s.fenv);
        let amp = v.amp.next(&s.amp) * v.velocity * s.gain;
        *l_out += yl * amp;
        *r_out += yr * amp;
    }
    if v.amp.is_idle() {
        v.active = false;
        v.held = false;
        v.sustained = false;
    }
}

#[cfg(test)]
mod tests {
    use crate::test_util::*;
    use crate::{Instrument, NoteEvent};

    fn synth(params: &[(&str, f64)]) -> Instrument {
        let owned: Vec<(String, f64)> = params.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        let lookup = move |id: &str| owned.iter().find(|(k, _)| k == id).map(|(_, v)| *v);
        Instrument::build("synth", SR, &lookup, &[]).unwrap()
    }

    #[test]
    fn plays_the_right_pitch() {
        // A plain sine through an open filter: A4 must measure 440 Hz.
        let mut s = synth(&[("osc1", 3.0), ("mix", 0.0), ("cutoff", 18000.0), ("res", 0.0), ("envAmt", 0.0)]);
        s.handle(NoteEvent::On { pitch: 69, velocity: 1.0 });
        let (l, _) = render(&mut s, SR as usize / 2);
        let f = frequency(&l[4800..], SR);
        assert!((f - 440.0).abs() < 1.0, "{f}");
    }

    #[test]
    fn releases_to_silence() {
        let mut s = synth(&[("release", 50.0)]);
        s.handle(NoteEvent::On { pitch: 60, velocity: 0.8 });
        let (l, _) = render(&mut s, 4800);
        assert!(peak(&l) > 0.05);
        s.handle(NoteEvent::Off { pitch: 60 });
        render(&mut s, 9600);
        assert!(s.is_silent());
    }

    #[test]
    fn the_sustain_pedal_holds_released_notes() {
        let mut s = synth(&[("release", 20.0)]);
        s.handle(NoteEvent::Sustain(true));
        s.handle(NoteEvent::On { pitch: 60, velocity: 0.8 });
        s.handle(NoteEvent::Off { pitch: 60 });
        render(&mut s, 9600);
        assert!(!s.is_silent(), "held by the pedal");
        s.handle(NoteEvent::Sustain(false));
        render(&mut s, 9600);
        assert!(s.is_silent());
    }

    #[test]
    fn a_new_instrument_keeps_the_notes_of_the_old_one() {
        let mut old = synth(&[]);
        old.handle(NoteEvent::On { pitch: 64, velocity: 0.8 });
        render(&mut old, 2400);
        let mut new = synth(&[("cutoff", 800.0)]);
        new.adopt(&mut old);
        assert!(!new.is_silent() && old.is_silent());
        let (l, _) = render(&mut new, 2400);
        assert!(peak(&l) > 0.01);
        new.handle(NoteEvent::Off { pitch: 64 });
        render(&mut new, 48_000);
        assert!(new.is_silent(), "the adopted note still releases");
    }

    #[test]
    fn more_notes_than_voices_steal_the_oldest() {
        let mut s = synth(&[]);
        for p in 40..70 {
            s.handle(NoteEvent::On { pitch: p, velocity: 0.5 });
        }
        let (l, _) = render(&mut s, 2400);
        assert!(l.iter().all(|x| x.is_finite()) && peak(&l) < 4.0);
    }

    #[test]
    fn mono_mode_falls_back_to_the_held_note() {
        let mut s = synth(&[("mode", 1.0), ("osc1", 3.0), ("mix", 0.0), ("cutoff", 18000.0), ("envAmt", 0.0)]);
        s.handle(NoteEvent::On { pitch: 57, velocity: 1.0 });
        s.handle(NoteEvent::On { pitch: 69, velocity: 1.0 });
        s.handle(NoteEvent::Off { pitch: 69 });
        let (l, _) = render(&mut s, 24_000);
        let f = frequency(&l[4800..], SR);
        assert!((f - 220.0).abs() < 1.0, "{f}");
    }
}
