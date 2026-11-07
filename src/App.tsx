import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect } from "react";
import { findShortcutCommand, isEnabled } from "./commands";
import { AnalysisPanel } from "./components/AnalysisPanel";
import { LessonCard } from "./components/LessonCard";
import { MissingFilesDialog } from "./components/MissingFilesDialog";
import { WelcomeScreen } from "./components/WelcomeScreen";
import { PodcastPanel } from "./components/PodcastPanel";
import { AboutDialog } from "./components/AboutDialog";
import { SketchMode } from "./components/SketchMode";
import { BrowserPanel } from "./components/BrowserPanel";
import { CommandPalette } from "./components/CommandPalette";
import { DeviceChain } from "./components/DeviceChain";
import { EffectDialog } from "./components/EffectDialog";
import { ExportDialog } from "./components/ExportDialog";
import { GenerateDialog } from "./components/GenerateDialog";
import { MacroDialog } from "./components/MacroDialog";
import { NoteToolDialog } from "./components/NoteToolDialog";
import { Icon, type IconName } from "./components/Icon";
import { SettingsDialog } from "./components/SettingsDialog";
import { Toasts } from "./components/Toasts";
import { ToolRail } from "./components/ToolRail";
import { TopBar } from "./components/TopBar";
import { TransportBar } from "./components/TransportBar";
import { backend } from "./engine/backend";
import { engine, inTauri, type DeviceEvent } from "./engine/bridge";
import { ingestFrame, simulateMeters } from "./state/meters";
import { AUDIO_EXTENSIONS, MIDI_EXTENSIONS, positionBeats, useStore } from "./state/store";
import { tempoOf } from "./state/tempo";
import type { ViewId } from "./state/types";
import { ArrangeView } from "./views/ArrangeView";
import { EditorView } from "./views/EditorView";
import { MixerView } from "./views/MixerView";
import { PianoRollView } from "./views/PianoRollView";
import { SessionView } from "./views/SessionView";

const TABS: [ViewId, string, IconName, string][] = [
  ["arrange", "Arrangement", "arrange", "F1"],
  ["session", "Session", "grid", "F2"],
  ["mixer", "Mixer", "sliders", "F3"],
  ["editor", "Audio editor", "wave", "F4"],
  ["piano", "Piano roll", "piano", "F5"],
];

/** The computer keyboard as a piano, by physical key so every layout plays the same: A is C, W is C♯, … */
const KEY_NOTES: Record<string, number> = {
  KeyA: 0,
  KeyW: 1,
  KeyS: 2,
  KeyE: 3,
  KeyD: 4,
  KeyF: 5,
  KeyT: 6,
  KeyG: 7,
  KeyY: 8,
  KeyH: 9,
  KeyU: 10,
  KeyJ: 11,
  KeyK: 12,
  KeyO: 13,
  KeyL: 14,
  KeyP: 15,
  Semicolon: 16,
  Quote: 17,
};

/** Drives the playhead once per frame (and, in the browser preview, the meters). */
function useClock() {
  useEffect(() => {
    let raf = 0;
    const loop = (now: number) => {
      useStore.getState().tick(now);
      if (!inTauri) {
        const s = useStore.getState();
        simulateMeters(s, positionBeats(s));
      }
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, []);
}

/** Meter frames lock the UI clock to the engine; the status poll feeds the transport chip. */
function useEngine() {
  useEffect(() => {
    let midiSeen = -1;
    let revisionSeen = -1;
    let refreshing = false;
    let launchesSeen = "";
    let silencedAt = -1e9;
    engine.subscribeMeters((frame) => {
      const s = useStore.getState();
      ingestFrame(frame, new Set(s.project.tracks.filter((t) => t.arm && t.kind === "audio").map((t) => t.id)));
      s.syncFromEngine(frame.position);
      if (frame.previewing !== s.previewing) s.setPreviewing(frame.previewing);
      // A broken plugin sent NaN or infinity: say so, once in a while.
      if (frame.silenced && performance.now() - silencedAt > 10_000) {
        silencedAt = performance.now();
        s.toast({
          tone: "warning",
          title: "Something sent broken audio",
          detail: "A device or plugin produced invalid samples; Soniq silenced them instead of playing them. Switch devices off one by one to find it.",
        });
      }
      // Launch states re-render the session only when they change; progress
      // goes through clipProgress, like the meters.
      const launches = JSON.stringify(frame.clips.map(([id, c]) => [id, c.playing, c.queued, c.stopping, c.arrangement]));
      if (launches !== launchesSeen) {
        launchesSeen = launches;
        s.setClipStates(Object.fromEntries(frame.clips));
      }
      if (midiSeen >= 0 && frame.midi !== midiSeen) useStore.setState({ midiAt: performance.now() });
      midiSeen = frame.midi;
      // A MIDI controller changed the project: fetch it. One fetch at a time;
      // a change during a fetch is picked up by the next frame after it.
      if (revisionSeen < 0) revisionSeen = frame.revision;
      else if (frame.revision !== revisionSeen && !refreshing) {
        revisionSeen = frame.revision;
        refreshing = true;
        void s.refreshProject().finally(() => (refreshing = false));
      }
    });
    if (!inTauri) return;
    let live = true;
    // Devices coming and going: the engine has already moved; say where to.
    const unlistenDevices = listen<DeviceEvent>("audio-device", ({ payload }) => {
      const s = useStore.getState();
      if ("stopped" in payload && payload.stopped) s.setPlaying(false);
      const stopped = "stopped" in payload && payload.stopped ? " Playback stopped." : "";
      if (payload.kind === "lost")
        s.toast({ tone: "warning", title: `${payload.previous || "The audio output"} is gone`, detail: `Playing on ${payload.device} now.${stopped}` });
      else if (payload.kind === "switched") s.toast({ tone: "info", title: `Now playing on ${payload.device}`, detail: `Instead of ${payload.previous}.${stopped}` });
      else if (payload.kind === "noOutput")
        s.toast({ tone: "danger", title: "No audio output", detail: `${payload.error} Soniq keeps looking, and plays as soon as one is there.${stopped}`, sticky: true });
      else if (payload.kind === "inputLost") {
        if (s.recording) void s.stopRecording(false);
        s.toast({ tone: "warning", title: `${payload.device || "The audio input"} is gone`, detail: "Recording stopped and kept up to here. It opens again when it is back." });
      } else s.toast({ tone: "success", title: `${payload.device} is back`, detail: "The input is open again." });
      void poll();
    });
    const poll = () =>
      Promise.all([engine.status(), engine.inputStatus()])
        .then(([status, input]) => {
          if (!live) return;
          useStore.getState().setEngineStatus(status);
          useStore.getState().setInputStatus(input);
        })
        .catch((err) => console.error("Engine status failed", err));
    void poll();
    const id = window.setInterval(poll, 2000);
    return () => {
      live = false;
      window.clearInterval(id);
      void unlistenDevices.then((fn) => fn());
    };
  }, []);

  // The engine wraps the loop itself, so it needs the range in seconds.
  const loop = useStore((s) => s.loop);
  const tempo = useStore((s) => tempoOf(s.project));
  useEffect(() => {
    engine.setLoop(loop.enabled, tempo.secondsAt(loop.start), tempo.secondsAt(loop.end));
  }, [loop, tempo]);

  // With no MIDI track armed, keyboards play the selected track.
  const selected = useStore((s) => s.selectedTrackId);
  useEffect(() => engine.midiSelect(selected || null), [selected]);
}

function useShortcuts() {
  useEffect(() => {
    // Computer-keyboard notes still sounding, by physical key.
    const held = new Map<string, number>();
    const release = (code: string) => {
      const pitch = held.get(code);
      if (pitch === undefined) return;
      held.delete(code);
      engine.note(null, pitch, 0, false);
    };

    const onKey = (e: KeyboardEvent) => {
      const s = useStore.getState();
      // Dialogs handle their own keys.
      if (s.paletteOpen || s.settingsOpen || s.exportOpen || s.effectDialog || s.generatorOpen || s.macroDialog || s.noteTool || s.welcomeOpen || s.missingOpen || s.sketchOpen || s.aboutOpen) return;
      const target = e.target as HTMLElement | null;
      const isRange = target instanceof HTMLInputElement && target.type === "range";
      const typing = !isRange && !!target?.closest("input, textarea, select, [contenteditable='true']");
      const mod = e.ctrlKey || e.metaKey;
      // File and palette shortcuts work everywhere; the rest leave text fields alone.
      if (typing && !(mod && "kosni".includes(e.key.toLowerCase()))) return;
      // Enter keeps its usual job on focused buttons, and Tab keeps moving focus.
      if (e.key === "Enter" && target?.closest("button, a, [role='slider']")) return;
      if (e.key === "Tab" && target && target !== document.body) return;
      // Sliders and knobs keep their arrow keys.
      if (e.key.startsWith("Arrow") && (isRange || target?.closest("[role='slider']"))) return;

      if (s.computerKeys && !mod && !e.altKey) {
        const offset = KEY_NOTES[e.code];
        if (offset !== undefined) {
          e.preventDefault();
          if (e.repeat || held.has(e.code)) return;
          const pitch = Math.min(127, Math.max(0, (s.keysOctave + 2) * 12 + offset));
          held.set(e.code, pitch);
          engine.note(null, pitch, s.keysVelocity / 127, true);
          return;
        }
        const change = { KeyZ: [-1, 0], KeyX: [1, 0], KeyC: [0, -20], KeyV: [0, 20] }[e.code];
        if (change) {
          e.preventDefault();
          if (change[0]) s.setKeysOctave(s.keysOctave + change[0]);
          else s.setKeysVelocity(s.keysVelocity + change[1]);
          return;
        }
      }

      const cmd = findShortcutCommand(e);
      if (!cmd || !isEnabled(cmd)) return;
      e.preventDefault();
      if (e.repeat && !cmd.id.startsWith("view.zoom") && !cmd.id.startsWith("notes.")) return;
      cmd.run?.();
    };
    const onKeyUp = (e: KeyboardEvent) => release(e.code);
    // A key let go while the window was in the background must not hang.
    const releaseAll = () => [...held.keys()].forEach(release);
    window.addEventListener("keydown", onKey);
    window.addEventListener("keyup", onKeyUp);
    window.addEventListener("blur", releaseAll);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("keyup", onKeyUp);
      window.removeEventListener("blur", releaseAll);
    };
  }, []);
}

const extension = (path: string) => path.split(".").pop()?.toLowerCase() ?? "";
const isAudioFile = (path: string) => AUDIO_EXTENSIONS.includes(extension(path));
const isMidiFile = (path: string) => MIDI_EXTENSIONS.includes(extension(path));

/** What sits under a window position: a sampler or drum pad, or a lane and the beat there. */
function dropTarget(x: number, y: number): { el: HTMLElement | null; sample?: HTMLElement; lane?: HTMLElement; beats?: number } {
  const el = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-sample-target], [data-track-id], [data-new-track]") ?? null;
  if (!el) return { el };
  if (el.dataset.sampleTarget) return { el, sample: el };
  const s = useStore.getState();
  const beats = Math.max(0, Math.round((x - el.getBoundingClientRect().left) / s.pxPerBeat));
  return { el, lane: el, beats };
}

/** Files dragged in from Explorer land where they are dropped: audio and MIDI on the timeline, samples on a sampler or pad. */
function useFileDrop() {
  useEffect(() => {
    if (!inTauri) return;
    let highlighted: HTMLElement | null = null;
    const highlight = (el: HTMLElement | null) => {
      if (el === highlighted) return;
      highlighted?.classList.remove("drop-target");
      el?.classList.add("drop-target");
      highlighted = el;
    };
    const unlisten = getCurrentWebview().onDragDropEvent(({ payload }) => {
      if (payload.type === "leave") return highlight(null);
      const ratio = window.devicePixelRatio || 1;
      const { el, sample, lane, beats } = dropTarget(payload.position.x / ratio, payload.position.y / ratio);
      if (payload.type !== "drop") return highlight(el);
      highlight(null);
      const store = useStore.getState();
      const audio = payload.paths.filter(isAudioFile);
      const midi = payload.paths.filter(isMidiFile);
      if (sample) {
        const [track, device, key] = (sample.dataset.sampleTarget ?? "").split("|");
        if (audio[0]) void store.loadSample(track, device, Number(key), audio[0]);
        else store.toast({ tone: "info", title: "Drop an audio file here", detail: `Samples can be ${AUDIO_EXTENSIONS.join(", ")}.` });
        return;
      }
      if (!audio.length && !midi.length) {
        store.toast({ tone: "info", title: "Only audio and MIDI files can be dropped", detail: `Soniq reads ${[...AUDIO_EXTENSIONS, ...MIDI_EXTENSIONS].join(", ")}.` });
        return;
      }
      const kind = lane?.dataset.trackKind;
      const at = lane ? beats : undefined;
      if (midi.length) void store.importMidi(midi, kind === "midi" ? lane?.dataset.trackId : undefined, at);
      if (audio.length) void store.importAudio(audio, kind === "audio" ? lane?.dataset.trackId : undefined, at);
    });
    return () => void unlisten.then((fn) => fn());
  }, []);
}

/** Window title, and a last chance to keep unsaved work when closing. */
function useWindow() {
  const name = useStore((s) => s.project.name);
  const dirty = useStore((s) => s.dirty);
  useEffect(() => {
    const title = name ? `${dirty ? "• " : ""}${name} — Soniq Studio` : "Soniq Studio";
    if (inTauri) void getCurrentWindow().setTitle(title);
    else document.title = title;
  }, [name, dirty]);

  useEffect(() => {
    if (!inTauri) return;
    const unlisten = getCurrentWindow().onCloseRequested(async (event) => {
      if (!(await useStore.getState().confirmDiscard())) {
        event.preventDefault();
        return;
      }
      await backend.closing().catch(() => {});
    });
    return () => void unlisten.then((fn) => fn());
  }, []);
}

export default function App() {
  useClock();
  useEngine();
  useShortcuts();
  useFileDrop();
  useWindow();

  const ready = useStore((s) => s.ready);
  const view = useStore((s) => s.view);
  const mode = useStore((s) => s.mode);
  const theme = useStore((s) => s.theme);
  const browserOpen = useStore((s) => s.browserOpen);
  const analysisOpen = useStore((s) => s.analysisOpen);
  const welcomeOpen = useStore((s) => s.welcomeOpen);
  const podcastOpen = useStore((s) => s.podcastOpen);
  const sketchOpen = useStore((s) => s.sketchOpen);
  // A new colour set or density redraws everything (the tokens are already in place).
  useStore((s) => s.colorSet);
  useStore((s) => s.density);
  const toggleAnalysis = useStore((s) => s.toggleAnalysis);
  const setView = useStore((s) => s.setView);

  useEffect(() => {
    void useStore.getState().load();
  }, []);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  if (!ready) {
    return (
      <div className="app splash" data-mode={mode}>
        <span className="mark" aria-hidden="true">
          <svg viewBox="0 0 24 24" width="22" height="22">
            <path d="M3 12h2.5l2-5 3 10 3-8 2 5 1.5-2H21" />
          </svg>
        </span>
        <span className="muted">Loading Soniq Studio…</span>
        <Toasts />
      </div>
    );
  }

  return (
    <div className="app" data-mode={mode}>
      <TopBar />
      <TransportBar />
      <div className="work">
        <ToolRail />
        {browserOpen && <BrowserPanel />}
        <main className="stage shine" aria-label="Workspace">
          <div className="tabs" role="tablist" aria-label="Views">
            {TABS.map(([id, label, icon, key]) => (
              <button
                key={id}
                type="button"
                role="tab"
                aria-selected={view === id}
                aria-controls="view-panel"
                className={view === id ? "tab on" : "tab"}
                onClick={() => setView(id)}
              >
                <Icon name={icon} size={15} />
                {label}
                <span className="kbd pro-only">{key}</span>
              </button>
            ))}
            <button
              type="button"
              className={`tab tab-end${analysisOpen ? " on" : ""}`}
              aria-pressed={analysisOpen}
              aria-label="Meters"
              title="Meters: loudness, true peak, correlation and spectrum of the master (Ctrl+Alt+M)"
              onClick={toggleAnalysis}
            >
              <Icon name="meter" size={16} />
            </button>
          </div>
          <div className="view" id="view-panel" role="tabpanel" aria-label={TABS.find((t) => t[0] === view)?.[1]}>
            {view === "arrange" && <ArrangeView />}
            {view === "session" && <SessionView />}
            {view === "mixer" && <MixerView />}
            {view === "editor" && <EditorView />}
            {view === "piano" && <PianoRollView />}
          </div>
          {(view === "arrange" || view === "session" || view === "mixer") && <DeviceChain />}
        </main>
        {analysisOpen && <AnalysisPanel />}
        {podcastOpen && <PodcastPanel />}
      </div>
      <CommandPalette />
      <SettingsDialog />
      <ExportDialog />
      <EffectDialog />
      <GenerateDialog />
      <MacroDialog />
      <NoteToolDialog />
      <MissingFilesDialog />
      {welcomeOpen && <WelcomeScreen />}
      {sketchOpen && <SketchMode />}
      <AboutDialog />
      <LessonCard />
      <Toasts />
    </div>
  );
}
