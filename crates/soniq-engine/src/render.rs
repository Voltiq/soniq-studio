//! The audio callback's work. Nothing in here locks, allocates or blocks.


use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cpal::{FromSample, SizedSample};
use soniq_dsp::{FxContext, NoteEvent};
use soniq_io::AudioBuffer;

use crate::graph::{CONTROL, ClipClock, ClipNode, ClipWarp, FadeShape, Graph, MAX_BLOCK, MAX_RETURNS, MAX_TRACKS, Notes, ReturnNode, Timing, TrackNode, adopt_chain};
use crate::launch::{Launched, SlotContent, SlotNode, Wait, next_line};
use crate::meters::PeakMeter;
use crate::meters::Safety;
use crate::metronome::Metronome;
pub use crate::monitor::MonitorFeed;
use crate::tempo::TempoMap;
use crate::transport::Transport;
use crate::workers::Workers;

/// How a preview plays: once or round and round, and on what beat grid it
/// starts while the song plays (0: at once).
#[derive(Debug, Clone, Copy, Default)]
pub struct PreviewMode {
    pub looping: bool,
    pub grid: f64,
}

/// Messages to the audio thread besides new graphs.
pub enum Control {
    /// Plays a buffer (at the engine rate) once, on top of everything else.
    Preview(Arc<AudioBuffer>, PreviewMode),
    StopPreview,
    Monitor(Option<Box<MonitorFeed>>),
    /// Starts the clip in a scene's slot on the track with this key (`None`
    /// stops the track) on the next line of a grid this many beats apart.
    Launch { track: u64, scene: Option<usize>, grid: f64 },
    /// Every track plays its clip in the scene; tracks with none stop.
    LaunchScene { scene: usize, grid: f64 },
    /// Every session clip stops.
    StopClips { grid: f64 },
    /// Every track plays the arrangement again, at once.
    BackToArrangement,
    /// Sends one effect's output (by device id) to the queue, or stops.
    Tap(Option<Box<(String, rtrb::Producer<f32>)>>),
}

/// Whatever the audio thread lets go of, sent back to be freed elsewhere.
pub enum Garbage {
    Graph(#[allow(dead_code)] Box<Graph>),
    Preview(#[allow(dead_code)] Arc<AudioBuffer>),
    Monitor(#[allow(dead_code)] Box<MonitorFeed>),
    Tap(#[allow(dead_code)] Box<(String, rtrb::Producer<f32>)>),
}

/// A note played live (MIDI input, the computer keyboard, the piano roll),
/// for the track with this key.
#[derive(Debug, Clone, Copy)]
pub struct LiveEvent {
    /// [`track_key`](crate::graph::track_key) of the track, or [`ALL_TRACKS`].
    pub track: u64,
    pub event: NoteEvent,
}

/// Sends a live event to every instrument (panic).
pub const ALL_TRACKS: u64 = 0;

/// The renderer's connections to the control thread.
pub struct Links {
    pub graphs: rtrb::Consumer<Box<Graph>>,
    pub controls: rtrb::Consumer<Control>,
    pub live: rtrb::Consumer<LiveEvent>,
    pub garbage: rtrb::Producer<Garbage>,
    pub master_meter: Arc<PeakMeter>,
    /// True while a preview plays.
    pub previewing: Arc<AtomicBool>,
    /// Session clips starting and stopping, for recording them into the arrangement.
    pub launched: rtrb::Producer<Launched>,
    /// Threads that share the tracks' work; without them the audio thread does it all.
    pub workers: Option<Arc<Workers>>,
    /// What the master plays (interleaved stereo, without the click), while
    /// `analysing` is on: for the loudness meters and the spectrum.
    pub analysis: Option<rtrb::Producer<f32>>,
    pub analysing: Arc<AtomicBool>,
    /// The speakers' last guard; `None` for offline renders, which are not limited.
    pub safety: Option<Arc<Safety>>,
}

impl Links {
    /// Links for a renderer nobody else talks to (offline renders, tests),
    /// starting on `graph`.
    pub fn detached(graph: Graph) -> Self {
        let (mut tx, graphs) = rtrb::RingBuffer::new(1);
        let _ = tx.push(Box::new(graph));
        let (_, controls) = rtrb::RingBuffer::new(1);
        let (_, live) = rtrb::RingBuffer::new(1);
        let (garbage, _) = rtrb::RingBuffer::new(4);
        let (launched, _) = rtrb::RingBuffer::new(1);
        Self {
            graphs,
            controls,
            live,
            garbage,
            master_meter: Arc::new(PeakMeter::default()),
            previewing: Arc::new(AtomicBool::new(false)),
            launched,
            workers: None,
            analysis: None,
            analysing: Arc::new(AtomicBool::new(false)),
            safety: None,
        }
    }
}

/// The safety limiter's ceiling: −0.3 dBFS.
const CEILING: f32 = 0.966_05;

/// The last thing before the device: a block with NaN or infinity becomes
/// silence; then, if on, peaks over the ceiling are turned down at once and
/// let back up over about 100 ms.
fn protect(left: &mut [f32], right: &mut [f32], gain: &mut f32, release: f32, safety: &Safety) {
    if left.iter().chain(right.iter()).any(|s| !s.is_finite()) {
        left.fill(0.0);
        right.fill(0.0);
        *gain = 1.0;
        safety.silence();
        return;
    }
    if !safety.enabled() {
        *gain = 1.0;
        return;
    }
    let mut lowest = 1.0_f32;
    for (l, r) in left.iter_mut().zip(right.iter_mut()) {
        *gain += (1.0 - *gain) * release;
        let peak = l.abs().max(r.abs());
        if peak * *gain > CEILING {
            *gain = CEILING / peak;
        }
        lowest = lowest.min(*gain);
        *l *= *gain;
        *r *= *gain;
    }
    if lowest < 0.999 {
        safety.reduced(-20.0 * lowest.log10());
    }
}

pub struct Renderer {
    transport: Arc<Transport>,
    graph: Box<Graph>,
    links: Links,
    master: [Vec<f32>; 2],
    click: Vec<f32>,
    prev_gain: Vec<[f32; 2]>,
    /// Each track's send levels at the end of the last block, so sends ramp too.
    prev_sends: Vec<[f32; MAX_RETURNS]>,
    prev_returns: Vec<[f32; 2]>,
    prev_master: f32,
    metronome: Metronome,
    sample_rate: f64,
    last_beat: i64,
    preview: Option<Arc<AudioBuffer>>,
    preview_pos: usize,
    /// Frames of silence before the preview starts (to land on the beat), and whether it loops.
    preview_delay: usize,
    preview_loop: bool,
    monitor: Option<Box<MonitorFeed>>,
    /// The safety limiter's gain now, and how fast it comes back up per sample.
    safety_gain: f32,
    safety_release: f32,
    /// Where the timeline continues if nothing jumps; `None` while stopped.
    expected: Option<u64>,
    /// The transport's tempo throughout, for graphs without a tempo track.
    fallback: TempoMap,
}

impl Renderer {
    pub fn new(transport: Arc<Transport>, sample_rate: u32, links: Links) -> Self {
        Self {
            expected: None,
            transport,
            graph: Box::new(Graph::empty()),
            links,
            master: [vec![0.0; MAX_BLOCK], vec![0.0; MAX_BLOCK]],
            click: vec![0.0; MAX_BLOCK],
            prev_gain: vec![[0.0; 2]; MAX_TRACKS],
            prev_sends: vec![[0.0; MAX_RETURNS]; MAX_TRACKS],
            prev_returns: vec![[0.0; 2]; MAX_RETURNS],
            prev_master: 1.0,
            metronome: Metronome::new(sample_rate as f32),
            sample_rate: f64::from(sample_rate),
            last_beat: -1,
            preview: None,
            preview_pos: 0,
            preview_delay: 0,
            preview_loop: false,
            monitor: None,
            safety_gain: 1.0,
            safety_release: 1.0 - (-1.0 / (0.1 * f64::from(sample_rate.max(1)))).exp() as f32,
            fallback: TempoMap::constant(120.0, f64::from(sample_rate)),
        }
    }

    /// The song's tempo track, or the transport's tempo throughout.
    fn tempo(&self) -> &TempoMap {
        self.graph.tempo.as_ref().unwrap_or(&self.fallback)
    }

    /// Fills an interleaved device buffer.
    pub fn process<T>(&mut self, out: &mut [T], channels: usize)
    where
        T: SizedSample + FromSample<f32>,
    {
        self.take_new_graphs();
        self.take_controls();
        self.take_live();
        let channels = channels.max(1);
        let frames = out.len() / channels;
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(MAX_BLOCK);
            self.render_block(n);
            if let Some(safety) = self.links.safety.as_deref() {
                let [left, right] = &mut self.master;
                protect(&mut left[..n], &mut right[..n], &mut self.safety_gain, self.safety_release, safety);
            }
            let [left, right] = &self.master;
            for i in 0..n {
                let frame = &mut out[(done + i) * channels..(done + i + 1) * channels];
                if channels == 1 {
                    frame[0] = T::from_sample((left[i] + right[i]) * 0.5);
                } else {
                    frame[0] = T::from_sample(left[i]);
                    frame[1] = T::from_sample(right[i]);
                    frame[2..].fill(T::from_sample(0.0));
                }
            }
            done += n;
        }
    }

    fn take_new_graphs(&mut self) {
        while let Ok(mut next) = self.links.graphs.pop() {
            // Instruments carry on: the new one adopts the voices of the old,
            // so a project change never cuts off a note.
            let playing = self.expected.is_some();
            let pos = self.transport.position_samples();
            for track in &mut next.tracks {
                let Some(old) = self.graph.tracks.iter_mut().find(|t| t.key == track.key) else { continue };
                // Effects keep their tails and envelopes too, warped clips their
                // grains, delay compensation what it holds.
                adopt_chain(&mut track.fx, &mut old.fx);
                track.comp.adopt(&mut old.comp);
                for clip in track.clips.iter_mut().filter(|c| c.key != 0) {
                    let was = old.clips.iter().find(|c| c.key == clip.key).and_then(|c| c.warp.as_ref());
                    if let (Some(warp), Some(was)) = (clip.warp.as_mut(), was) {
                        warp.warper.adopt(&was.warper);
                    }
                }
                // The session carries on; a clip deleted while it plays stops.
                let launch = &mut track.launch;
                *launch = old.launch;
                if launch.slot(&track.slots).is_none() {
                    launch.playing = None;
                }
                let (Some(new), Some(old)) = (track.instrument.as_mut(), old.instrument.as_mut()) else { continue };
                if old.id == new.id {
                    new.adopt(old);
                    if playing {
                        match launch.slot(&track.slots) {
                            _ if launch.arrangement => new.release_orphans(pos),
                            Some(SlotNode { content: SlotContent::Midi { events, longest }, len, .. }) => {
                                new.release_orphans_in(events, *longest, launch.phase % len);
                            }
                            _ => new.release_clip_notes(),
                        }
                    }
                }
            }
            for ret in &mut next.returns {
                if let Some(old) = self.graph.returns.iter_mut().find(|r| r.key == ret.key) {
                    adopt_chain(&mut ret.fx, &mut old.fx);
                    ret.comp.adopt(&mut old.comp);
                }
            }
            next.dry_comp.adopt(&mut self.graph.dry_comp);
            adopt_chain(&mut next.master_fx, &mut self.graph.master_fx);
            let old = std::mem::replace(&mut self.graph, next);
            // Only fails if the control thread stopped draining; then freeing
            // here is the lesser evil.
            let _ = self.links.garbage.push(Garbage::Graph(old));
        }
    }

    fn take_live(&mut self) {
        while let Ok(LiveEvent { track, event }) = self.links.live.pop() {
            for t in self.graph.tracks.iter_mut().filter(|t| track == ALL_TRACKS || t.key == track) {
                if let Some(node) = t.instrument.as_mut() {
                    // Through the track's MIDI effects, at the start of the next block.
                    node.post(event);
                }
            }
        }
    }

    fn take_controls(&mut self) {
        while let Ok(message) = self.links.controls.pop() {
            match message {
                Control::Preview(buffer, mode) => {
                    if let Some(old) = self.preview.replace(buffer) {
                        let _ = self.links.garbage.push(Garbage::Preview(old));
                    }
                    self.preview_pos = 0;
                    self.preview_loop = mode.looping;
                    // While the song plays, it starts on the next line of the grid.
                    self.preview_delay = match self.expected {
                        Some(pos) if mode.grid > 0.0 => (next_line(pos, mode.grid, self.tempo()) - pos) as usize,
                        _ => 0,
                    };
                    self.links.previewing.store(true, Ordering::Relaxed);
                }
                Control::StopPreview => self.end_preview(),
                Control::Tap(tap) => {
                    // The old tap (wherever it is) goes; the new one goes on its device, if that plays.
                    for node in self.graph.fx_nodes_mut() {
                        if let Some(old) = node.tap.take() {
                            let _ = self.links.garbage.push(Garbage::Tap(old));
                        }
                    }
                    if let Some(tap) = tap {
                        match self.graph.fx_nodes_mut().find(|n| n.id == tap.0) {
                            Some(node) => node.tap = Some(tap),
                            None => {
                                let _ = self.links.garbage.push(Garbage::Tap(tap));
                            }
                        }
                    }
                }
                Control::Monitor(mut feed) => {
                    if let Some(feed) = feed.as_mut() {
                        feed.set_output_rate(self.sample_rate);
                    }
                    if let Some(old) = std::mem::replace(&mut self.monitor, feed) {
                        let _ = self.links.garbage.push(Garbage::Monitor(old));
                    }
                }
                Control::Launch { track, scene, grid } => {
                    let wait = self.wait(grid);
                    if let Some(t) = self.graph.tracks.iter_mut().find(|t| t.key == track) {
                        t.launch.queue(scene, wait);
                        if scene.is_some() {
                            self.start_for_launch();
                        }
                    }
                }
                Control::LaunchScene { scene, grid } => {
                    let wait = self.wait(grid);
                    self.start_for_launch();
                    // Group tracks have no clips: they play what their tracks send them.
                    for t in self.graph.tracks.iter_mut().filter(|t| t.group.is_none()) {
                        let has = t.slots.iter().any(|s| s.scene == scene);
                        t.launch.queue(has.then_some(scene), wait);
                    }
                }
                Control::StopClips { grid } => {
                    let wait = self.wait(grid);
                    for t in self.graph.tracks.iter_mut().filter(|t| t.launch.playing.is_some() || t.launch.queued.is_some()) {
                        t.launch.queue(None, wait);
                    }
                }
                Control::BackToArrangement => {
                    let at = self.expected.unwrap_or_else(|| self.transport.position_samples());
                    for t in self.graph.tracks.iter_mut().filter(|t| !t.launch.arrangement) {
                        t.launch.back_to_arrangement();
                        if let Some(node) = t.instrument.as_mut() {
                            node.release_clip_notes();
                        }
                        let _ = self.links.launched.push(Launched { track: t.key, scene: None, at });
                    }
                }
            }
        }
    }

    /// Launching starts the transport. Starting it here, with the launch
    /// already queued, means the clip starts on the first sample played.
    fn start_for_launch(&self) {
        if !self.transport.is_playing() {
            self.transport.set_playing(true);
        }
    }

    /// When a launch made now takes effect: the next line of a grid `beats`
    /// apart, or the first sample played if the transport is stopped.
    fn wait(&self, beats: f64) -> Wait {
        match self.expected {
            None => Wait::Start,
            Some(pos) => {
                let grid = beats.max(0.0);
                Wait::At { at: next_line(pos, grid, self.tempo()), grid }
            }
        }
    }

    fn end_preview(&mut self) {
        if let Some(old) = self.preview.take() {
            let _ = self.links.garbage.push(Garbage::Preview(old));
        }
        self.links.previewing.store(false, Ordering::Relaxed);
    }

    /// Renders `n` (≤ MAX_BLOCK) frames into the master buffers.
    pub fn render_block(&mut self, n: usize) {
        let n = n.min(MAX_BLOCK);
        self.fallback.set_constant(self.transport.bpm(), self.sample_rate);
        self.master[0][..n].fill(0.0);
        self.master[1][..n].fill(0.0);
        self.click[..n].fill(0.0);
        let monitoring = self.pull_monitor(n);

        if self.transport.is_playing() {
            let sr = self.sample_rate;
            let start = self.transport.position_samples();
            let (loop_on, loop_start, loop_end) = self.transport.loop_range();
            let (ls, le) = ((loop_start * sr).round() as u64, (loop_end * sr).round() as u64);
            let looping = loop_on && le > ls;

            // Count-in first: clicks only, the playhead waits.
            let mut written = self.render_count_in(n);
            if written > 0 {
                // Players hear themselves during the count-in too.
                self.mix_tracks(0, written, 0, false);
            }
            let mut pos = start;
            while written < n {
                let mut seg = n - written;
                if looping && pos < le {
                    seg = seg.min((le - pos) as usize);
                }
                self.mix_tracks(pos, seg, written, true);
                self.render_clicks(pos, seg, written);
                let before = pos;
                pos += seg as u64;
                if looping && before < le && pos >= le {
                    pos = ls;
                }
                written += seg;
            }
            self.transport.advance(start, pos);
        } else {
            self.last_beat = -1;
            // Stopping the transport stops the session's clips.
            if self.expected.is_some() {
                for track in &mut self.graph.tracks {
                    track.launch.halt();
                }
            }
            // Let a click that is still ringing finish instead of cutting it off.
            for c in &mut self.click[..n] {
                *c = self.metronome.tick();
            }
            // Live notes, ringing instruments and monitored input still play.
            self.mix_tracks(0, n, 0, false);
            // Playback restarts with a one-block fade-in instead of a click;
            // tracks playing their input, an instrument or an effect's tail
            // keep their level.
            for (gain, track) in self.prev_gain.iter_mut().zip(&self.graph.tracks) {
                if !(monitoring && track.monitor.is_some()) && track.instrument.is_none() && track.fx.is_empty() {
                    *gain = [0.0, 0.0];
                }
            }
        }

        for track in &self.graph.tracks {
            track.status.publish(&track.launch, &track.slots);
        }
        self.finish_master(n);
    }

    /// Reads one block of monitored input, at the output's rate and clock.
    /// Returns whether any track plays it.
    fn pull_monitor(&mut self, n: usize) -> bool {
        let Some(feed) = self.monitor.as_mut() else { return false };
        if !self.graph.tracks.iter().any(|t| t.monitor.is_some()) {
            feed.idle();
            return false;
        }
        feed.render(n);
        true
    }

    /// Renders up to `n` frames of count-in clicks; returns how many it used.
    fn render_count_in(&mut self, n: usize) -> usize {
        let (total, left) = self.transport.count_in();
        if left == 0 {
            return 0;
        }
        // The count-in plays at the tempo where recording starts.
        let samples_per_beat = self.tempo().samples_per_beat_at(self.transport.position_samples() as f64);
        let beats_per_bar = i64::from(self.transport.beats_per_bar());
        let count = (left as usize).min(n);
        for i in 0..count {
            let elapsed = (total - left) as f64 + i as f64;
            let beat = (elapsed / samples_per_beat).floor() as i64;
            if elapsed - beat as f64 * samples_per_beat < 1.0 {
                self.metronome.trigger(beat.rem_euclid(beats_per_bar) == 0);
            }
            self.click[i] = self.metronome.tick();
        }
        self.transport.consume_count_in(count as u64);
        // The song's first beat after the count-in must click again.
        self.last_beat = -1;
        count
    }

    /// Settles every gain ramp at its target, so offline renders start at
    /// full level instead of fading in.
    pub fn prime(&mut self) {
        self.take_new_graphs();
        for ((gain, sends), track) in self.prev_gain.iter_mut().zip(self.prev_sends.iter_mut()).zip(&self.graph.tracks) {
            *gain = track.gain;
            for (slot, level) in sends.iter_mut().zip(&track.sends) {
                *slot = *level;
            }
        }
        for (slot, ret) in self.prev_returns.iter_mut().zip(&self.graph.returns) {
            *slot = ret.gain;
        }
        self.prev_master = self.graph.master_gain;
    }

    /// Mixes every track into the master for `n` frames from timeline
    /// position `pos`, writing at `offset` in the block. Without `clips`,
    /// the timeline is silent (the transport is stopped): live notes,
    /// monitored input and effect tails still play. When automation or a
    /// modulator moves anything, the work goes in steps of [`CONTROL`]
    /// samples, each starting with the values of its moment.
    fn mix_tracks(&mut self, pos: u64, n: usize, offset: usize, clips: bool) {
        if !self.graph.tracks.iter().any(TrackNode::controlled) {
            return self.mix_chunk(pos, n, offset, clips);
        }
        // Stopped, automation shows the values at the playhead.
        let here = if clips { pos } else { self.transport.position_samples() };
        let mut done = 0;
        while done < n {
            let m = (n - done).min(CONTROL);
            let at = here + if clips { done as u64 } else { 0 };
            let Graph { tracks, tempo, .. } = &mut *self.graph;
            let tempo = tempo.as_ref().unwrap_or(&self.fallback);
            for track in tracks.iter_mut().filter(|t| t.controlled()) {
                track.apply_controls(at, m, clips, tempo, self.sample_rate);
            }
            self.mix_chunk(pos + done as u64, m, offset + done, clips);
            done += m;
        }
    }

    fn mix_chunk(&mut self, pos: u64, n: usize, offset: usize, clips: bool) {
        // The timeline jumped (seek, loop, start or stop): notes the clips
        // started are released before anything new plays.
        let jumped = !clips || self.expected != Some(pos);
        self.expected = clips.then_some(pos + n as u64);
        let Self { graph, master, prev_gain, prev_sends, prev_returns, monitor, links, fallback, sample_rate, .. } = self;
        let Links { workers, launched, .. } = links;
        let Graph { tracks, returns, master_fx, groups, tempo, dry, dry_comp, .. } = &mut **graph;
        let stretch = Stretch {
            pos,
            n,
            offset,
            clips,
            jumped,
            tempo: tempo.as_ref().unwrap_or(fallback),
            sample_rate: *sample_rate as f32,
            input: monitor.as_deref().map(|feed| &feed.input[..]),
        };
        let ctx = stretch.at(pos);
        let [master_l, master_r] = master;
        for [gl, gr] in groups.iter_mut() {
            gl[..n].fill(0.0);
            gr[..n].fill(0.0);
        }
        for [il, ir] in returns.iter_mut().map(|r| &mut r.input) {
            il[..n].fill(0.0);
            ir[..n].fill(0.0);
        }
        let [dry_l, dry_r] = dry;
        dry_l[..n].fill(0.0);
        dry_r[..n].fill(0.0);
        let run = |jobs: usize, job: &(dyn Fn(usize) + Sync)| match workers.as_deref() {
            Some(w) => w.run(jobs, job),
            None => (0..jobs).for_each(job),
        };
        let all = Tracks(tracks.as_mut_ptr(), tracks.len());
        let sidechains = tracks.iter().any(|t| t.fx.iter().any(|f| f.sidechain.is_some()));

        // 1. Each track's own sound (the arrangement or its session clip,
        // instrument, input): what sidechains hear. When no effect listens to
        // another track, the effects come in the same job.
        run(all.1, &|i| {
            // SAFETY: each job has its own track; nothing else touches the tracks meanwhile.
            own_sound(unsafe { &mut *all.0.add(i) }, &stretch);
            if !sidechains && unsafe { (*all.0.add(i)).group.is_none() } {
                // SAFETY: as above: nothing reads another track's sound.
                unsafe { effects(all, i, &stretch) };
            }
        });
        for track in tracks.iter_mut() {
            for at in track.switched.drain(..) {
                let _ = launched.push(at);
            }
        }

        // 2. The tracks' effects (once every track has its own sound), then
        // in order their faders and sends; group tracks last, once every
        // track in them has played into them.
        if sidechains {
            run(all.1, &|i| {
                // SAFETY: each job changes only its own track's effects and
                // output; the tracks' own sound is only read.
                if unsafe { (*all.0.add(i)).group.is_none() } {
                    unsafe { effects(all, i, &stretch) };
                }
            });
        }
        for (ti, track) in tracks.iter().enumerate().filter(|(_, t)| t.group.is_none()) {
            play_out(ti, track, n, groups, [&mut dry_l[..], &mut dry_r[..]], returns, prev_gain, prev_sends);
        }
        let inputs = &groups[..];
        let group_job = |i: usize| {
            // SAFETY: as for the other tracks. A group track writes its own
            // sound here, so groups run one at a time when one of them
            // listens to another track.
            let t = unsafe { all.0.add(i) };
            let Some(input) = unsafe { (*t).group }.and_then(|g| inputs.get(g)) else { return };
            let raw = unsafe { &mut (*t).raw };
            raw[0][..n].copy_from_slice(&input[0][..n]);
            raw[1][..n].copy_from_slice(&input[1][..n]);
            unsafe { effects(all, i, &stretch) };
        };
        if tracks.iter().any(|t| t.group.is_some() && t.fx.iter().any(|f| f.sidechain.is_some())) {
            (0..all.1).for_each(group_job);
        } else {
            run(all.1, &group_job);
        }
        for (ti, track) in tracks.iter().enumerate().filter(|(_, t)| t.group.is_some()) {
            play_out(ti, track, n, groups, [&mut dry_l[..], &mut dry_r[..]], returns, prev_gain, prev_sends);
        }

        // 3. The returns: their effects and faders.
        let ramp = |i: usize, from: f32, to: f32| from + (to - from) * (i + 1) as f32 / n as f32;
        for (ri, ret) in returns.iter_mut().enumerate() {
            let [il, ir] = &mut ret.input;
            for node in ret.fx.iter_mut() {
                let key = node.sidechain.and_then(|s| tracks.get(s)).map(|t| (&t.raw[0][..n], &t.raw[1][..n]));
                node.fx.process(&mut il[..n], &mut ir[..n], key, &ctx);
                node.after(&il[..n], &ir[..n]);
            }
            ret.comp.process(&mut il[..n], &mut ir[..n]);
            let target = ret.gain;
            let prev = prev_returns.get(ri).copied().unwrap_or(target);
            let (mut peak_l, mut peak_r) = (0.0_f32, 0.0_f32);
            for i in 0..n {
                let l = il[i] * ramp(i, prev[0], target[0]);
                let r = ir[i] * ramp(i, prev[1], target[1]);
                peak_l = peak_l.max(l.abs());
                peak_r = peak_r.max(r.abs());
                master_l[offset + i] += l;
                master_r[offset + i] += r;
            }
            if let Some(slot) = prev_returns.get_mut(ri) {
                *slot = target;
            }
            ret.meter.update(peak_l, peak_r);
        }

        // The dry mix, held back as long as the slowest return.
        dry_comp.process(&mut dry_l[..n], &mut dry_r[..n]);
        for i in 0..n {
            master_l[offset + i] += dry_l[i];
            master_r[offset + i] += dry_r[i];
        }

        // 4. The master's effects, on this stretch of the mix.
        let (ml, mr) = (&mut master_l[offset..offset + n], &mut master_r[offset..offset + n]);
        for node in master_fx.iter_mut() {
            let key = node.sidechain.and_then(|s| tracks.get(s)).map(|t| (&t.raw[0][..n], &t.raw[1][..n]));
            node.fx.process(ml, mr, key, &ctx);
            node.after(ml, mr);
        }
    }

    fn render_clicks(&mut self, pos: u64, n: usize, offset: usize) {
        let beats_per_bar = i64::from(self.transport.beats_per_bar());
        let enabled = self.transport.metronome_enabled();
        // The beat lines in this stretch, found from the tempo track once per beat.
        let mut beat = self.tempo().beat_at(pos as f64).floor() as i64;
        let mut line = self.tempo().sample_at(beat as f64);
        let mut next = self.tempo().sample_at((beat + 1) as f64);
        for i in 0..n {
            let p = pos + i as u64;
            if p as f64 >= next {
                beat += 1;
                line = next;
                next = self.tempo().sample_at((beat + 1) as f64);
            }
            if beat != self.last_beat {
                // Only click on the exact boundary, so seeking mid-beat stays silent.
                if enabled && p as f64 - line < 1.0 {
                    self.metronome.trigger(beat.rem_euclid(beats_per_bar) == 0);
                }
                self.last_beat = beat;
            }
            self.click[offset + i] = self.metronome.tick();
        }
    }

    fn finish_master(&mut self, n: usize) {
        let target = self.graph.master_gain;
        let prev = self.prev_master;
        let [left, right] = &mut self.master;
        // A preview plays at its own level, on top of the mix.
        let preview = self.preview.as_ref().map(|p| {
            let l = p.channels.first().map_or(&[][..], Vec::as_slice);
            (l, p.channels.get(1).map_or(l, Vec::as_slice))
        });
        let (delay, looping) = (self.preview_delay, self.preview_loop);
        let (mut peak_l, mut peak_r) = (0.0_f32, 0.0_f32);
        // Only whole blocks go to the analysis: a full queue skips one.
        let analysing = self.links.analysing.load(Ordering::Relaxed);
        let mut tap = self.links.analysis.as_mut().filter(|t| analysing && t.slots() >= 2 * n);
        for i in 0..n {
            let g = prev + (target - prev) * (i + 1) as f32 / n as f32;
            left[i] *= g;
            right[i] *= g;
            if let Some((pl, pr)) = preview
                && let Some(mut at) = (self.preview_pos + i).checked_sub(delay)
            {
                if looping && !pl.is_empty() {
                    at %= pl.len();
                }
                left[i] += pl.get(at).copied().unwrap_or(0.0);
                right[i] += pr.get(at).copied().unwrap_or(0.0);
            }
            peak_l = peak_l.max(left[i].abs());
            peak_r = peak_r.max(right[i].abs());
            if let Some(tap) = tap.as_mut() {
                let _ = tap.push(left[i]);
                let _ = tap.push(right[i]);
            }
            // The click is a cue: it ignores the master fader and the meter.
            left[i] += self.click[i];
            right[i] += self.click[i];
        }
        self.prev_master = target;
        self.links.master_meter.update(peak_l, peak_r);
        if let Some((pl, _)) = preview {
            self.preview_pos += n;
            if !looping && self.preview_pos >= delay + pl.len() {
                self.end_preview();
            }
        }
    }

    /// The last rendered block, for tests and offline rendering.
    pub fn output(&self, n: usize) -> (&[f32], &[f32]) {
        (&self.master[0][..n], &self.master[1][..n])
    }
}

/// What every track's work in one stretch shares, read from any thread.
struct Stretch<'a> {
    pos: u64,
    n: usize,
    /// Where the stretch starts in the block (for the monitored input).
    offset: usize,
    /// The timeline plays (otherwise only live notes, input and tails).
    clips: bool,
    jumped: bool,
    tempo: &'a TempoMap,
    sample_rate: f32,
    /// This block's monitored input, by channel.
    input: Option<&'a [Vec<f32>]>,
}

impl Stretch<'_> {
    /// Where `pos` is in the song, for effects that follow the tempo.
    fn at(&self, pos: u64) -> FxContext {
        FxContext {
            playing: self.clips,
            pos,
            beat: self.tempo.beat_at(pos as f64),
            samples_per_beat: self.tempo.samples_per_beat_at(pos as f64),
            sample_rate: self.sample_rate,
        }
    }
}

/// The graph's tracks, for jobs on several threads. A job changes only its
/// own track; other tracks' own sound (`raw`) it may read, for sidechains.
#[derive(Clone, Copy)]
struct Tracks(*mut TrackNode, usize);

// SAFETY: tracks are `Send`, and the jobs keep to the rule above.
unsafe impl Send for Tracks {}
unsafe impl Sync for Tracks {}

/// A track's own sound for the stretch: its arrangement or session clip,
/// its instrument and its monitored input.
fn own_sound(track: &mut TrackNode, s: &Stretch) {
    let n = s.n;
    let TrackNode { raw: [sl, sr], clips: timeline, instrument, slots, launch, key, monitor: input, mods, switched, .. } = track;
    sl[..n].fill(0.0);
    sr[..n].fill(0.0);
    // A jump releases the notes the arrangement started; session clips play on.
    if s.jumped
        && (launch.arrangement || !s.clips)
        && let Some(node) = instrument.as_mut()
    {
        node.release_clip_notes();
    }
    if s.jumped && s.clips {
        launch.rejoin(s.pos, s.tempo);
    }
    // Session clips loop at the song's starting tempo.
    let loop_beat = s.tempo.samples_per_beat_at(0.0);

    // In stretches, split where a launch or follow action is due.
    let mut done = 0;
    while done < n {
        let here = s.pos + done as u64;
        let seg = if s.clips { launch.until_switch(here, n - done, slots) } else { n - done };
        if seg > 0 {
            let (l, r) = (&mut sl[done..done + seg], &mut sr[done..done + seg]);
            let in_session = s.clips && !launch.arrangement;
            if s.clips && launch.arrangement {
                for clip in timeline.iter_mut() {
                    mix_clip(clip, here, l, r, Timing::Timeline(s.tempo));
                }
            }
            if in_session
                && let Some(scene) = launch.playing
                && let Some(SlotNode { content: SlotContent::Audio(clip), .. }) = slots.iter_mut().find(|slot| slot.scene == scene)
            {
                loop_clip(clip, launch.phase, l, r, loop_beat);
            }
            if let Some(node) = instrument.as_mut() {
                let slot = launch.slot(slots).filter(|_| in_session);
                let notes = match slot {
                    _ if s.clips && launch.arrangement => Notes::Timeline,
                    Some(SlotNode { content: SlotContent::Midi { events, .. }, len, .. }) => Notes::Loop { events, len: *len, phase: launch.phase },
                    _ => Notes::Live,
                };
                node.render(&s.at(here), l, r, notes);
            }
            if s.clips && launch.playing.is_some() {
                launch.phase += seg as u64;
            }
        }
        if seg < n - done {
            let at = here + seg as u64;
            launch.switch(at, slots);
            if let Some(node) = instrument.as_mut() {
                node.release_clip_notes();
            }
            // Room was made when the graph was built: this never allocates.
            if switched.len() < switched.capacity() {
                switched.push(Launched { track: *key, scene: launch.playing, at });
            }
        }
        done += seg;
    }

    if let (Some((l, r)), Some(feed)) = (*input, s.input) {
        let first = &feed[0];
        let (il, ir) = (feed.get(l).unwrap_or(first), feed.get(r).unwrap_or(first));
        for i in 0..n {
            sl[i] += il[s.offset + i];
            sr[i] += ir[s.offset + i];
        }
    }
    // Envelope followers hear this, and move their parameter in the next step.
    for m in mods.iter_mut() {
        m.follow(&sl[..n], &sr[..n]);
    }
}

/// Track `i`'s effects and delay compensation, from its own sound into its output.
///
/// # Safety
/// While this runs, no other thread touches track `i`'s output, effects or
/// delay compensation, and nothing changes any track's own sound.
unsafe fn effects(tracks: Tracks, i: usize, s: &Stretch) {
    let n = s.n;
    // SAFETY: fields reached through the pointer, never the whole track, so
    // other threads may read the other tracks' own sound meanwhile.
    unsafe {
        let t = tracks.0.add(i);
        let raw = &(*t).raw;
        let [ol, or] = &mut (*t).out;
        ol[..n].copy_from_slice(&raw[0][..n]);
        or[..n].copy_from_slice(&raw[1][..n]);
        let ctx = s.at(s.pos);
        for node in (*t).fx.iter_mut() {
            let key = node.sidechain.filter(|&k| k != i && k < tracks.1).map(|k| {
                let key = &(*tracks.0.add(k)).raw;
                (&key[0][..n], &key[1][..n])
            });
            node.fx.process(&mut ol[..n], &mut or[..n], key, &ctx);
            node.after(&ol[..n], &or[..n]);
        }
        // Held back to come out with the slowest track.
        (*t).comp.process(&mut ol[..n], &mut or[..n]);
    }
}

/// Plays a track's processed sound through its fader into its group or the
/// dry mix, and into the returns. Gains ramp from the last block's, so fader
/// moves never click.
#[allow(clippy::too_many_arguments)]
fn play_out(
    ti: usize,
    track: &TrackNode,
    n: usize,
    groups: &mut [[Vec<f32>; 2]],
    dry: [&mut [f32]; 2],
    returns: &mut [ReturnNode],
    prev_gain: &mut [[f32; 2]],
    prev_sends: &mut [[f32; MAX_RETURNS]],
) {
    let ramp = |i: usize, from: f32, to: f32| from + (to - from) * (i + 1) as f32 / n as f32;
    let TrackNode { out: [ol, or], gain, sends, meter, output, .. } = track;
    let target = *gain;
    let prev = prev_gain.get(ti).copied().unwrap_or(target);
    let (mut peak_l, mut peak_r) = (0.0_f32, 0.0_f32);
    let [dry_l, dry_r] = dry;
    let (dest_l, dest_r) = match output.and_then(|g| groups.get_mut(g)) {
        Some([gl, gr]) => (&mut gl[..n], &mut gr[..n]),
        None => (&mut dry_l[..n], &mut dry_r[..n]),
    };
    for i in 0..n {
        let l = ol[i] * ramp(i, prev[0], target[0]);
        let r = or[i] * ramp(i, prev[1], target[1]);
        peak_l = peak_l.max(l.abs());
        peak_r = peak_r.max(r.abs());
        dest_l[i] += l;
        dest_r[i] += r;
    }
    if let Some(slot) = prev_gain.get_mut(ti) {
        *slot = target;
    }
    meter.update(peak_l, peak_r);

    // Sends: before the fader or after it, as each return asks.
    let mut was = prev_sends.get(ti).copied().unwrap_or([0.0; MAX_RETURNS]);
    for (ri, ret) in returns.iter_mut().enumerate().take(MAX_RETURNS) {
        let level = sends.get(ri).copied().unwrap_or(0.0);
        let from = was[ri];
        was[ri] = level;
        if level == 0.0 && from == 0.0 {
            continue;
        }
        let [il, ir] = &mut ret.input;
        for i in 0..n {
            let s = ramp(i, from, level);
            let (gl, gr) = if ret.pre_fader { (1.0, 1.0) } else { (ramp(i, prev[0], target[0]), ramp(i, prev[1], target[1])) };
            il[i] += ol[i] * gl * s;
            ir[i] += or[i] * gr * s;
        }
    }
    if let Some(slot) = prev_sends.get_mut(ti) {
        *slot = was;
    }
}

/// A clip's level over its length: gain, fades and crossfades.
#[derive(Clone, Copy)]
struct Envelope {
    gain: f32,
    len: u64,
    fade_in: u64,
    fade_out: u64,
    xfade_in: u64,
    xfade_out: u64,
    curve: FadeShape,
}

impl Envelope {
    fn of(clip: &ClipNode) -> Self {
        Self {
            gain: clip.gain,
            len: clip.len,
            fade_in: clip.fade_in,
            fade_out: clip.fade_out,
            xfade_in: clip.xfade_in,
            xfade_out: clip.xfade_out,
            curve: clip.curve,
        }
    }

    fn flat(&self) -> bool {
        self.fade_in == 0 && self.fade_out == 0 && self.xfade_in == 0 && self.xfade_out == 0
    }

    /// The gain `at` samples into the clip.
    fn at(&self, at: u64) -> f32 {
        let mut g = self.gain;
        if self.flat() {
            return g;
        }
        let to_end = self.len - at;
        let shape = self.curve;
        if at < self.fade_in {
            g *= shape.gain(at as f32 / self.fade_in as f32, false);
        }
        if to_end < self.fade_out {
            g *= shape.gain(to_end as f32 / self.fade_out as f32, false);
        }
        // Standard crossfades are equal power, so the overlap keeps its loudness.
        if at < self.xfade_in {
            g *= shape.gain(at as f32 / self.xfade_in as f32, true);
        }
        if to_end < self.xfade_out {
            g *= shape.gain(to_end as f32 / self.xfade_out as f32, true);
        }
        g
    }
}

/// Adds what `clip` plays in the stretch of `out_l.len()` samples from
/// position `pos` (on the timeline, or into a session clip's loop), with
/// its gain and fades. A warped clip reads its audio through its warper.
fn mix_clip(clip: &mut ClipNode, pos: u64, out_l: &mut [f32], out_r: &mut [f32], timing: Timing<'_>) {
    let s = pos.max(clip.start);
    let e = (pos + out_l.len() as u64).min(clip.start + clip.len);
    if s >= e {
        return;
    }
    let Some(left) = clip.buffer.channels.first() else { return };
    let right = clip.buffer.channels.get(1).unwrap_or(left);
    let within = s - clip.start;
    let dst = (s - pos) as usize;
    let env = Envelope::of(clip);
    if let Some(warp) = clip.warp.as_deref_mut() {
        let count = (e - s) as usize;
        let clock = ClipClock { timing, start_beat: warp.start_beat, per_beat: warp.per_beat, offset: clip.offset as f64 };
        let ClipWarp { warper, left: wl, right: wr, .. } = warp;
        warper.render(&clip.buffer, &clock, s, &mut wl[..count], &mut wr[..count]);
        for i in 0..count {
            let g = env.at(within + i as u64);
            out_l[dst + i] += wl[i] * g;
            out_r[dst + i] += wr[i] * g;
        }
        return;
    }
    let src = (clip.offset + within) as usize;
    let count = ((e - s) as usize).min(left.len().saturating_sub(src));
    for i in 0..count {
        let g = env.at(within + i as u64);
        out_l[dst + i] += left[src + i] * g;
        out_r[dst + i] += right[src + i] * g;
    }
}

/// Adds a looping session clip (starting at 0), `phase` samples into its
/// playing; a warped one keeps `samples_per_beat` (the song's starting tempo).
fn loop_clip(clip: &mut ClipNode, phase: u64, out_l: &mut [f32], out_r: &mut [f32], samples_per_beat: f64) {
    let len = clip.len.max(1);
    let mut done = 0;
    while done < out_l.len() {
        let at = (phase + done as u64) % len;
        let count = ((len - at) as usize).min(out_l.len() - done);
        mix_clip(clip, at, &mut out_l[done..done + count], &mut out_r[done..done + count], Timing::Loop { samples_per_beat });
        done += count;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{ClipNode, InstrumentNode, ScheduledNote, TrackNode, crossfade_overlaps, track_gains};

    fn setup(graph: Graph) -> (Renderer, Arc<Transport>) {
        let transport = Arc::new(Transport::new());
        transport.set_sample_rate(1000);
        let mut renderer = Renderer::new(Arc::clone(&transport), 1000, Links::detached(graph));
        renderer.take_new_graphs();
        (renderer, transport)
    }

    fn constant_clip(start: u64, len: u64, value: f32) -> ClipNode {
        ClipNode::new(start, len, 0, Arc::new(AudioBuffer::new(1000, vec![vec![value; len as usize]])))
    }

    #[test]
    fn clip_fades_ramp_in_and_out() {
        let mut clip = constant_clip(0, 100, 1.0);
        clip.fade_in = 10;
        clip.fade_out = 20;
        let (mut r, t) = setup(graph_with(vec![clip], [1.0, 1.0]));
        r.prime();
        t.set_playing(true);
        r.render_block(100);
        let (left, _) = r.output(100);
        assert_eq!(left[0], 0.0);
        assert!((left[5] - 0.5).abs() < 1e-6);
        assert_eq!(left[50], 1.0);
        assert!((left[90] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn overlapping_clips_crossfade_with_equal_power() {
        let mut clips = vec![constant_clip(0, 100, 1.0), constant_clip(60, 100, 1.0)];
        crossfade_overlaps(&mut clips);
        assert_eq!((clips[0].xfade_out, clips[1].xfade_in), (40, 40));
        let (mut r, t) = setup(graph_with(clips, [1.0, 1.0]));
        r.prime();
        t.set_playing(true);
        r.render_block(160);
        let (left, _) = r.output(160);
        // Mid-overlap both clips sit at sin(45°): the sum is √2 in amplitude, equal in power.
        assert!((left[80] - 2f32.sqrt()).abs() < 0.02, "{}", left[80]);
        assert_eq!(left[50], 1.0);
        assert_eq!(left[120], 1.0);
    }

    #[test]
    fn a_clip_inside_another_is_layered_not_crossfaded() {
        let mut clips = vec![constant_clip(0, 100, 1.0), constant_clip(20, 30, 1.0)];
        crossfade_overlaps(&mut clips);
        assert_eq!((clips[0].xfade_out, clips[1].xfade_in), (0, 0));
    }

    #[test]
    fn preview_plays_once_over_the_stopped_transport() {
        let transport = Arc::new(Transport::new());
        transport.set_sample_rate(1000);
        let (mut controls_tx, controls) = rtrb::RingBuffer::new(4);
        let previewing = Arc::new(AtomicBool::new(false));
        let mut links = Links::detached(Graph::empty());
        links.controls = controls;
        links.previewing = Arc::clone(&previewing);
        let mut r = Renderer::new(transport, 1000, links);
        let _ = controls_tx.push(Control::Preview(Arc::new(AudioBuffer::new(1000, vec![vec![0.25; 150]])), PreviewMode::default()));
        let mut out = vec![0.0_f32; 200];
        r.process(&mut out, 2);
        assert!(previewing.load(Ordering::Relaxed));
        assert_eq!(out[0], 0.25);
        r.process(&mut out, 2);
        assert_eq!(out[99], 0.25, "frame 49 of the second block is frame 149 of the preview");
        assert_eq!(out[100], 0.0);
        assert!(!previewing.load(Ordering::Relaxed), "the preview ended");
    }

    #[test]
    fn an_audition_started_mid_beat_loops_from_the_next_beat() {
        let transport = Arc::new(Transport::new());
        transport.set_sample_rate(1000);
        transport.set_bpm(120.0);
        transport.set_playing(true);
        let (mut controls_tx, controls) = rtrb::RingBuffer::new(4);
        let mut links = Links::detached(Graph::empty());
        links.controls = controls;
        let mut r = Renderer::new(transport, 1000, links);
        // 120 BPM at 1000 Hz: a beat every 500 frames. Play the first 100.
        let mut out = vec![0.0_f32; 200];
        r.process(&mut out, 2);
        let _ = controls_tx.push(Control::Preview(Arc::new(AudioBuffer::new(1000, vec![vec![0.25; 300]])), PreviewMode { looping: true, grid: 1.0 }));
        let mut heard = Vec::new();
        for _ in 0..12 {
            r.process(&mut out, 2);
            heard.extend(out.iter().step_by(2).copied());
        }
        let first = heard.iter().position(|&x| x != 0.0).unwrap();
        assert_eq!(first + 100, 500, "it starts on beat 2");
        // It loops: still playing 300 frames after its first pass.
        assert_eq!(heard[first + 300], 0.25);
        assert_eq!(heard[first + 650], 0.25);
    }

    #[test]
    fn monitored_input_plays_while_stopped() {
        let transport = Arc::new(Transport::new());
        transport.set_sample_rate(1000);
        let (mut input_tx, input_rx) = rtrb::RingBuffer::new(256);
        let (mut controls_tx, controls) = rtrb::RingBuffer::new(4);
        let mut track = TrackNode::new([1.0, 1.0], vec![], Arc::new(PeakMeter::default()));
        track.monitor = Some((1, 1));
        let mut links = Links::detached(Graph::new(vec![track], 1.0));
        links.controls = controls;
        let mut r = Renderer::new(transport, 1000, links);
        r.take_new_graphs();
        r.prime();
        let _ = controls_tx.push(Control::Monitor(Some(Box::new(MonitorFeed::new(input_rx, 2, 1000)))));
        // Stereo input: channel 2 carries 0.5. It arrives in deliveries of 40
        // frames; the feed waits for a backlog that covers one before playing.
        let deliver = |tx: &mut rtrb::Producer<f32>| {
            for _ in 0..40 {
                let _ = tx.push(0.1);
                let _ = tx.push(0.5);
            }
        };
        let mut out = vec![0.0_f32; 16];
        deliver(&mut input_tx);
        r.process(&mut out, 2);
        assert_eq!(&out[..4], &[0.0; 4], "building the backlog");
        deliver(&mut input_tx);
        r.process(&mut out, 2);
        assert!(out.iter().all(|s| (s - 0.5).abs() < 1e-6), "{out:?}");
    }

    #[test]
    fn count_in_clicks_without_moving_the_playhead() {
        let (mut r, t) = setup(graph_with(vec![constant_clip(0, 2000, 0.25)], [1.0, 1.0]));
        r.prime();
        t.start_count_in(500);
        t.set_playing(true);
        r.render_block(400);
        assert_eq!(t.position_samples(), 0);
        // The count-in ends 100 frames into this block; the song starts there.
        r.render_block(300);
        assert_eq!(t.position_samples(), 200);
    }

    fn graph_with(clips: Vec<ClipNode>, gain: [f32; 2]) -> Graph {
        Graph::new(vec![TrackNode::new(gain, clips, Arc::new(PeakMeter::default()))], 1.0)
    }

    #[test]
    fn a_compressor_reports_its_reduction_and_a_tap_hears_its_output() {
        let mut graph = graph_with(vec![constant_clip(0, 4000, 0.9)], [1.0, 1.0]);
        let mut node = fx("c1", "comp", &[("thresh", -30.0), ("ratio", 8.0), ("attack", 0.1)]);
        let meter = Arc::new(PeakMeter::default());
        node.meter = Some(Arc::clone(&meter));
        let (producer, mut consumer) = rtrb::RingBuffer::<f32>::new(8192);
        node.tap = Some(Box::new(("c1".into(), producer)));
        graph.tracks[0].fx.push(node);
        let (mut r, t) = setup(graph);
        r.prime();
        t.set_playing(true);
        let mut out = vec![0.0_f32; 1000];
        r.process(&mut out, 2);
        // −1 dBFS in, threshold −30, ratio 8: about 25 dB of reduction.
        let db = meter.take()[0];
        assert!(db > 15.0, "{db}");
        assert_eq!(consumer.slots(), 500, "every frame of the block, mono");
        let first = consumer.pop().unwrap();
        assert!(first.abs() < 0.9, "after the compressor: {first}");
    }

    #[test]
    fn the_safety_limiter_keeps_the_speakers_under_the_ceiling() {
        let (mut r, t) = setup(graph_with(vec![constant_clip(0, 4000, 1.0)], [4.0, 4.0]));
        let safety = Arc::new(Safety::default());
        r.links.safety = Some(Arc::clone(&safety));
        r.prime();
        t.set_playing(true);
        let mut out = vec![0.0_f32; 2000];
        r.process(&mut out, 2);
        assert!(out.iter().all(|s| s.abs() <= CEILING + 1e-6), "never over −0.3 dBFS");
        assert!(out[1000..].iter().all(|s| s.abs() > 0.95), "but right up to it");
        let (reduction, silenced) = safety.take();
        assert!(reduction > 12.0 && !silenced, "{reduction}");
        // Switched off, the overs go through (the device clips them).
        safety.set_enabled(false);
        r.process(&mut out, 2);
        assert!(out.iter().any(|s| s.abs() > 3.0));
    }

    #[test]
    fn a_broken_block_is_silenced_and_offline_renders_are_not_limited() {
        let (mut r, t) = setup(graph_with(vec![constant_clip(0, 4000, f32::NAN)], [1.0, 1.0]));
        let safety = Arc::new(Safety::default());
        r.links.safety = Some(Arc::clone(&safety));
        r.prime();
        t.set_playing(true);
        let mut out = vec![1.0_f32; 200];
        r.process(&mut out, 2);
        assert!(out.iter().all(|s| *s == 0.0));
        assert!(safety.take().1, "it says so");
        // No safety on offline renders (exports keep what the mix does).
        let (mut r, t) = setup(graph_with(vec![constant_clip(0, 4000, 1.0)], [4.0, 4.0]));
        r.prime();
        t.set_playing(true);
        r.process(&mut out, 2);
        assert!(out.iter().any(|s| s.abs() > 3.0));
    }

    #[test]
    fn plays_a_clip_at_its_position() {
        let (mut r, t) = setup(graph_with(vec![constant_clip(10, 20, 0.5)], [1.0, 1.0]));
        t.set_playing(true);
        // The first block fades in from silence; measure the second pass.
        r.render_block(40);
        t.seek_seconds(0.0);
        r.render_block(40);
        let (left, _) = r.output(40);
        assert_eq!(left[9], 0.0);
        assert_eq!(left[10], 0.5);
        assert_eq!(left[29], 0.5);
        assert_eq!(left[30], 0.0);
        assert_eq!(t.position_samples(), 40);
    }

    #[test]
    fn muted_track_is_silent() {
        let (mut r, t) = setup(graph_with(vec![constant_clip(0, 100, 0.5)], track_gains(0.0, 0.0, false)));
        t.set_playing(true);
        r.render_block(50);
        assert!(r.output(50).0.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn pan_right_attenuates_left_only() {
        let gains = track_gains(0.0, 1.0, true);
        assert_eq!(gains, [0.0, 1.0]);
        let gains = track_gains(0.0, -0.5, true);
        assert_eq!(gains, [1.0, 0.5]);
    }

    #[test]
    fn loop_wraps_sample_accurately() {
        let (mut r, t) = setup(graph_with(vec![], [1.0, 1.0]));
        t.set_loop(true, 0.010, 0.020); // samples 10..20 at 1 kHz
        t.seek_seconds(0.015);
        t.set_playing(true);
        r.render_block(12); // 15..20 then 10..17
        assert_eq!(t.position_samples(), 17);
    }

    /// A renderer with handles to send it graphs, live notes and launches,
    /// and to hear what the session started.
    struct Rig {
        r: Renderer,
        t: Arc<Transport>,
        graphs: rtrb::Producer<Box<Graph>>,
        live: rtrb::Producer<LiveEvent>,
        controls: rtrb::Producer<Control>,
        launched: rtrb::Consumer<Launched>,
    }

    impl Rig {
        /// At 48 kHz.
        fn new(graph: Graph) -> Self {
            Self::at(graph, 48_000)
        }

        fn at(graph: Graph, rate: u32) -> Self {
            let t = Arc::new(Transport::new());
            t.set_sample_rate(rate);
            let (mut graphs, graph_rx) = rtrb::RingBuffer::new(4);
            let _ = graphs.push(Box::new(graph));
            let (live, live_rx) = rtrb::RingBuffer::new(64);
            let (controls, controls_rx) = rtrb::RingBuffer::new(16);
            let (launched_tx, launched) = rtrb::RingBuffer::new(64);
            let mut links = Links::detached(Graph::empty());
            links.graphs = graph_rx;
            links.live = live_rx;
            links.controls = controls_rx;
            links.launched = launched_tx;
            let mut r = Renderer::new(Arc::clone(&t), rate, links);
            r.take_new_graphs();
            r.prime();
            Self { r, t, graphs, live, controls, launched }
        }

        fn send(&mut self, control: Control) {
            assert!(self.controls.push(control).is_ok());
        }

        fn launched(&mut self) -> Vec<(Option<usize>, u64)> {
            std::iter::from_fn(|| self.launched.pop().ok()).map(|l| (l.scene, l.at)).collect()
        }

        /// Renders `frames` and returns the left channel.
        fn play(&mut self, frames: usize) -> Vec<f32> {
            let mut out = vec![0.0_f32; frames * 2];
            for chunk in out.chunks_mut(1024) {
                self.r.process(chunk, 2);
            }
            out.iter().step_by(2).copied().collect()
        }
    }

    /// A sine synth with an instant attack and a 5 ms release.
    fn synth_track(notes: &[ScheduledNote]) -> TrackNode {
        let params = |id: &str| match id {
            "osc1" => Some(3.0),
            "mix" | "envAmt" => Some(0.0),
            "cutoff" => Some(18_000.0),
            "attack" => Some(0.1),
            "release" => Some(5.0),
            _ => None,
        };
        let synth = soniq_dsp::Instrument::build("synth", 48_000, &params, &[]).unwrap();
        let mut track = TrackNode::new([1.0, 1.0], vec![], Arc::new(PeakMeter::default()));
        track.key = 7;
        track.instrument = Some(InstrumentNode::new("dev".into(), synth, notes));
        track
    }

    fn one_track(track: TrackNode) -> Graph {
        Graph::new(vec![track], 1.0)
    }

    fn fx(id: &str, kind: &str, params: &[(&str, f64)]) -> crate::graph::FxNode {
        let bank = soniq_dsp::ParamBank::new(params.iter().map(|(k, v)| (k.to_string(), *v)));
        crate::graph::FxNode::new(id.into(), soniq_dsp::AudioFx::build(kind, 1000, &bank).unwrap(), None, bank)
    }

    fn playing(graph: Graph, frames: usize) -> Vec<f32> {
        let (mut r, t) = setup(graph);
        r.prime();
        t.set_playing(true);
        r.render_block(frames);
        r.output(frames).0.to_vec()
    }

    /// A plugin stand-in: it holds its input back and says by how much.
    struct Late(crate::graph::Delay, u32);

    impl soniq_dsp::External for Late {
        fn process(&mut self, _: &soniq_dsp::FxContext, left: &mut [f32], right: &mut [f32], _: &[soniq_dsp::BlockEvent]) {
            self.0.process(left, right);
        }

        fn configure(&mut self, _: &dyn soniq_dsp::Params) {}

        fn latency(&self) -> u32 {
            self.1
        }
    }

    fn late(samples: u32) -> crate::graph::FxNode {
        let fx = soniq_dsp::AudioFx::External(Box::new(Late(crate::graph::Delay::new(samples as usize), samples)));
        crate::graph::FxNode::new(format!("late{samples}"), fx, None, soniq_dsp::ParamBank::default())
    }

    /// A track with one click at sample 100.
    fn click_track(key: u64) -> TrackNode {
        let mut track = TrackNode::new([1.0, 1.0], vec![constant_clip(100, 1, 1.0)], Arc::new(PeakMeter::default()));
        track.key = key;
        track
    }

    #[test]
    fn plugin_latency_is_made_up_for_on_the_other_tracks() {
        let mut slow = click_track(1);
        slow.fx.push(late(32));
        let mut graph = Graph::new(vec![slow, click_track(2)], 1.0);
        graph.compensate();
        assert_eq!(graph.latency, 32);
        let out = playing(graph, 300);
        // Both clicks land together, 32 samples late, instead of one after the other.
        let hits: Vec<(usize, f32)> = out.iter().copied().enumerate().filter(|(_, s)| *s != 0.0).collect();
        assert_eq!(hits, [(132, 2.0)]);
    }

    #[test]
    fn a_slow_return_holds_the_dry_mix_back_with_it() {
        let mut track = click_track(1);
        track.sends = vec![1.0];
        let mut graph = Graph::new(vec![track], 1.0);
        graph.returns.push(crate::graph::ReturnNode::new(9, [1.0, 1.0], true, vec![late(20)], Arc::new(PeakMeter::default())));
        graph.compensate();
        assert_eq!(graph.latency, 20);
        let out = playing(graph, 300);
        // Dry and wet together, 20 samples late.
        let hits: Vec<(usize, f32)> = out.iter().copied().enumerate().filter(|(_, s)| *s != 0.0).collect();
        assert_eq!(hits, [(120, 2.0)]);
    }

    #[test]
    fn mixer_automation_follows_the_compensated_sound() {
        use crate::graph::{Dest, LaneNode};
        // A fast track waits 50 samples for a slow one; its volume drops at sample 100 of the song.
        let mut slow = TrackNode::new([0.0, 0.0], Vec::new(), Arc::new(PeakMeter::default()));
        slow.key = 1;
        slow.fx.push(late(50));
        let mut fast = TrackNode::new([1.0, 1.0], vec![constant_clip(0, 400, 1.0)], Arc::new(PeakMeter::default()));
        fast.key = 2;
        fast.lanes.push(LaneNode { dest: Dest::Volume, points: vec![(0, 0.0), (99, 0.0), (100, -60.0)] });
        let mut graph = Graph::new(vec![slow, fast], 1.0);
        graph.compensate();
        let out = playing(graph, 300);
        // The drop comes with the sound of sample 100, which now plays at 150.
        // (Automation steps every 64 samples and ramps over the next step.)
        // Read at the song's own time, it would already be fading at 140.
        assert!(out[140] > 0.99, "{}", out[140]);
        assert!(out[290] < 0.01, "{}", out[290]);
    }

    #[test]
    fn volume_automation_fades_the_track() {
        use crate::graph::{Dest, LaneNode};
        let mut track = TrackNode::new([1.0, 1.0], vec![constant_clip(0, 1000, 1.0)], Arc::new(PeakMeter::default()));
        track.lanes.push(LaneNode { dest: Dest::Volume, points: vec![(0, 0.0), (1000, -60.0)] });
        let out = playing(Graph::new(vec![track], 1.0), 1000);
        assert!((out[10] - 1.0).abs() < 0.05);
        assert!(out[500] > 0.02 && out[500] < 0.06, "−30 dB half way: {}", out[500]);
        assert!(out[990] < 0.003);
    }

    #[test]
    fn effect_parameters_follow_their_lane() {
        use crate::graph::{Dest, LaneNode};
        let mut track = TrackNode::new([1.0, 1.0], vec![constant_clip(0, 1000, 1.0)], Arc::new(PeakMeter::default()));
        track.fx.push(fx("u", "utility", &[("gain", 0.0)]));
        // Utility gain: 0 dB, then −60 dB from sample 500.
        track.lanes.push(LaneNode { dest: Dest::Fx { fx: 0, param: 0 }, points: vec![(0, 0.0), (500, 0.0), (501, -60.0)] });
        let out = playing(Graph::new(vec![track], 1.0), 1000);
        assert!((out[300] - 1.0).abs() < 0.01 && out[950] < 0.01, "{} {}", out[300], out[950]);
    }

    #[test]
    fn an_lfo_and_a_follower_move_parameters() {
        use crate::graph::{Dest, ModNode, ModSource};
        // A square LFO at 1 Hz on the utility's gain (−36…36 dB): +18 dB, then −18 dB.
        let mut track = TrackNode::new([1.0, 1.0], vec![constant_clip(0, 1000, 0.1)], Arc::new(PeakMeter::default()));
        track.fx.push(fx("u", "utility", &[("gain", 0.0)]));
        let lfo = ModSource::Lfo { shape: 2, rate: 1.0, sync: None, phase: 0.0 };
        track.mods.push(ModNode::new(Dest::Fx { fx: 0, param: 0 }, lfo, 0.5, 0.0, (-36.0, 36.0, false)));
        let out = playing(Graph::new(vec![track], 1.0), 1000);
        assert!(out[400] > 0.7 && out[900] < 0.02, "{} {}", out[400], out[900]);

        // A follower turning the track's own gain down as it gets loud.
        let mut track = TrackNode::new([1.0, 1.0], vec![constant_clip(0, 1000, 1.0)], Arc::new(PeakMeter::default()));
        track.fx.push(fx("u", "utility", &[("gain", 0.0)]));
        let follower = ModSource::Follower { gain: 1.0, attack: 0.9, release: 0.99 };
        track.mods.push(ModNode::new(Dest::Fx { fx: 0, param: 0 }, follower, -1.0, 0.0, (-36.0, 36.0, false)));
        let out = playing(Graph::new(vec![track], 1.0), 1000);
        assert!(out[5] > 0.9 && out[900] < 0.05, "{} {}", out[5], out[900]);
    }

    #[test]
    fn tracks_in_a_group_play_through_it() {
        let make = |group_gain: f32| {
            let mut child = TrackNode::new([1.0, 1.0], vec![constant_clip(0, 200, 0.25)], Arc::new(PeakMeter::default()));
            child.output = Some(0);
            let mut group = TrackNode::new([group_gain, group_gain], vec![], Arc::new(PeakMeter::default()));
            group.group = Some(0);
            group.fx.push(fx("u", "utility", &[("gain", 6.0)]));
            // The group comes first in the list, as in the arrangement.
            Graph::new(vec![group, child], 1.0).with_groups(1)
        };
        let out = playing(make(1.0), 200);
        assert!((out[100] - 0.25 * 1.995).abs() < 0.01, "through the group's effects: {}", out[100]);
        let out = playing(make(0.0), 200);
        assert!(out.iter().all(|s| *s == 0.0), "a muted group silences its tracks");
    }

    #[test]
    fn inserts_run_before_the_fader_and_the_master_chain_after_the_mix() {
        // +6 dB of utility gain on the track, the fader at half, then another +6 dB on the master.
        let mut track = TrackNode::new([0.5, 0.5], vec![constant_clip(0, 1000, 0.1)], Arc::new(PeakMeter::default()));
        track.fx.push(fx("u1", "utility", &[("gain", 6.0)]));
        let mut graph = Graph::new(vec![track], 1.0);
        graph.master_fx.push(fx("u2", "utility", &[("gain", 6.0)]));
        let (mut r, t) = setup(graph);
        r.prime();
        t.set_playing(true);
        r.render_block(500);
        let (left, _) = r.output(500);
        assert!((left[400] - 0.1 * 2.0 * 0.5 * 2.0).abs() < 0.005, "{}", left[400]);
    }

    #[test]
    fn sends_feed_the_returns_before_or_after_the_fader() {
        for (pre, fader, expected) in [(false, 0.5, 0.25 * 0.5), (true, 0.0, 0.25)] {
            let mut track = TrackNode::new([fader, fader], vec![constant_clip(0, 1000, 1.0)], Arc::new(PeakMeter::default()));
            track.sends = vec![0.25];
            let mut graph = Graph::new(vec![track], 1.0);
            graph.returns.push(crate::graph::ReturnNode::new(9, [1.0, 1.0], pre, Vec::new(), Arc::new(PeakMeter::default())));
            let (mut r, t) = setup(graph);
            r.prime();
            t.set_playing(true);
            r.render_block(200);
            // The track itself plus what the return hands back.
            let got = r.output(200).0[100];
            assert!((got - (fader + expected)).abs() < 1e-5, "pre {pre}: {got}");
        }
    }

    #[test]
    fn a_sidechain_hears_another_tracks_own_sound() {
        // The bass's compressor listens to the kick, which is muted: it still ducks.
        let kick = TrackNode::new([0.0, 0.0], vec![constant_clip(0, 2000, 0.9)], Arc::new(PeakMeter::default()));
        let mut bass = TrackNode::new([1.0, 1.0], vec![constant_clip(0, 2000, 0.05)], Arc::new(PeakMeter::default()));
        let mut comp = fx("c", "comp", &[("thresh", -20.0), ("ratio", 10.0), ("attack", 1.0), ("knee", 0.0), ("makeup", 0.0)]);
        comp.sidechain = Some(0);
        bass.fx.push(comp);
        let (mut r, t) = setup(Graph::new(vec![kick, bass], 1.0));
        r.prime();
        t.set_playing(true);
        r.render_block(1000);
        let got = r.output(1000).0[900];
        assert!(got < 0.05 * 0.3, "ducked by the kick: {got}");
    }

    fn loudest(x: &[f32]) -> f32 {
        x.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
    }

    fn hz(x: &[f32]) -> f32 {
        let ups: Vec<usize> = (1..x.len()).filter(|&i| x[i - 1] <= 0.0 && x[i] > 0.0).collect();
        (ups.len() - 1) as f32 * 48_000.0 / (ups[ups.len() - 1] - ups[0]) as f32
    }

    #[test]
    fn clip_notes_start_on_their_sample() {
        let note = ScheduledNote { start: 1000, end: 3000, pitch: 69, velocity: 1.0 };
        let mut rig = Rig::new(one_track(synth_track(&[note])));
        rig.t.set_playing(true);
        let out = rig.play(2048);
        assert!(out[..1000].iter().all(|s| *s == 0.0));
        assert!(loudest(&out[1000..1100]) > 1e-3);
    }

    #[test]
    fn live_notes_play_while_stopped() {
        let mut rig = Rig::new(one_track(synth_track(&[])));
        let _ = rig.live.push(LiveEvent { track: 7, event: NoteEvent::On { pitch: 69, velocity: 1.0 } });
        let out = rig.play(4800);
        assert!((hz(&out[480..]) - 440.0).abs() < 2.0);
        let _ = rig.live.push(LiveEvent { track: 7, event: NoteEvent::Off { pitch: 69 } });
        let out = rig.play(9600);
        assert!(loudest(&out[4800..]) < 1e-4);
    }

    #[test]
    fn midi_effects_change_what_the_instrument_hears() {
        // C♯ played through a Scale effect in C major: the synth plays D.
        let mut track = synth_track(&[]);
        let scale = soniq_dsp::MidiFx::build("scale", &|_: &str| None, soniq_dsp::Key::parse("C major")).unwrap();
        track.instrument = track.instrument.map(|node| node.with_fx(vec![("scale".into(), scale)]));
        let mut rig = Rig::new(one_track(track));
        let _ = rig.live.push(LiveEvent { track: 7, event: NoteEvent::On { pitch: 61, velocity: 1.0 } });
        let out = rig.play(9600);
        assert!((hz(&out[480..]) - 293.66).abs() < 2.0, "{}", hz(&out[480..]));
        // The note-off for C♯ ends the D.
        let _ = rig.live.push(LiveEvent { track: 7, event: NoteEvent::Off { pitch: 61 } });
        let out = rig.play(9600);
        assert!(loudest(&out[4800..]) < 1e-4);
    }

    #[test]
    fn a_jump_releases_clip_notes_but_not_live_ones() {
        let note = ScheduledNote { start: 0, end: 480_000, pitch: 60, velocity: 1.0 };
        let mut rig = Rig::new(one_track(synth_track(&[note])));
        rig.t.set_playing(true);
        rig.play(1024);
        let _ = rig.live.push(LiveEvent { track: 7, event: NoteEvent::On { pitch: 72, velocity: 1.0 } });
        rig.play(1024);
        // Jump past the start of the long note: it stops, the held key does not.
        rig.t.seek_seconds(2.0);
        let out = rig.play(9600);
        assert!((hz(&out[4800..]) - 523.25).abs() < 2.0, "{}", hz(&out[4800..]));
    }

    #[test]
    fn edits_keep_notes_sounding_but_stop_deleted_ones() {
        let note = ScheduledNote { start: 0, end: 96_000, pitch: 60, velocity: 1.0 };
        let mut rig = Rig::new(one_track(synth_track(&[note])));
        rig.t.set_playing(true);
        rig.play(2048);
        // The same song rebuilt, as when a fader moves: the note carries on.
        let _ = rig.graphs.push(Box::new(one_track(synth_track(&[note]))));
        let out = rig.play(4800);
        assert!(loudest(&out[..256]) > 0.01 && loudest(&out[4000..]) > 0.01);
        // The note deleted while it plays: it stops instead of hanging.
        let _ = rig.graphs.push(Box::new(one_track(synth_track(&[]))));
        let out = rig.play(9600);
        assert!(loudest(&out[4800..]) < 1e-4);
    }

    /// A track at 1 kHz (120 BPM: 500 samples a beat) with a long arrangement
    /// clip at 0.5 and session clips in scenes 0 and 1.
    fn session_track() -> TrackNode {
        let ramp = |len: usize, base: f32| ClipNode::new(0, len as u64, 0, Arc::new(AudioBuffer::new(1000, vec![(0..len).map(|i| base + i as f32 / 10_000.0).collect()])));
        let mut track = TrackNode::new([1.0, 1.0], vec![constant_clip(0, 10_000, 0.5)], Arc::new(PeakMeter::default()));
        track.key = 3;
        track.slots = vec![SlotNode::audio(0, ramp(300, 0.1)), SlotNode::audio(1, ramp(200, 0.2))];
        track
    }

    #[test]
    fn a_session_clip_starts_on_the_beat_loops_and_takes_the_track_over() {
        let mut rig = Rig::at(one_track(session_track()), 1000);
        rig.t.set_playing(true);
        rig.r.render_block(120);
        rig.send(Control::Launch { track: 3, scene: Some(0), grid: 1.0 });
        rig.r.take_controls();
        rig.r.render_block(1000);
        let out = rig.r.output(1000).0;
        // Samples 120…499 still play the arrangement; the clip starts on beat 2.
        assert!((out[379] - 0.5).abs() < 1e-6, "{}", out[379]);
        assert!((out[380] - 0.1).abs() < 1e-6, "{}", out[380]);
        // 300 samples later it comes round again.
        assert!((out[380 + 305] - 0.1005).abs() < 1e-6, "{}", out[685]);
        assert_eq!(rig.launched(), [(Some(0), 500)]);
        let state = rig.r.graph.tracks[0].status.read();
        assert_eq!((state.playing, state.arrangement), (Some(0), false));

        // Stopping the track silences it; back to the arrangement brings that back at once.
        rig.send(Control::Launch { track: 3, scene: None, grid: 0.0 });
        rig.r.take_controls();
        rig.r.render_block(100);
        assert!(rig.r.output(100).0.iter().all(|s| *s == 0.0));
        rig.send(Control::BackToArrangement);
        rig.r.take_controls();
        rig.r.render_block(10);
        assert!((rig.r.output(10).0[0] - 0.5).abs() < 1e-6);
        assert!(rig.r.graph.tracks[0].status.read().arrangement);
    }

    #[test]
    fn follow_actions_move_on_and_stopping_the_transport_stops_the_session() {
        let mut track = session_track();
        track.slots[0] = SlotNode::audio(0, constant_clip(0, 300, 0.1)).with_follow(Some((crate::Follow::Next, 400)));
        let mut rig = Rig::at(one_track(track), 1000);
        // Launched while stopped: it starts with playback, on its first sample.
        rig.send(Control::LaunchScene { scene: 0, grid: 4.0 });
        rig.r.take_controls();
        rig.t.set_playing(true);
        rig.r.render_block(500);
        let out = rig.r.output(500).0;
        assert!((out[0] - 0.1).abs() < 1e-6 && (out[399] - 0.1).abs() < 1e-6);
        assert!((out[400] - 0.2).abs() < 1e-6, "the next clip after 400 samples: {}", out[400]);
        assert_eq!(rig.launched(), [(Some(0), 0), (Some(1), 400)]);
        rig.t.set_playing(false);
        rig.r.render_block(10);
        let state = rig.r.graph.tracks[0].status.read();
        assert_eq!((state.playing, state.arrangement), (None, false));
    }

    #[test]
    fn a_midi_session_clip_loops_its_notes_and_survives_edits() {
        // A 2400-sample loop with one note over its first half.
        let note = ScheduledNote { start: 0, end: 1200, pitch: 69, velocity: 1.0 };
        let with_slot = || {
            let mut track = synth_track(&[]);
            track.slots = vec![SlotNode::midi(0, 2400, &[note])];
            one_track(track)
        };
        let mut rig = Rig::new(with_slot());
        rig.send(Control::Launch { track: 7, scene: Some(0), grid: 1.0 });
        rig.t.set_playing(true);
        let out = rig.play(4800);
        assert!(loudest(&out[100..1100]) > 0.01);
        assert!(loudest(&out[1700..2300]) < 1e-4, "released half way");
        assert!(loudest(&out[2500..3500]) > 0.01, "plays again when the loop comes round");
        // The project rebuilt (a fader moved): the clip plays on where it was.
        let _ = rig.graphs.push(Box::new(with_slot()));
        let out = rig.play(2400);
        assert!(loudest(&out[100..1100]) > 0.01);
        assert_eq!(rig.r.graph.tracks[0].launch.playing, Some(0));
    }

    #[test]
    fn a_warped_clip_plays_its_beats_on_the_songs() {
        // At 1 kHz: audio at its own 120 BPM (500 samples a beat), a song at 240 (250).
        let ramp = Arc::new(AudioBuffer::new(1000, vec![(0..4000).map(|i| i as f32 / 10_000.0).collect()]));
        let warper = soniq_dsp::Warper::new(soniq_dsp::WarpMode::Repitch, None, 1000);
        let clip = ClipNode::new(0, 1000, 0, ramp).with_warp(crate::graph::ClipWarp::new(warper, 0.0, 500.0));
        let graph = Graph::new(vec![TrackNode::new([1.0, 1.0], vec![clip], Arc::new(PeakMeter::default()))], 1.0).with_tempo(TempoMap::constant(240.0, 1000.0));
        let out = playing(graph, 1000);
        // Output sample 100 is beat 0.4, which is source sample 200.
        assert!((out[100] - 0.02).abs() < 1e-4, "{}", out[100]);
        assert!((out[900] - 0.18).abs() < 1e-4, "{}", out[900]);
        // A tempo track: 240 BPM for a beat, then 120. Beat 1.5 is at 250 + 250.
        let ramp = Arc::new(AudioBuffer::new(1000, vec![(0..4000).map(|i| i as f32 / 10_000.0).collect()]));
        let warper = soniq_dsp::Warper::new(soniq_dsp::WarpMode::Repitch, None, 1000);
        let clip = ClipNode::new(0, 2000, 0, ramp).with_warp(crate::graph::ClipWarp::new(warper, 0.0, 500.0));
        let tempo = TempoMap::new([(0.0, 0.0, 240.0, 0.0), (1.0, 0.25, 120.0, 0.0)], 1000.0);
        let graph = Graph::new(vec![TrackNode::new([1.0, 1.0], vec![clip], Arc::new(PeakMeter::default()))], 1.0).with_tempo(tempo);
        let out = playing(graph, 1000);
        assert!((out[500] - 0.075).abs() < 1e-4, "{}", out[500]);
    }

    #[test]
    fn stopped_engine_does_not_move() {
        let (mut r, t) = setup(graph_with(vec![constant_clip(0, 100, 0.5)], [1.0, 1.0]));
        r.render_block(30);
        assert_eq!(t.position_samples(), 0);
        assert!(r.output(30).0.iter().all(|s| *s == 0.0));
    }

    /// A song with something of everything: a sidechain (or not), a synth, a
    /// group with its own effect, a send to a return, a volume lane.
    fn busy_song(sidechain: bool) -> Graph {
        use crate::graph::{Dest, LaneNode};
        let meter = || Arc::new(PeakMeter::default());
        let wave = |freq: f32, len: usize| Arc::new(AudioBuffer::new(48_000, vec![(0..len).map(|i| (i as f32 * freq * 0.0013).sin() * 0.4).collect()]));
        let kick = TrackNode::new([0.0, 0.0], vec![ClipNode::new(0, 9000, 0, wave(3.0, 9000))], meter());
        let mut bass = TrackNode::new([0.8, 0.7], vec![ClipNode::new(500, 8000, 0, wave(7.0, 8000))], meter());
        let mut comp = fx("c", "comp", &[("thresh", -20.0), ("ratio", 10.0), ("attack", 1.0), ("knee", 0.0), ("makeup", 0.0)]);
        comp.sidechain = sidechain.then_some(0);
        bass.fx.push(comp);
        bass.sends = vec![0.3];
        bass.lanes.push(LaneNode { dest: Dest::Volume, points: vec![(0, 0.0), (6000, -24.0)] });
        let mut synth = synth_track(&[ScheduledNote { start: 300, end: 5000, pitch: 57, velocity: 0.8 }, ScheduledNote { start: 2000, end: 7000, pitch: 64, velocity: 0.6 }]);
        synth.key = 3;
        synth.fx.push(fx("u", "utility", &[("gain", -3.0)]));
        let mut group = TrackNode::new([0.9, 0.9], vec![], meter());
        group.group = Some(0);
        group.fx.push(fx("g", "comp", &[("thresh", -12.0), ("ratio", 4.0), ("attack", 5.0), ("knee", 6.0), ("makeup", 0.0)]));
        let children = (0..3).map(|i| {
            let mut child = TrackNode::new([0.6, 0.5], vec![ClipNode::new(i * 700, 6000, 0, wave(11.0 + i as f32, 6000))], meter());
            child.output = Some(0);
            child.fx.push(fx("u", "utility", &[("gain", 2.0 * i as f64)]));
            child
        });
        let mut tracks = vec![group, kick, bass, synth];
        tracks.extend(children);
        let mut graph = Graph::new(tracks, 0.9).with_groups(1);
        graph.returns.push(crate::graph::ReturnNode::new(9, [1.0, 1.0], false, vec![fx("r", "utility", &[("gain", -6.0)])], meter()));
        graph
    }

    /// Both channels of `frames` of a song, played with `workers` worker threads.
    fn played_on(graph: Graph, frames: usize, workers: usize) -> Vec<f32> {
        let t = Arc::new(Transport::new());
        t.set_sample_rate(48_000);
        let mut links = Links::detached(graph);
        links.workers = (workers > 0).then(|| Arc::new(Workers::new(workers)));
        let mut r = Renderer::new(Arc::clone(&t), 48_000, links);
        r.prime();
        t.set_playing(true);
        let mut out = vec![0.0_f32; frames * 2];
        for chunk in out.chunks_mut(512) {
            r.process(chunk, 2);
        }
        out
    }

    #[test]
    fn several_cores_mix_exactly_what_one_does() {
        for sidechain in [false, true] {
            let alone = played_on(busy_song(sidechain), 9000, 0);
            let shared = played_on(busy_song(sidechain), 9000, 3);
            assert!(loudest(&alone) > 0.1, "the song plays");
            let first = alone.iter().zip(&shared).position(|(a, b)| a.to_bits() != b.to_bits());
            assert_eq!(first, None, "sidechain {sidechain}: bit for bit the same");
        }
    }

    #[test]
    fn the_analysis_hears_the_master_but_not_the_click() {
        let mut links = Links::detached(Graph::new(vec![TrackNode::new([0.5, 0.25], vec![constant_clip(0, 1000, 1.0)], Arc::new(PeakMeter::default()))], 1.0));
        let (tap, mut heard) = rtrb::RingBuffer::new(4000);
        links.analysis = Some(tap);
        let analysing = Arc::clone(&links.analysing);
        let t = Arc::new(Transport::new());
        t.set_sample_rate(1000);
        t.set_metronome(true);
        let mut r = Renderer::new(Arc::clone(&t), 1000, links);
        r.prime();
        t.set_playing(true);
        r.render_block(100);
        assert_eq!(heard.slots(), 0, "nothing while nobody asks");
        analysing.store(true, Ordering::Relaxed);
        // Samples 100–600, with the second beat's click at 500.
        r.render_block(500);
        let got: Vec<f32> = std::iter::from_fn(|| heard.pop().ok()).collect();
        assert_eq!(got.len(), 1000);
        assert!(got.chunks(2).all(|f| f == [0.5, 0.25]), "the mix without the click");
        assert!(r.output(500).0[401] != 0.5, "while the output has it");
    }

    /// `cargo test --release -p soniq-engine -- --ignored --nocapture`
    #[test]
    #[ignore = "a timing, not a check"]
    fn several_cores_render_a_heavy_song_faster() {
        let song = || {
            let tracks = (0..32)
                .map(|i| {
                    let notes: Vec<ScheduledNote> = (0..8).map(|n| ScheduledNote { start: n * 12_000, end: n * 12_000 + 30_000, pitch: 40 + (i * 3 + n as u8 * 5) % 40, velocity: 0.7 }).collect();
                    let mut track = synth_track(&notes);
                    track.key = 100 + u64::from(i);
                    track.gain = [0.05, 0.05];
                    for k in 0..4 {
                        track.fx.push(fx(&format!("c{k}"), "comp", &[("thresh", -20.0), ("ratio", 4.0), ("attack", 5.0), ("knee", 6.0), ("makeup", 0.0)]));
                    }
                    track
                })
                .collect();
            Graph::new(tracks, 1.0)
        };
        let cores = std::thread::available_parallelism().map_or(1, usize::from);
        for workers in [0, (cores - 1).min(7)] {
            let started = std::time::Instant::now();
            let out = played_on(song(), 48_000 * 10, workers);
            let took = started.elapsed();
            println!("{} core(s): 10 s of 32 synth tracks in {took:?} (peak {})", workers + 1, loudest(&out));
        }
    }
}
