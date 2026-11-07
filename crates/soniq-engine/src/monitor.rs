//! Monitored input, brought to the output's clock.
//!
//! The input and the output are separate streams: often separate devices,
//! sometimes at different sample rates, and never on exactly the same clock.
//! The feed resamples the input to the output rate (cubic interpolation) and
//! keeps a small, steady backlog by nudging the ratio up to ±0.3 %, so the
//! two clocks can drift apart for hours without clicks or growing delay.
//! Nothing here allocates once built: it runs on the audio thread.

use crate::graph::MAX_BLOCK;

/// Input frames the feed can hold between blocks (about a third of a second at 48 kHz).
const PENDING: usize = 16_384;
/// The most the ratio is nudged to follow the backlog.
const NUDGE: f64 = 0.003;

/// Input audio for monitoring: interleaved samples from the input stream.
pub struct MonitorFeed {
    samples: rtrb::Consumer<f32>,
    channels: usize,
    /// The input's sample rate.
    rate: u32,
    /// Input frames waiting, by channel; `filled` of them are real.
    pending: Vec<Vec<f32>>,
    filled: usize,
    /// Where the next output frame is read, in input frames into `pending`.
    pos: f64,
    /// Input frames per output frame: the two rates, and that nudged.
    nominal: f64,
    step: f64,
    /// The backlog's error, smoothed over many blocks.
    drift: f64,
    /// The largest number of frames the input delivered at once: the backlog must cover it.
    burst: usize,
    /// Waiting for the backlog to build up (at the start, and after running dry).
    priming: bool,
    /// One output block per input channel, de-interleaved.
    pub(crate) input: Vec<Vec<f32>>,
}

/// Catmull-Rom between `y1` and `y2`.
fn cubic(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let a = -0.5 * y0 + 1.5 * y1 - 1.5 * y2 + 0.5 * y3;
    let b = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c = -0.5 * y0 + 0.5 * y2;
    ((a * t + b) * t + c) * t + y1
}

impl MonitorFeed {
    pub fn new(samples: rtrb::Consumer<f32>, channels: usize, rate: u32) -> Self {
        let channels = channels.max(1);
        Self {
            samples,
            channels,
            rate,
            pending: vec![vec![0.0; PENDING]; channels],
            filled: 0,
            pos: 1.0,
            nominal: 1.0,
            step: 1.0,
            drift: 0.0,
            burst: 0,
            priming: true,
            input: vec![vec![0.0; MAX_BLOCK]; channels],
        }
    }

    /// The output's rate, once the renderer has the feed.
    pub(crate) fn set_output_rate(&mut self, output: f64) {
        self.nominal = f64::from(self.rate) / output.max(1.0);
        self.step = self.nominal;
    }

    /// Nothing listens: throw away what came in and start again later.
    pub(crate) fn idle(&mut self) {
        let ready = self.samples.slots();
        if let Ok(chunk) = self.samples.read_chunk(ready) {
            chunk.commit_all();
        }
        self.filled = 0;
        self.pos = 1.0;
        self.drift = 0.0;
        self.priming = true;
    }

    /// Moves whatever the input delivered into `pending`.
    fn fill(&mut self) {
        let frames = self.samples.slots() / self.channels;
        let take = frames.min(PENDING - self.filled);
        for f in 0..take {
            for ch in &mut self.pending {
                ch[self.filled + f] = self.samples.pop().unwrap_or(0.0);
            }
        }
        self.filled += take;
        self.burst = self.burst.max(take);
        // No room (it fell far behind): drop the rest, the backlog jump below catches up.
        for _ in take * self.channels..frames * self.channels {
            let _ = self.samples.pop();
        }
    }

    /// Fills `input` with `n` frames at the output rate.
    pub(crate) fn render(&mut self, n: usize) {
        self.fill();
        let per_block = n as f64 * self.nominal;
        let target = self.burst as f64 + per_block + 16.0;
        // Frames past the read position that interpolation can still use.
        let ahead = self.filled as f64 - 2.0 - self.pos;
        if self.priming {
            if ahead < target {
                self.input.iter_mut().for_each(|ch| ch[..n].fill(0.0));
                return;
            }
            self.priming = false;
        }
        // Far too much waiting (a hiccup, or the input started first): skip to a steady backlog.
        if ahead > 4.0 * target + per_block {
            self.pos = self.filled as f64 - 2.0 - target;
            self.drift = 0.0;
        }
        let ahead = self.filled as f64 - 2.0 - self.pos;
        let error = ((ahead - target) / target).clamp(-1.0, 1.0);
        self.drift += (error - self.drift) * 0.02;
        self.step = self.nominal * (1.0 + NUDGE * self.drift);

        let mut i = 0;
        while i < n {
            if self.pos + 2.0 >= self.filled as f64 {
                // Ran dry: silence until the backlog is back.
                self.input.iter_mut().for_each(|ch| ch[i..n].fill(0.0));
                self.priming = true;
                break;
            }
            let k = self.pos as usize;
            let t = (self.pos - k as f64) as f32;
            for (out, p) in self.input.iter_mut().zip(&self.pending) {
                out[i] = cubic(p[k - 1], p[k], p[k + 1], p[k + 2], t);
            }
            self.pos += self.step;
            i += 1;
        }
        // Keep one frame before the read position for the next interpolation.
        let drop = (self.pos as usize).saturating_sub(1).min(self.filled);
        if drop > 0 {
            for ch in &mut self.pending {
                ch.copy_within(drop..self.filled, 0);
            }
            self.filled -= drop;
            self.pos -= drop as f64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds a 440 Hz tone at `in_rate` (arriving in chunks every 10 ms, its
    /// clock `skew` fast) and plays it out at `out_rate` in 128-frame blocks
    /// for `seconds`. Returns the output and the most input ever waiting.
    fn run(in_rate: u32, out_rate: u32, skew: f64, seconds: f64) -> (Vec<f32>, usize) {
        let (mut tx, rx) = rtrb::RingBuffer::<f32>::new(in_rate as usize);
        let mut feed = MonitorFeed::new(rx, 1, in_rate);
        feed.set_output_rate(f64::from(out_rate));
        let (block, chunk) = (128usize, (f64::from(in_rate) / 100.0) as usize);
        let mut out = Vec::new();
        let (mut sent, mut next_chunk, mut most) = (0usize, 0.0_f64, 0usize);
        let mut clock = 0.0_f64;
        while clock < seconds {
            // The input's chunks arrive on its own (slightly fast) clock.
            while next_chunk <= clock {
                for _ in 0..chunk {
                    let t = sent as f64 / f64::from(in_rate);
                    let _ = tx.push((2.0 * std::f64::consts::PI * 440.0 * t).sin() as f32 * 0.5);
                    sent += 1;
                }
                next_chunk += 0.01 / skew;
            }
            feed.render(block);
            most = most.max(feed.filled);
            out.extend_from_slice(&feed.input[0][..block]);
            clock += block as f64 / f64::from(out_rate);
        }
        (out, most)
    }

    fn crossings(x: &[f32]) -> usize {
        x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
    }

    #[test]
    fn another_rate_comes_out_at_the_right_pitch_without_gaps() {
        let (out, _) = run(44_100, 48_000, 1.0, 4.0);
        // After the start, one second holds 440 cycles and never a run of silence.
        let second = &out[96_000..144_000];
        assert!((crossings(second) as i32 - 440).abs() <= 2, "{} cycles", crossings(second));
        let longest_quiet = second.split(|s| s.abs() > 1e-6).map(<[f32]>::len).max().unwrap_or(0);
        assert!(longest_quiet < 4, "a gap of {longest_quiet} frames");
        assert!(second.iter().all(|s| s.abs() <= 0.52), "no overshoot");
    }

    #[test]
    fn clocks_that_drift_apart_keep_a_steady_backlog() {
        for skew in [1.002, 0.998] {
            let (out, most) = run(48_000, 48_000, skew, 30.0);
            assert!(most < 2_000, "the backlog stays small: {most} frames ({skew})");
            let late = &out[out.len() - 48_000..];
            let longest_quiet = late.split(|s| s.abs() > 1e-6).map(<[f32]>::len).max().unwrap_or(0);
            assert!(longest_quiet < 4, "no gaps after 30 s: {longest_quiet} ({skew})");
        }
    }
}
