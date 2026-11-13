//! Wavetable: two oscillators that each read through a table of waveforms
//! (the position picks the waveform and can be swept while a note sounds), a
//! sub and noise, unison, a filter with drive, two envelopes, two LFOs, and a
//! modulation matrix: eight slots that each send a source (an envelope, an
//! LFO, velocity, the key, a random value per note) to a destination (a
//! table position, pitch, the filter, a level, the pan) by an amount.
//!
//! Every waveform is held band-limited once per octave, so high notes do not
//! alias. The tables are made the first time a Wavetable is built (on the
//! control thread) and shared from then on.

use std::sync::OnceLock;

use rustfft::FftPlanner;
use rustfft::num_complex::Complex32;

use crate::common::{Adsr, Env, Noise, Svf, SvfCoef, db_gain, midi_hz, pan_gains, pick_voice, sin_turns};
use crate::synth::{FilterKind, filter};
use crate::{NoteEvent, Params};

const VOICES: usize = 12;
const MAX_UNISON: usize = 5;
/// Samples between modulation updates.
const CONTROL: usize = 32;
/// Keeps a full chord well below clipping.
const VOICE_GAIN: f32 = 0.24;
/// The widest unison detune, in semitones either side.
const MAX_SPREAD: f32 = 0.3;

/// Samples in one waveform.
const SIZE: usize = 2048;
/// One waveform plus a copy of its first sample, so reads never wrap.
const ROW: usize = SIZE + 1;
/// Waveforms in a table.
pub const FRAMES: usize = 16;
/// Band-limited copies of every waveform: copy `m` keeps `HARMONICS >> m` harmonics.
const MIPS: usize = 11;
const HARMONICS: usize = 1024;

pub const TABLE_NAMES: [&str; 8] = ["Basic shapes", "Pulse width", "Harmonic sweep", "Vowels", "FM", "Sync", "Digital", "Organ"];

/// Slots in the modulation matrix (parameters `m1src`, `m1dst`, `m1amt`, …).
pub const SLOTS: usize = 8;
const SLOT_IDS: [[&str; 3]; SLOTS] = [
    ["m1src", "m1dst", "m1amt"],
    ["m2src", "m2dst", "m2amt"],
    ["m3src", "m3dst", "m3amt"],
    ["m4src", "m4dst", "m4amt"],
    ["m5src", "m5dst", "m5amt"],
    ["m6src", "m6dst", "m6amt"],
    ["m7src", "m7dst", "m7amt"],
    ["m8src", "m8dst", "m8amt"],
];

/// Matrix sources, by index: off, envelope 2, LFO 1, LFO 2, velocity, key, random.
const SOURCES: usize = 7;
/// Matrix destinations, by index.
const POS_A: usize = 0;
const POS_B: usize = 1;
const PITCH: usize = 2;
const PITCH_B: usize = 3;
const CUTOFF: usize = 4;
const RES: usize = 5;
const LEVEL_A: usize = 6;
const LEVEL_B: usize = 7;
const VOLUME: usize = 8;
const PAN: usize = 9;
const DESTS: usize = 10;

/// Semitones of pitch, and octaves of cutoff, a full amount moves.
const PITCH_RANGE: f32 = 12.0;
const CUTOFF_RANGE: f32 = 5.0;

/// One table: every waveform at every band limit, `mips[m][frame * ROW + i]`.
struct Table {
    mips: Vec<Vec<f32>>,
}

/// The harmonics of waveform `t` (0–1 through the table) of a table:
/// `out[k]` is harmonic `k` as a rotating pointer (its real part plays).
fn harmonics(table: usize, t: f32, planner: &mut FftPlanner<f32>) -> Vec<Complex32> {
    use std::f32::consts::{PI, TAU};
    let mut out = vec![Complex32::default(); HARMONICS + 1];
    // A harmonic that starts at zero and rises, like a sine.
    let sine = |a: f32| Complex32::new(0.0, -a);
    // A waveform given as a curve over one period: its harmonics are measured.
    let mut measured = |curve: &dyn Fn(f32) -> f32, out: &mut Vec<Complex32>| {
        let mut buf: Vec<Complex32> = (0..SIZE).map(|i| Complex32::new(curve(i as f32 / SIZE as f32), 0.0)).collect();
        planner.plan_fft_forward(SIZE).process(&mut buf);
        out[1..HARMONICS].copy_from_slice(&buf[1..HARMONICS]);
    };
    match table {
        // Sine to triangle to saw to square.
        0 => {
            let shape = |which: usize, k: usize| -> f32 {
                let kf = k as f32;
                let odd = k % 2 == 1;
                match which {
                    0 => f32::from(u8::from(k == 1)),
                    1 if odd => (8.0 / (PI * PI)) / (kf * kf) * if (k / 2).is_multiple_of(2) { 1.0 } else { -1.0 },
                    2 => (2.0 / PI) / kf * if odd { 1.0 } else { -1.0 },
                    3 if odd => (4.0 / PI) / kf,
                    _ => 0.0,
                }
            };
            let at = t * 3.0;
            let from = (at as usize).min(2);
            let f = at - from as f32;
            for (k, h) in out.iter_mut().enumerate().skip(1) {
                *h = sine(shape(from, k) * (1.0 - f) + shape(from + 1, k) * f);
            }
        }
        // A pulse that narrows from a square to a thin spike.
        1 => {
            let duty = 0.5 - 0.46 * t;
            for (k, h) in out.iter_mut().enumerate().skip(1) {
                let kf = k as f32;
                *h = Complex32::new(2.0 / (kf * PI) * (kf * PI * duty).sin(), 0.0);
            }
        }
        // A peak that climbs through the harmonics.
        2 => {
            let centre = 1.0 + 31.0 * t;
            let width = 1.0 + centre * 0.15;
            for (k, h) in out.iter_mut().enumerate().skip(1) {
                let kf = k as f32;
                *h = sine((0.15 + (-((kf - centre) / width).powi(2)).exp()) / kf.sqrt());
            }
        }
        // A, E, I, O, U: three formants each, as harmonics of a low voice.
        3 => {
            const VOWELS: [[f32; 3]; 5] = [[6.6, 9.9, 22.2], [4.8, 16.7, 22.5], [2.5, 20.8, 27.4], [5.2, 7.6, 21.9], [2.7, 7.9, 20.4]];
            let at = t * 4.0;
            let from = (at as usize).min(3);
            let f = at - from as f32;
            let centres: [f32; 3] = std::array::from_fn(|j| VOWELS[from][j] * (1.0 - f) + VOWELS[from + 1][j] * f);
            for (k, h) in out.iter_mut().enumerate().skip(1) {
                let kf = k as f32;
                let formants: f32 = centres.iter().zip([(1.0, 1.5), (0.6, 2.0), (0.35, 3.0)]).map(|(c, (gain, width))| gain * (-((kf - c) / width).powi(2)).exp()).sum();
                *h = sine(formants + if k == 1 { 0.5 } else { 0.0 });
            }
        }
        // A sine bent by another an octave up, further and further.
        4 => {
            let index = 6.0 * t;
            measured(&|x| (TAU * x + index * ((TAU * 2.0 * x).sin() + 0.5 * t * (TAU * 3.0 * x).sin())).sin(), &mut out);
        }
        // A saw restarted before it finishes: hard sync, one to four times per period.
        5 => {
            let ratio = 1.0 + 3.0 * t;
            measured(&|x| 2.0 * (x * ratio).fract() - 1.0, &mut out);
        }
        // Harmonics that come and go in a pattern.
        6 => {
            for (k, h) in out.iter_mut().enumerate().skip(1) {
                let kf = k as f32;
                *h = sine((kf * (0.37 + 2.9 * t)).sin().abs().powi(3) / kf.powf(0.7) * if k % 2 == 0 { -1.0 } else { 1.0 });
            }
        }
        // Drawbars pulled out one after another.
        _ => {
            const BARS: [usize; 9] = [1, 2, 3, 4, 6, 8, 10, 12, 16];
            for (j, &k) in BARS.iter().enumerate() {
                let pulled = (t * 9.0 - j as f32 + 1.0).clamp(0.0, 1.0);
                out[k] = sine(pulled / (1.0 + 0.3 * j as f32));
            }
        }
    }
    out
}

impl Table {
    fn new(table: usize) -> Self {
        let mut planner = FftPlanner::new();
        let inverse = planner.plan_fft_inverse(SIZE);
        let mut mips = vec![vec![0.0f32; FRAMES * ROW]; MIPS];
        let mut buf = vec![Complex32::default(); SIZE];
        for frame in 0..FRAMES {
            let spectrum = harmonics(table, frame as f32 / (FRAMES - 1) as f32, &mut planner);
            let mut scale = 1.0;
            for (m, mip) in mips.iter_mut().enumerate() {
                let keep = (HARMONICS >> m).clamp(1, HARMONICS - 1);
                buf.fill(Complex32::default());
                for k in 1..=keep {
                    buf[k] = spectrum[k];
                    buf[SIZE - k] = spectrum[k].conj();
                }
                inverse.process(&mut buf);
                if m == 0 {
                    // The same loudness from every waveform, without thin ones peaking too high.
                    let rms = (buf.iter().map(|c| c.re * c.re).sum::<f32>() / SIZE as f32).sqrt();
                    let peak = buf.iter().fold(0.0f32, |p, c| p.max(c.re.abs()));
                    scale = (0.5 / rms.max(1e-9)).min(1.6 / peak.max(1e-9));
                }
                let row = &mut mip[frame * ROW..(frame + 1) * ROW];
                for (s, c) in row.iter_mut().zip(&buf) {
                    *s = c.re * scale;
                }
                row[SIZE] = row[0];
            }
        }
        Self { mips }
    }
}

/// Every table, made once.
fn tables() -> &'static [Table] {
    static TABLES: OnceLock<Vec<Table>> = OnceLock::new();
    TABLES.get_or_init(|| (0..TABLE_NAMES.len()).map(Table::new).collect())
}

/// The waveforms of a table as `points` samples each, to draw them.
pub fn table_shapes(table: usize, points: usize) -> Vec<Vec<f32>> {
    let all = tables();
    // A copy with 128 harmonics: every detail a small drawing can show.
    let mip = &all[table.min(all.len() - 1)].mips[3];
    let points = points.clamp(2, SIZE);
    (0..FRAMES).map(|f| (0..points).map(|i| mip[f * ROW + i * SIZE / points]).collect()).collect()
}

/// The band-limited copy for an oscillator moving `step` periods per sample.
fn mip_for(step: f32) -> usize {
    let needed = (2.0 * step * HARMONICS as f32).max(1.0);
    (needed.log2().ceil() as usize).min(MIPS - 1)
}

/// Reads waveform position `pos` (0 … FRAMES − 1) of a table copy at `phase` (0–1).
#[inline]
fn read(mip: &[f32], pos: f32, phase: f32) -> f32 {
    let frame = (pos as usize).min(FRAMES - 2);
    let between = pos - frame as f32;
    let x = phase * SIZE as f32;
    let i = (x as usize).min(SIZE - 1);
    let f = x - i as f32;
    let a = frame * ROW + i;
    let b = a + ROW;
    let low = mip[a] + (mip[a + 1] - mip[a]) * f;
    let high = mip[b] + (mip[b + 1] - mip[b]) * f;
    low + (high - low) * between
}

#[derive(Debug, Clone, Copy, Default)]
struct Slot {
    source: usize,
    dest: usize,
    amount: f32,
}

struct Settings {
    tables: [usize; 2],
    /// 0–1 through the table.
    pos: [f32; 2],
    /// Semitones against the played note.
    semis: [f32; 2],
    levels: [f32; 2],
    unison: usize,
    spread: f32,
    sub: f32,
    noise: f32,
    filter: FilterKind,
    cutoff: f32,
    res: f32,
    /// Linear drive into the filter; 1 is off.
    drive: f32,
    amp: Adsr,
    env2: Adsr,
    /// 0 sine, 1 triangle, 2 saw, 3 square, 4 random steps.
    lfo_shapes: [usize; 2],
    /// Turns per sample.
    lfo_rates: [f32; 2],
    slots: [Slot; SLOTS],
    /// Fraction of the pitch distance left after each control step; 0 jumps.
    glide: f32,
    gain: f32,
}

impl Settings {
    fn new(sr: f32, p: &dyn Params) -> Self {
        let sec = |id: &str, d: f64| (p.or(id, d) / 1000.0) as f32;
        let pct = |id: &str, d: f64| (p.or(id, d) / 100.0) as f32;
        let choice = |id: &str, d: f64, count: usize| (p.or(id, d).round().max(0.0) as usize).min(count - 1);
        let glide = sec("glide", 0.0);
        let mut slots = [Slot::default(); SLOTS];
        for (slot, [source, dest, amount]) in slots.iter_mut().zip(SLOT_IDS) {
            *slot = Slot { source: choice(source, 0.0, SOURCES), dest: choice(dest, 0.0, DESTS), amount: pct(amount, 0.0).clamp(-1.0, 1.0) };
        }
        Self {
            tables: [choice("tableA", 0.0, TABLE_NAMES.len()), choice("tableB", 0.0, TABLE_NAMES.len())],
            pos: [pct("posA", 0.0).clamp(0.0, 1.0), pct("posB", 0.0).clamp(0.0, 1.0)],
            semis: [(p.or("semiA", 0.0) + p.or("fineA", 0.0) / 100.0) as f32, (p.or("semiB", 0.0) + p.or("fineB", 0.0) / 100.0) as f32],
            levels: [pct("levelA", 100.0).clamp(0.0, 1.0), pct("levelB", 0.0).clamp(0.0, 1.0)],
            unison: (p.or("unison", 1.0).round() as usize).clamp(1, MAX_UNISON),
            spread: pct("spread", 30.0),
            sub: pct("sub", 0.0),
            noise: pct("noise", 0.0),
            filter: FilterKind::from_index(p.or("filter", 0.0)),
            cutoff: p.or("cutoff", 4000.0) as f32,
            res: pct("res", 20.0),
            drive: db_gain(p.or("drive", 0.0)),
            amp: Adsr::new(sec("attack", 4.0), sec("decay", 300.0), pct("sustain", 80.0), sec("release", 280.0), sr),
            env2: Adsr::new(sec("e2Attack", 2.0), sec("e2Decay", 500.0), pct("e2Sustain", 0.0), sec("e2Release", 300.0), sr),
            lfo_shapes: [choice("lfo1Shape", 0.0, 5), choice("lfo2Shape", 0.0, 5)],
            lfo_rates: [p.or("lfo1Rate", 2.0) as f32 / sr, p.or("lfo2Rate", 0.5) as f32 / sr],
            slots,
            glide: if glide <= 0.0 { 0.0 } else { (-4.6 * CONTROL as f32 / (glide * sr)).exp() },
            gain: db_gain(p.or("volume", 0.0)) * VOICE_GAIN,
        }
    }
}

/// What the matrix moves, where it stood at the end of a control step: the
/// next step glides on from here, so nothing steps audibly.
#[derive(Debug, Clone, Copy, Default)]
struct Moved {
    /// Waveform positions, 0 … FRAMES − 1.
    pos: [f32; 2],
    levels: [f32; 2],
    /// Left and right gain: volume and pan.
    gains: [f32; 2],
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
    /// Oscillator A and B phase per unison voice.
    phases: [[f32; 2]; MAX_UNISON],
    sub: f32,
    amp: Env,
    env2: Env,
    /// [left, right][first, second stage]
    svf: [[Svf; 2]; 2],
    /// This note's random value, −1…1.
    random: f32,
    moved: Moved,
    /// Nothing to glide from yet.
    fresh: bool,
}

/// A free-running LFO shared by every voice.
#[derive(Debug, Clone, Copy, Default)]
struct Lfo {
    phase: f32,
    /// The value held by the random shape.
    held: f32,
}

impl Lfo {
    fn value(&self, shape: usize) -> f32 {
        let p = self.phase;
        match shape {
            1 => 1.0 - 4.0 * (p - 0.5).abs(),
            2 => 1.0 - 2.0 * p,
            3 => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            4 => self.held,
            _ => sin_turns(p),
        }
    }

    fn advance(&mut self, by: f32, rng: &mut Noise) {
        self.phase += by;
        if self.phase >= 1.0 {
            self.phase = self.phase.fract();
            self.held = rng.next();
        }
    }
}

pub struct Wavetable {
    sr: f32,
    s: Settings,
    tables: &'static [Table],
    voices: [Voice; VOICES],
    clock: u64,
    lfos: [Lfo; 2],
    pedal: bool,
    last_note: Option<f32>,
    rng: Noise,
}

impl Wavetable {
    pub fn new(sr: f32, params: &dyn Params) -> Self {
        Self {
            sr,
            s: Settings::new(sr, params),
            tables: tables(),
            voices: [Voice::default(); VOICES],
            clock: 0,
            lfos: [Lfo::default(); 2],
            pedal: false,
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
                let i = pick_voice(self.voices.iter().map(|v| (v.active, v.held, v.age)));
                let target = f32::from(pitch);
                let from = if self.s.glide > 0.0 { self.last_note.unwrap_or(target) } else { target };
                if !self.voices[i].active {
                    // Unison voices start out of step with each other.
                    let mut phases = [[0.0; 2]; MAX_UNISON];
                    for p in phases.iter_mut().flatten() {
                        *p = self.rng.next() * 0.5 + 0.5;
                    }
                    // The first voice starts its waveform at the beginning, for a firm attack.
                    phases[0] = [0.0; 2];
                    self.voices[i] = Voice { phases, fresh: true, ..Voice::default() };
                }
                let random = self.rng.next();
                let clock = self.clock;
                let v = &mut self.voices[i];
                *v = Voice { active: true, held: true, sustained: false, pitch, velocity, age: clock, note: from, target, random, ..*v };
                v.amp.start();
                v.env2.start();
                self.last_note = Some(target);
            }
            NoteEvent::Off { pitch } => {
                let pedal = self.pedal;
                for v in self.voices.iter_mut().filter(|v| v.active && v.held && v.pitch == pitch) {
                    v.held = false;
                    if pedal {
                        v.sustained = true;
                    } else {
                        v.amp.release();
                        v.env2.release();
                    }
                }
            }
            NoteEvent::Sustain(down) => {
                self.pedal = down;
                if !down {
                    for v in self.voices.iter_mut().filter(|v| v.sustained) {
                        v.sustained = false;
                        v.amp.release();
                        v.env2.release();
                    }
                }
            }
            NoteEvent::AllNotesOff => {
                self.pedal = false;
                for v in self.voices.iter_mut().filter(|v| v.active) {
                    v.held = false;
                    v.sustained = false;
                    v.amp.release();
                    v.env2.release();
                }
            }
            NoteEvent::AllSoundOff => {
                self.pedal = false;
                self.voices = [Voice::default(); VOICES];
            }
        }
    }

    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let mut done = 0;
        while done < left.len() {
            let len = (left.len() - done).min(CONTROL);
            let lfos = [self.lfos[0].value(self.s.lfo_shapes[0]), self.lfos[1].value(self.s.lfo_shapes[1])];
            for (lfo, rate) in self.lfos.iter_mut().zip(self.s.lfo_rates) {
                lfo.advance(rate * len as f32, &mut self.rng);
            }
            for v in self.voices.iter_mut().filter(|v| v.active) {
                render_voice(v, &self.s, self.tables, self.sr, lfos, &mut self.rng, &mut left[done..done + len], &mut right[done..done + len]);
            }
            done += len;
        }
    }

    pub fn adopt(&mut self, old: &mut Wavetable) {
        std::mem::swap(&mut self.voices, &mut old.voices);
        self.clock = old.clock;
        self.lfos = old.lfos;
        self.pedal = old.pedal;
        self.last_note = old.last_note;
    }

    pub fn is_silent(&self) -> bool {
        !self.voices.iter().any(|v| v.active)
    }
}

// One voice's block: everything it needs is passed in, so the caller can hold the rest.
#[allow(clippy::too_many_arguments)]
fn render_voice(v: &mut Voice, s: &Settings, tables: &[Table], sr: f32, lfos: [f32; 2], rng: &mut Noise, left: &mut [f32], right: &mut [f32]) {
    v.note = if s.glide > 0.0 { v.target + (v.note - v.target) * s.glide } else { v.target };

    // The matrix: each slot adds its source, scaled, to its destination.
    let sources = [0.0, v.env2.level, lfos[0], lfos[1], v.velocity, ((v.note - 60.0) / 48.0).clamp(-1.0, 1.0), v.random];
    let mut m = [0.0f32; DESTS];
    for slot in s.slots.iter().filter(|slot| slot.source != 0) {
        m[slot.dest] += slot.amount * sources[slot.source];
    }
    let top = (FRAMES - 1) as f32;
    let (pan_l, pan_r) = pan_gains(m[PAN]);
    let volume = (1.0 + m[VOLUME]).clamp(0.0, 2.0);
    let now = Moved {
        pos: [(s.pos[0] + m[POS_A]).clamp(0.0, 1.0) * top, (s.pos[1] + m[POS_B]).clamp(0.0, 1.0) * top],
        levels: [(s.levels[0] + m[LEVEL_A]).clamp(0.0, 1.0), (s.levels[1] + m[LEVEL_B]).clamp(0.0, 1.0)],
        gains: [pan_l * volume, pan_r * volume],
    };
    if v.fresh {
        v.moved = now;
        v.fresh = false;
    }
    let from = v.moved;
    v.moved = now;

    let note = v.note + m[PITCH] * PITCH_RANGE;
    let base = [midi_hz(note + s.semis[0]) / sr, midi_hz(note + s.semis[1] + m[PITCH_B] * PITCH_RANGE) / sr];
    let cutoff = (s.cutoff * (m[CUTOFF] * CUTOFF_RANGE).exp2()).clamp(20.0, 20_000.0);
    let res = (s.res + m[RES]).clamp(0.0, 1.0);
    let coef = SvfCoef::new(cutoff, res, sr);
    // In a 24 dB filter only the first stage resonates, so the peak stays single.
    let coef2 = SvfCoef::new(cutoff, 0.0, sr);

    let unison = s.unison;
    let mut steps = [[0.0_f32; 2]; MAX_UNISON];
    let mut pans = [(1.0_f32, 1.0_f32); MAX_UNISON];
    for u in 0..unison {
        let offset = if unison > 1 { u as f32 / (unison - 1) as f32 * 2.0 - 1.0 } else { 0.0 };
        let ratio = (offset * s.spread * MAX_SPREAD / 12.0).exp2();
        steps[u] = [(base[0] * ratio).min(0.45), (base[1] * ratio).min(0.45)];
        if unison > 1 {
            pans[u] = pan_gains(offset * s.spread);
        }
    }
    let norm = 1.0 / (unison as f32).sqrt();
    // The band limit follows the highest unison voice.
    let widest = (s.spread * MAX_SPREAD / 12.0).exp2();
    let mips = [&tables[s.tables[0]].mips[mip_for(base[0] * widest)], &tables[s.tables[1]].mips[mip_for(base[1] * widest)]];
    let plays = [from.levels[0] > 0.0 || now.levels[0] > 0.0, from.levels[1] > 0.0 || now.levels[1] > 0.0];
    let sub_step = (midi_hz(note - 12.0) / sr).min(0.45);

    let n = left.len() as f32;
    for (i, (l_out, r_out)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
        // Every moved value glides across the block.
        let t = (i + 1) as f32 / n;
        let mix = |a: f32, b: f32| a + (b - a) * t;
        let pos = [mix(from.pos[0], now.pos[0]), mix(from.pos[1], now.pos[1])];
        let levels = [mix(from.levels[0], now.levels[0]), mix(from.levels[1], now.levels[1])];

        let (mut l, mut r) = (0.0, 0.0);
        for u in 0..unison {
            let ph = &mut v.phases[u];
            let mut x = 0.0;
            for o in 0..2 {
                if plays[o] {
                    x += read(mips[o], pos[o], ph[o]) * levels[o];
                }
                ph[o] += steps[u][o];
                if ph[o] >= 1.0 {
                    ph[o] -= 1.0;
                }
            }
            l += x * pans[u].0;
            r += x * pans[u].1;
        }
        l *= norm;
        r *= norm;
        if s.sub > 0.0 {
            let sub = sin_turns(v.sub) * s.sub;
            l += sub;
            r += sub;
        }
        v.sub += sub_step;
        if v.sub >= 1.0 {
            v.sub -= 1.0;
        }
        if s.noise > 0.0 {
            l += rng.next() * s.noise * 0.5;
            r += rng.next() * s.noise * 0.5;
        }
        if s.drive > 1.001 {
            l = (l * s.drive).tanh();
            r = (r * s.drive).tanh();
        }
        let yl = filter(&mut v.svf[0], &coef, &coef2, s.filter, l);
        let yr = filter(&mut v.svf[1], &coef, &coef2, s.filter, r);
        v.env2.next(&s.env2);
        let amp = v.amp.next(&s.amp) * v.velocity * s.gain;
        *l_out += yl * amp * mix(from.gains[0], now.gains[0]);
        *r_out += yr * amp * mix(from.gains[1], now.gains[1]);
    }
    if v.amp.is_idle() {
        v.active = false;
        v.held = false;
        v.sustained = false;
    }
}

#[cfg(test)]
mod tests {
    use super::{FRAMES, TABLE_NAMES, table_shapes};
    use crate::test_util::*;
    use crate::{Instrument, NoteEvent};

    /// A plain patch: oscillator A alone, the filter wide open, nothing in the matrix.
    const PLAIN: [(&str, f64); 6] = [("cutoff", 18000.0), ("res", 0.0), ("m1src", 0.0), ("m2src", 0.0), ("release", 30.0), ("levelB", 0.0)];

    fn wavetable(params: &[(&str, f64)]) -> Instrument {
        let owned: Vec<(String, f64)> = PLAIN.iter().chain(params).map(|(k, v)| (k.to_string(), *v)).collect();
        // Later entries override the plain patch.
        let lookup = move |id: &str| owned.iter().rev().find(|(k, _)| k == id).map(|(_, v)| *v);
        Instrument::build("wavetable", SR, &lookup, &[]).unwrap()
    }

    /// The level of `hz` in a signal (a single-bin DFT).
    fn level_at(x: &[f32], hz: f32) -> f32 {
        let w = std::f32::consts::TAU * hz / SR as f32;
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (i, s) in x.iter().enumerate() {
            re += s * (w * i as f32).cos();
            im += s * (w * i as f32).sin();
        }
        2.0 * (re * re + im * im).sqrt() / x.len() as f32
    }

    #[test]
    fn plays_the_right_pitch() {
        // Basic shapes at position 0 is a sine: A4 must measure 440 Hz.
        let mut s = wavetable(&[]);
        s.handle(NoteEvent::On { pitch: 69, velocity: 1.0 });
        let (l, _) = render(&mut s, SR as usize / 2);
        let f = frequency(&l[4800..], SR);
        assert!((f - 440.0).abs() < 1.0, "{f}");
        s.handle(NoteEvent::Off { pitch: 69 });
        render(&mut s, 24_000);
        assert!(s.is_silent());
    }

    #[test]
    fn the_position_changes_the_waveform() {
        // Basic shapes: a sine at 0 % has no third harmonic, the square at 100 % has a strong one.
        let third = |pos: f64| {
            let mut s = wavetable(&[("posA", pos)]);
            s.handle(NoteEvent::On { pitch: 57, velocity: 1.0 });
            let (l, _) = render(&mut s, 24_000);
            level_at(&l[9600..], 660.0) / level_at(&l[9600..], 220.0)
        };
        assert!(third(0.0) < 0.01, "{}", third(0.0));
        assert!((third(100.0) - 1.0 / 3.0).abs() < 0.03, "{}", third(100.0));
    }

    #[test]
    fn high_notes_do_not_alias() {
        // A saw at 3520 Hz: harmonics up to 21 kHz, and nothing folded back between them.
        let mut s = wavetable(&[("posA", 66.7)]);
        s.handle(NoteEvent::On { pitch: 105, velocity: 1.0 });
        let (l, _) = render(&mut s, 48_000);
        let x = &l[24_000..];
        let fundamental = level_at(x, 3520.0);
        assert!(fundamental > 0.05, "{fundamental}");
        // 48 000 − 8 × 3520 = 19 840 Hz is where the eighth harmonic of a naive saw would fold to.
        for hz in [19_840.0, 16_320.0, 5000.0, 1000.0] {
            assert!(level_at(x, hz) < fundamental * 0.004, "{hz}: {}", level_at(x, hz));
        }
    }

    #[test]
    fn the_matrix_moves_its_destinations() {
        // Envelope 2 opening the filter: with the amount, the start of a note is brighter than its end.
        let brightness = |amount: f64| {
            let mut s = wavetable(&[("posA", 66.7), ("cutoff", 300.0), ("m1src", 1.0), ("m1dst", 4.0), ("m1amt", amount), ("e2Decay", 3000.0)]);
            s.handle(NoteEvent::On { pitch: 45, velocity: 1.0 });
            let (l, _) = render(&mut s, 48_000);
            // Whole periods of the note, past the attack.
            (level_at(&l[4800..9600], 1650.0), level_at(&l[43_200..], 1650.0))
        };
        let (start, end) = brightness(60.0);
        assert!(start > end * 8.0, "{start} {end}");
        let (closed, _) = brightness(0.0);
        assert!(start > closed * 20.0, "{start} {closed}");

        // Velocity to volume: a soft note is quieter than without the slot.
        let loudness = |amount: f64| {
            let mut s = wavetable(&[("m1src", 4.0), ("m1dst", 8.0), ("m1amt", amount)]);
            s.handle(NoteEvent::On { pitch: 60, velocity: 0.5 });
            let (l, _) = render(&mut s, 9600);
            peak(&l[4800..])
        };
        assert!((loudness(-100.0) / loudness(0.0) - 0.5).abs() < 0.05, "{} {}", loudness(-100.0), loudness(0.0));

        // An LFO on the pitch: the frequency wanders around the note.
        let mut s = wavetable(&[("m1src", 2.0), ("m1dst", 2.0), ("m1amt", 50.0), ("lfo1Rate", 1.0), ("lfo1Shape", 3.0)]);
        s.handle(NoteEvent::On { pitch: 69, velocity: 1.0 });
        let (l, _) = render(&mut s, 48_000);
        // A square LFO at 1 Hz: half a second six semitones up, then six down.
        let (up, down) = (frequency(&l[4800..19_200], SR), frequency(&l[28_800..43_200], SR));
        assert!((up - 440.0 * 2f32.powf(0.5)).abs() < 3.0 && (down - 440.0 / 2f32.powf(0.5)).abs() < 3.0, "{up} {down}");
    }

    #[test]
    fn a_new_instrument_keeps_the_notes_of_the_old_one() {
        let mut old = wavetable(&[]);
        old.handle(NoteEvent::On { pitch: 64, velocity: 0.8 });
        render(&mut old, 2400);
        let mut new = wavetable(&[("tableA", 3.0), ("posA", 40.0)]);
        new.adopt(&mut old);
        assert!(!new.is_silent() && old.is_silent());
        let (l, _) = render(&mut new, 2400);
        assert!(peak(&l) > 0.01);
        new.handle(NoteEvent::Off { pitch: 64 });
        render(&mut new, 48_000);
        assert!(new.is_silent(), "the adopted note still releases");
    }

    #[test]
    fn every_table_has_sound_in_every_waveform() {
        for (table, name) in TABLE_NAMES.iter().enumerate() {
            let shapes = table_shapes(table, 512);
            assert_eq!(shapes.len(), FRAMES);
            for (frame, shape) in shapes.iter().enumerate() {
                let top = shape.iter().fold(0.0f32, |p, s| p.max(s.abs()));
                assert!(shape.iter().all(|s| s.is_finite()) && top > 0.2 && top < 2.0, "{name} waveform {frame}: {top}");
            }
        }
        // More notes than voices, with unison and both oscillators: finite and bounded.
        let mut s = wavetable(&[("unison", 5.0), ("levelB", 100.0), ("tableB", 5.0), ("posB", 50.0), ("sub", 50.0), ("noise", 20.0)]);
        for p in 40..70 {
            s.handle(NoteEvent::On { pitch: p, velocity: 0.5 });
        }
        let (l, _) = render(&mut s, 2400);
        assert!(l.iter().all(|x| x.is_finite()) && peak(&l) < 6.0);
    }
}
