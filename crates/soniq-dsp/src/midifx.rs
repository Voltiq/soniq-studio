//! MIDI effects: they change notes before the instrument hears them. Each
//! works a block at a time on events with sample offsets, keeps its state in
//! fixed-size arrays (the audio thread never allocates), and remembers what
//! every incoming note became, so its note-off ends exactly that even after a
//! parameter changed.

use crate::common::Noise;
use crate::{NoteEvent, Params};

/// A note event `at` samples into the block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockEvent {
    pub at: u32,
    pub event: NoteEvent,
}

/// Where a block sits in time, for effects that follow the tempo.
#[derive(Debug, Clone, Copy)]
pub struct FxContext {
    /// The timeline is moving; `pos` is its sample at the block start.
    pub playing: bool,
    pub pos: u64,
    /// Where `pos` is in the song, in beats (the tempo track included).
    pub beat: f64,
    /// The tempo there.
    pub samples_per_beat: f64,
    pub sample_rate: f32,
}

/// Adds an event without growing the buffer; a full buffer drops it.
pub fn push_event(out: &mut Vec<BlockEvent>, at: u32, event: NoteEvent) {
    if out.len() < out.capacity() {
        out.push(BlockEvent { at, event });
    }
}

/// Stable, in-place sort by time. Blocks hold few events, so insertion sort.
pub fn sort_events(events: &mut [BlockEvent]) {
    for i in 1..events.len() {
        let mut j = i;
        while j > 0 && events[j - 1].at > events[j].at {
            events.swap(j - 1, j);
            j -= 1;
        }
    }
}

// ---------- Keys and scales (mirrors src/state/music.ts) ----------

const SCALES: [(&str, &[u8]); 12] = [
    ("major", &[0, 2, 4, 5, 7, 9, 11]),
    ("minor", &[0, 2, 3, 5, 7, 8, 10]),
    ("dorian", &[0, 2, 3, 5, 7, 9, 10]),
    ("phrygian", &[0, 1, 3, 5, 7, 8, 10]),
    ("lydian", &[0, 2, 4, 6, 7, 9, 11]),
    ("mixolydian", &[0, 2, 4, 5, 7, 9, 10]),
    ("harmonic minor", &[0, 2, 3, 5, 7, 8, 11]),
    ("melodic minor", &[0, 2, 3, 5, 7, 9, 11]),
    ("major pentatonic", &[0, 2, 4, 7, 9]),
    ("minor pentatonic", &[0, 3, 5, 7, 10]),
    ("blues", &[0, 3, 5, 6, 7, 10]),
    ("chromatic", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
];

/// The pitch classes of a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    classes: [bool; 12],
}

impl Key {
    pub fn new(root: u8, scale: &str) -> Self {
        let steps = SCALES.iter().find(|(name, _)| *name == scale).map_or(SCALES[0].1, |(_, s)| s);
        let mut classes = [false; 12];
        for s in steps {
            classes[usize::from((root + s) % 12)] = true;
        }
        Self { classes }
    }

    /// "A minor", "E♭ dorian", "Bb major", "F# minor pentatonic".
    pub fn parse(name: &str) -> Self {
        let name = name.trim();
        let mut chars = name.chars();
        let base = match chars.next().map(|c| c.to_ascii_uppercase()) {
            Some('C') => 0,
            Some('D') => 2,
            Some('E') => 4,
            Some('F') => 5,
            Some('G') => 7,
            Some('A') => 9,
            Some('B') => 11,
            _ => return Self::new(0, "major"),
        };
        let rest = chars.as_str();
        let (shift, rest) = match rest.chars().next() {
            Some('#' | '♯') => (1, &rest[rest.chars().next().map_or(0, char::len_utf8)..]),
            Some('b' | '♭') if !rest.trim_start_matches(['b', '♭']).starts_with('l') => (11, &rest[rest.chars().next().map_or(0, char::len_utf8)..]),
            _ => (0, rest),
        };
        Self::new((base + shift) % 12, &rest.trim().to_lowercase())
    }

    pub fn contains(&self, pitch: u8) -> bool {
        self.classes[usize::from(pitch % 12)]
    }

    /// The nearest pitch in the key; ties go up.
    pub fn snap(&self, pitch: u8) -> u8 {
        if self.contains(pitch) {
            return pitch;
        }
        for d in 1..12u8 {
            if pitch.checked_add(d).is_some_and(|p| p <= 127 && self.contains(p)) {
                return pitch + d;
            }
            if pitch.checked_sub(d).is_some_and(|p| self.contains(p)) {
                return pitch - d;
            }
        }
        pitch
    }
}

const ROOT_COUNT: f64 = 12.0;
const NONE: u8 = u8::MAX;

fn option(p: &dyn Params, id: &str, default: f64) -> usize {
    p.or(id, default).round().max(0.0) as usize
}

// ---------- The effects ----------

// Each effect is built once per graph and stays in place, so variant sizes do not matter.
#[allow(clippy::large_enum_variant)]
pub enum MidiFx {
    Arp(Arp),
    Chord(Chord),
    Scale(ScaleFx),
    Length(NoteLength),
    Random(RandomFx),
    Velocity(VelocityFx),
}

/// Device types that change notes.
pub const MIDI_FX_TYPES: [&str; 6] = ["arp", "chord", "scale", "notelength", "random", "velocity"];

impl MidiFx {
    /// The effect for a device type, or `None` if the type is not a MIDI effect.
    /// `song` is the project's key, for effects set to follow it.
    pub fn build(kind: &str, p: &dyn Params, song: Key) -> Option<Self> {
        Some(match kind {
            "arp" => Self::Arp(Arp::new(p)),
            "chord" => Self::Chord(Chord::new(p)),
            "scale" => Self::Scale(ScaleFx::new(p, song)),
            "notelength" => Self::Length(NoteLength::new(p)),
            "random" => Self::Random(RandomFx::new(p, song)),
            "velocity" => Self::Velocity(VelocityFx::new(p)),
            _ => return None,
        })
    }

    /// Reads `input` (sorted by time) and writes this block's notes to `out`,
    /// sorted by time. `frames` is the block length.
    pub fn process(&mut self, input: &[BlockEvent], out: &mut Vec<BlockEvent>, frames: u32, ctx: &FxContext) {
        match self {
            Self::Arp(fx) => fx.process(input, out, frames, ctx),
            Self::Chord(fx) => fx.process(input, out),
            Self::Scale(fx) => fx.process(input, out),
            Self::Length(fx) => fx.process(input, out, frames, ctx),
            Self::Random(fx) => fx.process(input, out),
            Self::Velocity(fx) => fx.process(input, out),
        }
    }

    /// Takes over the notes the effect it replaces was handling.
    pub fn adopt(&mut self, old: &mut MidiFx) {
        match (self, old) {
            (Self::Arp(a), Self::Arp(b)) => std::mem::swap(&mut a.s, &mut b.s),
            (Self::Chord(a), Self::Chord(b)) => std::mem::swap(&mut a.map, &mut b.map),
            (Self::Scale(a), Self::Scale(b)) => std::mem::swap(&mut a.map, &mut b.map),
            (Self::Length(a), Self::Length(b)) => std::mem::swap(&mut a.s, &mut b.s),
            (Self::Random(a), Self::Random(b)) => std::mem::swap(&mut a.s, &mut b.s),
            _ => {}
        }
    }
}

/// Note-ons go through `map`, note-offs end what their note-on became.
fn mapped(input: &[BlockEvent], out: &mut Vec<BlockEvent>, map: &mut [u8; 128], mut on: impl FnMut(u8, f32) -> (u8, f32)) {
    for e in input {
        match e.event {
            NoteEvent::On { pitch, velocity } => {
                let (p, v) = on(pitch, velocity);
                map[usize::from(pitch)] = p;
                push_event(out, e.at, NoteEvent::On { pitch: p, velocity: v });
            }
            NoteEvent::Off { pitch } => {
                let p = std::mem::replace(&mut map[usize::from(pitch)], NONE);
                push_event(out, e.at, NoteEvent::Off { pitch: if p == NONE { pitch } else { p } });
            }
            other => push_event(out, e.at, other),
        }
    }
}

/// Stacks up to three intervals on every note (0 is off).
pub struct Chord {
    shifts: [i8; 3],
    map: [[u8; 4]; 128],
}

impl Chord {
    fn new(p: &dyn Params) -> Self {
        let shift = |id: &str, d: f64| p.or(id, d).round().clamp(-36.0, 36.0) as i8;
        Self { shifts: [shift("shift1", 4.0), shift("shift2", 7.0), shift("shift3", 0.0)], map: [[NONE; 4]; 128] }
    }

    fn process(&mut self, input: &[BlockEvent], out: &mut Vec<BlockEvent>) {
        for e in input {
            match e.event {
                NoteEvent::On { pitch, velocity } => {
                    let mut notes = [NONE; 4];
                    notes[0] = pitch;
                    let mut n = 1;
                    for s in self.shifts.iter().filter(|s| **s != 0) {
                        let p = i16::from(pitch) + i16::from(*s);
                        if (0..=127).contains(&p) && !notes[..n].contains(&(p as u8)) {
                            notes[n] = p as u8;
                            n += 1;
                        }
                    }
                    self.map[usize::from(pitch)] = notes;
                    for p in &notes[..n] {
                        push_event(out, e.at, NoteEvent::On { pitch: *p, velocity });
                    }
                }
                NoteEvent::Off { pitch } => {
                    let notes = std::mem::replace(&mut self.map[usize::from(pitch)], [NONE; 4]);
                    if notes[0] == NONE {
                        push_event(out, e.at, e.event);
                    }
                    for p in notes.iter().filter(|p| **p != NONE) {
                        push_event(out, e.at, NoteEvent::Off { pitch: *p });
                    }
                }
                other => push_event(out, e.at, other),
            }
        }
    }
}

/// Moves every note to the nearest note of a key.
pub struct ScaleFx {
    key: Key,
    map: [u8; 128],
}

impl ScaleFx {
    fn new(p: &dyn Params, song: Key) -> Self {
        let scale = option(p, "scale", 0.0);
        let key = if scale == 0 {
            song
        } else {
            let root = (p.or("root", 0.0).round().rem_euclid(ROOT_COUNT)) as u8;
            Key::new(root, SCALES.get(scale - 1).map_or("major", |s| s.0))
        };
        Self { key, map: [NONE; 128] }
    }

    fn process(&mut self, input: &[BlockEvent], out: &mut Vec<BlockEvent>) {
        let key = self.key;
        mapped(input, out, &mut self.map, |p, v| (key.snap(p), v));
    }
}

/// Lets every note last a set time, whatever the key does.
pub struct NoteLength {
    /// Beats (synced) or seconds.
    length: f64,
    synced: bool,
    s: LengthState,
}

#[derive(Clone, Copy)]
struct LengthState {
    /// When each pitch ends, on the effect's own sample clock.
    ends: [u64; 128],
    clock: u64,
}

const SYNC_BEATS: [f64; 6] = [0.125, 0.25, 0.5, 1.0, 2.0, 4.0];

impl NoteLength {
    fn new(p: &dyn Params) -> Self {
        let sync = option(p, "sync", 0.0);
        let synced = sync > 0;
        let length = if synced { SYNC_BEATS[(sync - 1).min(SYNC_BEATS.len() - 1)] } else { p.or("length", 250.0) / 1000.0 };
        Self { length, synced, s: LengthState { ends: [u64::MAX; 128], clock: 0 } }
    }

    fn process(&mut self, input: &[BlockEvent], out: &mut Vec<BlockEvent>, frames: u32, ctx: &FxContext) {
        let samples = if self.synced { self.length * ctx.samples_per_beat } else { self.length * f64::from(ctx.sample_rate) };
        let samples = samples.max(1.0) as u64;
        let s = &mut self.s;
        for e in input {
            match e.event {
                NoteEvent::On { pitch, .. } => {
                    s.ends[usize::from(pitch)] = s.clock + u64::from(e.at) + samples;
                    push_event(out, e.at, e.event);
                }
                // The key going up changes nothing; the timer ends the note.
                NoteEvent::Off { pitch } if s.ends[usize::from(pitch)] != u64::MAX => {}
                NoteEvent::AllNotesOff | NoteEvent::AllSoundOff => {
                    for (pitch, end) in s.ends.iter_mut().enumerate() {
                        if *end != u64::MAX {
                            *end = u64::MAX;
                            push_event(out, e.at, NoteEvent::Off { pitch: pitch as u8 });
                        }
                    }
                    push_event(out, e.at, e.event);
                }
                other => push_event(out, e.at, other),
            }
        }
        let block_end = s.clock + u64::from(frames);
        for (pitch, end) in s.ends.iter_mut().enumerate() {
            if *end < block_end {
                let at = end.saturating_sub(s.clock) as u32;
                *end = u64::MAX;
                push_event(out, at, NoteEvent::Off { pitch: pitch as u8 });
            }
        }
        s.clock = block_end;
        sort_events(out);
    }
}

/// Shifts notes by a random amount, some of the time.
pub struct RandomFx {
    chance: f32,
    range: u8,
    /// 0 both ways, 1 up, 2 down.
    direction: u8,
    key: Option<Key>,
    s: RandomState,
}

#[derive(Clone, Copy)]
struct RandomState {
    map: [u8; 128],
    rng: Noise,
}

impl RandomFx {
    fn new(p: &dyn Params, song: Key) -> Self {
        Self {
            chance: (p.or("chance", 50.0) / 100.0) as f32,
            range: p.or("range", 12.0).round().clamp(1.0, 48.0) as u8,
            direction: option(p, "direction", 0.0).min(2) as u8,
            key: (option(p, "inKey", 0.0) == 1).then_some(song),
            s: RandomState { map: [NONE; 128], rng: Noise(0x2545_f491) },
        }
    }

    fn process(&mut self, input: &[BlockEvent], out: &mut Vec<BlockEvent>) {
        let (chance, range, direction, key) = (self.chance, i16::from(self.range), self.direction, self.key);
        let rng = &mut self.s.rng;
        mapped(input, out, &mut self.s.map, |pitch, v| {
            let roll = (rng.next() + 1.0) * 0.5;
            if roll >= chance {
                return (pitch, v);
            }
            let amount = (((rng.next() + 1.0) * 0.5 * range as f32).floor() as i16 + 1).min(range);
            let signed = match direction {
                1 => amount,
                2 => -amount,
                _ if rng.next() < 0.0 => -amount,
                _ => amount,
            };
            let p = (i16::from(pitch) + signed).clamp(0, 127) as u8;
            (key.map_or(p, |k| k.snap(p)), v)
        });
    }
}

/// Reshapes velocities: a curve, an output range and some randomness.
pub struct VelocityFx {
    /// Exponent of the curve; below 1 lifts soft notes.
    curve: f32,
    low: f32,
    high: f32,
    random: f32,
    rng: Noise,
}

impl VelocityFx {
    fn new(p: &dyn Params) -> Self {
        let drive = (p.or("drive", 0.0) / 100.0).clamp(-1.0, 1.0) as f32;
        Self {
            curve: (-2.0 * drive).exp2(),
            low: (p.or("outLow", 1.0) / 127.0) as f32,
            high: (p.or("outHigh", 127.0) / 127.0) as f32,
            random: (p.or("random", 0.0) / 100.0) as f32,
            rng: Noise(0x68e3_1da4),
        }
    }

    fn process(&mut self, input: &[BlockEvent], out: &mut Vec<BlockEvent>) {
        for e in input {
            let event = match e.event {
                NoteEvent::On { pitch, velocity } => {
                    let shaped = velocity.clamp(0.0, 1.0).powf(self.curve);
                    let v = self.low + shaped * (self.high - self.low) + self.rng.next() * self.random * 0.5;
                    NoteEvent::On { pitch, velocity: v.clamp(1.0 / 127.0, 1.0) }
                }
                other => other,
            };
            push_event(out, e.at, event);
        }
    }
}

// ---------- Arpeggiator ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    Up,
    Down,
    UpDown,
    DownUp,
    Played,
    Random,
    Chord,
}

const RATES: [f64; 6] = [1.0, 0.5, 1.0 / 3.0, 0.25, 1.0 / 6.0, 0.125];
const HELD: usize = 32;
const SOUNDING: usize = 32;

/// Plays the held notes one after another (or together, in chord style), in
/// time with the song while it plays and from the first key while it does not.
pub struct Arp {
    style: Style,
    /// Beats per step.
    rate: f64,
    gate: f64,
    octaves: u8,
    s: ArpState,
}

#[derive(Clone, Copy)]
struct ArpState {
    held: [(u8, f32); HELD],
    count: usize,
    step: u32,
    /// Notes the arp started, and when (on its own clock) each one ends.
    sounding: [(u8, u64); SOUNDING],
    sounding_count: usize,
    clock: u64,
    /// While the song is stopped: the next step, on the arp's clock.
    next_tick: Option<u64>,
    rng: Noise,
}

impl Arp {
    fn new(p: &dyn Params) -> Self {
        let style = match option(p, "style", 0.0) {
            1 => Style::Down,
            2 => Style::UpDown,
            3 => Style::DownUp,
            4 => Style::Played,
            5 => Style::Random,
            6 => Style::Chord,
            _ => Style::Up,
        };
        Self {
            style,
            rate: RATES[option(p, "rate", 3.0).min(RATES.len() - 1)],
            gate: (p.or("gate", 80.0) / 100.0).clamp(0.05, 2.0),
            octaves: p.or("octaves", 1.0).round().clamp(1.0, 4.0) as u8,
            s: ArpState {
                held: [(0, 0.0); HELD],
                count: 0,
                step: 0,
                sounding: [(0, 0); SOUNDING],
                sounding_count: 0,
                clock: 0,
                next_tick: None,
                rng: Noise(0x1b87_3593),
            },
        }
    }

    fn process(&mut self, input: &[BlockEvent], out: &mut Vec<BlockEvent>, frames: u32, ctx: &FxContext) {
        let step_len = (self.rate * ctx.samples_per_beat).max(1.0);
        let mut i = 0;
        // Song time: steps sit on the beat grid (in beats, so tempo changes keep them on it).
        let mut grid = (ctx.beat / self.rate.max(1e-6) - 1e-9).ceil();
        if !ctx.playing {
            // The song stopped: carry on from now if keys are still held.
            if self.s.count > 0 && self.s.next_tick.is_none() {
                self.s.next_tick = Some(self.s.clock);
            }
        } else {
            self.s.next_tick = None;
        }
        loop {
            let tick = if ctx.playing {
                let t = (grid * self.rate - ctx.beat) * ctx.samples_per_beat;
                (t < f64::from(frames)).then(|| t.max(0.0).round() as u32)
            } else {
                self.s.next_tick.filter(|t| *t < self.s.clock + u64::from(frames)).map(|t| t.saturating_sub(self.s.clock) as u32)
            };
            let off = self.next_off(frames);
            let next_in = input.get(i).map(|e| e.at);
            // Ends first, then the keys, then the next step.
            let candidates = [off.map(|(at, _)| at), next_in, tick];
            let Some(at) = candidates.iter().flatten().min().copied() else { break };
            if off.is_some_and(|(o, _)| o == at) {
                let (_, k) = off.unwrap_or_default();
                let (pitch, _) = self.s.sounding[k];
                self.remove_sounding(k);
                push_event(out, at, NoteEvent::Off { pitch });
            } else if next_in == Some(at) {
                self.key(input[i], out, ctx.playing);
                i += 1;
            } else {
                self.tick(at, step_len, out);
                if ctx.playing {
                    grid += 1.0;
                } else if let Some(t) = self.s.next_tick.as_mut() {
                    *t += step_len.round() as u64;
                }
            }
        }
        self.s.clock += u64::from(frames);
    }

    /// The earliest ending note within the block: (offset, index).
    fn next_off(&self, frames: u32) -> Option<(u32, usize)> {
        let end = self.s.clock + u64::from(frames);
        self.s.sounding[..self.s.sounding_count]
            .iter()
            .enumerate()
            .filter(|(_, (_, t))| *t < end)
            .min_by_key(|(_, (_, t))| *t)
            .map(|(k, (_, t))| (t.saturating_sub(self.s.clock) as u32, k))
    }

    fn remove_sounding(&mut self, k: usize) {
        let s = &mut self.s;
        s.sounding[k] = s.sounding[s.sounding_count - 1];
        s.sounding_count -= 1;
    }

    fn key(&mut self, e: BlockEvent, out: &mut Vec<BlockEvent>, playing: bool) {
        let s = &mut self.s;
        match e.event {
            NoteEvent::On { pitch, velocity } => {
                if s.held[..s.count].iter().any(|(p, _)| *p == pitch) {
                    return;
                }
                if s.count == 0 {
                    s.step = 0;
                    // Stopped: the first key starts the pattern at once.
                    if !playing {
                        s.next_tick = Some(s.clock + u64::from(e.at));
                    }
                }
                if s.count < HELD {
                    s.held[s.count] = (pitch, velocity);
                    s.count += 1;
                }
            }
            NoteEvent::Off { pitch } => {
                if let Some(k) = s.held[..s.count].iter().position(|(p, _)| *p == pitch) {
                    s.held.copy_within(k + 1..s.count, k);
                    s.count -= 1;
                }
                if s.count == 0 {
                    s.next_tick = None;
                }
            }
            NoteEvent::AllNotesOff | NoteEvent::AllSoundOff => {
                s.count = 0;
                s.next_tick = None;
                for (pitch, _) in s.sounding[..s.sounding_count].iter() {
                    push_event(out, e.at, NoteEvent::Off { pitch: *pitch });
                }
                s.sounding_count = 0;
                push_event(out, e.at, e.event);
            }
            NoteEvent::Sustain(_) => push_event(out, e.at, e.event),
        }
    }

    fn tick(&mut self, at: u32, step_len: f64, out: &mut Vec<BlockEvent>) {
        let n = self.s.count;
        if n == 0 {
            return;
        }
        // The notes in play order: sorted (or as played), across the octaves.
        let mut order = [(0u8, 0f32); HELD];
        order[..n].copy_from_slice(&self.s.held[..n]);
        if self.style != Style::Played {
            order[..n].sort_unstable_by_key(|(p, _)| *p);
        }
        let octaves = usize::from(self.octaves);
        let total = n * octaves;
        let nth = |k: usize| {
            let (p, v) = order[k % n];
            ((usize::from(p) + 12 * (k / n)).min(127) as u8, v)
        };
        let step = self.s.step as usize;
        let ends = self.s.clock + u64::from(at) + (self.gate * step_len).max(1.0) as u64;
        let mut play = |arp: &mut Self, pitch: u8, velocity: f32| {
            // A note still ringing at this pitch ends before it starts again.
            if let Some(k) = arp.s.sounding[..arp.s.sounding_count].iter().position(|(p, _)| *p == pitch) {
                arp.remove_sounding(k);
                push_event(out, at, NoteEvent::Off { pitch });
            }
            if arp.s.sounding_count < SOUNDING {
                arp.s.sounding[arp.s.sounding_count] = (pitch, ends);
                arp.s.sounding_count += 1;
                push_event(out, at, NoteEvent::On { pitch, velocity });
            }
        };
        match self.style {
            Style::Chord => {
                let octave = step % octaves;
                for k in 0..n {
                    let (p, v) = nth(k + octave * n);
                    play(self, p, v);
                }
            }
            style => {
                let k = match style {
                    Style::Up | Style::Played => step % total,
                    Style::Down => total - 1 - step % total,
                    Style::UpDown | Style::DownUp => {
                        let period = if total > 1 { 2 * total - 2 } else { 1 };
                        let i = step % period;
                        let up = if i < total { i } else { period - i };
                        if style == Style::UpDown { up } else { total - 1 - up }
                    }
                    _ => (((self.s.rng.next() + 1.0) * 0.5 * total as f32) as usize).min(total - 1),
                };
                let (p, v) = nth(k);
                play(self, p, v);
            }
        }
        self.s.step = self.s.step.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params<'a>(list: &'a [(&'a str, f64)]) -> impl Fn(&str) -> Option<f64> + 'a {
        move |id: &str| list.iter().find(|(k, _)| *k == id).map(|(_, v)| *v)
    }

    fn on(at: u32, pitch: u8) -> BlockEvent {
        BlockEvent { at, event: NoteEvent::On { pitch, velocity: 0.8 } }
    }

    fn off(at: u32, pitch: u8) -> BlockEvent {
        BlockEvent { at, event: NoteEvent::Off { pitch } }
    }

    /// Runs `fx` over `blocks` of `frames`, feeding `input` in the first block.
    fn run(fx: &mut MidiFx, input: &[BlockEvent], frames: u32, blocks: u32, playing: bool) -> Vec<(u64, NoteEvent)> {
        let mut all = Vec::new();
        for b in 0..blocks {
            let mut out = Vec::with_capacity(256);
            let pos = u64::from(b * frames);
            let ctx = FxContext { playing, pos, beat: pos as f64 / 1000.0, samples_per_beat: 1000.0, sample_rate: 48_000.0 };
            let feed: Vec<BlockEvent> = input.iter().filter(|e| e.at / frames == b).map(|e| BlockEvent { at: e.at % frames, ..*e }).collect();
            fx.process(&feed, &mut out, frames, &ctx);
            all.extend(out.iter().map(|e| (u64::from(b * frames + e.at), e.event)));
        }
        all
    }

    fn ons(events: &[(u64, NoteEvent)]) -> Vec<(u64, u8)> {
        events.iter().filter_map(|(t, e)| if let NoteEvent::On { pitch, .. } = e { Some((*t, *pitch)) } else { None }).collect()
    }

    #[test]
    fn keys_parse_and_snap() {
        let a_minor = Key::parse("A minor");
        assert!(a_minor.contains(60) && !a_minor.contains(61));
        assert_eq!(a_minor.snap(61), 62, "C♯ goes up to D (a tie)");
        assert_eq!(Key::parse("E♭ major").snap(64), 65, "E is not in E♭ major; F is");
        assert!(Key::parse("Bb major").contains(70));
        assert!(Key::parse("F# minor pentatonic").contains(66));
        assert!(!Key::parse("F# minor pentatonic").contains(68));
    }

    #[test]
    fn chord_stacks_and_releases_its_notes() {
        let mut fx = MidiFx::build("chord", &params(&[]), Key::parse("C major")).unwrap();
        let out = run(&mut fx, &[on(0, 60), off(100, 60)], 256, 1, true);
        assert_eq!(ons(&out), vec![(0, 60), (0, 64), (0, 67)]);
        assert_eq!(out.iter().filter(|(_, e)| matches!(e, NoteEvent::Off { .. })).count(), 3);
    }

    #[test]
    fn a_parameter_change_still_ends_the_right_notes() {
        let key = Key::parse("C major");
        let mut old = MidiFx::build("chord", &params(&[]), key).unwrap();
        run(&mut old, &[on(0, 60)], 256, 1, true);
        let mut new = MidiFx::build("chord", &params(&[("shift1", 3.0)]), key).unwrap();
        new.adopt(&mut old);
        let out = run(&mut new, &[off(0, 60)], 256, 1, true);
        let offs: Vec<_> = out.iter().map(|(_, e)| *e).collect();
        assert_eq!(offs, vec![NoteEvent::Off { pitch: 60 }, NoteEvent::Off { pitch: 64 }, NoteEvent::Off { pitch: 67 }]);
    }

    #[test]
    fn scale_follows_the_song_key() {
        let mut fx = MidiFx::build("scale", &params(&[]), Key::parse("D minor")).unwrap();
        let out = run(&mut fx, &[on(0, 61), off(10, 61)], 256, 1, true);
        assert_eq!(out, vec![(0, NoteEvent::On { pitch: 62, velocity: 0.8 }), (10, NoteEvent::Off { pitch: 62 })]);
    }

    #[test]
    fn note_length_ends_notes_on_its_own() {
        // 100 ms at 48 kHz = 4800 samples, whatever the key does.
        let mut fx = MidiFx::build("notelength", &params(&[("length", 100.0)]), Key::parse("C major")).unwrap();
        let out = run(&mut fx, &[on(10, 60), off(20, 60)], 1024, 8, false);
        assert_eq!(out, vec![(10, NoteEvent::On { pitch: 60, velocity: 0.8 }), (4810, NoteEvent::Off { pitch: 60 })]);
    }

    #[test]
    fn velocity_maps_into_the_output_range() {
        let mut fx = MidiFx::build("velocity", &params(&[("outLow", 64.0), ("outHigh", 64.0)]), Key::parse("C major")).unwrap();
        let out = run(&mut fx, &[on(0, 60)], 64, 1, true);
        let NoteEvent::On { velocity, .. } = out[0].1 else { panic!() };
        assert!((velocity - 64.0 / 127.0).abs() < 1e-4);
    }

    #[test]
    fn random_stays_in_range_and_in_key() {
        let mut fx = MidiFx::build("random", &params(&[("chance", 100.0), ("range", 5.0), ("inKey", 1.0)]), Key::parse("C major")).unwrap();
        let input: Vec<BlockEvent> = (0..64).flat_map(|i| [on(i * 10, 60), off(i * 10 + 5, 60)]).collect();
        let out = run(&mut fx, &input, 1024, 1, true);
        for (_, p) in ons(&out) {
            assert!((55..=65).contains(&p) && Key::parse("C major").contains(p) && p != 60, "{p}");
        }
        // Every note-off ends what its note-on became.
        let pairs: Vec<u8> = out.iter().map(|(_, e)| match e { NoteEvent::On { pitch, .. } | NoteEvent::Off { pitch } => *pitch, _ => 0 }).collect();
        assert!(pairs.chunks(2).all(|c| c[0] == c[1]));
    }

    #[test]
    fn arp_steps_through_the_chord_on_the_beat_grid() {
        // 1/16 notes at 1000 samples a beat: a step every 250 samples.
        let mut fx = MidiFx::build("arp", &params(&[("rate", 3.0), ("gate", 50.0)]), Key::parse("C major")).unwrap();
        let out = run(&mut fx, &[on(0, 67), on(0, 60), on(0, 64)], 250, 6, true);
        assert_eq!(ons(&out), vec![(0, 60), (250, 64), (500, 67), (750, 60), (1000, 64), (1250, 67)]);
        // Each step lasts half a step.
        assert!(out.contains(&(125, NoteEvent::Off { pitch: 60 })));
    }

    #[test]
    fn arp_up_down_and_octaves_and_stopping() {
        let mut fx = MidiFx::build("arp", &params(&[("style", 2.0), ("rate", 3.0), ("octaves", 2.0)]), Key::parse("C major")).unwrap();
        let out = run(&mut fx, &[on(0, 60), on(0, 64), off(1500, 60), off(1500, 64)], 250, 8, false);
        let pitches: Vec<u8> = ons(&out).iter().map(|(_, p)| *p).collect();
        assert_eq!(pitches, vec![60, 64, 72, 76, 72, 64]);
        // Keys up: nothing new starts, and everything that started has ended.
        let on_count = out.iter().filter(|(_, e)| matches!(e, NoteEvent::On { .. })).count();
        let off_count = out.iter().filter(|(_, e)| matches!(e, NoteEvent::Off { .. })).count();
        assert_eq!(on_count, off_count);
    }
}
