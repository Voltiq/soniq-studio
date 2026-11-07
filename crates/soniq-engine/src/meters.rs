use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex};

use crate::launch::{ClipState, ClipStatus};

/// Highest absolute sample per channel since the last read. Written by the
/// audio thread with `fetch_max` (non-negative f32 bits sort like integers),
/// read and reset by the meter thread.
#[derive(Default)]
pub struct PeakMeter {
    left: AtomicU32,
    right: AtomicU32,
}

impl PeakMeter {
    pub fn update(&self, left: f32, right: f32) {
        if left.is_finite() {
            self.left.fetch_max(left.abs().to_bits(), Relaxed);
        }
        if right.is_finite() {
            self.right.fetch_max(right.abs().to_bits(), Relaxed);
        }
    }

    pub fn take(&self) -> [f32; 2] {
        [f32::from_bits(self.left.swap(0, Relaxed)), f32::from_bits(self.right.swap(0, Relaxed))]
    }
}

/// The last stage before the speakers: a brick-wall limiter at −0.3 dBFS (on
/// unless switched off) and a guard that silences a block holding NaN or
/// infinity (a broken plugin) instead of letting it reach the speakers.
pub struct Safety {
    enabled: AtomicBool,
    /// The most gain reduction since the last read, in dB (non-negative f32 bits).
    reduction: AtomicU32,
    /// A block was silenced since the last read.
    silenced: AtomicBool,
}

impl Default for Safety {
    fn default() -> Self {
        Self { enabled: AtomicBool::new(true), reduction: AtomicU32::new(0), silenced: AtomicBool::new(false) }
    }
}

impl Safety {
    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Relaxed);
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Relaxed)
    }

    pub(crate) fn reduced(&self, db: f32) {
        self.reduction.fetch_max(db.max(0.0).to_bits(), Relaxed);
    }

    pub(crate) fn silence(&self) {
        self.silenced.store(true, Relaxed);
    }

    /// The most gain reduction (dB) and whether anything was silenced, since the last read.
    pub fn take(&self) -> (f32, bool) {
        (f32::from_bits(self.reduction.swap(0, Relaxed)), self.silenced.swap(false, Relaxed))
    }
}

/// Every meter the engine writes, and what each track's session plays, keyed by track id.
#[derive(Default)]
pub struct Meters {
    pub master: Arc<PeakMeter>,
    pub safety: Arc<Safety>,
    /// Gain reduction of dynamics devices, by device id.
    devices: Mutex<HashMap<String, Arc<PeakMeter>>>,
    tracks: Mutex<HashMap<String, Arc<PeakMeter>>>,
    clips: Mutex<HashMap<String, Arc<ClipStatus>>>,
}

impl Meters {
    /// The meter for a track, created on first use. Never called on the audio thread.
    pub fn track(&self, id: &str) -> Arc<PeakMeter> {
        let mut tracks = self.tracks.lock().unwrap_or_else(|p| p.into_inner());
        Arc::clone(tracks.entry(id.to_string()).or_default())
    }

    /// The session status of a track, created on first use. Never called on the audio thread.
    pub fn clip_status(&self, id: &str) -> Arc<ClipStatus> {
        let mut clips = self.clips.lock().unwrap_or_else(|p| p.into_inner());
        Arc::clone(clips.entry(id.to_string()).or_default())
    }

    /// What every track's session plays, for tracks where that is anything.
    pub fn clip_states(&self) -> Vec<(String, ClipState)> {
        let clips = self.clips.lock().unwrap_or_else(|p| p.into_inner());
        clips
            .iter()
            .map(|(id, s)| (id.clone(), s.read()))
            .filter(|(_, s)| !s.arrangement || s.playing.is_some() || s.queued.is_some() || s.stopping)
            .collect()
    }

    /// Forgets meters of tracks that no longer exist.
    pub fn retain(&self, ids: &HashSet<&str>) {
        let mut tracks = self.tracks.lock().unwrap_or_else(|p| p.into_inner());
        tracks.retain(|id, _| ids.contains(id.as_str()));
        let mut clips = self.clips.lock().unwrap_or_else(|p| p.into_inner());
        clips.retain(|id, _| ids.contains(id.as_str()));
    }

    /// The gain-reduction meter of a device, created on first use. Never called on the audio thread.
    pub fn device(&self, id: &str) -> Arc<PeakMeter> {
        let mut devices = self.devices.lock().unwrap_or_else(|p| p.into_inner());
        Arc::clone(devices.entry(id.to_string()).or_default())
    }

    /// Forgets devices no longer in the song.
    pub fn retain_devices(&self, ids: &HashSet<&str>) {
        let mut devices = self.devices.lock().unwrap_or_else(|p| p.into_inner());
        devices.retain(|id, _| ids.contains(id.as_str()));
    }

    /// Reads and resets the devices' gain reduction (dB), for those turning anything down.
    pub fn take_devices(&self) -> Vec<(String, f32)> {
        let devices = self.devices.lock().unwrap_or_else(|p| p.into_inner());
        devices.iter().map(|(id, m)| (id.clone(), m.take()[0])).filter(|(_, db)| *db > 0.05).collect()
    }

    /// Reads and resets every meter.
    pub fn take_all(&self) -> (Vec<(String, [f32; 2])>, [f32; 2]) {
        let tracks = self.tracks.lock().unwrap_or_else(|p| p.into_inner());
        let levels = tracks.iter().map(|(id, m)| (id.clone(), m.take())).collect();
        (levels, self.master.take())
    }
}
