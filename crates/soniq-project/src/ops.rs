//! Every change the UI can make to a project, as data.

use serde::Deserialize;

use crate::{
    Asset, AutoPoint, AutoTarget, AutomationLane, Chain, ChainOwner, Clip, FX_RACK, FxChain, ClipContent, ControllerMapping, Device, DeviceParam, DeviceRole, EnvPoint, FollowAction, History,
    MAX_BPM, MIN_BPM, MODULATOR_TYPES, MappingTarget, Marker, MeterChange, Monitor, Note, PLUGIN_TYPE, ParamRef, Project, SIDECHAIN_TYPES, SampleRef, TakeLane, TempoMap, TempoPoint, Track,
    TrackKind, TrackRole, Warp, FadeCurve, find_device_mut,
};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TrackPatch {
    pub name: Option<String>,
    pub volume: Option<f64>,
    pub pan: Option<f64>,
    pub mute: Option<bool>,
    pub solo: Option<bool>,
    pub arm: Option<bool>,
    pub sends: Option<[f64; 2]>,
    pub input: Option<String>,
    pub monitor: Option<Monitor>,
    pub role: Option<TrackRole>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ReturnPatch {
    pub volume: Option<f64>,
    pub pan: Option<f64>,
    pub mute: Option<bool>,
    pub pre_fader: Option<bool>,
}

/// Changes to one chain of an instrument rack.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChainPatch {
    pub name: Option<String>,
    pub on: Option<bool>,
    pub key_low: Option<u8>,
    pub key_high: Option<u8>,
    pub vel_low: Option<u8>,
    pub vel_high: Option<u8>,
    pub volume: Option<f64>,
    pub pan: Option<f64>,
}

// Ops come one at a time from the UI and are applied at once, never kept in
// bulk (the history keeps projects), so a large variant costs nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Op {
    SetTrack { id: String, patch: TrackPatch },
    SetReturn { id: String, patch: ReturnPatch },
    SetMaster { volume: f64 },
    AddTrack { kind: TrackKind },
    RemoveTrack { id: String },
    /// Device ops name the chain by its owner: a track id, a return id, or "master".
    AddDevice { track: String, device: Device },
    RemoveDevice { track: String, device: String },
    ToggleDevice { track: String, device: String },
    SetParam { track: String, device: String, param: String, value: f64 },
    /// Sets which of a plugin's controls its device shows (and automation
    /// can reach). Automation and mappings on controls taken away go too.
    SetPluginParams { track: String, device: String, params: Vec<DeviceParam> },
    /// Saves the mixer as it is as a new mix (named "Mix 1", "Mix 2"… without a name).
    SaveMix { name: Option<String> },
    /// Sets the mixer as a saved mix has it.
    RecallMix { id: String },
    /// Saves the mixer as it is into a saved mix.
    UpdateMix { id: String },
    RenameMix { id: String, name: String },
    RemoveMix { id: String },
    /// A/B: the first time, keeps the mixer as it is as "A" and goes on as
    /// "B"; then each time swaps the mixer with the other one.
    SwapMix,
    /// Ends A/B, keeping the mix that plays.
    KeepMix,
    /// The track plays `asset` (its clips and devices rendered, added to the
    /// song) while its devices rest.
    FreezeTrack { track: String, asset: Asset },
    UnfreezeTrack { track: String },
    /// A frozen track becomes an audio track playing its rendered sound:
    /// its clips and devices go.
    FlattenTrack { track: String },
    /// Lets a compressor or gate listen to another track (`None`: its own input).
    SetSidechain {
        track: String,
        device: String,
        #[serde(default)]
        source: Option<String>,
    },
    SetTempo { bpm: f64 },
    /// Replaces the tempo track (sorted); none plays the song at its tempo throughout.
    SetTempoPoints { points: Vec<TempoPoint> },
    /// Makes an audio clip follow the song's tempo, or play at its own speed (`None`).
    SetWarp {
        clip: String,
        #[serde(default)]
        warp: Option<Warp>,
    },
    SetTimeSignature { beats_per_bar: u32 },
    SetKey { key: String },
    DeleteClip { clip: String },
    /// Moves a clip to `start` (beats) on `track`, which must be the same kind.
    MoveClip { clip: String, track: String, start: f64 },
    /// Sets a clip's timeline start and length (beats). Moving the start
    /// reveals or hides audio at the front, like dragging a clip's left edge.
    TrimClip {
        clip: String,
        start: f64,
        length: f64,
        /// Ripple editing: the clips after it on its track follow its end.
        #[serde(default)]
        ripple: bool,
    },
    /// Moves several timeline clips together: `delta` beats later (earlier
    /// if negative) and `tracks` tracks down (up if negative).
    MoveClips {
        clips: Vec<String>,
        delta: f64,
        #[serde(default)]
        tracks: i32,
    },
    /// Deletes several clips; with `ripple`, the clips after each close its gap.
    DeleteClips {
        clips: Vec<String>,
        #[serde(default)]
        ripple: bool,
    },
    /// Copies several clips as a block right after them; with `ripple`, the
    /// clips after the block on their tracks move along to make room.
    DuplicateClips {
        clips: Vec<String>,
        #[serde(default)]
        ripple: bool,
    },
    /// The shape of the clips' fades and crossfades (`None`: the standard).
    SetFadeCurve {
        clips: Vec<String>,
        #[serde(default)]
        curve: Option<FadeCurve>,
    },
    /// Copies (or, with `cut`, moves) `start..end` seconds of an audio clip
    /// onto a new track just below, at the same time (Audacity's Duplicate
    /// and Split New). No audio is copied: the new clip plays the same file.
    ExtractToTrack {
        clip: String,
        start: f64,
        end: f64,
        #[serde(default)]
        cut: bool,
    },
    /// Cuts a clip in two at `at` (beats on the timeline).
    SplitClip { clip: String, at: f64 },
    /// Places a copy right after the clip; a linked copy of a MIDI clip shares
    /// its notes with it from then on.
    DuplicateClip {
        clip: String,
        #[serde(default)]
        linked: bool,
    },
    /// Lets a linked MIDI clip go its own way.
    UnlinkClip { clip: String },
    /// Fade lengths in seconds (audio clips).
    SetClipFades { clip: String, fade_in: f64, fade_out: f64 },
    /// A marker at `start` (beats), or a region when `end` is given.
    AddMarker { name: Option<String>, start: f64, end: Option<f64> },
    /// Renames or moves a marker; `end` only applies to regions.
    SetMarker { id: String, name: Option<String>, start: Option<f64>, end: Option<f64> },
    RemoveMarker { id: String },
    /// A MIDI clip on a MIDI track, `length` beats long: on the timeline, or
    /// in a scene's slot (replacing what is there).
    AddMidiClip {
        track: String,
        start: f64,
        length: f64,
        #[serde(default)]
        notes: Vec<Note>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        slot: Option<usize>,
    },
    /// Replaces an automation lane's points (sorted, kept inside the target's
    /// range); no points removes the lane.
    SetAutomation { track: String, target: AutoTarget, points: Vec<AutoPoint> },
    /// Points a modulator at a parameter of another device on its track (`None`: at nothing).
    SetModTarget {
        track: String,
        device: String,
        #[serde(default)]
        target: Option<ParamRef>,
    },
    /// Puts tracks into a new group track, placed where the first of them was.
    GroupTracks { tracks: Vec<String> },
    /// Takes a group's tracks out of it and deletes the group track.
    Ungroup { group: String },
    /// Moves a track into a group (after its last track) or out of one (`None`).
    SetTrackGroup {
        track: String,
        #[serde(default)]
        group: Option<String>,
    },
    /// Copies a timeline clip into a scene's slot on its track, replacing what is there.
    PutInSlot { clip: String, scene: usize },
    ClearSlot { track: String, scene: usize },
    /// Plays audio clips backwards (or forwards again).
    ReverseClips { clips: Vec<String> },
    /// An audio clip's pitch in semitones; its length stays.
    SetClipTranspose { clip: String, semitones: f64 },
    /// An audio clip's gain over time: breakpoints in beats from its start, in dB.
    SetClipEnvelope { clip: String, points: Vec<EnvPoint> },
    /// A time signature from bar `bar` (0-based, 1 or later) on; `None` takes the change there away.
    SetMeter {
        bar: u32,
        #[serde(default)]
        beats_per_bar: Option<u32>,
    },
    /// Moves a region and everything in it (clips, automation, markers) so it
    /// starts at `to` (beats): the gap it leaves closes, and the song opens up for it.
    MoveSection { marker: String, to: f64 },
    /// Inserts a copy of a region and everything in it right after it.
    DuplicateSection { marker: String },
    /// Removes a region and everything in it, and closes the gap.
    DeleteSection { marker: String },
    /// Comping: puts a stretch (beats) of a take on the track's timeline, over what is there.
    CompTake { track: String, lane: String, start: f64, end: f64 },
    DeleteTake { track: String, lane: String },
    /// Removes every take of the track; the timeline keeps what was comped.
    ClearTakes { track: String },
    /// Writes a scene's clips into the arrangement at `at` (beats), each
    /// repeated for as long as the scene's longest clip.
    SceneToArrangement { scene: usize, at: f64 },
    /// What a session clip does after it has played a while (`None`: keeps looping).
    SetFollow {
        clip: String,
        #[serde(default)]
        follow: Option<FollowAction>,
    },
    /// Session clips start on the next multiple of this many beats (0: at once).
    SetLaunchQuantize { beats: f64 },
    AddScene,
    /// Replaces a MIDI clip's notes. `label` names the undo step ("Move notes").
    SetNotes {
        clip: String,
        notes: Vec<Note>,
        #[serde(default)]
        label: Option<String>,
    },
    RenameClip { clip: String, name: String },
    /// Loads an asset (already in the project) into a drum pad (`key` is its
    /// note) or a sampler (`key` 0); `None` clears it.
    SetDeviceSample { track: String, device: String, key: u8, asset: Option<String> },
    /// Adds a chain playing a new instrument of type `kind` to an instrument
    /// rack, or an empty chain to an audio effect rack (which takes no `kind`,
    /// and may sit on a return or the master too).
    AddChain {
        track: String,
        device: String,
        #[serde(default)]
        kind: String,
    },
    RemoveChain { track: String, device: String, chain: String },
    SetChain { track: String, device: String, chain: String, patch: ChainPatch },
    /// Points macro `index` (0–7) at a chain's parameter, swept from `min` to
    /// `max` (its whole range unless given); no `index` unmaps it.
    MapMacro {
        track: String,
        device: String,
        chain: String,
        /// In an audio effect rack: the effect in the chain.
        #[serde(default)]
        inner: Option<String>,
        param: String,
        #[serde(default)]
        index: Option<u8>,
        #[serde(default)]
        min: Option<f64>,
        #[serde(default)]
        max: Option<f64>,
    },
    /// Points a MIDI controller (CC `cc`, on `channel` or any) at `target`,
    /// replacing whatever that control or that target was mapped to before.
    AddMapping {
        #[serde(default)]
        channel: Option<u8>,
        cc: u8,
        target: MappingTarget,
    },
    RemoveMapping { id: String },
}

/// Shortest clip the editing ops will produce, in beats.
const MIN_CLIP_BEATS: f64 = 1.0 / 64.0;

#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error("No track with id {0}")]
    NoTrack(String),
    #[error("No device with id {0}")]
    NoDevice(String),
    #[error("No clip with id {0}")]
    NoClip(String),
    #[error("{0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Default)]
pub struct Applied {
    pub label: String,
    /// Id of something the op created (a track), so the UI can select it.
    pub created: Option<String>,
    /// The tempo changed, so timeline positions in samples must be rebuilt.
    pub tempo_changed: bool,
}

/// Applies `op` to `project` and records an undo step. On error nothing changes.
pub fn apply(project: &mut Project, history: &mut History, op: Op) -> Result<Applied, OpError> {
    let before = project.clone();
    let key = coalesce_key(&op);
    match mutate(project, op) {
        Ok(applied) => {
            history.record(before, &applied.label, key.as_deref());
            Ok(applied)
        }
        Err(err) => {
            *project = before;
            Err(err)
        }
    }
}

/// Continuous controls share a key per target so a whole drag is one undo step.
fn coalesce_key(op: &Op) -> Option<String> {
    match op {
        Op::SetTrack { id, patch } if patch.volume.is_some() || patch.pan.is_some() || patch.sends.is_some() => {
            Some(format!("track:{id}:{}", if patch.volume.is_some() { "vol" } else if patch.pan.is_some() { "pan" } else { "sends" }))
        }
        Op::SetReturn { id, patch } if patch.volume.is_some() || patch.pan.is_some() => Some(format!("return:{id}")),
        Op::SetMaster { .. } => Some("master".into()),
        Op::SetParam { device, param, .. } => Some(format!("param:{device}:{param}")),
        Op::SetTempo { .. } => Some("tempo".into()),
        Op::SetTempoPoints { .. } => Some("tempo-track".into()),
        Op::SetClipFades { clip, .. } => Some(format!("fades:{clip}")),
        Op::SetClipEnvelope { clip, .. } => Some(format!("env:{clip}")),
        Op::SetClipTranspose { clip, .. } => Some(format!("transpose:{clip}")),
        Op::SetChain { chain, patch, .. } if patch.name.is_none() && patch.on.is_none() => Some(format!("chain:{chain}")),
        // Dragging a breakpoint is one undo step.
        Op::SetAutomation { track, target, .. } => Some(format!("auto:{track}:{target:?}")),
        _ => None,
    }
}

fn track_mut<'a>(project: &'a mut Project, id: &str) -> Result<&'a mut Track, OpError> {
    project.track_mut(id).ok_or_else(|| OpError::NoTrack(id.to_string()))
}

/// The device chain of a track, a return or the master.
fn chain_mut<'a>(project: &'a mut Project, owner: &str) -> Result<(ChainOwner, &'a mut Vec<Device>), OpError> {
    project.chain_mut(owner).ok_or_else(|| OpError::NoTrack(owner.to_string()))
}

/// A device in a chain, or the instrument of one of its rack chains.
fn device_mut<'a>(project: &'a mut Project, owner: &str, id: &str) -> Result<&'a mut Device, OpError> {
    let (_, devices) = chain_mut(project, owner)?;
    find_device_mut(devices, id).ok_or_else(|| OpError::NoDevice(id.to_string()))
}

/// An instrument rack on the track.
fn rack_mut<'a>(track: &'a mut Track, id: &str) -> Result<&'a mut Device, OpError> {
    let device = track.devices.iter_mut().find(|d| d.id == id).ok_or_else(|| OpError::NoDevice(id.to_string()))?;
    if device.kind != "rack" {
        return Err(OpError::Invalid(format!("{} has no chains", device.name)));
    }
    Ok(device)
}

/// A clip on the timeline: session clips stay in their slots.
/// Several timeline clips, as (track, clip) indexes in order.
fn timeline_clips(project: &Project, ids: &[String]) -> Result<Vec<(usize, usize)>, OpError> {
    if ids.is_empty() {
        return Err(OpError::Invalid("Select some clips first.".into()));
    }
    let mut found: Vec<(usize, usize)> = ids.iter().map(|id| timeline_clip(project, id)).collect::<Result<_, _>>()?;
    found.sort_unstable();
    found.dedup();
    Ok(found)
}

/// Ripple: the timeline clips on `track` that start at or after `from`
/// (other than `except`) move by `by` beats.
fn shift_after(track: &mut Track, from: f64, by: f64, except: &[String]) {
    for c in track.clips.iter_mut().filter(|c| c.slot.is_none() && c.start >= from - 1e-9 && !except.contains(&c.id)) {
        c.start = (c.start + by).max(0.0);
    }
}

fn timeline_clip(project: &Project, id: &str) -> Result<(usize, usize), OpError> {
    let (ti, ci) = project.find_clip(id).ok_or_else(|| OpError::NoClip(id.to_string()))?;
    if project.tracks[ti].clips[ci].slot.is_some() {
        return Err(OpError::Invalid("That clip waits in the session view; put it on the timeline first.".into()));
    }
    Ok((ti, ci))
}

/// The range an automation lane's values stay in.
fn target_range(track: &Track, target: &AutoTarget) -> Result<(f64, f64), OpError> {
    Ok(match target {
        AutoTarget::Volume => (-60.0, 6.0),
        AutoTarget::Pan => (-1.0, 1.0),
        AutoTarget::Send { index } if usize::from(*index) < track.sends.len() => (0.0, 1.0),
        AutoTarget::Send { .. } => return Err(OpError::Invalid("There is no such send".into())),
        AutoTarget::Param { device, param } => {
            let d = track.devices.iter().find(|d| d.id == *device).ok_or_else(|| OpError::NoDevice(device.clone()))?;
            let p = d.params.iter().find(|p| p.id == *param).ok_or_else(|| OpError::Invalid(format!("No parameter {param}")))?;
            (p.min, p.max)
        }
    })
}

fn chain_index(rack: &Device, id: &str) -> Result<usize, OpError> {
    rack.chains.iter().position(|c| c.id == id).ok_or_else(|| OpError::Invalid("That chain no longer exists".into()))
}

/// An audio effect rack in a track's, a return's or the master's chain; `None` when the device is another kind.
fn fx_rack_mut<'a>(project: &'a mut Project, owner: &str, id: &str) -> Result<Option<&'a mut Device>, OpError> {
    let (_, devices) = chain_mut(project, owner)?;
    let device = devices.iter_mut().find(|d| d.id == id).ok_or_else(|| OpError::NoDevice(id.to_string()))?;
    Ok((device.kind == FX_RACK).then_some(device))
}

fn fx_chain_index(rack: &Device, id: &str) -> Result<usize, OpError> {
    rack.fx_chains.iter().position(|c| c.id == id).ok_or_else(|| OpError::Invalid("That chain no longer exists".into()))
}

/// Changes an effect rack chain's name, switch, level and pan (it has no zones).
fn set_fx_chain(c: &mut FxChain, patch: ChainPatch) -> Result<(), OpError> {
    if let Some(name) = patch.name {
        let name = name.trim();
        if name.is_empty() {
            return Err(OpError::Invalid("A chain needs a name".into()));
        }
        c.name = name.to_string();
    }
    if let Some(on) = patch.on {
        c.on = on;
    }
    if let Some(v) = patch.volume {
        c.volume = v.clamp(-60.0, 6.0);
    }
    if let Some(v) = patch.pan {
        c.pan = v.clamp(-1.0, 1.0);
    }
    Ok(())
}

fn mutate(project: &mut Project, op: Op) -> Result<Applied, OpError> {
    let label = |s: String| Applied { label: s, ..Default::default() };
    match op {
        Op::SetTrack { id, patch } => {
            let track = track_mut(project, &id)?;
            let what = if let Some(v) = patch.volume {
                track.volume = v.clamp(-60.0, 6.0);
                "volume"
            } else if let Some(v) = patch.pan {
                track.pan = v.clamp(-1.0, 1.0);
                "pan"
            } else if let Some(v) = patch.sends {
                track.sends = [v[0].clamp(0.0, 1.0), v[1].clamp(0.0, 1.0)];
                "send"
            } else if let Some(v) = patch.mute {
                track.mute = v;
                if v { "mute" } else { "unmute" }
            } else if let Some(v) = patch.solo {
                track.solo = v;
                if v { "solo" } else { "unsolo" }
            } else if let Some(v) = patch.arm {
                track.arm = v;
                if v { "arm" } else { "disarm" }
            } else if let Some(name) = patch.name {
                let name = name.trim();
                if name.is_empty() {
                    return Err(OpError::Invalid("A track needs a name".into()));
                }
                track.name = name.to_string();
                "rename"
            } else if let Some(input) = patch.input {
                track.input = input;
                "input"
            } else if let Some(monitor) = patch.monitor {
                track.monitor = monitor;
                "monitoring"
            } else if let Some(role) = patch.role {
                track.role = role;
                match role {
                    TrackRole::Speaker => "as a speaker",
                    TrackRole::Music => "as music",
                    TrackRole::None => "with no podcast role",
                }
            } else {
                return Err(OpError::Invalid("Empty track change".into()));
            };
            Ok(label(format!("{} {what}", track.name)))
        }
        Op::SetReturn { id, patch } => {
            let ret = project.returns.iter_mut().find(|r| r.id == id).ok_or_else(|| OpError::NoTrack(id.clone()))?;
            if let Some(v) = patch.volume {
                ret.volume = v.clamp(-60.0, 6.0);
            }
            if let Some(v) = patch.pan {
                ret.pan = v.clamp(-1.0, 1.0);
            }
            if let Some(v) = patch.mute {
                ret.mute = v;
            }
            if let Some(v) = patch.pre_fader {
                ret.pre_fader = v;
            }
            Ok(label(format!("{} change", ret.name)))
        }
        Op::SetMaster { volume } => {
            project.master_volume = volume.clamp(-60.0, 6.0);
            Ok(label("Master volume".into()))
        }
        Op::AddTrack { kind } => {
            let count = project.tracks.iter().filter(|t| t.kind == kind).count() + 1;
            let (name, what) = match kind {
                TrackKind::Audio => (format!("Audio {count}"), "audio"),
                TrackKind::Midi => (format!("MIDI {count}"), "MIDI"),
                TrackKind::Group => (format!("Group {count}"), "group"),
            };
            let id = crate::new_id("t");
            let track = Track::new(id.clone(), name, kind, project.next_color());
            project.tracks.push(track);
            Ok(Applied { label: format!("Add {what} track"), created: Some(id), tempo_changed: false })
        }
        Op::RemoveTrack { id } => {
            let index = project.tracks.iter().position(|t| t.id == id).ok_or_else(|| OpError::NoTrack(id.clone()))?;
            let track = project.tracks.remove(index);
            // A deleted group's tracks stay, outside any group.
            for t in project.tracks.iter_mut().filter(|t| t.group.as_deref() == Some(id.as_str())) {
                t.group = None;
            }
            project.prune_assets();
            project.prune_mappings();
            project.prune_sidechains();
            Ok(label(format!("Delete {}", track.name)))
        }
        Op::AddDevice { track, mut device } => {
            let (owner, devices) = chain_mut(project, &track)?;
            let midi_fx = crate::catalog::is_midi_fx(&device.kind);
            if (device.role == DeviceRole::Instrument || midi_fx) && owner != ChainOwner::MidiTrack {
                return Err(OpError::Invalid(format!("{} needs a MIDI track", device.name)));
            }
            if MODULATOR_TYPES.contains(&device.kind.as_str()) && matches!(owner, ChainOwner::Return | ChainOwner::Master) {
                return Err(OpError::Invalid(format!("{} works on tracks", device.name)));
            }
            let inside = matches!(owner, ChainOwner::Pad | ChainOwner::Rack);
            let plain = device.role == DeviceRole::Effect && !midi_fx && !MODULATOR_TYPES.contains(&device.kind.as_str()) && device.kind != PLUGIN_TYPE && device.kind != FX_RACK;
            if inside && !plain {
                let host = if owner == ChainOwner::Pad { "Drum pads" } else { "Rack chains" };
                return Err(OpError::Invalid(format!("{host} take built-in audio effects; {} goes on the track", device.name)));
            }
            device.id = crate::new_id("dev");
            // A copied rack's chains and effects are new ones too.
            let mut renamed: Vec<(String, String)> = Vec::new();
            for chain in &mut device.fx_chains {
                let id = crate::new_id("ch");
                renamed.push((std::mem::replace(&mut chain.id, id.clone()), id));
                for inner in &mut chain.devices {
                    let id = crate::new_id("dev");
                    renamed.push((std::mem::replace(&mut inner.id, id.clone()), id));
                }
            }
            if device.kind == FX_RACK {
                let now = |old: &str| renamed.iter().find(|(was, _)| was == old).map(|(_, new)| new.clone());
                for m in &mut device.macros {
                    m.chain = now(&m.chain).unwrap_or_default();
                    m.device = m.device.as_deref().and_then(now);
                }
            }
            device.sidechain = None;
            device.modulates = None;
            let name = device.name.clone();
            // Notes flow through MIDI effects, then the instrument, then audio effects.
            let after_midi_fx = devices.iter().take_while(|d| crate::catalog::is_midi_fx(&d.kind)).count();
            if device.role == DeviceRole::Instrument {
                // An instrument replaces the current one.
                devices.retain(|d| d.role != DeviceRole::Instrument);
                devices.insert(after_midi_fx, device);
            } else if midi_fx {
                devices.insert(after_midi_fx, device);
            } else {
                devices.push(device);
            }
            Ok(label(format!("Add {name}")))
        }
        Op::RemoveDevice { track, device } => {
            let (_, devices) = chain_mut(project, &track)?;
            let index = devices.iter().position(|d| d.id == device).ok_or(OpError::NoDevice(device))?;
            let removed = devices.remove(index);
            project.prune_pad_chains();
            project.prune_macros();
            project.prune_assets();
            project.prune_mappings();
            project.prune_targets();
            Ok(label(format!("Remove {}", removed.name)))
        }
        Op::ToggleDevice { track, device } => {
            let device = device_mut(project, &track, &device)?;
            device.on = !device.on;
            Ok(label(format!("{} {}", if device.on { "Enable" } else { "Bypass" }, device.name)))
        }
        Op::SaveMix { name } => {
            let id = crate::new_id("x");
            let count = project.mixes.len() + 1;
            let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("Mix {count}"));
            let mix = project.capture_mix(id.clone(), name.clone());
            project.mixes.push(mix);
            Ok(Applied { label: format!("Save mix “{name}”"), created: Some(id), tempo_changed: false })
        }
        Op::RecallMix { id } => {
            let mix = project.mixes.iter().find(|m| m.id == id).cloned().ok_or_else(|| OpError::Invalid("That mix no longer exists.".into()))?;
            project.apply_mix(&mix);
            Ok(label(format!("Recall mix “{}”", mix.name)))
        }
        Op::UpdateMix { id } => {
            let at = project.mixes.iter().position(|m| m.id == id).ok_or_else(|| OpError::Invalid("That mix no longer exists.".into()))?;
            let name = project.mixes[at].name.clone();
            project.mixes[at] = project.capture_mix(id, name.clone());
            Ok(label(format!("Update mix “{name}”")))
        }
        Op::RenameMix { id, name } => {
            let name = name.trim().to_string();
            if name.is_empty() {
                return Err(OpError::Invalid("Give the mix a name.".into()));
            }
            let mix = project.mixes.iter_mut().find(|m| m.id == id).ok_or_else(|| OpError::Invalid("That mix no longer exists.".into()))?;
            mix.name = name.clone();
            Ok(label(format!("Rename mix to “{name}”")))
        }
        Op::RemoveMix { id } => {
            let at = project.mixes.iter().position(|m| m.id == id).ok_or_else(|| OpError::Invalid("That mix no longer exists.".into()))?;
            let mix = project.mixes.remove(at);
            Ok(label(format!("Delete mix “{}”", mix.name)))
        }
        Op::SwapMix => match project.other_mix.take() {
            None => {
                project.other_mix = Some(project.capture_mix(crate::new_id("x"), "A".into()));
                Ok(label("Compare mixes A/B".into()))
            }
            Some(other) => {
                let now = if other.name == "A" { "B" } else { "A" };
                project.other_mix = Some(project.capture_mix(other.id.clone(), now.into()));
                project.apply_mix(&other);
                Ok(label(format!("Switch to mix {}", other.name)))
            }
        },
        Op::KeepMix => {
            let other = project.other_mix.take().ok_or_else(|| OpError::Invalid("No A/B comparison is running.".into()))?;
            let playing = if other.name == "A" { "B" } else { "A" };
            Ok(label(format!("Keep mix {playing}")))
        }
        Op::FreezeTrack { track, asset } => {
            let t = project.track_mut(&track).ok_or_else(|| OpError::NoTrack(track.clone()))?;
            if t.kind == TrackKind::Group {
                return Err(OpError::Invalid("Group tracks play their tracks: freeze those instead".into()));
            }
            t.frozen = Some(asset.id.clone());
            t.arm = false;
            let name = t.name.clone();
            if project.asset(&asset.id).is_none() {
                project.assets.push(asset);
            }
            project.prune_assets();
            Ok(label(format!("Freeze {name}")))
        }
        Op::UnfreezeTrack { track } => {
            let t = project.track_mut(&track).ok_or_else(|| OpError::NoTrack(track.clone()))?;
            t.frozen = None;
            let name = t.name.clone();
            project.prune_assets();
            Ok(label(format!("Unfreeze {name}")))
        }
        Op::FlattenTrack { track } => {
            let t = project.track(&track).ok_or_else(|| OpError::NoTrack(track.clone()))?;
            let asset = t.frozen.clone().ok_or_else(|| OpError::Invalid(format!("Freeze {} first", t.name)))?;
            let duration = project.asset(&asset).map(Asset::duration).ok_or_else(|| OpError::Invalid("The frozen audio is missing".into()))?;
            let t = project.track_mut(&track).ok_or_else(|| OpError::NoTrack(track.clone()))?;
            let name = t.name.clone();
            t.kind = TrackKind::Audio;
            t.devices.clear();
            t.clips = vec![Clip::audio(format!("{name} (flattened)"), asset, 0.0, 0.0, duration)];
            t.frozen = None;
            t.arm = false;
            project.prune_targets();
            project.prune_mappings();
            project.prune_assets();
            Ok(label(format!("Flatten {name}")))
        }
        Op::SetPluginParams { track, device, params } => {
            let device = device_mut(project, &track, &device)?;
            if device.kind != PLUGIN_TYPE {
                return Err(OpError::Invalid(format!("{} is not a plugin", device.name)));
            }
            // The Dry/Wet around a plugin effect is the engine's, not one of the plugin's controls: it stays.
            let wet = device.params.iter().find(|p| p.id == "wet").cloned();
            device.params = params;
            device.params.retain(|p| p.id != "wet");
            device.params.extend(wet);
            let name = device.name.clone();
            project.prune_targets();
            project.prune_mappings();
            Ok(label(format!("{name} controls")))
        }
        Op::SetSidechain { track, device, source } => {
            if let Some(src) = &source {
                if project.track(src).is_none() {
                    return Err(OpError::NoTrack(src.clone()));
                }
                if *src == track {
                    return Err(OpError::Invalid("A track can't be its own sidechain".into()));
                }
            }
            let what = source.as_deref().and_then(|s| project.track(s)).map(|t| t.name.clone());
            let device = device_mut(project, &track, &device)?;
            if !SIDECHAIN_TYPES.contains(&device.kind.as_str()) {
                return Err(OpError::Invalid(format!("{} has no sidechain", device.name)));
            }
            device.sidechain = source;
            Ok(label(match what {
                Some(name) => format!("{} listens to {name}", device.name),
                None => format!("{} listens to its own input", device.name),
            }))
        }
        Op::SetParam { track, device, param, value } => {
            let device = device_mut(project, &track, &device)?;
            let p = device.params.iter_mut().find(|p| p.id == param).ok_or(OpError::Invalid(format!("No parameter {param}")))?;
            let mut v = value.clamp(p.min, p.max);
            if let Some(step) = p.step.filter(|s| *s > 0.0) {
                v = (p.min + ((v - p.min) / step).round() * step).clamp(p.min, p.max);
            }
            p.value = v;
            Ok(label(format!("{} {}", device.name, p.label)))
        }
        Op::SetTempo { bpm } => {
            if !bpm.is_finite() {
                return Err(OpError::Invalid("Tempo must be a number".into()));
            }
            project.bpm = (bpm.clamp(MIN_BPM, MAX_BPM) * 100.0).round() / 100.0;
            Ok(Applied { label: "Tempo".into(), created: None, tempo_changed: true })
        }
        Op::SetTempoPoints { points } => {
            let mut points: Vec<TempoPoint> = points
                .into_iter()
                .filter(|p| p.time.is_finite() && p.bpm.is_finite())
                .map(|p| TempoPoint { time: p.time.max(0.0), bpm: (p.bpm.clamp(MIN_BPM, MAX_BPM) * 100.0).round() / 100.0 })
                .collect();
            points.sort_by(|a, b| a.time.total_cmp(&b.time));
            // The song's tempo is where the track starts, so turning it off changes nothing at the start.
            if let Some(first) = points.first() {
                project.bpm = first.bpm;
            }
            let what = if points.is_empty() { "Clear the tempo track" } else { "Tempo track" };
            project.tempo = points;
            Ok(Applied { label: what.into(), created: None, tempo_changed: true })
        }
        Op::SetWarp { clip, warp } => {
            let (ti, ci) = project.find_clip(&clip).ok_or_else(|| OpError::NoClip(clip.clone()))?;
            let c = &mut project.tracks[ti].clips[ci];
            let ClipContent::Audio { warp: current, .. } = &mut c.content else {
                return Err(OpError::Invalid("Only audio clips warp".into()));
            };
            *current = warp.map(|w| Warp { bpm: (w.bpm.clamp(MIN_BPM, MAX_BPM) * 100.0).round() / 100.0, ..w });
            let what = if current.is_some() { "Warp" } else { "Unwarp" };
            Ok(label(format!("{what} “{}”", c.name)))
        }
        Op::SetTimeSignature { beats_per_bar } => {
            project.beats_per_bar = beats_per_bar.clamp(1, 32);
            Ok(Applied { label: "Time signature".into(), created: None, tempo_changed: true })
        }
        Op::SetKey { key } => {
            project.key = key;
            Ok(label("Key".into()))
        }
        Op::DeleteClip { clip } => {
            let (ti, ci) = project.find_clip(&clip).ok_or(OpError::NoClip(clip))?;
            let removed = project.tracks[ti].clips.remove(ci);
            project.prune_assets();
            Ok(label(format!("Delete clip “{}”", removed.name)))
        }
        Op::MoveClip { clip, track, start } => {
            let (ti, ci) = timeline_clip(project, &clip)?;
            let target = project.tracks.iter().position(|t| t.id == track).ok_or(OpError::NoTrack(track))?;
            if project.tracks[target].kind != project.tracks[ti].kind {
                return Err(OpError::Invalid("Audio clips go on audio tracks and MIDI clips on MIDI tracks.".into()));
            }
            let mut moved = project.tracks[ti].clips.remove(ci);
            moved.start = start.max(0.0);
            let name = moved.name.clone();
            let clips = &mut project.tracks[target].clips;
            clips.push(moved);
            clips.sort_by(|a, b| a.start.total_cmp(&b.start));
            Ok(label(format!("Move clip “{name}”")))
        }
        Op::TrimClip { clip, start, length, ripple } => {
            let (ti, ci) = project.find_clip(&clip).ok_or(OpError::NoClip(clip))?;
            let tempo = project.tempo_map();
            let asset_duration = project.tracks[ti].clips[ci].asset().and_then(|a| project.asset(a)).map(|a| a.duration());
            let c = &mut project.tracks[ti].clips[ci];
            let old_end = c.end(&tempo);
            trim(c, start.max(0.0), length.max(MIN_CLIP_BEATS), &tempo, asset_duration);
            let (name, moved) = (c.name.clone(), c.end(&tempo) - old_end);
            let id = c.id.clone();
            if ripple && c.slot.is_none() && moved.abs() > 1e-9 {
                shift_after(&mut project.tracks[ti], old_end, moved, &[id]);
            }
            Ok(label(format!("Trim clip “{name}”")))
        }
        Op::MoveClips { clips, delta, tracks } => {
            let picked = timeline_clips(project, &clips)?;
            let count = project.tracks.len() as i64;
            for &(ti, _) in &picked {
                let to = ti as i64 + i64::from(tracks);
                if to < 0 || to >= count {
                    return Err(OpError::Invalid("There is no track there for all of them.".into()));
                }
                if project.tracks[to as usize].kind != project.tracks[ti].kind {
                    return Err(OpError::Invalid("Some of them would land on a track of another kind: audio clips go on audio tracks, MIDI on MIDI.".into()));
                }
            }
            // Nothing goes before the start of the song.
            let earliest = picked.iter().map(|&(ti, ci)| project.tracks[ti].clips[ci].start).fold(f64::INFINITY, f64::min);
            let delta = delta.max(-earliest);
            let mut taken = Vec::new();
            for &(ti, ci) in picked.iter().rev() {
                taken.push((ti, project.tracks[ti].clips.remove(ci)));
            }
            for (ti, mut c) in taken {
                c.start += delta;
                project.tracks[(ti as i64 + i64::from(tracks)) as usize].clips.push(c);
            }
            for t in &mut project.tracks {
                t.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
            }
            Ok(label(format!("Move {} clips", picked.len())))
        }
        Op::DeleteClips { clips, ripple } => {
            let tempo = project.tempo_map();
            let mut found: Vec<(usize, usize)> = clips.iter().map(|id| project.find_clip(id).ok_or_else(|| OpError::NoClip(id.clone()))).collect::<Result<_, _>>()?;
            found.sort_unstable();
            found.dedup();
            // Gaps left on the timeline, per track: (start, end) in beats.
            let mut gaps: Vec<(usize, f64, f64)> = Vec::new();
            for &(ti, ci) in found.iter().rev() {
                let c = project.tracks[ti].clips.remove(ci);
                if c.slot.is_none() {
                    gaps.push((ti, c.start, c.end(&tempo)));
                }
            }
            if ripple {
                // Latest gap first, so earlier ones are measured where they were.
                gaps.sort_by(|a, b| b.1.total_cmp(&a.1));
                for (ti, start, end) in gaps {
                    shift_after(&mut project.tracks[ti], end, start - end, &[]);
                }
            }
            project.prune_assets();
            let what = if found.len() == 1 { "Delete clip".to_string() } else { format!("Delete {} clips", found.len()) };
            Ok(label(if ripple { format!("{what} (ripple)") } else { what }))
        }
        Op::DuplicateClips { clips, ripple } => {
            let tempo = project.tempo_map();
            let picked = timeline_clips(project, &clips)?;
            let start = picked.iter().map(|&(ti, ci)| project.tracks[ti].clips[ci].start).fold(f64::INFINITY, f64::min);
            let end = picked.iter().map(|&(ti, ci)| project.tracks[ti].clips[ci].end(&tempo)).fold(0.0, f64::max);
            let span = end - start;
            let copies: Vec<(usize, Clip)> = picked
                .iter()
                .map(|&(ti, ci)| {
                    let mut c = project.tracks[ti].clips[ci].clone();
                    c.id = crate::new_id("c");
                    c.start += span;
                    if let ClipContent::Midi { link, .. } = &mut c.content {
                        *link = None;
                    }
                    (ti, c)
                })
                .collect();
            if ripple {
                let mut tracks: Vec<usize> = picked.iter().map(|p| p.0).collect();
                tracks.dedup();
                for ti in tracks {
                    shift_after(&mut project.tracks[ti], end, span, &clips);
                }
            }
            for (ti, c) in copies {
                project.tracks[ti].clips.push(c);
            }
            for t in &mut project.tracks {
                t.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
            }
            Ok(label(if picked.len() == 1 { "Duplicate clip".into() } else { format!("Duplicate {} clips", picked.len()) }))
        }
        Op::SetFadeCurve { clips, curve } => {
            for id in &clips {
                let (ti, ci) = project.find_clip(id).ok_or_else(|| OpError::NoClip(id.clone()))?;
                if let ClipContent::Audio { curve: shape, .. } = &mut project.tracks[ti].clips[ci].content {
                    *shape = curve;
                }
            }
            let name = match curve {
                None => "standard",
                Some(FadeCurve::Linear) => "linear",
                Some(FadeCurve::EqualPower) => "equal power",
                Some(FadeCurve::SCurve) => "S-curve",
            };
            Ok(label(format!("Fade shape: {name}")))
        }
        Op::ExtractToTrack { clip, start, end, cut } => {
            let (ti, ci) = timeline_clip(project, &clip)?;
            let tempo = project.tempo_map();
            let original = project.tracks[ti].clips[ci].clone();
            let ClipContent::Audio { duration, .. } = original.content else {
                return Err(OpError::Invalid("Only audio clips can be split off this way.".into()));
            };
            let (from, to) = (start.clamp(0.0, duration), end.clamp(0.0, duration));
            if to - from < 0.001 {
                return Err(OpError::Invalid("Select some audio first.".into()));
            }
            let (a, b) = (original.beat_after(&tempo, original.start, from), original.beat_after(&tempo, original.start, to));
            let mut piece = original.clone();
            let mut left = None;
            let mut right = None;
            if a > original.start + MIN_CLIP_BEATS {
                let (l, r) = split(&piece, a, &tempo);
                left = Some(l);
                piece = r;
            }
            if b < piece.end(&tempo) - MIN_CLIP_BEATS {
                let (m, r) = split(&piece, b, &tempo);
                piece = m;
                right = Some(r);
            }
            piece.id = crate::new_id("c");
            let source = &project.tracks[ti];
            let mut track = Track::new(crate::new_id("t"), format!("{} {}", source.name, if cut { "(split)" } else { "(copy)" }), TrackKind::Audio, source.color);
            track.group = source.group.clone();
            track.clips.push(piece);
            let id = track.id.clone();
            if cut {
                let rest: Vec<Clip> = left.into_iter().chain(right).collect();
                project.tracks[ti].clips.splice(ci..=ci, rest);
            }
            project.tracks.insert(ti + 1, track);
            let what = if cut { "Split New" } else { "Duplicate to a new track" };
            Ok(Applied { label: what.into(), created: Some(id), tempo_changed: false })
        }
        Op::SplitClip { clip, at } => {
            let (ti, ci) = timeline_clip(project, &clip)?;
            let tempo = project.tempo_map();
            let original = &project.tracks[ti].clips[ci];
            if at <= original.start + MIN_CLIP_BEATS || at >= original.end(&tempo) - MIN_CLIP_BEATS {
                return Err(OpError::Invalid("Split inside the clip, not at its edge.".into()));
            }
            let (left, right) = split(original, at, &tempo);
            let id = right.id.clone();
            let name = left.name.clone();
            project.tracks[ti].clips[ci] = left;
            project.tracks[ti].clips.insert(ci + 1, right);
            Ok(Applied { label: format!("Split clip “{name}”"), created: Some(id), tempo_changed: false })
        }
        Op::DuplicateClip { clip, linked } => {
            let (ti, ci) = timeline_clip(project, &clip)?;
            if linked {
                let ClipContent::Midi { link, .. } = &mut project.tracks[ti].clips[ci].content else {
                    return Err(OpError::Invalid("Only MIDI clips can be linked.".into()));
                };
                link.get_or_insert_with(|| crate::new_id("lnk"));
            }
            let mut copy = project.tracks[ti].clips[ci].clone();
            copy.id = crate::new_id("c");
            copy.start = copy.end(&project.tempo_map());
            // A plain duplicate is a clip of its own, even of a linked one.
            if let (false, ClipContent::Midi { link, .. }) = (linked, &mut copy.content) {
                *link = None;
            }
            let (id, name) = (copy.id.clone(), copy.name.clone());
            project.tracks[ti].clips.insert(ci + 1, copy);
            let what = if linked { "Linked copy of" } else { "Duplicate clip" };
            Ok(Applied { label: format!("{what} “{name}”"), created: Some(id), tempo_changed: false })
        }
        Op::UnlinkClip { clip } => {
            let (ti, ci) = project.find_clip(&clip).ok_or(OpError::NoClip(clip))?;
            let c = &mut project.tracks[ti].clips[ci];
            if let ClipContent::Midi { link, .. } = &mut c.content {
                *link = None;
            }
            Ok(label(format!("Unlink “{}”", c.name)))
        }
        Op::SetClipFades { clip, fade_in: new_in, fade_out: new_out } => {
            let (ti, ci) = project.find_clip(&clip).ok_or(OpError::NoClip(clip))?;
            let c = &mut project.tracks[ti].clips[ci];
            let ClipContent::Audio { duration, fade_in, fade_out, .. } = &mut c.content else {
                return Err(OpError::Invalid("Only audio clips have fades.".into()));
            };
            (*fade_in, *fade_out) = fit_fades(new_in, new_out, *duration);
            Ok(label(format!("Fades on “{}”", c.name)))
        }
        Op::AddMarker { name, start, end } => {
            let start = start.max(0.0);
            let end = end.map(|e| e.max(start + MIN_CLIP_BEATS));
            let kind = if end.is_some() { "Region" } else { "Marker" };
            let count = project.markers.iter().filter(|m| m.end.is_some() == end.is_some()).count() + 1;
            let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("{kind} {count}"));
            let id = crate::new_id("m");
            project.markers.push(Marker { id: id.clone(), name, start, end });
            project.markers.sort_by(|a, b| a.start.total_cmp(&b.start));
            Ok(Applied { label: format!("Add {}", kind.to_lowercase()), created: Some(id), tempo_changed: false })
        }
        Op::SetMarker { id, name, start, end } => {
            let marker = project.markers.iter_mut().find(|m| m.id == id).ok_or(OpError::Invalid("That marker no longer exists.".into()))?;
            let what = if let Some(name) = name {
                let name = name.trim();
                if name.is_empty() {
                    return Err(OpError::Invalid("A marker needs a name".into()));
                }
                marker.name = name.to_string();
                "Rename"
            } else {
                // Moving the start of a region moves the whole region unless a new end is given.
                let new_start = start.unwrap_or(marker.start).max(0.0);
                let shift = new_start - marker.start;
                marker.end = match (marker.end, end) {
                    (Some(_), Some(e)) => Some(e.max(new_start + MIN_CLIP_BEATS)),
                    (Some(e), None) => Some(e + shift),
                    (None, _) => None,
                };
                marker.start = new_start;
                "Move"
            };
            let name = marker.name.clone();
            project.markers.sort_by(|a, b| a.start.total_cmp(&b.start));
            Ok(label(format!("{what} “{name}”")))
        }
        Op::RemoveMarker { id } => {
            let index = project.markers.iter().position(|m| m.id == id).ok_or(OpError::Invalid("That marker no longer exists.".into()))?;
            let removed = project.markers.remove(index);
            Ok(label(format!("Delete “{}”", removed.name)))
        }
        Op::AddMidiClip { track, start, length, notes, name, slot } => {
            let scenes = project.scenes.len();
            let track = track_mut(project, &track)?;
            if track.kind != TrackKind::Midi {
                return Err(OpError::Invalid("MIDI clips go on MIDI tracks.".into()));
            }
            let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("{} {}", track.name, track.clips.len() + 1));
            let mut clip = Clip::midi(name, start.max(0.0), length.max(MIN_CLIP_BEATS), notes);
            if let Some(scene) = slot {
                if scene >= scenes {
                    return Err(OpError::Invalid("There is no such scene".into()));
                }
                track.clips.retain(|c| c.slot != Some(scene));
                clip.start = 0.0;
                clip.slot = Some(scene);
            }
            let id = clip.id.clone();
            track.clips.push(clip);
            track.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
            Ok(Applied { label: "Add MIDI clip".into(), created: Some(id), tempo_changed: false })
        }
        Op::SetNotes { clip, notes, label: what } => {
            let (ti, ci) = project.find_clip(&clip).ok_or_else(|| OpError::NoClip(clip.clone()))?;
            if !matches!(project.tracks[ti].clips[ci].content, ClipContent::Midi { .. }) {
                return Err(OpError::Invalid("Only MIDI clips have notes.".into()));
            }
            project.set_clip_notes(&clip, notes);
            Ok(label(what.unwrap_or_else(|| "Edit notes".into())))
        }
        Op::RenameClip { clip, name } => {
            let (ti, ci) = project.find_clip(&clip).ok_or(OpError::NoClip(clip))?;
            let name = name.trim();
            if name.is_empty() {
                return Err(OpError::Invalid("A clip needs a name".into()));
            }
            project.tracks[ti].clips[ci].name = name.to_string();
            Ok(label("Rename clip".into()))
        }
        Op::SetDeviceSample { track, device, key, asset } => {
            if let Some(id) = &asset
                && project.asset(id).is_none()
            {
                return Err(OpError::Invalid("That audio is not in the project.".into()));
            }
            let device = device_mut(project, &track, &device)?;
            device.samples.retain(|s| s.key != key);
            let loading = asset.is_some();
            if let Some(asset) = asset {
                device.samples.push(SampleRef { key, asset });
                device.samples.sort_by_key(|s| s.key);
            }
            let what = if loading { format!("Load a sample into {}", device.name) } else { format!("Clear a sample in {}", device.name) };
            project.prune_assets();
            Ok(label(what))
        }
        Op::AddChain { track, device, kind } => {
            if let Some(rack) = fx_rack_mut(project, &track, &device)? {
                let chain = FxChain::new(format!("Chain {}", rack.fx_chains.len() + 1));
                let (id, name) = (chain.id.clone(), chain.name.clone());
                rack.fx_chains.push(chain);
                return Ok(Applied { label: format!("Add {name}"), created: Some(id), tempo_changed: false });
            }
            let instrument = crate::catalog::device(&kind, &[]).ok_or_else(|| OpError::Invalid(format!("Unknown device {kind}")))?;
            if instrument.role != DeviceRole::Instrument || kind == "rack" {
                return Err(OpError::Invalid(format!("{} can't go in a rack chain", instrument.name)));
            }
            let rack = rack_mut(track_mut(project, &track)?, &device)?;
            let chain = Chain::new(instrument);
            let (id, name) = (chain.id.clone(), chain.name.clone());
            rack.chains.push(chain);
            Ok(Applied { label: format!("Add a {name} chain"), created: Some(id), tempo_changed: false })
        }
        Op::RemoveChain { track, device, chain } => {
            if let Some(rack) = fx_rack_mut(project, &track, &device)? {
                let removed = rack.fx_chains.remove(fx_chain_index(rack, &chain)?);
                project.prune_macros();
                project.prune_assets();
                project.prune_mappings();
                return Ok(label(format!("Remove the {} chain", removed.name)));
            }
            let rack = rack_mut(track_mut(project, &track)?, &device)?;
            let removed = rack.chains.remove(chain_index(rack, &chain)?);
            rack.macros.retain(|m| m.chain != chain);
            project.prune_assets();
            project.prune_mappings();
            Ok(label(format!("Remove the {} chain", removed.name)))
        }
        Op::SetChain { track, device, chain, patch } => {
            if let Some(rack) = fx_rack_mut(project, &track, &device)? {
                let index = fx_chain_index(rack, &chain)?;
                set_fx_chain(&mut rack.fx_chains[index], patch)?;
                return Ok(label(format!("Change the {} chain", rack.fx_chains[index].name)));
            }
            let rack = rack_mut(track_mut(project, &track)?, &device)?;
            let index = chain_index(rack, &chain)?;
            let c = &mut rack.chains[index];
            if let Some(name) = patch.name {
                let name = name.trim();
                if name.is_empty() {
                    return Err(OpError::Invalid("A chain needs a name".into()));
                }
                c.name = name.to_string();
            }
            if let Some(on) = patch.on {
                c.on = on;
            }
            // The low end of a zone never passes the high end: moving one pushes the other.
            if let Some(v) = patch.key_low {
                c.key_low = v.min(127);
                c.key_high = c.key_high.max(c.key_low);
            }
            if let Some(v) = patch.key_high {
                c.key_high = v.min(127);
                c.key_low = c.key_low.min(c.key_high);
            }
            if let Some(v) = patch.vel_low {
                c.vel_low = v.clamp(1, 127);
                c.vel_high = c.vel_high.max(c.vel_low);
            }
            if let Some(v) = patch.vel_high {
                c.vel_high = v.clamp(1, 127);
                c.vel_low = c.vel_low.min(c.vel_high);
            }
            if let Some(v) = patch.volume {
                c.volume = v.clamp(-60.0, 6.0);
            }
            if let Some(v) = patch.pan {
                c.pan = v.clamp(-1.0, 1.0);
            }
            Ok(label(format!("Change the {} chain", c.name)))
        }
        Op::MapMacro { track, device, chain, inner, param, index, min, max } => {
            let effects = fx_rack_mut(project, &track, &device)?.is_some();
            let rack = if effects { fx_rack_mut(project, &track, &device)?.expect("checked just above") } else { rack_mut(track_mut(project, &track)?, &device)? };
            // The device whose parameter it is: the chain's instrument, or one of an effect chain's effects.
            let (target, chain_name) = if effects {
                let c = &rack.fx_chains[fx_chain_index(rack, &chain)?];
                let id = inner.clone().ok_or_else(|| OpError::Invalid("Which effect in the chain?".into()))?;
                (c.devices.iter().find(|d| d.id == id).ok_or(OpError::NoDevice(id))?, c.name.clone())
            } else {
                let c = &rack.chains[chain_index(rack, &chain)?];
                (&c.device, c.name.clone())
            };
            let inner = if effects { inner } else { None };
            let p = target.params.iter().find(|p| p.id == param).ok_or(OpError::Invalid(format!("No parameter {param}")))?.clone();
            let what = if effects { format!("{} {}", target.name, p.label) } else { format!("{chain_name} {}", p.label) };
            // One macro per parameter.
            rack.macros.retain(|m| !(m.chain == chain && m.device == inner && m.param == param));
            let Some(index) = index else { return Ok(label(format!("Unmap {what}"))) };
            if index >= crate::MACROS {
                return Err(OpError::Invalid(format!("There are {} macros", crate::MACROS)));
            }
            let map = crate::MacroMap {
                index,
                chain,
                device: inner,
                param,
                min: min.unwrap_or(p.min).clamp(p.min, p.max),
                max: max.unwrap_or(p.max).clamp(p.min, p.max),
            };
            // A macro's first mapping takes the parameter's place, so nothing jumps.
            if !rack.macros.iter().any(|m| m.index == index) {
                let macro_id = format!("macro{}", index + 1);
                let amount = map.amount(&p, p.value) * 100.0;
                if let Some(m) = rack.params.iter_mut().find(|q| q.id == macro_id) {
                    m.value = amount;
                }
            }
            rack.macros.push(map);
            Ok(label(format!("Map {what} to macro {}", index + 1)))
        }
        Op::SetAutomation { track, target, points } => {
            let track = track_mut(project, &track)?;
            let (lo, hi) = target_range(track, &target)?;
            let mut points: Vec<AutoPoint> = points
                .into_iter()
                .filter(|p| p.time.is_finite() && p.value.is_finite())
                .map(|p| AutoPoint { time: p.time.max(0.0), value: p.value.clamp(lo, hi) })
                .collect();
            points.sort_by(|a, b| a.time.total_cmp(&b.time));
            track.automation.retain(|lane| lane.target != target);
            let what = if points.is_empty() { "Clear automation" } else { "Draw automation" };
            if !points.is_empty() {
                track.automation.push(AutomationLane { target, points });
            }
            Ok(label(format!("{what} on {}", track.name)))
        }
        Op::SetModTarget { track, device, target } => {
            let track = track_mut(project, &track)?;
            if let Some(t) = &target {
                if t.device == device {
                    return Err(OpError::Invalid("A modulator can't move itself".into()));
                }
                let found = track.devices.iter().find(|d| d.id == t.device).ok_or_else(|| OpError::NoDevice(t.device.clone()))?;
                if MODULATOR_TYPES.contains(&found.kind.as_str()) || !found.params.iter().any(|p| p.id == t.param) {
                    return Err(OpError::Invalid("A modulator moves another device's parameter".into()));
                }
            }
            let d = track.devices.iter_mut().find(|d| d.id == device).ok_or_else(|| OpError::NoDevice(device.clone()))?;
            if !MODULATOR_TYPES.contains(&d.kind.as_str()) {
                return Err(OpError::Invalid(format!("{} is not a modulator", d.name)));
            }
            d.modulates = target;
            Ok(label(format!("Point {}", d.name)))
        }
        Op::GroupTracks { tracks } => {
            let first = tracks.iter().filter_map(|id| project.tracks.iter().position(|t| t.id == *id)).min().ok_or(OpError::Invalid("Pick tracks to group".into()))?;
            if project.tracks.iter().any(|t| tracks.contains(&t.id) && t.kind == TrackKind::Group) {
                return Err(OpError::Invalid("Groups can't go inside groups".into()));
            }
            let count = project.tracks.iter().filter(|t| t.kind == TrackKind::Group).count() + 1;
            let id = crate::new_id("t");
            let group = Track::new(id.clone(), format!("Group {count}"), TrackKind::Group, project.tracks[first].color);
            let (mut members, rest): (Vec<Track>, Vec<Track>) = std::mem::take(&mut project.tracks).into_iter().partition(|t| tracks.contains(&t.id));
            for m in &mut members {
                m.group = Some(id.clone());
            }
            project.tracks = rest;
            let at = first.min(project.tracks.len());
            project.tracks.splice(at..at, std::iter::once(group).chain(members));
            Ok(Applied { label: "Group tracks".into(), created: Some(id), tempo_changed: false })
        }
        Op::Ungroup { group } => {
            let index = project.tracks.iter().position(|t| t.id == group && t.kind == TrackKind::Group).ok_or_else(|| OpError::NoTrack(group.clone()))?;
            let removed = project.tracks.remove(index);
            for t in project.tracks.iter_mut().filter(|t| t.group.as_deref() == Some(group.as_str())) {
                t.group = None;
            }
            project.prune_mappings();
            project.prune_sidechains();
            Ok(label(format!("Ungroup {}", removed.name)))
        }
        Op::SetTrackGroup { track, group } => {
            let index = project.tracks.iter().position(|t| t.id == track).ok_or_else(|| OpError::NoTrack(track.clone()))?;
            if project.tracks[index].kind == TrackKind::Group {
                return Err(OpError::Invalid("Groups can't go inside groups".into()));
            }
            if let Some(g) = &group
                && !project.tracks.iter().any(|t| t.id == *g && t.kind == TrackKind::Group)
            {
                return Err(OpError::NoTrack(g.clone()));
            }
            let mut moved = project.tracks.remove(index);
            // It lands right after the last track of the group it joins or leaves.
            let block = group.as_deref().or(moved.group.as_deref()).map(str::to_string);
            let at = match block {
                Some(g) => {
                    let head = project.tracks.iter().position(|t| t.id == g).unwrap_or(index.min(project.tracks.len()));
                    project.tracks.iter().rposition(|t| t.group.as_deref() == Some(g.as_str())).map_or(head + 1, |last| last + 1)
                }
                None => index,
            };
            moved.group = group;
            let name = moved.name.clone();
            project.tracks.insert(at.min(project.tracks.len()), moved);
            Ok(label(format!("Move {name}")))
        }
        Op::PutInSlot { clip, scene } => {
            let (ti, ci) = project.find_clip(&clip).ok_or(OpError::NoClip(clip))?;
            while project.scenes.len() <= scene {
                project.scenes.push(format!("Scene {}", project.scenes.len() + 1));
            }
            let track = &mut project.tracks[ti];
            let copy = track.clips[ci].in_slot(scene);
            let id = copy.id.clone();
            track.clips.retain(|c| c.slot != Some(scene));
            track.clips.push(copy);
            Ok(Applied { label: "Put clip in slot".into(), created: Some(id), tempo_changed: false })
        }
        Op::ClearSlot { track, scene } => {
            let track = track_mut(project, &track)?;
            track.clips.retain(|c| c.slot != Some(scene));
            project.prune_assets();
            Ok(label("Clear slot".into()))
        }
        Op::ReverseClips { clips } => {
            let found = timeline_or_slot_audio(project, &clips)?;
            for &(ti, ci) in &found {
                if let ClipContent::Audio { reverse, fade_in, fade_out, .. } = &mut project.tracks[ti].clips[ci].content {
                    *reverse = !*reverse;
                    // What faded out at the end now fades in at the start.
                    std::mem::swap(fade_in, fade_out);
                }
            }
            Ok(label(if found.len() == 1 { format!("Reverse “{}”", project.tracks[found[0].0].clips[found[0].1].name) } else { format!("Reverse {} clips", found.len()) }))
        }
        Op::SetClipTranspose { clip, semitones } => {
            let (ti, ci) = project.find_clip(&clip).ok_or(OpError::NoClip(clip))?;
            let c = &mut project.tracks[ti].clips[ci];
            let ClipContent::Audio { transpose, .. } = &mut c.content else {
                return Err(OpError::Invalid("Only audio clips are transposed here; move a MIDI clip's notes instead.".into()));
            };
            *transpose = semitones.clamp(-24.0, 24.0).round();
            Ok(label(format!("Transpose “{}”", c.name)))
        }
        Op::SetClipEnvelope { clip, points } => {
            let tempo = project.tempo_map();
            let (ti, ci) = project.find_clip(&clip).ok_or(OpError::NoClip(clip))?;
            let c = &mut project.tracks[ti].clips[ci];
            let length = c.length_beats(&tempo);
            let ClipContent::Audio { envelope, .. } = &mut c.content else {
                return Err(OpError::Invalid("Only audio clips have a gain envelope.".into()));
            };
            let mut points: Vec<EnvPoint> = points.into_iter().filter(|p| p.time.is_finite() && p.gain.is_finite()).map(|p| EnvPoint { time: p.time.clamp(0.0, length), gain: p.gain.clamp(-60.0, 12.0) }).collect();
            points.sort_by(|a, b| a.time.total_cmp(&b.time));
            points.truncate(256);
            *envelope = points;
            Ok(label(format!("Gain envelope of “{}”", c.name)))
        }
        Op::SetMeter { bar, beats_per_bar } => {
            if bar == 0 {
                return Err(OpError::Invalid("The first bar has the song's own time signature: set it in the transport bar.".into()));
            }
            project.meters.retain(|m| m.bar != bar);
            if let Some(beats) = beats_per_bar {
                project.meters.push(MeterChange { bar, beats_per_bar: beats.clamp(1, 32) });
                project.meters.sort_by_key(|m| m.bar);
            }
            // A change to the length already in force is none.
            let mut current = project.beats_per_bar;
            project.meters.retain(|m| {
                let changes = m.beats_per_bar != current;
                if changes {
                    current = m.beats_per_bar;
                }
                changes
            });
            Ok(label(format!("Time signature at bar {}", bar + 1)))
        }
        Op::MoveSection { marker, to } => {
            let (from, end, name) = region(project, &marker)?;
            let to = to.max(0.0);
            if to > from + 1e-9 && to < end - 1e-9 {
                return Err(OpError::Invalid("Drop the section before or after itself.".into()));
            }
            if (to - from).abs() < 1e-9 || (to - end).abs() < 1e-9 {
                return Err(OpError::Invalid("The section is already there.".into()));
            }
            let len = end - from;
            let section = copy_section(project, from, end, false);
            remove_time(project, from, end);
            let at = if to >= end { to - len } else { to };
            open_time(project, at, len);
            paste_section(project, at, section);
            project.prune_assets();
            Ok(label(format!("Move section “{name}”")))
        }
        Op::DuplicateSection { marker } => {
            let (from, end, name) = region(project, &marker)?;
            let section = copy_section(project, from, end, true);
            open_time(project, end, end - from);
            paste_section(project, end, section);
            Ok(label(format!("Duplicate section “{name}”")))
        }
        Op::DeleteSection { marker } => {
            let (from, end, name) = region(project, &marker)?;
            remove_time(project, from, end);
            project.prune_assets();
            Ok(label(format!("Delete section “{name}”")))
        }
        Op::CompTake { track, lane, start, end } => {
            let tempo = project.tempo_map();
            let track = track_mut(project, &track)?;
            let li = track.takes.iter().position(|l| l.id == lane).ok_or_else(|| OpError::Invalid("That take no longer exists.".into()))?;
            let (from, to) = (start.min(end).max(0.0), start.max(end));
            let pieces: Vec<Clip> = track.takes[li]
                .clips
                .iter()
                .filter_map(|c| {
                    let (s, e) = (c.start.max(from), c.end(&tempo).min(to));
                    (e - s >= MIN_CLIP_BEATS).then(|| {
                        let mut piece = c.clone();
                        piece.id = crate::new_id("c");
                        trim(&mut piece, s, e - s, &tempo, None);
                        piece
                    })
                })
                .collect();
            if pieces.is_empty() {
                return Err(OpError::Invalid("Nothing was recorded there on that take.".into()));
            }
            let name = track.takes[li].name.clone();
            for mut piece in pieces {
                let (s, e) = (piece.start, piece.end(&tempo));
                clear_range(track, s, e, &tempo);
                // A few milliseconds of fade on each side of a join, so it never clicks.
                soften(&mut piece, true, true);
                for c in track.clips.iter_mut().filter(|c| c.slot.is_none()) {
                    let (cs, ce) = (c.start, c.end(&tempo));
                    if (ce - s).abs() < 1e-6 {
                        soften(c, false, true);
                    }
                    if (cs - e).abs() < 1e-6 {
                        soften(c, true, false);
                    }
                }
                track.clips.push(piece);
            }
            track.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
            Ok(label(format!("Comp from {name}")))
        }
        Op::DeleteTake { track, lane } => {
            let track = track_mut(project, &track)?;
            let li = track.takes.iter().position(|l| l.id == lane).ok_or_else(|| OpError::Invalid("That take no longer exists.".into()))?;
            let removed = track.takes.remove(li);
            project.prune_assets();
            Ok(label(format!("Delete {}", removed.name)))
        }
        Op::ClearTakes { track } => {
            let track = track_mut(project, &track)?;
            if track.takes.is_empty() {
                return Err(OpError::Invalid(format!("{} has no takes", track.name)));
            }
            track.takes.clear();
            let name = track.name.clone();
            project.prune_assets();
            Ok(label(format!("Delete the takes of {name}")))
        }
        Op::SceneToArrangement { scene, at } => {
            let tempo = project.tempo_map();
            let at = at.max(0.0);
            let with: Vec<(String, f64)> = project.tracks.iter().filter_map(|t| t.slot(scene).map(|c| (t.id.clone(), c.length_beats(&tempo)))).collect();
            let longest = with.iter().map(|(_, len)| *len).fold(0.0, f64::max);
            if longest < MIN_CLIP_BEATS {
                return Err(OpError::Invalid("That scene has no clips.".into()));
            }
            for (track, _) in &with {
                record_slot(project, track, scene, at, at + longest, 0.0);
            }
            let name = project.scenes.get(scene).cloned().unwrap_or_else(|| format!("Scene {}", scene + 1));
            Ok(label(format!("{name} to the arrangement")))
        }
        Op::SetFollow { clip, follow } => {
            let (ti, ci) = project.find_clip(&clip).ok_or(OpError::NoClip(clip))?;
            let c = &mut project.tracks[ti].clips[ci];
            if c.slot.is_none() {
                return Err(OpError::Invalid("Follow actions are for clips in the session view".into()));
            }
            c.follow = follow.map(|f| FollowAction { bars: f.bars.clamp(0.25, 128.0), ..f });
            Ok(label(format!("Follow action of “{}”", c.name)))
        }
        Op::SetLaunchQuantize { beats } => {
            project.launch_quantize = beats.clamp(0.0, 32.0);
            Ok(label("Launch quantisation".into()))
        }
        Op::AddScene => {
            let name = format!("Scene {}", project.scenes.len() + 1);
            project.scenes.push(name.clone());
            Ok(label(format!("Add {name}")))
        }
        Op::AddMapping { channel, cc, target } => {
            if cc > 119 {
                return Err(OpError::Invalid(format!("CC {cc} is not a control")));
            }
            if !project.has_target(&target) {
                return Err(OpError::Invalid("That control is no longer in the song".into()));
            }
            // One target per control, one control per target.
            project.mappings.retain(|m| !(m.cc == cc && m.channel == channel) && m.target != target);
            let id = crate::new_id("map");
            project.mappings.push(ControllerMapping { id: id.clone(), channel, cc, target });
            Ok(Applied { label: format!("Map CC {cc}"), created: Some(id), tempo_changed: false })
        }
        Op::RemoveMapping { id } => {
            let before = project.mappings.len();
            project.mappings.retain(|m| m.id != id);
            if project.mappings.len() == before {
                return Err(OpError::Invalid("That mapping no longer exists".into()));
            }
            Ok(label("Remove a MIDI mapping".into()))
        }
    }
}

/// The change a MIDI controller makes when it sends `value` (0–127) to
/// `target`, as an op, so it is undoable like turning the knob on screen.
/// `None` when the target is gone.
pub fn controller_op(project: &Project, target: &MappingTarget, value: u8) -> Option<Op> {
    let t = f64::from(value.min(127)) / 127.0;
    Some(match target {
        // Any chain: a track's, a return's or the master's.
        MappingTarget::Param { track, device, param } => {
            let d = project.chain(track)?.iter().flat_map(Device::with_inner).find(|d| d.id == *device)?;
            let p = d.params.iter().find(|p| p.id == *param)?;
            // Log parameters sweep evenly by octave; SetParam snaps stepped ones.
            let value = if p.log == Some(true) && p.min > 0.0 { p.min * (p.max / p.min).powf(t) } else { p.min + (p.max - p.min) * t };
            Op::SetParam { track: track.clone(), device: device.clone(), param: param.clone(), value }
        }
        // A fader law: unity gain about two thirds of the way up, +6 dB at the top.
        MappingTarget::Volume { track } => {
            let gain = 2.0 * t * t;
            let volume = if gain > 0.0 { (20.0 * gain.log10()).clamp(-60.0, 6.0) } else { -60.0 };
            Op::SetTrack { id: project.track(track)?.id.clone(), patch: TrackPatch { volume: Some((volume * 10.0).round() / 10.0), ..Default::default() } }
        }
        // 64 is the centre.
        MappingTarget::Pan { track } => {
            let pan = (f64::from(value.min(127)) - 64.0) / if value >= 64 { 63.0 } else { 64.0 };
            Op::SetTrack { id: project.track(track)?.id.clone(), patch: TrackPatch { pan: Some(pan), ..Default::default() } }
        }
    })
}

/// Clamps fades to the clip, scaling both down if they would overlap.
fn fit_fades(fade_in: f64, fade_out: f64, duration: f64) -> (f64, f64) {
    let (a, b) = (fade_in.max(0.0), fade_out.max(0.0));
    let total = a + b;
    if total > duration && total > 0.0 { (a * duration / total, b * duration / total) } else { (a, b) }
}

/// Moves a clip's edges. For audio, the left edge slides the offset into the
/// asset (never before its first sample) and the right edge never runs past
/// its last sample. For MIDI, notes move with the content and are cut to fit.
fn trim(clip: &mut Clip, start: f64, length: f64, tempo: &TempoMap, asset_duration: Option<f64>) {
    let old_start = clip.start;
    let end = start + length;
    // A reversed clip plays its stretch from the end: it is trimmed as the
    // forward clip of the reversed file, then turned back.
    let reversed = match &mut clip.content {
        ClipContent::Audio { reverse: true, offset, duration, .. } => {
            let stretch_end = *offset + *duration;
            let file = asset_duration.unwrap_or(stretch_end);
            *offset = (file - stretch_end).max(0.0);
            Some(file)
        }
        _ => None,
    };
    // Audio: where the new edges fall in the file (tempo track and warping included).
    let audio = match &clip.content {
        ClipContent::Audio { offset, .. } => {
            let shortest = clip.audio_seconds(tempo, start, start + MIN_CLIP_BEATS);
            let new_offset = (offset + clip.audio_seconds(tempo, old_start, start)).max(0.0);
            let new_start = clip.beat_after(tempo, old_start, new_offset - offset);
            let available = asset_duration.map_or(f64::INFINITY, |d| d - new_offset);
            Some((new_start, new_offset, clip.audio_seconds(tempo, new_start, end).clamp(shortest, available.max(shortest))))
        }
        ClipContent::Midi { .. } => None,
    };
    match &mut clip.content {
        ClipContent::Audio { offset, duration, fade_in, fade_out, .. } => {
            let (new_start, new_offset, new_duration) = audio.unwrap_or((old_start, *offset, *duration));
            *offset = new_offset;
            *duration = new_duration;
            (*fade_in, *fade_out) = fit_fades(*fade_in, *fade_out, *duration);
            if let Some(file) = reversed {
                *offset = (file - *offset - *duration).max(0.0);
            }
            clip.start = new_start;
            let now = clip.end(tempo) - clip.start;
            recut_envelope(clip, old_start - new_start, now);
        }
        ClipContent::Midi { length: clip_length, notes, link } => {
            let shift = start - old_start;
            // Moving the start changes this clip's notes alone: it leaves its link.
            if shift.abs() > 1e-9 {
                *link = None;
            }
            *clip_length = length;
            notes.retain_mut(|n| {
                n.start -= shift;
                if n.start < 0.0 {
                    n.length += n.start;
                    n.start = 0.0;
                }
                n.length = n.length.min(length - n.start);
                n.length > 0.0 && n.start < length
            });
            clip.start = start;
        }
    }
}

/// Two clips that together play exactly what `clip` played.
fn split(clip: &Clip, at: f64, tempo: &TempoMap) -> (Clip, Clip) {
    let mut left = clip.clone();
    let mut right = clip.clone();
    right.id = crate::new_id("c");
    right.start = at;
    let cut = at - clip.start;
    let cut_sec = clip.audio_seconds(tempo, clip.start, at);
    match (&mut left.content, &mut right.content) {
        (
            ClipContent::Audio { duration: ld, fade_out: lo, .. },
            ClipContent::Audio { offset: ro, duration: rd, fade_in: ri, .. },
        ) => {
            *ld = cut_sec;
            *lo = 0.0;
            *ro += cut_sec;
            *rd -= cut_sec;
            *ri = 0.0;
        }
        (ClipContent::Midi { length: ll, notes: ln, link: l_link }, ClipContent::Midi { length: rl, notes: rn, link: r_link }) => {
            // The halves hold different notes now.
            *l_link = None;
            *r_link = None;
            *rl = *ll - cut;
            *ll = cut;
            ln.retain_mut(|n| {
                n.length = n.length.min(cut - n.start);
                n.start < cut
            });
            rn.retain_mut(|n| {
                n.start -= cut;
                n.start >= 0.0
            });
        }
        _ => unreachable!("both halves are clones of one clip"),
    }
    // Backwards, the left half plays the end of the stretch and the right half its start.
    if let (ClipContent::Audio { reverse: true, offset: lo, duration: ld, .. }, ClipContent::Audio { offset: ro, duration: rd, .. }) = (&mut left.content, &mut right.content) {
        let stretch_start = *lo;
        *lo = stretch_start + *rd;
        *ro = stretch_start;
        let _ = ld;
    }
    recut_envelope(&mut left, 0.0, cut);
    let rest = clip.end(tempo) - at;
    recut_envelope(&mut right, -cut, rest);
    (left, right)
}

/// Clears `from`…`to` (beats) on a track's timeline: clips inside go, clips
/// across an edge are cut back to it.
fn clear_range(track: &mut Track, from: f64, to: f64, tempo: &TempoMap) {
    let mut kept = Vec::with_capacity(track.clips.len());
    for clip in std::mem::take(&mut track.clips) {
        let (s, e) = (clip.start, clip.end(tempo));
        if clip.slot.is_some() || e <= from + 1e-9 || s >= to - 1e-9 {
            kept.push(clip);
            continue;
        }
        if s < from {
            let mut head = clip.clone();
            trim(&mut head, s, from - s, tempo, None);
            kept.push(head);
        }
        if e > to {
            let mut tail = clip;
            if s < from {
                tail.id = crate::new_id("c");
            }
            trim(&mut tail, to, e - to, tempo, None);
            kept.push(tail);
        }
    }
    track.clips = kept;
}

/// Moves a clip's gain envelope by `by` beats (its start moved the other way)
/// and cuts it to the clip's new length: the level at both edges stays what it was.
fn recut_envelope(clip: &mut Clip, by: f64, length: f64) {
    let ClipContent::Audio { envelope, .. } = &mut clip.content else { return };
    if envelope.is_empty() {
        return;
    }
    for p in envelope.iter_mut() {
        p.time += by;
    }
    let before = envelope.iter().any(|p| p.time < 0.0);
    let after = envelope.iter().any(|p| p.time > length);
    if !before && !after {
        return;
    }
    let level = |at: f64| {
        let next = envelope.partition_point(|p| p.time <= at);
        match (next.checked_sub(1).map(|i| envelope[i]), envelope.get(next)) {
            (Some(a), Some(b)) => a.gain + (b.gain - a.gain) * ((at - a.time) / (b.time - a.time).max(1e-9)),
            (Some(a), None) => a.gain,
            (None, Some(b)) => b.gain,
            (None, None) => 0.0,
        }
    };
    let (first, last) = (level(0.0), level(length));
    envelope.retain(|p| (0.0..=length).contains(&p.time));
    if before && envelope.first().is_none_or(|p| p.time > 1e-9) {
        envelope.insert(0, EnvPoint { time: 0.0, gain: first });
    }
    if after && envelope.last().is_none_or(|p| p.time < length - 1e-9) {
        envelope.push(EnvPoint { time: length, gain: last });
    }
}

/// Audio clips by id, on the timeline or in session slots, as (track, clip) indexes.
fn timeline_or_slot_audio(project: &Project, ids: &[String]) -> Result<Vec<(usize, usize)>, OpError> {
    if ids.is_empty() {
        return Err(OpError::Invalid("Select an audio clip first.".into()));
    }
    ids.iter()
        .map(|id| {
            let (ti, ci) = project.find_clip(id).ok_or_else(|| OpError::NoClip(id.clone()))?;
            match project.tracks[ti].clips[ci].content {
                ClipContent::Audio { .. } => Ok((ti, ci)),
                ClipContent::Midi { .. } => Err(OpError::Invalid("Only audio clips play backwards.".into())),
            }
        })
        .collect()
}

/// A region marker's start, end and name.
fn region(project: &Project, marker: &str) -> Result<(f64, f64, String), OpError> {
    let m = project.markers.iter().find(|m| m.id == marker).ok_or_else(|| OpError::Invalid("That region no longer exists.".into()))?;
    let end = m.end.filter(|e| *e - m.start >= MIN_CLIP_BEATS).ok_or_else(|| OpError::Invalid("Sections are regions: drag a marker's edge out, or use Region.".into()))?;
    Ok((m.start, end, m.name.clone()))
}

/// What a stretch of the song holds, lifted out: each track's clips (cut to
/// the stretch, starting from 0), each automation lane's points, the markers.
struct Section {
    clips: Vec<Vec<Clip>>,
    points: Vec<Vec<Vec<AutoPoint>>>,
    markers: Vec<Marker>,
}

/// Copies `from`…`to` (beats) of every track. `fresh` gives the markers new
/// ids (a duplicate); otherwise they keep theirs (a move).
fn copy_section(project: &Project, from: f64, to: f64, fresh: bool) -> Section {
    let tempo = project.tempo_map();
    let clips = project
        .tracks
        .iter()
        .map(|track| {
            track
                .clips
                .iter()
                .filter(|c| c.slot.is_none())
                .filter_map(|c| {
                    let (s, e) = (c.start.max(from), c.end(&tempo).min(to));
                    (e - s >= MIN_CLIP_BEATS).then(|| {
                        let mut piece = c.clone();
                        piece.id = crate::new_id("c");
                        trim(&mut piece, s, e - s, &tempo, None);
                        piece.start -= from;
                        piece
                    })
                })
                .collect()
        })
        .collect();
    let inside = |t: f64| t >= from - 1e-9 && t < to - 1e-9;
    let points = project
        .tracks
        .iter()
        .map(|track| track.automation.iter().map(|lane| lane.points.iter().filter(|p| inside(p.time)).map(|p| AutoPoint { time: p.time - from, value: p.value }).collect()).collect())
        .collect();
    let markers = project
        .markers
        .iter()
        .filter(|m| inside(m.start))
        .map(|m| Marker { id: if fresh { crate::new_id("m") } else { m.id.clone() }, name: m.name.clone(), start: m.start - from, end: m.end.map(|e| e.min(to) - from) })
        .collect();
    Section { clips, points, markers }
}

/// Takes `from`…`to` (beats) out of the song: what is inside goes, what
/// comes after moves up to close the gap.
fn remove_time(project: &mut Project, from: f64, to: f64) {
    let tempo = project.tempo_map();
    let len = to - from;
    for track in &mut project.tracks {
        clear_range(track, from, to, &tempo);
        shift_after(track, to, -len, &[]);
        for clip in track.takes.iter_mut().flat_map(|l| l.clips.iter_mut()).filter(|c| c.start >= to - 1e-9) {
            clip.start -= len;
        }
        for lane in &mut track.automation {
            lane.points.retain(|p| p.time < from - 1e-9 || p.time >= to - 1e-9);
            for p in lane.points.iter_mut().filter(|p| p.time >= to - 1e-9) {
                p.time -= len;
            }
        }
    }
    project.markers.retain(|m| m.start < from - 1e-9 || m.start >= to - 1e-9);
    for m in &mut project.markers {
        if m.start >= to - 1e-9 {
            m.start -= len;
            m.end = m.end.map(|e| e - len);
        } else if let Some(e) = m.end.filter(|e| *e > from) {
            // A region reaching into the stretch ends where it began, or loses its length.
            m.end = Some(if e >= to { e - len } else { from }.max(m.start + MIN_CLIP_BEATS));
        }
    }
}

/// Opens `len` beats of room at `at`: clips across it are split there, and
/// everything from `at` on moves later.
fn open_time(project: &mut Project, at: f64, len: f64) {
    let tempo = project.tempo_map();
    for track in &mut project.tracks {
        let mut clips = Vec::with_capacity(track.clips.len() + 1);
        for clip in std::mem::take(&mut track.clips) {
            if clip.slot.is_none() && clip.start < at - 1e-9 && clip.end(&tempo) > at + 1e-9 {
                let (left, right) = split(&clip, at, &tempo);
                clips.push(left);
                clips.push(right);
            } else {
                clips.push(clip);
            }
        }
        track.clips = clips;
        shift_after(track, at, len, &[]);
        for clip in track.takes.iter_mut().flat_map(|l| l.clips.iter_mut()).filter(|c| c.start >= at - 1e-9) {
            clip.start += len;
        }
        for p in track.automation.iter_mut().flat_map(|l| l.points.iter_mut()).filter(|p| p.time >= at - 1e-9) {
            p.time += len;
        }
    }
    for m in &mut project.markers {
        if m.start >= at - 1e-9 {
            m.start += len;
            m.end = m.end.map(|e| e + len);
        } else if let Some(e) = m.end.filter(|e| *e > at + 1e-9) {
            m.end = Some(e + len);
        }
    }
}

/// Puts a lifted stretch back at `at` (into room opened for it).
fn paste_section(project: &mut Project, at: f64, section: Section) {
    for ((track, clips), lanes) in project.tracks.iter_mut().zip(section.clips).zip(section.points) {
        track.clips.extend(clips.into_iter().map(|mut c| {
            c.start += at;
            c
        }));
        track.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
        for (lane, points) in track.automation.iter_mut().zip(lanes) {
            lane.points.extend(points.into_iter().map(|p| AutoPoint { time: p.time + at, value: p.value }));
            lane.points.sort_by(|a, b| a.time.total_cmp(&b.time));
        }
    }
    project.markers.extend(section.markers.into_iter().map(|m| Marker { start: m.start + at, end: m.end.map(|e| e + at), ..m }));
    project.markers.sort_by(|a, b| a.start.total_cmp(&b.start));
}

/// Seconds of fade where comped pieces meet.
const COMP_FADE: f64 = 0.005;

/// At least a short fade at an audio clip's start and/or end.
fn soften(clip: &mut Clip, start: bool, end: bool) {
    if let ClipContent::Audio { fade_in, fade_out, duration, .. } = &mut clip.content {
        let fade = COMP_FADE.min(*duration / 4.0);
        if start {
            *fade_in = fade_in.max(fade);
        }
        if end {
            *fade_out = fade_out.max(fade);
        }
    }
}

/// Loop recording: keeps every pass round the loop (`passes`, in the order
/// played) as a take lane under the track, and puts one on its timeline over
/// what was there: the last pass, or the one before it when the last was cut
/// short (under half the loop, `full` beats). Returns how many takes were added.
pub fn place_takes(project: &mut Project, track: &str, passes: Vec<Clip>, full: f64) -> usize {
    let tempo = project.tempo_map();
    let Some(track) = project.track_mut(track) else { return 0 };
    let Some(last) = passes.len().checked_sub(1) else { return 0 };
    let pick = if last == 0 || passes[last].length_beats(&tempo) >= 0.5 * full { last } else { last - 1 };
    // Numbered on from the highest take the track has (deleted ones leave gaps).
    let taken = track.takes.iter().filter_map(|l| l.name.strip_prefix("Take ")?.parse::<usize>().ok()).max().unwrap_or(0);
    for (i, clip) in passes.into_iter().enumerate() {
        if i == pick {
            let (s, e) = (clip.start, clip.end(&tempo));
            clear_range(track, s, e, &tempo);
            let mut main = clip.clone();
            main.id = crate::new_id("c");
            track.clips.push(main);
        }
        track.takes.push(TakeLane { id: crate::new_id("tk"), name: format!("Take {}", taken + i + 1), clips: vec![clip] });
    }
    track.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    last + 1
}

/// Writes the session clip in `scene`'s slot into the arrangement from
/// `from` to `to` (beats), over whatever the track had there: one copy per
/// time round its loop, the first starting `offset` beats into it. Returns
/// how many clips it placed.
pub fn record_slot(project: &mut Project, track: &str, scene: usize, from: f64, to: f64, offset: f64) -> usize {
    let tempo = project.tempo_map();
    let Some(track) = project.track_mut(track) else { return 0 };
    let Some(source) = track.slot(scene).cloned() else { return 0 };
    let len = source.length_beats(&tempo);
    if len < MIN_CLIP_BEATS || to - from < MIN_CLIP_BEATS {
        return 0;
    }
    clear_range(track, from, to, &tempo);
    let mut placed = 0;
    let (mut at, mut phase) = (from, offset.rem_euclid(len));
    while to - at >= MIN_CLIP_BEATS {
        let mut copy = source.clone();
        copy.id = crate::new_id("c");
        copy.slot = None;
        copy.follow = None;
        copy.start = at - phase;
        // An unwarped audio clip is as long in beats as the tempo where it lands makes it.
        let here = copy.length_beats(&tempo).max(MIN_CLIP_BEATS);
        let piece = (here - phase).min(to - at).max(MIN_CLIP_BEATS);
        trim(&mut copy, at, piece, &tempo, None);
        track.clips.push(copy);
        placed += 1;
        at += piece;
        phase = 0.0;
    }
    track.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    placed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(json: &str) -> Op {
        serde_json::from_str(json).expect("valid op json")
    }

    #[test]
    fn ops_deserialise_from_ui_json() {
        let parsed = op(r#"{"op":"setTrack","id":"t1","patch":{"volume":-3}}"#);
        assert!(matches!(parsed, Op::SetTrack { ref patch, .. } if patch.volume == Some(-3.0)));
        let parsed = op(r#"{"op":"setTimeSignature","beatsPerBar":3}"#);
        assert!(matches!(parsed, Op::SetTimeSignature { beats_per_bar: 3 }));
    }

    #[test]
    fn failed_op_leaves_project_and_history_untouched() {
        let mut project = Project::empty("Song");
        let mut history = History::new();
        let before = project.clone();
        let result = apply(&mut project, &mut history, op(r#"{"op":"removeTrack","id":"nope"}"#));
        assert!(result.is_err());
        assert_eq!(project, before);
        assert!(history.undo_label().is_none());
    }

    #[test]
    fn add_track_is_undoable() {
        let mut project = Project::empty("Song");
        let mut history = History::new();
        let applied = apply(&mut project, &mut history, op(r#"{"op":"addTrack","kind":"audio"}"#)).unwrap();
        assert_eq!(project.tracks.len(), 5);
        assert!(project.track(applied.created.as_deref().unwrap()).is_some());
        history.undo(&mut project);
        assert_eq!(project.tracks.len(), 4);
    }

    #[test]
    fn instruments_are_refused_on_audio_tracks() {
        let mut project = Project::empty("Song");
        let mut history = History::new();
        let audio = project.tracks[0].id.clone();
        let device = crate::catalog::device("synth", &[]).unwrap();
        let result = apply(&mut project, &mut history, Op::AddDevice { track: audio, device });
        assert!(matches!(result, Err(OpError::Invalid(_))));
    }

    /// A song at 60 bpm (one beat = one second) with a 10 s asset and a clip
    /// playing seconds 2–6 of it at beat 4.
    fn song_with_audio_clip() -> (Project, String) {
        let mut project = Project::empty("Song");
        project.bpm = 60.0;
        project.assets.push(crate::Asset {
            id: "a1".into(),
            name: "Take".into(),
            channels: 1,
            sample_rate: 1000,
            frames: 10_000,
            path: None,
            missing: false,
        });
        let clip = Clip::audio("Take", "a1", 4.0, 2.0, 4.0);
        let id = clip.id.clone();
        project.tracks[0].clips.push(clip);
        (project, id)
    }

    fn audio(project: &Project, id: &str) -> (f64, f64, f64) {
        let (ti, ci) = project.find_clip(id).unwrap();
        let c = &project.tracks[ti].clips[ci];
        let ClipContent::Audio { offset, duration, .. } = c.content else { panic!("audio clip") };
        (c.start, offset, duration)
    }

    #[test]
    fn a_warped_clip_keeps_its_beats_when_the_tempo_changes() {
        let (mut project, id) = song_with_audio_clip();
        let mut history = History::new();
        let length = |p: &Project| {
            let (ti, ci) = p.find_clip(&id).unwrap();
            p.tracks[ti].clips[ci].length_beats(&p.tempo_map())
        };
        // 4 s at 60 BPM: 4 beats. Unwarped at 120 BPM it covers 8.
        apply(&mut project, &mut history, Op::SetTempo { bpm: 120.0 }).unwrap();
        assert!((length(&project) - 8.0).abs() < 1e-9);
        // Warped with its own tempo at 60 BPM, it stays 4 beats at any tempo.
        apply(&mut project, &mut history, Op::SetWarp { clip: id.clone(), warp: Some(Warp { bpm: 60.0, mode: crate::WarpMode::Beats }) }).unwrap();
        assert!((length(&project) - 4.0).abs() < 1e-9);
        apply(&mut project, &mut history, Op::SetTempo { bpm: 90.0 }).unwrap();
        assert!((length(&project) - 4.0).abs() < 1e-9);
        // Splitting it in the middle cuts its audio in the middle too.
        let right = apply(&mut project, &mut history, Op::SplitClip { clip: id.clone(), at: 6.0 }).unwrap().created.unwrap();
        assert_eq!(audio(&project, &right), (6.0, 4.0, 2.0));
        let midi = project.tracks[2].id.clone();
        assert!(apply(&mut project, &mut history, Op::AddMidiClip { track: midi, start: 0.0, length: 4.0, notes: vec![], name: None, slot: None }).is_ok());
    }

    #[test]
    fn the_tempo_track_moves_unwarped_audio_in_beats() {
        let (mut project, id) = song_with_audio_clip();
        let mut history = History::new();
        // 60 BPM up to beat 4, then a jump to 120: the clip's 4 s now cover 8 beats.
        let points = vec![TempoPoint { time: 0.0, bpm: 60.0 }, TempoPoint { time: 4.0, bpm: 60.0 }, TempoPoint { time: 4.0, bpm: 120.0 }];
        apply(&mut project, &mut history, Op::SetTempoPoints { points }).unwrap();
        let (ti, ci) = project.find_clip(&id).unwrap();
        assert!((project.tracks[ti].clips[ci].length_beats(&project.tempo_map()) - 8.0).abs() < 1e-9);
        // Trimming its left edge by two beats (one second at 120) skips a second of audio.
        apply(&mut project, &mut history, Op::TrimClip { clip: id.clone(), start: 6.0, length: 6.0, ripple: false }).unwrap();
        let (start, offset, duration) = audio(&project, &id);
        assert!((start - 6.0).abs() < 1e-9 && (offset - 3.0).abs() < 1e-9 && (duration - 3.0).abs() < 1e-9, "{start} {offset} {duration}");
        apply(&mut project, &mut history, Op::SetTempoPoints { points: vec![] }).unwrap();
        assert!(project.tempo.is_empty() && project.bpm == 60.0);
    }

    /// Three clips on the first track (2 beats long at 4, 8 and 12) and one on the second at 4.
    fn song_with_clips() -> (Project, [String; 4]) {
        let (mut project, _) = song_with_audio_clip();
        project.tracks[0].clips.clear();
        let ids: Vec<String> = [(0, 4.0), (0, 8.0), (0, 12.0), (1, 4.0)]
            .iter()
            .map(|&(t, at)| {
                let c = Clip::audio("Take", "a1", at, 0.0, 2.0);
                let id = c.id.clone();
                project.tracks[t].clips.push(c);
                id
            })
            .collect();
        (project, [ids[0].clone(), ids[1].clone(), ids[2].clone(), ids[3].clone()])
    }

    fn starts(project: &Project, track: usize) -> Vec<f64> {
        project.tracks[track].clips.iter().map(|c| c.start).collect()
    }

    #[test]
    fn several_clips_move_delete_and_duplicate_together() {
        let (mut project, [a, b, _, d]) = song_with_clips();
        let mut history = History::new();
        apply(&mut project, &mut history, Op::MoveClips { clips: vec![a.clone(), d.clone()], delta: 1.0, tracks: 0 }).unwrap();
        assert_eq!((starts(&project, 0), starts(&project, 1)), (vec![5.0, 8.0, 12.0], vec![5.0]));
        // Down a track: the first track's clip lands on the second; the second has no audio track under it.
        assert!(apply(&mut project, &mut history, Op::MoveClips { clips: vec![a.clone(), d.clone()], delta: 0.0, tracks: 1 }).is_err());
        apply(&mut project, &mut history, Op::MoveClips { clips: vec![a.clone()], delta: -100.0, tracks: 1 }).unwrap();
        assert_eq!(starts(&project, 1), vec![0.0, 5.0], "never before the start");
        history.undo(&mut project);
        history.undo(&mut project);
        apply(&mut project, &mut history, Op::DuplicateClips { clips: vec![a.clone(), b.clone()], ripple: false }).unwrap();
        // The block 4–10 copies to 10–16, landing over the clip at 12.
        assert_eq!(starts(&project, 0), vec![4.0, 8.0, 10.0, 12.0, 14.0]);
        let copies: Vec<String> = project.tracks[0].clips.iter().filter(|c| c.start == 10.0 || c.start == 14.0).map(|c| c.id.clone()).collect();
        apply(&mut project, &mut history, Op::DeleteClips { clips: copies, ripple: false }).unwrap();
        assert_eq!(starts(&project, 0), vec![4.0, 8.0, 12.0]);
        assert_eq!(history.undo_label(), Some("Delete 2 clips"));
    }

    #[test]
    fn ripple_closes_gaps_and_makes_room() {
        let (mut project, [a, b, c, d]) = song_with_clips();
        let mut history = History::new();
        // Deleting the middle clip pulls the last one 2 beats earlier; the other track stays.
        apply(&mut project, &mut history, Op::DeleteClips { clips: vec![b.clone()], ripple: true }).unwrap();
        assert_eq!((starts(&project, 0), starts(&project, 1)), (vec![4.0, 10.0], vec![4.0]));
        history.undo(&mut project);
        // Two gaps at once: each clip after them moves by what came before it.
        apply(&mut project, &mut history, Op::DeleteClips { clips: vec![a.clone(), b.clone()], ripple: true }).unwrap();
        assert_eq!(starts(&project, 0), vec![8.0]);
        history.undo(&mut project);
        // Duplicating with ripple pushes the rest along.
        apply(&mut project, &mut history, Op::DuplicateClips { clips: vec![b.clone()], ripple: true }).unwrap();
        assert_eq!(starts(&project, 0), vec![4.0, 8.0, 10.0, 14.0]);
        history.undo(&mut project);
        // Trimming an end with ripple: the clips after follow it.
        apply(&mut project, &mut history, Op::TrimClip { clip: a.clone(), start: 4.0, length: 1.0, ripple: true }).unwrap();
        assert_eq!(starts(&project, 0), vec![4.0, 7.0, 11.0]);
        let _ = (c, d);
    }

    #[test]
    fn duplicate_and_split_new_put_a_piece_on_a_new_track() {
        let (mut project, id) = song_with_audio_clip();
        let mut history = History::new();
        let tracks = project.tracks.len();
        // Seconds 1–3 of the clip (at 60 BPM, beats 5–7; the clip plays 2–6 s of the file from beat 4).
        let copy = apply(&mut project, &mut history, Op::ExtractToTrack { clip: id.clone(), start: 1.0, end: 3.0, cut: false }).unwrap().created.unwrap();
        assert_eq!(project.tracks.len(), tracks + 1);
        assert_eq!(project.tracks[1].id, copy, "right below");
        let piece = project.tracks[1].clips[0].id.clone();
        assert_eq!(audio(&project, &piece), (5.0, 3.0, 2.0));
        assert_eq!(audio(&project, &id), (4.0, 2.0, 4.0), "the original is untouched");
        history.undo(&mut project);
        apply(&mut project, &mut history, Op::ExtractToTrack { clip: id.clone(), start: 1.0, end: 3.0, cut: true }).unwrap();
        let left: Vec<(f64, f64, f64)> = project.tracks[0].clips.iter().map(|c| audio(&project, &c.id)).collect();
        assert_eq!(left, vec![(4.0, 2.0, 1.0), (7.0, 5.0, 1.0)], "a gap where the piece was");
        assert_eq!(project.tracks[1].name, "Audio 1 (split)");
    }

    #[test]
    fn fade_shapes_are_kept_and_cleared() {
        let (mut project, [a, b, ..]) = song_with_clips();
        let mut history = History::new();
        apply(&mut project, &mut history, Op::SetFadeCurve { clips: vec![a.clone(), b.clone()], curve: Some(FadeCurve::SCurve) }).unwrap();
        let shape = |p: &Project, id: &str| {
            let (ti, ci) = p.find_clip(id).unwrap();
            match &p.tracks[ti].clips[ci].content {
                ClipContent::Audio { curve, .. } => *curve,
                ClipContent::Midi { .. } => None,
            }
        };
        assert_eq!(shape(&project, &b), Some(FadeCurve::SCurve));
        let json = serde_json::to_string(&project).unwrap();
        assert!(json.contains("\"curve\":\"sCurve\""));
        apply(&mut project, &mut history, Op::SetFadeCurve { clips: vec![a.clone()], curve: None }).unwrap();
        assert_eq!(shape(&project, &a), None);
    }

    #[test]
    fn split_keeps_the_audio_continuous() {
        let (mut project, id) = song_with_audio_clip();
        let mut history = History::new();
        let applied = apply(&mut project, &mut history, Op::SplitClip { clip: id.clone(), at: 5.5 }).unwrap();
        assert_eq!(audio(&project, &id), (4.0, 2.0, 1.5));
        assert_eq!(audio(&project, applied.created.as_deref().unwrap()), (5.5, 3.5, 2.5));
    }

    #[test]
    fn trimming_the_left_edge_slides_the_offset_but_not_past_the_file() {
        let (mut project, id) = song_with_audio_clip();
        let mut history = History::new();
        // Drag the left edge 3 beats earlier: only 2 s of audio exist before the offset.
        apply(&mut project, &mut history, Op::TrimClip { clip: id.clone(), start: 1.0, length: 7.0, ripple: false }).unwrap();
        assert_eq!(audio(&project, &id), (2.0, 0.0, 6.0));
        // Drag the right edge far out: stops at the end of the 10 s file.
        apply(&mut project, &mut history, Op::TrimClip { clip: id.clone(), start: 2.0, length: 50.0, ripple: false }).unwrap();
        assert_eq!(audio(&project, &id), (2.0, 0.0, 10.0));
    }

    #[test]
    fn moving_audio_onto_a_midi_track_is_refused() {
        let (mut project, id) = song_with_audio_clip();
        let mut history = History::new();
        let midi = project.tracks[2].id.clone();
        assert!(apply(&mut project, &mut history, Op::MoveClip { clip: id, track: midi, start: 0.0 }).is_err());
    }

    #[test]
    fn fades_never_overlap() {
        let (mut project, id) = song_with_audio_clip();
        let mut history = History::new();
        apply(&mut project, &mut history, Op::SetClipFades { clip: id.clone(), fade_in: 3.0, fade_out: 3.0 }).unwrap();
        let (ti, ci) = project.find_clip(&id).unwrap();
        let ClipContent::Audio { fade_in, fade_out, .. } = project.tracks[ti].clips[ci].content else { panic!() };
        assert_eq!((fade_in, fade_out), (2.0, 2.0));
    }

    #[test]
    fn regions_move_as_a_whole_and_get_numbered_names() {
        let mut project = Project::empty("Song");
        let mut history = History::new();
        let applied = apply(&mut project, &mut history, op(r#"{"op":"addMarker","start":8,"end":16}"#)).unwrap();
        let id = applied.created.unwrap();
        assert_eq!(project.markers[0].name, "Region 1");
        apply(&mut project, &mut history, Op::SetMarker { id: id.clone(), name: None, start: Some(4.0), end: None }).unwrap();
        assert_eq!((project.markers[0].start, project.markers[0].end), (4.0, Some(12.0)));
        apply(&mut project, &mut history, op(r#"{"op":"addMarker","start":2}"#)).unwrap();
        assert_eq!(project.markers[0].name, "Marker 1", "markers stay sorted by position");
        history.undo(&mut project);
        history.undo(&mut project);
        assert_eq!(project.markers[0].start, 8.0);
    }

    #[test]
    fn old_projects_load_without_markers_or_monitoring() {
        let mut json = serde_json::to_value(Project::empty("Song")).unwrap();
        json.as_object_mut().unwrap().remove("markers");
        json["tracks"][0].as_object_mut().unwrap().remove("monitor");
        let project: Project = serde_json::from_value(json).unwrap();
        assert!(project.markers.is_empty());
        assert_eq!(project.tracks[0].monitor, Monitor::Off);
    }

    #[test]
    fn notes_are_sorted_and_clamped_and_midi_clips_stay_on_midi_tracks() {
        let mut project = Project::empty("Song");
        let mut history = History::new();
        let audio = project.tracks[0].id.clone();
        let midi = project.tracks[2].id.clone();
        assert!(apply(&mut project, &mut history, Op::AddMidiClip { track: audio, start: 0.0, length: 4.0, notes: vec![], name: None, slot: None }).is_err());
        let applied = apply(&mut project, &mut history, Op::AddMidiClip { track: midi, start: 4.0, length: 4.0, notes: vec![], name: None, slot: None }).unwrap();
        let clip = applied.created.unwrap();
        let set = op(&format!(
            r#"{{"op":"setNotes","clip":"{clip}","label":"Draw notes","notes":[{{"start":2,"length":1,"pitch":64,"velocity":3}},{{"start":-1,"length":0,"pitch":60,"velocity":0.5}}]}}"#
        ));
        apply(&mut project, &mut history, set).unwrap();
        let (ti, ci) = project.find_clip(&clip).unwrap();
        let ClipContent::Midi { notes, .. } = &project.tracks[ti].clips[ci].content else { panic!("MIDI clip") };
        assert_eq!(notes[0].pitch, 60, "sorted by start");
        assert_eq!((notes[0].start, notes[0].length), (0.0, crate::MIN_NOTE));
        assert_eq!(notes[1].velocity, 1.0);
        assert_eq!(history.undo_label(), Some("Draw notes"));
    }

    #[test]
    fn device_samples_keep_their_audio_and_steps_snap() {
        let (mut project, clip) = song_with_audio_clip();
        let mut history = History::new();
        let midi = project.tracks[2].id.clone();
        let rack = crate::catalog::device("drum-rack", &[]).unwrap();
        apply(&mut project, &mut history, Op::AddDevice { track: midi.clone(), device: rack }).unwrap();
        let rack = project.tracks[2].devices[0].id.clone();
        apply(&mut project, &mut history, Op::SetDeviceSample { track: midi.clone(), device: rack.clone(), key: 38, asset: Some("a1".into()) }).unwrap();
        // The clip goes, but the pad still plays the audio.
        apply(&mut project, &mut history, Op::DeleteClip { clip }).unwrap();
        assert!(project.asset("a1").is_some());
        apply(&mut project, &mut history, Op::SetDeviceSample { track: midi.clone(), device: rack.clone(), key: 38, asset: None }).unwrap();
        assert!(project.asset("a1").is_none());
        apply(&mut project, &mut history, Op::SetParam { track: midi, device: rack, param: "p38.tune".into(), value: 2.6 }).unwrap();
        assert_eq!(project.tracks[2].devices[0].param("p38.tune"), Some(3.0));
    }

    #[test]
    fn drum_pads_and_returns_hold_audio_effects() {
        let mut project = Project::empty("Song");
        let mut history = History::new();
        let midi = project.tracks[2].id.clone();
        let rack = crate::catalog::device("drum-rack", &[]).unwrap();
        apply(&mut project, &mut history, Op::AddDevice { track: midi.clone(), device: rack }).unwrap();
        let rack = project.tracks[2].devices[0].id.clone();
        let pad = format!("{midi}/{rack}/38");
        let ret = format!("{midi}/{rack}/A");
        let device = |kind| crate::catalog::device(kind, &[]).unwrap();
        apply(&mut project, &mut history, Op::AddDevice { track: pad.clone(), device: device("comp") }).unwrap();
        apply(&mut project, &mut history, Op::AddDevice { track: ret.clone(), device: device("reverb") }).unwrap();
        assert!(apply(&mut project, &mut history, Op::AddDevice { track: pad.clone(), device: device("synth") }).is_err(), "no instruments on a pad");
        assert!(apply(&mut project, &mut history, Op::AddDevice { track: pad.clone(), device: device("arp") }).is_err(), "no MIDI effects on a pad");
        assert!(apply(&mut project, &mut history, Op::AddDevice { track: format!("{midi}/{rack}/100"), device: device("eq") }).is_err(), "no pad 100");
        let chains = &project.tracks[2].devices[0].pad_chains;
        assert_eq!(chains.iter().map(|c| (c.key.as_str(), c.devices.len())).collect::<Vec<_>>(), [("38", 1), ("A", 1)]);
        // Their knobs and switches work through the same chain path.
        let comp = chains[0].devices[0].id.clone();
        apply(&mut project, &mut history, Op::SetParam { track: pad.clone(), device: comp.clone(), param: "ratio".into(), value: 8.0 }).unwrap();
        apply(&mut project, &mut history, Op::ToggleDevice { track: pad.clone(), device: comp.clone() }).unwrap();
        let c = &project.chain(&pad).unwrap()[0];
        assert_eq!((c.param("ratio"), c.on), (Some(8.0), false));
        // Removing the last effect drops the pad's chain; undo brings it back.
        apply(&mut project, &mut history, Op::RemoveDevice { track: pad.clone(), device: comp }).unwrap();
        assert_eq!(project.tracks[2].devices[0].pad_chains.len(), 1);
        history.undo(&mut project);
        assert_eq!(project.tracks[2].devices[0].pad_chains.len(), 2);
    }

    #[test]
    fn clips_reverse_transpose_and_carry_a_gain_envelope() {
        // 60 BPM; the clip plays seconds 2–6 of a 10 s file from beat 4.
        let (mut project, clip) = song_with_audio_clip();
        let mut history = History::new();
        let audio = |p: &Project, i: usize| match &p.tracks[0].clips[i].content {
            ClipContent::Audio { offset, duration, reverse, transpose, envelope, .. } => (*offset, *duration, *reverse, *transpose, envelope.clone()),
            _ => unreachable!(),
        };
        let points = vec![EnvPoint { time: 3.0, gain: -90.0 }, EnvPoint { time: 1.0, gain: 0.0 }, EnvPoint { time: 9.0, gain: 3.0 }];
        apply(&mut project, &mut history, Op::SetClipEnvelope { clip: clip.clone(), points }).unwrap();
        // Sorted, inside the clip, silence at −60 dB.
        assert_eq!(audio(&project, 0).4, [EnvPoint { time: 1.0, gain: 0.0 }, EnvPoint { time: 3.0, gain: -60.0 }, EnvPoint { time: 4.0, gain: 3.0 }]);
        apply(&mut project, &mut history, Op::SetClipTranspose { clip: clip.clone(), semitones: 30.4 }).unwrap();
        assert_eq!(audio(&project, 0).3, 24.0);
        apply(&mut project, &mut history, Op::ReverseClips { clips: vec![clip.clone()] }).unwrap();
        assert!(audio(&project, 0).2);

        // Split a beat in: backwards, the left piece plays the last second of the stretch.
        apply(&mut project, &mut history, Op::SplitClip { clip: clip.clone(), at: 5.0 }).unwrap();
        assert_eq!((audio(&project, 0).0, audio(&project, 0).1), (5.0, 1.0));
        assert_eq!((audio(&project, 1).0, audio(&project, 1).1), (2.0, 3.0));
        // The envelope stays where it was in the song: the right piece's points move a beat earlier.
        assert_eq!(audio(&project, 1).4.iter().map(|p| p.time).collect::<Vec<_>>(), [0.0, 2.0, 3.0]);
        // The left piece keeps what falls inside it.
        assert_eq!(audio(&project, 0).4, [EnvPoint { time: 1.0, gain: 0.0 }]);
        // Trimming the right piece's start by a beat takes a second off the end of its stretch.
        let right = project.tracks[0].clips[1].id.clone();
        apply(&mut project, &mut history, Op::TrimClip { clip: right, start: 6.0, length: 2.0, ripple: false }).unwrap();
        assert_eq!((audio(&project, 1).0, audio(&project, 1).1), (2.0, 2.0));
        // Its envelope is cut at the new start, at the level the line had there (halfway from 0 to −60 dB).
        assert_eq!(audio(&project, 1).4, [EnvPoint { time: 0.0, gain: -30.0 }, EnvPoint { time: 1.0, gain: -60.0 }, EnvPoint { time: 2.0, gain: 3.0 }]);
        // Forwards again.
        apply(&mut project, &mut history, Op::ReverseClips { clips: vec![clip] }).unwrap();
        assert!(!audio(&project, 0).2);
        let midi = project.tracks[2].id.clone();
        let m = apply(&mut project, &mut history, Op::AddMidiClip { track: midi, start: 0.0, length: 4.0, notes: vec![], name: None, slot: None }).unwrap().created.unwrap();
        assert!(apply(&mut project, &mut history, Op::ReverseClips { clips: vec![m] }).is_err());
    }

    #[test]
    fn time_signature_changes_are_kept_in_order_and_meaningful() {
        let mut project = Project::empty("Song");
        let mut history = History::new();
        let mut set = |project: &mut Project, bar, beats| apply(project, &mut history, Op::SetMeter { bar, beats_per_bar: beats });
        assert!(set(&mut project, 0, Some(3)).is_err(), "the first bar is the song's own");
        set(&mut project, 8, Some(3)).unwrap();
        set(&mut project, 4, Some(5)).unwrap();
        set(&mut project, 6, Some(5)).unwrap();
        // The change at bar 6 repeats the one before it: it goes.
        assert_eq!(project.meters.iter().map(|m| (m.bar, m.beats_per_bar)).collect::<Vec<_>>(), [(4, 5), (8, 3)]);
        assert_eq!(project.meter_map().bar_start(9), 4.0 * 4.0 + 4.0 * 5.0 + 3.0);
        set(&mut project, 4, None).unwrap();
        assert_eq!(project.meters.len(), 1);
    }

    #[test]
    fn sections_move_duplicate_and_delete_as_blocks() {
        // 60 BPM. A: beats 0–4, B: 4–8 (the 4 s clip of the test song), C: 8–12.
        let (mut project, b) = song_with_audio_clip();
        let mut history = History::new();
        let track = project.tracks[0].id.clone();
        project.tracks[0].clips.push(Clip::audio("A", "a1", 0.0, 0.0, 4.0));
        project.tracks[0].clips.push(Clip::audio("C", "a1", 8.0, 6.0, 4.0));
        project.tracks[0].clips.sort_by(|x, y| x.start.total_cmp(&y.start));
        apply(&mut project, &mut history, Op::SetAutomation { track: track.clone(), target: AutoTarget::Volume, points: vec![AutoPoint { time: 1.0, value: -3.0 }, AutoPoint { time: 5.0, value: -6.0 }, AutoPoint { time: 9.0, value: -9.0 }] }).unwrap();
        let region = apply(&mut project, &mut history, Op::AddMarker { name: Some("Verse".into()), start: 4.0, end: Some(8.0) }).unwrap().created.unwrap();
        apply(&mut project, &mut history, Op::AddMarker { name: Some("End".into()), start: 10.0, end: None }).unwrap();
        let names = |p: &Project| p.tracks[0].clips.iter().map(|c| (c.name.clone(), c.start)).collect::<Vec<_>>();
        let auto = |p: &Project| p.tracks[0].automation[0].points.iter().map(|x| (x.time, x.value)).collect::<Vec<_>>();
        let marks = |p: &Project| p.markers.iter().map(|m| (m.name.clone(), m.start, m.end)).collect::<Vec<_>>();
        let named = |n: &str, s: f64| (n.to_string(), s);

        // The verse to the front: A and everything after it make room.
        apply(&mut project, &mut history, Op::MoveSection { marker: region.clone(), to: 0.0 }).unwrap();
        assert_eq!(names(&project), [named("Take", 0.0), named("A", 4.0), named("C", 8.0)]);
        assert_eq!(auto(&project), [(1.0, -6.0), (5.0, -3.0), (9.0, -9.0)]);
        assert_eq!(marks(&project), [("Verse".to_string(), 0.0, Some(4.0)), ("End".to_string(), 10.0, None)]);
        assert!(project.tracks[0].clips.iter().any(|c| c.id != b), "the pieces are new clips");

        // To the end (dropped at beat 12): A and C close up, the verse follows them.
        apply(&mut project, &mut history, Op::MoveSection { marker: region.clone(), to: 12.0 }).unwrap();
        assert_eq!(names(&project), [named("A", 0.0), named("C", 4.0), named("Take", 8.0)]);
        assert_eq!(marks(&project), [("End".to_string(), 6.0, None), ("Verse".to_string(), 8.0, Some(12.0))]);
        assert!(apply(&mut project, &mut history, Op::MoveSection { marker: region.clone(), to: 10.0 }).is_err(), "not into itself");

        // A copy right after it, with its automation and a region of its own.
        apply(&mut project, &mut history, Op::DuplicateSection { marker: region.clone() }).unwrap();
        assert_eq!(names(&project), [named("A", 0.0), named("C", 4.0), named("Take", 8.0), named("Take", 12.0)]);
        assert_eq!(auto(&project), [(1.0, -3.0), (5.0, -9.0), (9.0, -6.0), (13.0, -6.0)]);
        assert_eq!(project.markers.iter().filter(|m| m.name == "Verse").count(), 2);

        // Deleting the first verse closes the gap; a clip across the edge of a section is cut there.
        apply(&mut project, &mut history, Op::DeleteSection { marker: region }).unwrap();
        assert_eq!(names(&project), [named("A", 0.0), named("C", 4.0), named("Take", 8.0)]);
        let across = apply(&mut project, &mut history, Op::AddMarker { name: Some("Half".into()), start: 2.0, end: Some(6.0) }).unwrap().created.unwrap();
        apply(&mut project, &mut history, Op::DeleteSection { marker: across }).unwrap();
        assert_eq!(names(&project), [named("A", 0.0), named("C", 2.0), named("Take", 4.0)]);
        assert_eq!(history.undo_label(), Some("Delete section “Half”"));
    }

    #[test]
    fn loop_takes_are_kept_and_comped_from() {
        // 60 BPM: a beat is a second. A 10 s recording over a 4-beat loop at beat 4: a
        // first pass from beat 4, a second, and a third cut short after one beat.
        let (mut project, old) = song_with_audio_clip();
        let mut history = History::new();
        let track = project.tracks[0].id.clone();
        let pass = |offset: f64, len: f64| Clip::audio("Take", "a1", 4.0, offset, len);
        assert_eq!(place_takes(&mut project, &track, vec![pass(0.0, 4.0), pass(4.0, 4.0), pass(8.0, 1.0)], 4.0), 3);
        let t = &project.tracks[0];
        assert_eq!(t.takes.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Take 1", "Take 2", "Take 3"]);
        // The second pass plays (the third was cut short), over the clip that was there.
        assert!(t.clips.iter().all(|c| c.id != old));
        let offset_of = |c: &Clip| match c.content {
            ClipContent::Audio { offset, .. } => offset,
            _ => unreachable!(),
        };
        assert_eq!(t.clips.iter().map(|c| (c.start, offset_of(c))).collect::<Vec<_>>(), [(4.0, 4.0)]);

        // Swipe beats 5–6 of Take 1 into the comp: three pieces, joined with short fades.
        let lane = t.takes[0].id.clone();
        apply(&mut project, &mut history, Op::CompTake { track: track.clone(), lane: lane.clone(), start: 6.0, end: 5.0 }).unwrap();
        let t = &project.tracks[0];
        assert_eq!(t.clips.iter().map(|c| (c.start, offset_of(c))).collect::<Vec<_>>(), [(4.0, 4.0), (5.0, 1.0), (6.0, 6.0)]);
        let fades = |c: &Clip| match c.content {
            ClipContent::Audio { fade_in, fade_out, .. } => (fade_in, fade_out),
            _ => unreachable!(),
        };
        assert_eq!(t.clips.iter().map(fades).collect::<Vec<_>>(), [(0.0, 0.005), (0.005, 0.005), (0.005, 0.0)]);
        assert_eq!(history.undo_label(), Some("Comp from Take 1"));
        assert!(apply(&mut project, &mut history, Op::CompTake { track: track.clone(), lane: lane.clone(), start: 20.0, end: 24.0 }).is_err(), "nothing recorded there");

        // The takes keep the recording alive without the timeline; without them it goes.
        for id in project.tracks[0].clips.iter().map(|c| c.id.clone()).collect::<Vec<_>>() {
            apply(&mut project, &mut history, Op::DeleteClip { clip: id }).unwrap();
        }
        assert!(project.asset("a1").is_some());
        apply(&mut project, &mut history, Op::DeleteTake { track: track.clone(), lane }).unwrap();
        assert_eq!(project.tracks[0].takes.len(), 2);
        apply(&mut project, &mut history, Op::ClearTakes { track: track.clone() }).unwrap();
        assert!(project.asset("a1").is_none());
        history.undo(&mut project);
        assert_eq!(project.tracks[0].takes.len(), 2);
        // More takes carry on numbering.
        place_takes(&mut project, &track, vec![pass(0.0, 4.0), pass(4.0, 4.0)], 4.0);
        assert_eq!(project.tracks[0].takes.last().unwrap().name, "Take 5");
    }

    #[test]
    fn a_scene_goes_into_the_arrangement_as_long_as_its_longest_clip() {
        let mut project = Project::empty("Song");
        let mut history = History::new();
        let (a, b) = (project.tracks[2].id.clone(), project.tracks[3].id.clone());
        let mut add = |track: &String, length| {
            apply(&mut project, &mut history, Op::AddMidiClip { track: track.clone(), start: 0.0, length, notes: vec![], name: None, slot: Some(0) }).unwrap();
        };
        add(&a, 4.0);
        add(&b, 8.0);
        assert!(apply(&mut project, &mut history, Op::SceneToArrangement { scene: 3, at: 0.0 }).is_err());
        apply(&mut project, &mut history, Op::SceneToArrangement { scene: 0, at: 8.0 }).unwrap();
        let timeline = |i: usize| project.tracks[i].clips.iter().filter(|c| c.slot.is_none()).map(|c| c.start).collect::<Vec<_>>();
        assert_eq!(timeline(2), [8.0, 12.0], "the short clip twice");
        assert_eq!(timeline(3), [8.0]);
        // The scene's own clips stay in their slots; one undo step takes the copies away.
        assert!(project.tracks[2].slot(0).is_some());
        history.undo(&mut project);
        assert!(project.tracks[2].clips.iter().all(|c| c.slot.is_some()));
    }

    #[test]
    fn linked_clips_share_their_notes_until_unlinked() {
        let mut project = Project::empty("Song");
        let mut history = History::new();
        let midi = project.tracks[2].id.clone();
        let first = apply(&mut project, &mut history, Op::AddMidiClip { track: midi, start: 0.0, length: 4.0, notes: vec![], name: None, slot: None })
            .unwrap()
            .created
            .unwrap();
        let copy = apply(&mut project, &mut history, Op::DuplicateClip { clip: first.clone(), linked: true }).unwrap().created.unwrap();
        let plain = apply(&mut project, &mut history, Op::DuplicateClip { clip: copy.clone(), linked: false }).unwrap().created.unwrap();
        let note = Note { start: 1.0, length: 1.0, pitch: 60, velocity: 0.8 };
        apply(&mut project, &mut history, Op::SetNotes { clip: copy.clone(), notes: vec![note.clone()], label: None }).unwrap();
        let notes_of = |p: &Project, id: &str| {
            let (ti, ci) = p.find_clip(id).unwrap();
            let ClipContent::Midi { notes, .. } = &p.tracks[ti].clips[ci].content else { panic!() };
            notes.len()
        };
        assert_eq!((notes_of(&project, &first), notes_of(&project, &copy)), (1, 1), "an edit to one shows in both");
        assert_eq!(notes_of(&project, &plain), 0, "a plain duplicate is a clip of its own");
        apply(&mut project, &mut history, Op::UnlinkClip { clip: copy.clone() }).unwrap();
        apply(&mut project, &mut history, Op::SetNotes { clip: first.clone(), notes: vec![], label: None }).unwrap();
        assert_eq!((notes_of(&project, &first), notes_of(&project, &copy)), (0, 1));
    }

    #[test]
    fn midi_effects_go_before_the_instrument() {
        let mut project = Project::empty("Song");
        let mut history = History::new();
        let midi = project.tracks[2].id.clone();
        let add = |project: &mut Project, history: &mut History, kind: &str| {
            let device = crate::catalog::device(kind, &[]).unwrap();
            apply(project, history, Op::AddDevice { track: midi.clone(), device }).unwrap();
        };
        add(&mut project, &mut history, "reverb");
        add(&mut project, &mut history, "arp");
        add(&mut project, &mut history, "chord");
        add(&mut project, &mut history, "epiano");
        let kinds: Vec<&str> = project.tracks[2].devices.iter().map(|d| d.kind.as_str()).collect();
        assert_eq!(kinds, ["arp", "chord", "epiano", "reverb"]);
        let audio = project.tracks[0].id.clone();
        let arp = crate::catalog::device("arp", &[]).unwrap();
        assert!(apply(&mut project, &mut history, Op::AddDevice { track: audio, device: arp }).is_err());
    }

    #[test]
    fn automation_lanes_hold_their_range_and_go_with_their_device() {
        let mut project = Project::empty("Song");
        let mut history = History::new();
        let midi = project.tracks[2].id.clone();
        let synth = project.tracks[2].devices[0].id.clone();
        let cutoff = AutoTarget::Param { device: synth.clone(), param: "cutoff".into() };
        let points = vec![AutoPoint { time: 8.0, value: 99_999.0 }, AutoPoint { time: -2.0, value: 100.0 }];
        apply(&mut project, &mut history, Op::SetAutomation { track: midi.clone(), target: cutoff.clone(), points }).unwrap();
        let lane = &project.tracks[2].automation[0];
        assert_eq!(lane.points[0], AutoPoint { time: 0.0, value: 100.0 }, "sorted, and time starts at 0");
        assert_eq!(lane.points[1].value, 18_000.0, "kept inside the parameter's range");
        assert_eq!(lane.value_at(4.0), Some(9050.0));
        assert_eq!(lane.value_at(20.0), Some(18_000.0));
        let vol = vec![AutoPoint { time: 0.0, value: -100.0 }];
        apply(&mut project, &mut history, Op::SetAutomation { track: midi.clone(), target: AutoTarget::Volume, points: vol }).unwrap();
        assert_eq!(project.tracks[2].automation[1].points[0].value, -60.0);
        assert!(apply(&mut project, &mut history, Op::SetAutomation { track: midi.clone(), target: AutoTarget::Send { index: 5 }, points: vec![] }).is_err());

        // An LFO on the track moves the synth's cutoff, until the synth goes.
        let lfo = crate::catalog::device("lfo", &[]).unwrap();
        apply(&mut project, &mut history, Op::AddDevice { track: midi.clone(), device: lfo }).unwrap();
        let lfo = project.tracks[2].devices.last().unwrap().id.clone();
        let target = Some(ParamRef { device: synth.clone(), param: "cutoff".into() });
        apply(&mut project, &mut history, Op::SetModTarget { track: midi.clone(), device: lfo.clone(), target }).unwrap();
        let itself = Some(ParamRef { device: lfo.clone(), param: "rate".into() });
        assert!(apply(&mut project, &mut history, Op::SetModTarget { track: midi.clone(), device: lfo.clone(), target: itself }).is_err());
        apply(&mut project, &mut history, Op::RemoveDevice { track: midi.clone(), device: synth }).unwrap();
        assert_eq!(project.tracks[2].automation.len(), 1, "only the volume lane is left");
        assert_eq!(project.tracks[2].devices[0].modulates, None);
    }

    #[test]
    fn groups_gather_tracks_and_let_them_go() {
        let mut project = Project::empty("Song");
        let mut history = History::new();
        let ids: Vec<String> = project.tracks.iter().map(|t| t.id.clone()).collect();
        let group = apply(&mut project, &mut history, Op::GroupTracks { tracks: vec![ids[3].clone(), ids[1].clone()] }).unwrap().created.unwrap();
        let order: Vec<(&str, Option<&str>)> = project.tracks.iter().map(|t| (t.name.as_str(), t.group.as_deref())).collect();
        let g = Some(group.as_str());
        assert_eq!(order, [("Audio 1", None), ("Group 1", None), ("Audio 2", g), ("MIDI 2", g), ("MIDI 1", None)]);
        assert!(apply(&mut project, &mut history, Op::GroupTracks { tracks: vec![group.clone()] }).is_err(), "no groups in groups");
        apply(&mut project, &mut history, Op::SetTrackGroup { track: ids[0].clone(), group: Some(group.clone()) }).unwrap();
        let names: Vec<&str> = project.tracks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Group 1", "Audio 2", "MIDI 2", "Audio 1", "MIDI 1"]);
        apply(&mut project, &mut history, Op::Ungroup { group }).unwrap();
        assert!(project.tracks.iter().all(|t| t.group.is_none() && t.kind != TrackKind::Group));
    }

    #[test]
    fn a_recorded_session_clip_loops_over_the_arrangement() {
        let mut project = Project::empty("Song");
        let mut history = History::new();
        let midi = project.tracks[2].id.clone();
        let note = |start: f64| Note { start, length: 0.5, pitch: 60, velocity: 0.8 };
        // On the timeline: a clip over beats 0–16. In scene 0: a 4-beat clip.
        let add = |project: &mut Project, history: &mut History, length, notes, slot| {
            apply(project, history, Op::AddMidiClip { track: midi.clone(), start: 0.0, length, notes, name: None, slot }).unwrap();
        };
        add(&mut project, &mut history, 16.0, vec![note(0.0), note(15.0)], None);
        add(&mut project, &mut history, 4.0, vec![note(0.0), note(3.0)], Some(0));
        // Played from beat 5 to beat 14, launched 2 beats into its loop.
        assert_eq!(record_slot(&mut project, &midi, 0, 5.0, 14.0, 2.0), 3);
        let track = &project.tracks[2];
        let spans: Vec<(f64, f64)> = track.timeline().map(|c| (c.start, c.length_beats(&TempoMap::constant(120.0)))).collect();
        assert_eq!(spans, [(0.0, 5.0), (5.0, 2.0), (7.0, 4.0), (11.0, 3.0), (14.0, 2.0)]);
        let starts = |i: usize| match &track.timeline().nth(i).unwrap().content {
            ClipContent::Midi { notes, .. } => notes.iter().map(|n| n.start).collect::<Vec<_>>(),
            ClipContent::Audio { .. } => unreachable!(),
        };
        assert_eq!(starts(1), [1.0], "the first copy starts 2 beats in");
        assert_eq!(starts(2), [0.0, 3.0]);
        assert_eq!(starts(0), [0.0], "what was left of the timeline clip before beat 5");
        assert_eq!(starts(4), [1.0], "and after beat 14");
        assert!(track.slot(0).is_some(), "the session clip stays in its slot");
    }

    #[test]
    fn session_clips_wait_in_slots_off_the_timeline() {
        let mut project = Project::empty("Song");
        let mut history = History::new();
        let midi = project.tracks[2].id.clone();
        let clip = apply(&mut project, &mut history, Op::AddMidiClip { track: midi.clone(), start: 8.0, length: 4.0, notes: vec![], name: None, slot: None })
            .unwrap()
            .created
            .unwrap();
        let copy = apply(&mut project, &mut history, Op::PutInSlot { clip: clip.clone(), scene: 1 }).unwrap().created.unwrap();
        let track = &project.tracks[2];
        assert_eq!(track.timeline().count(), 1);
        assert_eq!(track.slot(1).map(|c| (c.id.as_str(), c.start)), Some((copy.as_str(), 0.0)));
        assert!(apply(&mut project, &mut history, Op::MoveClip { clip: copy.clone(), track: midi.clone(), start: 2.0 }).is_err());
        let follow = Some(FollowAction { action: crate::Follow::Next, bars: 2.0 });
        apply(&mut project, &mut history, Op::SetFollow { clip: copy.clone(), follow }).unwrap();
        assert!(apply(&mut project, &mut history, Op::SetFollow { clip, follow }).is_err(), "timeline clips have no follow action");
        apply(&mut project, &mut history, Op::AddMidiClip { track: midi.clone(), start: 3.0, length: 8.0, notes: vec![], name: None, slot: Some(1) }).unwrap();
        assert_eq!(project.tracks[2].clips.iter().filter(|c| c.slot == Some(1)).count(), 1, "a new clip replaces the slot's");
        apply(&mut project, &mut history, Op::ClearSlot { track: midi, scene: 1 }).unwrap();
        assert!(project.tracks[2].slot(1).is_none());

        // Old files named timeline clips in their slots.
        let mut json = serde_json::to_value(&project).unwrap();
        json["tracks"][2]["slots"] = serde_json::json!([null, "MIDI 1 1", null]);
        let mut old: Project = serde_json::from_value(json).unwrap();
        old.migrate_slots();
        assert_eq!(old.tracks[2].slot(1).map(|c| c.start), Some(0.0));
        assert!(serde_json::to_value(&old).unwrap()["tracks"][2].get("slots").is_none());
    }

    #[test]
    fn returns_and_the_master_hold_effects_and_sidechains_follow_tracks() {
        let mut project = Project::empty("Song");
        let mut history = History::new();
        let ret = project.returns[0].id.clone();
        let master = crate::MASTER.to_string();
        let device = |kind: &str| crate::catalog::device(kind, &[]).unwrap();
        apply(&mut project, &mut history, Op::AddDevice { track: master.clone(), device: device("comp") }).unwrap();
        apply(&mut project, &mut history, Op::AddDevice { track: ret.clone(), device: device("eq") }).unwrap();
        assert_eq!(project.returns[0].devices.len(), 2);
        assert!(apply(&mut project, &mut history, Op::AddDevice { track: master.clone(), device: device("synth") }).is_err(), "instruments need a MIDI track");
        assert!(apply(&mut project, &mut history, Op::AddDevice { track: ret.clone(), device: device("arp") }).is_err());

        // The master's compressor listens to a track; an EQ cannot.
        let kick = project.tracks[2].id.clone();
        let comp = project.master_devices[0].id.clone();
        apply(&mut project, &mut history, Op::SetSidechain { track: master.clone(), device: comp.clone(), source: Some(kick.clone()) }).unwrap();
        assert_eq!(project.master_devices[0].sidechain.as_deref(), Some(kick.as_str()));
        let eq = project.returns[0].devices[1].id.clone();
        assert!(apply(&mut project, &mut history, Op::SetSidechain { track: ret.clone(), device: eq.clone(), source: Some(kick.clone()) }).is_err());
        let vocal_comp = device("comp");
        let audio = project.tracks[0].id.clone();
        apply(&mut project, &mut history, Op::AddDevice { track: audio.clone(), device: vocal_comp }).unwrap();
        let own = project.tracks[0].devices[0].id.clone();
        assert!(apply(&mut project, &mut history, Op::SetSidechain { track: audio.clone(), device: own, source: Some(audio) }).is_err(), "not itself");

        // Parameters of a return's device, and the return's pre/post switch.
        apply(&mut project, &mut history, Op::SetParam { track: ret.clone(), device: eq, param: "b4gain".into(), value: 3.0 }).unwrap();
        assert_eq!(project.returns[0].devices[1].param("b4gain"), Some(3.0));
        apply(&mut project, &mut history, Op::SetReturn { id: ret, patch: ReturnPatch { pre_fader: Some(true), ..Default::default() } }).unwrap();
        assert!(project.returns[0].pre_fader);

        // Deleting the track lets the compressor hear its own input again.
        apply(&mut project, &mut history, Op::RemoveTrack { id: kick }).unwrap();
        assert_eq!(project.master_devices[0].sidechain, None);
        let json = serde_json::to_string(&project).unwrap();
        assert_eq!(serde_json::from_str::<Project>(&json).unwrap(), project);
    }

    #[test]
    fn racks_hold_chains_that_macros_turn() {
        let mut project = Project::empty("Song");
        let mut history = History::new();
        let midi = project.tracks[2].id.clone();
        let rack = crate::catalog::device("rack", &[]).unwrap();
        apply(&mut project, &mut history, Op::AddDevice { track: midi.clone(), device: rack }).unwrap();
        let rack = project.tracks[2].devices[0].id.clone();
        let add = |project: &mut Project, history: &mut History, kind: &str| {
            apply(project, history, Op::AddChain { track: midi.clone(), device: rack.clone(), kind: kind.into() }).map(|a| a.created.unwrap())
        };
        let synth = add(&mut project, &mut history, "synth").unwrap();
        let keys = add(&mut project, &mut history, "epiano").unwrap();
        assert!(add(&mut project, &mut history, "rack").is_err(), "no racks in racks");
        assert!(add(&mut project, &mut history, "reverb").is_err());

        // Zones keep low ≤ high.
        let patch = ChainPatch { key_low: Some(60), ..Default::default() };
        apply(&mut project, &mut history, Op::SetChain { track: midi.clone(), device: rack.clone(), chain: keys.clone(), patch }).unwrap();
        let patch = ChainPatch { key_high: Some(40), ..Default::default() };
        apply(&mut project, &mut history, Op::SetChain { track: midi.clone(), device: rack.clone(), chain: keys.clone(), patch }).unwrap();
        let chain = &project.tracks[2].devices[0].chains[1];
        assert_eq!((chain.key_low, chain.key_high), (40, 40));

        // Chain instruments take parameter changes like any device.
        let synth_device = project.tracks[2].devices[0].chains[0].device.id.clone();
        apply(&mut project, &mut history, Op::SetParam { track: midi.clone(), device: synth_device, param: "cutoff".into(), value: 1000.0 }).unwrap();

        // The first mapping takes the parameter's place; then the macro drives it, by octave for a log parameter.
        let map = Op::MapMacro {
            track: midi.clone(),
            device: rack.clone(),
            chain: synth.clone(),
            inner: None,
            param: "cutoff".into(),
            index: Some(0),
            min: Some(100.0),
            max: Some(10_000.0),
        };
        apply(&mut project, &mut history, map).unwrap();
        let device = &project.tracks[2].devices[0];
        assert!((device.param("macro1").unwrap() - 50.0).abs() < 1e-9);
        assert!((device.chain_param(&device.chains[0], "cutoff").unwrap() - 1000.0).abs() < 1e-6);
        apply(&mut project, &mut history, Op::SetParam { track: midi.clone(), device: rack.clone(), param: "macro1".into(), value: 100.0 }).unwrap();
        let device = &project.tracks[2].devices[0];
        assert!((device.chain_param(&device.chains[0], "cutoff").unwrap() - 10_000.0).abs() < 1e-6);

        // Removing a chain drops its mappings.
        apply(&mut project, &mut history, Op::RemoveChain { track: midi, device: rack, chain: synth }).unwrap();
        let device = &project.tracks[2].devices[0];
        assert_eq!((device.chains.len(), device.macros.len()), (1, 0));
        let json = serde_json::to_string(&project).unwrap();
        assert_eq!(serde_json::from_str::<Project>(&json).unwrap(), project);
    }

    #[test]
    fn effect_racks_hold_chains_of_effects_that_macros_turn() {
        let (mut project, _) = song_with_audio_clip();
        let mut history = History::new();
        let device = |kind: &str| crate::catalog::device(kind, &[]).unwrap();
        // On the master: effect racks go wherever effects go.
        let master = crate::MASTER.to_string();
        apply(&mut project, &mut history, Op::AddDevice { track: master.clone(), device: device("fx-rack") }).unwrap();
        let rack = project.master_devices[0].id.clone();
        let add = |project: &mut Project, history: &mut History| apply(project, history, Op::AddChain { track: master.clone(), device: rack.clone(), kind: String::new() }).map(|a| a.created.unwrap());
        let dry = add(&mut project, &mut history).unwrap();
        let wet = add(&mut project, &mut history).unwrap();
        assert_eq!(project.master_devices[0].fx_chains.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Chain 1", "Chain 2"]);

        // A chain is a device chain of its own, for built-in audio effects.
        let inside = format!("{master}/{rack}/{wet}");
        apply(&mut project, &mut history, Op::AddDevice { track: inside.clone(), device: device("comp") }).unwrap();
        apply(&mut project, &mut history, Op::AddDevice { track: inside.clone(), device: device("convolution") }).unwrap();
        for kind in ["synth", "arp", "lfo", "fx-rack"] {
            assert!(apply(&mut project, &mut history, Op::AddDevice { track: inside.clone(), device: device(kind) }).is_err(), "{kind} in a rack chain");
        }
        assert!(apply(&mut project, &mut history, Op::AddDevice { track: format!("{master}/{rack}/nope"), device: device("eq") }).is_err());
        let chain = project.chain(&inside).unwrap();
        let (comp, reverb) = (chain[0].id.clone(), chain[1].id.clone());
        assert_eq!(chain.iter().map(|d| d.kind.as_str()).collect::<Vec<_>>(), ["comp", "convolution"]);

        // Its effects take parameter changes, and a reverb in it its own impulse response, which keeps the file in the song.
        apply(&mut project, &mut history, Op::SetParam { track: inside.clone(), device: comp.clone(), param: "thresh".into(), value: -30.0 }).unwrap();
        apply(&mut project, &mut history, Op::SetDeviceSample { track: inside.clone(), device: reverb.clone(), key: 0, asset: Some("a1".into()) }).unwrap();
        project.tracks[0].clips.clear();
        project.prune_assets();
        assert!(project.asset("a1").is_some(), "the impulse response is still used");

        // Level, pan and name; zones mean nothing here.
        let patch = ChainPatch { volume: Some(-90.0), pan: Some(0.5), name: Some(" Crushed ".into()), key_low: Some(40), ..Default::default() };
        apply(&mut project, &mut history, Op::SetChain { track: master.clone(), device: rack.clone(), chain: wet.clone(), patch }).unwrap();
        let c = &project.master_devices[0].fx_chains[1];
        assert_eq!((c.name.as_str(), c.volume, c.pan), ("Crushed", -60.0, 0.5));

        // A macro takes the knob's place, then drives it.
        let map = |inner: Option<String>, index: Option<u8>| Op::MapMacro { track: master.clone(), device: rack.clone(), chain: wet.clone(), inner, param: "thresh".into(), index, min: None, max: None };
        assert!(apply(&mut project, &mut history, map(None, Some(2))).is_err(), "which effect?");
        apply(&mut project, &mut history, map(Some(comp.clone()), Some(2))).unwrap();
        let r = &project.master_devices[0];
        let amount = r.param("macro3").unwrap();
        assert!((r.fx_chain_param(&wet, &r.fx_chains[1].devices[0], "thresh").unwrap() + 30.0).abs() < 1e-6, "nothing jumps: {amount}");
        apply(&mut project, &mut history, Op::SetParam { track: master.clone(), device: rack.clone(), param: "macro3".into(), value: 100.0 }).unwrap();
        let r = &project.master_devices[0];
        let top = r.fx_chains[1].devices[0].params.iter().find(|p| p.id == "thresh").unwrap().max;
        assert_eq!(r.fx_chain_param(&wet, &r.fx_chains[1].devices[0], "thresh"), Some(top));
        let json = serde_json::to_string(&project).unwrap();
        assert_eq!(serde_json::from_str::<Project>(&json).unwrap(), project);

        // A copy of the rack (a preset, a duplicate) gets ids of its own and keeps its mapping.
        let copy = project.master_devices[0].clone();
        apply(&mut project, &mut history, Op::AddDevice { track: master.clone(), device: copy }).unwrap();
        let (first, second) = (&project.master_devices[0], &project.master_devices[1]);
        assert!(second.fx_chains[1].id != first.fx_chains[1].id && second.fx_chains[1].devices[0].id != comp);
        assert_eq!(second.macros.len(), 1);
        assert_eq!(second.macros[0].chain, second.fx_chains[1].id);
        assert_eq!(second.macros[0].device.as_deref(), Some(second.fx_chains[1].devices[0].id.as_str()));
        let second = second.id.clone();
        apply(&mut project, &mut history, Op::RemoveDevice { track: master.clone(), device: second }).unwrap();

        // Taking the effect away drops its mapping; taking the chain away takes its effects along, and the response's file.
        apply(&mut project, &mut history, Op::RemoveDevice { track: inside.clone(), device: comp }).unwrap();
        assert!(project.master_devices[0].macros.is_empty());
        apply(&mut project, &mut history, Op::RemoveChain { track: master.clone(), device: rack.clone(), chain: wet }).unwrap();
        assert_eq!(project.master_devices[0].fx_chains.len(), 1);
        assert!(project.asset("a1").is_none());
        apply(&mut project, &mut history, Op::RemoveChain { track: master, device: rack, chain: dry }).unwrap();
        assert!(project.master_devices[0].fx_chains.is_empty());
    }

    #[test]
    fn the_wavetable_and_dry_wet_are_in_the_catalog() {
        let wavetable = crate::catalog::device("wavetable", &[]).unwrap();
        assert_eq!(wavetable.role, DeviceRole::Instrument);
        // Eight matrix slots of a source, a destination and an amount.
        assert_eq!(wavetable.params.iter().filter(|p| p.group.as_deref() == Some("Matrix")).count(), 24);
        assert_eq!(wavetable.params.iter().find(|p| p.id == "m1dst").unwrap().options.as_ref().unwrap()[4], "Cutoff");
        // Every audio effect has a Dry/Wet: its own mix, or the one the engine puts around it.
        for spec in crate::catalog::specs().iter().filter(|s| s.category == "audio-fx" && s.kind != "denoise") {
            assert!(spec.params.iter().any(|p| p.id == "mix" || p.id == "wet"), "{}", spec.kind);
        }
        // A device saved before it had one gets it, fully wet.
        let mut gate = crate::catalog::device("gate", &[]).unwrap();
        gate.params.retain(|p| p.id != "wet");
        crate::catalog::refresh(&mut gate);
        assert_eq!(gate.param("wet"), Some(100.0));
    }

    #[test]
    fn controllers_map_to_one_target_each_and_turn_it() {
        let mut project = Project::empty("Song");
        let mut history = History::new();
        let midi = project.tracks[2].id.clone();
        let synth = project.tracks[2].devices[0].id.clone();
        let cutoff = MappingTarget::Param { track: midi.clone(), device: synth.clone(), param: "cutoff".into() };
        let volume = MappingTarget::Volume { track: midi.clone() };
        apply(&mut project, &mut history, Op::AddMapping { channel: None, cc: 21, target: cutoff.clone() }).unwrap();
        apply(&mut project, &mut history, Op::AddMapping { channel: None, cc: 22, target: cutoff.clone() }).unwrap();
        apply(&mut project, &mut history, Op::AddMapping { channel: None, cc: 22, target: volume.clone() }).unwrap();
        assert_eq!(project.mappings.len(), 1, "CC 22 moved to the volume and cutoff lost its controls");
        apply(&mut project, &mut history, Op::AddMapping { channel: Some(0), cc: 21, target: cutoff.clone() }).unwrap();
        let ghost = MappingTarget::Param { track: midi.clone(), device: synth.clone(), param: "nope".into() };
        assert!(apply(&mut project, &mut history, Op::AddMapping { channel: None, cc: 23, target: ghost }).is_err());

        // A log parameter sweeps by octave: half way is the geometric middle.
        let op = controller_op(&project, &cutoff, 127).unwrap();
        apply(&mut project, &mut history, op).unwrap();
        assert_eq!(project.tracks[2].devices[0].param("cutoff"), Some(18_000.0));
        let Some(Op::SetTrack { patch, .. }) = controller_op(&project, &volume, 127) else { panic!("a track change") };
        assert_eq!(patch.volume, Some(6.0));
        let Some(Op::SetTrack { patch, .. }) = controller_op(&project, &MappingTarget::Pan { track: midi.clone() }, 64) else { panic!() };
        assert_eq!(patch.pan, Some(0.0));

        // Mappings go with what they point at.
        apply(&mut project, &mut history, Op::RemoveDevice { track: midi.clone(), device: synth }).unwrap();
        assert_eq!(project.mappings.len(), 1);
        apply(&mut project, &mut history, Op::RemoveTrack { id: midi }).unwrap();
        assert!(project.mappings.is_empty());
    }

    #[test]
    fn a_frozen_track_keeps_its_audio_until_unfrozen_and_flattens_into_it() {
        let mut project = Project::empty("Song");
        let mut history = History::new();
        let midi = project.tracks[2].id.clone();
        let synth = crate::catalog::device("synth", &[]).unwrap();
        apply(&mut project, &mut history, Op::AddDevice { track: midi.clone(), device: synth }).unwrap();
        let synth_id = project.tracks[2].devices[0].id.clone();
        project.tracks[2].clips.push(Clip::midi("Riff", 0.0, 4.0, Vec::new()));
        let cutoff = MappingTarget::Param { track: midi.clone(), device: synth_id, param: "cutoff".into() };
        apply(&mut project, &mut history, Op::AddMapping { channel: None, cc: 21, target: cutoff }).unwrap();
        let frozen = crate::Asset { id: "f1".into(), name: "MIDI 1 (frozen)".into(), channels: 2, sample_rate: 1000, frames: 3000, path: None, missing: false };

        apply(&mut project, &mut history, Op::FreezeTrack { track: midi.clone(), asset: frozen.clone() }).unwrap();
        assert_eq!(project.tracks[2].frozen.as_deref(), Some("f1"));
        project.prune_assets();
        assert_eq!(project.assets.len(), 1, "a frozen track's audio is kept");
        history.undo(&mut project);
        assert!(project.tracks[2].frozen.is_none() && project.assets.is_empty(), "undo takes its audio away");
        history.redo(&mut project);

        // Unfreezing lets the audio go; undo brings both back.
        apply(&mut project, &mut history, Op::UnfreezeTrack { track: midi.clone() }).unwrap();
        assert!(project.tracks[2].frozen.is_none() && project.assets.is_empty());
        history.undo(&mut project);
        assert_eq!(project.tracks[2].frozen.as_deref(), Some("f1"));
        assert_eq!(project.assets.len(), 1);

        apply(&mut project, &mut history, Op::FlattenTrack { track: midi.clone() }).unwrap();
        let t = &project.tracks[2];
        assert_eq!(t.kind, TrackKind::Audio);
        assert!(t.devices.is_empty() && t.frozen.is_none());
        assert_eq!(t.clips.len(), 1);
        assert_eq!(t.clips[0].asset(), Some("f1"));
        assert!(matches!(t.clips[0].content, ClipContent::Audio { duration, .. } if duration == 3.0));
        assert!(project.mappings.is_empty(), "mappings to the synth went with it");
        assert_eq!(project.assets, [frozen]);

        // Only a frozen track flattens.
        assert!(apply(&mut project, &mut history, Op::FlattenTrack { track: midi }).is_err());
    }

    #[test]
    fn mixes_come_back_as_saved_and_a_b_swaps_two_of_them() {
        let mut project = Project::empty("Song");
        let mut history = History::new();
        let (first, second) = (project.tracks[0].id.clone(), project.tracks[1].id.clone());
        let set = |project: &mut Project, history: &mut History, id: &str, json: &str| {
            apply(project, history, op(&format!(r#"{{"op":"setTrack","id":"{id}","patch":{json}}}"#))).unwrap();
        };
        set(&mut project, &mut history, &first, r#"{"volume":-6}"#);
        set(&mut project, &mut history, &first, r#"{"pan":-0.5}"#);
        let saved = apply(&mut project, &mut history, Op::SaveMix { name: None }).unwrap().created.unwrap();
        assert_eq!(project.mixes[0].name, "Mix 1");
        set(&mut project, &mut history, &first, r#"{"volume":-20}"#);
        set(&mut project, &mut history, &first, r#"{"mute":true}"#);
        set(&mut project, &mut history, &second, r#"{"solo":true}"#);
        project.master_volume = -3.0;
        apply(&mut project, &mut history, Op::RecallMix { id: saved.clone() }).unwrap();
        let t = &project.tracks[0];
        assert_eq!((t.volume, t.pan, t.mute), (-6.0, -0.5, false));
        assert!(!project.tracks[1].solo);
        assert_eq!(project.master_volume, 0.0);

        // A/B: B starts as A; changes go to B; swapping plays A and keeps B.
        apply(&mut project, &mut history, Op::SwapMix).unwrap();
        assert_eq!(project.other_mix.as_ref().map(|m| m.name.as_str()), Some("A"));
        set(&mut project, &mut history, &first, r#"{"volume":-12}"#);
        apply(&mut project, &mut history, Op::SwapMix).unwrap();
        assert_eq!(project.tracks[0].volume, -6.0, "A plays");
        assert_eq!(project.other_mix.as_ref().map(|m| (m.name.as_str(), m.tracks[0].volume)), Some(("B", -12.0)));
        apply(&mut project, &mut history, Op::SwapMix).unwrap();
        assert_eq!(project.tracks[0].volume, -12.0, "B again");
        apply(&mut project, &mut history, Op::KeepMix).unwrap();
        assert!(project.other_mix.is_none());
        assert_eq!(project.tracks[0].volume, -12.0);

        // A mix only touches what it knows: a track added since keeps its fader.
        apply(&mut project, &mut history, op(r#"{"op":"addTrack","kind":"audio"}"#)).unwrap();
        let added = project.tracks.last().unwrap().id.clone();
        set(&mut project, &mut history, &added, r#"{"volume":-9}"#);
        apply(&mut project, &mut history, Op::RecallMix { id: saved.clone() }).unwrap();
        assert_eq!(project.tracks.last().unwrap().volume, -9.0);

        apply(&mut project, &mut history, Op::RenameMix { id: saved.clone(), name: " Rough ".into() }).unwrap();
        assert_eq!(project.mixes[0].name, "Rough");
        apply(&mut project, &mut history, Op::RemoveMix { id: saved }).unwrap();
        assert!(project.mixes.is_empty());
        let json = serde_json::to_string(&project).unwrap();
        assert_eq!(serde_json::from_str::<Project>(&json).unwrap(), project);
    }

    #[test]
    fn project_round_trips_through_json() {
        let project = Project::empty("Song");
        let json = serde_json::to_string(&project).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(project, back);
    }
}
