use serde::{Deserialize, Serialize};

use crate::catalog;
use crate::meter::{MeterChange, MeterMap};
use crate::tempo::{TempoMap, TempoPoint};

/// Bumped whenever the file format changes in a way that needs a migration.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub version: u32,
    pub id: String,
    pub name: String,
    pub bpm: f64,
    pub beats_per_bar: u32,
    pub key: String,
    pub tracks: Vec<Track>,
    pub returns: Vec<ReturnTrack>,
    pub scenes: Vec<String>,
    pub master_volume: f64,
    /// Audio files and renders the clips refer to. Sample data lives in the pool.
    pub assets: Vec<Asset>,
    /// Markers and regions on the timeline (Audacity's labels).
    #[serde(default)]
    pub markers: Vec<Marker>,
    /// MIDI controller knobs, faders and buttons and what they move.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mappings: Vec<ControllerMapping>,
    /// Effects on the master, before its fader.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub master_devices: Vec<Device>,
    /// Session clips start on the next multiple of this many beats (0: at once).
    #[serde(default = "default_quantize")]
    pub launch_quantize: f64,
    /// The tempo track; empty, the song plays at `bpm` throughout.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tempo: Vec<TempoPoint>,
    /// Time-signature changes along the song (`beats_per_bar` covers its start).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub meters: Vec<MeterChange>,
    /// Saved mixes to go back to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mixes: Vec<Mix>,
    /// While comparing two mixes (A/B): the one not playing, named "A" or "B".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_mix: Option<Mix>,
}

fn default_quantize() -> f64 {
    4.0
}

/// The mixer as it was: every track's and return's fader, pan and mute,
/// the tracks' solo and sends, and the master fader.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mix {
    pub id: String,
    pub name: String,
    pub tracks: Vec<TrackMix>,
    pub returns: Vec<ReturnMix>,
    pub master_volume: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackMix {
    pub track: String,
    pub volume: f64,
    pub pan: f64,
    pub mute: bool,
    pub solo: bool,
    pub sends: [f64; 2],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReturnMix {
    #[serde(rename = "return")]
    pub id: String,
    pub volume: f64,
    pub pan: f64,
    pub mute: bool,
}

impl Project {
    /// Beats ↔ seconds, with the tempo track.
    pub fn tempo_map(&self) -> TempoMap {
        TempoMap::new(self.bpm, &self.tempo)
    }

    /// Where the bar lines fall.
    pub fn meter_map(&self) -> MeterMap {
        MeterMap::new(self.beats_per_bar, &self.meters)
    }
}

/// A parameter of a device on the same track: what a modulator moves.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamRef {
    pub device: String,
    pub param: String,
}

/// Device types that move another device's parameter instead of sound.
pub const MODULATOR_TYPES: [&str; 2] = ["lfo", "follower"];

/// The id that names the master's chain in device ops.
pub const MASTER: &str = "master";

/// What holds a device chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainOwner {
    MidiTrack,
    AudioTrack,
    GroupTrack,
    Return,
    Master,
    /// A Drum Rack pad's effects, or one of the rack's returns.
    Pad,
    /// A chain of an audio effect rack.
    Rack,
}

/// The device type of an audio effect rack.
pub const FX_RACK: &str = "fx-rack";

/// A chain inside a device, addressed as `owner/device/key`. In a Drum Rack
/// the key is a pad's note ("36"–"99") or "A" / "B" for the rack's two
/// returns; in an audio effect rack it is the chain's id.
pub fn nested_path(owner: &str) -> Option<(&str, &str, &str)> {
    let mut parts = owner.splitn(3, '/');
    Some((parts.next()?, parts.next()?, parts.next()?))
}

fn pad_key(key: &str) -> bool {
    key == "A" || key == "B" || key.parse::<u8>().is_ok_and(|n| (36..=99).contains(&n))
}

/// A device in a chain, or the instrument of one of its instrument racks.
pub fn find_device_mut<'a>(devices: &'a mut [Device], id: &str) -> Option<&'a mut Device> {
    if let Some(i) = devices.iter().position(|d| d.id == id) {
        return Some(&mut devices[i]);
    }
    devices.iter_mut().flat_map(|d| d.chains.iter_mut()).map(|c| &mut c.device).find(|d| d.id == id)
}

/// A MIDI controller's knob, fader or button (a CC number) moving something in the song.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControllerMapping {
    pub id: String,
    /// 0–15; `None` listens on every channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<u8>,
    pub cc: u8,
    pub target: MappingTarget,
}

/// What a controller moves.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MappingTarget {
    /// A device parameter: any device on the track, a rack's macros, or a rack chain's instrument.
    Param { track: String, device: String, param: String },
    Volume { track: String },
    Pan { track: String },
}

impl MappingTarget {
    pub fn track(&self) -> &str {
        match self {
            Self::Param { track, .. } | Self::Volume { track } | Self::Pan { track } => track,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Marker {
    pub id: String,
    pub name: String,
    /// Position in beats.
    pub start: f64,
    /// A region ends here (beats); a point marker has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
}

/// When an audio track plays its input through the speakers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Monitor {
    /// Never: the safe default with a laptop microphone and speakers.
    #[default]
    Off,
    /// While the track is armed.
    Auto,
    /// Always.
    In,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub id: String,
    pub name: String,
    pub channels: u16,
    pub sample_rate: u32,
    pub frames: u64,
    /// Where the audio lives on disk: absolute in memory, relative to the
    /// project file when saved. `None` for audio that only exists in memory
    /// (renders and edits) until the project is saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Set on load when the file could not be found or decoded.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub missing: bool,
}

impl Asset {
    pub fn duration(&self) -> f64 {
        self.frames as f64 / f64::from(self.sample_rate.max(1))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackKind {
    Audio,
    Midi,
    /// Sums the tracks grouped into it, through its own effects and fader.
    Group,
}

/// What a track is in a podcast: someone speaking, or music that dips under them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackRole {
    #[default]
    None,
    Speaker,
    Music,
}

impl TrackRole {
    pub fn is_none(&self) -> bool {
        *self == TrackRole::None
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub name: String,
    pub kind: TrackKind,
    /// 1–8, the --track-N colour tokens.
    pub color: u8,
    /// Timeline clips, and session clips waiting in a scene's slot ([`Clip::slot`]).
    pub clips: Vec<Clip>,
    /// Fader in dB.
    pub volume: f64,
    /// −1 (left) … 1 (right).
    pub pan: f64,
    pub mute: bool,
    pub solo: bool,
    pub arm: bool,
    pub devices: Vec<Device>,
    /// Send levels to return A and B, 0…1.
    pub sends: [f64; 2],
    /// Older files named a timeline clip per scene here; on load each becomes
    /// a session clip of its own ([`Project::migrate_slots`]).
    #[serde(default, skip_serializing)]
    pub slots: Vec<Option<String>>,
    /// Audio tracks: which input channels they record. "1", "2", … for a
    /// mono channel, "1/2", "3/4", … for a stereo pair.
    #[serde(default = "default_input")]
    pub input: String,
    #[serde(default)]
    pub monitor: Monitor,
    /// What moves over time on this track: its mixer and its devices' parameters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub automation: Vec<AutomationLane>,
    /// The group track this one plays into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Frozen: the asset holding what its clips and devices sound like (from
    /// the start of the song), played instead while the devices rest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen: Option<String>,
    /// Recorded passes kept beside the timeline, to comp from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub takes: Vec<TakeLane>,
    /// Podcast mode: a speaker (levelled) or music (ducked under the speakers).
    #[serde(default, skip_serializing_if = "TrackRole::is_none")]
    pub role: TrackRole,
}

/// One pass of a loop recording, kept under its track. Its clips never play
/// themselves: comping copies stretches of them onto the track's timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TakeLane {
    pub id: String,
    pub name: String,
    pub clips: Vec<Clip>,
}

/// A breakpoint of a clip's gain envelope.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EnvPoint {
    /// Beats from the clip's start.
    pub time: f64,
    /// dB; −60 and below is silence.
    pub gain: f64,
}

fn is_false(v: &bool) -> bool {
    !*v
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// What an automation lane (or a modulator) moves.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AutoTarget {
    Volume,
    Pan,
    /// Send to return `index` (0 for A).
    Send { index: u8 },
    /// A parameter of a device on the track.
    Param { device: String, param: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoPoint {
    /// Beats from the song's start.
    pub time: f64,
    pub value: f64,
}

/// Breakpoints joined by straight lines; before the first and after the
/// last the value holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationLane {
    pub target: AutoTarget,
    pub points: Vec<AutoPoint>,
}

impl AutomationLane {
    /// The value at `time` (beats).
    pub fn value_at(&self, time: f64) -> Option<f64> {
        let first = self.points.first()?;
        let after = self.points.partition_point(|p| p.time <= time);
        if after == 0 {
            return Some(first.value);
        }
        let a = self.points[after - 1];
        Some(match self.points.get(after) {
            Some(b) if b.time > a.time => a.value + (b.value - a.value) * (time - a.time) / (b.time - a.time),
            _ => a.value,
        })
    }
}

/// What a session clip does after it has played for a while.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Follow {
    Stop,
    /// Plays itself again from the start.
    Again,
    Next,
    Previous,
    First,
    Last,
    /// Any clip on the track, this one included.
    Any,
    /// Any other clip on the track.
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowAction {
    pub action: Follow,
    /// After this many bars of playing.
    pub bars: f64,
}

fn default_input() -> String {
    "1".into()
}

impl Track {
    /// Whether the input plays through this track right now.
    pub fn monitoring(&self) -> bool {
        self.kind == TrackKind::Audio && (self.monitor == Monitor::In || self.monitor == Monitor::Auto && self.arm)
    }

    /// The input channels ("1", "1/2", …) as zero-based (left, right) indexes.
    pub fn input_channels(&self) -> (usize, usize) {
        let mut picked = self.input.split('/').filter_map(|s| s.trim().parse::<usize>().ok()).map(|n| n.saturating_sub(1));
        let left = picked.next().unwrap_or(0);
        (left, picked.next().unwrap_or(left))
    }

    /// A device on the track or inside one of its instrument racks.
    pub fn device_mut(&mut self, id: &str) -> Option<&mut Device> {
        find_device_mut(&mut self.devices, id)
    }
}

impl Track {
    pub fn new(id: String, name: impl Into<String>, kind: TrackKind, color: u8) -> Self {
        let devices = match kind {
            TrackKind::Midi => catalog::device("synth", &[]).into_iter().collect(),
            TrackKind::Audio | TrackKind::Group => Vec::new(),
        };
        Self {
            id,
            name: name.into(),
            kind,
            color: color.clamp(1, 8),
            clips: Vec::new(),
            volume: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
            arm: false,
            devices,
            sends: [0.0, 0.0],
            slots: Vec::new(),
            input: default_input(),
            monitor: Monitor::Off,
            automation: Vec::new(),
            group: None,
            frozen: None,
            takes: Vec::new(),
            role: TrackRole::None,
        }
    }

    /// The clips on the timeline (not those waiting in session slots).
    pub fn timeline(&self) -> impl Iterator<Item = &Clip> {
        self.clips.iter().filter(|c| c.slot.is_none())
    }

    /// The session clip waiting in a scene's slot.
    pub fn slot(&self, scene: usize) -> Option<&Clip> {
        self.clips.iter().find(|c| c.slot == Some(scene))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: String,
    pub name: String,
    /// Position on the timeline in beats (0 for session clips).
    pub start: f64,
    #[serde(flatten)]
    pub content: ClipContent,
    /// Session view: the scene whose slot this clip waits in; `None` on the timeline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<usize>,
    /// Session view: what happens after the clip has played a while.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow: Option<FollowAction>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", rename_all_fields = "camelCase")]
pub enum ClipContent {
    /// Plays a stretch of an asset: at its own speed, or warped to the song's tempo.
    Audio {
        asset: String,
        /// Seconds into the asset where the clip starts.
        offset: f64,
        /// Seconds of audio the clip plays.
        duration: f64,
        /// Clip gain in dB.
        #[serde(default)]
        gain: f64,
        /// Linear fade lengths in seconds.
        #[serde(default)]
        fade_in: f64,
        #[serde(default)]
        fade_out: f64,
        /// Follows the song's tempo: its beats stay on the song's beats.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        warp: Option<Warp>,
        /// The shape of the fades and crossfades; `None` is the standard:
        /// straight fades, equal-power crossfades.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        curve: Option<FadeCurve>,
        /// Plays its stretch of the audio backwards.
        #[serde(default, skip_serializing_if = "is_false")]
        reverse: bool,
        /// Pitch change in semitones; the length stays.
        #[serde(default, skip_serializing_if = "is_zero")]
        transpose: f64,
        /// Clip gain over time: breakpoints in beats from the clip's start,
        /// joined by straight lines (in dB), holding before and after.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        envelope: Vec<EnvPoint>,
    },
    Midi {
        /// Length in beats.
        length: f64,
        notes: Vec<Note>,
        /// Clips with the same link share their notes (FL-style patterns):
        /// editing one changes them all.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        link: Option<String>,
    },
}

/// The shape of a clip's fades and crossfades.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FadeCurve {
    /// A straight line: quick at the quiet end (a crossfade dips in the middle).
    Linear,
    /// Constant loudness through a crossfade.
    EqualPower,
    /// Slow at both ends, quick in the middle.
    SCurve,
}

/// How a warped clip keeps time with the song.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Warp {
    /// The audio's own tempo: at this song tempo it plays at its original speed.
    pub bpm: f64,
    pub mode: WarpMode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WarpMode {
    /// Each hit plays at its own speed, moved to its place on the beat (drums, loops).
    #[default]
    Beats,
    /// Stretched in short overlapping grains, pitch kept (melodies, chords, voices).
    Tones,
    /// Played faster or slower like a record: pitch follows the tempo.
    Repitch,
}

impl Clip {
    /// Length on the timeline in beats. An audio clip's depends on where it
    /// sits when it plays at its own speed through tempo changes.
    pub fn length_beats(&self, tempo: &TempoMap) -> f64 {
        match &self.content {
            ClipContent::Audio { duration, warp: Some(w), .. } => duration * w.bpm / 60.0,
            ClipContent::Audio { duration, .. } => tempo.beats_at(tempo.seconds_at(self.start) + duration) - self.start,
            ClipContent::Midi { length, .. } => *length,
        }
    }

    /// Seconds of the clip's audio between two timeline beats (warped
    /// clips count in their own tempo).
    pub fn audio_seconds(&self, tempo: &TempoMap, from: f64, to: f64) -> f64 {
        match &self.content {
            ClipContent::Audio { warp: Some(w), .. } => (to - from) * 60.0 / w.bpm,
            _ => tempo.seconds_at(to) - tempo.seconds_at(from),
        }
    }

    /// The timeline beat `seconds` of the clip's audio after beat `from`.
    pub fn beat_after(&self, tempo: &TempoMap, from: f64, seconds: f64) -> f64 {
        match &self.content {
            ClipContent::Audio { warp: Some(w), .. } => from + seconds * w.bpm / 60.0,
            _ => tempo.beats_at(tempo.seconds_at(from) + seconds),
        }
    }

    pub fn asset(&self) -> Option<&str> {
        match &self.content {
            ClipContent::Audio { asset, .. } => Some(asset),
            ClipContent::Midi { .. } => None,
        }
    }

    pub fn end(&self, tempo: &TempoMap) -> f64 {
        self.start + self.length_beats(tempo)
    }

    /// A new audio clip playing `duration` seconds of `asset` from its start.
    pub fn audio(name: impl Into<String>, asset: impl Into<String>, start: f64, offset: f64, duration: f64) -> Self {
        Self {
            id: crate::new_id("c"),
            name: name.into(),
            start,
            content: ClipContent::Audio {
                asset: asset.into(),
                offset,
                duration,
                gain: 0.0,
                fade_in: 0.0,
                fade_out: 0.0,
                warp: None,
                curve: None,
                reverse: false,
                transpose: 0.0,
                envelope: Vec::new(),
            },
            slot: None,
            follow: None,
        }
    }

    /// A new MIDI clip `length` beats long.
    pub fn midi(name: impl Into<String>, start: f64, length: f64, mut notes: Vec<Note>) -> Self {
        tidy_notes(&mut notes);
        Self { id: crate::new_id("c"), name: name.into(), start, content: ClipContent::Midi { length, notes, link: None }, slot: None, follow: None }
    }

    /// A copy waiting in a scene's slot, starting from its own beginning.
    pub fn in_slot(&self, scene: usize) -> Self {
        let mut clip = self.clone();
        clip.id = crate::new_id("c");
        clip.start = 0.0;
        clip.slot = Some(scene);
        if let ClipContent::Midi { link, .. } = &mut clip.content {
            *link = None;
        }
        clip
    }

    /// The link a MIDI clip shares its notes through, if any.
    pub fn link(&self) -> Option<&str> {
        match &self.content {
            ClipContent::Midi { link, .. } => link.as_deref(),
            ClipContent::Audio { .. } => None,
        }
    }
}

/// Shortest note the editing ops keep, in beats.
pub const MIN_NOTE: f64 = 1.0 / 128.0;

/// Clamps notes into range and sorts them by start, then pitch: the order
/// the piano roll's selection relies on.
pub fn tidy_notes(notes: &mut Vec<Note>) {
    notes.retain(|n| n.start.is_finite() && n.length.is_finite() && n.velocity.is_finite());
    for n in notes.iter_mut() {
        n.start = n.start.max(0.0);
        n.length = n.length.max(MIN_NOTE);
        n.pitch = n.pitch.min(127);
        n.velocity = n.velocity.clamp(0.01, 1.0);
    }
    notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    /// Beats from the clip start.
    pub start: f64,
    pub length: f64,
    pub pitch: u8,
    pub velocity: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceRole {
    Instrument,
    Effect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    pub role: DeviceRole,
    pub on: bool,
    pub params: Vec<DeviceParam>,
    /// Audio a sampler or drum rack plays.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub samples: Vec<SampleRef>,
    /// An instrument rack's chains: instruments layered and split by key and velocity.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chains: Vec<Chain>,
    /// What an instrument rack's macros move.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub macros: Vec<MacroMap>,
    /// The track a compressor or gate listens to instead of its own input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidechain: Option<String>,
    /// What a modulator (LFO, envelope follower) moves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modulates: Option<ParamRef>,
    /// The plugin a `plugin` device hosts. Its parameters are the plugin's
    /// controls the device panel shows (and automation can reach).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<PluginRef>,
    /// The plugin's own state (base64) as of the last save.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// A Drum Rack's pad effect chains and its two returns.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pad_chains: Vec<PadChain>,
    /// An audio effect rack's chains: effects side by side.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fx_chains: Vec<FxChain>,
}

/// One chain of an audio effect rack: it hears the rack's input, runs it
/// through its effects in order, and joins the other chains at its level and pan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FxChain {
    pub id: String,
    pub name: String,
    pub on: bool,
    /// Level in dB.
    pub volume: f64,
    /// −1 (left) … 1 (right).
    pub pan: f64,
    pub devices: Vec<Device>,
}

impl FxChain {
    pub fn new(name: String) -> Self {
        Self { id: crate::new_id("ch"), name, on: true, volume: 0.0, pan: 0.0, devices: Vec::new() }
    }
}

/// A Drum Rack pad's own audio effects, or one of the rack's two returns
/// (which every pad's Send A and Send B feed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PadChain {
    /// The pad's note ("36"–"99"), or "A" / "B" for a return.
    pub key: String,
    pub devices: Vec<Device>,
}

/// The device type of a hosted plugin.
pub const PLUGIN_TYPE: &str = "plugin";

/// A plugin a device hosts: the file, which of its plugins, and who made it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRef {
    /// The plugin format ("clap").
    pub format: String,
    pub path: String,
    pub id: String,
    #[serde(default)]
    pub vendor: String,
}

/// Device types that can listen to another track.
pub const SIDECHAIN_TYPES: [&str; 3] = ["comp", "gate", "vocoder"];

/// How many macros an instrument rack has (parameters `macro1` … `macro8`).
pub const MACROS: u8 = 8;

impl Device {
    pub fn param(&self, id: &str) -> Option<f64> {
        self.params.iter().find(|p| p.id == id).map(|p| p.value)
    }

    /// This device and the instruments in its chains.
    pub fn with_inner(&self) -> impl Iterator<Item = &Device> {
        std::iter::once(self).chain(self.chains.iter().map(|c| &c.device))
    }

    /// This device and every device inside it: rack instruments, pad effects, effect rack chains.
    pub fn with_nested(&self) -> impl Iterator<Item = &Device> {
        let pads = self.pad_chains.iter().flat_map(|c| c.devices.iter());
        let racked = self.fx_chains.iter().flat_map(|c| c.devices.iter());
        self.with_inner().chain(pads).chain(racked)
    }

    /// A parameter of an effect in one of an effect rack's chains as the rack
    /// plays it: a macro mapped to it overrides its own value.
    pub fn fx_chain_param(&self, chain: &str, device: &Device, id: &str) -> Option<f64> {
        let p = device.params.iter().find(|p| p.id == id)?;
        Some(match self.macros.iter().find(|m| m.chain == chain && m.device.as_deref() == Some(device.id.as_str()) && m.param == id) {
            Some(m) => m.value(p, self.param(&format!("macro{}", m.index + 1)).unwrap_or(0.0) / 100.0),
            None => p.value,
        })
    }

    /// A chain instrument's parameter as the rack plays it: a macro mapped
    /// to it overrides its own value.
    pub fn chain_param(&self, chain: &Chain, id: &str) -> Option<f64> {
        let p = chain.device.params.iter().find(|p| p.id == id)?;
        Some(match self.macros.iter().find(|m| m.chain == chain.id && m.param == id) {
            Some(m) => m.value(p, self.param(&format!("macro{}", m.index + 1)).unwrap_or(0.0) / 100.0),
            None => p.value,
        })
    }
}

/// One chain of an instrument rack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chain {
    pub id: String,
    pub name: String,
    pub on: bool,
    /// The chain's instrument.
    pub device: Device,
    /// The notes it plays, inclusive.
    pub key_low: u8,
    pub key_high: u8,
    /// The velocities (1–127) it plays, inclusive.
    pub vel_low: u8,
    pub vel_high: u8,
    /// Level in dB.
    pub volume: f64,
    /// −1 (left) … 1 (right).
    pub pan: f64,
}

impl Chain {
    pub fn new(device: Device) -> Self {
        Self {
            id: crate::new_id("ch"),
            name: device.name.clone(),
            on: true,
            device,
            key_low: 0,
            key_high: 127,
            vel_low: 1,
            vel_high: 127,
            volume: 0.0,
            pan: 0.0,
        }
    }
}

/// A macro of an instrument rack turning one of its chains' parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroMap {
    /// 0 … MACROS − 1.
    pub index: u8,
    pub chain: String,
    /// In an audio effect rack: the effect in the chain whose parameter it turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    pub param: String,
    /// The parameter's value with the macro at 0 % and at 100 %.
    pub min: f64,
    pub max: f64,
}

impl MacroMap {
    /// The parameter's value with the macro at `amount` (0–1): along the
    /// parameter's own curve (log parameters sweep evenly by octave), on its steps.
    pub fn value(&self, p: &DeviceParam, amount: f64) -> f64 {
        let t = amount.clamp(0.0, 1.0);
        let v = if p.log == Some(true) && self.min > 0.0 && self.max > 0.0 {
            self.min * (self.max / self.min).powf(t)
        } else {
            self.min + (self.max - self.min) * t
        };
        let v = match p.step.filter(|s| *s > 0.0) {
            Some(step) => p.min + ((v - p.min) / step).round() * step,
            None => v,
        };
        v.clamp(p.min, p.max)
    }

    /// Where the macro (0–1) has to be for the parameter to sit at `value`.
    pub fn amount(&self, p: &DeviceParam, value: f64) -> f64 {
        let t = if p.log == Some(true) && self.min > 0.0 && self.max > 0.0 && value > 0.0 && self.max != self.min {
            (value / self.min).ln() / (self.max / self.min).ln()
        } else if self.max != self.min {
            (value - self.min) / (self.max - self.min)
        } else {
            0.0
        };
        t.clamp(0.0, 1.0)
    }
}

/// A sample loaded into a device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleRef {
    /// The drum pad's note; 0 for a sampler's one sample.
    pub key: u8,
    pub asset: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceParam {
    pub id: String,
    pub label: String,
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub unit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bipolar: Option<bool>,
    /// Values snap to multiples of this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    /// Named choices; the value is the index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    /// Section of the device panel, or `pad:N` for a drum pad's parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReturnTrack {
    pub id: String,
    pub name: String,
    pub volume: f64,
    pub pan: f64,
    pub mute: bool,
    pub devices: Vec<Device>,
    /// Tracks send to it before their fader (it hears them whatever the fader does).
    #[serde(default)]
    pub pre_fader: bool,
}

pub const DEFAULT_SCENES: [&str; 5] = ["Scene 1", "Scene 2", "Scene 3", "Scene 4", "Scene 5"];

impl Project {
    /// The mixer as it is now, as a mix called `name`.
    pub fn capture_mix(&self, id: String, name: String) -> Mix {
        Mix {
            id,
            name,
            tracks: self
                .tracks
                .iter()
                .map(|t| TrackMix { track: t.id.clone(), volume: t.volume, pan: t.pan, mute: t.mute, solo: t.solo, sends: t.sends })
                .collect(),
            returns: self.returns.iter().map(|r| ReturnMix { id: r.id.clone(), volume: r.volume, pan: r.pan, mute: r.mute }).collect(),
            master_volume: self.master_volume,
        }
    }

    /// Sets the mixer as `mix` has it. Tracks added since keep their own
    /// settings; tracks it knows that are gone are passed over.
    pub fn apply_mix(&mut self, mix: &Mix) {
        for m in &mix.tracks {
            if let Some(t) = self.tracks.iter_mut().find(|t| t.id == m.track) {
                (t.volume, t.pan, t.mute, t.solo, t.sends) = (m.volume, m.pan, m.mute, m.solo, m.sends);
            }
        }
        for m in &mix.returns {
            if let Some(r) = self.returns.iter_mut().find(|r| r.id == m.id) {
                (r.volume, r.pan, r.mute) = (m.volume, m.pan, m.mute);
            }
        }
        self.master_volume = mix.master_volume;
    }

    /// An empty song: two audio and two MIDI tracks, like most DAWs start with.
    pub fn empty(name: impl Into<String>) -> Self {
        let tracks = vec![
            Track::new(crate::new_id("t"), "Audio 1", TrackKind::Audio, 1),
            Track::new(crate::new_id("t"), "Audio 2", TrackKind::Audio, 5),
            Track::new(crate::new_id("t"), "MIDI 1", TrackKind::Midi, 6),
            Track::new(crate::new_id("t"), "MIDI 2", TrackKind::Midi, 2),
        ];
        Self {
            version: SCHEMA_VERSION,
            id: crate::new_id("p"),
            name: name.into(),
            bpm: 120.0,
            beats_per_bar: 4,
            key: "C major".into(),
            tracks,
            returns: default_returns(),
            scenes: DEFAULT_SCENES.iter().map(|s| s.to_string()).collect(),
            master_volume: 0.0,
            assets: Vec::new(),
            markers: Vec::new(),
            mappings: Vec::new(),
            master_devices: Vec::new(),
            launch_quantize: default_quantize(),
            tempo: Vec::new(),
            mixes: Vec::new(),
            other_mix: None,
            meters: Vec::new(),
        }
    }

    /// Older files named a timeline clip in each session slot; each becomes a
    /// session clip of its own (a copy that starts from its beginning).
    pub fn migrate_slots(&mut self) {
        for track in &mut self.tracks {
            for (scene, name) in std::mem::take(&mut track.slots).into_iter().enumerate() {
                let Some(name) = name else { continue };
                if track.slot(scene).is_some() {
                    continue;
                }
                let copy = track.timeline().find(|c| c.name == name).map(|c| c.in_slot(scene));
                track.clips.extend(copy);
            }
        }
    }

    /// Automation lanes and modulators whose device or parameter is gone go too.
    pub fn prune_targets(&mut self) {
        for track in &mut self.tracks {
            let has = |devices: &[Device], device: &str, param: &str| devices.iter().any(|d| d.id == device && d.params.iter().any(|p| p.id == param));
            let devices = track.devices.clone();
            track.automation.retain(|lane| match &lane.target {
                AutoTarget::Param { device, param } => has(&devices, device, param),
                _ => true,
            });
            for d in &mut track.devices {
                if d.modulates.as_ref().is_some_and(|t| !has(&devices, &t.device, &t.param)) {
                    d.modulates = None;
                }
            }
        }
    }

    /// The device chain of a track, a return track, or the master ([`MASTER`]).
    pub fn chain(&self, owner: &str) -> Option<&[Device]> {
        if let Some((host, device, key)) = nested_path(owner) {
            let rack = self.chain(host)?.iter().find(|d| d.id == device)?;
            return match rack.kind.as_str() {
                "drum-rack" if pad_key(key) => Some(rack.pad_chains.iter().find(|c| c.key == key).map_or(&[][..], |c| c.devices.as_slice())),
                FX_RACK => rack.fx_chains.iter().find(|c| c.id == key).map(|c| c.devices.as_slice()),
                _ => None,
            };
        }
        if owner == MASTER {
            return Some(&self.master_devices);
        }
        if let Some(t) = self.track(owner) {
            return Some(&t.devices);
        }
        self.returns.iter().find(|r| r.id == owner).map(|r| r.devices.as_slice())
    }

    /// The device chain of a track, a return track or the master, and what holds it.
    pub fn chain_mut(&mut self, owner: &str) -> Option<(ChainOwner, &mut Vec<Device>)> {
        if let Some((host, device, key)) = nested_path(owner) {
            let (_, devices) = self.chain_mut(host)?;
            let rack = devices.iter_mut().find(|d| d.id == device)?;
            if rack.kind == FX_RACK {
                return rack.fx_chains.iter_mut().find(|c| c.id == key).map(|c| (ChainOwner::Rack, &mut c.devices));
            }
            if rack.kind != "drum-rack" || !pad_key(key) {
                return None;
            }
            let at = match rack.pad_chains.iter().position(|c| c.key == key) {
                Some(at) => at,
                None => {
                    rack.pad_chains.push(PadChain { key: key.to_string(), devices: Vec::new() });
                    rack.pad_chains.len() - 1
                }
            };
            return Some((ChainOwner::Pad, &mut rack.pad_chains[at].devices));
        }
        if owner == MASTER {
            return Some((ChainOwner::Master, &mut self.master_devices));
        }
        if let Some(t) = self.tracks.iter_mut().find(|t| t.id == owner) {
            let kind = match t.kind {
                TrackKind::Midi => ChainOwner::MidiTrack,
                TrackKind::Audio => ChainOwner::AudioTrack,
                TrackKind::Group => ChainOwner::GroupTrack,
            };
            return Some((kind, &mut t.devices));
        }
        self.returns.iter_mut().find(|r| r.id == owner).map(|r| (ChainOwner::Return, &mut r.devices))
    }

    /// Every device chain in the song: tracks, returns and the master.
    pub fn chains_mut(&mut self) -> impl Iterator<Item = &mut Device> {
        let tracks = self.tracks.iter_mut().flat_map(|t| t.devices.iter_mut());
        tracks.chain(self.returns.iter_mut().flat_map(|r| r.devices.iter_mut())).chain(self.master_devices.iter_mut())
    }

    /// A sidechain whose track is gone listens to its own input again.
    pub fn prune_sidechains(&mut self) {
        let tracks: std::collections::HashSet<String> = self.tracks.iter().map(|t| t.id.clone()).collect();
        for device in self.chains_mut() {
            if device.sidechain.as_ref().is_some_and(|t| !tracks.contains(t)) {
                device.sidechain = None;
            }
        }
    }

    /// Whether a mapping's target is still in the song.
    pub fn has_target(&self, target: &MappingTarget) -> bool {
        match target {
            MappingTarget::Param { track, device, param } => self
                .chain(track)
                .is_some_and(|chain| chain.iter().flat_map(Device::with_inner).any(|d| d.id == *device && d.params.iter().any(|p| p.id == *param))),
            MappingTarget::Volume { track } | MappingTarget::Pan { track } => self.track(track).is_some(),
        }
    }

    /// Drops mappings whose track, device or parameter is gone.
    pub fn prune_mappings(&mut self) {
        let keep: Vec<bool> = self.mappings.iter().map(|m| self.has_target(&m.target)).collect();
        let mut keep = keep.into_iter();
        self.mappings.retain(|_| keep.next().unwrap_or(true));
    }

    pub fn track(&self, id: &str) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_mut(&mut self, id: &str) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    pub fn asset(&self, id: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.id == id)
    }

    /// Finds a clip anywhere in the project: (track index, clip index).
    pub fn find_clip(&self, clip_id: &str) -> Option<(usize, usize)> {
        self.tracks
            .iter()
            .enumerate()
            .find_map(|(ti, t)| t.clips.iter().position(|c| c.id == clip_id).map(|ci| (ti, ci)))
    }

    /// Sets a MIDI clip's notes, and those of every clip linked to it.
    /// Returns how many clips changed.
    pub fn set_clip_notes(&mut self, clip_id: &str, mut notes: Vec<Note>) -> usize {
        tidy_notes(&mut notes);
        let link = self.find_clip(clip_id).and_then(|(ti, ci)| self.tracks[ti].clips[ci].link().map(str::to_string));
        let mut changed = 0;
        for clip in self.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()) {
            let linked = clip.id == clip_id || (link.is_some() && clip.link() == link.as_deref());
            if let (true, ClipContent::Midi { notes: current, .. }) = (linked, &mut clip.content) {
                current.clone_from(&notes);
                changed += 1;
            }
        }
        changed
    }

    /// Drops asset entries that no clip or device refers to any more.
    pub fn prune_assets(&mut self) {
        let takes = self.tracks.iter().flat_map(|t| t.takes.iter().flat_map(|l| l.clips.iter().filter_map(|c| c.asset())));
        let clips = self.tracks.iter().flat_map(|t| t.clips.iter().filter_map(|c| c.asset())).chain(takes);
        // Samples anywhere: instruments, and impulse responses on returns, the master and inside racks.
        let chains = self.tracks.iter().map(|t| &t.devices).chain(self.returns.iter().map(|r| &r.devices)).chain(std::iter::once(&self.master_devices));
        let samples = chains.flat_map(|c| c.iter().flat_map(Device::with_nested).flat_map(|d| d.samples.iter().map(|s| s.asset.as_str())));
        let frozen = self.tracks.iter().filter_map(|t| t.frozen.as_deref());
        let used: std::collections::HashSet<String> = clips.chain(samples).chain(frozen).map(str::to_string).collect();
        self.assets.retain(|a| used.contains(&a.id));
    }

    /// Brings devices saved by older versions up to date with the catalog.
    pub fn refresh_devices(&mut self) {
        for device in self.chains_mut() {
            catalog::refresh(device);
            for chain in &mut device.chains {
                catalog::refresh(&mut chain.device);
            }
            for chain in &mut device.pad_chains {
                chain.devices.iter_mut().for_each(catalog::refresh);
            }
            for chain in &mut device.fx_chains {
                chain.devices.iter_mut().for_each(catalog::refresh);
            }
        }
    }

    /// Macros of effect racks whose chain or effect is gone go.
    pub fn prune_macros(&mut self) {
        for rack in self.chains_mut().filter(|d| d.kind == FX_RACK) {
            let chains = &rack.fx_chains;
            rack.macros.retain(|m| {
                chains.iter().find(|c| c.id == m.chain).is_some_and(|c| c.devices.iter().any(|d| Some(d.id.as_str()) == m.device.as_deref() && d.params.iter().any(|p| p.id == m.param)))
            });
        }
    }

    /// Drum Rack chains left without effects go.
    pub fn prune_pad_chains(&mut self) {
        for device in self.tracks.iter_mut().flat_map(|t| t.devices.iter_mut()) {
            device.pad_chains.retain(|c| !c.devices.is_empty());
        }
    }

    /// The next free colour, cycling through the palette.
    pub fn next_color(&self) -> u8 {
        (self.tracks.len() % 8) as u8 + 1
    }
}

pub fn default_returns() -> Vec<ReturnTrack> {
    vec![
        ReturnTrack {
            id: crate::new_id("r"),
            name: "A · Reverb".into(),
            volume: -6.0,
            pan: 0.0,
            mute: false,
            devices: catalog::device("reverb", &[("mix", 100.0)]).into_iter().collect(),
            pre_fader: false,
        },
        ReturnTrack {
            id: crate::new_id("r"),
            name: "B · Delay".into(),
            volume: -9.0,
            pan: 0.0,
            mute: false,
            devices: catalog::device("delay", &[("mix", 100.0)]).into_iter().collect(),
            pre_fader: false,
        },
    ]
}
