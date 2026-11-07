/*
 * The step sequencer (FL Studio's channel rack, for one clip): rows of drum
 * pads or notes in the key, one cell per grid step. Click or drag across
 * cells to switch steps on and off; the lane below sets each step's
 * velocity, length or timing shift for the selected row. It edits the same
 * notes as the piano roll, so the two views always agree.
 */
import { useEffect, useMemo, useRef, useState, type CSSProperties, type PointerEvent } from "react";
import { trackVar } from "../components/trackColor";
import { engine } from "../engine/bridge";
import { padName } from "../state/devices";
import { noteName, parseKey, scaleClasses } from "../state/music";
import { quantize } from "../state/notes";
import { positionBeats, useStore } from "../state/store";
import { sortNotes, type MidiClip, type Note, type Track } from "../state/types";
import { Select } from "../components/Select";

const HEAD_W = 156;
const LANE_H = 64;
type Lane = "velocity" | "length" | "shift";

const LANES: [Lane, string][] = [
  ["velocity", "Velocity"],
  ["length", "Length"],
  ["shift", "Shift"],
];
const SWINGS = [0, 10, 20, 30, 40, 50, 60];
const FILLS: [number, string][] = [
  [1, "every step"],
  [2, "every 2 steps"],
  [4, "every 4 steps"],
  [8, "every 8 steps"],
];

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/** The note a cell stands for: that pitch, starting nearest that step. */
function noteAt(notes: Note[], pitch: number, step: number, grid: number): number {
  return notes.findIndex((n) => n.pitch === pitch && Math.round(n.start / grid) === step);
}

function PlayColumn({ clip, grid, stepW, steps }: { clip: MidiClip; grid: number; stepW: number; steps: number }) {
  const beats = useStore(positionBeats);
  const playing = useStore((s) => s.playing);
  const step = Math.floor((beats - clip.start) / grid);
  if (!playing || step < 0 || step >= steps) return null;
  return <div className="steps-now" style={{ left: HEAD_W + step * stepW, width: stepW }} aria-hidden="true" />;
}

export function StepSequencer({ track, clip }: { track: Track; clip: MidiClip }) {
  const [box, setBox] = useState<HTMLDivElement | null>(null);
  const [width, setWidth] = useState(900);
  useEffect(() => {
    if (!box) return;
    const ro = new ResizeObserver(() => setWidth(box.clientWidth));
    ro.observe(box);
    return () => ro.disconnect();
  }, [box]);
  const grid = useStore((s) => s.pianoGrid) || 0.25;
  const keyName = useStore((s) => s.project.key);
  const assets = useStore((s) => s.project.assets);
  const st = useStore.getState();
  const rack = track.devices.find((d) => d.type === "drum-rack" && d.role === "instrument");
  const steps = Math.max(1, Math.round(clip.length / grid));
  const stepW = clamp(Math.floor((width - HEAD_W - 4) / steps), 18, 56);
  const perBeat = Math.max(1, Math.round(1 / grid));

  const [preview, setPreviewState] = useState<Note[] | null>(null);
  const previewRef = useRef<Note[] | null>(null);
  const setPreview = (next: Note[] | null) => {
    previewRef.current = next;
    setPreviewState(next);
  };
  const notes = preview ?? clip.notes;

  // Drum pads top to bottom (51 … 36), or the key's notes around what is there.
  const rows = useMemo(() => {
    if (rack) return Array.from({ length: 16 }, (_, i) => 51 - i);
    const key = parseKey(keyName);
    const inKey = key.scale === "chromatic" ? null : scaleClasses(key);
    const used = clip.notes.map((n) => n.pitch);
    let lo = used.length ? Math.min(...used) - 2 : 60;
    let hi = used.length ? Math.max(...used) + 2 : 72;
    if (hi - lo < 12) {
      lo = Math.round((lo + hi) / 2) - 6;
      hi = lo + 12;
    }
    const list: number[] = [];
    for (let p = Math.min(127, hi); p >= Math.max(0, lo); p--) if (!inKey || inKey.has(p % 12) || used.includes(p)) list.push(p);
    return list;
  }, [rack, keyName, clip.notes]);

  const [row, setRow] = useState(() => (rack ? 36 : rows[Math.floor(rows.length / 2)]));
  const [lane, setLane] = useState<Lane>("velocity");
  const label = (pitch: number) => {
    if (!rack) return noteName(pitch);
    const sample = rack.samples?.find((x) => x.key === pitch);
    return sample ? (assets.find((a) => a.id === sample.asset)?.name ?? "Sample") : padName(pitch);
  };
  const audition = (pitch: number) => {
    engine.note(track.id, pitch, 0.85, true);
    window.setTimeout(() => engine.note(track.id, pitch, 0, false), 160);
  };
  const save = async (next: Note[], what: string) => {
    setPreview(next);
    await st.setNotes(clip.id, sortNotes(next), what, []);
    setPreview(null);
  };

  // Painting cells: the first cell decides whether the drag switches steps on or off.
  const paint = useRef<{ on: boolean; base: Note[]; last: string } | null>(null);
  const cellAt = (x: number, y: number) => {
    const el = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-step]");
    return el ? { pitch: Number(el.dataset.pitch), step: Number(el.dataset.step) } : null;
  };
  const toggle = (pitch: number, step: number) => {
    const p = paint.current;
    if (!p) return;
    const list = previewRef.current ?? p.base;
    const i = noteAt(list, pitch, step, grid);
    if (p.on && i < 0) setPreview([...list, { start: step * grid, length: grid, pitch, velocity: 0.8 }]);
    else if (!p.on && i >= 0) setPreview(list.filter((_, k) => k !== i));
  };
  const onGridDown = (e: PointerEvent<HTMLDivElement>) => {
    const cell = cellAt(e.clientX, e.clientY);
    if (!cell || e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    const on = noteAt(clip.notes, cell.pitch, cell.step, grid) < 0;
    paint.current = { on, base: clip.notes, last: `${cell.pitch}:${cell.step}` };
    setRow(cell.pitch);
    toggle(cell.pitch, cell.step);
    if (on) audition(cell.pitch);
  };
  const onGridMove = (e: PointerEvent<HTMLDivElement>) => {
    const p = paint.current;
    const cell = p && cellAt(e.clientX, e.clientY);
    if (!p || !cell || `${cell.pitch}:${cell.step}` === p.last) return;
    p.last = `${cell.pitch}:${cell.step}`;
    toggle(cell.pitch, cell.step);
  };
  const onGridUp = () => {
    const p = paint.current;
    paint.current = null;
    const next = previewRef.current;
    if (p && next) void save(next, p.on ? "Add steps" : "Remove steps");
  };

  // The lane: drag across steps of the selected row to set their value.
  const laneDrag = useRef<{ base: Note[]; rect: DOMRect } | null>(null);
  const setLaneValue = (clientX: number, clientY: number) => {
    const d = laneDrag.current;
    if (!d) return;
    const step = Math.floor((clientX - d.rect.left) / stepW);
    const y = clamp(1 - (clientY - d.rect.top - 4) / (LANE_H - 8), 0, 1);
    const list = previewRef.current ?? d.base;
    const i = noteAt(list, row, step, grid);
    if (i < 0) return;
    const n = list[i];
    const changed =
      lane === "velocity"
        ? { ...n, velocity: clamp(y, 0.05, 1) }
        : lane === "length"
          ? { ...n, length: clamp(y, 0.05, 1) * grid }
          : { ...n, start: Math.max(0, step * grid + clamp(y - 0.5, -0.45, 0.45) * grid) };
    setPreview(list.map((x, k) => (k === i ? changed : x)));
  };

  const rowNotes = (pitch: number) => Array.from({ length: steps }, (_, k) => noteAt(notes, pitch, k, grid));
  const selectedRow = rowNotes(row);
  const fill = (every: number) => {
    const kept = clip.notes.filter((n) => n.pitch !== row || n.start >= clip.length);
    const added = Array.from({ length: Math.ceil(steps / every) }, (_, k) => ({ start: k * every * grid, length: grid, pitch: row, velocity: k % 4 === 0 ? 1 : 0.8 }));
    void save([...kept, ...added], "Fill steps");
  };
  const style = { "--c": trackVar(track.color), "--step": `${stepW}px`, "--head": `${HEAD_W}px` } as CSSProperties;

  return (
    <div ref={setBox} className="steps card-inset" style={style}>
      <div className="steps-bar">
        <label className="steps-opt">
          <span className="lab">Steps</span>
          <span className="selwrap">
            <Select
              className="select sm"
              value={steps}
              onChange={(e) => st.trimClip(clip.id, clip.start, Number(e.target.value) * grid)}
              aria-label="Number of steps"
            >
              {[...new Set([8, 16, 32, 64, steps])].sort((a, b) => a - b).map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </Select>
          </span>
        </label>
        <label className="steps-opt tip-b" data-tip="Delays every second step, for a shuffled groove">
          <span className="lab">Swing</span>
          <span className="selwrap">
            <Select
              className="select sm"
              defaultValue={0}
              aria-label="Swing"
              onChange={(e) => {
                const swing = Number(e.target.value) / 100;
                st.editNotes((n) => quantize(n, [], grid, 1, swing, false), "Swing");
              }}
            >
              {SWINGS.map((v) => (
                <option key={v} value={v}>
                  {v}%
                </option>
              ))}
            </Select>
          </span>
        </label>
        <span className="vsep" />
        <span className="muted steps-row-name">{label(row)}</span>
        <label className="steps-opt">
          <span className="lab">Fill</span>
          <span className="selwrap">
            <Select className="select sm" value="" aria-label={`Fill ${label(row)}`} onChange={(e) => e.target.value && fill(Number(e.target.value))}>
              <option value="">Choose…</option>
              {FILLS.map(([v, l]) => (
                <option key={v} value={v}>
                  {l}
                </option>
              ))}
            </Select>
          </span>
        </label>
        <button
          className="btn sm"
          type="button"
          disabled={!selectedRow.some((i) => i >= 0)}
          onClick={() => void save(clip.notes.filter((n) => n.pitch !== row), `Clear ${label(row)}`)}
        >
          Clear row
        </button>
      </div>

      <div className="steps-scroll">
        <div className="steps-grid" role="grid" aria-label="Steps" onPointerDown={onGridDown} onPointerMove={onGridMove} onPointerUp={onGridUp} onPointerCancel={() => (paint.current = null)}>
          {rows.map((pitch) => (
            <div key={pitch} className={`steps-row${pitch === row ? " on" : ""}`} role="row">
              <button
                type="button"
                className="steps-head"
                onPointerDown={(e) => e.stopPropagation()}
                onClick={() => {
                  setRow(pitch);
                  audition(pitch);
                }}
                title={`${label(pitch)} (${noteName(pitch)}) · click to hear and select`}
              >
                <span className="steps-name">{label(pitch)}</span>
                <span className="steps-note">{noteName(pitch)}</span>
              </button>
              {rowNotes(pitch).map((i, k) => (
                <div
                  key={k}
                  role="gridcell"
                  aria-selected={i >= 0}
                  aria-label={`${label(pitch)} step ${k + 1}`}
                  className={`step${i >= 0 ? " lit" : ""}${Math.floor(k / perBeat) % 2 ? " alt" : ""}`}
                  data-pitch={pitch}
                  data-step={k}
                  style={i >= 0 ? ({ "--v": notes[i].velocity } as CSSProperties) : undefined}
                />
              ))}
            </div>
          ))}
          <PlayColumn clip={clip} grid={grid} stepW={stepW} steps={steps} />
        </div>

      <div className="steps-lane">
        <div className="steps-lane-head">
          <div className="seg" role="group" aria-label="Lane">
            {LANES.map(([id, l]) => (
              <button key={id} type="button" aria-pressed={lane === id} onClick={() => setLane(id)}>
                {l}
              </button>
            ))}
          </div>
        </div>
        <div
          className="steps-lane-cells"
          style={{ height: LANE_H }}
          onPointerDown={(e) => {
            e.currentTarget.setPointerCapture(e.pointerId);
            laneDrag.current = { base: clip.notes, rect: e.currentTarget.getBoundingClientRect() };
            setLaneValue(e.clientX, e.clientY);
          }}
          onPointerMove={(e) => setLaneValue(e.clientX, e.clientY)}
          onPointerUp={() => {
            const d = laneDrag.current;
            laneDrag.current = null;
            const next = previewRef.current;
            if (d && next) void save(next, lane === "velocity" ? "Step velocity" : lane === "length" ? "Step length" : "Step shift");
          }}
          aria-label={`${LANES.find(([id]) => id === lane)?.[1]} of ${label(row)}`}
        >
          {selectedRow.map((i, k) => {
            const n = i >= 0 ? notes[i] : null;
            const value = !n ? 0 : lane === "velocity" ? n.velocity : lane === "length" ? Math.min(1, n.length / grid) : n.start / grid - k;
            return (
              <div key={k} className={`lane-cell${Math.floor(k / perBeat) % 2 ? " alt" : ""}`}>
                {n &&
                  (lane === "shift" ? (
                    <span className="lane-bar shift" style={{ bottom: value >= 0 ? "50%" : `${50 + value * 100}%`, height: `${Math.max(2, Math.abs(value) * 100)}%` }} />
                  ) : (
                    <span className="lane-bar" style={{ height: `${Math.max(4, value * 100)}%` }} />
                  ))}
              </div>
            );
          })}
        </div>
      </div>
      </div>
    </div>
  );
}
