/*
 * The project backend. In the desktop app the Rust side owns the project,
 * the undo history and the audio pool; outside Tauri a small in-memory
 * stand-in (./previewBackend.ts) keeps the UI usable for design work.
 */
import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  BatchOutput,
  BatchProgress,
  BatchReport,
  ClipboardAction,
  EditOp,
  Effect,
  ExportOptions,
  ExportReport,
  ChaptersFile,
  CrashStatus,
  Audition,
  Device,
  LibraryView,
  MarkKind,
  SpectrumPlot,
  FixAnalysis,
  SpeakerLevel,
  FrequencyScale,
  GenerateTarget,
  Generator,
  Macro,
  MidiRecordMode,
  Op,
  PeakData,
  PluginList,
  PluginParamInfo,
  HistoryList,
  ProjectReply,
  RecentProject,
  TemplateInfo,
  WarpMode,
} from "../state/types";
import { inTauri } from "./bridge";
import { createPreviewBackend } from "./previewBackend";

/** Spectrogram bytes per channel, `columns × rows` from the top row; 0 = −100 dB, 255 = 0 dB. */
export interface SpectrogramData {
  channels: Uint8Array[];
  columns: number;
  rows: number;
  sampleRate: number;
}

/** Waveform detail: raw samples when zoomed in far enough, else (min, max) per column. */
export interface WaveWindow {
  channels: Float32Array[];
  count: number;
  raw: boolean;
  sampleRate: number;
  /** First frame of the asset covered. */
  first: number;
}

export interface Backend {
  load(): Promise<ProjectReply>;
  apply(op: Op): Promise<ProjectReply>;
  undo(): Promise<ProjectReply>;
  redo(): Promise<ProjectReply>;
  /** A new song from a template ("empty" without one). */
  newProject(template?: string): Promise<ProjectReply>;
  templates(): Promise<TemplateInfo[]>;
  /** Saves the song's setup (no clips or audio) as a template of the user's. */
  saveTemplate(name: string): Promise<TemplateInfo>;
  deleteTemplate(id: string): Promise<void>;
  recent(): Promise<RecentProject[]>;
  forgetRecent(path: string): Promise<RecentProject[]>;
  historyList(): Promise<HistoryList>;
  /** Crash reports kept on this computer, and whether they are on. */
  crashStatus(): Promise<CrashStatus>;
  crashEnable(on: boolean): Promise<void>;
  /** Every report so far has been seen. */
  crashSeen(): Promise<void>;
  crashRead(file: string): Promise<string>;
  crashDelete(file: string): Promise<void>;
  /** An error the interface could not handle; kept only while reports are on. */
  crashUi(message: string, stack?: string): Promise<string | null>;
  /** Undoes (below zero) or redoes that many steps at once. */
  historyJump(steps: number): Promise<ProjectReply>;
  /** Looks through a folder (and the folders in it) for the song's missing audio. */
  findAssets(folder: string): Promise<ProjectReply>;
  /** Points one asset at a file. */
  locateAsset(asset: string, path: string): Promise<ProjectReply>;
  openDemo(): Promise<ProjectReply>;
  open(path: string): Promise<ProjectReply>;
  /** Saves to `path`, or to the current file when omitted. */
  save(path?: string): Promise<ProjectReply>;
  importFiles(paths: string[], track?: string, start?: number): Promise<ProjectReply>;
  /** `start`/`end` are seconds from the clip start; omitted means the whole clip. */
  edit(clip: string, op: EditOp, start?: number, end?: number, zeroCross?: boolean): Promise<ProjectReply>;
  clipboard(clip: string, action: ClipboardAction, start?: number, end?: number, zeroCross?: boolean): Promise<ProjectReply>;
  /** Runs effects (one, or a macro's steps) over the selection or the whole clip. */
  effect(clip: string, effects: Effect[], start?: number, end?: number, zeroCross?: boolean, label?: string): Promise<ProjectReply>;
  /** Plays the processed selection; resolves to its length in seconds. */
  previewEffect(clip: string, effects: Effect[], start?: number, end?: number): Promise<number>;
  /** The average spectrum of the selection (or the whole clip). */
  spectrumPlot(clip: string, start: number | undefined, end: number | undefined, size: number): Promise<SpectrumPlot>;
  /** RMS level of a stretch of a clip, dBFS. */
  clipLevel(clip: string, start: number, end: number): Promise<number>;
  /** Finds clipping, beats or sounds and marks them on the timeline (one undo step). */
  clipMark(clip: string, start: number | undefined, end: number | undefined, kind: MarkKind): Promise<ProjectReply>;
  /** Redraws a short damaged stretch (seconds from the clip start) from the sound around it. */
  repair(clip: string, start: number, end: number): Promise<ProjectReply>;
  /** What Fix my audio finds in the selection (or the whole clip), and what it would do. */
  fixAnalyse(clip: string, start: number | undefined, end: number | undefined, target: number): Promise<FixAnalysis>;
  /** How loud each track's clips are. */
  podcastMeasure(tracks: string[]): Promise<SpeakerLevel[]>;
  /** Levels every clip on these tracks to `target` LUFS, in one undo step. */
  podcastLevel(tracks: string[], target: number): Promise<ProjectReply>;
  /** A volume lane on each music track that dips by `depth` dB under speech. */
  podcastDuck(music: string[], speakers: string[], depth: number): Promise<ProjectReply>;
  /** Writes the song's chapters (its markers) to a file; resolves to how many. */
  chaptersExport(path: string, format: ChaptersFile): Promise<number>;
  stopPreview(): Promise<void>;
  captureNoiseProfile(clip: string, start?: number, end?: number): Promise<number>;
  noiseProfile(): Promise<number | null>;
  spectral(clip: string, start: number, end: number, lowHz: number, highHz: number, gainDb: number, invert: boolean): Promise<ProjectReply>;
  spectrogram(asset: string, start: number, end: number, columns: number, rows: number, scale: FrequencyScale): Promise<SpectrogramData>;
  waveWindow(asset: string, start: number, end: number, columns: number): Promise<WaveWindow>;
  draw(clip: string, channel: number | null, first: number, samples: number[]): Promise<ProjectReply>;
  generate(generator: Generator, target: GenerateTarget): Promise<ProjectReply>;
  macros(): Promise<Macro[]>;
  saveMacros(macros: Macro[]): Promise<void>;
  batch(files: string[], steps: Effect[], output: BatchOutput, onProgress: (p: BatchProgress) => void): Promise<BatchReport>;
  peaks(asset: string): Promise<PeakData>;
  recoveryAvailable(): Promise<boolean>;
  restoreRecovery(): Promise<ProjectReply>;
  discardRecovery(): Promise<void>;
  closing(): Promise<void>;
  /** `punch` is the range in seconds that keeps what is played inside it; `midiMode` how MIDI lands on existing clips. */
  /** `pass` (sketch mode): keep one pass round this loop range, in seconds. */
  /** `slot` (track, scene): that track alone records, into that session slot. */
  recordStart(countIn: boolean, punch: [number, number] | null, midiMode: MidiRecordMode, pass?: [number, number] | null, slot?: [string, number] | null): Promise<void>;
  /** Turns what was just played on a keyboard into a clip. */
  midiCapture(): Promise<ProjectReply>;
  recordStop(keepPlaying: boolean): Promise<ProjectReply>;
  orphanedRecordings(): Promise<string[]>;
  recoverRecordings(): Promise<string[]>;
  discardRecordings(): Promise<void>;
  exportMix(options: ExportOptions): Promise<ExportReport>;
  /** Places the parts of MIDI files at `start` (beats): on `track` if there is one part, else on new tracks. */
  midiImport(paths: string[], track?: string, start?: number): Promise<ProjectReply>;
  /** Writes one clip, or every MIDI track; resolves to what was written ("3 tracks"). */
  midiExport(path: string, clip?: string): Promise<string>;
  /** Loads an audio file into a sampler (key 0) or a drum pad (key = its note). */
  loadSample(track: string, device: string, key: number, path: string): Promise<ProjectReply>;
  /** Where the hits in a sample start, as fractions of its length (sensitivity 0–100). */
  sampleOnsets(asset: string, sensitivity: number): Promise<number[]>;
  /** A wavetable's waveforms, to draw them: one curve of `points` values (about −1.6…1.6) per waveform. */
  wavetableShapes(table: number, points: number): Promise<number[][]>;
  /** Warps an audio clip to the song, finding its own tempo first; the reply's note says what it found. */
  warpClip(clip: string, mode?: WarpMode): Promise<ProjectReply>;
  /** A new MIDI track playing an audio clip's slices: at its transients, or every `division` beats. */
  sliceToMidi(clip: string, division: number | null): Promise<ProjectReply>;
  /** The CLAP plugins in the standard folders and the user's (new or changed files are scanned; all on `rescan`). */
  pluginsList(rescan: boolean): Promise<PluginList>;
  libraryGet(): Promise<LibraryView>;
  libraryAddFolder(path: string): Promise<LibraryView>;
  libraryRemoveFolder(path: string): Promise<LibraryView>;
  libraryRescan(): Promise<LibraryView>;
  libraryFavourite(path: string, on: boolean): Promise<void>;
  libraryTags(path: string, tags: string[]): Promise<string[]>;
  libraryUsed(path: string): Promise<void>;
  /** Plays a library sample; in time, stretched to the song's tempo and moved to its key. */
  libraryAudition(path: string, sync: boolean): Promise<Audition>;
  libraryStop(): Promise<void>;
  /** Saves a device as a preset file; returns its path. */
  presetSave(owner: string, device: string, name: string): Promise<string>;
  presetLoad(path: string): Promise<Device>;
  pluginsSetFolders(folders: string[]): Promise<PluginList>;
  /** Adds a plugin to a chain (a track, a return or the master), starting it in its sandbox. */
  pluginAdd(track: string, path: string, id: string): Promise<ProjectReply>;
  /** Opens (or brings forward) a plugin's own window, or closes it. */
  pluginWindow(device: string, show: boolean): Promise<ProjectReply>;
  pluginParams(device: string): Promise<PluginParamInfo[]>;
  /** What the plugin writes for these parameters' values. */
  pluginTexts(device: string, ids: number[]): Promise<[number, number, string][]>;
  /** Starts a crashed or failed plugin again from the state saved with the song. */
  pluginReload(device: string): Promise<ProjectReply>;
  /** Renders a track's clips through its devices, for it to play instead while they rest. */
  trackFreeze(track: string): Promise<ProjectReply>;
  /** Renders a clip through its track's devices onto a new audio track below, muting the source track. */
  clipRender(clip: string): Promise<ProjectReply>;
}

/** Decodes the binary peak format written by the `asset_peaks` command. */
function parsePeaks(buffer: ArrayBuffer): PeakData {
  const header = new DataView(buffer, 0, 16);
  const channels = header.getUint32(0, true);
  const step = header.getUint32(4, true);
  const sampleRate = header.getUint32(8, true);
  const count = header.getUint32(12, true);
  return {
    step,
    sampleRate,
    channels: Array.from({ length: channels }, (_, ch) => new Float32Array(buffer.slice(16 + ch * count * 4, 16 + (ch + 1) * count * 4))),
  };
}

function parseSpectrogram(buffer: ArrayBuffer): SpectrogramData {
  const h = new DataView(buffer, 0, 16);
  const [channels, columns, rows, sampleRate] = [0, 4, 8, 12].map((o) => h.getUint32(o, true));
  const size = columns * rows;
  return { columns, rows, sampleRate, channels: Array.from({ length: channels }, (_, c) => new Uint8Array(buffer, 16 + c * size, size)) };
}

function parseWindow(buffer: ArrayBuffer): WaveWindow {
  const h = new DataView(buffer, 0, 20);
  const [channels, count, raw, sampleRate, first] = [0, 4, 8, 12, 16].map((o) => h.getUint32(o, true));
  const per = raw ? count : count * 2;
  return {
    count,
    raw: raw === 1,
    sampleRate,
    first,
    channels: Array.from({ length: channels }, (_, c) => new Float32Array(buffer.slice(20 + c * per * 4, 20 + (c + 1) * per * 4))),
  };
}

const desktop: Backend = {
  load: () => invoke("project_get"),
  apply: (op) => invoke("project_apply", { op }),
  undo: () => invoke("project_undo"),
  redo: () => invoke("project_redo"),
  newProject: (template) => invoke("project_new", { template: template ?? null }),
  templates: () => invoke("templates_list"),
  saveTemplate: (name) => invoke("template_save", { name }),
  deleteTemplate: (id) => invoke("template_delete", { id }),
  recent: () => invoke("recent_projects"),
  forgetRecent: (path) => invoke("recent_forget", { path }),
  historyList: () => invoke("history_list"),
  crashStatus: () => invoke("crash_status"),
  crashEnable: (on) => invoke("crash_enable", { on }),
  crashSeen: () => invoke("crash_seen"),
  crashRead: (file) => invoke("crash_read", { file }),
  crashDelete: (file) => invoke("crash_delete", { file }),
  crashUi: (message, stack) => invoke("crash_ui", { message, stack: stack ?? null }),
  historyJump: (steps) => invoke("history_jump", { steps }),
  findAssets: (folder) => invoke("assets_find", { folder }),
  locateAsset: (asset, path) => invoke("asset_locate", { asset, path }),
  openDemo: () => invoke("project_open_demo"),
  open: (path) => invoke("project_open", { path }),
  save: (path) => invoke("project_save", { path: path ?? null }),
  importFiles: (paths, track, start) => invoke("project_import", { paths, track: track ?? null, start: start ?? null }),
  edit: (clip, op, start, end, zeroCross) => invoke("clip_edit", { clip, op, start: start ?? null, end: end ?? null, zeroCross: !!zeroCross }),
  clipboard: (clip, action, start, end, zeroCross) =>
    invoke("clip_clipboard", { clip, action, start: start ?? null, end: end ?? null, zeroCross: !!zeroCross }),
  effect: (clip, effects, start, end, zeroCross, label) =>
    invoke("clip_effect", { clip, effects, start: start ?? null, end: end ?? null, zeroCross: !!zeroCross, label: label ?? null }),
  previewEffect: (clip, effects, start, end) => invoke("clip_effect_preview", { clip, effects, start: start ?? null, end: end ?? null }),
  repair: (clip, start, end) => invoke("clip_repair", { clip, start, end }),
  spectrumPlot: (clip, start, end, size) => invoke("clip_spectrum_plot", { clip, start: start ?? null, end: end ?? null, size }),
  clipLevel: (clip, start, end) => invoke("clip_level", { clip, start, end }),
  clipMark: (clip, start, end, kind) => invoke("clip_mark", { clip, start: start ?? null, end: end ?? null, kind }),
  fixAnalyse: (clip, start, end, target) => invoke("clip_fix_analyse", { clip, start: start ?? null, end: end ?? null, target }),
  podcastMeasure: (tracks) => invoke("podcast_measure", { tracks }),
  podcastLevel: (tracks, target) => invoke("podcast_level", { tracks, target }),
  podcastDuck: (music, speakers, depth) => invoke("podcast_duck", { music, speakers, depth }),
  chaptersExport: (path, format) => invoke("chapters_export", { path, format }),
  stopPreview: () => invoke("preview_stop"),
  captureNoiseProfile: (clip, start, end) => invoke("noise_profile_capture", { clip, start: start ?? null, end: end ?? null }),
  noiseProfile: () => invoke("noise_profile_status"),
  spectral: (clip, start, end, lowHz, highHz, gainDb, invert) => invoke("clip_spectral", { clip, start, end, lowHz, highHz, gainDb, invert }),
  spectrogram: async (id, start, end, columns, rows, scale) =>
    parseSpectrogram(await invoke<ArrayBuffer>("asset_spectrogram", { id, start, end, columns, rows, scale })),
  waveWindow: async (id, start, end, columns) => parseWindow(await invoke<ArrayBuffer>("asset_window", { id, start, end, columns })),
  draw: (clip, channel, first, samples) => invoke("clip_draw", { clip, channel, first, samples }),
  generate: (generator, target) => invoke("generate_audio", { generator, target }),
  macros: () => invoke("macros_get"),
  saveMacros: (macros) => invoke("macros_set", { macros }),
  batch: (files, steps, output, onProgress) => {
    const progress = new Channel<BatchProgress>();
    progress.onmessage = onProgress;
    return invoke("batch_process", { files, steps, output, progress });
  },
  peaks: async (asset) => parsePeaks(await invoke<ArrayBuffer>("asset_peaks", { id: asset })),
  recoveryAvailable: () => invoke("recovery_available"),
  restoreRecovery: () => invoke("recovery_restore"),
  discardRecovery: () => invoke("recovery_discard"),
  closing: () => invoke("app_closing"),
  recordStart: (countIn, punch, midiMode, pass, slot) => invoke("record_start", { countIn, punch, midiMode, pass: pass ?? null, slot: slot ?? null }),
  midiCapture: () => invoke("midi_capture"),
  recordStop: (keepPlaying) => invoke("record_stop", { keepPlaying }),
  orphanedRecordings: () => invoke("recordings_orphaned"),
  recoverRecordings: () => invoke("recordings_recover"),
  discardRecordings: () => invoke("recordings_discard"),
  exportMix: (options) => invoke("export_mix", { options }),
  midiImport: (paths, track, start) => invoke("midi_import", { paths, track: track ?? null, start: start ?? null }),
  midiExport: (path, clip) => invoke("midi_export", { path, clip: clip ?? null }),
  loadSample: (track, device, key, path) => invoke("device_load_sample", { track, device, key, path }),
  sampleOnsets: (asset, sensitivity) => invoke("sample_onsets", { asset, sensitivity }),
  wavetableShapes: (table, points) => invoke("wavetable_shapes", { table, points }),
  warpClip: (clip, mode) => invoke("clip_warp", { clip, mode: mode ?? null }),
  sliceToMidi: (clip, division) => invoke("slice_to_midi", { clip, division }),
  pluginsList: (rescan) => invoke("plugins_list", { rescan }),
  libraryGet: () => invoke("library_get"),
  libraryAddFolder: (path) => invoke("library_add_folder", { path }),
  libraryRemoveFolder: (path) => invoke("library_remove_folder", { path }),
  libraryRescan: () => invoke("library_rescan"),
  libraryFavourite: (path, on) => invoke("library_favourite", { path, on }),
  libraryTags: (path, tags) => invoke("library_tags", { path, tags }),
  libraryUsed: (path) => invoke("library_used", { path }),
  libraryAudition: (path, sync) => invoke("library_audition", { path, sync }),
  libraryStop: () => invoke("library_stop"),
  presetSave: (owner, device, name) => invoke("preset_save", { owner, device, name }),
  presetLoad: (path) => invoke("preset_load", { path }),
  pluginsSetFolders: (folders) => invoke("plugins_set_folders", { folders }),
  pluginAdd: (track, path, id) => invoke("plugin_add", { track, path, id }),
  pluginWindow: (device, show) => invoke("plugin_window", { device, show }),
  pluginParams: (device) => invoke("plugin_params", { device }),
  pluginTexts: (device, ids) => invoke("plugin_texts", { device, ids }),
  pluginReload: (device) => invoke("plugin_reload", { device }),
  trackFreeze: (track) => invoke("track_freeze", { track }),
  clipRender: (clip) => invoke("clip_render", { clip }),
};

export const backend: Backend = inTauri ? desktop : createPreviewBackend();
