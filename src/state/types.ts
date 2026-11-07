/*
 * The project model, mirroring soniq-project (Rust) field for field. The Rust
 * side owns the project; the UI receives it after every change.
 */
import { tempoOf, type TempoMap } from "./tempo";

export type TrackKind = "audio" | "midi" | "group";
export type ViewId = "arrange" | "session" | "mixer" | "editor" | "piano";
export type ToolId = "pointer" | "range" | "draw" | "split" | "erase" | "automation" | "zoom" | "hand";
export type Mode = "essentials" | "pro";
export type Theme = "dark" | "light";
export type Snap = "bar" | "beat" | "1/8" | "1/16" | "off";
export type SimpleEditOp = "normalize" | "amplify" | "fadeIn" | "fadeOut" | "reverse" | "silence";
/** Mirrors `soniq_io::EditOp`: plain strings, or `{ loudness: targetLufs }`. */
export type EditOp = SimpleEditOp | { loudness: number };
export type ClipboardAction = "copy" | "cut" | "delete" | "paste";

export interface Note {
  /** Beats from the clip start. */
  start: number;
  length: number;
  pitch: number;
  velocity: number;
}

interface ClipBase {
  id: string;
  name: string;
  /** Timeline position in beats (0 for a session clip). */
  start: number;
  /** Session view: the scene whose slot this clip waits in; absent on the timeline. */
  slot?: number;
  /** Session view: what happens after the clip has played a while. */
  follow?: FollowAction;
}

export type Follow = "stop" | "again" | "next" | "previous" | "first" | "last" | "any" | "other";

export interface FollowAction {
  action: Follow;
  /** After this many bars of playing. */
  bars: number;
}

export interface AudioClip extends ClipBase {
  kind: "audio";
  asset: string;
  /** Seconds into the asset where the clip starts. */
  offset: number;
  /** Seconds of audio the clip plays. */
  duration: number;
  /** Clip gain in dB. */
  gain: number;
  /** Linear fade lengths in seconds. */
  fadeIn: number;
  fadeOut: number;
  /** Follows the song's tempo: its beats stay on the song's beats. */
  warp?: Warp;
  /** The shape of the fades and crossfades; absent is the standard (straight fades, equal-power crossfades). */
  curve?: FadeCurve;
}

export type FadeCurve = "linear" | "equalPower" | "sCurve";

export type WarpMode = "beats" | "tones" | "repitch";

export interface Warp {
  /** The audio's own tempo: at this song tempo it plays at its original speed. */
  bpm: number;
  mode: WarpMode;
}

/** A point of the tempo track (mirrors `soniq_project::TempoPoint`). */
export interface TempoPoint {
  time: number;
  bpm: number;
}

export interface MidiClip extends ClipBase {
  kind: "midi";
  /** Length in beats. */
  length: number;
  notes: Note[];
  /** Clips with the same link share their notes (patterns). */
  link?: string;
}

export type Clip = AudioClip | MidiClip;

export interface Asset {
  id: string;
  name: string;
  channels: number;
  sampleRate: number;
  frames: number;
  path?: string;
  missing?: boolean;
}

export interface DeviceParam {
  id: string;
  label: string;
  value: number;
  min: number;
  max: number;
  default: number;
  unit: string;
  log?: boolean;
  bipolar?: boolean;
  /** Values snap to multiples of this. */
  step?: number;
  /** Named choices; the value is the index. */
  options?: string[];
  /** Panel section, or `pad:N` for a drum pad's parameters. */
  group?: string;
}

export type DeviceRole = "instrument" | "effect";

/** A sample loaded into a device: a drum pad's note, or 0 for the sampler. */
export interface SampleRef {
  key: number;
  asset: string;
}

export interface Device {
  id: string;
  type: string;
  name: string;
  role: DeviceRole;
  on: boolean;
  params: DeviceParam[];
  samples?: SampleRef[];
  /** An instrument rack's chains. */
  chains?: Chain[];
  /** What an instrument rack's macros move. */
  macros?: MacroMap[];
  /** The track a compressor or gate listens to instead of its own input. */
  sidechain?: string;
  /** What a modulator (LFO, envelope follower) moves: a parameter of another device on its track. */
  modulates?: ParamRef;
  /** The plugin a `plugin` device hosts; its params are the plugin controls the panel shows. */
  plugin?: PluginRef;
  /** The plugin's own state (base64) as of the last save. */
  state?: string;
  /** A Drum Rack's pad effects and its returns A and B. */
  padChains?: PadChain[];
}

/** A Drum Rack pad's audio effects (key: its note), or return "A" or "B". */
export interface PadChain {
  key: string;
  devices: Device[];
}

/** A key (loops) or a root note (one-shots), as the library heard it. */
export interface KeyGuess {
  tonic: number;
  minor: boolean;
  root: boolean;
}

export interface SampleInfo {
  seconds: number;
  channels: number;
  sampleRate: number;
  kind: "oneShot" | "loop";
  bpm: number | null;
  key: KeyGuess | null;
  peakDb: number;
}

export interface LibraryEntry {
  path: string;
  name: string;
  folder: string;
  kind: "sample" | "preset";
  size: number;
  modified: number;
  info?: SampleInfo;
  autoTags: string[];
  device?: string;
  error?: string;
}

export interface LibraryView {
  folders: string[];
  entries: LibraryEntry[];
  favourites: string[];
  tags: Record<string, string[]>;
  recent: string[];
  /** While indexing: files done and files to do. */
  scanning: [number, number] | null;
  presets: string;
}

export interface Audition {
  seconds: number;
  looping: boolean;
  fromBpm: number | null;
  semitones: number;
}

export interface PluginRef {
  format: string;
  path: string;
  id: string;
  vendor: string;
}

/** How a plugin device's plugin is doing (each runs in a sandbox process of its own). */
export interface PluginStatus {
  state: "starting" | "running" | "crashed" | "failed";
  message: string | null;
  /** Samples it delays its output by. */
  latency: number;
  /** It has a window of its own, and whether that is open. */
  gui: boolean;
  window: boolean;
}

/** A plugin the browser lists. */
export interface PluginEntry {
  format: "clap" | "vst3";
  path: string;
  id: string;
  name: string;
  vendor: string;
  version: string;
  description: string;
  instrument: boolean;
}

export interface PluginList {
  plugins: PluginEntry[];
  /** Files that could not be loaded: path and why. */
  failed: [string, string][];
  /** The standard CLAP folders, then the user's. */
  standard: string[];
  folders: string[];
}

/** One of a plugin's parameters, as the plugin reports it. */
export interface PluginParamInfo {
  id: number;
  name: string;
  module: string;
  min: number;
  max: number;
  default: number;
  value: number;
  text: string;
  stepped: boolean;
  /** Steps a stepped value takes from min to max (0: one per unit). */
  steps: number;
  automatable: boolean;
  hidden: boolean;
  readonly: boolean;
}

export interface ParamRef {
  device: string;
  param: string;
}

/** What an automation lane moves. */
export type AutoTarget = { kind: "volume" } | { kind: "pan" } | { kind: "send"; index: number } | { kind: "param"; device: string; param: string };

export interface AutoPoint {
  /** Beats from the song's start. */
  time: number;
  value: number;
}

/** Breakpoints joined by straight lines; the value holds before the first and after the last. */
export interface AutomationLane {
  target: AutoTarget;
  points: AutoPoint[];
}

export function sameAutoTarget(a: AutoTarget, b: AutoTarget): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "send" && b.kind === "send") return a.index === b.index;
  if (a.kind === "param" && b.kind === "param") return a.device === b.device && a.param === b.param;
  return true;
}

/** A lane's value at `time` (beats), as the engine plays it. */
export function laneValue(points: AutoPoint[], time: number): number | null {
  if (points.length === 0) return null;
  const after = points.findIndex((p) => p.time > time);
  if (after === 0) return points[0].value;
  if (after < 0) return points[points.length - 1].value;
  const a = points[after - 1];
  const b = points[after];
  return b.time > a.time ? a.value + ((b.value - a.value) * (time - a.time)) / (b.time - a.time) : a.value;
}

/** One chain of an instrument rack: an instrument for a range of keys and velocities. */
export interface Chain {
  id: string;
  name: string;
  on: boolean;
  device: Device;
  keyLow: number;
  keyHigh: number;
  /** 1–127. */
  velLow: number;
  velHigh: number;
  /** dB. */
  volume: number;
  /** −1…1. */
  pan: number;
}

export type ChainPatch = Partial<Pick<Chain, "name" | "on" | "keyLow" | "keyHigh" | "velLow" | "velHigh" | "volume" | "pan">>;

/** Macro `index` (0–7) sweeping a chain's parameter from `min` to `max`. */
export interface MacroMap {
  index: number;
  chain: string;
  param: string;
  min: number;
  max: number;
}

/** Podcast mode: a speaker (levelled), music (ducked under the speakers), or neither. */
export type TrackRole = "none" | "speaker" | "music";

export interface Track {
  id: string;
  name: string;
  kind: TrackKind;
  /** 1–8, maps to the --track-N tokens. */
  color: number;
  clips: Clip[];
  /** Fader in dB. */
  volume: number;
  /** −1 (left) … 1 (right). */
  pan: number;
  mute: boolean;
  solo: boolean;
  arm: boolean;
  devices: Device[];
  /** Send levels to return A and B, 0…1. */
  sends: [number, number];
  /** Audio tracks: input channels to record, "1", "2", "1/2", … */
  input: string;
  /** When the track plays its input: never, while armed, or always. */
  monitor: Monitor;
  /** What moves over time: the mixer and the devices' parameters. */
  automation?: AutomationLane[];
  /** The group track this one plays into. */
  group?: string;
  /** Frozen: the asset its clips and devices were rendered to, played instead while the devices rest. */
  frozen?: string;
  /** Podcast mode's role; absent means neither. */
  role?: TrackRole;
}

/** The clips on the timeline (not those waiting in session slots). */
export function timelineClips(track: Track): Clip[] {
  return track.clips.filter((c) => c.slot == null);
}

/** The session clip waiting in a scene's slot. */
export function slotClip(track: Track, scene: number): Clip | undefined {
  return track.clips.find((c) => c.slot === scene);
}

export type Monitor = "off" | "auto" | "in";

/** A point marker, or a region when it has an end (Audacity's labels). */
export interface Marker {
  id: string;
  name: string;
  /** Beats. */
  start: number;
  end?: number;
}

export interface ReturnTrack {
  id: string;
  name: string;
  volume: number;
  pan: number;
  mute: boolean;
  devices: Device[];
  /** Tracks send to it before their fader. */
  preFader: boolean;
}

/** The id that names the master's chain in device ops. */
export const MASTER = "master";

export interface Project {
  version: number;
  id: string;
  name: string;
  bpm: number;
  beatsPerBar: number;
  key: string;
  tracks: Track[];
  returns: ReturnTrack[];
  scenes: string[];
  masterVolume: number;
  assets: Asset[];
  markers: Marker[];
  /** MIDI controller knobs, faders and buttons and what they move. */
  mappings?: ControllerMapping[];
  /** Effects on the master, before its fader. */
  masterDevices?: Device[];
  /** Session clips start on the next multiple of this many beats (0: at once). */
  launchQuantize: number;
  /** The tempo track; empty, the song plays at `bpm` throughout. */
  tempo?: TempoPoint[];
  /** Saved mixes to go back to. */
  mixes?: Mix[];
  /** While comparing two mixes (A/B): the one not playing, named "A" or "B". */
  otherMix?: Mix | null;
}

/** A song to start from: one of Soniq's, or the user's own. */
export interface TemplateInfo {
  id: string;
  name: string;
  description: string;
  builtin: boolean;
  /** The view the song opens in. */
  view: ViewId;
}

/** A song opened or saved lately. */
export interface RecentProject {
  path: string;
  name: string;
  /** Seconds since 1970. */
  opened: number;
  /** The file is still there. */
  exists: boolean;
}

/** What undo reaches (oldest first) and redo reaches (next first). */
export interface HistoryList {
  past: string[];
  future: string[];
}

/** The mixer as it was: faders, pans and mutes, the tracks' solo and sends, and the master fader. */
export interface Mix {
  id: string;
  name: string;
  tracks: { track: string; volume: number; pan: number; mute: boolean; solo: boolean; sends: [number, number] }[];
  returns: { return: string; volume: number; pan: number; mute: boolean }[];
  masterVolume: number;
}

/** What a MIDI controller moves: a device parameter (a rack's macros and chain instruments too), or a track's volume or pan. */
export type MappingTarget =
  | { kind: "param"; track: string; device: string; param: string }
  | { kind: "volume"; track: string }
  | { kind: "pan"; track: string };

export interface ControllerMapping {
  id: string;
  /** 0–15; absent listens on every channel. */
  channel?: number;
  cc: number;
  target: MappingTarget;
}

export function sameTarget(a: MappingTarget, b: MappingTarget): boolean {
  if (a.kind !== b.kind || a.track !== b.track) return false;
  return a.kind !== "param" || (b.kind === "param" && a.device === b.device && a.param === b.param);
}

export interface TrackPatch {
  name?: string;
  volume?: number;
  pan?: number;
  mute?: boolean;
  solo?: boolean;
  arm?: boolean;
  sends?: [number, number];
  input?: string;
  monitor?: Monitor;
  role?: TrackRole;
}

/** Every change the UI can ask for; mirrors `soniq_project::Op`. */
export type Op =
  | { op: "setTrack"; id: string; patch: TrackPatch }
  | { op: "setReturn"; id: string; patch: { volume?: number; pan?: number; mute?: boolean; preFader?: boolean } }
  | { op: "setMaster"; volume: number }
  | { op: "addTrack"; kind: "audio" | "midi" }
  | { op: "removeTrack"; id: string }
  | { op: "addDevice"; track: string; device: Device }
  | { op: "removeDevice"; track: string; device: string }
  | { op: "toggleDevice"; track: string; device: string }
  | { op: "setParam"; track: string; device: string; param: string; value: number }
  | { op: "setTempo"; bpm: number }
  /** No points plays the song at its tempo throughout. */
  | { op: "setTempoPoints"; points: TempoPoint[] }
  | { op: "setWarp"; clip: string; warp: Warp | null }
  | { op: "setTimeSignature"; beatsPerBar: number }
  | { op: "setKey"; key: string }
  | { op: "deleteClip"; clip: string }
  | { op: "moveClip"; clip: string; track: string; start: number }
  | { op: "trimClip"; clip: string; start: number; length: number; ripple?: boolean }
  | { op: "moveClips"; clips: string[]; delta: number; tracks?: number }
  | { op: "deleteClips"; clips: string[]; ripple?: boolean }
  | { op: "duplicateClips"; clips: string[]; ripple?: boolean }
  | { op: "setFadeCurve"; clips: string[]; curve: FadeCurve | null }
  | { op: "extractToTrack"; clip: string; start: number; end: number; cut?: boolean }
  | { op: "splitClip"; clip: string; at: number }
  | { op: "duplicateClip"; clip: string; linked?: boolean }
  | { op: "unlinkClip"; clip: string }
  | { op: "setClipFades"; clip: string; fadeIn: number; fadeOut: number }
  | { op: "addMarker"; name?: string; start: number; end?: number }
  | { op: "setMarker"; id: string; name?: string; start?: number; end?: number }
  | { op: "removeMarker"; id: string }
  | { op: "addMidiClip"; track: string; start: number; length: number; notes?: Note[]; name?: string; slot?: number }
  /** No points removes the lane. */
  | { op: "setAutomation"; track: string; target: AutoTarget; points: AutoPoint[] }
  | { op: "setModTarget"; track: string; device: string; target: ParamRef | null }
  | { op: "groupTracks"; tracks: string[] }
  | { op: "ungroup"; group: string }
  | { op: "setTrackGroup"; track: string; group: string | null }
  | { op: "putInSlot"; clip: string; scene: number }
  | { op: "clearSlot"; track: string; scene: number }
  | { op: "setFollow"; clip: string; follow: FollowAction | null }
  | { op: "setLaunchQuantize"; beats: number }
  | { op: "addScene" }
  | { op: "setNotes"; clip: string; notes: Note[]; label?: string }
  | { op: "renameClip"; clip: string; name: string }
  | { op: "setDeviceSample"; track: string; device: string; key: number; asset: string | null }
  | { op: "addChain"; track: string; device: string; kind: string }
  | { op: "removeChain"; track: string; device: string; chain: string }
  | { op: "setChain"; track: string; device: string; chain: string; patch: ChainPatch }
  /** No `index` unmaps the parameter. */
  | { op: "mapMacro"; track: string; device: string; chain: string; param: string; index?: number; min?: number; max?: number }
  | { op: "setSidechain"; track: string; device: string; source: string | null }
  | { op: "setPluginParams"; track: string; device: string; params: DeviceParam[] }
  | { op: "unfreezeTrack"; track: string }
  /** Saves the mixer as it is ("Mix 1", "Mix 2"… without a name). */
  | { op: "saveMix"; name?: string }
  | { op: "recallMix"; id: string }
  /** Saves the mixer as it is into a saved mix. */
  | { op: "updateMix"; id: string }
  | { op: "renameMix"; id: string; name: string }
  | { op: "removeMix"; id: string }
  /** A/B: keeps the mixer as "A" and goes on as "B" the first time; then swaps them. */
  | { op: "swapMix" }
  /** Ends A/B, keeping the mix that plays. */
  | { op: "keepMix" }
  /** A frozen track becomes an audio track playing its rendered sound. */
  | { op: "flattenTrack"; track: string }
  | { op: "addMapping"; channel?: number; cc: number; target: MappingTarget }
  | { op: "removeMapping"; id: string };

/** How MIDI recording lands where a clip already is: a new clip on top, into it, or replacing its notes. */
export type MidiRecordMode = "new" | "overdub" | "replace";

export interface MidiPortStatus {
  name: string;
  enabled: boolean;
  connected: boolean;
}

export type ExportFormat = "wav" | "flac" | "mp3" | "ogg" | "opus";

/** Tags written into exported files; empty fields are left out. */
export interface Metadata {
  title: string;
  artist: string;
  album: string;
  year: string;
  genre: string;
  track: string;
  comment: string;
}

/** Mirrors `soniq_io::Encoding`. */
export interface Encoding {
  format: ExportFormat;
  /** WAV and FLAC: 16, 24 or 32 (float WAV). */
  bits: 16 | 24 | 32;
  /** MP3 and Opus, kbps. */
  bitrate: number;
  /** Ogg Vorbis, 0–10. */
  quality: number;
  metadata: Metadata;
}

export interface ExportOptions extends Encoding {
  path: string;
  sampleRate: number | null;
  /** Range in seconds. */
  start: number;
  end: number;
  /** Integrated loudness target in LUFS. */
  loudness: number | null;
  /** Export only this track (a stem). */
  track?: string;
  /** Chapters from the markers, written into the file. */
  chapters?: boolean;
  /** Also a chapters file next to it. */
  chaptersFile?: ChaptersFile | null;
  /** Reach the loudness target with a limiter instead of stopping short. */
  limit?: boolean;
}

/** Chapters files: Podcasting 2.0 JSON, `00:00 Title` lines, or a CUE sheet. */
export type ChaptersFile = "json" | "txt" | "cue";

export type Waveform = "sine" | "square" | "sawtooth" | "triangle";
export type NoiseColor = "white" | "pink" | "brown";

/** Mirrors `soniq_io::Effect`: `type` plus that effect's parameters. */
export type Effect =
  | { type: "noiseReduction"; reduction: number; sensitivity: number; smoothing: number }
  | { type: "clickRemoval"; sensitivity: number; maxWidth: number }
  | { type: "dcOffset" }
  | { type: "highPass"; frequency: number; slope: number }
  | { type: "lowPass"; frequency: number; slope: number }
  | { type: "bassTreble"; bass: number; treble: number }
  | { type: "graphicEq"; bands: number[] }
  | { type: "compressor"; threshold: number; ratio: number; attack: number; release: number; makeup: boolean }
  | { type: "limiter"; ceiling: number; release: number }
  | { type: "echo"; delay: number; decay: number }
  | { type: "reverb"; room: number; damping: number; mix: number }
  | { type: "changeSpeed"; percent: number }
  | { type: "changeTempo"; percent: number }
  | { type: "changePitch"; semitones: number }
  | { type: "truncateSilence"; threshold: number; minSilence: number; keep: number }
  | { type: "normalize"; peak: number }
  | { type: "loudness"; target: number }
  | { type: "amplify"; gain: number }
  | { type: "denoise"; reduction: number }
  | { type: "trimSilence"; threshold: number; keep: number }
  | { type: "level"; target: number; maxGain: number }
  | { type: "fixAudio"; target: number }
  | { type: "curveEq"; points: [number, number][] }
  | { type: "autoDuck"; track: string; depth: number; threshold: number; fadeDown: number; fadeUp: number; maxPause: number };

export type EffectType = Effect["type"];

/** Mirrors `soniq_io::Generator`. Amplitudes in dBFS. */
export type Generator =
  | { type: "tone"; waveform: Waveform; frequency: number; amplitude: number; seconds: number }
  | { type: "chirp"; waveform: Waveform; from: number; to: number; amplitude: number; seconds: number }
  | { type: "noise"; color: NoiseColor; amplitude: number; seconds: number }
  | { type: "dtmf"; digits: string; tone: number; gap: number; amplitude: number }
  | { type: "silence"; seconds: number }
  | { type: "rhythmTrack"; bpm: number; beatsPerBar: number; bars: number; sound: "tick" | "beep" | "wood"; accent: boolean; amplitude: number };

export type GeneratorType = Generator["type"];

export type GenerateTarget = { kind: "clip"; clip: string; start: number; end: number } | { kind: "track"; track: string | null; at: number };

export type FrequencyScale = "linear" | "log" | "mel";

/** A named chain of effects. */
export interface Macro {
  id: string;
  name: string;
  steps: Effect[];
  /** Shipped with the app; cannot be edited or deleted. */
  builtIn?: boolean;
}

export interface BatchOutput extends Encoding {
  folder: string;
  suffix: string;
}

export interface BatchProgress {
  done: number;
  total: number;
  file: string;
}

export interface BatchReport {
  written: string[];
  failed: { file: string; error: string }[];
  folder: string;
}

export interface ExportReport {
  path: string;
  seconds: number;
  loudness: number | null;
  peakDb: number;
  gainDb: number;
  heldBack: boolean;
  /** The limiter held the peaks so the target could be reached. */
  limited: boolean;
  chapters: number;
  chaptersFile: string | null;
}

/** One kind of repair in Fix my audio, in the order they run. */
export type FixKind = "dcOffset" | "rumble" | "clicks" | "noise" | "ends" | "pauses" | "loudness";

/** Mirrors `soniq_io::Diagnosis`: levels in dBFS, times in seconds. */
export interface Diagnosis {
  seconds: number;
  peakDb: number;
  loudness: number | null;
  dcOffset: number;
  rumbleDb: number;
  noiseDb: number;
  signalDb: number;
  clicks: number;
  silenceDb: number;
  lead: number;
  tail: number;
  pauses: number;
  pauseSeconds: number;
}

export interface FixStep {
  kind: FixKind;
  suggested: boolean;
  effects: Effect[];
}

export interface FixAnalysis {
  diagnosis: Diagnosis;
  steps: FixStep[];
}

/** A crash report kept on this computer (mirrors `crash::CrashReport`). */
export interface CrashReport {
  file: string;
  /** Seconds since 1970. */
  time: number;
  summary: string;
  seen: boolean;
}

export interface CrashStatus {
  enabled: boolean;
  asked: boolean;
  folder: string;
  reports: CrashReport[];
}

/** Mirrors `soniq_io::SpectrumPlot`: dB per bin, `sampleRate / size` Hz apart. */
export interface SpectrumPlot {
  sampleRate: number;
  size: number;
  db: number[];
  peakHz: number;
}

/** What Analyze finds and marks (mirrors `processing::MarkKind`). */
export type MarkKind = { kind: "clipping" } | { kind: "beats"; sensitivity: number } | { kind: "sounds"; threshold: number; minSilence: number; minSound: number };

/** How loud a track's clips are, for podcast mode. */
export interface SpeakerLevel {
  track: string;
  loudness: number | null;
  seconds: number;
}

export interface ProjectReply {
  project: Project;
  canUndo: boolean;
  canRedo: boolean;
  undoLabel: string | null;
  redoLabel: string | null;
  dirty: boolean;
  path: string | null;
  created: string | null;
  warnings: string[];
  clipped: boolean;
  /** What an effect found or did ("Removed 3 clicks"). */
  note: string | null;
  /** How each plugin device's plugin is doing, by device id (desktop only). */
  plugins?: Record<string, PluginStatus>;
  /** How far plugin delay compensation holds the whole mix back (desktop only). */
  pluginLatencyMs?: number;
}

/** Waveform overview: one absolute peak per `step` samples, per channel. */
export interface PeakData {
  channels: Float32Array[];
  step: number;
  sampleRate: number;
}

/** Notes in the order the backend keeps them: by start, then pitch. */
export function sortNotes<T extends Note>(notes: T[]): T[] {
  return [...notes].sort((a, b) => a.start - b.start || a.pitch - b.pitch);
}

const SNAP_BEATS: Record<Snap, number> = { bar: 4, beat: 1, "1/8": 0.5, "1/16": 0.25, off: 0 };

/** `beats` on the arrangement's grid. */
export function snapBeats(beats: number, snap: Snap, beatsPerBar: number): number {
  const step = snap === "bar" ? beatsPerBar : SNAP_BEATS[snap];
  return step ? Math.round(beats / step) * step : beats;
}

/** Length of a clip on the timeline, in beats. An unwarped audio clip's depends on the tempo where it sits (mirrors `Clip::length_beats`). */
export function clipLength(clip: Clip, tempo: TempoMap): number {
  if (clip.kind === "midi") return clip.length;
  if (clip.warp) return (clip.duration * clip.warp.bpm) / 60;
  return tempo.beatsAt(tempo.secondsAt(clip.start) + clip.duration) - clip.start;
}

/** Where the song ends: the last clip on the timeline, at least 32 bars. */
export function projectLength(project: Project): number {
  const tempo = tempoOf(project);
  let end = project.beatsPerBar * 32;
  for (const t of project.tracks) for (const c of timelineClips(t)) end = Math.max(end, c.start + clipLength(c, tempo));
  return end;
}
