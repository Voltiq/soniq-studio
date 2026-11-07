import { useEffect } from "react";
import { inTauri } from "../engine/bridge";
import { positionBeats, useStore, type SketchPhase } from "../state/store";
import { tempoOf } from "../state/tempo";
import { timelineClips } from "../state/types";
import { Icon } from "./Icon";
import { trackVar } from "./trackColor";

const BARS = [1, 2, 4, 8];
const INPUTS: [string, string][] = [
  ["1", "Input 1"],
  ["2", "Input 2"],
  ["1/2", "Inputs 1 and 2 (stereo)"],
];

const LABELS: Record<SketchPhase, string> = {
  idle: "Record",
  count: "Get ready",
  wait: "At the top",
  rec: "Recording",
  saving: "Saving",
};

/** How far round the loop the playhead is, 0–1. */
function useLoopFraction(): number {
  return useStore((s) => {
    const span = s.loop.end - s.loop.start;
    return span > 0 ? Math.min(1, Math.max(0, (positionBeats(s) - s.loop.start) / span)) : 0;
  });
}

function RecordButton() {
  const phase = useStore((s) => s.sketchPhase);
  const playing = useStore((s) => s.playing);
  const sketchRecord = useStore((s) => s.sketchRecord);
  const fraction = useLoopFraction();
  const r = 92;
  const around = 2 * Math.PI * r;
  const live = phase === "rec" || phase === "count" || phase === "wait";
  return (
    <button
      type="button"
      className={`sketch-rec ${phase}`}
      aria-pressed={live}
      aria-label={live ? "Stop recording this layer" : "Record a layer"}
      disabled={phase === "saving"}
      onClick={() => void sketchRecord()}
    >
      <svg viewBox="0 0 200 200" aria-hidden="true">
        <circle className="ring-back" cx="100" cy="100" r={r} />
        <circle
          className="ring-run"
          cx="100"
          cy="100"
          r={r}
          strokeDasharray={around}
          strokeDashoffset={around * (1 - (playing || live ? fraction : 0))}
          transform="rotate(-90 100 100)"
        />
      </svg>
      <span className="sketch-dot" aria-hidden="true" />
      <span className="sketch-label">{LABELS[phase]}</span>
    </button>
  );
}

export function SketchMode() {
  const project = useStore((s) => s.project);
  const playing = useStore((s) => s.playing);
  const phase = useStore((s) => s.sketchPhase);
  const bars = useStore((s) => s.sketchBars);
  const input = useStore((s) => s.sketchInput);
  const metronome = useStore((s) => s.metronome);
  const canUndo = useStore((s) => s.history.canUndo);
  const { closeSketch, setSketchBars, setSketchInput, setBpm, toggleMetronome, togglePlay, sketchRecord, run, undo } = useStore.getState();
  const tempo = tempoOf(project);
  const loopSeconds = tempo.secondsAt(bars * project.beatsPerBar);
  const layers = project.tracks.filter((t) => t.kind !== "group");
  const busy = phase !== "idle";

  // Space plays, R records; the rest of the app's shortcuts wait until you leave.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (target?.closest("input, select, textarea") || e.ctrlKey || e.metaKey || e.altKey) return;
      if (e.code === "Space") {
        e.preventDefault();
        togglePlay();
      } else if (e.key.toLowerCase() === "r" && !e.repeat) {
        e.preventDefault();
        void sketchRecord();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [togglePlay, sketchRecord]);

  const status =
    phase === "count"
      ? "One bar of clicks, then it records."
      : phase === "wait"
        ? "Recording starts when the loop comes round to the top."
        : phase === "rec"
          ? `Recording one pass of the loop. It stops by itself at the end and plays on.`
          : phase === "saving"
            ? "Saving the layer…"
            : layers.length === 0
              ? `Press the big button (or R) and play: it records one pass of the ${bars}-bar loop, then keeps looping it. Each press adds a layer.`
              : "Press record for another layer over the loop, or open it all in the arrangement.";

  return (
    <div className="sketch" role="dialog" aria-modal="true" aria-labelledby="sketch-title">
      <header className="sketch-head">
        <h2 id="sketch-title" className="sketch-brand">
          <Icon name="mic" size={16} />
          Sketch
        </h2>
        <div className="sketch-opts">
          <label className="sketch-field" title="Tempo">
            <input
              type="number"
              min={40}
              max={240}
              step={1}
              aria-label="Tempo in BPM"
              value={Math.round(project.bpm)}
              disabled={busy}
              onChange={(e) => Number(e.target.value) >= 40 && setBpm(Number(e.target.value))}
            />
            <span className="fix">BPM</span>
          </label>
          <div className="seg sm" role="group" aria-label="Loop length">
            {BARS.map((b) => (
              <button key={b} type="button" aria-pressed={bars === b} disabled={busy} onClick={() => setSketchBars(b)}>
                {b} {b === 1 ? "bar" : "bars"}
              </button>
            ))}
          </div>
          <button className={metronome ? "tbtn on" : "tbtn"} type="button" aria-pressed={metronome} title="Metronome" onClick={toggleMetronome}>
            <Icon name="metronome" size={16} />
          </button>
          <span className="selwrap sm">
            <select className="select" aria-label="What to record" value={input} disabled={busy} onChange={(e) => setSketchInput(e.target.value)}>
              {INPUTS.map(([v, label]) => (
                <option key={v} value={v}>
                  {label}
                </option>
              ))}
            </select>
            <Icon name="chevronDown" size={14} />
          </span>
        </div>
        <button className="btn" type="button" onClick={() => void closeSketch()}>
          <Icon name="arrange" size={15} />
          Open in the arrangement
        </button>
      </header>

      <main className="sketch-stage">
        <RecordButton />
        <p className="sketch-status" role="status">
          {status}
        </p>
        <div className="sketch-transport">
          <button className={playing ? "btn on" : "btn"} type="button" onClick={togglePlay} disabled={phase === "saving"}>
            <Icon name={playing ? "stop" : "play"} size={14} />
            {playing ? "Stop" : "Play the loop"}
          </button>
          <button className="btn btn-ghost" type="button" disabled={!canUndo || busy} onClick={() => void undo()}>
            <Icon name="undo" size={14} />
            Undo
          </button>
        </div>
        <p className="muted sketch-note">
          {inTauri
            ? `Records ${INPUTS.find(([v]) => v === input)?.[1].toLowerCase() ?? "input 1"} of the input set in Settings; it opens only while a layer records. A pass is ${loopSeconds.toFixed(1)} s at ${Math.round(project.bpm)} BPM.`
            : "Recording needs the desktop app."}
        </p>
      </main>

      <footer className="sketch-layers" aria-label="Layers">
        {layers.length === 0 ? (
          <span className="muted">No layers yet.</span>
        ) : (
          layers.map((t) => {
            const clips = timelineClips(t);
            return (
              <div key={t.id} className={`sketch-layer${t.mute ? " muted" : ""}${t.arm ? " armed" : ""}`}>
                <span className="sw" style={{ background: trackVar(t.color) }} aria-hidden="true" />
                <span className="sketch-layer-name">{t.name}</span>
                <span className="muted mono">{clips.length ? `${clips.length} ${clips.length === 1 ? "take" : "takes"}` : t.arm ? "recording" : "empty"}</span>
                <button
                  className={t.mute ? "ib sm on" : "ib sm"}
                  type="button"
                  aria-pressed={t.mute}
                  aria-label={`${t.mute ? "Unmute" : "Mute"} ${t.name}`}
                  onClick={() => void run({ op: "setTrack", id: t.id, patch: { mute: !t.mute } })}
                >
                  <span className="mono">M</span>
                </button>
                <button className="ib sm" type="button" aria-label={`Remove ${t.name}`} disabled={busy} onClick={() => void run({ op: "removeTrack", id: t.id })}>
                  <Icon name="trash" size={13} />
                </button>
              </div>
            );
          })
        )}
      </footer>
    </div>
  );
}
