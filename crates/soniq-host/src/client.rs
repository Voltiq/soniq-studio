//! The app's side of a sandboxed plugin: starting its process, asking it
//! things, and the engine node that sends it blocks.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command as Process, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering::*};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use soniq_dsp::{BlockEvent, External, FxContext, NoteEvent, Params};

use crate::protocol::{Command, Descriptor, Loaded, Message, ParamInfo, read_frame, write_frame};
use crate::shm::{self, Event, MAX_EVENTS, MAX_FRAMES, RESET, Shared, Transport, Watch, kind};

/// Parameters a device can show and automate at once.
const SLOTS: usize = 1024;
/// Blocks in a row a plugin may miss before it is skipped until it catches up.
const MISSES: u32 = 3;
/// How long an export waits for a block.
const OFFLINE_WAIT_MS: u32 = 10_000;

/// How to start a sandbox: a program, and the arguments that make it one.
#[derive(Debug, Clone)]
pub struct SandboxCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

impl SandboxCommand {
    fn process(&self, mode: &[&str]) -> Process {
        let mut p = Process::new(&self.program);
        p.args(&self.args).args(mode).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            p.creation_flags(CREATE_NO_WINDOW);
        }
        p
    }
}

/// Lists the plugins in a `.clap` file, loading it in a sandbox of its own
/// so a file that crashes on loading only fails its own scan.
pub fn scan(sandbox: &SandboxCommand, file: &Path, timeout: Duration) -> Result<Vec<Descriptor>, String> {
    let mut child = sandbox.process(&["--scan", &file.to_string_lossy()]).spawn().map_err(|e| format!("Could not start the plugin scanner: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("No scanner output.")?;
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(read_frame::<Message>(&mut stdout));
    });
    let answer = rx.recv_timeout(timeout);
    if answer.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    match answer {
        Ok(Ok((Message::Scanned { plugins }, _))) => Ok(plugins),
        Ok(Ok((Message::Failed { message }, _))) => Err(message),
        Ok(_) => Err("It crashed while loading.".into()),
        Err(_) => Err("It took too long to load.".into()),
    }
}

/// What the engine's node shares with the plugin's owner.
struct Audio {
    shared: Shared,
    watch: Watch,
    /// Held by whoever is sending a block (the live engine or an export).
    busy: AtomicBool,
    /// An export owns the plugin: the live engine leaves it alone.
    exclusive: AtomicBool,
    /// Who sent the last block (1 live, 2 an export): a change of hands starts the plugin afresh.
    owner: AtomicU8,
    crashed: AtomicBool,
    /// It stopped answering: blocks pass it by until it catches up.
    stalled: AtomicBool,
    misses: AtomicU32,
    /// The value each parameter slot last had in the plugin (f64 bits), so
    /// values are only sent when they change.
    sent: Box<[AtomicU64]>,
    slots: Mutex<HashMap<u32, usize>>,
    /// Samples the plugin delays its output by, as it last said.
    latency: AtomicU32,
}

impl Audio {
    fn slot(&self, id: u32) -> Option<usize> {
        let mut slots = self.slots.lock().ok()?;
        let next = slots.len();
        let slot = *slots.entry(id).or_insert(next);
        (slot < SLOTS).then_some(slot)
    }
}

/// Replies come back in order; one that missed its request is skipped.
struct Replies {
    rx: Receiver<(Message, Vec<u8>)>,
    stale: usize,
}

/// A plugin running in its sandbox.
pub struct Plugin {
    pub info: Loaded,
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    replies: Mutex<Replies>,
    audio: Arc<Audio>,
}

impl Plugin {
    /// Starts a sandbox and loads the plugin `id` from `file` at `rate`.
    /// `notices` hears what the plugin does by itself (on another thread).
    pub fn spawn(sandbox: &SandboxCommand, file: &Path, id: &str, rate: f64, notices: impl Fn(Message) + Send + 'static) -> Result<Self, String> {
        let name = shm::unique_name();
        let shared = Shared::create(&name).map_err(|e| format!("Could not share memory with the plugin: {e}"))?;
        let mut child = sandbox.process(&["--host"]).spawn().map_err(|e| format!("Could not start the plugin's sandbox: {e}"))?;
        #[cfg(windows)]
        let watch = Watch::new(&child).map_err(|e| e.to_string())?;
        #[cfg(not(windows))]
        let watch = Watch;
        let audio = Arc::new(Audio {
            shared,
            watch,
            busy: AtomicBool::new(false),
            exclusive: AtomicBool::new(false),
            owner: AtomicU8::new(0),
            crashed: AtomicBool::new(false),
            stalled: AtomicBool::new(false),
            misses: AtomicU32::new(0),
            sent: (0..SLOTS).map(|_| AtomicU64::new(f64::NAN.to_bits())).collect(),
            slots: Mutex::default(),
            latency: AtomicU32::new(0),
        });
        let (stdin, mut stdout) = (child.stdin.take().ok_or("No pipe to the plugin.")?, child.stdout.take().ok_or("No pipe from the plugin.")?);
        let (tx, rx) = channel();
        let shared = Arc::clone(&audio);
        std::thread::Builder::new()
            .name("plugin replies".into())
            .spawn(move || {
                loop {
                    match read_frame::<Message>(&mut stdout) {
                        Ok((message, payload)) if message.is_notice() => {
                            // The plugin already has a value it reports: no need to send it back.
                            if let Message::ParamChanged { id, value: Some(value), .. } = message
                                && let Some(&slot) = shared.slots.lock().ok().as_ref().and_then(|s| s.get(&id))
                            {
                                shared.sent[slot].store(value.to_bits(), Relaxed);
                            }
                            if let Message::LatencyChanged { latency } = message {
                                shared.latency.store(latency, Relaxed);
                            }
                            drop(payload);
                            notices(message);
                        }
                        Ok(reply) => {
                            if tx.send(reply).is_err() {
                                break;
                            }
                        }
                        // Something the app cannot read: skip it.
                        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {}
                        Err(_) => {
                            shared.crashed.store(true, Release);
                            notices(Message::Exited);
                            break;
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let mut plugin = Self {
            info: Loaded { name: String::new(), inputs: 0, outputs: 0, notes: false, gui: false, latency: 0 },
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            replies: Mutex::new(Replies { rx, stale: 0 }),
            audio,
        };
        let load = Command::Load { path: file.to_string_lossy().into_owned(), id: id.to_string(), shm: name, rate };
        match plugin.request(&load, &[], Duration::from_secs(30))? {
            (Message::Loaded { info }, _) => {
                plugin.audio.latency.store(info.latency, Relaxed);
                plugin.info = info;
            }
            _ => return Err("The plugin did not load.".into()),
        }
        Ok(plugin)
    }

    fn request(&self, command: &Command, payload: &[u8], timeout: Duration) -> Result<(Message, Vec<u8>), String> {
        if self.crashed() {
            return Err("The plugin has crashed.".into());
        }
        let mut replies = self.replies.lock().map_err(|_| "The plugin is unavailable.")?;
        {
            let mut stdin = self.stdin.lock().map_err(|_| "The plugin is unavailable.")?;
            write_frame(&mut *stdin, command, payload).map_err(|_| "The plugin has crashed.".to_string())?;
        }
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match replies.rx.recv_timeout(left) {
                Ok(_) if replies.stale > 0 => replies.stale -= 1,
                Ok((Message::Failed { message }, _)) => return Err(message),
                Ok(reply) => return Ok(reply),
                Err(RecvTimeoutError::Timeout) => {
                    replies.stale += 1;
                    return Err("The plugin is not answering.".into());
                }
                Err(RecvTimeoutError::Disconnected) => return Err("The plugin has crashed.".into()),
            }
        }
    }

    const WAIT: Duration = Duration::from_secs(5);

    pub fn crashed(&self) -> bool {
        self.audio.crashed.load(Acquire)
    }

    /// Samples the plugin delays its output by, as it last said.
    pub fn latency(&self) -> u32 {
        self.audio.latency.load(Relaxed)
    }

    pub fn params(&self) -> Result<Vec<ParamInfo>, String> {
        match self.request(&Command::Params, &[], Self::WAIT)? {
            (Message::Params { params }, _) => Ok(params),
            _ => Err("Unexpected answer.".into()),
        }
    }

    /// Each parameter's value and text, as the plugin shows them now.
    pub fn texts(&self, ids: &[u32]) -> Result<Vec<(u32, f64, String)>, String> {
        match self.request(&Command::Texts { ids: ids.to_vec() }, &[], Self::WAIT)? {
            (Message::Texts { texts }, _) => Ok(texts),
            _ => Err("Unexpected answer.".into()),
        }
    }

    pub fn save_state(&self) -> Result<Vec<u8>, String> {
        match self.request(&Command::SaveState, &[], Self::WAIT)? {
            (Message::State, state) => Ok(state),
            _ => Err("Unexpected answer.".into()),
        }
    }

    pub fn load_state(&self, state: &[u8]) -> Result<(), String> {
        self.request(&Command::LoadState, state, Self::WAIT).map(drop)
    }

    pub fn show_gui(&self, title: &str) -> Result<(), String> {
        self.request(&Command::ShowGui { title: title.to_string() }, &[], Self::WAIT).map(drop)
    }

    pub fn hide_gui(&self) -> Result<(), String> {
        self.request(&Command::HideGui, &[], Self::WAIT).map(drop)
    }

    /// Restarts the plugin at another sample rate.
    pub fn activate(&self, rate: f64) -> Result<(), String> {
        self.request(&Command::Activate { rate }, &[], Duration::from_secs(10)).map(drop)
    }

    /// The node the engine runs. `params` pairs each id in the device's
    /// parameter bank with the plugin's own id. An export's node (`live`
    /// false) has the plugin to itself until it is dropped.
    pub fn processor(&self, params: &[(String, u32)], instrument: bool, live: bool) -> Processor {
        if !live {
            self.audio.exclusive.store(true, Release);
        }
        let params = params.iter().filter_map(|(key, id)| Some((key.clone(), *id, self.audio.slot(*id)?, f64::NAN))).collect();
        Processor { audio: Arc::clone(&self.audio), params, instrument, live }
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        if !self.crashed() {
            let _ = self.request(&Command::Quit, &[], Duration::from_millis(500));
        }
        if let Ok(mut child) = self.child.lock() {
            let deadline = Instant::now() + Duration::from_millis(500);
            while Instant::now() < deadline && matches!(child.try_wait(), Ok(None)) {
                std::thread::sleep(Duration::from_millis(10));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// A plugin in the engine's graph: every block goes to the sandbox and back.
/// An effect that does not answer in time is passed by; an instrument is silent.
pub struct Processor {
    audio: Arc<Audio>,
    /// Each parameter: its id in the device's bank, the plugin's id, its slot, the value wanted.
    params: Vec<(String, u32, usize, f64)>,
    instrument: bool,
    live: bool,
}

impl Processor {
    /// Sends one block and waits for it; whether the plugin answered.
    fn run(&self, ctx: &FxContext, left: &mut [f32], right: &mut [f32], notes: &[BlockEvent]) -> bool {
        let a = &*self.audio;
        if a.crashed.load(Acquire) || (self.live && a.exclusive.load(Acquire)) {
            return false;
        }
        if self.live {
            if a.busy.compare_exchange(false, true, Acquire, Relaxed).is_err() {
                return false;
            }
        } else {
            while a.busy.compare_exchange(false, true, Acquire, Relaxed).is_err() {
                std::hint::spin_loop();
            }
        }
        let answered = self.exchange(ctx, left, right, notes);
        a.busy.store(false, Release);
        answered
    }

    fn exchange(&self, ctx: &FxContext, left: &mut [f32], right: &mut [f32], notes: &[BlockEvent]) -> bool {
        let a = &*self.audio;
        let n = left.len().min(right.len()).min(MAX_FRAMES);
        // Safety: this side holds the turn (`busy`) and the sandbox is not working on the block.
        let b = unsafe { &mut *a.shared.block() };
        if a.stalled.load(Relaxed) {
            if b.done.load(Acquire) != b.request.load(Relaxed) {
                return false;
            }
            a.stalled.store(false, Relaxed);
        }
        let owner = if self.live { 1 } else { 2 };
        let before = a.owner.swap(owner, Relaxed);
        b.frames = n as u32;
        b.flags = if before != 0 && before != owner { RESET } else { 0 };
        let sr = f64::from(ctx.sample_rate.max(1.0));
        b.transport = Transport {
            playing: u32::from(ctx.playing),
            tempo: if ctx.samples_per_beat > 0.0 { 60.0 * sr / ctx.samples_per_beat } else { 120.0 },
            beat: ctx.beat,
            seconds: ctx.pos as f64 / sr,
        };
        let mut count = 0;
        let mut push = |event: Event| {
            if count < MAX_EVENTS {
                b.events[count] = event;
                count += 1;
            }
        };
        for (_, id, slot, value) in &self.params {
            let sent = &a.sent[*slot];
            if value.is_finite() && f64::from_bits(sent.load(Relaxed)) != *value {
                sent.store(value.to_bits(), Relaxed);
                push(Event { at: 0, kind: kind::PARAM, id: *id, value: *value });
            }
        }
        for e in notes {
            let (k, id, value) = match e.event {
                NoteEvent::On { pitch, velocity } => (kind::NOTE_ON, u32::from(pitch), f64::from(velocity)),
                NoteEvent::Off { pitch } => (kind::NOTE_OFF, u32::from(pitch), 0.0),
                NoteEvent::Sustain(down) => (kind::SUSTAIN, 0, if down { 1.0 } else { 0.0 }),
                NoteEvent::AllNotesOff => (kind::ALL_NOTES_OFF, 0, 0.0),
                NoteEvent::AllSoundOff => (kind::ALL_SOUND_OFF, 0, 0.0),
            };
            push(Event { at: e.at.min(n.saturating_sub(1) as u32), kind: k, id, value });
        }
        b.event_count = count as u32;
        if self.instrument {
            b.input[0][..n].fill(0.0);
            b.input[1][..n].fill(0.0);
        } else {
            b.input[0][..n].copy_from_slice(&left[..n]);
            b.input[1][..n].copy_from_slice(&right[..n]);
        }

        let request = b.request.load(Relaxed).wrapping_add(1);
        b.request.store(request, Release);
        a.shared.signal_go();
        let wait_ms = if self.live { ((n as f64 / sr * 2000.0).ceil() as u32).max(5) } else { OFFLINE_WAIT_MS };
        let deadline = Instant::now() + Duration::from_millis(u64::from(wait_ms));
        // An export waits long, in slices, so a plugin that dies is noticed at once.
        let slice = if self.live { wait_ms } else { 50 };
        let answered = loop {
            if b.done.load(Acquire) == request {
                break true;
            }
            let left_ms = deadline.saturating_duration_since(Instant::now()).as_millis() as u32;
            if left_ms == 0 {
                break false;
            }
            if !a.shared.wait_done(left_ms.min(slice)) && !self.live && (a.crashed.load(Acquire) || !a.watch.alive()) {
                break b.done.load(Acquire) == request;
            }
        };
        if !answered {
            if !a.watch.alive() {
                a.crashed.store(true, Release);
            } else if a.misses.fetch_add(1, Relaxed) + 1 >= MISSES {
                a.stalled.store(true, Relaxed);
            }
            return false;
        }
        a.misses.store(0, Relaxed);
        if self.instrument {
            for i in 0..n {
                left[i] += b.output[0][i];
                right[i] += b.output[1][i];
            }
        } else {
            left[..n].copy_from_slice(&b.output[0][..n]);
            right[..n].copy_from_slice(&b.output[1][..n]);
        }
        true
    }
}

impl External for Processor {
    fn process(&mut self, ctx: &FxContext, left: &mut [f32], right: &mut [f32], notes: &[BlockEvent]) {
        // Blocks longer than the shared memory holds go in pieces.
        let n = left.len().min(right.len());
        let mut done = 0;
        while done < n {
            let m = (n - done).min(MAX_FRAMES);
            let notes_here: &[BlockEvent] = if done == 0 { notes } else { &[] };
            let piece = FxContext { pos: ctx.pos + done as u64, ..*ctx };
            self.run(&piece, &mut left[done..done + m], &mut right[done..done + m], notes_here);
            done += m;
        }
    }

    fn configure(&mut self, params: &dyn Params) {
        for (key, _, _, value) in &mut self.params {
            if let Some(v) = params.get(key) {
                *value = v;
            }
        }
    }

    fn latency(&self) -> u32 {
        self.audio.latency.load(Relaxed)
    }
}

impl Drop for Processor {
    fn drop(&mut self) {
        if !self.live {
            self.audio.exclusive.store(false, Release);
        }
    }
}

