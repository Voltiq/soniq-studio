//! What the audio thread plays: a flat, pre-computed mixer graph.
//!
//! The control thread builds a new `Graph` whenever the project changes and
//! hands it over through a lock-free queue. Everything is resolved up front
//! (timeline positions in samples, gains in linear terms, scratch buffers
//! allocated) so the callback only reads and adds.

use std::sync::Arc;

use soniq_dsp::{AudioFx, BlockEvent, FxContext, Instrument, MidiFx, NoteEvent, ParamBank, WarpClock, Warper, push_event};
use soniq_io::AudioBuffer;

use crate::launch::{ClipStatus, Launch, SlotNode};
use crate::meters::PeakMeter;
use crate::tempo::TempoMap;

/// Largest block the renderer processes at once; bigger device buffers are split.
pub const MAX_BLOCK: usize = 2048;
/// Tracks beyond this are ignored by the renderer's gain smoothing state.
pub const MAX_TRACKS: usize = 512;

/// How a clip's fades and crossfades curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FadeShape {
    /// Straight fades, equal-power crossfades.
    #[default]
    Standard,
    Linear,
    EqualPower,
    SCurve,
}

impl FadeShape {
    /// The gain `x` (0–1) of the way into a fade in; a fade out reads it backwards.
    pub fn gain(self, x: f32, crossfade: bool) -> f32 {
        let x = x.clamp(0.0, 1.0);
        match self {
            FadeShape::Standard if crossfade => (x * std::f32::consts::FRAC_PI_2).sin(),
            FadeShape::Standard | FadeShape::Linear => x,
            FadeShape::EqualPower => (x * std::f32::consts::FRAC_PI_2).sin(),
            FadeShape::SCurve => 0.5 - 0.5 * (x * std::f32::consts::PI).cos(),
        }
    }
}

pub struct ClipNode {
    /// Timeline position and length in samples at the engine rate.
    pub start: u64,
    pub len: u64,
    /// First sample of `buffer` to play.
    pub offset: u64,
    pub gain: f32,
    /// Linear fade lengths in samples.
    pub fade_in: u64,
    pub fade_out: u64,
    /// Equal-power crossfades with the neighbouring clips, in samples.
    pub xfade_in: u64,
    pub xfade_out: u64,
    pub curve: FadeShape,
    /// Audio at the engine sample rate.
    pub buffer: Arc<AudioBuffer>,
    /// Identifies the clip across rebuilt graphs, so a warped one keeps its grains.
    pub key: u64,
    /// Follows the song's tempo instead of playing at its own speed (boxed: its scratch buffers are large).
    pub warp: Option<Box<ClipWarp>>,
}

impl ClipNode {
    pub fn new(start: u64, len: u64, offset: u64, buffer: Arc<AudioBuffer>) -> Self {
        Self { start, len, offset, gain: 1.0, fade_in: 0, fade_out: 0, xfade_in: 0, xfade_out: 0, curve: FadeShape::Standard, buffer, key: 0, warp: None }
    }

    pub fn with_warp(mut self, warp: ClipWarp) -> Self {
        self.warp = Some(Box::new(warp));
        self
    }
}

/// A warped clip: where it sits in beats, its own tempo, and how it stretches.
pub struct ClipWarp {
    pub warper: Warper,
    /// The clip's start on the timeline, in beats.
    pub start_beat: f64,
    /// Source samples per beat at the audio's own tempo.
    pub per_beat: f64,
    /// One block of warped audio, before gain and fades.
    pub(crate) left: Vec<f32>,
    pub(crate) right: Vec<f32>,
}

impl ClipWarp {
    pub fn new(warper: Warper, start_beat: f64, per_beat: f64) -> Self {
        Self { warper, start_beat, per_beat: per_beat.max(1e-6), left: vec![0.0; MAX_BLOCK], right: vec![0.0; MAX_BLOCK] }
    }
}

/// How output positions turn into beats for a warped clip: on the
/// timeline (the tempo track), or looping in a session slot at one tempo.
#[derive(Clone, Copy)]
pub(crate) enum Timing<'a> {
    Timeline(&'a TempoMap),
    Loop { samples_per_beat: f64 },
}

/// The source sample a warped clip plays at each output sample, and back.
pub(crate) struct ClipClock<'a> {
    pub timing: Timing<'a>,
    pub start_beat: f64,
    pub per_beat: f64,
    pub offset: f64,
}

impl WarpClock for ClipClock<'_> {
    fn source_at(&self, out: f64) -> f64 {
        let beats = match self.timing {
            Timing::Timeline(tempo) => tempo.beat_at(out) - self.start_beat,
            Timing::Loop { samples_per_beat } => out / samples_per_beat,
        };
        self.offset + beats * self.per_beat
    }

    fn output_at(&self, source: f64) -> f64 {
        let beats = (source - self.offset) / self.per_beat;
        match self.timing {
            Timing::Timeline(tempo) => tempo.sample_at(self.start_beat + beats),
            Timing::Loop { samples_per_beat } => beats * samples_per_beat,
        }
    }
}

/// Where clips on one track overlap, the earlier one fades out and the later
/// one fades in across the overlap. A clip that sits entirely inside another
/// is layered instead. `clips` must be sorted by start.
pub fn crossfade_overlaps(clips: &mut [ClipNode]) {
    for i in 1..clips.len() {
        let (before, after) = clips.split_at_mut(i);
        let (a, b) = (&mut before[i - 1], &mut after[0]);
        let a_end = a.start + a.len;
        if b.start < a_end && b.start + b.len > a_end && b.start > a.start {
            let overlap = a_end - b.start;
            a.xfade_out = overlap;
            b.xfade_in = overlap;
        }
    }
}

/// A note from a clip, in samples at the engine rate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScheduledNote {
    pub start: u64,
    pub end: u64,
    pub pitch: u8,
    pub velocity: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimedEvent {
    pub at: u64,
    pub event: NoteEvent,
}

/// The pitches a track's clips have switched on and not yet off.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct HeldNotes([u64; 2]);

impl HeldNotes {
    pub fn set(&mut self, pitch: u8, on: bool) {
        let (word, bit) = (usize::from(pitch >> 6) & 1, pitch & 63);
        if on { self.0[word] |= 1 << bit } else { self.0[word] &= !(1 << bit) }
    }

    pub fn contains(&self, pitch: u8) -> bool {
        self.0[usize::from(pitch >> 6) & 1] & (1 << (pitch & 63)) != 0
    }

    pub fn is_empty(&self) -> bool {
        self.0 == [0, 0]
    }

    pub fn pitches(self) -> impl Iterator<Item = u8> {
        (0..128u8).filter(move |p| self.contains(*p))
    }
}

/// Live notes and releases waiting for the next block.
const INBOX: usize = 256;
/// Notes a block can carry through the MIDI effects.
const BLOCK_EVENTS: usize = 512;

/// A track's instrument, its MIDI effects and the notes its clips play.
pub struct InstrumentNode {
    /// The device id: a rebuilt graph hands sounding voices over when it matches.
    pub id: String,
    pub instrument: Box<Instrument>,
    /// Its parameter values, which automation and modulators change.
    pub bank: ParamBank,
    /// MIDI effects in chain order, with their device ids.
    pub fx: Vec<(String, MidiFx)>,
    /// Note-ons and note-offs sorted by time, offs first at the same sample.
    pub events: Vec<TimedEvent>,
    /// The longest note, in samples: a note that started earlier than this
    /// before a position cannot still sound there.
    pub longest: u64,
    pub(crate) held: HeldNotes,
    pub(crate) inbox: Vec<NoteEvent>,
    /// The block's notes on their way through the effects (fixed capacity).
    a: Vec<BlockEvent>,
    b: Vec<BlockEvent>,
}

/// Note-ons and note-offs sorted by time (offs first at the same sample),
/// and the longest note.
pub(crate) fn note_events(notes: &[ScheduledNote]) -> (Vec<TimedEvent>, u64) {
    let mut events = Vec::with_capacity(notes.len() * 2);
    let mut longest = 0;
    for n in notes {
        let end = n.end.max(n.start + 1);
        longest = longest.max(end - n.start);
        events.push(TimedEvent { at: n.start, event: NoteEvent::On { pitch: n.pitch, velocity: n.velocity } });
        events.push(TimedEvent { at: end, event: NoteEvent::Off { pitch: n.pitch } });
    }
    let rank = |e: &TimedEvent| matches!(e.event, NoteEvent::On { .. }) as u8;
    events.sort_by(|a, b| a.at.cmp(&b.at).then(rank(a).cmp(&rank(b))));
    (events, longest)
}

/// Where an instrument's notes come from in a stretch of time.
#[derive(Clone, Copy)]
pub(crate) enum Notes<'a> {
    /// Only notes played live (stopped, or a track whose session stopped).
    Live,
    /// The arrangement's clips, at the timeline position.
    Timeline,
    /// A looping session clip, `phase` samples into its playing.
    Loop { events: &'a [TimedEvent], len: u64, phase: u64 },
}

/// Starts or ends a clip note, keeping track of what is held. An off for a
/// note the clips never started (playback began mid-note) must not cut a
/// note played live.
fn schedule(held: &mut HeldNotes, out: &mut Vec<BlockEvent>, at: u32, event: NoteEvent) {
    match event {
        NoteEvent::On { pitch, .. } => {
            held.set(pitch, true);
            push_event(out, at, event);
        }
        NoteEvent::Off { pitch } if held.contains(pitch) => {
            held.set(pitch, false);
            push_event(out, at, event);
        }
        _ => {}
    }
}

/// The held notes that no longer sound at `pos` in `events`.
fn orphans(held: HeldNotes, events: &[TimedEvent], longest: u64, pos: u64) -> HeldNotes {
    let mut gone = HeldNotes::default();
    if held.is_empty() {
        return gone;
    }
    let first = pos.saturating_sub(longest);
    let before = &events[..events.partition_point(|e| e.at < pos)];
    for pitch in held.pitches() {
        let sounding = before
            .iter()
            .rev()
            .take_while(|e| e.at >= first)
            .find_map(|e| match e.event {
                NoteEvent::On { pitch: p, .. } if p == pitch => Some(true),
                NoteEvent::Off { pitch: p } if p == pitch => Some(false),
                _ => None,
            })
            .unwrap_or(false);
        if !sounding {
            gone.set(pitch, true);
        }
    }
    gone
}

impl InstrumentNode {
    pub fn new(id: String, instrument: Instrument, notes: &[ScheduledNote]) -> Self {
        let (events, longest) = note_events(notes);
        Self {
            id,
            instrument: Box::new(instrument),
            bank: ParamBank::default(),
            fx: Vec::new(),
            events,
            longest,
            held: HeldNotes::default(),
            inbox: Vec::with_capacity(INBOX),
            a: Vec::with_capacity(BLOCK_EVENTS),
            b: Vec::with_capacity(BLOCK_EVENTS),
        }
    }

    /// The MIDI effects the notes pass through first, in order.
    pub fn with_fx(mut self, fx: Vec<(String, MidiFx)>) -> Self {
        self.fx = fx;
        self
    }

    /// The instrument's parameter values, for automation and modulators to change.
    pub fn with_bank(mut self, bank: ParamBank) -> Self {
        self.bank = bank;
        self
    }

    /// Queues a note for the next block (live notes, releases). A full inbox drops it.
    pub(crate) fn post(&mut self, event: NoteEvent) {
        if self.inbox.len() < self.inbox.capacity() {
            self.inbox.push(event);
        }
    }

    /// Takes over the sounding voices and effect state of the node this one replaces.
    pub(crate) fn adopt(&mut self, old: &mut InstrumentNode) {
        self.instrument.adopt(&mut old.instrument);
        self.held = std::mem::take(&mut old.held);
        std::mem::swap(&mut self.inbox, &mut old.inbox);
        let same_chain = self.fx.len() == old.fx.len() && self.fx.iter().zip(&old.fx).all(|(a, b)| a.0 == b.0);
        for (id, fx) in &mut self.fx {
            if let Some((_, was)) = old.fx.iter_mut().find(|(o, _)| o == id) {
                fx.adopt(was);
            }
        }
        // Effects added or removed: notes they changed could hang, so let everything ring out.
        if !same_chain {
            self.post(NoteEvent::AllNotesOff);
        }
    }

    /// Releases every note the timeline started, leaving notes played live alone.
    pub(crate) fn release_clip_notes(&mut self) {
        for pitch in std::mem::take(&mut self.held).pitches() {
            self.post(NoteEvent::Off { pitch });
        }
    }

    /// After an edit, notes that no longer exist at `pos` stop instead of hanging.
    pub(crate) fn release_orphans(&mut self, pos: u64) {
        let gone = orphans(self.held, &self.events, self.longest, pos);
        self.release(gone);
    }

    /// The same for a session clip: `events` at `pos` in its loop.
    pub(crate) fn release_orphans_in(&mut self, events: &[TimedEvent], longest: u64, pos: u64) {
        let gone = orphans(self.held, events, longest, pos);
        self.release(gone);
    }

    fn release(&mut self, gone: HeldNotes) {
        for pitch in gone.pitches() {
            self.held.set(pitch, false);
            self.post(NoteEvent::Off { pitch });
        }
    }

    /// Renders `left.len()` frames: queued notes first, then the clip notes
    /// that fall inside, through the MIDI effects into the instrument,
    /// sample-accurately.
    pub(crate) fn render(&mut self, ctx: &FxContext, left: &mut [f32], right: &mut [f32], notes: Notes) {
        let n = left.len();
        let Self { instrument, fx, events, held, inbox, a, b, .. } = self;
        a.clear();
        for event in inbox.drain(..) {
            push_event(a, 0, event);
        }
        match notes {
            Notes::Live => {}
            Notes::Timeline => {
                let (pos, end) = (ctx.pos, ctx.pos + n as u64);
                let mut i = events.partition_point(|e| e.at < pos);
                while let Some(&TimedEvent { at, event }) = events.get(i).filter(|e| e.at < end) {
                    schedule(held, a, (at - pos) as u32, event);
                    i += 1;
                }
            }
            Notes::Loop { events, len, phase } => {
                let mut done = 0;
                while done < n as u64 {
                    let from = (phase + done) % len;
                    let to = len.min(from + (n as u64 - done));
                    let mut i = events.partition_point(|e| e.at < from);
                    while let Some(&TimedEvent { at, event }) = events.get(i).filter(|e| e.at < to) {
                        schedule(held, a, (done + at - from) as u32, event);
                        i += 1;
                    }
                    done += to - from;
                    // The loop comes round: whatever it still holds ends here.
                    if to == len {
                        for pitch in std::mem::take(held).pitches() {
                            push_event(a, done as u32, NoteEvent::Off { pitch });
                        }
                    }
                }
            }
        }
        for (_, effect) in fx.iter_mut() {
            b.clear();
            effect.process(a, b, n as u32, ctx);
            std::mem::swap(a, b);
        }
        // A plugin takes the whole block with its notes at once.
        if let Instrument::External(plugin) = &mut **instrument {
            plugin.process(ctx, left, right, a);
            return;
        }
        instrument.set_context(ctx);
        let mut cursor = 0;
        for e in a.iter() {
            let at = (e.at as usize).min(n);
            if at > cursor {
                instrument.render(&mut left[cursor..at], &mut right[cursor..at]);
                cursor = at;
            }
            instrument.handle(e.event);
        }
        if cursor < n && !instrument.is_silent() {
            instrument.render(&mut left[cursor..], &mut right[cursor..]);
        }
    }
}

/// An audio effect in a chain.
pub struct FxNode {
    /// The device id: a rebuilt graph hands the effect's state over when it matches.
    pub id: String,
    pub fx: AudioFx,
    /// The track (index into [`Graph::tracks`]) a compressor or gate listens to.
    pub sidechain: Option<usize>,
    /// Its parameter values, which automation and modulators change.
    pub bank: ParamBank,
    /// Gain reduction (dB) for the device's meter, on dynamics effects.
    pub meter: Option<Arc<PeakMeter>>,
    /// Its output, for a live spectrum in its panel: the device id and the queue.
    pub tap: Option<Box<(String, rtrb::Producer<f32>)>>,
}

impl FxNode {
    pub fn new(id: String, fx: AudioFx, sidechain: Option<usize>, bank: ParamBank) -> Self {
        Self { id, fx, sidechain, bank, meter: None, tap: None }
    }

    /// After processing a block: the meter reads its gain reduction, the tap its output.
    pub(crate) fn after(&mut self, left: &[f32], right: &[f32]) {
        if let (Some(meter), Some(db)) = (&self.meter, self.fx.reduction()) {
            meter.update(db, 0.0);
        }
        if let Some(tap) = self.tap.as_mut()
            && tap.1.slots() >= left.len()
        {
            for (l, r) in left.iter().zip(right) {
                let _ = tap.1.push(0.5 * (l + r));
            }
        }
    }
}

/// Samples between automation and modulator updates.
pub const CONTROL: usize = 64;

/// Where an automated or modulated value goes on its track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dest {
    Volume,
    Pan,
    Send(usize),
    /// A parameter of an effect: its index in the track's chain and in its bank.
    Fx { fx: usize, param: usize },
    /// A parameter of the track's instrument (its index in the instrument's bank).
    Instrument { param: usize },
}

/// An automation lane: breakpoints in samples, joined by straight lines.
pub struct LaneNode {
    pub dest: Dest,
    pub points: Vec<(u64, f64)>,
}

impl LaneNode {
    pub fn value_at(&self, pos: u64) -> f64 {
        let i = self.points.partition_point(|p| p.0 <= pos);
        match (i.checked_sub(1).map(|j| self.points[j]), self.points.get(i)) {
            (Some(a), Some(b)) if b.0 > a.0 => a.1 + (b.1 - a.1) * (pos - a.0) as f64 / (b.0 - a.0) as f64,
            (Some(a), _) => a.1,
            (None, Some(b)) => b.1,
            (None, None) => 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModSource {
    /// Shape 0–5: sine, triangle, square, saw up, saw down, random. Free at
    /// `rate` Hz, or one cycle every `sync` beats locked to the song.
    Lfo { shape: u8, rate: f64, sync: Option<f64>, phase: f64 },
    /// Follows the track's own level; attack and release are per-sample factors.
    Follower { gain: f64, attack: f64, release: f64 },
}

/// A modulator moving one parameter of a device on its track.
pub struct ModNode {
    pub dest: Dest,
    pub source: ModSource,
    /// −1…1: how far it moves the parameter, as a share of its range.
    pub depth: f64,
    /// The parameter's resting value, range and curve.
    pub base: f64,
    pub min: f64,
    pub max: f64,
    pub log: bool,
    /// An automation lane on the same parameter sets the resting value instead.
    pub lane: Option<usize>,
    /// Free-running LFO phase in turns, or the follower's level.
    pub(crate) state: f64,
    held: f64,
    cycle: i64,
    seed: u32,
}

impl ModNode {
    pub fn new(dest: Dest, source: ModSource, depth: f64, base: f64, (min, max, log): (f64, f64, bool)) -> Self {
        Self { dest, source, depth: depth.clamp(-1.0, 1.0), base, min, max, log, lane: None, state: 0.0, held: 0.0, cycle: i64::MIN, seed: 0x9e37_79b9 }
    }

    fn norm(&self, v: f64) -> f64 {
        if self.max <= self.min {
            return 0.0;
        }
        if self.log && self.min > 0.0 { (v.max(self.min) / self.min).ln() / (self.max / self.min).ln() } else { (v - self.min) / (self.max - self.min) }
    }

    fn denorm(&self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        if self.log && self.min > 0.0 { self.min * (self.max / self.min).powf(t) } else { self.min + (self.max - self.min) * t }
    }

    /// The modulator's output for the next `n` samples: −1…1 for an LFO, 0…1 for a follower.
    fn wave(&mut self, at: u64, n: usize, playing: bool, tempo: &TempoMap, sr: f64) -> f64 {
        let ModSource::Lfo { shape, rate, sync, phase } = self.source else { return self.state };
        let turns = match sync {
            Some(beats) if playing => tempo.beat_at(at as f64) / beats + phase,
            _ => {
                // Free running (and synced ones while the song is stopped).
                let hz = sync.map_or(rate, |beats| sr / (beats * tempo.samples_per_beat_at(at as f64)));
                let t = self.state + phase;
                self.state = (self.state + hz * n as f64 / sr).fract();
                t
            }
        };
        let t = turns.rem_euclid(1.0);
        match shape {
            0 => (t * std::f64::consts::TAU).sin(),
            1 => 1.0 - 4.0 * (t - 0.5).abs(),
            2 => {
                if t < 0.5 { 1.0 } else { -1.0 }
            }
            3 => 2.0 * t - 1.0,
            4 => 1.0 - 2.0 * t,
            _ => {
                // Sample and hold: a new random value each cycle.
                let cycle = turns.floor() as i64;
                if cycle != self.cycle {
                    self.cycle = cycle;
                    self.seed ^= self.seed << 13;
                    self.seed ^= self.seed >> 17;
                    self.seed ^= self.seed << 5;
                    self.held = f64::from(self.seed) / f64::from(u32::MAX) * 2.0 - 1.0;
                }
                self.held
            }
        }
    }

    /// The parameter's value now, moved from `base`.
    fn value(&mut self, base: f64, at: u64, n: usize, playing: bool, tempo: &TempoMap, sr: f64) -> f64 {
        let wave = self.wave(at, n, playing, tempo, sr);
        // An LFO swings either way by half the range at full depth; a follower pushes one way.
        let reach = if matches!(self.source, ModSource::Lfo { .. }) { 0.5 } else { 1.0 };
        self.denorm(self.norm(base) + self.depth * wave * reach)
    }

    /// Follows the track's level over the samples just rendered.
    pub(crate) fn follow(&mut self, left: &[f32], right: &[f32]) {
        let ModSource::Follower { gain, attack, release } = self.source else { return };
        for (l, r) in left.iter().zip(right) {
            let level = (f64::from(l.abs().max(r.abs())) * gain).min(1.0);
            let coef = if level > self.state { attack } else { release };
            self.state = level + (self.state - level) * coef;
        }
    }
}

/// Lets every effect in `new` carry on from the one with the same id in `old`.
pub(crate) fn adopt_chain(new: &mut [FxNode], old: &mut [FxNode]) {
    for node in new {
        if let Some(was) = old.iter_mut().find(|o| o.id == node.id) {
            node.fx.adopt(&mut was.fx);
            // A live spectrum stays on its device.
            node.tap = was.tap.take();
        }
    }
}

/// Returns beyond this get no send smoothing (and no sends).
pub const MAX_RETURNS: usize = 8;

fn block() -> [Vec<f32>; 2] {
    [vec![0.0; MAX_BLOCK], vec![0.0; MAX_BLOCK]]
}

/// Holds a stereo signal back by a fixed number of samples: what plugin
/// delay compensation adds to every path that is quicker than the slowest.
#[derive(Default)]
pub struct Delay {
    lines: [Vec<f32>; 2],
    write: usize,
}

impl Delay {
    /// A delay of `samples` (allocates: build time only).
    pub fn new(samples: usize) -> Self {
        Self { lines: [vec![0.0; samples], vec![0.0; samples]], write: 0 }
    }

    pub fn samples(&self) -> usize {
        self.lines[0].len()
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let len = self.samples();
        if len == 0 {
            return;
        }
        let [dl, dr] = &mut self.lines;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let w = self.write;
            std::mem::swap(l, &mut dl[w]);
            std::mem::swap(r, &mut dr[w]);
            self.write = if w + 1 == len { 0 } else { w + 1 };
        }
    }

    /// Carries on from the delay this one replaces, when it is as long.
    pub fn adopt(&mut self, old: &mut Delay) {
        if old.samples() == self.samples() {
            std::mem::swap(&mut self.lines, &mut old.lines);
            self.write = old.write;
        }
    }
}

pub struct TrackNode {
    /// Identifies the track across rebuilt graphs and for live notes.
    pub key: u64,
    /// Left and right gain with volume, pan, mute and solo already applied.
    pub gain: [f32; 2],
    pub clips: Vec<ClipNode>,
    pub meter: Arc<PeakMeter>,
    /// Input channels (left, right) played through this track, when monitoring.
    pub monitor: Option<(usize, usize)>,
    pub instrument: Option<InstrumentNode>,
    /// Audio effects after the clips and instrument, before the fader.
    pub fx: Vec<FxNode>,
    /// Send level to each return (linear, 0 when the track is muted).
    pub sends: Vec<f32>,
    /// The fader as set, for automation to start from: dB, pan, and whether
    /// mute and solo let the track be heard.
    pub mixer: (f64, f64, bool),
    pub lanes: Vec<LaneNode>,
    pub mods: Vec<ModNode>,
    /// The group (index into [`Graph::groups`]) this track plays into, instead of the master.
    pub output: Option<usize>,
    /// For a group track: which group input it plays.
    pub group: Option<usize>,
    /// Session clips, sorted by scene.
    pub slots: Vec<SlotNode>,
    /// What the session plays; each new graph takes it over.
    pub(crate) launch: Launch,
    pub status: Arc<ClipStatus>,
    /// The track's own sound (clips, instrument, input): what sidechains hear.
    pub(crate) raw: [Vec<f32>; 2],
    /// After the effects.
    pub(crate) out: [Vec<f32>; 2],
    /// Makes up for plugins on slower tracks, after the effects.
    pub(crate) comp: Delay,
    /// How late the sound reaching the fader is, so volume, pan and send
    /// automation are read that much later too.
    pub(crate) lag: u64,
    /// Session clips that started or stopped in this stretch, handed on once
    /// every track has played it (room for a few, made when it is built).
    pub(crate) switched: Vec<crate::launch::Launched>,
}

impl TrackNode {
    pub fn new(gain: [f32; 2], clips: Vec<ClipNode>, meter: Arc<PeakMeter>) -> Self {
        Self {
            key: 0,
            gain,
            clips,
            meter,
            monitor: None,
            instrument: None,
            fx: Vec::new(),
            sends: Vec::new(),
            mixer: (0.0, 0.0, true),
            lanes: Vec::new(),
            mods: Vec::new(),
            output: None,
            group: None,
            slots: Vec::new(),
            launch: Launch::default(),
            status: Arc::new(ClipStatus::default()),
            raw: block(),
            out: block(),
            comp: Delay::default(),
            lag: 0,
            switched: Vec::with_capacity(4),
        }
    }

    /// Samples its instrument and effects delay its sound by.
    pub fn latency(&self) -> u64 {
        let fx: u64 = self.fx.iter().map(|f| u64::from(f.fx.latency())).sum();
        fx + self.instrument.as_ref().map_or(0, |i| u64::from(i.instrument.latency()))
    }

    /// Automation and modulators move something on this track.
    pub fn controlled(&self) -> bool {
        !self.lanes.is_empty() || !self.mods.is_empty()
    }

    /// Moves what the automation lanes and modulators move, for the `n`
    /// samples from timeline position `at`. Devices whose values changed take
    /// them without allocating.
    pub(crate) fn apply_controls(&mut self, at: u64, n: usize, playing: bool, tempo: &TempoMap, sr: f64) {
        let Self { lanes, mods, fx, instrument, gain, sends, mixer, lag, .. } = self;
        let (mut volume, mut pan, audible) = *mixer;
        let mut dirty = Dirty::default();
        for lane in lanes.iter() {
            // The fader hears the sound late by the plugins and the compensation.
            let mixer_at = at.saturating_sub(*lag);
            match lane.dest {
                Dest::Volume => volume = lane.value_at(mixer_at),
                Dest::Pan => pan = lane.value_at(mixer_at),
                Dest::Send(i) => {
                    if let Some(s) = sends.get_mut(i) {
                        *s = if audible { lane.value_at(mixer_at) as f32 } else { 0.0 };
                    }
                }
                dest => dirty.set(dest, lane.value_at(at), fx, instrument),
            }
        }
        for m in mods.iter_mut() {
            let base = m.lane.and_then(|l| lanes.get(l)).map_or(m.base, |lane| lane.value_at(at));
            let v = m.value(base, at, n, playing, tempo, sr);
            dirty.set(m.dest, v, fx, instrument);
        }
        *gain = track_gains(volume, pan, audible);
        for (i, node) in fx.iter_mut().enumerate() {
            if dirty.fx(i) {
                node.fx.configure(&node.bank);
            }
        }
        if dirty.instrument
            && let Some(node) = instrument
        {
            node.instrument.configure(&node.bank);
        }
    }
}

/// Which devices got new values in a control step.
#[derive(Default)]
struct Dirty {
    fx: u64,
    /// Effects past the 64th are retuned whenever anything changed.
    many: bool,
    instrument: bool,
}

impl Dirty {
    fn set(&mut self, dest: Dest, v: f64, fx: &mut [FxNode], instrument: &mut Option<InstrumentNode>) {
        match dest {
            Dest::Fx { fx: i, param } => {
                if fx.get_mut(i).is_some_and(|node| node.bank.set(param, v)) {
                    match 1u64.checked_shl(i as u32) {
                        Some(bit) => self.fx |= bit,
                        None => self.many = true,
                    }
                }
            }
            Dest::Instrument { param } if instrument.as_mut().is_some_and(|node| node.bank.set(param, v)) => self.instrument = true,
            _ => {}
        }
    }

    fn fx(&self, i: usize) -> bool {
        1u64.checked_shl(i as u32).map_or(self.many, |bit| self.fx & bit != 0)
    }
}

/// A return (send) track: what the tracks send it, through its effects and fader.
pub struct ReturnNode {
    pub key: u64,
    pub gain: [f32; 2],
    /// Tracks send before their fader.
    pub pre_fader: bool,
    pub fx: Vec<FxNode>,
    pub meter: Arc<PeakMeter>,
    pub(crate) input: [Vec<f32>; 2],
    /// Makes up for plugins on slower returns, after the effects.
    pub(crate) comp: Delay,
}

impl ReturnNode {
    pub fn new(key: u64, gain: [f32; 2], pre_fader: bool, fx: Vec<FxNode>, meter: Arc<PeakMeter>) -> Self {
        Self { key, gain, pre_fader, fx, meter, input: block(), comp: Delay::default() }
    }

    pub fn latency(&self) -> u64 {
        self.fx.iter().map(|f| u64::from(f.fx.latency())).sum()
    }
}

/// A stable key for a track id (FNV-1a), used to route live notes.
pub fn track_key(id: &str) -> u64 {
    id.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

pub struct Graph {
    pub tracks: Vec<TrackNode>,
    pub returns: Vec<ReturnNode>,
    /// Effects on the sum of everything, before the master fader.
    pub master_fx: Vec<FxNode>,
    pub master_gain: f32,
    /// What each group track's tracks send it, gathered before the group plays.
    pub groups: Vec<[Vec<f32>; 2]>,
    /// The tempo track at the engine rate; without one the transport's tempo holds throughout.
    pub tempo: Option<TempoMap>,
    /// Tracks and groups playing straight into the master, held back as long
    /// as the slowest return so dry and wet stay together.
    pub(crate) dry: [Vec<f32>; 2],
    pub(crate) dry_comp: Delay,
    /// Samples everything comes out late by: the slowest track and return,
    /// and the master's effects.
    pub latency: u64,
}

impl Graph {
    /// Every effect: on the tracks, the returns and the master.
    pub fn fx_nodes_mut(&mut self) -> impl Iterator<Item = &mut FxNode> {
        self.tracks
            .iter_mut()
            .flat_map(|t| t.fx.iter_mut())
            .chain(self.returns.iter_mut().flat_map(|r| r.fx.iter_mut()))
            .chain(self.master_fx.iter_mut())
    }

    pub fn new(tracks: Vec<TrackNode>, master_gain: f32) -> Self {
        Self {
            tracks,
            returns: Vec::new(),
            master_fx: Vec::new(),
            master_gain,
            groups: Vec::new(),
            tempo: None,
            dry: block(),
            dry_comp: Delay::default(),
            latency: 0,
        }
    }

    /// Plugin delay compensation: every track is held back so it comes out
    /// with the slowest one (a track in a group with the group's plugins
    /// counted in), every return likewise, and the dry mix as long as the
    /// slowest return. Call once the graph is complete (allocates).
    pub fn compensate(&mut self) {
        let own: Vec<u64> = self.tracks.iter().map(TrackNode::latency).collect();
        let group_latency = |g: usize| self.tracks.iter().position(|t| t.group == Some(g)).map_or(0, |i| own[i]);
        let path: Vec<u64> = self.tracks.iter().enumerate().map(|(i, t)| if t.group.is_some() { own[i] } else { own[i] + t.output.map_or(0, group_latency) }).collect();
        let slowest = path.iter().copied().max().unwrap_or(0);
        for (i, track) in self.tracks.iter_mut().enumerate() {
            // A group's tracks come in already lined up for it.
            let hold = if track.group.is_some() { 0 } else { slowest - path[i] };
            track.comp = Delay::new(hold as usize);
            track.lag = own[i] + hold;
        }
        let returns: Vec<u64> = self.returns.iter().map(ReturnNode::latency).collect();
        let wet = returns.iter().copied().max().unwrap_or(0);
        for (ret, own) in self.returns.iter_mut().zip(&returns) {
            ret.comp = Delay::new((wet - own) as usize);
        }
        self.dry_comp = Delay::new(wet as usize);
        let master: u64 = self.master_fx.iter().map(|f| u64::from(f.fx.latency())).sum();
        self.latency = slowest + wet + master;
    }

    /// Room for `count` group inputs (allocates: build time only).
    pub fn with_groups(mut self, count: usize) -> Self {
        self.groups = (0..count).map(|_| block()).collect();
        self
    }

    pub fn with_tempo(mut self, tempo: TempoMap) -> Self {
        self.tempo = Some(tempo);
        self
    }

    pub fn empty() -> Self {
        Self::new(Vec::new(), 1.0)
    }
}

pub fn db_to_gain(db: f64) -> f32 {
    if db <= -60.0 { 0.0 } else { 10f64.powf(db / 20.0) as f32 }
}

/// Balance-style pan: centre is unity on both sides, turning one way only
/// attenuates the other side. Silent when the track is not audible.
pub fn track_gains(volume_db: f64, pan: f64, audible: bool) -> [f32; 2] {
    if !audible {
        return [0.0, 0.0];
    }
    let g = db_to_gain(volume_db);
    let pan = pan.clamp(-1.0, 1.0) as f32;
    [g * (1.0 - pan.max(0.0)), g * (1.0 + pan.min(0.0))]
}
