//! Real-time audio engine.
//!
//! The audio callback runs on a driver thread owned by cpal. It owns a
//! [`render::Renderer`] that plays the current [`Graph`]; new graphs arrive
//! through a lock-free queue and old ones go back the same way to be freed
//! off the audio thread. Transport state is shared through atomics, so the
//! callback never locks, allocates or blocks.

pub mod demo;
mod devices;
mod graph;
mod launch;
mod meters;
mod metronome;
mod offline;
mod record;
mod render;
mod tempo;
mod workers;
mod transport;

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread::{self, JoinHandle};

use serde::Serialize;
use soniq_io::AudioBuffer;

pub use devices::{DeviceInfo, DeviceOptions, Direction, EngineConfig, device_options, list_devices};
pub use graph::{
    CONTROL, ClipNode, ClipWarp, Dest, FxNode, Graph, InstrumentNode, LaneNode, MAX_BLOCK, MAX_RETURNS, ModNode, ModSource, ReturnNode, ScheduledNote,
    TimedEvent, TrackNode, crossfade_overlaps, db_to_gain, track_gains, track_key,
};
pub use tempo::TempoMap;
pub use launch::{ClipState, ClipStatus, Follow, Launched, SlotNode};
pub use meters::{Meters, PeakMeter};
pub use offline::render_offline;
pub use record::{InputStatus, Recorder, TAKE_SUFFIX, Take};
pub use render::{ALL_TRACKS, LiveEvent, MonitorFeed, Renderer};
pub use soniq_dsp::NoteEvent;

use render::{Control, Garbage, Links};
use workers::Workers;
pub use transport::Transport;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub running: bool,
    pub host: String,
    pub device: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub buffer_frames: u32,
    /// From rendering to hearing, as the driver reports it.
    pub output_latency_ms: f64,
    pub error: Option<String>,
    /// Cores the mix is spread over (the audio thread and its workers).
    pub cores: usize,
}

struct StreamHandle {
    shutdown: mpsc::Sender<()>,
    thread: Option<JoinHandle<()>>,
    graphs: rtrb::Producer<Box<Graph>>,
    controls: rtrb::Producer<Control>,
    live: rtrb::Producer<LiveEvent>,
    garbage: rtrb::Consumer<Garbage>,
    launched: rtrb::Consumer<Launched>,
    analysis: rtrb::Consumer<f32>,
    /// The newest graph if the queue was full; retried by `flush`.
    pending: Option<Box<Graph>>,
}

pub struct AudioEngine {
    transport: Arc<Transport>,
    meters: Arc<Meters>,
    status: Mutex<EngineStatus>,
    buffer_frames: Arc<AtomicU32>,
    output_latency_us: Arc<AtomicU32>,
    /// Samples the live graph comes out late by (plugin delay compensation).
    graph_latency: AtomicU64,
    previewing: Arc<AtomicBool>,
    stream: Mutex<Option<StreamHandle>>,
    /// Kept across restarts of the stream while the number of cores stays the same.
    workers: Mutex<Option<Arc<Workers>>>,
    /// The master's sound goes to the analysis queue.
    analysing: Arc<AtomicBool>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl AudioEngine {
    pub fn new() -> Self {
        Self {
            transport: Arc::new(Transport::new()),
            meters: Arc::new(Meters::default()),
            status: Mutex::new(EngineStatus::default()),
            buffer_frames: Arc::new(AtomicU32::new(0)),
            output_latency_us: Arc::new(AtomicU32::new(0)),
            graph_latency: AtomicU64::new(0),
            previewing: Arc::new(AtomicBool::new(false)),
            stream: Mutex::new(None),
            workers: Mutex::new(None),
            analysing: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Opens (or reopens) the output described by `config`. `cpal::Stream` is
    /// not `Send` on every platform, so a dedicated thread owns it.
    pub fn start(&self, config: &EngineConfig) -> Result<EngineStatus, String> {
        self.stop();
        let (graph_tx, graph_rx) = rtrb::RingBuffer::<Box<Graph>>::new(32);
        let (control_tx, control_rx) = rtrb::RingBuffer::<Control>::new(16);
        // Room for a burst of MIDI (a chord, a sustain sweep) between two callbacks.
        let (live_tx, live_rx) = rtrb::RingBuffer::<LiveEvent>::new(1024);
        let (garbage_tx, garbage_rx) = rtrb::RingBuffer::<Garbage>::new(64);
        let (launched_tx, launched_rx) = rtrb::RingBuffer::<Launched>::new(1024);
        // About a second of stereo at the highest rates (an even size keeps pairs together).
        let (analysis_tx, analysis_rx) = rtrb::RingBuffer::<f32>::new(2 * 192_000);
        let (ready_tx, ready_rx) = mpsc::channel::<Result<EngineStatus, String>>();
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
        self.previewing.store(false, Ordering::Relaxed);
        let wanted = workers::worker_count(config.cores);
        let pool = {
            let mut slot = lock(&self.workers);
            match slot.as_ref() {
                Some(pool) if pool.len() == wanted => Arc::clone(pool),
                _ => Arc::clone(slot.insert(Arc::new(Workers::new(wanted)))),
            }
        };
        let parts = devices::StreamParts {
            transport: Arc::clone(&self.transport),
            buffer_frames: Arc::clone(&self.buffer_frames),
            output_latency_us: Arc::clone(&self.output_latency_us),
            links: Links {
                graphs: graph_rx,
                controls: control_rx,
                live: live_rx,
                garbage: garbage_tx,
                master_meter: Arc::clone(&self.meters.master),
                previewing: Arc::clone(&self.previewing),
                launched: launched_tx,
                workers: (!pool.is_empty()).then_some(pool),
                analysis: Some(analysis_tx),
                analysing: Arc::clone(&self.analysing),
            },
        };
        let config = config.clone();

        let thread = thread::Builder::new()
            .name("soniq-audio".into())
            .spawn(move || match devices::open_output(&config, parts) {
                Ok((stream, status)) => {
                    use cpal::traits::StreamTrait;
                    if let Err(err) = stream.play() {
                        let _ = ready_tx.send(Err(format!("Could not start the audio stream: {err}")));
                        return;
                    }
                    let _ = ready_tx.send(Ok(status));
                    // Blocks until a shutdown message arrives or the sender is dropped.
                    let _ = shutdown_rx.recv();
                    drop(stream);
                }
                Err(err) => {
                    let _ = ready_tx.send(Err(err));
                }
            })
            .map_err(|e| format!("Could not start the audio thread: {e}"))?;

        let mut result = ready_rx.recv().unwrap_or_else(|_| Err("The audio thread stopped unexpectedly.".into()));
        if let Ok(status) = &mut result {
            status.cores = wanted + 1;
        }
        match &result {
            Ok(status) => {
                self.buffer_frames.store(0, Ordering::Relaxed);
                *lock(&self.status) = status.clone();
                *lock(&self.stream) = Some(StreamHandle {
                    shutdown: shutdown_tx,
                    thread: Some(thread),
                    graphs: graph_tx,
                    controls: control_tx,
                    live: live_tx,
                    garbage: garbage_rx,
                    launched: launched_rx,
                    analysis: analysis_rx,
                    pending: None,
                });
            }
            Err(err) => {
                let _ = thread.join();
                let mut status = lock(&self.status);
                status.running = false;
                status.error = Some(err.clone());
            }
        }
        result
    }

    pub fn stop(&self) {
        if let Some(mut handle) = lock(&self.stream).take() {
            let _ = handle.shutdown.send(());
            if let Some(thread) = handle.thread.take() {
                let _ = thread.join();
            }
        }
        lock(&self.status).running = false;
    }

    /// Sends the master's sound to [`Self::drain_analysis`], or stops sending it.
    pub fn set_analysing(&self, on: bool) {
        self.analysing.store(on, Ordering::Relaxed);
    }

    /// Hands what the master played since the last call to `f`, as
    /// interleaved stereo in one or two pieces.
    pub fn drain_analysis(&self, f: impl FnOnce(&[f32], &[f32])) {
        let mut guard = lock(&self.stream);
        let Some(handle) = guard.as_mut() else { return };
        // Whole frames only: the audio thread may be between a left and its right.
        let ready = handle.analysis.slots() & !1;
        if let Ok(chunk) = handle.analysis.read_chunk(ready) {
            let (first, second) = chunk.as_slices();
            f(first, second);
            chunk.commit_all();
        }
    }

    /// Hands a new graph to the audio thread and frees graphs it has finished with.
    pub fn set_graph(&self, graph: Graph) {
        self.graph_latency.store(graph.latency, Ordering::Relaxed);
        let mut guard = lock(&self.stream);
        let Some(handle) = guard.as_mut() else { return };
        // A newer graph supersedes one still waiting for room in the queue.
        handle.pending = Some(Box::new(graph));
        Self::push_pending(handle);
    }

    /// Retries a graph that found the queue full. Call it regularly (the meter
    /// loop does) so a burst of edits never leaves the engine on a stale graph.
    pub fn flush(&self) {
        if let Some(handle) = lock(&self.stream).as_mut() {
            Self::push_pending(handle);
        }
    }

    fn push_pending(handle: &mut StreamHandle) {
        while handle.garbage.pop().is_ok() {}
        if let Some(graph) = handle.pending.take()
            && let Err(rtrb::PushError::Full(graph)) = handle.graphs.push(graph)
        {
            handle.pending = Some(graph);
        }
    }

    pub fn status(&self) -> EngineStatus {
        let mut status = lock(&self.status).clone();
        // Drivers may pick their own size; report what the callback actually gets.
        let actual = self.buffer_frames.load(Ordering::Relaxed);
        if actual > 0 {
            status.buffer_frames = actual;
        }
        status.output_latency_ms = self.output_latency() * 1000.0;
        status
    }

    /// Seconds from rendering a sample to hearing it, as the driver reports it.
    pub fn output_latency(&self) -> f64 {
        f64::from(self.output_latency_us.load(Ordering::Relaxed)) / 1e6
    }

    /// Seconds plugins (and their compensation) hold the mix back by.
    pub fn plugin_latency(&self) -> f64 {
        self.graph_latency.load(Ordering::Relaxed) as f64 / f64::from(self.sample_rate().max(1))
    }

    fn send(&self, control: Control) -> Result<(), String> {
        let mut guard = lock(&self.stream);
        let handle = guard.as_mut().ok_or("The audio engine is not running.")?;
        handle.controls.push(control).map_err(|_| "The audio engine is busy; try again.".to_string())
    }

    /// Plays `buffer` (at the engine rate) once, on top of the mix.
    pub fn preview(&self, buffer: Arc<AudioBuffer>) -> Result<(), String> {
        self.send(Control::Preview(buffer))?;
        // Report it as playing right away, before the audio thread picks it up.
        self.previewing.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub fn stop_preview(&self) {
        let _ = self.send(Control::StopPreview);
        self.previewing.store(false, Ordering::Relaxed);
    }

    pub fn is_previewing(&self) -> bool {
        self.previewing.load(Ordering::Relaxed)
    }

    /// Hands the input to the audio thread for monitoring (`None` stops it).
    pub fn set_monitor(&self, feed: Option<MonitorFeed>) {
        let _ = self.send(Control::Monitor(feed.map(Box::new)));
    }

    /// Starts the clip in a scene's slot on the track with key `track`
    /// ([`track_key`]), or stops the track with `None`, on the next line of
    /// a grid `quantize` beats apart (at once with 0). Launched while
    /// stopped, it starts with playback.
    pub fn launch(&self, track: u64, scene: Option<usize>, quantize: f64) -> Result<(), String> {
        self.send(Control::Launch { track, scene, grid: quantize })
    }

    /// Every track plays its clip in `scene`; tracks without one stop.
    pub fn launch_scene(&self, scene: usize, quantize: f64) -> Result<(), String> {
        self.send(Control::LaunchScene { scene, grid: quantize })
    }

    pub fn stop_clips(&self, quantize: f64) -> Result<(), String> {
        self.send(Control::StopClips { grid: quantize })
    }

    /// Every track plays the arrangement again, at once.
    pub fn back_to_arrangement(&self) -> Result<(), String> {
        self.send(Control::BackToArrangement)
    }

    /// Session clips started and stopped since the last call, in order.
    pub fn take_launched(&self) -> Vec<Launched> {
        let mut out = Vec::new();
        if let Some(handle) = lock(&self.stream).as_mut() {
            while let Ok(l) = handle.launched.pop() {
                out.push(l);
            }
        }
        out
    }

    /// Plays a note live on the instrument of the track with key `track`
    /// ([`track_key`]), or on every instrument with [`ALL_TRACKS`].
    pub fn play_note(&self, track: u64, event: NoteEvent) -> bool {
        lock(&self.stream).as_mut().is_some_and(|h| h.live.push(LiveEvent { track, event }).is_ok())
    }

    pub fn transport(&self) -> &Transport {
        &self.transport
    }

    /// A shared handle, for the input stream and other threads.
    pub fn transport_handle(&self) -> Arc<Transport> {
        Arc::clone(&self.transport)
    }

    pub fn meters(&self) -> &Arc<Meters> {
        &self.meters
    }

    pub fn sample_rate(&self) -> u32 {
        self.transport.sample_rate()
    }
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.stop();
    }
}
