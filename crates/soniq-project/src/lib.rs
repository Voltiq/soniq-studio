//! The Soniq Studio project: a plain, serialisable description of a song.
//!
//! Audio sample data never lives here, only references to pool assets, so a
//! `Project` is cheap to clone. That is what makes snapshot-based undo viable.

pub mod catalog;
mod history;
mod model;
mod ops;
mod tempo;

pub use history::History;
pub use model::*;
pub use ops::{Applied, ChainPatch, Op, OpError, ReturnPatch, TrackPatch, apply, controller_op, record_slot};
pub use tempo::{MAX_BPM, MIN_BPM, TempoMap, TempoPoint, TempoSegment};

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Short, unique-enough ids: a millisecond timestamp plus a process counter.
pub fn new_id(prefix: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let millis = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{:x}{:03x}", millis & 0xff_ffff_ffff, n & 0xfff)
}
