import { ask, open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { getVersion } from "@tauri-apps/api/app";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { create } from "zustand";
import { backend } from "../engine/backend";
import { engine, inTauri, type ClipState, type EngineStatus, type InputStatus } from "../engine/bridge";
import { chainHost, createDevice, deviceSpec } from "./devices";
import { editLabel } from "./edits";
import { formatClock } from "./format";
import { loadKeymap, saveKeymap, type Keymap } from "./keymap";
import { reconcile } from "./reconcile";
import { beatAfter, tempoOf } from "./tempo";
import { BUILTIN_MACROS, describeEffect } from "./effects";
import { copyOf, paste, remove, snap, type NoteEdit } from "./notes";
import type { NoteToolType } from "./noteTools";
import { LESSONS, loadLessonsDone, saveLessonsDone } from "./lessons";
import { COLOR_SETS, DEFAULT_ACCENT, applyAppearance, isHex, type Appearance, type Density } from "./appearance";
import {
  clipLength,
  sameAutoTarget,
  sameTarget,
  slotClip,
  timelineClips,
  type AutoPoint,
  type AutoTarget,
  type FollowAction,
  type ParamRef,
  type TempoPoint,
  type Warp,
  type WarpMode,
  type AudioClip,
  type MappingTarget,
  type MidiClip,
  type MidiRecordMode,
  type RecentProject,
  type TemplateInfo,
  type Note,
  type ExportFormat,
  type ExportReport,
  type ChaptersFile,
  type TrackRole,
  type BatchOutput,
  type BatchProgress,
  type BatchReport,
  type ClipboardAction,
  type EditOp,
  type Effect,
  type EffectType,
  type ExportOptions,
  type FrequencyScale,
  type Generator,
  type Macro,
  type Mode,
  type Op,
  type DeviceParam,
  type PluginEntry,
  type PluginList,
  type PluginStatus,
  type Project,
  type ProjectReply,
  type Snap,
  type Theme,
  type ToolId,
  type Track,
  type TrackPatch,
  type ViewId,
} from "./types";

/** What one export writes: the song, the loop, one file per region, or one per track. */
export type ExportMode = "song" | "loop" | "regions" | "stems";

/** Choices the export dialog opens with (podcast mode's "Export the episode…"). */
export interface ExportPreset {
  format?: ExportFormat;
  bitrate?: number;
  loudness?: number;
  limit?: boolean;
  chapters?: boolean;
  chaptersFile?: ChaptersFile;
}

/** Where Soniq Studio lives: releases, issues, the code. */
export const REPO_URL = "https://github.com/Voltiq/soniq-studio";

/** Sketch mode's big button: waiting, counting in, waiting for the top of the loop, recording, saving. */
export type SketchPhase = "idle" | "count" | "wait" | "rec" | "saving";

export interface EditorSelection {
  /** Seconds from the clip start. */
  start: number;
  end: number;
  /** A frequency band, when selected on the spectrogram. */
  lowHz?: number;
  highHz?: number;
}

export type SpectralMode = "delete" | "attenuate" | "isolate";

export type ToastTone = "success" | "info" | "warning" | "danger";

export interface ToastAction {
  label: string;
  run: () => void;
}

export interface Toast {
  id: number;
  tone: ToastTone;
  title: string;
  detail?: string;
  actions?: ToastAction[];
  /** Stays until dismissed or acted on. */
  sticky?: boolean;
}

interface Loop {
  enabled: boolean;
  start: number;
  end: number;
}

interface HistoryState {
  canUndo: boolean;
  canRedo: boolean;
  undoLabel: string | null;
  redoLabel: string | null;
}

/** Audio formats the desktop decoder reads (soniq_io::SUPPORTED_EXTENSIONS). */
export const AUDIO_EXTENSIONS = ["wav", "wave", "flac", "mp3", "ogg", "oga", "opus", "m4a", "mp4", "aac", "caf", "aif", "aiff"];
export const MIDI_EXTENSIONS = ["mid", "midi", "smf"];

export type PianoRows = "all" | "scale" | "used";

export interface QuantizeSettings {
  /** Beats. */
  grid: number;
  /** 0–1: how far notes move toward the grid. */
  strength: number;
  /** 0–1: how late every second grid step lands. */
  swing: number;
  /** Also quantize where notes end. */
  ends: boolean;
}

export interface Store {
  ready: boolean;
  project: Project;
  history: HistoryState;
  dirty: boolean;
  path: string | null;
  busy: string | null;

  playing: boolean;
  recording: boolean;
  /** One bar of clicks before recording from a stop. */
  countIn: boolean;
  /** Where the current recording started, in beats. */
  recordStartBeats: number | null;
  metronome: boolean;
  loop: Loop;
  positionSec: number;
  snap: Snap;

  view: ViewId;
  mode: Mode;
  theme: Theme;
  /** Keyboard shortcuts: the set and the user's own changes. */
  keymap: Keymap;
  tool: ToolId;
  browserOpen: boolean;
  /** The meters panel (loudness, peaks, spectrum) on the right. */
  analysisOpen: boolean;
  /** Help › About: the version, updates, crash reports ("crashes" opens the newest report). */
  aboutOpen: "about" | "crashes" | null;
  appVersion: string | null;
  /** A newer version found on GitHub. */
  update: { version: string; notes: string; date: string | null } | null;
  updateStatus: "idle" | "checking" | "downloading" | "installing";
  /** 0–1 while downloading, when the size is known. */
  updateProgress: number | null;
  /** Look for updates once a day by itself. */
  autoUpdate: boolean;
  /** The theme editor's accent (`#rrggbb`), track colour set and density. */
  accent: string;
  colorSet: string;
  density: Density;
  /** Podcast mode's panel: speakers, ducking, chapters. */
  podcastOpen: boolean;
  exportPreset: ExportPreset | null;
  /** Sketch mode: one big record button and a loop. */
  sketchOpen: boolean;
  sketchBars: number;
  /** The input channels each layer records ("1", "2", "1/2"). */
  sketchInput: string;
  sketchPhase: SketchPhase;
  /** The layer being recorded. */
  sketchLayer: string | null;
  /** The start page: templates, recent songs, lessons. */
  welcomeOpen: boolean;
  /** What the start page puts the cursor in when it opens. */
  welcomeFocus: "template" | null;
  /** The start page opens with the app. */
  welcomeAtStart: boolean;
  templates: TemplateInfo[];
  recent: RecentProject[];
  /** The History popover by undo and redo. */
  historyOpen: boolean;
  /** The Find missing files dialog. */
  missingOpen: boolean;
  /** The lesson being followed, and its step. */
  lesson: { id: string; step: number } | null;
  /** Lessons finished, by id. */
  lessonsDone: string[];
  pxPerBeat: number;
  selectedTrackId: string;
  /** A return track or the master whose devices the chain panel shows instead of the selected track's. */
  chainOwner: string | null;
  selectedClipId: string | null;
  /** Seconds from the start of the clip open in the editor. */
  editorSelection: EditorSelection | null;
  /** Where Paste lands without a selection, in seconds from the clip start. */
  editorCursor: number;
  /** The part of the clip the editor shows, in seconds from its start; null shows all of it. */
  editorView: { start: number; end: number } | null;
  editorDisplay: "waveform" | "spectrogram";
  spectrumScale: FrequencyScale;
  zeroCross: boolean;
  hasClipboard: boolean;
  /** What each track's session plays, as the engine reports it; tracks playing the arrangement are absent. */
  clipStates: Record<string, ClipState>;
  /** Tracks picked together (Ctrl- or Shift-click on their headers), for grouping. */
  trackSelection: string[];
  /** Group tracks shown folded: their tracks are hidden in the arrangement. */
  folded: string[];
  /** Tracks whose automation lanes show, with the lanes asked for that have no points yet. */
  automationOpen: Record<string, AutoTarget[]>;
  /** The tempo lane under the markers: shown or hidden by hand, or null to show it when the song has tempo changes. */
  tempoLane: boolean | null;
  /** How each plugin device's plugin is doing, by device id. */
  pluginStatus: Record<string, PluginStatus>;
  /** How far plugin delay compensation holds the mix back, in ms. */
  pluginLatencyMs: number;
  /** The plugins found on disk (null until first asked for). */
  pluginList: PluginList | null;
  scanningPlugins: boolean;
  paletteOpen: boolean;
  settingsOpen: boolean;
  exportOpen: boolean;
  /** The effect whose dialog is open. */
  effectDialog: EffectType | null;
  generatorOpen: boolean;
  macroDialog: "clip" | "batch" | null;
  /** Built-in macros first, then the user's. */
  macros: Macro[];
  /** An effect preview is playing. */
  previewing: boolean;
  /** Seconds of noise in the learned profile. */
  noiseProfileSeconds: number | null;
  /** Record only inside the loop range, with a bar of pre-roll. */
  punch: boolean;
  selectedMarkerId: string | null;
  /** Indices into the notes of the clip open in the piano roll. */
  pianoSelection: number[];
  /** Piano roll grid in beats; 0 places freely. */
  pianoGrid: number;
  pianoRows: PianoRows;
  /** Drawn and moved notes land on the key's scale. */
  snapToScale: boolean;
  quantizeSettings: QuantizeSettings;
  noteClipboard: Note[];
  /** Piano roll zoom in pixels per beat; null fits the clip. */
  pianoZoom: number | null;
  /** Show the notes of other MIDI clips playing at the same time, faded. */
  pianoGhosts: boolean;
  /** The piano roll shows notes, or a step grid (FL's channel rack). */
  pianoMode: "notes" | "steps";
  /** The note tool whose dialog is open. */
  noteTool: NoteToolType | null;
  /** Letter keys play notes, like a small keyboard. */
  computerKeys: boolean;
  midiRecordMode: MidiRecordMode;
  /** Octave of the A key, Live-style (3 → C3, middle C). */
  keysOctave: number;
  keysVelocity: number;
  /** When MIDI last came in (performance.now()), for the activity light. */
  midiAt: number;
  /** What the next MIDI controller that moves gets mapped to, and its name. */
  midiLearn: { target: MappingTarget; label: string } | null;
  engineStatus: EngineStatus | null;
  inputStatus: InputStatus | null;
  toasts: Toast[];

  load(): Promise<void>;
  applyReply(reply: ProjectReply): void;
  run(op: Op): Promise<void>;

  togglePlay(): void;
  setPlaying(playing: boolean): void;
  stop(): void;
  toggleRecord(): Promise<void>;
  stopRecording(keepPlaying: boolean): Promise<void>;
  toggleCountIn(): void;
  toggleMetronome(): void;
  toggleLoop(): void;
  setLoopRange(start: number, end: number): void;
  seekBeats(beats: number): void;
  tick(now: number): void;
  syncFromEngine(positionSeconds: number): void;
  setBpm(bpm: number): void;
  /** Replaces the tempo track (none: the song tempo throughout); the playhead keeps its beat. */
  setTempoPoints(points: TempoPoint[]): void;
  /** Warps an audio clip to the song, finding its own tempo first. */
  warpClip(clipId: string, mode?: WarpMode): Promise<void>;
  setWarp(clipId: string, warp: Warp | null): void;
  /** A new MIDI track playing the clip's slices: at its transients, or every `division` beats. */
  sliceToMidi(clipId: string, division: number | null): Promise<void>;
  setBeatsPerBar(beats: number): void;
  setKey(key: string): void;
  setSnap(snap: Snap): void;

  setView(view: ViewId): void;
  setMode(mode: Mode): void;
  setTheme(theme: Theme): void;
  setTool(tool: ToolId): void;
  toggleBrowser(): void;
  toggleAnalysis(): void;
  setAboutOpen(open: "about" | "crashes" | null): void;
  setAutoUpdate(on: boolean): void;
  /** `manual`: say so when there is nothing new, or when the check fails. */
  checkForUpdates(manual: boolean): Promise<void>;
  /** Downloads and installs the update found, then restarts (asks about unsaved changes first). */
  installUpdate(): Promise<void>;
  /** After launch: crash reports to show, asking once about keeping them, the daily update check. */
  startupChecks(): Promise<void>;
  setAppearance(patch: Partial<Appearance>): void;
  togglePodcast(): void;
  setTrackRole(id: string, role: TrackRole): void;
  levelSpeakers(tracks: string[], target: number): Promise<void>;
  duckMusic(music: string[], speakers: string[], depth: number): Promise<void>;
  /** A marker named "Chapter N" at the playhead. */
  addChapter(): void;
  exportChapters(): Promise<void>;
  openSketch(fresh: boolean): Promise<void>;
  closeSketch(): Promise<void>;
  setSketchBars(bars: number): void;
  setSketchInput(input: string): void;
  /** Records a layer, or ends the one recording. */
  sketchRecord(): Promise<void>;
  finishSketchLayer(): Promise<void>;
  setKeymap(keymap: Keymap): void;
  setWelcomeOpen(open: boolean, focus?: "template"): void;
  setWelcomeAtStart(on: boolean): void;
  /** Templates and recent songs, fetched again. */
  loadStart(): Promise<void>;
  saveAsTemplate(name: string): Promise<boolean>;
  deleteTemplate(id: string): Promise<void>;
  forgetRecent(path: string): Promise<void>;
  setHistoryOpen(open: boolean): void;
  /** Undoes (below zero) or redoes that many steps at once. */
  jumpHistory(steps: number): Promise<void>;
  setMissingOpen(open: boolean): void;
  /** Asks for a folder and looks there for the song's missing audio. */
  findMissing(): Promise<void>;
  /** Asks where one missing file is. */
  locateAsset(asset: string): Promise<void>;
  startLesson(id: string): void;
  setLessonStep(step: number): void;
  endLesson(finished: boolean): void;
  toggleTempoLane(): void;
  zoom(factor: number): void;
  zoomToFit(width: number, lengthBeats: number): void;
  selectTrack(id: string): void;
  /** Shows a return's (or the master's) devices in the chain panel; null goes back to the selected track. */
  selectChain(id: string | null): void;
  selectClip(trackId: string, clipId: string): void;
  setEditorSelection(range: EditorSelection | null): void;
  setEditorCursor(seconds: number): void;
  setEditorView(view: { start: number; end: number } | null): void;
  /** Zooms the editor around `anchor` seconds (the view centre by default). */
  zoomEditor(factor: number, anchor?: number): void;
  zoomEditorToSelection(): void;
  setEditorDisplay(display: "waveform" | "spectrogram"): void;
  setSpectrumScale(scale: FrequencyScale): void;
  toggleZeroCross(): void;

  updateTrack(id: string, patch: TrackPatch): void;
  updateReturn(id: string, patch: { volume?: number; pan?: number; mute?: boolean; preFader?: boolean }): void;
  setMasterVolume(db: number): void;
  setParam(trackId: string, deviceId: string, paramId: string, value: number): void;
  addTrack(kind: "audio" | "midi"): void;
  removeTrack(id: string): void;
  /** Adds a device to a chain: a track, a return, or the master. */
  addDevice(owner: string, type: string): void;
  /** Lets a compressor or gate listen to another track (null: its own input). */
  setSidechain(owner: string, device: string, source: string | null): void;
  /** Finds the plugins on disk; `rescan` loads every file again. */
  loadPlugins(rescan: boolean): Promise<void>;
  setPluginFolders(folders: string[]): Promise<void>;
  /** Asks for a folder and looks for plugins in it too. */
  addPluginFolder(): Promise<void>;
  /** Adds a plugin to a chain: a track, a return, or the master. */
  addPlugin(owner: string, plugin: PluginEntry): Promise<void>;
  pluginWindow(device: string, show: boolean): Promise<void>;
  reloadPlugin(device: string): Promise<void>;
  /** Renders the track's clips through its devices; it plays that while they rest. */
  freezeTrack(track: string): Promise<void>;
  unfreezeTrack(track: string): void;
  /** A frozen track becomes an audio track playing its rendered sound. */
  flattenTrack(track: string): void;
  /** Renders a clip through its track's devices onto a new audio track below. */
  renderClip(clip: string): Promise<void>;
  /** The plugin controls a device shows (and automation can reach). */
  setPluginControls(owner: string, device: string, params: DeviceParam[]): void;
  removeDevice(trackId: string, deviceId: string): void;
  toggleDevice(trackId: string, deviceId: string): void;
  deleteSelectedClip(): void;
  moveClip(clip: string, track: string, start: number): void;
  trimClip(clip: string, start: number, length: number): void;
  splitClip(clip: string, at: number): void;
  splitAtPlayhead(): void;
  duplicateSelectedClip(): void;
  setClipFades(clip: string, fadeIn: number, fadeOut: number): void;
  edit(op: EditOp): Promise<void>;
  editorClipboard(action: ClipboardAction): Promise<void>;
  trimToSelection(): void;
  openEffect(type: EffectType | null): void;
  /** Runs effects over the selection (or the whole clip); true when it worked. */
  applyEffects(effects: Effect[], label?: string): Promise<boolean>;
  previewEffects(effects: Effect[]): Promise<void>;
  stopPreview(): void;
  setPreviewing(previewing: boolean): void;
  captureNoiseProfile(): Promise<void>;
  spectralEdit(mode: SpectralMode): Promise<void>;
  setGeneratorOpen(open: boolean): void;
  generate(generator: Generator, into: "clip" | "track"): Promise<boolean>;
  drawSamples(channel: number | null, first: number, samples: number[]): Promise<void>;
  setMacroDialog(dialog: "clip" | "batch" | null): void;
  loadMacros(): Promise<void>;
  saveMacros(userMacros: Macro[]): Promise<void>;
  runBatch(files: string[], steps: Effect[], output: BatchOutput, onProgress: (p: BatchProgress) => void): Promise<BatchReport | null>;
  togglePunch(): void;
  addMarker(): void;
  addRegionFromLoop(): void;
  selectMarker(id: string | null): void;
  moveMarker(id: string, start: number, end?: number): void;
  renameMarker(id: string, name: string): void;
  removeMarker(id: string): void;
  setExportOpen(open: boolean, preset?: ExportPreset): void;
  exportMix(options: Omit<ExportOptions, "path">, mode: ExportMode): Promise<void>;

  setPianoSelection(selection: number[]): void;
  setPianoGrid(grid: number): void;
  setPianoRows(rows: PianoRows): void;
  toggleSnapToScale(): void;
  setQuantizeSettings(settings: QuantizeSettings): void;
  setPianoZoom(pxPerBeat: number | null): void;
  togglePianoGhosts(): void;
  setPianoMode(mode: "notes" | "steps"): void;
  setNoteTool(tool: NoteToolType | null): void;
  /** A copy of the selected MIDI clip that keeps sharing its notes with it. */
  duplicateLinked(): void;
  unlinkClip(clip: string): void;
  /** Replaces a MIDI clip's notes as one undo step, then selects `selection`. */
  setNotes(clipId: string, notes: Note[], label: string, selection?: number[]): Promise<void>;
  /** Runs a note edit on the clip in the piano roll. */
  editNotes(edit: (notes: Note[], selection: number[]) => NoteEdit, label: string): void;
  pianoCopy(): void;
  pianoCut(): void;
  pianoPaste(): void;
  pianoDelete(): void;
  pianoSelectAll(): void;
  /** A MIDI clip (one bar and empty unless given) on a MIDI track; resolves to its id. */
  newMidiClip(trackId?: string, start?: number, length?: number, notes?: Note[], name?: string): Promise<string | null>;
  openPianoRoll(clipId?: string): void;
  importMidi(paths?: string[], trackId?: string, beats?: number): Promise<void>;
  exportMidi(clipId?: string): Promise<void>;
  /** Loads a file (picked when `path` is omitted) into a sampler (key 0) or a drum pad. */
  loadSample(trackId: string, deviceId: string, key: number, path?: string): Promise<void>;
  clearSample(trackId: string, deviceId: string, key: number): void;
  toggleComputerKeys(): void;
  setMidiRecordMode(mode: MidiRecordMode): void;
  captureMidi(): Promise<void>;
  setKeysOctave(octave: number): void;
  setKeysVelocity(velocity: number): void;
  /** Maps the next MIDI controller that moves to `target` (`label` names it in messages); null stops. */
  learnMidi(target: MappingTarget | null, label?: string): void;
  /** Fetches the project again after the backend changed it (a MIDI controller moved something). */
  refreshProject(): Promise<void>;

  /** A new song from a template ("empty" without one). */
  /** Resolves to whether a new song was made (the user may keep the old one). */
  newProject(template?: string): Promise<boolean>;
  openProject(path?: string): Promise<void>;
  openDemo(): Promise<void>;
  saveProject(): Promise<boolean>;
  saveProjectAs(): Promise<boolean>;
  importAudio(paths?: string[], trackId?: string, beats?: number): Promise<void>;
  confirmDiscard(): Promise<boolean>;
  checkRecovery(): Promise<void>;

  launchSlot(trackId: string, scene: number): void;
  launchScene(scene: number): void;
  stopTrack(trackId: string): void;
  stopAllClips(): void;
  /** Every track plays the arrangement again. */
  backToArrangement(): void;
  setClipStates(states: Record<string, ClipState>): void;
  /** An empty one-bar MIDI clip in a scene's slot, selected. */
  newSlotClip(trackId: string, scene: number): Promise<void>;
  /** A copy of a clip in a scene's slot on its track. */
  putInSlot(clipId: string, scene: number): void;
  clearSlot(trackId: string, scene: number): void;
  setFollow(clipId: string, follow: FollowAction | null): void;
  setLaunchQuantize(beats: number): void;
  addScene(): void;

  /** Adds a track to the picked tracks, or takes it out. */
  toggleTrackSelection(id: string): void;
  /** Groups the picked tracks (or the selected one). */
  groupTracks(ids?: string[]): void;
  ungroup(group: string): void;
  setTrackGroup(track: string, group: string | null): void;
  toggleFold(group: string): void;
  /** Shows or hides a track's automation lanes. */
  toggleAutomation(trackId: string): void;
  /** Shows the lane for `target` on its track, empty if it has no points yet. */
  showAutomation(trackId: string, target: AutoTarget): void;
  /** Hides a lane; a lane with points keeps them. */
  hideAutomationLane(trackId: string, target: AutoTarget): void;
  setAutomation(trackId: string, target: AutoTarget, points: AutoPoint[]): void;
  setModTarget(trackId: string, deviceId: string, target: ParamRef | null): void;

  undo(): Promise<void>;
  redo(): Promise<void>;
  toast(toast: Omit<Toast, "id">): void;
  dismissToast(id: number): void;
  setPaletteOpen(open: boolean): void;
  setSettingsOpen(open: boolean): void;
  setEngineStatus(status: EngineStatus | null): void;
  setInputStatus(status: InputStatus | null): void;
}

function readPref<T extends string>(key: string, allowed: T[], fallback: T): T {
  try {
    const v = localStorage.getItem(`soniq.${key}`) as T | null;
    return v && allowed.includes(v) ? v : fallback;
  } catch {
    return fallback;
  }
}

function readAccent(): string {
  try {
    const v = localStorage.getItem("soniq.accent");
    return v && isHex(v) ? v : DEFAULT_ACCENT;
  } catch {
    return DEFAULT_ACCENT;
  }
}

/** The update `check()` found, ready to download. */
let pendingUpdate: Update | null = null;
/** Once a day, give or take. */
const UPDATE_EVERY_MS = 20 * 3600 * 1000;

/** Sketch mode's recording: how many more times the loop must come round before the layer is complete. */
let sketchTake: { wraps: number } | null = null;
/** The loop a song had before sketch mode took it over; given back on leaving. */
let loopBeforeSketch: Loop | null = null;

function writePref(key: string, value: string) {
  try {
    localStorage.setItem(`soniq.${key}`, value);
  } catch {
    // Preferences are a convenience; private windows may refuse storage.
  }
}

// The UI clock: anchored to a position and a timestamp, re-anchored on every
// seek, tempo change and engine sync.
let anchorSec = 0;
let anchorTime = 0;
// While a count-in plays, the playhead waits until this time.
let countInUntil = 0;
let toastId = 1;
// React runs mount effects twice in development; the first load wins.
let loading: Promise<void> | null = null;

/** Beats ↔ seconds on the song's timeline, its tempo track included. */
const beatsToSec = (beats: number, project: Pick<Project, "bpm" | "tempo">) => tempoOf(project).secondsAt(beats);
const secToBeats = (sec: number, project: Pick<Project, "bpm" | "tempo">) => tempoOf(project).beatsAt(sec);

function anchor(sec: number) {
  anchorSec = sec;
  anchorTime = performance.now();
}

const message = (err: unknown) => (err instanceof Error ? err.message : String(err));

/** The editor selection, when it is more than a sliver. */
function selectionOf(s: Pick<Store, "editorSelection">): EditorSelection | null {
  const sel = s.editorSelection;
  return sel && sel.end - sel.start > 0.001 ? sel : null;
}

const describeSteps = (effects: Effect[]) => (effects.length === 1 ? describeEffect(effects[0]) : `${effects.length} effects`);

/** A name that works as a file name on every OS. */
const safeName = (name: string) => name.replace(/[\\/:*?"<>|]/g, "_").trim() || "Untitled";

const FORMAT_NAMES: Record<ExportFormat, string> = {
  wav: "WAV audio",
  flac: "FLAC audio",
  mp3: "MP3 audio",
  ogg: "Ogg Vorbis audio",
  opus: "Opus audio",
};

const EMPTY_PROJECT: Project = {
  version: 1,
  id: "",
  name: "",
  bpm: 120,
  beatsPerBar: 4,
  key: "C major",
  tracks: [],
  returns: [],
  scenes: [],
  masterVolume: 0,
  assets: [],
  markers: [],
  launchQuantize: 4,
};

const PROJECT_FILTER = [{ name: "Soniq project", extensions: ["soniq"] }];

export const useStore = create<Store>((set, get) => {
  const pushToast = (toast: Omit<Toast, "id">) => {
    const id = toastId++;
    set((s) => ({ toasts: [...s.toasts.slice(-3), { ...toast, id }] }));
    if (!toast.sticky) window.setTimeout(() => get().dismissToast(id), toast.actions ? 8000 : 4200);
  };

  const fail = (title: string) => (err: unknown) => pushToast({ tone: "danger", title, detail: message(err) });

  /** Runs a backend call that replaces the project, with a busy label and error toast. */
  const withReply = async (label: string | null, call: () => Promise<ProjectReply>, failure: string): Promise<ProjectReply | null> => {
    if (label) set({ busy: label });
    try {
      const reply = await call();
      get().applyReply(reply);
      return reply;
    } catch (err) {
      fail(failure)(err);
      return null;
    } finally {
      if (label) set({ busy: null });
    }
  };

  /** A new song: stop, rewind and clear UI state tied to the old one. */
  const resetForNewSong = () => {
    get().setPlaying(false);
    get().seekBeats(0);
    set({ clipStates: {}, editorSelection: null, selectedClipId: null, trackSelection: [], folded: [], automationOpen: {}, tempoLane: null });
  };

  /** Outside the desktop app nothing reports launches: the session plays them at once. */
  const simulate = (trackId: string, scene: number | null) =>
    set((s) => {
      const clipStates = { ...s.clipStates };
      clipStates[trackId] = { playing: scene, queued: null, stopping: false, progress: 0, arrangement: false };
      return { clipStates };
    });

  /** A launch started the engine's transport: the UI clock runs from where it is. */
  const followEngineStart = () => {
    if (get().playing) return;
    anchor(get().positionSec);
    countInUntil = 0;
    set({ playing: true });
  };

  /** Shows a launch waiting for its line until the engine reports it. */
  const pending = (trackId: string, scene: number | null) =>
    set((s) => {
      const was = s.clipStates[trackId] ?? { playing: null, queued: null, stopping: false, progress: 0, arrangement: true };
      return { clipStates: { ...s.clipStates, [trackId]: { ...was, queued: scene, stopping: scene === null && was.playing !== null } } };
    });

  return {
    ready: false,
    project: EMPTY_PROJECT,
    history: { canUndo: false, canRedo: false, undoLabel: null, redoLabel: null },
    dirty: false,
    path: null,
    busy: null,

    playing: false,
    recording: false,
    countIn: readPref<"on" | "off">("countIn", ["on", "off"], "off") === "on",
    recordStartBeats: null,
    metronome: false,
    loop: { enabled: false, start: 32, end: 48 },
    positionSec: 0,
    snap: "beat",

    view: "arrange",
    mode: readPref<Mode>("mode", ["essentials", "pro"], "pro"),
    theme: readPref<Theme>("theme", ["dark", "light"], "dark"),
    keymap: loadKeymap(),
    tool: "pointer",
    browserOpen: true,
    analysisOpen: false,
    aboutOpen: null,
    appVersion: null,
    update: null,
    updateStatus: "idle",
    updateProgress: null,
    autoUpdate: readPref<"on" | "off">("autoUpdate", ["on", "off"], "on") === "on",
    accent: readAccent(),
    colorSet: readPref("colorSet", COLOR_SETS.map((c) => c.id), "graphiq"),
    density: readPref<Density>("density", ["comfortable", "compact"], "comfortable"),
    podcastOpen: false,
    exportPreset: null,
    sketchOpen: false,
    sketchBars: Number(readPref("sketchBars", ["1", "2", "4", "8"], "4")),
    sketchInput: readPref("sketchInput", ["1", "2", "1/2"], "1"),
    sketchPhase: "idle",
    sketchLayer: null,
    welcomeOpen: false,
    welcomeFocus: null,
    welcomeAtStart: readPref<"on" | "off">("welcome", ["on", "off"], "on") === "on",
    templates: [],
    recent: [],
    historyOpen: false,
    missingOpen: false,
    lesson: null,
    lessonsDone: loadLessonsDone(),
    pxPerBeat: 14,
    selectedTrackId: "",
    chainOwner: null,
    selectedClipId: null,
    editorSelection: null,
    editorCursor: 0,
    editorView: null,
    editorDisplay: readPref<"waveform" | "spectrogram">("editorDisplay", ["waveform", "spectrogram"], "waveform"),
    spectrumScale: readPref<FrequencyScale>("spectrumScale", ["linear", "log", "mel"], "log"),
    zeroCross: readPref<"on" | "off">("zeroCross", ["on", "off"], "on") === "on",
    hasClipboard: false,
    clipStates: {},
    trackSelection: [],
    folded: [],
    automationOpen: {},
    tempoLane: null,
    pluginStatus: {},
    pluginLatencyMs: 0,
    pluginList: null,
    scanningPlugins: false,
    paletteOpen: false,
    settingsOpen: false,
    exportOpen: false,
    effectDialog: null,
    generatorOpen: false,
    macroDialog: null,
    macros: BUILTIN_MACROS,
    previewing: false,
    noiseProfileSeconds: null,
    punch: false,
    selectedMarkerId: null,
    pianoSelection: [],
    pianoGrid: 0.25,
    pianoRows: readPref<PianoRows>("pianoRows", ["all", "scale", "used"], "all"),
    snapToScale: readPref<"on" | "off">("snapToScale", ["on", "off"], "off") === "on",
    quantizeSettings: { grid: 0.25, strength: 1, swing: 0, ends: false },
    noteClipboard: [],
    pianoZoom: null,
    pianoGhosts: readPref<"on" | "off">("pianoGhosts", ["on", "off"], "on") === "on",
    pianoMode: readPref<"notes" | "steps">("pianoMode", ["notes", "steps"], "notes"),
    noteTool: null,
    computerKeys: readPref<"on" | "off">("computerKeys", ["on", "off"], "off") === "on",
    midiRecordMode: readPref<MidiRecordMode>("midiRecordMode", ["new", "overdub", "replace"], "new"),
    keysOctave: 3,
    keysVelocity: 100,
    midiAt: 0,
    midiLearn: null,
    engineStatus: null,
    inputStatus: null,
    toasts: [],

    load: () => {
      loading ??= (async () => {
        try {
          get().applyReply(await backend.load());
          set({ ready: true, welcomeOpen: get().welcomeAtStart });
          void get().loadStart();
          void get().loadMacros();
          void backend.noiseProfile().then((noiseProfileSeconds) => set({ noiseProfileSeconds }));
          await get().checkRecovery();
          void get().startupChecks();
        } catch (err) {
          fail("Could not load the project")(err);
        }
      })();
      return loading;
    },

    applyReply: (reply) => {
      set((s) => {
        const project = reconcile(s.project, reply.project);
        const tracks = project.tracks;
        // Something new (a track, or a clip from split or duplicate) becomes the selection.
        const createdTrack = tracks.find((t) => t.id === reply.created);
        const createdClipTrack = tracks.find((t) => t.clips.some((c) => c.id === reply.created));
        const keepTrack = tracks.some((t) => t.id === s.selectedTrackId) ? s.selectedTrackId : (tracks[0]?.id ?? "");
        const selectedTrackId = createdTrack?.id ?? createdClipTrack?.id ?? keepTrack;
        const clipExists = tracks.some((t) => t.clips.some((c) => c.id === s.selectedClipId));
        const createdMarker = project.markers.some((m) => m.id === reply.created);
        const markerExists = project.markers.some((m) => m.id === s.selectedMarkerId);
        return {
          project,
          history: { canUndo: reply.canUndo, canRedo: reply.canRedo, undoLabel: reply.undoLabel, redoLabel: reply.redoLabel },
          dirty: reply.dirty,
          path: reply.path,
          selectedTrackId,
          selectedClipId: createdClipTrack ? reply.created : clipExists ? s.selectedClipId : null,
          selectedMarkerId: createdMarker ? reply.created : markerExists ? s.selectedMarkerId : null,
          pluginStatus: reply.plugins ?? {},
          pluginLatencyMs: reply.pluginLatencyMs ?? 0,
        };
      });
      for (const warning of reply.warnings) pushToast({ tone: "warning", title: warning });
    },

    run: async (op) => {
      try {
        get().applyReply(await backend.apply(op));
      } catch (err) {
        pushToast({ tone: "info", title: message(err) });
      }
    },

    togglePlay: () => get().setPlaying(!get().playing),

    setPlaying: (playing) => {
      if (!playing && get().recording) {
        // In sketch mode stopping keeps the layer (as far as it got) and disarms it.
        if (get().sketchOpen) void get().finishSketchLayer().then(() => get().setPlaying(false));
        else void get().stopRecording(false);
        return;
      }
      anchor(get().positionSec);
      countInUntil = 0;
      engine.play(playing);
      set({ playing });
    },

    stop: () => {
      const s = get();
      if (s.recording) void s.stopRecording(false);
      else if (s.playing) s.setPlaying(false);
      else s.seekBeats(s.loop.enabled ? s.loop.start : 0);
    },

    toggleRecord: async () => {
      const s = get();
      if (s.recording) return s.stopRecording(true);
      // With session clips, recording also writes what the session plays into the arrangement.
      const sessionClips = s.project.tracks.some((t) => t.clips.some((c) => c.slot != null));
      if (!s.project.tracks.some((t) => t.arm) && !sessionClips) {
        pushToast({ tone: "info", title: "Arm a track first", detail: "Press the record dot on a track header, then record. Audio tracks record the input, MIDI tracks the keyboard." });
        return;
      }
      if (!inTauri) {
        pushToast({ tone: "info", title: "Recording needs the desktop app", detail: "Run npm run tauri dev." });
        return;
      }
      const { beatsPerBar } = s.project;
      const punch = s.punch && s.loop.end > s.loop.start;
      // Punching in from a stop starts a bar early, so you can play into it.
      if (punch && !s.playing) s.seekBeats(Math.max(0, s.loop.start - beatsPerBar));
      const countIn = s.countIn && !s.playing;
      try {
        await backend.recordStart(countIn, punch ? [beatsToSec(s.loop.start, s.project), beatsToSec(s.loop.end, s.project)] : null, s.midiRecordMode);
      } catch (err) {
        fail("Could not start recording")(err);
        return;
      }
      // While punching, the transport plays straight through the loop range.
      if (punch) engine.setLoop(false, 0, 0);
      const now = get();
      anchor(now.positionSec);
      // The count-in is a bar at the tempo where recording starts.
      if (countIn) countInUntil = performance.now() + ((beatsPerBar * 60) / tempoOf(s.project).bpmAt(positionBeats(now))) * 1000;
      set({ recording: true, playing: true, recordStartBeats: punch ? s.loop.start : positionBeats(now) });
    },

    stopRecording: async (keepPlaying) => {
      const s = get();
      const start = s.recordStartBeats ?? positionBeats(s);
      const end = s.punch ? Math.min(positionBeats(s), s.loop.end) : positionBeats(s);
      const seconds = beatsToSec(end, s.project) - beatsToSec(start, s.project);
      anchor(s.positionSec);
      countInUntil = 0;
      set({ recording: false, recordStartBeats: null, playing: keepPlaying && s.playing });
      engine.setLoop(s.loop.enabled, beatsToSec(s.loop.start, s.project), beatsToSec(s.loop.end, s.project));
      const reply = await withReply("Saving the recording…", () => backend.recordStop(keepPlaying), "Recording failed");
      const clips = (p: Project) => p.tracks.reduce((n, t) => n + t.clips.length, 0);
      if (reply && clips(reply.project) > clips(s.project) && seconds > 0.05) {
        const names = reply.project.tracks.filter((t) => t.arm).map((t) => t.name);
        pushToast({
          tone: "success",
          title: `Recorded ${formatClock(seconds)}`,
          detail: names.length ? `On ${names.join(", ")}` : "What the session played is in the arrangement now",
        });
      }
    },

    toggleCountIn: () => {
      const countIn = !get().countIn;
      writePref("countIn", countIn ? "on" : "off");
      set({ countIn });
    },

    toggleMetronome: () => {
      const metronome = !get().metronome;
      engine.setMetronome(metronome);
      set({ metronome });
    },

    toggleLoop: () => set((s) => ({ loop: { ...s.loop, enabled: !s.loop.enabled } })),

    setLoopRange: (start, end) => {
      const a = Math.max(0, Math.min(start, end));
      const b = Math.max(start, end);
      if (b - a < 0.25) return;
      set({ loop: { enabled: true, start: a, end: b } });
    },

    seekBeats: (beats) => {
      const sec = beatsToSec(Math.max(0, beats), get().project);
      anchor(sec);
      engine.seek(sec);
      set({ positionSec: sec });
    },

    tick: (now) => {
      const s = get();
      if (!s.playing) return;
      if (now < countInUntil) {
        // The count-in clicks while the playhead waits.
        anchor(s.positionSec);
        return;
      }
      let sec = anchorSec + (now - anchorTime) / 1000;
      // Sketch takes keep the loop going; only ordinary punch-ins play straight through.
      const punching = s.recording && s.punch && !sketchTake;
      if (sketchTake && s.sketchPhase === "count") set({ sketchPhase: "rec" });
      // Past the punch-out point the take is complete; playback carries on.
      if (punching && secToBeats(sec, s.project) > s.loop.end + 0.5) void s.stopRecording(true);
      if (s.loop.enabled && !punching) {
        const beats = secToBeats(sec, s.project);
        const prev = secToBeats(s.positionSec, s.project);
        if (prev < s.loop.end && beats >= s.loop.end) {
          sec = beatsToSec(s.loop.start + (beats - s.loop.end), s.project);
          anchor(sec);
          // The desktop engine wraps the loop itself, sample-accurately.
          if (!inTauri) engine.seek(sec);
          if (sketchTake && s.recording) {
            sketchTake.wraps -= 1;
            if (sketchTake.wraps <= 0) {
              sketchTake = null;
              // A moment past the top, so the pass's last notes are in the take whatever the latency.
              setTimeout(() => void get().finishSketchLayer(), 300);
            } else set({ sketchPhase: "rec" });
          }
        }
      }
      set({ positionSec: sec });
    },

    syncFromEngine: (positionSeconds) => {
      if (!get().playing) return;
      // Follow the engine whenever the drift is visible; it is the truth.
      if (Math.abs(positionSeconds - (anchorSec + (performance.now() - anchorTime) / 1000)) > 0.02) anchor(positionSeconds);
    },

    setBpm: (bpm) => {
      const s = get();
      const next = Math.min(999, Math.max(20, Math.round(bpm * 100) / 100));
      const points = s.project.tempo ?? [];
      if (points.length) {
        // With a tempo track the whole track moves, so the tempo at the playhead becomes `next`.
        const now = tempoOf(s.project).bpmAt(positionBeats(s));
        if (Math.abs(next - now) < 0.005) return;
        return s.setTempoPoints(points.map((p) => ({ time: p.time, bpm: (p.bpm * next) / now })));
      }
      if (next === s.project.bpm) return;
      // Keep the musical position when the tempo changes.
      const sec = beatsToSec(secToBeats(s.positionSec, s.project), { bpm: next, tempo: s.project.tempo });
      anchor(sec);
      engine.seek(sec);
      set({ positionSec: sec });
      void s.run({ op: "setTempo", bpm: next });
    },

    setTempoPoints: (points) => {
      const s = get();
      // Keep the musical position: the playhead stays on its beat.
      const beats = positionBeats(s);
      void s.run({ op: "setTempoPoints", points }).then(() => {
        const sec = beatsToSec(beats, get().project);
        anchor(sec);
        engine.seek(sec);
        set({ positionSec: sec });
      });
    },

    warpClip: async (clip, mode) => {
      if (!inTauri) {
        const s = get();
        const tempo = tempoOf(s.project);
        const start = s.project.tracks.flatMap((t) => t.clips).find((c) => c.id === clip)?.start ?? 0;
        return void s.run({ op: "setWarp", clip, warp: { bpm: tempo.bpmAt(start), mode: mode ?? "beats" } });
      }
      const reply = await withReply("Finding the tempo…", () => backend.warpClip(clip, mode), "Could not warp the clip");
      if (reply?.note) pushToast({ tone: "success", title: "Warped", detail: reply.note });
    },

    setWarp: (clip, warp) => void get().run({ op: "setWarp", clip, warp }),

    sliceToMidi: async (clip, division) => {
      if (!inTauri) {
        pushToast({ tone: "info", title: "Slicing needs the desktop app", detail: "Run npm run tauri dev." });
        return;
      }
      const reply = await withReply("Slicing…", () => backend.sliceToMidi(clip, division), "Could not slice the clip");
      if (reply?.note) pushToast({ tone: "success", title: "Sliced to MIDI", detail: reply.note });
    },

    setBeatsPerBar: (beatsPerBar) => void get().run({ op: "setTimeSignature", beatsPerBar }),
    setKey: (key) => void get().run({ op: "setKey", key }),
    setSnap: (snap) => set({ snap }),

    setView: (view) => set({ view }),
    setMode: (mode) => {
      writePref("mode", mode);
      set({ mode });
    },
    setTheme: (theme) => {
      writePref("theme", theme);
      set({ theme });
    },
    setTool: (tool) => set({ tool }),
    toggleBrowser: () => set((s) => ({ browserOpen: !s.browserOpen })),
    toggleAnalysis: () => set((s) => ({ analysisOpen: !s.analysisOpen })),

    setAppearance: (patch) => {
      const s = get();
      const next: Appearance = { accent: patch.accent ?? s.accent, colorSet: patch.colorSet ?? s.colorSet, density: patch.density ?? s.density };
      writePref("accent", next.accent);
      writePref("colorSet", next.colorSet);
      writePref("density", next.density);
      // Before the state changes, so everything that redraws reads the new tokens.
      applyAppearance(next);
      set(next);
    },

    togglePodcast: () => set((s) => ({ podcastOpen: !s.podcastOpen })),

    setAboutOpen: (aboutOpen) => set({ aboutOpen }),

    setAutoUpdate: (autoUpdate) => {
      writePref("autoUpdate", autoUpdate ? "on" : "off");
      set({ autoUpdate });
    },

    checkForUpdates: async (manual) => {
      if (!inTauri) {
        if (manual) pushToast({ tone: "info", title: "Updates need the desktop app" });
        return;
      }
      if (get().updateStatus !== "idle") return;
      set({ updateStatus: "checking" });
      try {
        const found = await check({ timeout: 20_000 });
        writePref("updateChecked", String(Date.now()));
        pendingUpdate = found;
        set({ update: found ? { version: found.version, notes: found.body ?? "", date: found.date ?? null } : null, updateStatus: "idle" });
        if (!found) {
          if (manual) pushToast({ tone: "success", title: "Soniq Studio is up to date", detail: get().appVersion ? `You have version ${get().appVersion}.` : undefined });
          return;
        }
        const id = toastId;
        pushToast({
          tone: "info",
          title: `Soniq Studio ${found.version} is ready`,
          detail: "Install it now and Soniq restarts; or later, from Help › About Soniq Studio.",
          sticky: true,
          actions: [
            {
              label: "Install and restart",
              run: () => {
                get().dismissToast(id);
                void get().installUpdate();
              },
            },
            {
              label: "What's new",
              run: () => {
                get().dismissToast(id);
                get().setAboutOpen("about");
              },
            },
          ],
        });
      } catch (err) {
        set({ updateStatus: "idle" });
        if (!manual) return;
        // The updater's own words are about JSON and remotes: say what it means.
        const text = message(err);
        pushToast({
          tone: "warning",
          title: "Could not check for updates",
          detail: /release json|remote|network|dns|connect|timed out/i.test(text)
            ? "GitHub could not be reached, or it has no release to offer yet. Check your connection and try again later."
            : text,
        });
      }
    },

    installUpdate: async () => {
      const update = pendingUpdate;
      if (!update || get().updateStatus !== "idle") return;
      // Unsaved changes are asked about first: the installer closes Soniq.
      if (!(await get().confirmDiscard())) return;
      set({ updateStatus: "downloading", updateProgress: null, busy: "Downloading the update…" });
      let total = 0;
      let got = 0;
      try {
        await update.downloadAndInstall((event) => {
          if (event.event === "Started") total = event.data.contentLength ?? 0;
          else if (event.event === "Progress") {
            got += event.data.chunkLength;
            set({ updateProgress: total ? got / total : null });
          } else set({ updateStatus: "installing", busy: "Installing…" });
        });
        // A clean close: no autosave left to offer back after the restart.
        await backend.closing();
        await relaunch();
      } catch (err) {
        set({ updateStatus: "idle", updateProgress: null, busy: null });
        fail("Could not install the update")(err);
      }
    },

    startupChecks: async () => {
      if (!inTauri) return;
      void getVersion().then((appVersion) => set({ appVersion })).catch(() => {});
      const status = await backend.crashStatus().catch(() => null);
      if (status) {
        const fresh = status.reports.filter((r) => !r.seen);
        if (fresh.length) {
          const id = toastId;
          pushToast({
            tone: "warning",
            title: fresh.length === 1 ? "Soniq crashed last time" : `Soniq crashed ${fresh.length} times since you last looked`,
            detail: "A report was saved on this computer. Look at it, and send it on if you like.",
            sticky: true,
            actions: [
              {
                label: "Show the report",
                run: () => {
                  get().dismissToast(id);
                  get().setAboutOpen("crashes");
                },
              },
              {
                label: "Dismiss",
                run: () => {
                  get().dismissToast(id);
                  void backend.crashSeen();
                },
              },
            ],
          });
        } else if (!status.asked) {
          const id = toastId;
          pushToast({
            tone: "info",
            title: "Keep crash reports?",
            detail: "If Soniq crashes, it can save what went wrong in a file on this computer, for you to send on if you like. Nothing is uploaded, and nothing from your songs is in it.",
            sticky: true,
            actions: [
              {
                label: "Keep reports",
                run: () => {
                  get().dismissToast(id);
                  void backend.crashEnable(true);
                },
              },
              {
                label: "No thanks",
                run: () => {
                  get().dismissToast(id);
                  void backend.crashEnable(false);
                },
              },
            ],
          });
        }
      }
      if (!get().autoUpdate) return;
      let last = 0;
      try {
        last = Number(localStorage.getItem("soniq.updateChecked")) || 0;
      } catch {
        // No storage: check this time.
      }
      if (Date.now() - last > UPDATE_EVERY_MS) await get().checkForUpdates(false);
    },

    setTrackRole: (id, role) => void get().run({ op: "setTrack", id, patch: { role } }),

    levelSpeakers: async (tracks, target) => {
      const reply = await withReply("Leveling the speakers…", () => backend.podcastLevel(tracks, target), "Could not level the speakers");
      if (reply?.note) pushToast({ tone: "success", title: "Speakers levelled", detail: reply.note });
    },

    duckMusic: async (music, speakers, depth) => {
      const reply = await withReply("Finding the speech…", () => backend.podcastDuck(music, speakers, depth), "Could not duck the music");
      if (!reply) return;
      // Show the lane that was written under each music track.
      set((s) => {
        const open = { ...s.automationOpen };
        for (const id of music) {
          const lanes = open[id] ?? [];
          if (!lanes.some((t) => t.kind === "volume")) open[id] = [...lanes, { kind: "volume" }];
        }
        return { automationOpen: open };
      });
      pushToast({ tone: "success", title: "Music ducked under speech", detail: reply.note ?? undefined });
    },

    addChapter: () => {
      const s = get();
      void s.run({ op: "addMarker", name: `Chapter ${s.project.markers.length + 1}`, start: positionBeats(s) });
    },

    exportChapters: async () => {
      const s = get();
      if (!inTauri) return fail("Saving chapters is not available")(new Error("Saving chapters needs the desktop app (npm run tauri dev)."));
      const path = await saveDialog({
        defaultPath: `${safeName(s.project.name)}.chapters.json`,
        filters: [
          { name: "Podcast chapters (JSON)", extensions: ["json"] },
          { name: "Text, a line each", extensions: ["txt"] },
          { name: "CUE sheet", extensions: ["cue"] },
        ],
      });
      if (!path) return;
      const ext = path.split(".").pop()?.toLowerCase();
      const format: ChaptersFile = ext === "txt" ? "txt" : ext === "cue" ? "cue" : "json";
      try {
        const n = await backend.chaptersExport(path, format);
        pushToast({
          tone: "success",
          title: `Saved ${n} ${n === 1 ? "chapter" : "chapters"}`,
          detail: path.split(/[\\/]/).pop(),
          actions: [{ label: "Show in folder", run: () => void revealItemInDir(path) }],
        });
      } catch (err) {
        fail("Could not save the chapters")(err);
      }
    },

    openSketch: async (fresh) => {
      if (fresh && !(await get().newProject("sketch"))) return;
      const s = get();
      // Over a song of your own, its loop comes back when you leave.
      loopBeforeSketch = fresh ? null : s.loop;
      set({ sketchOpen: true, welcomeOpen: false, loop: { enabled: true, start: 0, end: s.sketchBars * s.project.beatsPerBar } });
      if (!s.playing) s.seekBeats(0);
    },

    closeSketch: async () => {
      if (get().recording) await get().finishSketchLayer();
      set({ sketchOpen: false, view: "arrange", ...(loopBeforeSketch ? { loop: loopBeforeSketch } : {}) });
      loopBeforeSketch = null;
    },

    setSketchBars: (bars) => {
      writePref("sketchBars", String(bars));
      set((s) => ({ sketchBars: bars, loop: { enabled: true, start: 0, end: bars * s.project.beatsPerBar } }));
    },

    setSketchInput: (sketchInput) => {
      writePref("sketchInput", sketchInput);
      set({ sketchInput });
    },

    sketchRecord: async () => {
      const s = get();
      if (s.sketchPhase === "saving") return;
      if (s.recording) return get().finishSketchLayer();
      if (!inTauri) {
        pushToast({ tone: "info", title: "Recording needs the desktop app", detail: "Run npm run tauri dev." });
        return;
      }
      // Only the new layer is armed: the input (and the microphone) opens now and closes once it is saved.
      for (const t of s.project.tracks) if (t.arm) await get().run({ op: "setTrack", id: t.id, patch: { arm: false } });
      const isLayer = (name: string) => /^Layer \d+$/.test(name);
      let layer = get().project.tracks.find((t) => t.kind === "audio" && isLayer(t.name) && timelineClips(t).length === 0);
      if (!layer) {
        const number = get().project.tracks.filter((t) => isLayer(t.name)).length + 1;
        await get().run({ op: "addTrack", kind: "audio" });
        const created = get().selectedTrackId;
        await get().run({ op: "setTrack", id: created, patch: { name: `Layer ${number}` } });
        layer = get().project.tracks.find((t) => t.id === created);
      }
      if (!layer) return;
      const id = layer.id;
      if (layer.input !== get().sketchInput) await get().run({ op: "setTrack", id, patch: { input: get().sketchInput } });
      await get().run({ op: "setTrack", id, patch: { arm: true } });
      const now = get();
      // From a stop: a bar of count-in, then the pass starts at the top. While playing: at the next top.
      const fromTop = !now.playing;
      if (fromTop) now.seekBeats(now.loop.start);
      const pass: [number, number] = [beatsToSec(now.loop.start, now.project), beatsToSec(now.loop.end, now.project)];
      try {
        await backend.recordStart(fromTop, null, now.midiRecordMode, pass);
      } catch (err) {
        fail("Could not start recording")(err);
        await get().run({ op: "setTrack", id, patch: { arm: false } });
        return;
      }
      const at = get();
      anchor(at.positionSec);
      if (fromTop) countInUntil = performance.now() + ((at.project.beatsPerBar * 60) / tempoOf(at.project).bpmAt(at.loop.start)) * 1000;
      sketchTake = { wraps: fromTop ? 1 : 2 };
      set({ recording: true, playing: true, recordStartBeats: fromTop ? at.loop.start : positionBeats(at), sketchLayer: id, sketchPhase: fromTop ? "count" : "wait" });
    },

    finishSketchLayer: async () => {
      if (!get().recording) return;
      sketchTake = null;
      set({ sketchPhase: "saving" });
      await get().stopRecording(true);
      const id = get().sketchLayer;
      // Disarmed, the input closes again.
      if (id && get().project.tracks.some((t) => t.id === id && t.arm)) await get().run({ op: "setTrack", id, patch: { arm: false } });
      set({ sketchLayer: null, sketchPhase: "idle" });
    },
    setWelcomeOpen: (open, focus) => set({ welcomeOpen: open, welcomeFocus: open ? (focus ?? null) : null }),
    setWelcomeAtStart: (on) => {
      writePref("welcome", on ? "on" : "off");
      set({ welcomeAtStart: on });
    },
    loadStart: async () => {
      const [templates, recent] = await Promise.all([backend.templates().catch(() => [] as TemplateInfo[]), backend.recent().catch(() => [] as RecentProject[])]);
      set({ templates, recent });
    },
    saveAsTemplate: async (name) => {
      try {
        const info = await backend.saveTemplate(name);
        pushToast({ tone: "success", title: `Saved the template “${info.name}”`, detail: "It keeps the tracks, devices, sends and tempo; clips and audio stay with the song." });
        await get().loadStart();
        return true;
      } catch (err) {
        fail("Could not save the template")(err);
        return false;
      }
    },
    deleteTemplate: async (id) => {
      try {
        await backend.deleteTemplate(id);
        await get().loadStart();
      } catch (err) {
        fail("Could not delete the template")(err);
      }
    },
    forgetRecent: async (path) => {
      try {
        set({ recent: await backend.forgetRecent(path) });
      } catch (err) {
        fail("Could not change the list")(err);
      }
    },
    setHistoryOpen: (open) => set({ historyOpen: open }),
    jumpHistory: async (steps) => {
      if (steps) await withReply(null, () => backend.historyJump(steps), "Could not go back there");
    },
    setMissingOpen: (open) => set({ missingOpen: open }),
    findMissing: async () => {
      if (!inTauri) return fail("Not available here")(new Error("Finding files needs the desktop app (npm run tauri dev)."));
      const folder = await openDialog({ directory: true, multiple: false, title: "Where might the audio be?" });
      if (typeof folder !== "string") return;
      const reply = await withReply("Looking for the audio…", () => backend.findAssets(folder), "Could not look there");
      if (reply?.note) pushToast({ tone: reply.project.assets.some((a) => a.missing) ? "info" : "success", title: reply.note });
    },
    locateAsset: async (asset) => {
      if (!inTauri) return fail("Not available here")(new Error("Finding files needs the desktop app (npm run tauri dev)."));
      const name = get().project.assets.find((a) => a.id === asset)?.name ?? "the audio";
      const picked = await openDialog({ multiple: false, title: `Where is “${name}”?`, filters: [{ name: "Audio", extensions: AUDIO_EXTENSIONS }] });
      if (typeof picked !== "string") return;
      if (await withReply("Loading the audio…", () => backend.locateAsset(asset, picked), "Could not use that file")) pushToast({ tone: "success", title: `Found “${name}”` });
    },
    startLesson: (id) => {
      if (!LESSONS.some((l) => l.id === id)) return;
      set({ lesson: { id, step: 0 }, welcomeOpen: false });
    },
    setLessonStep: (step) => set((s) => (s.lesson ? { lesson: { ...s.lesson, step: Math.max(0, step) } } : {})),
    endLesson: (finished) => {
      const id = get().lesson?.id;
      if (finished && id) {
        const lessonsDone = [...new Set([...get().lessonsDone, id])];
        saveLessonsDone(lessonsDone);
        const title = LESSONS.find((l) => l.id === id)?.title;
        pushToast({ tone: "success", title: `Lesson done: ${title}`, detail: "More lessons are on the start page (File › Start page)." });
        set({ lesson: null, lessonsDone });
      } else {
        set({ lesson: null });
      }
    },
    setKeymap: (keymap) => {
      saveKeymap(keymap);
      set({ keymap });
    },
    toggleTempoLane: () => set((s) => ({ tempoLane: !tempoLaneShown(s) })),
    zoom: (factor) => set((s) => ({ pxPerBeat: Math.min(160, Math.max(3, s.pxPerBeat * factor)) })),
    zoomToFit: (width, lengthBeats) => set({ pxPerBeat: Math.min(160, Math.max(3, width / lengthBeats)) }),

    selectTrack: (id) => set({ selectedTrackId: id, chainOwner: null, trackSelection: [id] }),
    selectChain: (chainOwner) => set({ chainOwner }),
    selectClip: (trackId, clipId) => {
      if (get().selectedClipId !== clipId) set({ editorSelection: null, editorCursor: 0, editorView: null, pianoSelection: [], pianoZoom: null });
      set({ selectedTrackId: trackId, selectedClipId: clipId, selectedMarkerId: null, chainOwner: null });
    },
    setEditorSelection: (range) => set({ editorSelection: range }),
    setEditorCursor: (editorCursor) => set({ editorCursor }),
    setEditorView: (editorView) => set({ editorView }),

    zoomEditor: (factor, anchor) => {
      const s = get();
      const target = editorTarget(s);
      if (!target) return;
      const length = target.clip.duration;
      const rate = s.project.assets.find((a) => a.id === target.clip.asset)?.sampleRate ?? 48_000;
      const view = s.editorView ?? { start: 0, end: length };
      const span = view.end - view.start;
      // Deepest zoom: about 32 samples across the editor.
      const next = Math.min(length, Math.max(32 / rate, span / factor));
      const at = anchor ?? (view.start + view.end) / 2;
      const start = Math.min(length - next, Math.max(0, at - ((at - view.start) / span) * next));
      set({ editorView: next >= length - 1e-9 ? null : { start, end: start + next } });
    },

    zoomEditorToSelection: () => {
      const sel = selectionOf(get());
      if (sel) set({ editorView: { start: sel.start, end: sel.end } });
    },

    setEditorDisplay: (editorDisplay) => {
      writePref("editorDisplay", editorDisplay);
      set({ editorDisplay });
    },
    setSpectrumScale: (spectrumScale) => {
      writePref("spectrumScale", spectrumScale);
      set({ spectrumScale });
    },
    toggleZeroCross: () => {
      const zeroCross = !get().zeroCross;
      writePref("zeroCross", zeroCross ? "on" : "off");
      set({ zeroCross });
    },

    updateTrack: (id, patch) => void get().run({ op: "setTrack", id, patch }),
    updateReturn: (id, patch) => void get().run({ op: "setReturn", id, patch }),
    setMasterVolume: (volume) => void get().run({ op: "setMaster", volume }),
    setParam: (track, device, param, value) => void get().run({ op: "setParam", track, device, param, value }),
    addTrack: (kind) => void get().run({ op: "addTrack", kind }),
    removeTrack: (id) => void get().run({ op: "removeTrack", id }),

    addDevice: (owner, type) => {
      const s = get();
      const host = chainHost(s.project, owner);
      const spec = deviceSpec(type);
      if (!host || !spec) return;
      if (spec.category === "modulator") {
        if (host.kind === "return" || host.kind === "master") {
          pushToast({ tone: "info", title: `${spec.name} goes on a track`, detail: "Modulators move a knob of another device on the same track." });
          return;
        }
      } else if (spec.category !== "audio-fx" && host.kind !== "midi") {
        const what = spec.role === "instrument" ? "Instruments play notes" : "MIDI effects change notes";
        pushToast({ tone: "info", title: `${spec.name} needs a MIDI track`, detail: `${what}; pick a MIDI track or add one from the Track menu.` });
        return;
      }
      void s.run({ op: "addDevice", track: owner, device: createDevice(type) }).then(() => {
        pushToast({ tone: "success", title: `Added ${spec.name} to ${host.name}` });
      });
    },

    setSidechain: (track, device, source) => void get().run({ op: "setSidechain", track, device, source }),

    loadPlugins: async (rescan) => {
      if (get().scanningPlugins) return;
      set({ scanningPlugins: true });
      try {
        const list = await backend.pluginsList(rescan);
        set({ pluginList: list });
        if (rescan) {
          const found = `${list.plugins.length} ${list.plugins.length === 1 ? "plugin" : "plugins"}`;
          pushToast({ tone: list.failed.length ? "warning" : "success", title: `Found ${found}`, detail: list.failed.length ? `${list.failed.length} could not be loaded (Settings lists them).` : undefined });
        }
      } catch (err) {
        fail("Could not look for plugins")(err);
      } finally {
        set({ scanningPlugins: false });
      }
    },

    setPluginFolders: async (folders) => {
      set({ scanningPlugins: true });
      try {
        set({ pluginList: await backend.pluginsSetFolders(folders) });
      } catch (err) {
        fail("Could not change the plugin folders")(err);
      } finally {
        set({ scanningPlugins: false });
      }
    },

    addPluginFolder: async () => {
      const picked = await openDialog({ directory: true, title: "A folder with CLAP plugins" });
      if (typeof picked !== "string") return;
      const folders = get().pluginList?.folders ?? [];
      if (!folders.includes(picked)) await get().setPluginFolders([...folders, picked]);
    },

    addPlugin: async (owner, plugin) => {
      const host = chainHost(get().project, owner);
      if (!host) return;
      if (plugin.instrument && host.kind !== "midi") {
        pushToast({ tone: "info", title: `${plugin.name} needs a MIDI track`, detail: "Instruments play notes; pick a MIDI track or add one from the Track menu." });
        return;
      }
      const reply = await withReply(`Starting ${plugin.name}…`, () => backend.pluginAdd(owner, plugin.path, plugin.id), `Could not add ${plugin.name}`);
      if (reply) pushToast({ tone: "success", title: `Added ${plugin.name} to ${host.name}`, detail: "It runs in a sandbox of its own: if it crashes, Soniq carries on." });
    },

    pluginWindow: async (device, show) => {
      await withReply(null, () => backend.pluginWindow(device, show), show ? "Could not open the plugin's window" : "Could not close the plugin's window");
    },

    reloadPlugin: async (device) => {
      await withReply(null, () => backend.pluginReload(device), "Could not start the plugin again");
    },

    setPluginControls: (track, device, params) => void get().run({ op: "setPluginParams", track, device, params }),

    freezeTrack: async (track) => {
      const name = get().project.tracks.find((t) => t.id === track)?.name ?? "the track";
      const reply = await withReply(`Freezing ${name}…`, () => backend.trackFreeze(track), `Could not freeze ${name}`);
      if (reply) pushToast({ tone: "success", title: `Froze ${name}`, detail: "It plays its rendered sound while its devices rest. Unfreeze it to change them." });
    },
    unfreezeTrack: (track) => void get().run({ op: "unfreezeTrack", track }),
    flattenTrack: (track) => void get().run({ op: "flattenTrack", track }),
    renderClip: async (clip) => {
      const reply = await withReply("Rendering the clip…", () => backend.clipRender(clip), "Could not render the clip");
      if (reply) pushToast({ tone: "success", title: "Rendered to a new audio track", detail: reply.note ?? undefined });
    },

    removeDevice: (track, device) => void get().run({ op: "removeDevice", track, device }),
    toggleDevice: (track, device) => void get().run({ op: "toggleDevice", track, device }),

    deleteSelectedClip: () => {
      const clip = get().selectedClipId;
      if (clip) void get().run({ op: "deleteClip", clip });
    },

    moveClip: (clip, track, start) => void get().run({ op: "moveClip", clip, track, start }),
    trimClip: (clip, start, length) => void get().run({ op: "trimClip", clip, start, length }),
    splitClip: (clip, at) => void get().run({ op: "splitClip", clip, at }),
    setClipFades: (clip, fadeIn, fadeOut) => void get().run({ op: "setClipFades", clip, fadeIn, fadeOut }),

    splitAtPlayhead: () => {
      const s = get();
      const at = positionBeats(s);
      const tempo = tempoOf(s.project);
      const inside = (c: Track["clips"][number]) => c.slot == null && c.start < at && at < c.start + clipLength(c, tempo);
      const selected = s.project.tracks.flatMap((t) => t.clips).find((c) => c.id === s.selectedClipId && inside(c));
      const onTrack = s.project.tracks.find((t) => t.id === s.selectedTrackId)?.clips.find(inside);
      const clip = selected ?? onTrack;
      if (!clip) {
        pushToast({ tone: "info", title: "Nothing to split here", detail: "Put the playhead inside a clip on the selected track." });
        return;
      }
      s.splitClip(clip.id, at);
    },

    duplicateSelectedClip: () => {
      const clip = get().selectedClipId;
      if (clip) void get().run({ op: "duplicateClip", clip });
      else pushToast({ tone: "info", title: "Select a clip to duplicate" });
    },

    edit: async (op) => {
      const s = get();
      const target = editorTarget(s);
      if (!target) {
        pushToast({ tone: "info", title: "No audio clip to edit", detail: "Import audio, or select an audio clip in the arrangement." });
        return;
      }
      const label = editLabel(op);
      const sel = s.editorSelection && s.editorSelection.end - s.editorSelection.start > 0.001 ? s.editorSelection : null;
      const reply = await withReply(label, () => backend.edit(target.clip.id, op, sel?.start, sel?.end, s.zeroCross), `${label} failed`);
      if (!reply) return;
      const where = sel ? "the selection" : `all of “${target.clip.name}”`;
      if (reply.clipped) pushToast({ tone: "warning", title: `${label}: peaks above 0 dBFS`, detail: "Floats keep them, but they will clip on export. Normalize to fix." });
      else pushToast({ tone: "success", title: `${label} applied to ${where}` });
    },

    editorClipboard: async (action) => {
      const s = get();
      const target = editorTarget(s);
      if (!target) return;
      const sel = s.editorSelection && s.editorSelection.end - s.editorSelection.start > 0.001 ? s.editorSelection : null;
      if (action !== "paste" && !sel) {
        pushToast({ tone: "info", title: "Select some audio first", detail: "Drag across the waveform." });
        return;
      }
      const start = sel?.start ?? s.editorCursor;
      const end = sel?.end ?? s.editorCursor;
      const labels: Record<ClipboardAction, string> = { copy: "Copied", cut: "Cut", delete: "Deleted", paste: "Pasted" };
      const reply = await withReply(null, () => backend.clipboard(target.clip.id, action, start, end, s.zeroCross), `${labels[action]} failed`);
      if (!reply) return;
      if (action === "copy" || action === "cut") set({ hasClipboard: true });
      if (action !== "copy") set({ editorSelection: null, editorCursor: start });
      const length = sel ? ` ${formatClock(sel.end - sel.start)}` : "";
      pushToast({ tone: "success", title: action === "paste" ? "Pasted" : `${labels[action]}${length}` });
    },

    trimToSelection: () => {
      const s = get();
      const target = editorTarget(s);
      const sel = s.editorSelection;
      if (!target || !sel || sel.end - sel.start < 0.001) {
        pushToast({ tone: "info", title: "Select the audio to keep", detail: "Trim removes everything outside the selection." });
        return;
      }
      // Seconds into the clip's audio → timeline beats (warping and the tempo track included).
      const tempo = tempoOf(s.project);
      const at = (sec: number) => beatAfter(target.clip, tempo, target.clip.start, sec);
      s.trimClip(target.clip.id, at(sel.start), at(sel.end) - at(sel.start));
      set({ editorSelection: null, editorCursor: 0 });
    },

    openEffect: (effectDialog) => {
      if (effectDialog && !editorTarget(get())) {
        pushToast({ tone: "info", title: "No audio clip to process", detail: "Import audio, or select an audio clip in the arrangement." });
        return;
      }
      if (!effectDialog) get().stopPreview();
      set({ effectDialog });
    },

    applyEffects: async (effects, label) => {
      const s = get();
      const target = editorTarget(s);
      if (!target || !effects.length) return false;
      const sel = selectionOf(s);
      const title = label ?? describeSteps(effects);
      s.stopPreview();
      const reply = await withReply(title, () => backend.effect(target.clip.id, effects, sel?.start, sel?.end, s.zeroCross, label), `${title} failed`);
      if (!reply) return false;
      // Length-changing effects move everything after the selection.
      set({ editorSelection: null });
      const where = sel ? "the selection" : `all of “${target.clip.name}”`;
      if (reply.clipped) pushToast({ tone: "warning", title: `${title}: peaks above 0 dBFS`, detail: "Floats keep them, but they will clip on export. Normalize to fix." });
      else pushToast({ tone: "success", title: `${title} applied to ${where}`, detail: reply.note ?? undefined });
      return true;
    },

    previewEffects: async (effects) => {
      const s = get();
      const target = editorTarget(s);
      if (!target) return;
      if (s.playing) s.setPlaying(false);
      const sel = selectionOf(s);
      try {
        await backend.previewEffect(target.clip.id, effects, sel?.start, sel?.end);
        set({ previewing: true });
      } catch (err) {
        fail("Preview failed")(err);
      }
    },

    stopPreview: () => {
      if (!get().previewing) return;
      set({ previewing: false });
      void backend.stopPreview();
    },

    setPreviewing: (previewing) => set({ previewing }),

    captureNoiseProfile: async () => {
      const s = get();
      const target = editorTarget(s);
      const sel = selectionOf(s);
      if (!target || !sel) {
        pushToast({ tone: "info", title: "Select only noise first", detail: "Drag across a stretch with no speech or music: room tone, hiss or hum." });
        return;
      }
      try {
        const seconds = await backend.captureNoiseProfile(target.clip.id, sel.start, sel.end);
        set({ noiseProfileSeconds: seconds, editorSelection: null });
        pushToast({
          tone: seconds < 0.5 ? "warning" : "success",
          title: `Noise profile learned from ${formatClock(seconds)}`,
          detail: seconds < 0.5 ? "Half a second or more gives a steadier result." : "Now select what to clean (or nothing, for the whole clip) and apply.",
        });
      } catch (err) {
        fail("Could not learn the noise")(err);
      }
    },

    spectralEdit: async (mode) => {
      const s = get();
      const target = editorTarget(s);
      const sel = s.editorSelection;
      if (!target || !sel || sel.lowHz === undefined || sel.highHz === undefined) {
        pushToast({ tone: "info", title: "Select a frequency band first", detail: "In the spectrogram, drag a box around the sound to change." });
        return;
      }
      const gain = mode === "attenuate" ? -12 : -120;
      const labels: Record<SpectralMode, string> = { delete: "Deleted the band", attenuate: "Lowered the band by 12 dB", isolate: "Kept only the band" };
      const reply = await withReply(
        "Spectral edit…",
        () => backend.spectral(target.clip.id, sel.start, sel.end, sel.lowHz ?? 0, sel.highHz ?? 0, gain, mode === "isolate"),
        "Spectral edit failed",
      );
      if (reply) pushToast({ tone: "success", title: labels[mode], detail: `${Math.round(sel.lowHz)}–${Math.round(sel.highHz)} Hz, ${formatClock(sel.end - sel.start)}` });
    },

    setGeneratorOpen: (generatorOpen) => set({ generatorOpen }),

    generate: async (generator, into) => {
      const s = get();
      const target = editorTarget(s);
      const sel = selectionOf(s);
      const track = s.project.tracks.find((t) => t.id === s.selectedTrackId && t.kind === "audio");
      const reply = await withReply(
        "Generating…",
        () =>
          into === "clip" && target
            ? backend.generate(generator, { kind: "clip", clip: target.clip.id, start: sel?.start ?? s.editorCursor, end: sel?.end ?? s.editorCursor })
            : backend.generate(generator, { kind: "track", track: track?.id ?? null, at: positionBeats(s) }),
        "Could not generate",
      );
      if (!reply) return false;
      if (into === "clip") set({ editorSelection: null });
      pushToast({ tone: "success", title: into === "clip" ? `Inserted into “${target?.clip.name}”` : track ? `Added to ${track.name}` : "Added on a new track" });
      return true;
    },

    drawSamples: async (channel, first, samples) => {
      const target = editorTarget(get());
      if (!target || !samples.length) return;
      await withReply(null, () => backend.draw(target.clip.id, channel, first, samples), "Drawing failed");
    },

    setMacroDialog: (macroDialog) => set({ macroDialog }),

    loadMacros: async () => {
      const user = await backend.macros().catch(() => [] as Macro[]);
      set({ macros: [...BUILTIN_MACROS, ...user] });
    },

    saveMacros: async (userMacros) => {
      try {
        await backend.saveMacros(userMacros.filter((m) => !m.builtIn));
        set({ macros: [...BUILTIN_MACROS, ...userMacros.filter((m) => !m.builtIn)] });
      } catch (err) {
        fail("Could not save the macros")(err);
      }
    },

    runBatch: async (files, steps, output, onProgress) => {
      set({ busy: `Processing ${files.length} files…` });
      try {
        return await backend.batch(files, steps, output, onProgress);
      } catch (err) {
        fail("Batch processing failed")(err);
        return null;
      } finally {
        set({ busy: null });
      }
    },

    togglePunch: () => {
      const s = get();
      if (!s.punch && s.loop.end - s.loop.start < 0.25) {
        pushToast({ tone: "info", title: "Set a loop range first", detail: "Punch recording keeps only what you play inside it. Shift-drag the ruler." });
        return;
      }
      set({ punch: !s.punch });
    },

    addMarker: () => void get().run({ op: "addMarker", start: positionBeats(get()) }),

    addRegionFromLoop: () => {
      const { loop } = get();
      void get().run({ op: "addMarker", start: loop.start, end: loop.end });
    },

    selectMarker: (selectedMarkerId) => set(selectedMarkerId ? { selectedMarkerId, selectedClipId: null } : { selectedMarkerId }),
    moveMarker: (id, start, end) => void get().run({ op: "setMarker", id, start, end }),
    renameMarker: (id, name) => void get().run({ op: "setMarker", id, name }),
    removeMarker: (id) => void get().run({ op: "removeMarker", id }),

    setExportOpen: (exportOpen, preset) => set({ exportOpen, exportPreset: exportOpen ? (preset ?? null) : null }),

    exportMix: async (options, mode) => {
      if (!inTauri) {
        fail("Export is not available")(new Error("Export needs the desktop app (npm run tauri dev)."));
        return;
      }
      const s = get();
      const ext = options.format;
      const showReport = (report: ExportReport) => {
        const loud = report.loudness !== null ? ` · ${report.loudness.toFixed(1)} LUFS` : "";
        pushToast({
          tone: report.heldBack || report.peakDb > 0 ? "warning" : "success",
          title: `Exported ${report.path.split(/[\\/]/).pop()}`,
          detail:
            `${formatClock(report.seconds)}${loud} · peak ${report.peakDb.toFixed(1)} dBFS` +
            (report.limited ? " · the limiter held the peaks" : "") +
            (report.chapters ? ` · ${report.chapters} ${report.chapters === 1 ? "chapter" : "chapters"}${report.chaptersFile ? ", and a chapters file beside it" : ""}` : "") +
            (report.heldBack ? " · held back to keep peaks under −1 dBFS: switch on the limiter for more" : "") +
            (report.peakDb > 0 ? " · the mix clips: lower the master" : ""),
          actions: [{ label: "Show in folder", run: () => void revealItemInDir(report.path) }],
        });
      };

      if (mode === "song" || mode === "loop") {
        const path = await saveDialog({ defaultPath: `${safeName(s.project.name)}.${ext}`, filters: [{ name: FORMAT_NAMES[ext], extensions: [ext] }] });
        if (!path) return;
        set({ busy: "Exporting…", exportOpen: false });
        try {
          showReport(await backend.exportMix({ ...options, path }));
        } catch (err) {
          fail("Export failed")(err);
        } finally {
          set({ busy: null });
        }
        return;
      }

      // One file per region or per track, into a folder.
      const seconds = (beats: number) => beatsToSec(beats, s.project);
      const jobs =
        mode === "regions"
          ? s.project.markers
              .filter((m) => m.end !== undefined)
              .map((m) => ({ name: m.name, start: seconds(m.start), end: seconds(m.end ?? m.start), track: undefined as string | undefined }))
          : s.project.tracks.filter((t) => timelineClips(t).length > 0).map((t) => ({ name: t.name, start: options.start, end: options.end, track: t.id as string | undefined }));
      if (!jobs.length) {
        pushToast({ tone: "info", title: mode === "regions" ? "No regions to export" : "No tracks with clips" });
        return;
      }
      const folder = await openDialog({ directory: true, title: mode === "regions" ? "Export each region into…" : "Export stems into…" });
      if (typeof folder !== "string") return;
      set({ exportOpen: false });
      const sep = folder.includes("\\") ? "\\" : "/";
      const written: string[] = [];
      try {
        for (const [i, job] of jobs.entries()) {
          set({ busy: `Exporting ${i + 1} of ${jobs.length}…` });
          const path = `${folder.replace(/[\\/]$/, "")}${sep}${String(i + 1).padStart(2, "0")} ${safeName(job.name)}.${ext}`;
          const metadata = { ...options.metadata, title: options.metadata.title || job.name, track: options.metadata.track || String(i + 1) };
          const report = await backend.exportMix({ ...options, metadata, path, start: job.start, end: job.end, track: job.track });
          written.push(report.path);
        }
        pushToast({
          tone: "success",
          title: `Exported ${written.length} ${mode === "regions" ? "regions" : "stems"}`,
          detail: folder,
          actions: [{ label: "Show in folder", run: () => void revealItemInDir(written[0]) }],
        });
      } catch (err) {
        fail(`Export stopped after ${written.length} of ${jobs.length}`)(err);
      } finally {
        set({ busy: null });
      }
    },

    setPianoSelection: (pianoSelection) => set({ pianoSelection }),
    setPianoGrid: (pianoGrid) => set({ pianoGrid }),
    setPianoRows: (pianoRows) => {
      writePref("pianoRows", pianoRows);
      set({ pianoRows });
    },
    toggleSnapToScale: () => {
      const snapToScale = !get().snapToScale;
      writePref("snapToScale", snapToScale ? "on" : "off");
      set({ snapToScale });
    },
    setQuantizeSettings: (quantizeSettings) => set({ quantizeSettings }),
    setPianoZoom: (pianoZoom) => set({ pianoZoom }),
    togglePianoGhosts: () => {
      const pianoGhosts = !get().pianoGhosts;
      writePref("pianoGhosts", pianoGhosts ? "on" : "off");
      set({ pianoGhosts });
    },
    setNoteTool: (noteTool) => set({ noteTool }),
    setPianoMode: (pianoMode) => {
      writePref("pianoMode", pianoMode);
      set({ pianoMode });
    },

    duplicateLinked: () => {
      const s = get();
      const clip = s.project.tracks.flatMap((t) => t.clips).find((c) => c.id === s.selectedClipId);
      if (clip?.kind !== "midi") {
        pushToast({ tone: "info", title: "Select a MIDI clip", detail: "Linked copies share their notes: edit one and they all change." });
        return;
      }
      void s.run({ op: "duplicateClip", clip: clip.id, linked: true });
    },
    unlinkClip: (clip) => void get().run({ op: "unlinkClip", clip }),

    setNotes: async (clip, notes, label, selection) => {
      // Show the result at once; the backend's reply confirms it.
      if (selection) set({ pianoSelection: selection });
      try {
        get().applyReply(await backend.apply({ op: "setNotes", clip, notes, label }));
      } catch (err) {
        pushToast({ tone: "info", title: message(err) });
      }
    },

    editNotes: (edit, label) => {
      const s = get();
      const target = pianoTarget(s);
      if (!target) return;
      const result = edit(target.clip.notes, s.pianoSelection.filter((i) => i < target.clip.notes.length));
      // An edit that changes nothing (quantizing notes already on the grid) makes no undo step.
      if (JSON.stringify(result.notes) === JSON.stringify(target.clip.notes)) {
        set({ pianoSelection: result.selection });
        return;
      }
      void s.setNotes(target.clip.id, result.notes, label, result.selection);
    },

    pianoCopy: () => {
      const s = get();
      const target = pianoTarget(s);
      if (!target || !s.pianoSelection.length) return;
      set({ noteClipboard: copyOf(target.clip.notes, s.pianoSelection) });
    },

    pianoCut: () => {
      get().pianoCopy();
      get().editNotes(remove, "Cut notes");
    },

    pianoPaste: () => {
      const s = get();
      const target = pianoTarget(s);
      if (!target || !s.noteClipboard.length) return;
      // At the playhead when it is inside the clip, else at the clip start.
      const local = positionBeats(s) - target.clip.start;
      const at = local >= 0 && local < target.clip.length ? snap(local, s.pianoGrid || 0.25) : 0;
      s.editNotes((notes) => paste(notes, s.noteClipboard, at), "Paste notes");
    },

    pianoDelete: () => {
      if (get().pianoSelection.length) get().editNotes(remove, "Delete notes");
    },

    pianoSelectAll: () => {
      const target = pianoTarget(get());
      if (target) set({ pianoSelection: target.clip.notes.map((_, i) => i) });
    },

    newMidiClip: async (trackId, start, length, notes, name) => {
      const s = get();
      const track = s.project.tracks.find((t) => t.id === (trackId ?? s.selectedTrackId) && t.kind === "midi") ?? s.project.tracks.find((t) => t.kind === "midi");
      if (!track) {
        pushToast({ tone: "info", title: "Add a MIDI track first", detail: "Track › New MIDI track (Ctrl Shift T)." });
        return null;
      }
      const bar = s.project.beatsPerBar;
      const at = start ?? Math.floor(positionBeats(s) / bar) * bar;
      try {
        const reply = await backend.apply({ op: "addMidiClip", track: track.id, start: at, length: length ?? bar, notes, name });
        get().applyReply(reply);
        if (reply.created) set({ selectedClipId: reply.created, selectedTrackId: track.id, pianoSelection: [] });
        return reply.created;
      } catch (err) {
        pushToast({ tone: "info", title: message(err) });
        return null;
      }
    },

    openPianoRoll: (clipId) => {
      const s = get();
      const owner = clipId ? s.project.tracks.find((t) => t.clips.some((c) => c.id === clipId)) : undefined;
      if (owner && clipId) s.selectClip(owner.id, clipId);
      set({ view: "piano" });
    },

    importMidi: async (paths, trackId, beats) => {
      const s = get();
      let files = paths;
      if (!files) {
        if (!inTauri) return fail("Import is not available")(new Error("Importing MIDI needs the desktop app (npm run tauri dev)."));
        const picked = await openDialog({ multiple: true, filters: [{ name: "MIDI file", extensions: MIDI_EXTENSIONS }] });
        if (!picked) return;
        files = Array.isArray(picked) ? picked : [picked];
      }
      if (!files.length) return;
      const selected = s.project.tracks.find((t) => t.id === s.selectedTrackId);
      const track = trackId ?? (selected?.kind === "midi" && files.length === 1 ? selected.id : undefined);
      const reply = await withReply("Importing MIDI…", () => backend.midiImport(files, track, beats ?? positionBeats(s)), "MIDI import failed");
      if (!reply?.created) return;
      pushToast({ tone: "success", title: files.length === 1 ? "Imported the MIDI file" : `Imported ${files.length} MIDI files`, detail: reply.note ?? undefined });
    },

    exportMidi: async (clipId) => {
      if (!inTauri) return fail("Export is not available")(new Error("Export needs the desktop app (npm run tauri dev)."));
      const s = get();
      const clip = clipId ? s.project.tracks.flatMap((t) => t.clips).find((c) => c.id === clipId) : undefined;
      const path = await saveDialog({ defaultPath: `${safeName(clip?.name ?? s.project.name)}.mid`, filters: [{ name: "MIDI file", extensions: ["mid"] }] });
      if (!path) return;
      try {
        const what = await backend.midiExport(path, clipId);
        pushToast({
          tone: "success",
          title: `Exported ${path.split(/[\\/]/).pop()}`,
          detail: what,
          actions: [{ label: "Show in folder", run: () => void revealItemInDir(path) }],
        });
      } catch (err) {
        fail("MIDI export failed")(err);
      }
    },

    loadSample: async (trackId, deviceId, key, path) => {
      let file = path;
      if (!file) {
        if (!inTauri) return fail("Loading samples is not available")(new Error("Samples load in the desktop app (npm run tauri dev)."));
        const picked = await openDialog({ multiple: false, filters: [{ name: "Audio", extensions: AUDIO_EXTENSIONS }] });
        if (typeof picked !== "string") return;
        file = picked;
      }
      const target = file;
      const reply = await withReply("Loading the sample…", () => backend.loadSample(trackId, deviceId, key, target), "Could not load the sample");
      if (reply) pushToast({ tone: "success", title: `Loaded ${target.split(/[\\/]/).pop()}` });
    },

    clearSample: (track, device, key) => void get().run({ op: "setDeviceSample", track, device, key, asset: null }),

    learnMidi: (target, label = "it") => {
      if (target && !inTauri) {
        pushToast({ tone: "info", title: "MIDI learn needs the desktop app", detail: "Run npm run tauri dev and plug in a MIDI controller." });
        return;
      }
      set({ midiLearn: target ? { target, label } : null });
      engine.midiLearn(target);
      if (target) {
        pushToast({
          tone: "info",
          title: "Move a knob, fader or button on your MIDI controller",
          detail: `It will turn ${label}.`,
          actions: [{ label: "Cancel", run: () => get().learnMidi(null) }],
        });
      }
    },

    refreshProject: async () => {
      try {
        get().applyReply(await backend.load());
      } catch (err) {
        console.error("Could not fetch the project", err);
        return;
      }
      const learn = get().midiLearn;
      const learned = learn && get().project.mappings?.find((m) => sameTarget(m.target, learn.target));
      if (learn && learned) {
        set({ midiLearn: null });
        pushToast({ tone: "success", title: `CC ${learned.cc} now turns ${learn.label}`, detail: "Settings › MIDI lists every mapping." });
      }
    },

    toggleComputerKeys: () => {
      const computerKeys = !get().computerKeys;
      writePref("computerKeys", computerKeys ? "on" : "off");
      set({ computerKeys });
      if (computerKeys) {
        pushToast({
          tone: "info",
          title: "Computer keyboard plays notes",
          detail: "A S D F G H J K play the white keys, W E T Y U the black ones. Z and X change the octave, C and V the velocity.",
        });
      }
    },
    setMidiRecordMode: (midiRecordMode) => {
      writePref("midiRecordMode", midiRecordMode);
      set({ midiRecordMode });
    },

    captureMidi: async () => {
      if (!inTauri) return fail("Capture MIDI is not available")(new Error("Capture MIDI needs the desktop app (npm run tauri dev)."));
      try {
        const reply = await backend.midiCapture();
        get().applyReply(reply);
        pushToast({ tone: "success", title: "Captured what you played", detail: reply.note ?? undefined });
      } catch (err) {
        pushToast({ tone: "info", title: message(err) });
      }
    },
    setKeysOctave: (octave) => set({ keysOctave: Math.min(8, Math.max(-1, octave)) }),
    setKeysVelocity: (velocity) => set({ keysVelocity: Math.min(127, Math.max(1, velocity)) }),

    confirmDiscard: async () => {
      const s = get();
      if (!s.dirty) return true;
      const text = `“${s.project.name}” has unsaved changes. Discard them?`;
      if (!inTauri) return window.confirm(text);
      return ask(text, { title: "Unsaved changes", kind: "warning", okLabel: "Discard", cancelLabel: "Cancel" });
    },

    newProject: async (template) => {
      if (!(await get().confirmDiscard())) return false;
      if (!(await withReply(null, () => backend.newProject(template), "Could not create a project"))) return false;
      resetForNewSong();
      const view = get().templates.find((t) => t.id === template)?.view;
      // The podcast template opens with its tools.
      set({ welcomeOpen: false, ...(view ? { view } : {}), ...(template === "podcast" ? { podcastOpen: true } : {}) });
      return true;
    },

    openProject: async (path) => {
      if (!(await get().confirmDiscard())) return;
      let file = path;
      if (!file) {
        if (!inTauri) return fail("Open is not available")(new Error("Opening files needs the desktop app (npm run tauri dev)."));
        const picked = await openDialog({ multiple: false, filters: PROJECT_FILTER });
        if (typeof picked !== "string") return;
        file = picked;
      }
      const target = file;
      const reply = await withReply("Opening…", () => backend.open(target), "Could not open the project");
      if (!reply) return void get().loadStart();
      resetForNewSong();
      set({ welcomeOpen: false });
      void get().loadStart();
      const missing = reply.project.assets.filter((a) => a.missing).length;
      if (missing)
        pushToast({
          tone: "warning",
          title: `${missing} audio ${missing === 1 ? "file is" : "files are"} missing`,
          detail: "Their clips play silent until the audio is found.",
          actions: [{ label: "Find missing files…", run: () => get().setMissingOpen(true) }],
        });
    },

    openDemo: async () => {
      if (!(await get().confirmDiscard())) return;
      if (await withReply("Rendering the demo…", () => backend.openDemo(), "Could not open the demo")) {
        resetForNewSong();
        set({ welcomeOpen: false });
      }
    },

    saveProject: async () => {
      const s = get();
      if (!s.path) return s.saveProjectAs();
      const reply = await withReply("Saving…", () => backend.save(), "Could not save");
      if (reply) pushToast({ tone: "success", title: `Saved “${reply.project.name}”` });
      if (reply) void get().loadStart();
      return !!reply;
    },

    saveProjectAs: async () => {
      if (!inTauri) {
        fail("Save is not available")(new Error("Saving needs the desktop app (npm run tauri dev)."));
        return false;
      }
      const s = get();
      const path = await saveDialog({ defaultPath: `${s.project.name.replace(/[\\/:*?"<>|]/g, "_")}.soniq`, filters: PROJECT_FILTER });
      if (!path) return false;
      const reply = await withReply("Saving…", () => backend.save(path), "Could not save");
      if (reply) pushToast({ tone: "success", title: `Saved “${reply.project.name}”`, detail: "Audio is copied into the Samples folder next to it." });
      if (reply) void get().loadStart();
      return !!reply;
    },

    importAudio: async (paths, trackId, beats) => {
      const s = get();
      let files = paths;
      if (!files) {
        if (!inTauri) return fail("Import is not available")(new Error("Importing audio needs the desktop app (npm run tauri dev)."));
        const picked = await openDialog({ multiple: true, filters: [{ name: "Audio", extensions: AUDIO_EXTENSIONS }] });
        if (!picked) return;
        files = Array.isArray(picked) ? picked : [picked];
      }
      if (!files.length) return;
      const selected = s.project.tracks.find((t) => t.id === s.selectedTrackId);
      const track = trackId ?? (selected?.kind === "audio" && files.length === 1 ? selected.id : undefined);
      const start = beats ?? positionBeats(s);
      const count = files.length;
      const reply = await withReply(count === 1 ? "Importing…" : `Importing ${count} files…`, () => backend.importFiles(files, track, start), "Import failed");
      const imported = reply ? count - reply.warnings.length : 0;
      if (imported > 0) pushToast({ tone: "success", title: imported === 1 ? "Imported 1 file" : `Imported ${imported} files` });
    },

    checkRecovery: async () => {
      const orphans = await backend.orphanedRecordings().catch(() => [] as string[]);
      if (orphans.length) {
        const id = toastId;
        pushToast({
          tone: "warning",
          title: orphans.length === 1 ? "An unfinished recording was found" : `${orphans.length} unfinished recordings were found`,
          detail: "Soniq stopped while recording. Everything up to the last half second was kept.",
          sticky: true,
          actions: [
            {
              label: "Import",
              run: () => {
                get().dismissToast(id);
                void backend.recoverRecordings().then((paths) => get().importAudio(paths, undefined, 0));
              },
            },
            {
              label: "Discard",
              run: () => {
                get().dismissToast(id);
                void backend.discardRecordings();
              },
            },
          ],
        });
      }
      if (!(await backend.recoveryAvailable().catch(() => false))) return;
      const id = toastId;
      pushToast({
        tone: "warning",
        title: "Unsaved work from your last session",
        detail: "Soniq closed before it was saved. Restore it now, or discard it for good.",
        sticky: true,
        actions: [
          {
            label: "Restore",
            run: () => {
              get().dismissToast(id);
              void withReply("Restoring…", () => backend.restoreRecovery(), "Could not restore").then((r) => r && resetForNewSong());
            },
          },
          {
            label: "Discard",
            run: () => {
              get().dismissToast(id);
              void backend.discardRecovery();
            },
          },
        ],
      });
    },

    launchSlot: (trackId, scene) => {
      if (!inTauri) {
        if (!get().playing) get().setPlaying(true);
        return simulate(trackId, scene);
      }
      pending(trackId, scene);
      engine.launch(trackId, scene).then(followEngineStart, fail("Could not launch the clip"));
    },

    launchScene: (scene) => {
      const s = get();
      const tracks = s.project.tracks.filter((t) => t.kind !== "group");
      if (!inTauri) {
        if (!s.playing) s.setPlaying(true);
        return tracks.forEach((t) => simulate(t.id, slotClip(t, scene) ? scene : null));
      }
      tracks.forEach((t) => pending(t.id, slotClip(t, scene) ? scene : null));
      engine.launchScene(scene).then(followEngineStart, fail("Could not launch the scene"));
    },

    stopTrack: (trackId) => {
      if (!inTauri) return simulate(trackId, null);
      pending(trackId, null);
      engine.launch(trackId, null).catch(fail("Could not stop the track"));
    },

    stopAllClips: () => {
      const s = get();
      const active = Object.entries(s.clipStates).filter(([, c]) => c.playing !== null || c.queued !== null);
      if (!inTauri) return active.forEach(([id]) => simulate(id, null));
      active.forEach(([id]) => pending(id, null));
      engine.stopClips().catch(fail("Could not stop the clips"));
    },

    backToArrangement: () => {
      if (!inTauri) return set({ clipStates: {} });
      engine.backToArrangement().catch(fail("Could not go back to the arrangement"));
    },

    setClipStates: (clipStates) => set({ clipStates }),

    newSlotClip: async (trackId, scene) => {
      const s = get();
      const length = s.project.beatsPerBar;
      try {
        const reply = await backend.apply({ op: "addMidiClip", track: trackId, start: 0, length, slot: scene });
        get().applyReply(reply);
        if (reply.created) get().selectClip(trackId, reply.created);
      } catch (err) {
        pushToast({ tone: "info", title: message(err) });
      }
    },

    putInSlot: (clip, scene) => void get().run({ op: "putInSlot", clip, scene }),
    clearSlot: (track, scene) => void get().run({ op: "clearSlot", track, scene }),
    setFollow: (clip, follow) => void get().run({ op: "setFollow", clip, follow }),
    setLaunchQuantize: (beats) => void get().run({ op: "setLaunchQuantize", beats }),
    addScene: () => void get().run({ op: "addScene" }),

    toggleTrackSelection: (id) =>
      set((s) => {
        const picked = s.trackSelection.includes(id) ? s.trackSelection.filter((t) => t !== id) : [...s.trackSelection, id];
        return { trackSelection: picked, selectedTrackId: id, chainOwner: null };
      }),

    groupTracks: (ids) => {
      const s = get();
      const picked = ids ?? (s.trackSelection.length ? s.trackSelection : [s.selectedTrackId]);
      const tracks = s.project.tracks.filter((t) => picked.includes(t.id) && t.kind !== "group").map((t) => t.id);
      if (!tracks.length) {
        pushToast({ tone: "info", title: "Pick tracks to group", detail: "Ctrl-click track headers to pick several, then press Ctrl G." });
        return;
      }
      void s.run({ op: "groupTracks", tracks });
    },

    ungroup: (group) => void get().run({ op: "ungroup", group }),
    setTrackGroup: (track, group) => void get().run({ op: "setTrackGroup", track, group }),
    toggleFold: (group) => set((s) => ({ folded: s.folded.includes(group) ? s.folded.filter((g) => g !== group) : [...s.folded, group] })),

    toggleAutomation: (trackId) =>
      set((s) => {
        const automationOpen = { ...s.automationOpen };
        if (automationOpen[trackId]) delete automationOpen[trackId];
        else automationOpen[trackId] = [];
        return { automationOpen };
      }),

    showAutomation: (trackId, target) =>
      set((s) => {
        const shown = s.automationOpen[trackId] ?? [];
        const extra = shown.some((t) => sameAutoTarget(t, target)) ? shown : [...shown, target];
        return { automationOpen: { ...s.automationOpen, [trackId]: extra }, view: s.view === "arrange" ? s.view : "arrange" };
      }),

    hideAutomationLane: (trackId, target) =>
      set((s) => ({ automationOpen: { ...s.automationOpen, [trackId]: (s.automationOpen[trackId] ?? []).filter((t) => !sameAutoTarget(t, target)) } })),

    setAutomation: (track, target, points) => void get().run({ op: "setAutomation", track, target, points }),
    setModTarget: (track, device, target) => void get().run({ op: "setModTarget", track, device, target }),

    undo: async () => {
      const label = get().history.undoLabel;
      if (!label) return;
      if (await withReply(null, () => backend.undo(), "Undo failed")) pushToast({ tone: "info", title: `Undid ${label}` });
    },

    redo: async () => {
      const label = get().history.redoLabel;
      if (!label) return;
      if (await withReply(null, () => backend.redo(), "Redo failed")) pushToast({ tone: "info", title: `Redid ${label}` });
    },

    toast: pushToast,
    dismissToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
    setPaletteOpen: (paletteOpen) => set({ paletteOpen }),
    setSettingsOpen: (settingsOpen) => set({ settingsOpen }),
    setEngineStatus: (engineStatus) => set({ engineStatus }),
    setInputStatus: (inputStatus) => set({ inputStatus }),
  };
});

export function positionBeats(s: Pick<Store, "positionSec" | "project">): number {
  return secToBeats(s.positionSec, s.project);
}

/** Whether the arrangement shows the tempo lane: as asked, else when the song has tempo changes. */
export function tempoLaneShown(s: Pick<Store, "tempoLane" | "project">): boolean {
  return s.tempoLane ?? (s.project.tempo?.length ?? 0) > 0;
}

/** The MIDI clip the piano roll shows: the selected clip if it is MIDI, else the first MIDI clip around. */
export function pianoTarget(s: Pick<Store, "project" | "selectedTrackId" | "selectedClipId">): { track: Track; clip: MidiClip } | null {
  const tracks = s.project.tracks;
  const isMidi = (c: Track["clips"][number]): c is MidiClip => c.kind === "midi";
  for (const track of tracks) {
    const clip = track.clips.find((c): c is MidiClip => c.id === s.selectedClipId && isMidi(c));
    if (clip) return { track, clip };
  }
  const selected = tracks.find((t) => t.id === s.selectedTrackId);
  const own = selected?.clips.find(isMidi);
  if (selected && own) return { track: selected, clip: own };
  for (const track of tracks) {
    const clip = track.clips.find(isMidi);
    if (clip) return { track, clip };
  }
  return null;
}

/** The audio clip the editor works on: the selected clip if it is audio, else the first audio clip around. */
export function editorTarget(s: Pick<Store, "project" | "selectedTrackId" | "selectedClipId">): { track: Track; clip: AudioClip } | null {
  const tracks = s.project.tracks;
  const isAudio = (c: Track["clips"][number]): c is AudioClip => c.kind === "audio";
  for (const track of tracks) {
    const clip = track.clips.find((c): c is AudioClip => c.id === s.selectedClipId && isAudio(c));
    if (clip) return { track, clip };
  }
  const selected = tracks.find((t) => t.id === s.selectedTrackId);
  const own = selected?.clips.find(isAudio);
  if (selected && own) return { track: selected, clip: own };
  for (const track of tracks) {
    const clip = track.clips.find(isAudio);
    if (clip) return { track, clip };
  }
  return null;
}
