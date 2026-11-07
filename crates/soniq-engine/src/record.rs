//! Recording. The input stream is open only while a track is armed (so the
//! system's "microphone in use" indicator means something). Its callback
//! meters the input and, while recording, pushes samples into a lock-free ring
//! that a writer thread streams to a 32-bit float WAV, flushing the header
//! twice a second: if the app crashes, everything up to the last flush is a
//! valid file.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use serde::Serialize;

use crate::devices::{EngineConfig, find_host, find_input};
use crate::meters::PeakMeter;
use crate::render::MonitorFeed;
use crate::transport::Transport;

/// Raw takes end in this; the app turns them into per-track files and deletes
/// them, so one left behind means the app stopped mid-recording.
pub const TAKE_SUFFIX: &str = ".take.wav";

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputStatus {
    pub open: bool,
    pub device: String,
    pub sample_rate: u32,
    pub channels: u16,
    /// From the sound reaching the input to the app receiving it, as the driver reports it.
    pub latency_ms: f64,
    pub error: Option<String>,
}

/// A finished recording.
#[derive(Debug, Clone)]
pub struct Take {
    pub path: PathBuf,
    /// Where on the timeline the first sample belongs, in seconds.
    pub start_seconds: f64,
    pub frames: u64,
    pub channels: u16,
    pub sample_rate: u32,
}

struct Shared {
    recording: AtomicBool,
    /// Output-rate playhead position at the first recorded sample; MAX = not yet.
    start_position: AtomicU64,
    dropped: AtomicU64,
    /// Some track plays the input, so the callback feeds the monitor ring.
    monitoring: AtomicBool,
    input_latency_us: AtomicU64,
}

struct InputHandle {
    shutdown: mpsc::Sender<()>,
    thread: Option<JoinHandle<()>>,
    /// Parked here between recordings; the writer thread owns it while recording.
    samples: Option<rtrb::Consumer<f32>>,
    /// Input for monitoring, until the engine takes it.
    monitor: Option<rtrb::Consumer<f32>>,
    channels: u16,
    sample_rate: u32,
}

struct Writer {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<Result<(rtrb::Consumer<f32>, u64), String>>,
    path: PathBuf,
}

pub struct Recorder {
    meter: Arc<PeakMeter>,
    shared: Arc<Shared>,
    input: Mutex<Option<InputHandle>>,
    writer: Mutex<Option<Writer>>,
    status: Mutex<InputStatus>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            meter: Arc::new(PeakMeter::default()),
            shared: Arc::new(Shared {
                recording: AtomicBool::new(false),
                start_position: AtomicU64::new(u64::MAX),
                dropped: AtomicU64::new(0),
                monitoring: AtomicBool::new(false),
                input_latency_us: AtomicU64::new(0),
            }),
            input: Mutex::new(None),
            writer: Mutex::new(None),
            status: Mutex::new(InputStatus::default()),
        }
    }

    pub fn meter(&self) -> &Arc<PeakMeter> {
        &self.meter
    }

    pub fn status(&self) -> InputStatus {
        let mut status = lock(&self.status).clone();
        status.latency_ms = self.latency() * 1000.0;
        status
    }

    /// Seconds from the sound reaching the input to the callback receiving it.
    pub fn latency(&self) -> f64 {
        self.shared.input_latency_us.load(Relaxed) as f64 / 1e6
    }

    /// The input's monitoring feed and its sample rate, once per opened input.
    pub fn take_monitor_feed(&self) -> Option<(MonitorFeed, u32)> {
        let mut input = lock(&self.input);
        let handle = input.as_mut()?;
        let samples = handle.monitor.take()?;
        Some((MonitorFeed::new(samples, usize::from(handle.channels)), handle.sample_rate))
    }

    /// Whether the input callback feeds the monitoring ring.
    pub fn set_monitoring(&self, on: bool) {
        self.shared.monitoring.store(on, Relaxed);
    }

    pub fn is_open(&self) -> bool {
        lock(&self.input).is_some()
    }

    pub fn is_recording(&self) -> bool {
        self.shared.recording.load(Relaxed)
    }

    /// Opens the configured input (or the system default) on its own thread.
    pub fn open(&self, config: &EngineConfig, transport: Arc<Transport>) -> Result<InputStatus, String> {
        self.close();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(InputStatus, Consumers), String>>();
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
        let config = config.clone();
        let meter = Arc::clone(&self.meter);
        let shared = Arc::clone(&self.shared);

        let thread = thread::Builder::new()
            .name("soniq-input".into())
            .spawn(move || match open_stream(&config, transport, meter, shared) {
                Ok((stream, status, consumers)) => {
                    if let Err(e) = stream.play() {
                        let _ = ready_tx.send(Err(format!("Could not start the input: {e}")));
                        return;
                    }
                    let _ = ready_tx.send(Ok((status, consumers)));
                    let _ = shutdown_rx.recv();
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            })
            .map_err(|e| format!("Could not start the input thread: {e}"))?;

        match ready_rx.recv().unwrap_or_else(|_| Err("The input thread stopped unexpectedly.".into())) {
            Ok((status, (samples, monitor))) => {
                *lock(&self.input) = Some(InputHandle {
                    shutdown: shutdown_tx,
                    thread: Some(thread),
                    samples: Some(samples),
                    monitor: Some(monitor),
                    channels: status.channels,
                    sample_rate: status.sample_rate,
                });
                *lock(&self.status) = status.clone();
                Ok(status)
            }
            Err(e) => {
                let _ = thread.join();
                *lock(&self.status) = InputStatus { error: Some(e.clone()), ..Default::default() };
                Err(e)
            }
        }
    }

    pub fn close(&self) {
        if self.is_recording() {
            return;
        }
        if let Some(mut handle) = lock(&self.input).take() {
            let _ = handle.shutdown.send(());
            if let Some(t) = handle.thread.take() {
                let _ = t.join();
            }
        }
        let mut status = lock(&self.status);
        status.open = false;
    }

    /// Starts writing a take into `dir`. Samples are captured once the
    /// transport is playing and any count-in has finished.
    pub fn start(&self, dir: &Path) -> Result<PathBuf, String> {
        let mut input = lock(&self.input);
        let handle = input.as_mut().ok_or("No audio input is open. Arm a track first.")?;
        let mut consumer = handle.samples.take().ok_or("A recording is already running.")?;
        // Drop anything left from before.
        while consumer.pop().is_ok() {}

        std::fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
        let millis = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let path = dir.join(format!("take-{millis}{TAKE_SUFFIX}"));
        let spec = hound::WavSpec {
            channels: handle.channels,
            sample_rate: handle.sample_rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut wav = match hound::WavWriter::create(&path, spec) {
            Ok(w) => w,
            Err(e) => {
                handle.samples = Some(consumer);
                return Err(format!("Could not create {}: {e}", path.display()));
            }
        };

        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("soniq-recorder".into())
            .spawn(move || {
                let mut written: u64 = 0;
                let mut last_flush = Instant::now();
                loop {
                    let mut any = false;
                    while let Ok(s) = consumer.pop() {
                        wav.write_sample(s).map_err(|e| e.to_string())?;
                        written += 1;
                        any = true;
                    }
                    if last_flush.elapsed() > Duration::from_millis(500) {
                        // Rewrites the header, so the file is valid up to here.
                        wav.flush().map_err(|e| e.to_string())?;
                        last_flush = Instant::now();
                    }
                    if stop_flag.load(Relaxed) && consumer.is_empty() {
                        break;
                    }
                    if !any {
                        thread::sleep(Duration::from_millis(10));
                    }
                }
                wav.finalize().map_err(|e| e.to_string())?;
                Ok((consumer, written / u64::from(spec.channels.max(1))))
            })
            .map_err(|e| format!("Could not start the recorder thread: {e}"))?;

        self.shared.start_position.store(u64::MAX, Relaxed);
        self.shared.dropped.store(0, Relaxed);
        self.shared.recording.store(true, Relaxed);
        *lock(&self.writer) = Some(Writer { stop, thread, path: path.clone() });
        Ok(path)
    }

    /// Stops and finishes the take. `None` if nothing was captured (for
    /// example, stopped during the count-in); its file is removed then.
    pub fn stop(&self, transport: &Transport) -> Result<Option<Take>, String> {
        self.shared.recording.store(false, Relaxed);
        let Some(writer) = lock(&self.writer).take() else { return Ok(None) };
        writer.stop.store(true, Relaxed);
        let result = writer.thread.join().map_err(|_| "The recorder thread crashed.".to_string())?;
        let (consumer, frames) = result?;

        let mut input = lock(&self.input);
        let (channels, sample_rate) = match input.as_mut() {
            Some(handle) => {
                handle.samples = Some(consumer);
                (handle.channels, handle.sample_rate)
            }
            None => (1, 48_000),
        };
        let start = self.shared.start_position.load(Relaxed);
        if start == u64::MAX || frames == 0 {
            let _ = std::fs::remove_file(&writer.path);
            return Ok(None);
        }
        let dropped = self.shared.dropped.load(Relaxed);
        if dropped > 0 {
            eprintln!("recording dropped {dropped} samples: the disk could not keep up");
        }
        Ok(Some(Take {
            path: writer.path,
            start_seconds: start as f64 / f64::from(transport.sample_rate()),
            frames,
            channels,
            sample_rate,
        }))
    }
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}

/// Samples for the recorder, and for monitoring.
type Consumers = (rtrb::Consumer<f32>, rtrb::Consumer<f32>);
type Opened = (cpal::Stream, InputStatus, Consumers);

fn open_stream(config: &EngineConfig, transport: Arc<Transport>, meter: Arc<PeakMeter>, shared: Arc<Shared>) -> Result<Opened, String> {
    let host = find_host(config.host.as_deref());
    let device = find_input(&host, config.input.as_deref()).ok_or("No audio input was found.")?;
    let supported = device.default_input_config().map_err(|e| format!("Could not read the input format: {e}"))?;
    let format = supported.sample_format();
    let stream_config: cpal::StreamConfig = supported.into();
    let channels = stream_config.channels.max(1);
    let rate = stream_config.sample_rate.0;
    // Ten seconds of slack between the audio thread and the disk.
    let (producer, consumer) = rtrb::RingBuffer::<f32>::new(rate as usize * usize::from(channels) * 10);
    // Monitoring only ever needs a few blocks; half a second is plenty.
    let (monitor_tx, monitor_rx) = rtrb::RingBuffer::<f32>::new(rate as usize * usize::from(channels) / 2);

    let parts = (transport, meter, shared, producer, monitor_tx, usize::from(channels));
    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, &stream_config, parts),
        SampleFormat::I16 => build::<i16>(&device, &stream_config, parts),
        SampleFormat::U16 => build::<u16>(&device, &stream_config, parts),
        SampleFormat::I32 => build::<i32>(&device, &stream_config, parts),
        other => Err(format!("Unsupported input format: {other:?}")),
    }?;
    let status = InputStatus {
        open: true,
        device: device.name().unwrap_or_else(|_| "Default input".into()),
        sample_rate: rate,
        channels,
        latency_ms: 0.0,
        error: None,
    };
    Ok((stream, status, (consumer, monitor_rx)))
}

type Parts = (Arc<Transport>, Arc<PeakMeter>, Arc<Shared>, rtrb::Producer<f32>, rtrb::Producer<f32>, usize);

fn build<T>(device: &cpal::Device, config: &cpal::StreamConfig, parts: Parts) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let (transport, meter, shared, mut producer, mut monitor, channels) = parts;
    device
        .build_input_stream(
            config,
            move |data: &[T], info: &cpal::InputCallbackInfo| {
                let stamp = info.timestamp();
                if let Some(behind) = stamp.callback.duration_since(&stamp.capture) {
                    shared.input_latency_us.store(behind.as_micros().min(u128::from(u64::MAX)) as u64, Relaxed);
                }
                let (mut peak_l, mut peak_r) = (0.0_f32, 0.0_f32);
                for frame in data.chunks(channels) {
                    peak_l = peak_l.max(frame[0].to_sample::<f32>().abs());
                    peak_r = peak_r.max(frame.get(1).unwrap_or(&frame[0]).to_sample::<f32>().abs());
                }
                meter.update(peak_l, peak_r);

                // Whole buffers only, so the channels stay in step; the
                // renderer drains the ring every block.
                if shared.monitoring.load(Relaxed) && monitor.slots() >= data.len() {
                    for s in data {
                        let _ = monitor.push(s.to_sample::<f32>());
                    }
                }

                let capturing = shared.recording.load(Relaxed) && transport.is_playing() && transport.count_in().1 == 0;
                if !capturing {
                    return;
                }
                if shared.start_position.load(Relaxed) == u64::MAX {
                    shared.start_position.store(transport.position_samples(), Relaxed);
                }
                // Whole buffers only, so channels never get out of step.
                if producer.slots() < data.len() {
                    shared.dropped.fetch_add(data.len() as u64, Relaxed);
                    return;
                }
                for s in data {
                    let _ = producer.push(s.to_sample::<f32>());
                }
            },
            |err| eprintln!("audio input error: {err}"),
            None,
        )
        .map_err(|e| format!("Could not open the input: {e}"))
}
