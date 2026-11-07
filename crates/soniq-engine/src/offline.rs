//! Faster-than-real-time rendering for export: the same renderer the audio
//! thread runs, driven in a loop instead of by a device.

use std::sync::Arc;

use soniq_io::AudioBuffer;

use crate::graph::{Graph, MAX_BLOCK};
use crate::render::{Links, Renderer};
use crate::transport::Transport;

/// Renders `start..end` seconds of `graph` to stereo at `sample_rate`, in
/// time with the song: what the plugins hold back is rendered on past `end`
/// and as much dropped from the start.
pub fn render_offline(graph: Graph, sample_rate: u32, bpm: f64, beats_per_bar: u32, start: f64, end: f64) -> AudioBuffer {
    let late = graph.latency as usize;
    let transport = Arc::new(Transport::new());
    transport.set_sample_rate(sample_rate);
    transport.set_bpm(bpm);
    transport.set_beats_per_bar(beats_per_bar);
    transport.seek_seconds(start);
    transport.set_playing(true);

    let mut renderer = Renderer::new(transport, sample_rate, Links::detached(graph));
    renderer.prime();

    let frames = ((end - start).max(0.0) * f64::from(sample_rate)).round() as usize;
    let mut left = Vec::with_capacity(frames);
    let mut right = Vec::with_capacity(frames);
    let mut block = vec![0.0_f32; MAX_BLOCK * 2];
    let mut done = 0;
    while done < frames + late {
        let n = (frames + late - done).min(MAX_BLOCK);
        renderer.process(&mut block[..n * 2], 2);
        let skip = late.saturating_sub(done).min(n);
        for [l, r] in block[skip * 2..n * 2].as_chunks::<2>().0 {
            left.push(*l);
            right.push(*r);
        }
        done += n;
    }
    AudioBuffer::new(sample_rate, vec![left, right])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{ClipNode, TrackNode};
    use crate::meters::PeakMeter;

    #[test]
    fn renders_the_requested_range_at_full_level() {
        let clip = ClipNode::new(1000, 1000, 0, Arc::new(AudioBuffer::new(1000, vec![vec![0.5; 1000]])));
        let graph = Graph::new(vec![TrackNode::new([1.0, 1.0], vec![clip], Arc::new(PeakMeter::default()))], 1.0);
        let out = render_offline(graph, 1000, 120.0, 4, 0.5, 2.5);
        assert_eq!(out.frames(), 2000);
        assert_eq!(out.channels[0][499], 0.0);
        assert_eq!(out.channels[0][500], 0.5);
        assert_eq!(out.channels[1][1499], 0.5);
        assert_eq!(out.channels[0][1500], 0.0);
    }

    /// Holds its input back by its latency, like a look-ahead plugin.
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

    #[test]
    fn plugin_latency_is_cut_from_the_render() {
        let clip = ClipNode::new(1000, 1000, 0, Arc::new(AudioBuffer::new(1000, vec![vec![0.5; 1000]])));
        let mut track = TrackNode::new([1.0, 1.0], vec![clip], Arc::new(PeakMeter::default()));
        let fx = soniq_dsp::AudioFx::External(Box::new(Late(crate::graph::Delay::new(300), 300)));
        track.fx.push(crate::graph::FxNode::new("late".into(), fx, None, soniq_dsp::ParamBank::default()));
        let mut graph = Graph::new(vec![track], 1.0);
        graph.compensate();
        // The clip lands where it would without the plugin, though the
        // plugin plays it 0.3 s late.
        let out = render_offline(graph, 1000, 120.0, 4, 0.5, 2.5);
        assert_eq!(out.frames(), 2000);
        assert_eq!(out.channels[0][499], 0.0);
        assert_eq!(out.channels[0][500], 0.5);
        assert_eq!(out.channels[1][1499], 0.5);
        assert_eq!(out.channels[0][1500], 0.0);
    }
}
