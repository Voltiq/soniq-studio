//! Devices that run outside the engine: plugins in a sandbox process.

use crate::{BlockEvent, FxContext, Params};

/// A device the engine hands whole blocks to. An instrument sees the block's
/// notes at their sample offsets and adds its sound; an effect processes the
/// audio in place. Whatever it does must not allocate or lock on the audio
/// thread, and it must give up in time when its process does not answer.
pub trait External: Send {
    /// Processes one block: an effect changes `left` and `right` in place, an
    /// instrument adds to them.
    fn process(&mut self, ctx: &FxContext, left: &mut [f32], right: &mut [f32], notes: &[BlockEvent]);

    /// New parameter values (knobs, automation, modulators), sent with the next block.
    fn configure(&mut self, params: &dyn Params);

    /// Samples its output comes late by, which the engine makes up for on
    /// every other track.
    fn latency(&self) -> u32 {
        0
    }
}
