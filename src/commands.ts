/*
 * One registry for every action. The menus, the command palette and the
 * keyboard shortcuts all read from it. A command without `run` is planned:
 * it shows up disabled with the roadmap phase that delivers it.
 */
import { engine, inTauri } from "./engine/bridge";
import { DEVICE_CATALOG } from "./state/devices";
import { EDIT_LABELS } from "./state/edits";
import { EFFECTS } from "./state/effects";
import { parseKey, scaleClasses } from "./state/music";
import { duplicate, nudge, quantize, stretchEnds, transpose } from "./state/notes";
import { keyFor } from "./state/keymap";
import { openUrl } from "@tauri-apps/plugin-opener";
import { REPO_URL, editorTarget, pianoTarget, tempoLaneShown, useStore } from "./state/store";
import { zoomPiano } from "./state/pianoView";
import { projectLength, type SimpleEditOp, type ToolId, type ViewId } from "./state/types";

export type MenuId = "File" | "Edit" | "Track" | "Clip" | "Effect" | "Analyze" | "Transport" | "View" | "Help";
export const MENUS: MenuId[] = ["File", "Edit", "Track", "Clip", "Effect", "Analyze", "Transport", "View", "Help"];

export interface Command {
  id: string;
  label: string;
  menu: MenuId;
  group: number;
  shortcut?: string;
  alsoKeys?: string[];
  run?: () => void;
  enabled?: () => boolean;
  checked?: () => boolean;
  phase?: number;
}

const s = () => useStore.getState();
const inEditor = () => s().view === "editor";
const inPiano = () => s().view === "piano" && !!pianoTarget(s());
const hasNotes = () => inPiano() && s().pianoSelection.length > 0;
/** The key's pitch classes when notes should stay in it, else null. */
const scaleSet = () => {
  const key = parseKey(s().project.key);
  return s().snapToScale && key.scale !== "chromatic" ? scaleClasses(key) : null;
};
const gridStep = () => s().pianoGrid || 0.25;

const selectedTrack = () => s().project.tracks.find((t) => t.id === s().selectedTrackId);
const selectedClip = () => s().project.tracks.flatMap((t) => t.clips).find((c) => c.id === s().selectedClipId);

/** Arrow-key note commands; they only exist in the piano roll. */
const noteKeys: [string, string, string, () => void][] = [
  ["up", "Move notes up", "ArrowUp", () => s().editNotes((n, sel) => transpose(n, sel, 1, scaleSet()), "Transpose")],
  ["down", "Move notes down", "ArrowDown", () => s().editNotes((n, sel) => transpose(n, sel, -1, scaleSet()), "Transpose")],
  ["octaveUp", "Move notes up an octave", "Shift+ArrowUp", () => s().editNotes((n, sel) => transpose(n, sel, 12, null), "Transpose")],
  ["octaveDown", "Move notes down an octave", "Shift+ArrowDown", () => s().editNotes((n, sel) => transpose(n, sel, -12, null), "Transpose")],
  ["later", "Move notes later", "ArrowRight", () => s().editNotes((n, sel) => nudge(n, sel, gridStep()), "Move notes")],
  ["earlier", "Move notes earlier", "ArrowLeft", () => s().editNotes((n, sel) => nudge(n, sel, -gridStep()), "Move notes")],
  ["longer", "Lengthen notes", "Shift+ArrowRight", () => s().editNotes((n, sel) => stretchEnds(n, sel, gridStep()), "Resize notes")],
  ["shorter", "Shorten notes", "Shift+ArrowLeft", () => s().editNotes((n, sel) => stretchEnds(n, sel, -gridStep()), "Resize notes")],
];

function toggleOnSelected(key: "mute" | "solo" | "arm") {
  const t = selectedTrack();
  if (t) s().updateTrack(t.id, { [key]: !t[key] });
}

const views: [ViewId, string, string][] = [
  ["arrange", "Arrangement", "F1"],
  ["session", "Session", "F2"],
  ["mixer", "Mixer", "F3"],
  ["editor", "Audio editor", "F4"],
  ["piano", "Piano roll", "F5"],
];

const tools: [ToolId, string, string][] = [
  ["pointer", "Pointer", "V"],
  ["range", "Range select", "S"],
  ["draw", "Draw", "D"],
  ["split", "Split", "C"],
  ["erase", "Erase", "E"],
  ["automation", "Automation", "A"],
  ["zoom", "Zoom", "Z"],
  ["hand", "Hand", "H"],
];

const edits: SimpleEditOp[] = ["normalize", "amplify", "fadeIn", "fadeOut", "reverse", "silence"];

export const COMMANDS: Command[] = [
  { id: "file.new", label: "New project", menu: "File", group: 0, shortcut: "Ctrl+N", run: () => void s().newProject() },
  { id: "file.open", label: "Open…", menu: "File", group: 0, shortcut: "Ctrl+O", run: () => void s().openProject() },
  { id: "file.start", label: "Start page · templates, recent songs, lessons", menu: "File", group: 0, run: () => s().setWelcomeOpen(true) },
  { id: "file.template", label: "Save as template…", menu: "File", group: 0, run: () => s().setWelcomeOpen(true, "template"), enabled: () => inTauri },
  {
    id: "file.missing",
    label: "Find missing files…",
    menu: "File",
    group: 1,
    run: () => s().setMissingOpen(true),
    enabled: () => s().project.assets.some((a) => a.missing),
  },
  { id: "file.demo", label: "Open the demo song", menu: "File", group: 0, run: () => void s().openDemo() },
  { id: "file.save", label: "Save", menu: "File", group: 0, shortcut: "Ctrl+S", run: () => void s().saveProject() },
  { id: "file.saveAs", label: "Save as…", menu: "File", group: 0, shortcut: "Ctrl+Shift+S", run: () => void s().saveProjectAs() },
  { id: "file.import", label: "Import audio…", menu: "File", group: 1, shortcut: "Ctrl+I", run: () => void s().importAudio() },
  { id: "file.importMidi", label: "Import MIDI file…", menu: "File", group: 1, run: () => void s().importMidi() },
  { id: "file.export", label: "Export…", menu: "File", group: 1, shortcut: "Ctrl+Shift+E", run: () => s().setExportOpen(true) },
  { id: "file.stems", label: "Export stems or regions…", menu: "File", group: 1, run: () => s().setExportOpen(true) },
  {
    id: "file.chapters",
    label: "Export chapters from markers…",
    menu: "File",
    group: 1,
    run: () => void s().exportChapters(),
    enabled: () => s().project.markers.length > 0,
  },
  {
    id: "file.exportMidi",
    label: "Export MIDI…",
    menu: "File",
    group: 1,
    run: () => void s().exportMidi(),
    enabled: () => s().project.tracks.some((t) => t.clips.some((c) => c.kind === "midi")),
  },
  { id: "file.batch", label: "Batch process files…", menu: "File", group: 1, run: () => s().setMacroDialog("batch") },
  { id: "file.settings", label: "Settings · audio, MIDI, plugins, keys…", menu: "File", group: 2, shortcut: "Ctrl+,", run: () => s().setSettingsOpen(true) },

  { id: "edit.undo", label: "Undo", menu: "Edit", group: 0, shortcut: "Ctrl+Z", run: () => void s().undo(), enabled: () => s().history.canUndo },
  {
    id: "edit.redo",
    label: "Redo",
    menu: "Edit",
    group: 0,
    shortcut: "Ctrl+Shift+Z",
    alsoKeys: ["Ctrl+Y"],
    run: () => void s().redo(),
    enabled: () => s().history.canRedo,
  },
  // In the audio editor these act on the waveform selection, in the piano roll on the selected notes.
  {
    id: "edit.cut",
    label: "Cut",
    menu: "Edit",
    group: 1,
    shortcut: "Ctrl+X",
    run: () => (inPiano() ? s().pianoCut() : void s().editorClipboard("cut")),
    enabled: () => inEditor() || hasNotes(),
  },
  {
    id: "edit.copy",
    label: "Copy",
    menu: "Edit",
    group: 1,
    shortcut: "Ctrl+C",
    run: () => (inPiano() ? s().pianoCopy() : void s().editorClipboard("copy")),
    enabled: () => inEditor() || hasNotes(),
  },
  {
    id: "edit.paste",
    label: "Paste",
    menu: "Edit",
    group: 1,
    shortcut: "Ctrl+V",
    run: () => (inPiano() ? s().pianoPaste() : void s().editorClipboard("paste")),
    enabled: () => (inEditor() && s().hasClipboard) || (inPiano() && s().noteClipboard.length > 0),
  },
  {
    id: "edit.delete",
    label: "Delete",
    menu: "Edit",
    group: 1,
    shortcut: "Delete",
    alsoKeys: ["Backspace"],
    run: () => {
      const st = s();
      if (inPiano()) st.pianoDelete();
      else if (inEditor() && st.editorSelection) void st.editorClipboard("delete");
      else if (st.selectedMarkerId && !inEditor()) st.removeMarker(st.selectedMarkerId);
      else st.deleteSelectedClip();
    },
    enabled: () =>
      inPiano() ? hasNotes() : s().selectedClipId !== null || (inEditor() && !!s().editorSelection) || (!inEditor() && s().selectedMarkerId !== null),
  },
  {
    id: "edit.selectAll",
    label: "Select all",
    menu: "Edit",
    group: 1,
    shortcut: "Ctrl+A",
    run: () => {
      const st = s();
      const audio = editorTarget(st);
      if (inPiano()) st.pianoSelectAll();
      else if (audio) st.setEditorSelection({ start: 0, end: audio.clip.duration });
    },
    enabled: () => inPiano() || (inEditor() && !!editorTarget(s())),
  },
  {
    id: "edit.duplicateNew",
    label: "Duplicate to a new track",
    menu: "Edit",
    group: 1,
    run: () => void s().extractSelection(false),
    enabled: () => inEditor() && !!s().editorSelection,
  },
  {
    id: "edit.splitNew",
    label: "Split New (move to a new track)",
    menu: "Edit",
    group: 1,
    shortcut: "Ctrl+Alt+I",
    run: () => void s().extractSelection(true),
    enabled: () => inEditor() && !!s().editorSelection,
  },
  {
    id: "edit.ripple",
    label: "Ripple editing · deleting closes the gap",
    menu: "Edit",
    group: 2,
    run: () => s().toggleRipple(),
    checked: () => s().ripple,
  },
  { id: "edit.trim", label: "Trim to selection", menu: "Edit", group: 1, run: () => s().trimToSelection(), enabled: () => inEditor() && !!s().editorSelection },
  { id: "edit.marker", label: "Add marker at the playhead", menu: "Edit", group: 2, shortcut: "Ctrl+M", run: () => s().addMarker() },
  { id: "edit.region", label: "Add region from the loop range", menu: "Edit", group: 2, shortcut: "Ctrl+Shift+M", run: () => s().addRegionFromLoop() },
  { id: "edit.history", label: "History…", menu: "Edit", group: 0, shortcut: "Ctrl+H", run: () => s().setHistoryOpen(!s().historyOpen), checked: () => s().historyOpen },
  { id: "edit.palette", label: "Command palette…", menu: "Edit", group: 2, shortcut: "Ctrl+K", run: () => s().setPaletteOpen(true) },
  ...tools.map(
    ([id, label, key]): Command => ({
      id: `tool.${id}`,
      label: `${label} tool`,
      menu: "Edit",
      group: 3,
      shortcut: key,
      run: () => s().setTool(id),
      checked: () => s().tool === id,
    }),
  ),

  { id: "track.audio", label: "New audio track", menu: "Track", group: 0, shortcut: "Ctrl+T", run: () => s().addTrack("audio") },
  { id: "track.midi", label: "New MIDI track", menu: "Track", group: 0, shortcut: "Ctrl+Shift+T", run: () => s().addTrack("midi") },
  { id: "track.return", label: "New return track", menu: "Track", group: 0, phase: 4 },
  {
    id: "track.delete",
    label: "Delete selected track",
    menu: "Track",
    group: 0,
    shortcut: "Ctrl+Delete",
    run: () => selectedTrack() && s().removeTrack(s().selectedTrackId),
    enabled: () => !!selectedTrack(),
  },
  {
    id: "track.group",
    label: "Group selected tracks · Ctrl-click headers to pick several",
    menu: "Track",
    group: 1,
    shortcut: "Ctrl+G",
    run: () => s().groupTracks(),
    enabled: () => s().project.tracks.some((t) => t.kind !== "group" && (s().trackSelection.includes(t.id) || t.id === s().selectedTrackId)),
  },
  {
    id: "track.ungroup",
    label: "Ungroup",
    menu: "Track",
    group: 1,
    shortcut: "Ctrl+Shift+G",
    run: () => {
      const t = selectedTrack();
      const group = t?.kind === "group" ? t.id : t?.group;
      if (group) s().ungroup(group);
    },
    enabled: () => !!selectedTrack() && (selectedTrack()?.kind === "group" || !!selectedTrack()?.group),
  },
  {
    id: "track.automation",
    label: "Show automation of the selected track",
    menu: "Track",
    group: 1,
    run: () => selectedTrack() && s().toggleAutomation(s().selectedTrackId),
    checked: () => s().automationOpen[s().selectedTrackId] !== undefined,
    enabled: () => !!selectedTrack(),
  },
  {
    id: "track.freeze",
    label: "Freeze selected track",
    menu: "Track",
    group: 1,
    run: () => {
      const t = selectedTrack();
      if (t?.frozen) s().unfreezeTrack(t.id);
      else if (t) void s().freezeTrack(t.id);
    },
    checked: () => !!selectedTrack()?.frozen,
    enabled: () => !!selectedTrack() && selectedTrack()?.kind !== "group",
  },
  {
    id: "track.flatten",
    label: "Flatten selected track · a frozen track becomes audio",
    menu: "Track",
    group: 1,
    run: () => selectedTrack()?.frozen && s().flattenTrack(s().selectedTrackId),
    enabled: () => !!selectedTrack()?.frozen,
  },
  { id: "mix.save", label: "Save the mix", menu: "Track", group: 3, run: () => void s().run({ op: "saveMix" }) },
  {
    id: "mix.compare",
    label: "Compare mixes A/B · switch",
    menu: "Track",
    group: 3,
    run: () => void s().run({ op: "swapMix" }),
    checked: () => !!s().project.otherMix,
  },
  {
    id: "mix.keep",
    label: "Keep the mix that plays (end A/B)",
    menu: "Track",
    group: 3,
    run: () => void s().run({ op: "keepMix" }),
    enabled: () => !!s().project.otherMix,
  },
  { id: "track.mute", label: "Mute selected track", menu: "Track", group: 2, run: () => toggleOnSelected("mute"), checked: () => !!selectedTrack()?.mute },
  { id: "track.solo", label: "Solo selected track", menu: "Track", group: 2, run: () => toggleOnSelected("solo"), checked: () => !!selectedTrack()?.solo },
  { id: "track.arm", label: "Arm selected track", menu: "Track", group: 2, run: () => toggleOnSelected("arm"), checked: () => !!selectedTrack()?.arm },

  ...edits.map(
    (op): Command => ({
      id: `clip.${op}`,
      label: EDIT_LABELS[op],
      menu: "Clip",
      group: 0,
      run: () => void s().edit(op),
    }),
  ),
  {
    id: "clip.loudness",
    label: "Loudness normalize to −16 LUFS",
    menu: "Clip",
    group: 0,
    run: () => void s().edit({ loudness: -16 }),
  },
  { id: "clip.split", label: "Split at playhead", menu: "Clip", group: 1, shortcut: "Ctrl+E", run: () => s().splitAtPlayhead() },
  {
    id: "clip.duplicate",
    label: "Duplicate",
    menu: "Clip",
    group: 1,
    shortcut: "Ctrl+D",
    // In the audio editor with a selection: Audacity's Duplicate (onto a new track).
    run: () => (inPiano() ? s().editNotes(duplicate, "Duplicate notes") : inEditor() && s().editorSelection ? void s().extractSelection(false) : s().duplicateSelectedClip()),
    enabled: () => (inPiano() ? hasNotes() : s().selectedClipId !== null),
  },
  {
    id: "clip.duplicateLinked",
    label: "Duplicate as linked copy",
    menu: "Clip",
    group: 1,
    shortcut: "Ctrl+Alt+D",
    run: () => s().duplicateLinked(),
    enabled: () => selectedClip()?.kind === "midi",
  },
  {
    id: "clip.unlink",
    label: "Unlink clip",
    menu: "Clip",
    group: 1,
    run: () => {
      const c = selectedClip();
      if (c) s().unlinkClip(c.id);
    },
    enabled: () => {
      const c = selectedClip();
      return c?.kind === "midi" && !!c.link;
    },
  },
  { id: "clip.newMidi", label: "New MIDI clip", menu: "Clip", group: 1, run: () => void s().newMidiClip() },
  ...(
    [
      [null, "standard", "Fades: standard (straight, equal-power crossfades)"],
      ["linear", "linear", "Fades: linear"],
      ["equalPower", "equalPower", "Fades: equal power"],
      ["sCurve", "sCurve", "Fades: S-curve"],
    ] as const
  ).map(
    ([curve, id, label]): Command => ({
      id: `clip.fade.${id}`,
      label,
      menu: "Clip",
      group: 2,
      run: () => s().setFadeCurve(curve),
      enabled: () => selectedClip()?.kind === "audio",
      checked: () => {
        const c = selectedClip();
        return c?.kind === "audio" && (c.curve ?? null) === curve;
      },
    }),
  ),
  {
    id: "clip.fix",
    label: "Fix my audio…",
    menu: "Clip",
    group: 2,
    run: () => s().openEffect("fixAudio"),
    enabled: () => selectedClip()?.kind === "audio",
  },
  {
    id: "clip.render",
    label: "Render to a new audio track",
    menu: "Clip",
    group: 1,
    run: () => {
      const c = selectedClip();
      if (c) void s().renderClip(c.id);
    },
    enabled: () => {
      const c = selectedClip();
      return !!c && c.slot == null && !s().project.tracks.find((t) => t.clips.some((x) => x.id === c.id))?.frozen;
    },
  },
  {
    id: "clip.toSession",
    label: "Put a copy in the session",
    menu: "Clip",
    group: 1,
    run: () => {
      const c = selectedClip();
      const track = s().project.tracks.find((t) => t.clips.some((x) => x.id === c?.id));
      if (!c || !track) return;
      // The first scene with an empty slot on the clip's track, or a new scene.
      const scenes = s().project.scenes;
      const free = scenes.findIndex((_, i) => !track.clips.some((x) => x.slot === i));
      s().putInSlot(c.id, free < 0 ? scenes.length : free);
      s().setView("session");
    },
    enabled: () => {
      const c = selectedClip();
      return !!c && c.slot == null;
    },
  },
  {
    id: "clip.steps",
    label: "Step sequencer",
    menu: "Clip",
    group: 1,
    run: () => {
      s().setPianoMode("steps");
      s().openPianoRoll();
    },
    enabled: () => !!pianoTarget(s()),
  },
  {
    id: "clip.quantize",
    label: "Quantize notes",
    menu: "Clip",
    group: 1,
    shortcut: "Ctrl+U",
    run: () => {
      const q = s().quantizeSettings;
      s().editNotes((n, sel) => quantize(n, sel, q.grid, q.strength, q.swing, q.ends), "Quantize");
    },
    enabled: () => !!pianoTarget(s()),
  },
  { id: "clip.pianoRoll", label: "Edit notes in the piano roll", menu: "Clip", group: 1, run: () => s().openPianoRoll(), enabled: () => !!pianoTarget(s()) },
  ...noteKeys.map(
    ([id, label, key, run]): Command => ({ id: `notes.${id}`, label, menu: "Clip", group: 3, shortcut: key, run, enabled: inPiano }),
  ),
  { id: "notes.deselect", label: "Deselect notes", menu: "Clip", group: 3, shortcut: "Escape", run: () => s().setPianoSelection([]), enabled: hasNotes },
  {
    id: "clip.warp",
    label: "Warp to project tempo",
    menu: "Clip",
    group: 2,
    // Warping finds the clip's own tempo first; on a warped clip it lets the audio play at its own speed again.
    run: () => {
      const c = selectedClip();
      if (c?.kind === "audio") void (c.warp ? s().setWarp(c.id, null) : s().warpClip(c.id));
    },
    enabled: () => selectedClip()?.kind === "audio",
    checked: () => {
      const c = selectedClip();
      return c?.kind === "audio" && !!c.warp;
    },
  },
  {
    id: "clip.slice",
    label: "Slice to new MIDI track",
    menu: "Clip",
    group: 2,
    run: () => {
      const c = selectedClip();
      if (c?.kind === "audio") void s().sliceToMidi(c.id, null);
    },
    enabled: () => selectedClip()?.kind === "audio",
  },

  {
    id: "plugins.rescan",
    label: "Look for plugins again",
    menu: "Effect",
    group: 2,
    run: () => void s().loadPlugins(true),
  },
  ...DEVICE_CATALOG.filter((d) => d.category === "audio-fx").map(
    (d): Command => ({
      id: `fx.${d.type}`,
      label: `Add ${d.name}`,
      menu: "Effect",
      group: 0,
      run: () => s().addDevice(s().selectedTrackId, d.type),
    }),
  ),
  // Offline effects change the samples of the clip open in the editor (or the selected one).
  ...EFFECTS.map(
    (e): Command => ({
      id: `process.${e.type}`,
      label: `${e.name}…`,
      menu: "Effect",
      group: 1,
      run: () => s().openEffect(e.type),
      enabled: () => !!editorTarget(s()),
    }),
  ),
  {
    id: "process.repair",
    label: "Repair (redraw a click or dropout)",
    menu: "Effect",
    group: 1,
    run: () => void s().repairSelection(),
    enabled: () => inEditor() && !!s().editorSelection,
  },
  { id: "process.macro", label: "Apply a macro…", menu: "Effect", group: 2, run: () => s().setMacroDialog("clip"), enabled: () => !!editorTarget(s()) },
  { id: "process.generate", label: "Generate tone, noise or silence…", menu: "Effect", group: 2, run: () => s().setGeneratorOpen(true) },
  {
    id: "fx.plugins",
    label: "Browse plugins (CLAP, VST3)",
    menu: "Effect",
    group: 3,
    run: () => {
      if (!s().browserOpen) s().toggleBrowser();
      void s().loadPlugins(false);
    },
  },

  { id: "transport.play", label: "Play / pause", menu: "Transport", group: 0, shortcut: "Space", run: () => s().togglePlay() },
  { id: "transport.stop", label: "Stop · press twice to return", menu: "Transport", group: 0, shortcut: "Enter", run: () => s().stop() },
  { id: "transport.record", label: "Record", menu: "Transport", group: 0, shortcut: "R", run: () => void s().toggleRecord(), checked: () => s().recording },
  { id: "transport.countIn", label: "Count-in before recording", menu: "Transport", group: 0, run: () => s().toggleCountIn(), checked: () => s().countIn },
  { id: "transport.punch", label: "Punch in and out at the loop range", menu: "Transport", group: 0, run: () => s().togglePunch(), checked: () => s().punch },
  {
    id: "transport.back",
    label: "Back to arrangement",
    menu: "Transport",
    group: 0,
    run: () => s().backToArrangement(),
    enabled: () => Object.values(s().clipStates).some((c) => !c.arrangement),
  },
  { id: "transport.stopClips", label: "Stop all session clips", menu: "Transport", group: 0, run: () => s().stopAllClips() },
  { id: "transport.start", label: "Go to start", menu: "Transport", group: 1, shortcut: "Home", run: () => s().seekBeats(0) },
  { id: "transport.loop", label: "Loop", menu: "Transport", group: 1, shortcut: "L", run: () => s().toggleLoop(), checked: () => s().loop.enabled },
  { id: "transport.metronome", label: "Metronome", menu: "Transport", group: 1, shortcut: "M", run: () => s().toggleMetronome(), checked: () => s().metronome },
  {
    id: "transport.keys",
    label: "Computer keyboard plays notes",
    menu: "Transport",
    group: 2,
    shortcut: "Ctrl+Shift+K",
    run: () => s().toggleComputerKeys(),
    checked: () => s().computerKeys,
  },
  { id: "transport.panic", label: "Stop all notes", menu: "Transport", group: 2, run: () => engine.panic() },
  { id: "transport.capture", label: "Capture MIDI (what you just played)", menu: "Transport", group: 2, shortcut: "Ctrl+Shift+C", run: () => void s().captureMidi() },
  ...(
    [
      ["new", "MIDI recording makes a new clip"],
      ["overdub", "MIDI recording overdubs into the clip"],
      ["replace", "MIDI recording replaces notes in the clip"],
    ] as const
  ).map(
    ([mode, label]): Command => ({
      id: `transport.midi.${mode}`,
      label,
      menu: "Transport",
      group: 3,
      run: () => s().setMidiRecordMode(mode),
      checked: () => s().midiRecordMode === mode,
    }),
  ),

  ...views.map(
    ([id, label, key]): Command => ({
      id: `view.${id}`,
      label,
      menu: "View",
      group: 0,
      shortcut: key,
      run: () => s().setView(id),
      checked: () => s().view === id,
    }),
  ),
  {
    id: "view.swap",
    label: "Swap Arrangement and Session",
    menu: "View",
    group: 0,
    shortcut: "Tab",
    run: () => s().setView(s().view === "session" ? "arrange" : "session"),
  },
  { id: "view.browser", label: "Browser", menu: "View", group: 1, shortcut: "Ctrl+Alt+B", run: () => s().toggleBrowser(), checked: () => s().browserOpen },
  {
    id: "view.analysis",
    label: "Meters · loudness, true peak, spectrum",
    menu: "View",
    group: 1,
    shortcut: "Ctrl+Alt+M",
    run: () => s().toggleAnalysis(),
    checked: () => s().analysisOpen,
  },
  { id: "analyze.spectrum", label: "Plot spectrum…", menu: "Analyze", group: 0, run: () => s().setAnalyzeDialog("spectrum"), enabled: () => !!editorTarget(s()) },
  { id: "analyze.contrast", label: "Contrast (speech over background)…", menu: "Analyze", group: 0, run: () => s().setAnalyzeDialog("contrast"), enabled: () => !!editorTarget(s()) },
  { id: "analyze.clipping", label: "Find clipping", menu: "Analyze", group: 1, run: () => void s().markFound({ kind: "clipping" }), enabled: () => !!editorTarget(s()) },
  {
    id: "analyze.beats",
    label: "Beat finder",
    menu: "Analyze",
    group: 1,
    run: () => void s().markFound({ kind: "beats", sensitivity: 0.5 }),
    enabled: () => !!editorTarget(s()),
  },
  {
    id: "analyze.sounds",
    label: "Label sounds",
    menu: "Analyze",
    group: 1,
    run: () => void s().markFound({ kind: "sounds", threshold: -30, minSilence: 1, minSound: 0.1 }),
    enabled: () => !!editorTarget(s()),
  },
  { id: "view.analysis2", label: "Meters · loudness, true peak, spectrum", menu: "Analyze", group: 2, run: () => s().toggleAnalysis(), checked: () => s().analysisOpen },
  { id: "help.start", label: "Start page and lessons", menu: "Help", group: 0, run: () => s().setWelcomeOpen(true) },
  { id: "help.updates", label: "Check for updates…", menu: "Help", group: 1, run: () => void s().checkForUpdates(true) },
  { id: "help.crashes", label: "Crash reports…", menu: "Help", group: 1, run: () => s().setAboutOpen("crashes") },
  { id: "help.issue", label: "Report a problem on GitHub…", menu: "Help", group: 1, run: () => void openUrl(`${REPO_URL}/issues/new`) },
  { id: "help.about", label: "About Soniq Studio", menu: "Help", group: 2, run: () => s().setAboutOpen("about") },
  {
    id: "view.podcast",
    label: "Podcast tools · speakers, ducking, chapters",
    menu: "View",
    group: 1,
    run: () => s().togglePodcast(),
    checked: () => s().podcastOpen,
  },
  {
    id: "view.sketch",
    label: "Sketch mode · a big record button and a loop",
    menu: "View",
    group: 1,
    run: () => void (s().sketchOpen ? s().closeSketch() : s().openSketch(false)),
    checked: () => s().sketchOpen,
  },
  {
    id: "view.tempo",
    label: "Tempo track",
    menu: "View",
    group: 1,
    run: () => {
      s().toggleTempoLane();
      if (tempoLaneShown(s())) s().setView("arrange");
    },
    checked: () => tempoLaneShown(s()),
  },
  // In the audio editor and the piano roll these zoom that view instead of the arrangement.
  {
    id: "view.zoomIn",
    label: "Zoom in",
    menu: "View",
    group: 2,
    shortcut: "Ctrl+=",
    run: () => (inEditor() ? s().zoomEditor(2) : inPiano() ? zoomPiano(1.5) : s().zoom(1.25)),
  },
  {
    id: "view.zoomOut",
    label: "Zoom out",
    menu: "View",
    group: 2,
    shortcut: "Ctrl+-",
    run: () => (inEditor() ? s().zoomEditor(0.5) : inPiano() ? zoomPiano(1 / 1.5) : s().zoom(0.8)),
  },
  {
    id: "view.zoomFit",
    label: "Zoom to fit",
    menu: "View",
    group: 2,
    shortcut: "Ctrl+0",
    run: () => (inEditor() ? s().setEditorView(null) : inPiano() ? s().setPianoZoom(null) : fitArrangement()),
  },
  {
    id: "view.spectrogram",
    label: "Spectrogram in the editor",
    menu: "View",
    group: 2,
    run: () => s().setEditorDisplay(s().editorDisplay === "spectrogram" ? "waveform" : "spectrogram"),
    checked: () => s().editorDisplay === "spectrogram",
  },
  {
    id: "view.theme",
    label: "Light theme",
    menu: "View",
    group: 3,
    shortcut: "Ctrl+Shift+L",
    run: () => s().setTheme(s().theme === "dark" ? "light" : "dark"),
    checked: () => s().theme === "light",
  },
  {
    id: "view.mode",
    label: "Pro mode",
    menu: "View",
    group: 3,
    shortcut: "Ctrl+Shift+P",
    run: () => s().setMode(s().mode === "pro" ? "essentials" : "pro"),
    checked: () => s().mode === "pro",
  },
];

/** Fits the whole song into the arrangement's visible lane width. */
export function fitArrangement() {
  const lanes = document.querySelector<HTMLElement>(".arr-scroll");
  const width = (lanes?.clientWidth ?? window.innerWidth - 600) - 230;
  s().zoomToFit(width, projectLength(s().project));
}

export function isEnabled(cmd: Command): boolean {
  return !!cmd.run && (cmd.enabled ? cmd.enabled() : true);
}

export function matchesShortcut(shortcut: string, e: KeyboardEvent): boolean {
  const parts = shortcut.split("+");
  const key = parts.pop() ?? "";
  const ctrl = parts.includes("Ctrl");
  const shift = parts.includes("Shift");
  const alt = parts.includes("Alt");
  if ((e.ctrlKey || e.metaKey) !== ctrl || e.altKey !== alt) return false;
  const k = e.key;
  // Zoom-in sits on "=" on US layouts and on an unshifted "+" on German ones.
  if (key === "=") return k === "=" || k === "+";
  if (key === "Plus") return k === "+";
  if (e.shiftKey !== shift) return false;
  if (key === "Space") return k === " ";
  return k.toLowerCase() === key.toLowerCase();
}

/** A command's shortcut in the keymap in use (the set, then the user's own changes). */
export function shortcutOf(cmd: Command): string | undefined {
  return keyFor(cmd.id, cmd.shortcut, s().keymap);
}

export function findShortcutCommand(e: KeyboardEvent): Command | undefined {
  return COMMANDS.find((cmd) => {
    if (!cmd.run) return false;
    const key = shortcutOf(cmd);
    // Second keys (Ctrl+Y for redo) only while the command keeps its own.
    const keys = key === cmd.shortcut ? [key, ...(cmd.alsoKeys ?? [])] : [key];
    return keys.some((k) => k && matchesShortcut(k, e));
  });
}
