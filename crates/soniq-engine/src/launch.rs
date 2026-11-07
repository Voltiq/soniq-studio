//! Session clips. Each track plays at most one, started on the launch grid
//! and looping until another takes over, it is stopped, or its follow action
//! picks what comes next. A track that launches or stops a session clip
//! leaves the arrangement until "Back to arrangement".

use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32};

use serde::Serialize;

use crate::graph::{ClipNode, ScheduledNote, TimedEvent, note_events};
use crate::tempo::TempoMap;

/// What a session clip does when its follow time comes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Follow {
    Stop,
    Again,
    Next,
    Previous,
    First,
    Last,
    /// Any clip on the track, this one included.
    Any,
    /// Any other clip on the track.
    Other,
}

pub enum SlotContent {
    /// Audio from the clip's start: the node starts at 0 and its length is the loop.
    Audio(ClipNode),
    /// Notes sorted by time (offs first at the same sample), and the longest.
    Midi { events: Vec<TimedEvent>, longest: u64 },
}

/// A clip waiting in a scene's slot.
pub struct SlotNode {
    pub scene: usize,
    /// The loop, in samples (at least 1).
    pub len: u64,
    pub content: SlotContent,
    /// The follow action and after how many samples of playing it acts (at least 1).
    pub follow: Option<(Follow, u64)>,
}

impl SlotNode {
    pub fn midi(scene: usize, len: u64, notes: &[ScheduledNote]) -> Self {
        let (events, longest) = note_events(notes);
        Self { scene, len: len.max(1), content: SlotContent::Midi { events, longest }, follow: None }
    }

    pub fn audio(scene: usize, mut clip: ClipNode) -> Self {
        clip.start = 0;
        Self { scene, len: clip.len.max(1), content: SlotContent::Audio(clip), follow: None }
    }

    pub fn with_follow(mut self, follow: Option<(Follow, u64)>) -> Self {
        self.follow = follow.map(|(f, after)| (f, after.max(1)));
        self
    }
}

/// When a launch takes effect.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Wait {
    /// With the first sample played (it was launched while stopped).
    Start,
    /// At this timeline sample: a line of a grid `grid` beats apart.
    At { at: u64, grid: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Queued {
    /// The scene to play, or `None` to stop.
    pub scene: Option<usize>,
    pub wait: Wait,
}

/// The first line of a grid `grid` beats apart at or after timeline sample `pos`.
pub(crate) fn next_line(pos: u64, grid: f64, tempo: &TempoMap) -> u64 {
    if grid <= 1e-6 {
        return pos;
    }
    let beat = tempo.beat_at(pos as f64);
    let line = (beat / grid - 1e-6).ceil().max(0.0) * grid;
    (tempo.sample_at(line).round() as u64).max(pos)
}

/// A track's session state. It outlives graphs: each new graph takes it over.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Launch {
    /// The scene whose clip plays.
    pub playing: Option<usize>,
    /// Samples it has played.
    pub phase: u64,
    pub queued: Option<Queued>,
    /// No session clip has taken the track over.
    pub arrangement: bool,
    seed: u32,
}

impl Default for Launch {
    fn default() -> Self {
        Self { playing: None, phase: 0, queued: None, arrangement: true, seed: 0x2545_f491 }
    }
}

impl Launch {
    pub fn slot<'a>(&self, slots: &'a [SlotNode]) -> Option<&'a SlotNode> {
        let scene = self.playing?;
        slots.iter().find(|s| s.scene == scene)
    }

    /// Samples (at most `left`) from timeline position `here` until a launch
    /// or follow action is due; `left` if none is.
    pub fn until_switch(&self, here: u64, left: usize, slots: &[SlotNode]) -> usize {
        let samples = |n: u64| usize::try_from(n).unwrap_or(usize::MAX);
        let mut n = left;
        if let Some(q) = self.queued {
            n = n.min(match q.wait {
                Wait::Start => 0,
                Wait::At { at, .. } => samples(at.saturating_sub(here)),
            });
        }
        if let Some((_, after)) = self.slot(slots).and_then(|s| s.follow) {
            n = n.min(samples(after.saturating_sub(self.phase)));
        }
        n
    }

    /// Does what is due at `here`: the launch waiting, or else the follow action.
    pub fn switch(&mut self, here: u64, slots: &[SlotNode]) {
        let due = self.queued.filter(|q| match q.wait {
            Wait::Start => true,
            Wait::At { at, .. } => at <= here,
        });
        let next = match (due, self.slot(slots).and_then(|s| s.follow)) {
            (Some(q), _) => {
                self.queued = None;
                q.scene
            }
            (None, Some((follow, _))) => self.follow(follow, slots),
            (None, None) => return,
        };
        self.arrangement = false;
        self.playing = next.filter(|s| slots.iter().any(|x| x.scene == *s));
        self.phase = 0;
    }

    /// The scene a follow action goes to, among the track's clips (`slots` is sorted by scene).
    fn follow(&mut self, action: Follow, slots: &[SlotNode]) -> Option<usize> {
        let current = slots.iter().position(|s| Some(s.scene) == self.playing)?;
        let count = slots.len();
        let pick = match action {
            Follow::Stop => return None,
            Follow::Again => current,
            Follow::Next => (current + 1) % count,
            Follow::Previous => (current + count - 1) % count,
            Follow::First => 0,
            Follow::Last => count - 1,
            Follow::Any => self.random(count),
            Follow::Other if count > 1 => (current + 1 + self.random(count - 1)) % count,
            Follow::Other => current,
        };
        Some(slots[pick].scene)
    }

    /// xorshift: no allocation, no locks.
    fn random(&mut self, n: usize) -> usize {
        let mut x = self.seed;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        x as usize % n.max(1)
    }

    pub fn queue(&mut self, scene: Option<usize>, wait: Wait) {
        self.queued = Some(Queued { scene, wait });
    }

    /// The timeline jumped: a launch waiting for a grid line waits for the next one from `pos`.
    pub fn rejoin(&mut self, pos: u64, tempo: &TempoMap) {
        if let Some(Queued { wait: Wait::At { at, grid }, .. }) = &mut self.queued {
            *at = next_line(pos, *grid, tempo);
        }
    }

    /// The transport stopped: clips stop and launches waiting are dropped.
    pub fn halt(&mut self) {
        self.playing = None;
        self.queued = None;
        self.phase = 0;
    }

    pub fn back_to_arrangement(&mut self) {
        self.halt();
        self.arrangement = true;
    }
}

/// What a track's session is doing, as the UI shows it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipState {
    /// The scene whose clip plays.
    pub playing: Option<usize>,
    /// The scene whose clip starts on the next launch line.
    pub queued: Option<usize>,
    /// A stop waits for the next launch line.
    pub stopping: bool,
    /// How far through its loop the playing clip is, 0–1.
    pub progress: f32,
    /// The track plays the arrangement.
    pub arrangement: bool,
}

/// [`ClipState`] as the audio thread writes it.
pub struct ClipStatus {
    playing: AtomicI32,
    /// −1 for nothing waiting, −2 for a stop.
    queued: AtomicI32,
    progress: AtomicU32,
    arrangement: AtomicBool,
}

impl Default for ClipStatus {
    fn default() -> Self {
        Self { playing: AtomicI32::new(-1), queued: AtomicI32::new(-1), progress: AtomicU32::new(0), arrangement: AtomicBool::new(true) }
    }
}

impl ClipStatus {
    pub(crate) fn publish(&self, launch: &Launch, slots: &[SlotNode]) {
        let scene = |s: Option<usize>| s.map_or(-1, |s| i32::try_from(s).unwrap_or(i32::MAX));
        self.playing.store(scene(launch.playing), Relaxed);
        self.queued.store(
            match launch.queued {
                None => -1,
                Some(Queued { scene: None, .. }) => -2,
                Some(Queued { scene: s, .. }) => scene(s),
            },
            Relaxed,
        );
        let progress = launch.slot(slots).map_or(0.0, |s| (launch.phase % s.len) as f32 / s.len as f32);
        self.progress.store(progress.to_bits(), Relaxed);
        self.arrangement.store(launch.arrangement, Relaxed);
    }

    pub fn read(&self) -> ClipState {
        let scene = |v: i32| usize::try_from(v).ok();
        let queued = self.queued.load(Relaxed);
        ClipState {
            playing: scene(self.playing.load(Relaxed)),
            queued: scene(queued),
            stopping: queued == -2,
            progress: f32::from_bits(self.progress.load(Relaxed)),
            arrangement: self.arrangement.load(Relaxed),
        }
    }
}

/// A session clip started (or the track stopped) at a timeline sample: what
/// recording the session into the arrangement needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Launched {
    /// [`track_key`](crate::graph::track_key) of the track.
    pub track: u64,
    pub scene: Option<usize>,
    pub at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slots(scenes: &[usize]) -> Vec<SlotNode> {
        scenes.iter().map(|&s| SlotNode::midi(s, 100, &[])).collect()
    }

    #[test]
    fn launches_wait_for_the_next_grid_line() {
        // 120 BPM at 1 kHz: a beat is 500 samples, so a grid of 0.8 beats is 400.
        let tempo = TempoMap::constant(120.0, 1000.0);
        assert_eq!(next_line(0, 0.8, &tempo), 0);
        assert_eq!(next_line(1, 0.8, &tempo), 400);
        assert_eq!(next_line(800, 0.8, &tempo), 800);
        assert_eq!(next_line(801, 0.0, &tempo), 801);
        // A tempo track moves the lines: 60 BPM from beat 2 on, so beat 4 is at 1000 + 2000.
        let slower = TempoMap::new([(0.0, 0.0, 120.0, 0.0), (2.0, 1.0, 60.0, 0.0)], 1000.0);
        assert_eq!(next_line(1200, 4.0, &slower), 3000);
        let slots = slots(&[0, 2]);
        let mut l = Launch::default();
        l.queue(Some(2), Wait::At { at: 400, grid: 0.8 });
        assert_eq!(l.until_switch(150, 1000, &slots), 250);
        l.switch(400, &slots);
        assert_eq!((l.playing, l.arrangement, l.queued), (Some(2), false, None));
    }

    #[test]
    fn follow_actions_pick_the_next_clip() {
        let slots = slots(&[1, 3, 4]);
        let mut l = Launch { playing: Some(4), ..Launch::default() };
        assert_eq!(l.follow(Follow::Next, &slots), Some(1), "wraps to the first");
        assert_eq!(l.follow(Follow::Previous, &slots), Some(3));
        assert_eq!(l.follow(Follow::First, &slots), Some(1));
        assert_eq!(l.follow(Follow::Stop, &slots), None);
        for _ in 0..20 {
            assert_ne!(l.follow(Follow::Other, &slots), Some(4));
        }
        // A follow action after 50 samples: due then, and the phase restarts.
        let mut slots = slots;
        slots[2] = SlotNode::midi(4, 100, &[]).with_follow(Some((Follow::Again, 50)));
        l.phase = 30;
        assert_eq!(l.until_switch(0, 64, &slots), 20);
        l.phase = 50;
        l.switch(0, &slots);
        assert_eq!((l.playing, l.phase), (Some(4), 0));
    }
}
