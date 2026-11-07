import { useEffect, useRef, useState } from "react";
import { inTauri } from "../engine/bridge";
import { formatBars, formatClock } from "../state/format";
import { ROOTS, noteName } from "../state/music";
import { positionBeats, useStore } from "../state/store";
import { tempoOf } from "../state/tempo";
import type { Snap } from "../state/types";
import { Icon } from "./Icon";
import { Select } from "./Select";

/** Major and minor in every key; the piano roll offers the modes and other scales. */
const KEYS = ["major", "minor"].flatMap((scale) => ROOTS.map((root) => `${root} ${scale}`));
const SNAPS: Snap[] = ["bar", "beat", "1/8", "1/16", "off"];

/** Lights up for a moment whenever a note comes in from a MIDI keyboard or the computer keyboard. */
function MidiLight() {
  const at = useStore((s) => s.midiAt);
  const [lit, setLit] = useState(false);
  useEffect(() => {
    if (!at) return;
    setLit(true);
    const id = window.setTimeout(() => setLit(false), 180);
    return () => window.clearTimeout(id);
  }, [at]);
  return <span className={lit ? "midi-dot on" : "midi-dot"} aria-hidden="true" />;
}

function KeysChip() {
  const on = useStore((s) => s.computerKeys);
  const octave = useStore((s) => s.keysOctave);
  const velocity = useStore((s) => s.keysVelocity);
  const toggle = useStore((s) => s.toggleComputerKeys);
  const low = (octave + 2) * 12;
  return (
    <button
      className="chipb keys-chip tip-b"
      type="button"
      aria-pressed={on}
      data-tip="Play notes with the computer keyboard  ·  Ctrl Shift K  ·  Z X octave, C V velocity"
      onClick={toggle}
    >
      <MidiLight />
      <Icon name="keyboard" size={14} />
      Keys
      {on && (
        <span className="keys-range">
          {noteName(low)}–{noteName(low + 17)} · {velocity}
        </span>
      )}
    </button>
  );
}

function Position() {
  const beats = useStore(positionBeats);
  const sec = useStore((s) => s.positionSec);
  const bpb = useStore((s) => s.project.beatsPerBar);
  return (
    <div className="lcd" role="timer" aria-label="Playhead position">
      <span className="lcd-main">{formatBars(beats, bpb)}</span>
      <span className="lcd-sub">{formatClock(sec)}</span>
    </div>
  );
}

function BpmField() {
  // With a tempo track: the tempo at the playhead; typing a value moves the whole track.
  const bpm = useStore((s) => Math.round(tempoOf(s.project).bpmAt(positionBeats(s)) * 100) / 100);
  const tempoTrack = useStore((s) => (s.project.tempo?.length ?? 0) > 0);
  const setBpm = useStore((s) => s.setBpm);
  const [draft, setDraft] = useState<string | null>(null);
  const taps = useRef<number[]>([]);

  const commit = () => {
    const v = Number(draft);
    if (draft !== null && Number.isFinite(v) && v > 0) setBpm(v);
    setDraft(null);
  };

  const tap = () => {
    const now = performance.now();
    taps.current = [...taps.current.filter((t) => now - t < 2500), now].slice(-6);
    if (taps.current.length >= 3) {
      const gaps = taps.current.slice(1).map((t, i) => t - taps.current[i]);
      setBpm(Math.round(60000 / (gaps.reduce((a, b) => a + b, 0) / gaps.length)));
    }
  };

  return (
    <div className="tgroup">
      <label
        className={`field mono bpm${tempoTrack ? " follows tip-b" : ""}`}
        data-tip={tempoTrack ? "Tempo at the playhead, from the tempo track · a new value moves the whole track" : undefined}
      >
        <input
          aria-label={tempoTrack ? "Tempo at the playhead in beats per minute" : "Tempo in beats per minute"}
          inputMode="decimal"
          value={draft ?? String(bpm)}
          onChange={(e) => setDraft(e.target.value.replace(/[^0-9.]/g, ""))}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") (e.target as HTMLInputElement).blur();
            if (e.key === "Escape") setDraft(null);
            if (e.key === "ArrowUp" || e.key === "ArrowDown") {
              e.preventDefault();
              setBpm(bpm + (e.key === "ArrowUp" ? 1 : -1) * (e.shiftKey ? 0.1 : 1));
            }
          }}
        />
        <span className="fix">BPM</span>
      </label>
      <button className="btn sm ghost-ish tip-b" type="button" onClick={tap} data-tip="Tap 3+ times to set the tempo">
        Tap
      </button>
    </div>
  );
}

function EngineChip() {
  const status = useStore((s) => s.engineStatus);
  const compensation = useStore((s) => s.pluginLatencyMs);
  if (!inTauri) {
    return (
      <span className="chip" title="Running in a browser: the Rust audio engine is only available in the desktop app.">
        <span className="dot warn" aria-hidden="true" />
        Preview · no audio engine
      </span>
    );
  }
  if (!status?.running) {
    return (
      <span className="chip" title={status?.error ?? "Starting the audio engine"}>
        <span className="dot bad" aria-hidden="true" />
        {status?.error ? "Audio engine offline" : "Starting audio…"}
      </span>
    );
  }
  const latency = status.bufferFrames && status.sampleRate ? ((status.bufferFrames / status.sampleRate) * 1000).toFixed(1) : null;
  return (
    <span className="chip" title={`${status.device} · ${status.channels} channels`}>
      <span className="dot ok" aria-hidden="true" />
      {status.host} · {(status.sampleRate / 1000).toFixed(1)} kHz{latency && ` · ${status.bufferFrames} smp · ${latency} ms`}
      {compensation > 0.05 && (
        <span className="tip-b" data-tip="Plugin delay compensation: the mix waits this long for the slowest plugin">
          {` · +${compensation.toFixed(1)} ms PDC`}
        </span>
      )}
    </span>
  );
}

export function TransportBar() {
  const playing = useStore((s) => s.playing);
  const recording = useStore((s) => s.recording);
  const loop = useStore((s) => s.loop.enabled);
  const metronome = useStore((s) => s.metronome);
  const countIn = useStore((s) => s.countIn);
  const punch = useStore((s) => s.punch);
  const bpb = useStore((s) => s.project.beatsPerBar);
  const key = useStore((s) => s.project.key);
  const snap = useStore((s) => s.snap);
  const st = useStore.getState();

  // Announce transport changes to screen readers.
  const [announce, setAnnounce] = useState("");
  useEffect(() => setAnnounce(recording ? "Recording" : playing ? "Playing" : "Stopped"), [playing, recording]);

  return (
    <div className="transport shine" role="toolbar" aria-label="Transport">
      <div className="tgroup">
        <button className="ib tip-b" type="button" aria-label="Go to start" data-tip="Go to start  ·  Home" onClick={() => st.seekBeats(0)}>
          <Icon name="toStart" size={16} />
        </button>
        <button
          className={playing ? "tbtn on tip-b" : "tbtn tip-b"}
          type="button"
          aria-label={playing ? "Pause" : "Play"}
          aria-pressed={playing}
          data-tip="Play / pause  ·  Space"
          onClick={st.togglePlay}
        >
          <Icon name={playing ? "pause" : "play"} size={16} />
        </button>
        <button className="tbtn tip-b" type="button" aria-label="Stop" data-tip="Stop  ·  Enter (twice: back to start)" onClick={st.stop}>
          <Icon name="stop" size={14} />
        </button>
        <button
          className={recording ? "tbtn rec on tip-b" : "tbtn rec tip-b"}
          type="button"
          aria-label="Record"
          aria-pressed={recording}
          data-tip="Record  ·  R"
          onClick={() => void st.toggleRecord()}
        >
          <span className="rec-dot" aria-hidden="true" />
        </button>
        <button
          className={loop ? "ib toggled tip-b" : "ib tip-b"}
          type="button"
          aria-label="Loop"
          aria-pressed={loop}
          data-tip="Loop  ·  L  ·  Shift-drag the ruler to set"
          onClick={st.toggleLoop}
        >
          <Icon name="loop" size={17} />
        </button>
      </div>

      <span className="vsep" />
      <Position />
      <span className="vsep" />
      <BpmField />

      <span className="selwrap">
        <Select className="select" aria-label="Time signature" value={bpb} onChange={(e) => st.setBeatsPerBar(Number(e.target.value))}>
          {[2, 3, 4, 5, 6, 7].map((n) => (
            <option key={n} value={n}>
              {n} / 4
            </option>
          ))}
        </Select>
      </span>
      <span className="selwrap">
        <Select className="select" aria-label="Key and scale" value={key} onChange={(e) => st.setKey(e.target.value)}>
          {(KEYS.includes(key) ? KEYS : [key, ...KEYS]).map((k) => (
            <option key={k}>{k}</option>
          ))}
        </Select>
      </span>
      <button
        className={metronome ? "ib toggled tip-b" : "ib tip-b"}
        type="button"
        aria-label="Metronome"
        aria-pressed={metronome}
        data-tip="Metronome  ·  M"
        onClick={st.toggleMetronome}
      >
        <Icon name="metronome" size={17} />
      </button>
      <button
        className="chipb tip-b"
        type="button"
        aria-pressed={countIn}
        data-tip="One bar of clicks before recording starts"
        onClick={st.toggleCountIn}
      >
        Count-in
      </button>
      <button
        className="chipb tip-b"
        type="button"
        aria-pressed={punch}
        data-tip="Record only inside the loop range, starting a bar early"
        onClick={st.togglePunch}
      >
        Punch
      </button>
      <KeysChip />
      <button
        className="chipb tip-b"
        type="button"
        data-tip="Capture MIDI: turn what you just played into a clip, even without recording  ·  Ctrl Shift C"
        onClick={() => void st.captureMidi()}
      >
        Capture
      </button>

      <span className="vsep pro-only" />
      <div className="seg pro-only" role="group" aria-label="Snap to grid">
        {SNAPS.map((v) => (
          <button key={v} type="button" aria-pressed={snap === v} onClick={() => st.setSnap(v)}>
            {v === "off" ? "Off" : v === "bar" ? "Bar" : v === "beat" ? "Beat" : v}
          </button>
        ))}
      </div>

      <div className="tright">
        <EngineChip />
      </div>
      <span className="sr-only" aria-live="polite">
        {announce}
      </span>
    </div>
  );
}
