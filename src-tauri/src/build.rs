//! Turning a track's devices into what the engine plays: effects and
//! instruments with their parameter banks, clip notes, and the automation
//! lanes and modulators that move parameters while the song plays.

use std::collections::HashMap;
use std::sync::Arc;

use soniq_dsp::{AudioFx, External, Instrument, Key, MacroLink, MidiFx, PadChains, PadFx, ParamBank, Params, Rack, RackLayer, SampleSlot, Warper};
use soniq_engine::{ClipNode, ClipWarp, Dest, FadeShape, Follow, FxNode, InstrumentNode, LaneNode, ModNode, ModSource, ScheduledNote, SlotNode, db_to_gain, track_key};
use soniq_project::{AutoTarget, Clip, ClipContent, Device, DeviceParam, DeviceRole, EnvPoint, FadeCurve, MODULATOR_TYPES, PLUGIN_TYPE, TempoMap, Track, WarpMode};

use crate::session::Pool;

/// A device's parameter values, for the engine to hold and change.
pub fn bank(device: &Device) -> ParamBank {
    ParamBank::new(device.params.iter().map(|p| (p.id.clone(), p.value)))
}

/// The engine's node for a plugin device (whether it is an instrument),
/// while its plugin is running.
pub type PluginNode<'a> = &'a dyn Fn(&Device, bool) -> Option<Box<dyn External>>;

/// The audio effects of a chain that are switched on, in order, each with
/// the index of the track its sidechain listens to.
pub fn fx_chain(devices: &[Device], rate: u32, tracks: &HashMap<&str, usize>, plugin: PluginNode) -> Vec<FxNode> {
    devices
        .iter()
        .filter(|d| d.on && d.role == DeviceRole::Effect)
        .filter_map(|d| {
            let bank = bank(d);
            let fx = if d.kind == PLUGIN_TYPE { AudioFx::External(plugin(d, false)?) } else { AudioFx::build(&d.kind, rate, &bank)? };
            Some(FxNode::new(d.id.clone(), fx, d.sidechain.as_deref().and_then(|s| tracks.get(s).copied()), bank))
        })
        .collect()
}

/// The samples a device plays, at the engine rate, with the transients a
/// sampler slicing by them needs.
fn sample_slots(device: &Device, params: &dyn Params, pool: &Pool) -> Vec<SampleSlot> {
    let slicing = device.kind == "sampler" && params.or("mode", 0.0) == 3.0 && params.or("sliceBy", 0.0) == 1.0;
    let sensitivity = params.or("sensitivity", 50.0).clamp(0.0, 100.0).round() as u32;
    device
        .samples
        .iter()
        .filter_map(|s| {
            let asset = pool.get(&s.asset)?;
            let onsets = if slicing { pool.onsets(&s.asset, sensitivity) } else { None };
            Some(SampleSlot { key: s.key, buffer: Arc::clone(&asset.playback), onsets })
        })
        .collect()
}

/// An instrument rack's chains: each instrument with its own bank, and the
/// macros that turn its parameters.
fn rack(device: &Device, pool: &Pool, rate: u32) -> Instrument {
    let layers = device
        .chains
        .iter()
        .filter(|c| c.on && c.device.on)
        .filter_map(|c| {
            let bank = bank(&c.device);
            let instrument = Instrument::build(&c.device.kind, rate, &bank, &sample_slots(&c.device, &bank, pool))?;
            let links = device
                .macros
                .iter()
                .filter(|m| m.chain == c.id)
                .filter_map(|m| {
                    let p = c.device.params.iter().find(|p| p.id == m.param)?;
                    Some(MacroLink {
                        macro_index: usize::from(m.index),
                        param: bank.find(&m.param)?,
                        from: m.min,
                        to: m.max,
                        min: p.min,
                        max: p.max,
                        log: p.log == Some(true),
                        step: p.step,
                    })
                })
                .collect();
            Some(RackLayer {
                id: c.id.clone(),
                instrument,
                bank,
                links,
                keys: (c.key_low, c.key_high),
                velocities: (c.vel_low, c.vel_high),
                volume: c.volume,
                pan: c.pan as f32,
            })
        })
        .collect();
    Instrument::Rack(Rack::new(layers, &bank(device)))
}

/// A Drum Rack's pad effects and returns, as the engine runs them.
fn pad_chains(device: &Device, rate: u32) -> PadChains {
    let mut chains = PadChains::default();
    for chain in &device.pad_chains {
        let fx: Vec<PadFx> = chain
            .devices
            .iter()
            .filter(|d| d.on && d.role == DeviceRole::Effect && d.kind != PLUGIN_TYPE)
            .filter_map(|d| {
                // Nothing inside the rack makes up for a delay: the EQ stays minimum phase here.
                let bank = bank(d);
                let params = |id: &str| if id == "phase" { Some(0.0) } else { bank.get(id) };
                Some(PadFx { id: d.id.clone(), fx: AudioFx::build(&d.kind, rate, &params)? })
            })
            .collect();
        match chain.key.as_str() {
            "A" => chains.returns[0] = fx,
            "B" => chains.returns[1] = fx,
            key => {
                if let Ok(note) = key.parse::<u8>() {
                    chains.pads.push((note, fx));
                }
            }
        }
    }
    chains
}

/// The track's instrument (the first one switched on) with its MIDI effects
/// and every note its timeline clips play, in samples. Notes past a clip's
/// end are cut there.
pub fn instrument_node(track: &Track, pool: &Pool, timeline: &Timeline, song_key: Key, plugin: PluginNode) -> Option<InstrumentNode> {
    let rate = timeline.rate as u32;
    let device = track.devices.iter().find(|d| d.role == DeviceRole::Instrument && d.on)?;
    let params = bank(device);
    let mut instrument = match device.kind.as_str() {
        "rack" => rack(device, pool, rate),
        PLUGIN_TYPE => Instrument::External(plugin(device, true)?),
        kind => Instrument::build(kind, rate, &params, &sample_slots(device, &params, pool))?,
    };
    if let Instrument::Drums(drums) = &mut instrument {
        drums.set_chains(pad_chains(device, rate));
    }
    let fx: Vec<(String, MidiFx)> = track
        .devices
        .iter()
        .filter(|d| d.on)
        .filter_map(|d| MidiFx::build(&d.kind, &|id: &str| d.param(id), song_key).map(|fx| (d.id.clone(), fx)))
        .collect();
    let notes: Vec<ScheduledNote> = track.timeline().flat_map(|clip| clip_notes(clip, |beats| timeline.at(clip.start + beats))).collect();
    Some(InstrumentNode::new(device.id.clone(), instrument, &notes).with_fx(fx).with_bank(params))
}

/// Where beats fall at the engine rate: on the song's tempo track, and in
/// session clips, which loop at the song's starting tempo.
pub struct Timeline<'a> {
    pub tempo: &'a TempoMap,
    pub rate: f64,
    pub beats_per_bar: u32,
}

impl Timeline<'_> {
    /// The sample a timeline beat falls on.
    pub fn at(&self, beats: f64) -> u64 {
        (self.tempo.seconds_at(beats.max(0.0)) * self.rate).round() as u64
    }

    /// Samples per beat inside a session clip's loop.
    pub fn loop_beat(&self) -> f64 {
        self.rate * 60.0 / self.tempo.bpm_at(0.0)
    }

    /// The tempo track for the audio thread.
    pub fn engine_map(&self) -> soniq_engine::TempoMap {
        soniq_engine::TempoMap::new(self.tempo.segments().iter().map(|s| (s.beat, s.seconds, s.bpm, s.slope)), self.rate)
    }
}

/// A MIDI clip's notes in samples: `at` turns beats into the clip into the
/// sample they play at. Notes past the clip's end are cut there.
fn clip_notes(clip: &Clip, at: impl Fn(f64) -> u64) -> Vec<ScheduledNote> {
    let ClipContent::Midi { length, notes, .. } = &clip.content else { return Vec::new() };
    notes
        .iter()
        .filter(|n| n.start < *length)
        .map(|n| ScheduledNote { start: at(n.start), end: at((n.start + n.length).min(*length)), pitch: n.pitch, velocity: n.velocity as f32 })
        .collect()
}

/// An audio clip as the engine plays it: at its place on the timeline, or
/// (`in_loop`) from 0 in a session slot. A warped clip spans its beats and
/// stretches to fit them.
pub fn audio_node(clip: &Clip, pool: &Pool, timeline: &Timeline, in_loop: bool) -> Option<ClipNode> {
    let ClipContent::Audio { asset, offset, duration, gain, fade_in, fade_out, warp, curve, reverse, transpose, envelope } = &clip.content else { return None };
    // Backwards or at another pitch, it plays audio made from the asset for that.
    let cents = (transpose * 100.0).round() as i32;
    let buffer = pool.variant(asset, cents, *reverse)?;
    let rate = timeline.rate;
    let samples = |sec: f64| (sec.max(0.0) * rate).round() as u64;
    let start = if in_loop { 0 } else { timeline.at(clip.start) };
    let len = match warp {
        Some(w) if in_loop => (duration * w.bpm / 60.0 * timeline.loop_beat()).round() as u64,
        Some(_) => timeline.at(clip.end(timeline.tempo)).saturating_sub(start),
        None => samples(*duration),
    };
    // Backwards, its stretch of the file starts from the stretch's end in the reversed audio.
    let from = if *reverse { buffer.frames() as f64 / rate - offset - duration } else { *offset };
    let mut node = ClipNode::new(start, len, samples(from), buffer);
    node.gain = db_to_gain(*gain);
    node.env = gain_envelope(envelope, |beats| if in_loop { (beats * timeline.loop_beat()).round() as u64 } else { timeline.at(clip.start + beats).saturating_sub(start) });
    node.fade_in = samples(*fade_in);
    node.fade_out = samples(*fade_out);
    node.curve = match curve {
        None => FadeShape::Standard,
        Some(FadeCurve::Linear) => FadeShape::Linear,
        Some(FadeCurve::EqualPower) => FadeShape::EqualPower,
        Some(FadeCurve::SCurve) => FadeShape::SCurve,
    };
    node.key = track_key(&clip.id);
    if let Some(w) = warp {
        let mode = match w.mode {
            WarpMode::Beats => soniq_dsp::WarpMode::Beats,
            WarpMode::Tones => soniq_dsp::WarpMode::Tones,
            WarpMode::Repitch => soniq_dsp::WarpMode::Repitch,
        };
        let onsets = (w.mode == WarpMode::Beats).then(|| pool.variant_onsets(asset, cents, *reverse)).flatten();
        let start_beat = if in_loop { 0.0 } else { clip.start };
        node = node.with_warp(ClipWarp::new(Warper::new(mode, onsets, rate as u32), start_beat, rate * 60.0 / w.bpm));
    }
    Some(node)
}

/// A clip's gain envelope for the engine: each breakpoint at its sample from
/// the clip's start (`at` turns beats into those), as a gain. The lines
/// between points are straight in dB, so each is cut into short pieces.
fn gain_envelope(points: &[EnvPoint], at: impl Fn(f64) -> u64) -> Vec<(u64, f32)> {
    const PIECES: usize = 8;
    let mut out = Vec::with_capacity(points.len() * PIECES);
    for (i, p) in points.iter().enumerate() {
        if let Some(prev) = i.checked_sub(1).map(|j| points[j]) {
            for k in 1..PIECES {
                let f = k as f64 / PIECES as f64;
                out.push((at(prev.time + (p.time - prev.time) * f), db_to_gain(prev.gain + (p.gain - prev.gain) * f)));
            }
        }
        out.push((at(p.time), db_to_gain(p.gain)));
    }
    out
}

/// The track's session clips, sorted by scene, each looping over its length.
pub fn slot_nodes(track: &Track, pool: &Pool, timeline: &Timeline) -> Vec<SlotNode> {
    let beat = timeline.loop_beat();
    let mut slots: Vec<SlotNode> = track
        .clips
        .iter()
        .filter_map(|clip| {
            let scene = clip.slot?;
            let slot = match &clip.content {
                ClipContent::Midi { length, .. } => SlotNode::midi(scene, (length * beat).round() as u64, &clip_notes(clip, |b| (b.max(0.0) * beat).round() as u64)),
                ClipContent::Audio { .. } => SlotNode::audio(scene, audio_node(clip, pool, timeline, true)?),
            };
            let follow = clip.follow.map(|f| (follow(f.action), (f.bars * f64::from(timeline.beats_per_bar) * beat).round() as u64));
            Some(slot.with_follow(follow))
        })
        .collect();
    slots.sort_by_key(|s| s.scene);
    slots
}

fn follow(action: soniq_project::Follow) -> Follow {
    use soniq_project::Follow as F;
    match action {
        F::Stop => Follow::Stop,
        F::Again => Follow::Again,
        F::Next => Follow::Next,
        F::Previous => Follow::Previous,
        F::First => Follow::First,
        F::Last => Follow::Last,
        F::Any => Follow::Any,
        F::Other => Follow::Other,
    }
}

/// LFO sync choices in beats (the catalog's options; bars follow the time signature).
fn lfo_sync(choice: usize, beats_per_bar: f64) -> Option<f64> {
    [None, Some(0.25), Some(0.5), Some(1.0), Some(2.0), Some(beats_per_bar), Some(2.0 * beats_per_bar), Some(4.0 * beats_per_bar)].get(choice).copied().flatten()
}

/// The track's automation lanes and modulators, aimed at the effects and
/// instrument the graph built. Targets that are switched off are skipped.
pub fn controls(track: &Track, fx: &[FxNode], instrument: Option<&InstrumentNode>, timeline: &Timeline) -> (Vec<LaneNode>, Vec<ModNode>) {
    let (rate, beats_per_bar) = (timeline.rate, timeline.beats_per_bar);
    let dest_of = |device: &str, param: &str| -> Option<(Dest, &DeviceParam)> {
        let spec = track.devices.iter().find(|d| d.id == device)?.params.iter().find(|p| p.id == param)?;
        let dest = match instrument.filter(|i| i.id == device) {
            Some(inst) => Dest::Instrument { param: inst.bank.find(param)? },
            None => {
                let i = fx.iter().position(|f| f.id == device)?;
                Dest::Fx { fx: i, param: fx[i].bank.find(param)? }
            }
        };
        Some((dest, spec))
    };
    let lanes: Vec<LaneNode> = track
        .automation
        .iter()
        .filter_map(|lane| {
            let dest = match &lane.target {
                AutoTarget::Volume => Dest::Volume,
                AutoTarget::Pan => Dest::Pan,
                AutoTarget::Send { index } => Dest::Send(usize::from(*index)),
                AutoTarget::Param { device, param } => dest_of(device, param)?.0,
            };
            let points = lane.points.iter().map(|p| (timeline.at(p.time), p.value)).collect();
            Some(LaneNode { dest, points })
        })
        .collect();
    let ms = |p: &dyn Params, id: &str, d: f64| (-1.0 / (p.or(id, d).max(0.1) * 0.001 * rate)).exp();
    let mods = track
        .devices
        .iter()
        .filter(|d| d.on && MODULATOR_TYPES.contains(&d.kind.as_str()))
        .filter_map(|d| {
            let target = d.modulates.as_ref()?;
            let (dest, spec) = dest_of(&target.device, &target.param)?;
            let p = |id: &str| d.param(id);
            let source = if d.kind == "lfo" {
                ModSource::Lfo {
                    shape: p.or("shape", 0.0).round().clamp(0.0, 5.0) as u8,
                    rate: p.or("rate", 1.0),
                    sync: lfo_sync(p.or("sync", 0.0).round().max(0.0) as usize, f64::from(beats_per_bar)),
                    phase: p.or("phase", 0.0) / 360.0,
                }
            } else {
                ModSource::Follower { gain: 10f64.powf(p.or("gain", 0.0) / 20.0), attack: ms(&p, "attack", 10.0), release: ms(&p, "release", 200.0) }
            };
            let mut node = ModNode::new(dest, source, p.or("depth", 50.0) / 100.0, spec.value, (spec.min, spec.max, spec.log == Some(true)));
            node.lane = lanes.iter().position(|l| l.dest == dest);
            Some(node)
        })
        .collect();
    (lanes, mods)
}
