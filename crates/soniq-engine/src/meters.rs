use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicU32;
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

/// Every meter the engine writes, and what each track's session plays, keyed by track id.
#[derive(Default)]
pub struct Meters {
    pub master: Arc<PeakMeter>,
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

    /// Reads and resets every meter.
    pub fn take_all(&self) -> (Vec<(String, [f32; 2])>, [f32; 2]) {
        let tracks = self.tracks.lock().unwrap_or_else(|p| p.into_inner());
        let levels = tracks.iter().map(|(id, m)| (id.clone(), m.take())).collect();
        (levels, self.master.take())
    }
}
