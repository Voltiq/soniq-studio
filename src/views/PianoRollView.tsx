/*
 * The piano roll: draw, select, move, copy, resize and delete notes, a
 * velocity lane, scale highlighting and folding, quantize, legato and
 * transpose. One canvas draws the ruler, keyboard, grid and notes; a scroll
 * container underneath it provides the scrollbars.
 */
import { useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from "react";
import { useShallow } from "zustand/react/shallow";
import { Icon } from "../components/Icon";
import { Popover } from "../components/Menu";
import { NoteToolsMenu } from "../components/NoteToolDialog";
import { StepSequencer } from "./StepSequencer";
import { trackColor } from "../components/trackColor";
import { engine } from "../engine/bridge";
import { ROOTS, SCALE_NAMES, formatKey, isBlackKey, noteName, parseKey, scaleClasses, snapToScale } from "../state/music";
import { GRIDS, MIN_NOTE, commit, duplicate, legato, quantize, remove, transpose } from "../state/notes";
import { MAX_PPB, MIN_PPB, pianoView } from "../state/pianoView";
import { pianoTarget, positionBeats, useStore, type PianoRows } from "../state/store";
import { timelineClips, type MidiClip, type Note, type Track } from "../state/types";

const KEYS_W = 60;
const RULER_H = 22;
const ROW_H = 14;
const VEL_H = 64;
const EDGE = 6;

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

type DragKind = "move" | "start" | "end" | "marquee" | "draw" | "erase" | "keys" | "clip-end";

interface DragState {
  kind: DragKind;
  x0: number;
  y0: number;
  beat0: number;
  row0: number;
  /** The note grabbed (or drawn). */
  index: number;
  copy: boolean;
  additive: boolean;
  moved: boolean;
  base: Note[];
  selection: number[];
}

interface Preview {
  notes: Note[];
  selection: number[];
  /** Marquee in content pixels. */
  marquee?: { x0: number; y0: number; x1: number; y1: number };
  clipLength?: number;
}

const ROW_MODES: [PianoRows, string, string][] = [
  ["all", "All", "Every key"],
  ["scale", "In key", "Only the keys of the scale (and keys with notes)"],
  ["used", "Used", "Only keys that have notes"],
];

/** Theme colours the canvases need, read once per draw. */
function palette() {
  const css = getComputedStyle(document.documentElement);
  const v = (name: string) => css.getPropertyValue(name).trim();
  return {
    bg: v("--surface-1"),
    black: v("--surface-2"),
    keyWhite: v("--surface-4"),
    keyBlack: v("--bg"),
    text: v("--text-3"),
    text1: v("--text-1"),
    line: v("--line"),
    line2: v("--line-2"),
    bar: v("--grid-bar"),
    beat: v("--grid-beat"),
    accent: v("--accent"),
    accentSoft: v("--accent-soft"),
  };
}

function setupCanvas(canvas: HTMLCanvasElement, width: number, height: number) {
  const ratio = window.devicePixelRatio || 1;
  canvas.width = Math.max(1, Math.round(width * ratio));
  canvas.height = Math.max(1, Math.round(height * ratio));
  canvas.style.width = `${width}px`;
  canvas.style.height = `${height}px`;
  const ctx = canvas.getContext("2d");
  ctx?.setTransform(ratio, 0, 0, ratio, 0, 0);
  return ctx;
}

function Playhead({ clip, ppb, scrollX, width, height }: { clip: MidiClip; ppb: number; scrollX: number; width: number; height: number }) {
  const beats = useStore(positionBeats);
  const x = KEYS_W + (beats - clip.start) * ppb - scrollX;
  if (x < KEYS_W || x > width) return null;
  return <div className="pr-playhead" style={{ transform: `translateX(${x}px)`, height }} aria-hidden="true" />;
}

function QuantizeMenu({ disabled }: { disabled: boolean }) {
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const q = useStore((s) => s.quantizeSettings);
  const { setQuantizeSettings, editNotes } = useStore.getState();
  const apply = () => editNotes((notes, sel) => quantize(notes, sel, q.grid, q.strength, q.swing, q.ends), "Quantize");
  return (
    <div className="pr-split">
      <button className="btn sm tip-b" type="button" disabled={disabled} data-tip="Quantize the selection (or every note)  ·  Ctrl U" onClick={apply}>
        Quantize
      </button>
      <button className="btn sm" type="button" aria-label="Quantize settings" aria-haspopup="dialog" disabled={disabled} onClick={(e) => setAnchor(anchor ? null : e.currentTarget)}>
        <Icon name="chevronDown" size={13} />
      </button>
      {anchor && (
        <Popover anchor={anchor} onClose={() => setAnchor(null)} label="Quantize settings">
          <div className="pr-q">
            <label className="pr-q-row">
              <span>Grid</span>
              <span className="selwrap">
                <select className="select sm" value={q.grid} onChange={(e) => setQuantizeSettings({ ...q, grid: Number(e.target.value) })}>
                  {GRIDS.map(([label, beats]) => (
                    <option key={label} value={beats}>
                      {label}
                    </option>
                  ))}
                </select>
                <Icon name="chevronDown" size={13} />
              </span>
            </label>
            <label className="pr-q-row">
              <span>Strength</span>
              <input type="range" className="range" min={0} max={100} value={Math.round(q.strength * 100)} onChange={(e) => setQuantizeSettings({ ...q, strength: Number(e.target.value) / 100 })} />
              <output>{Math.round(q.strength * 100)}%</output>
            </label>
            <label className="pr-q-row">
              <span>Swing</span>
              <input type="range" className="range" min={0} max={100} value={Math.round(q.swing * 100)} onChange={(e) => setQuantizeSettings({ ...q, swing: Number(e.target.value) / 100 })} />
              <output>{Math.round(q.swing * 100)}%</output>
            </label>
            <label className="pr-q-check">
              <input type="checkbox" checked={q.ends} onChange={(e) => setQuantizeSettings({ ...q, ends: e.target.checked })} />
              Quantize note ends too
            </label>
            <button
              className="btn sm btn-primary"
              type="button"
              onClick={() => {
                setAnchor(null);
                apply();
              }}
            >
              Quantize
            </button>
          </div>
        </Popover>
      )}
    </div>
  );
}

export function PianoRollView() {
  const target = useStore(useShallow(pianoTarget));
  const st = useStore.getState();
  if (!target) {
    return (
      <div className="piano">
        <div className="empty">
          <Icon name="piano" size={22} />
          <p>
            <b>No MIDI clip yet</b>
            Make one on a MIDI track, or import a MIDI file. Double-clicking an empty spot on a MIDI track makes one too.
            <span>
              <button className="btn sm btn-primary" type="button" onClick={() => void st.newMidiClip()}>
                New MIDI clip
              </button>
              <button className="btn sm" type="button" onClick={() => void st.importMidi()}>
                Import MIDI…
              </button>
            </span>
          </p>
        </div>
      </div>
    );
  }
  return <PianoRoll key={target.clip.id} track={target.track} clip={target.clip} />;
}

function PianoRoll({ track, clip }: { track: Track; clip: MidiClip }) {
  const selection = useStore((s) => s.pianoSelection);
  const grid = useStore((s) => s.pianoGrid);
  const rowsMode = useStore((s) => s.pianoRows);
  const snapScale = useStore((s) => s.snapToScale);
  const keyName = useStore((s) => s.project.key);
  const bpb = useStore((s) => s.project.beatsPerBar);
  const zoom = useStore((s) => s.pianoZoom);
  const tool = useStore((s) => s.tool);
  const mode = useStore((s) => s.pianoMode);
  const theme = useStore((s) => s.theme);
  const tracks = useStore((s) => s.project.tracks);
  const scenes = useStore((s) => s.project.scenes);
  const st = useStore.getState();

  const key = useMemo(() => parseKey(keyName), [keyName]);
  const inKey = useMemo(() => (key.scale === "chromatic" ? null : scaleClasses(key)), [key]);

  const scrollRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const velRef = useRef<HTMLCanvasElement>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [scroll, setScroll] = useState({ x: 0, y: 0 });
  const [preview, setPreviewState] = useState<Preview | null>(null);
  // Pointer handlers read the latest preview from here: several events can
  // arrive before React renders again, and a stale one must never be saved.
  const previewRef = useRef<Preview | null>(null);
  const setPreview = (next: Preview | null) => {
    previewRef.current = next;
    setPreviewState(next);
  };
  const [pressed, setPressed] = useState<number | null>(null);
  const drag = useRef<DragState | null>(null);
  const auditioned = useRef<number | null>(null);
  const lastLength = useRef(grid || 0.25);
  const centred = useRef(false);

  const notes = preview?.notes ?? clip.notes;
  const sel = preview?.selection ?? selection.filter((i) => i < clip.notes.length);
  const clipLength = preview?.clipLength ?? clip.length;
  const color = trackColor(track.color);

  // Ghost notes: what the other MIDI clips play meanwhile (drum parts only beside drum parts).
  const ghostsOn = useStore((s) => s.pianoGhosts);
  const ghosts = useMemo(() => {
    // A session clip plays whenever it is launched: nothing on the timeline is "meanwhile".
    if (!ghostsOn || clip.slot != null) return [];
    const drums = (t: Track) => t.devices.some((d) => d.type === "drum-rack");
    const out: { start: number; length: number; pitch: number; color: string }[] = [];
    for (const t of tracks) {
      if (drums(t) !== drums(track)) continue;
      for (const c of timelineClips(t)) {
        if (c.kind !== "midi" || c.id === clip.id || c.start >= clip.start + clip.length || c.start + c.length <= clip.start) continue;
        for (const n of c.notes) {
          if (n.start >= c.length) continue;
          const start = n.start + c.start - clip.start;
          const length = Math.min(n.length, c.length - n.start);
          if (start + length > 0 && start < clip.length) out.push({ start, length, pitch: n.pitch, color: trackColor(t.color) });
        }
      }
    }
    return out;
  }, [ghostsOn, tracks, track, clip]);

  const rows = useMemo(() => {
    const used = new Set(clip.notes.map((n) => n.pitch));
    const all = Array.from({ length: 128 }, (_, i) => 127 - i);
    if (rowsMode === "used" && used.size) return all.filter((p) => used.has(p));
    if (rowsMode === "scale" && inKey) return all.filter((p) => inKey.has(p % 12) || used.has(p));
    return all;
  }, [clip.notes, rowsMode, inKey]);
  const rowOf = useMemo(() => new Map(rows.map((p, i) => [p, i])), [rows]);

  const viewW = Math.max(1, size.w - KEYS_W);
  const ppb = clamp(zoom ?? viewW / Math.max(bpb, clip.length + bpb * 0.25), MIN_PPB, MAX_PPB);
  const contentW = KEYS_W + Math.max(clipLength + bpb * 2, viewW / ppb) * ppb;
  const contentH = RULER_H + rows.length * ROW_H;
  const latest = useRef({ ppb });
  latest.current = { ppb };
  pianoView.ppb = ppb;

  const xOf = (beat: number) => KEYS_W + beat * ppb - scroll.x;
  const yOf = (row: number) => RULER_H + row * ROW_H - scroll.y;
  const snapTo = (beats: number, free: boolean) => (free || !grid ? beats : Math.round(beats / grid) * grid);
  const floorTo = (beats: number, free: boolean) => (free || !grid ? beats : Math.floor(beats / grid + 1e-9) * grid);
  const fitPitch = (pitch: number) => (snapScale ? snapToScale(pitch, inKey) : pitch);

  // Size and scroll position of the view.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setSize({ w: el.clientWidth, h: el.clientHeight }));
    ro.observe(el);
    // The step view replaces the canvas; coming back must centre again.
    return () => {
      ro.disconnect();
      centred.current = false;
    };
  }, [mode]);

  // Start centred on the notes (or middle C).
  useEffect(() => {
    const el = scrollRef.current;
    if (!el || size.h === 0 || centred.current) return;
    centred.current = true;
    const pitches = clip.notes.map((n) => n.pitch);
    const middle = pitches.length ? (Math.min(...pitches) + Math.max(...pitches)) / 2 : 60;
    const row = rowOf.get(Math.round(middle)) ?? rows.findIndex((p) => p <= middle);
    el.scrollTop = Math.max(0, row * ROW_H - (size.h - RULER_H) / 2);
  }, [size.h, clip.notes, rowOf, rows]);

  // The wheel scrolls; Ctrl + wheel zooms around the pointer.
  useEffect(() => {
    const canvas = canvasRef.current;
    const el = scrollRef.current;
    if (!canvas || !el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      if (e.ctrlKey || e.metaKey) {
        const rect = canvas.getBoundingClientRect();
        const px = e.clientX - rect.left - KEYS_W;
        const { ppb: now } = latest.current;
        const beat = (px + el.scrollLeft) / now;
        const next = clamp(now * (e.deltaY < 0 ? 1.2 : 1 / 1.2), MIN_PPB, MAX_PPB);
        useStore.getState().setPianoZoom(next);
        requestAnimationFrame(() => (el.scrollLeft = Math.max(0, beat * next - px)));
      } else if (e.shiftKey) {
        el.scrollLeft += e.deltaY;
      } else {
        el.scrollTop += e.deltaY;
        el.scrollLeft += e.deltaX;
      }
    };
    canvas.addEventListener("wheel", onWheel, { passive: false });
    return () => canvas.removeEventListener("wheel", onWheel);
  }, [mode]);

  // Draw the ruler, keyboard, grid and notes.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || size.w === 0) return;
    const ctx = setupCanvas(canvas, size.w, size.h);
    if (!ctx) return;
    const c = palette();
    const { w, h } = size;
    ctx.clearRect(0, 0, w, h);
    ctx.fillStyle = c.bg;
    ctx.fillRect(0, 0, w, h);

    const firstRow = Math.max(0, Math.floor(scroll.y / ROW_H));
    const lastRow = Math.min(rows.length - 1, Math.ceil((scroll.y + h - RULER_H) / ROW_H));
    const firstBeat = Math.max(0, scroll.x / ppb);
    const lastBeat = (scroll.x + w - KEYS_W) / ppb;

    // Rows: black keys darker, the key's notes tinted, its root stronger.
    ctx.save();
    ctx.beginPath();
    ctx.rect(KEYS_W, RULER_H, w - KEYS_W, h - RULER_H);
    ctx.clip();
    for (let r = firstRow; r <= lastRow; r++) {
      const pitch = rows[r];
      const y = yOf(r);
      if (isBlackKey(pitch)) {
        ctx.fillStyle = c.black;
        ctx.fillRect(KEYS_W, y, w - KEYS_W, ROW_H);
      }
      if (inKey?.has(pitch % 12)) {
        ctx.fillStyle = c.accentSoft;
        ctx.globalAlpha = pitch % 12 === key.root ? 0.9 : 0.35;
        ctx.fillRect(KEYS_W, y, w - KEYS_W, ROW_H);
        ctx.globalAlpha = 1;
      }
      ctx.fillStyle = pitch % 12 === 0 ? c.line2 : c.line;
      ctx.fillRect(KEYS_W, y + ROW_H - 1, w - KEYS_W, 1);
    }

    // Grid: bars, beats and the snap grid when there is room for it.
    const step = grid || 0.25;
    const drawStep = step * ppb >= 6 ? step : ppb >= 6 ? 1 : bpb;
    for (let b = Math.floor(firstBeat / drawStep) * drawStep; b <= lastBeat + drawStep; b += drawStep) {
      const x = Math.round(xOf(b));
      const isBar = Math.abs(b / bpb - Math.round(b / bpb)) < 1e-6;
      const isBeat = Math.abs(b - Math.round(b)) < 1e-6;
      ctx.fillStyle = isBar ? c.line2 : isBeat ? c.bar : c.beat;
      ctx.fillRect(x, RULER_H, 1, h - RULER_H);
    }

    // Past the clip's end: dimmed.
    const endX = xOf(clipLength);
    if (endX < w) {
      ctx.fillStyle = "rgba(0,0,0,0.28)";
      ctx.fillRect(Math.max(KEYS_W, endX), RULER_H, w - Math.max(KEYS_W, endX), h - RULER_H);
    }

    // Ghost notes: outlines in their own track's colour, under this clip's notes.
    ctx.lineWidth = 1;
    for (const g of ghosts) {
      const r = rowOf.get(g.pitch);
      if (r === undefined || r < firstRow - 1 || r > lastRow + 1) continue;
      const x = xOf(g.start);
      const gw = Math.max(3, g.length * ppb - 1);
      if (x + gw < KEYS_W || x > w) continue;
      ctx.globalAlpha = 0.5;
      ctx.strokeStyle = g.color;
      ctx.beginPath();
      ctx.roundRect(x + 0.5, yOf(r) + 1.5, gw - 1, ROW_H - 3, 3);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;

    // Notes.
    const selected = new Set(sel);
    ctx.font = `500 10px ${getComputedStyle(document.body).fontFamily}`;
    ctx.textBaseline = "middle";
    notes.forEach((n, i) => {
      const r = rowOf.get(n.pitch);
      if (r === undefined || r < firstRow - 1 || r > lastRow + 1) return;
      const x = xOf(n.start);
      const nw = Math.max(3, n.length * ppb - 1);
      if (x + nw < KEYS_W || x > w) return;
      const y = yOf(r) + 1;
      const on = selected.has(i);
      ctx.globalAlpha = (0.4 + 0.6 * n.velocity) * (n.start >= clipLength ? 0.4 : 1);
      ctx.fillStyle = color;
      ctx.beginPath();
      ctx.roundRect(x, y, nw, ROW_H - 2, 3);
      ctx.fill();
      ctx.globalAlpha = 1;
      if (on) {
        ctx.strokeStyle = c.text1;
        ctx.lineWidth = 1.5;
        ctx.stroke();
      }
      if (nw > 34) {
        ctx.fillStyle = "rgba(0,0,0,0.65)";
        ctx.fillText(noteName(n.pitch), x + 4, y + (ROW_H - 2) / 2 + 0.5);
      }
    });

    // Marquee.
    const m = preview?.marquee;
    if (m) {
      const x0 = Math.min(m.x0, m.x1) - scroll.x;
      const y0 = Math.min(m.y0, m.y1) - scroll.y;
      ctx.fillStyle = c.accentSoft;
      ctx.strokeStyle = c.accent;
      ctx.lineWidth = 1;
      ctx.fillRect(x0, y0, Math.abs(m.x1 - m.x0), Math.abs(m.y1 - m.y0));
      ctx.strokeRect(x0 + 0.5, y0 + 0.5, Math.abs(m.x1 - m.x0), Math.abs(m.y1 - m.y0));
    }
    ctx.restore();

    // Keyboard.
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, RULER_H, KEYS_W, h - RULER_H);
    ctx.clip();
    for (let r = firstRow; r <= lastRow; r++) {
      const pitch = rows[r];
      const y = yOf(r);
      const black = isBlackKey(pitch);
      ctx.fillStyle = pitch === pressed ? c.accent : black ? c.keyBlack : c.keyWhite;
      ctx.fillRect(0, y, black ? KEYS_W * 0.62 : KEYS_W - 1, ROW_H - 1);
      if (black && pitch !== pressed) {
        ctx.fillStyle = c.keyWhite;
        ctx.fillRect(KEYS_W * 0.62, y, KEYS_W * 0.38 - 1, ROW_H - 1);
      }
      // Label every C, or every key when folded (rows are no longer a keyboard).
      if (pitch % 12 === 0 || rowsMode !== "all") {
        ctx.fillStyle = pitch === pressed ? "#fff" : c.text;
        ctx.textAlign = "right";
        ctx.fillText(noteName(pitch), KEYS_W - 5, y + ROW_H / 2);
        ctx.textAlign = "left";
      }
    }
    ctx.restore();

    // Ruler: bars of the clip, and its end handle.
    ctx.fillStyle = c.bg;
    ctx.fillRect(0, 0, w, RULER_H);
    ctx.fillStyle = c.line2;
    ctx.fillRect(0, RULER_H - 1, w, 1);
    ctx.save();
    ctx.beginPath();
    ctx.rect(KEYS_W, 0, w - KEYS_W, RULER_H);
    ctx.clip();
    const barPx = bpb * ppb;
    const every = barPx < 28 ? 4 : barPx < 56 ? 2 : 1;
    ctx.fillStyle = c.text;
    ctx.font = `500 10.5px ${getComputedStyle(document.body).fontFamily}`;
    for (let bar = Math.floor(firstBeat / bpb); bar * bpb <= lastBeat; bar++) {
      if (bar % every) continue;
      const x = Math.round(xOf(bar * bpb));
      ctx.fillRect(x, RULER_H - 8, 1, 8);
      ctx.fillText(String(bar + 1), x + 4, RULER_H / 2);
    }
    if (endX >= KEYS_W - 8 && endX <= w + 8) {
      ctx.fillStyle = c.accent;
      ctx.beginPath();
      ctx.moveTo(endX, RULER_H - 1);
      ctx.lineTo(endX - 7, 3);
      ctx.lineTo(endX, 3);
      ctx.closePath();
      ctx.fill();
      ctx.fillRect(endX - 1, 3, 2, RULER_H - 4);
    }
    ctx.restore();
    ctx.fillStyle = c.bg;
    ctx.fillRect(0, 0, KEYS_W, RULER_H);
    // `theme` is a dependency so the colours follow a theme switch.
  }, [size, scroll, rows, rowOf, notes, sel, ppb, grid, bpb, inKey, key.root, clipLength, color, preview, pressed, rowsMode, theme, ghosts]); // eslint-disable-line react-hooks/exhaustive-deps

  // Velocity lane.
  useEffect(() => {
    const canvas = velRef.current;
    if (!canvas || size.w === 0) return;
    const ctx = setupCanvas(canvas, size.w, VEL_H);
    if (!ctx) return;
    const c = palette();
    ctx.clearRect(0, 0, size.w, VEL_H);
    ctx.fillStyle = c.line;
    ctx.fillRect(KEYS_W, VEL_H / 2, size.w - KEYS_W, 1);
    const selected = new Set(sel);
    notes.forEach((n, i) => {
      const x = xOf(n.start);
      if (x < KEYS_W - 4 || x > size.w) return;
      const bar = Math.max(2, n.velocity * (VEL_H - 6));
      ctx.fillStyle = selected.has(i) ? c.text1 : color;
      ctx.globalAlpha = selected.has(i) ? 0.9 : 0.75;
      ctx.fillRect(x, VEL_H - bar, 3, bar);
      ctx.fillRect(x - 2, VEL_H - bar, 7, 2);
      ctx.globalAlpha = 1;
    });
    ctx.fillStyle = c.text;
    ctx.font = `500 10.5px ${getComputedStyle(document.body).fontFamily}`;
    ctx.textBaseline = "top";
    ctx.fillText("Velocity", 6, 6);
  }, [size.w, scroll.x, notes, sel, ppb, color, theme]); // eslint-disable-line react-hooks/exhaustive-deps

  // A note held by the keyboard or a drag sounds on this track's instrument.
  const audition = (pitch: number | null) => {
    const prev = auditioned.current;
    if (prev === pitch) return;
    if (prev !== null) engine.note(track.id, prev, 0, false);
    if (pitch !== null) engine.note(track.id, pitch, 0.8, true);
    auditioned.current = pitch;
    setPressed(pitch);
  };
  useEffect(() => () => audition(null), []); // eslint-disable-line react-hooks/exhaustive-deps

  const local = (e: { clientX: number; clientY: number }) => {
    const rect = canvasRef.current?.getBoundingClientRect();
    const x = e.clientX - (rect?.left ?? 0);
    const y = e.clientY - (rect?.top ?? 0);
    return { x, y, beat: (x - KEYS_W + scroll.x) / ppb, row: Math.floor((y - RULER_H + scroll.y) / ROW_H) };
  };

  const hitNote = (list: Note[], beat: number, row: number, x: number): { index: number; zone: "body" | "start" | "end" } | null => {
    const pitch = rows[row];
    if (pitch === undefined) return null;
    for (let i = list.length - 1; i >= 0; i--) {
      const n = list[i];
      if (n.pitch !== pitch) continue;
      const xs = xOf(n.start);
      const xe = xs + Math.max(3, n.length * ppb - 1);
      if (x < xs - 1 || x > xe + 2) continue;
      if (x >= xe - EDGE) return { index: i, zone: "end" };
      if (x <= xs + EDGE * 0.7 && xe - xs > 16) return { index: i, zone: "start" };
      if (beat >= n.start - 1e-9) return { index: i, zone: "body" };
    }
    return null;
  };

  /** Saves the preview's notes as one undo step, keeping its selection. */
  const save = async (next: Note[], selected: (i: number) => boolean, label: string) => {
    const edit = commit(next, selected);
    setPreview({ notes: edit.notes, selection: edit.selection });
    await st.setNotes(clip.id, edit.notes, label, edit.selection);
    setPreview(null);
  };

  const onPointerDown = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    if (e.button === 1) return;
    const { x, y, beat, row } = local(e);
    const base = clip.notes;
    const baseSel = selection.filter((i) => i < base.length);
    const start = (kind: DragKind, index = -1, extra: Partial<DragState> = {}) => {
      e.currentTarget.setPointerCapture(e.pointerId);
      drag.current = { kind, x0: e.clientX, y0: e.clientY, beat0: beat, row0: row, index, copy: false, additive: false, moved: false, base, selection: baseSel, ...extra };
    };

    if (y < RULER_H) {
      if (x < KEYS_W) return;
      if (Math.abs(x - xOf(clip.length)) <= 8) return start("clip-end");
      st.seekBeats(clip.start + Math.max(0, floorTo(beat, e.altKey)));
      return;
    }
    if (x < KEYS_W) {
      const pitch = rows[row];
      if (pitch === undefined) return;
      start("keys");
      audition(pitch);
      return;
    }
    const hit = hitNote(base, beat, row, x);
    if (e.button === 2) {
      if (hit) void save(remove(base, [hit.index]).notes, () => false, "Delete note");
      return;
    }
    if (tool === "erase") {
      start("erase");
      if (hit) setPreview({ notes: base.filter((_, i) => i !== hit.index), selection: [] });
      return;
    }
    if (tool === "draw") {
      if (hit) {
        void save(remove(base, [hit.index]).notes, () => false, "Delete note");
        return;
      }
      const pitch = rows[row];
      if (pitch === undefined) return;
      const note = { start: Math.max(0, floorTo(beat, e.altKey)), length: lastLength.current, pitch: fitPitch(pitch), velocity: 0.8 };
      setPreview({ notes: [...base, note], selection: [base.length] });
      start("draw", base.length);
      audition(note.pitch);
      return;
    }
    if (hit) {
      let next = baseSel;
      if (e.shiftKey) next = baseSel.includes(hit.index) ? baseSel.filter((i) => i !== hit.index) : [...baseSel, hit.index];
      else if (!baseSel.includes(hit.index)) next = [hit.index];
      st.setPianoSelection(next);
      const kind: DragKind = hit.zone === "body" ? "move" : hit.zone;
      start(kind, hit.index, { selection: next.includes(hit.index) ? next : [...next, hit.index], copy: e.ctrlKey || e.metaKey });
      audition(base[hit.index].pitch);
      return;
    }
    start("marquee", -1, { additive: e.shiftKey });
  };

  const onPointerMove = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    const d = drag.current;
    const { x, y, beat, row } = local(e);
    if (!d) {
      // Show what a press would do.
      const hit = y > RULER_H && x > KEYS_W ? hitNote(notes, beat, row, x) : null;
      const onEnd = y < RULER_H && Math.abs(x - xOf(clip.length)) <= 8;
      e.currentTarget.style.cursor = onEnd || hit?.zone === "end" || hit?.zone === "start" ? "ew-resize" : hit ? "grab" : "";
      return;
    }
    if (!d.moved && Math.abs(e.clientX - d.x0) < 3 && Math.abs(e.clientY - d.y0) < 3) return;
    d.moved = true;
    const free = e.altKey;
    const delta = beat - d.beat0;
    const chosen = new Set(d.selection);

    switch (d.kind) {
      case "keys": {
        const pitch = rows[clamp(row, 0, rows.length - 1)];
        audition(pitch);
        break;
      }
      case "move": {
        const anchor = d.base[d.index];
        let shift = snapTo(anchor.start + delta, free) - anchor.start;
        shift = Math.max(shift, -Math.min(...d.selection.map((i) => d.base[i].start)));
        const anchorRow = rowOf.get(anchor.pitch) ?? 0;
        const target = rows[clamp(anchorRow + row - d.row0, 0, rows.length - 1)];
        let dp = fitPitch(target) - anchor.pitch;
        if (d.selection.some((i) => d.base[i].pitch + dp < 0 || d.base[i].pitch + dp > 127)) dp = 0;
        const moved = (n: Note) => ({ ...n, start: n.start + shift, pitch: n.pitch + dp });
        if (d.copy) {
          const copies = d.selection.map((i) => moved(d.base[i]));
          setPreview({ notes: [...d.base, ...copies], selection: copies.map((_, k) => d.base.length + k) });
        } else {
          setPreview({ notes: d.base.map((n, i) => (chosen.has(i) ? moved(n) : n)), selection: d.selection });
        }
        audition(anchor.pitch + dp);
        break;
      }
      case "end": {
        const anchor = d.base[d.index];
        const end = Math.max(anchor.start + MIN_NOTE, snapTo(anchor.start + anchor.length + delta, free));
        const change = end - (anchor.start + anchor.length);
        setPreview({ notes: d.base.map((n, i) => (chosen.has(i) ? { ...n, length: Math.max(MIN_NOTE, n.length + change) } : n)), selection: d.selection });
        break;
      }
      case "start": {
        const anchor = d.base[d.index];
        const change = snapTo(anchor.start + delta, free) - anchor.start;
        setPreview({
          notes: d.base.map((n, i) => {
            if (!chosen.has(i)) return n;
            const end = n.start + n.length;
            const s = clamp(n.start + change, 0, end - MIN_NOTE);
            return { ...n, start: s, length: end - s };
          }),
          selection: d.selection,
        });
        break;
      }
      case "draw": {
        const drawn = previewRef.current?.notes[d.index];
        if (!drawn) break;
        const length = Math.max(grid || MIN_NOTE, snapTo(beat, free) - drawn.start);
        const next = [...d.base, { ...drawn, length }];
        setPreview({ notes: next, selection: [d.index] });
        break;
      }
      case "erase": {
        const current = previewRef.current?.notes ?? d.base;
        const hit = hitNote(current, beat, row, x);
        if (hit) setPreview({ notes: current.filter((_, i) => i !== hit.index), selection: [] });
        break;
      }
      case "marquee": {
        const rect = canvasRef.current?.getBoundingClientRect();
        const x0 = d.x0 - (rect?.left ?? 0) + scroll.x;
        const y0 = d.y0 - (rect?.top ?? 0) + scroll.y;
        const x1 = x + scroll.x;
        const y1 = y + scroll.y;
        const [b0, b1] = [(Math.min(x0, x1) - KEYS_W) / ppb, (Math.max(x0, x1) - KEYS_W) / ppb];
        const [r0, r1] = [Math.floor((Math.min(y0, y1) - RULER_H) / ROW_H), Math.floor((Math.max(y0, y1) - RULER_H) / ROW_H)];
        const inside = d.base.flatMap((n, i) => {
          const r = rowOf.get(n.pitch);
          return r !== undefined && r >= r0 && r <= r1 && n.start < b1 && n.start + n.length > b0 ? [i] : [];
        });
        const selected = d.additive ? [...new Set([...d.selection, ...inside])] : inside;
        setPreview({ notes: d.base, selection: selected, marquee: { x0, y0, x1, y1 } });
        break;
      }
      case "clip-end": {
        const length = Math.max(grid || 0.25, snapTo(beat, free));
        setPreview({ notes: d.base, selection: d.selection, clipLength: length });
        break;
      }
    }
  };

  const onPointerUp = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    const d = drag.current;
    drag.current = null;
    audition(null);
    if (!d) return;
    const p = previewRef.current;
    switch (d.kind) {
      case "move":
      case "start":
      case "end":
        if (d.moved && p) {
          const label = d.copy ? "Copy notes" : d.kind === "move" ? "Move notes" : "Resize notes";
          if (d.kind !== "move") lastLength.current = p.notes[d.index]?.length ?? lastLength.current;
          void save(p.notes, (i) => p.selection.includes(i), label);
        } else if (!d.moved && !e.shiftKey) {
          st.setPianoSelection([d.index]);
        }
        break;
      case "draw":
        if (p) {
          lastLength.current = p.notes[d.index]?.length ?? lastLength.current;
          void save(p.notes, (i) => i === d.index, "Draw note");
        }
        break;
      case "erase":
        if (p && p.notes.length !== d.base.length) void save(p.notes, () => false, "Erase notes");
        else setPreview(null);
        break;
      case "marquee":
        if (d.moved && p) st.setPianoSelection(p.selection);
        else {
          st.setPianoSelection([]);
          st.seekBeats(clip.start + Math.max(0, floorTo(d.beat0, e.altKey)));
        }
        setPreview(null);
        break;
      case "clip-end":
        if (d.moved && p?.clipLength) st.trimClip(clip.id, clip.start, p.clipLength);
        setPreview(null);
        break;
      default:
        setPreview(null);
    }
  };

  /** Double-click: add a note where there is none, delete the one that is there. */
  const onDoubleClick = (e: ReactMouseEvent<HTMLCanvasElement>) => {
    const { x, y, beat, row } = local(e);
    if (y < RULER_H || x < KEYS_W || tool !== "pointer") return;
    const hit = hitNote(clip.notes, beat, row, x);
    if (hit) {
      void save(remove(clip.notes, [hit.index]).notes, () => false, "Delete note");
      return;
    }
    const pitch = rows[row];
    if (pitch === undefined) return;
    const note = { start: Math.max(0, floorTo(beat, e.altKey)), length: lastLength.current, pitch: fitPitch(pitch), velocity: 0.8 };
    void save([...clip.notes, note], (i) => i === clip.notes.length, "Add note");
    engine.note(track.id, note.pitch, 0.8, true);
    window.setTimeout(() => engine.note(track.id, note.pitch, 0, false), 250);
  };

  // Velocity lane: drag a bar (or the selection's bars) up and down, or paint across.
  const velDrag = useRef<{ base: Note[]; index: number; y0: number; selection: number[]; painting: boolean } | null>(null);
  const velAt = (clientY: number) => {
    const rect = velRef.current?.getBoundingClientRect();
    return clamp(1 - (clientY - (rect?.top ?? 0) - 3) / (VEL_H - 6), 0.01, 1);
  };
  const velHit = (list: Note[], clientX: number) => {
    const rect = velRef.current?.getBoundingClientRect();
    const x = clientX - (rect?.left ?? 0);
    let best = -1;
    let dist = 7;
    list.forEach((n, i) => {
      const d = Math.abs(xOf(n.start) + 1 - x);
      if (d < dist || (d === dist && sel.includes(i))) {
        best = i;
        dist = d;
      }
    });
    return best;
  };
  const onVelDown = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    if (e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    const index = velHit(clip.notes, e.clientX);
    const selected = selection.filter((i) => i < clip.notes.length);
    velDrag.current = { base: clip.notes, index, y0: e.clientY, selection: index >= 0 && selected.includes(index) ? selected : index >= 0 ? [index] : [], painting: index < 0 };
    onVelMove(e);
  };
  const onVelMove = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    const d = velDrag.current;
    if (!d) return;
    const v = velAt(e.clientY);
    if (d.painting) {
      const current = previewRef.current?.notes ?? d.base;
      const i = velHit(current, e.clientX);
      if (i >= 0) setPreview({ notes: current.map((n, k) => (k === i ? { ...n, velocity: v } : n)), selection: sel });
      return;
    }
    const change = v - d.base[d.index].velocity;
    const chosen = new Set(d.selection);
    setPreview({
      notes: d.base.map((n, i) => (chosen.has(i) ? { ...n, velocity: clamp(i === d.index ? v : n.velocity + change, 0.01, 1) } : n)),
      selection: d.selection,
    });
  };
  const onVelUp = () => {
    const d = velDrag.current;
    velDrag.current = null;
    const p = previewRef.current;
    if (d && p) void save(p.notes, (i) => p.selection.includes(i), "Velocity");
    else setPreview(null);
  };

  const midiClips = tracks.flatMap((t) => t.clips.filter((c): c is MidiClip => c.kind === "midi").map((c) => ({ t, c })));
  const linkedCount = clip.link ? midiClips.filter(({ c }) => c.link === clip.link).length : 0;
  const count = sel.length;
  const chosen = sel.map((i) => notes[i]).filter(Boolean);
  const lowest = chosen.length ? Math.min(...chosen.map((n) => n.pitch)) : 0;
  const highest = chosen.length ? Math.max(...chosen.map((n) => n.pitch)) : 0;
  const velocities = chosen.map((n) => Math.round(n.velocity * 127));
  const busy = !!preview;

  return (
    <div className="piano" data-tool={tool}>
      <div className="ed-bar">
        <span className="selwrap">
          <select
            className="select"
            aria-label="Clip to edit"
            value={clip.id}
            onChange={(e) => {
              const hit = midiClips.find(({ c }) => c.id === e.target.value);
              if (hit) st.selectClip(hit.t.id, hit.c.id);
            }}
          >
            {midiClips.map(({ t, c }) => (
              <option key={c.id} value={c.id}>
                {t.name} — {c.name}
                {c.slot != null ? ` (session, ${scenes[c.slot] ?? `scene ${c.slot + 1}`})` : ""}
              </option>
            ))}
          </select>
          <Icon name="chevronDown" size={14} />
        </span>
        <div className="seg" role="group" aria-label="Editor">
          <button type="button" aria-pressed={mode === "notes"} onClick={() => st.setPianoMode("notes")}>
            <Icon name="piano" size={14} />
            Notes
          </button>
          <button type="button" aria-pressed={mode === "steps"} onClick={() => st.setPianoMode("steps")}>
            <Icon name="steps" size={14} />
            Steps
          </button>
        </div>
        {clip.link && (
          <span className="chip tip-b" data-tip="Linked clips share their notes: every edit here changes them all. Clip › Unlink clip makes this one its own.">
            <Icon name="link" size={13} />
            Linked ×{linkedCount}
          </span>
        )}
        <span className="selwrap tip-b" data-tip={mode === "steps" ? "Length of a step" : "Grid for drawing, moving and resizing  ·  hold Alt to place freely"}>
          <select className="select sm" aria-label="Grid" value={grid} onChange={(e) => st.setPianoGrid(Number(e.target.value))}>
            {GRIDS.map(([label, beats]) => (
              <option key={label} value={beats}>
                {label}
              </option>
            ))}
            <option value={0}>Off</option>
          </select>
          <Icon name="chevronDown" size={13} />
        </span>
        <span className="vsep" />
        <span className="selwrap">
          <select className="select sm" aria-label="Key" value={key.root} onChange={(e) => st.setKey(formatKey({ ...key, root: Number(e.target.value) }))}>
            {ROOTS.map((r, i) => (
              <option key={r} value={i}>
                {r}
              </option>
            ))}
          </select>
          <Icon name="chevronDown" size={13} />
        </span>
        <span className="selwrap">
          <select className="select sm" aria-label="Scale" value={key.scale} onChange={(e) => st.setKey(formatKey({ ...key, scale: e.target.value }))}>
            {SCALE_NAMES.map((s) => (
              <option key={s} value={s}>
                {s[0].toUpperCase() + s.slice(1)}
              </option>
            ))}
          </select>
          <Icon name="chevronDown" size={13} />
        </span>
        {mode === "notes" && (
          <>
            <div className="seg" role="group" aria-label="Rows">
              {ROW_MODES.map(([rowMode, label, tip]) => (
                <button key={rowMode} type="button" className="tip-b" data-tip={tip} aria-pressed={rowsMode === rowMode} onClick={() => st.setPianoRows(rowMode)}>
                  {label}
                </button>
              ))}
            </div>
            <button className="chipb tip-b" type="button" aria-pressed={snapScale} data-tip="Drawn and moved notes stay in the key" onClick={st.toggleSnapToScale}>
              Snap to scale
            </button>
            <button className="chipb tip-b" type="button" aria-pressed={ghostsOn} data-tip="Show, faded, what the other MIDI clips play at the same time" onClick={st.togglePianoGhosts}>
              Ghost notes
            </button>
          </>
        )}
        <span className="vsep" />
        <div className="ed-ops" role="group" aria-label="Notes">
          <QuantizeMenu disabled={busy || !clip.notes.length} />
          <NoteToolsMenu disabled={busy} />
          <button className="btn sm tip-b" type="button" disabled={busy || !clip.notes.length} data-tip="Each note lasts until the next one" onClick={() => st.editNotes(legato, "Legato")}>
            Legato
          </button>
          <button className="btn sm tip-b" type="button" disabled={busy || !count} data-tip="Copy the selection after itself  ·  Ctrl D" onClick={() => st.editNotes(duplicate, "Duplicate notes")}>
            Duplicate
          </button>
          <button
            className="ib tip-b"
            type="button"
            aria-label="Down an octave"
            data-tip="Down an octave  ·  Shift ↓"
            disabled={busy || !clip.notes.length}
            onClick={() => st.editNotes((n, s) => transpose(n, s, -12, inKey), "Transpose")}
          >
            <Icon name="chevronDown" size={15} />
          </button>
          <button
            className="ib tip-b"
            type="button"
            aria-label="Up an octave"
            data-tip="Up an octave  ·  Shift ↑ (↑ ↓ move by a scale step)"
            disabled={busy || !clip.notes.length}
            onClick={() => st.editNotes((n, s) => transpose(n, s, 12, inKey), "Transpose")}
          >
            <Icon name="chevronUp" size={15} />
          </button>
        </div>
        <div className="ed-zoom" role="group" aria-label="Zoom">
          <button className="ib tip-b" type="button" aria-label="Zoom out" data-tip="Zoom out  ·  Ctrl −" onClick={() => st.setPianoZoom(clamp(ppb / 1.5, MIN_PPB, MAX_PPB))}>
            <Icon name="minus" size={15} />
          </button>
          <button className="ib tip-b" type="button" aria-label="Zoom in" data-tip="Zoom in  ·  Ctrl + or Ctrl-scroll" onClick={() => st.setPianoZoom(clamp(ppb * 1.5, MIN_PPB, MAX_PPB))}>
            <Icon name="plus" size={15} />
          </button>
          <button className="btn sm tip-b" type="button" disabled={zoom === null} data-tip="Show the whole clip  ·  Ctrl 0" onClick={() => st.setPianoZoom(null)}>
            Fit
          </button>
          <button className="btn sm tip-b" type="button" data-tip="Save this clip as a .mid file" onClick={() => void st.exportMidi(clip.id)}>
            Export MIDI…
          </button>
        </div>
      </div>

      {mode === "steps" ? (
        <StepSequencer track={track} clip={clip} />
      ) : (
      <div className="pr-main card-inset">
        <div className="pr-view">
          <div ref={scrollRef} className="pr-scroll" onScroll={(e) => setScroll({ x: e.currentTarget.scrollLeft, y: e.currentTarget.scrollTop })}>
            <div style={{ width: contentW, height: contentH }} />
          </div>
          <canvas
            ref={canvasRef}
            className="pr-canvas"
            onPointerDown={onPointerDown}
            onPointerMove={onPointerMove}
            onPointerUp={onPointerUp}
            onPointerCancel={() => {
              drag.current = null;
              audition(null);
              setPreview(null);
            }}
            onDoubleClick={onDoubleClick}
            onContextMenu={(e) => e.preventDefault()}
            aria-label={`Notes of ${clip.name}`}
          />
          <Playhead clip={clip} ppb={ppb} scrollX={scroll.x} width={size.w} height={size.h} />
        </div>
        <canvas ref={velRef} className="pr-vel" onPointerDown={onVelDown} onPointerMove={onVelMove} onPointerUp={onVelUp} aria-label="Velocity" />
      </div>
      )}

      <div className="ed-foot">
        <dl className="ed-readout">
          <div>
            <dt>Notes</dt>
            <dd>{count ? `${count} of ${notes.length} selected` : notes.length}</dd>
          </div>
          {count > 0 && (
            <>
              <div>
                <dt>Pitch</dt>
                <dd>{lowest === highest ? noteName(lowest) : `${noteName(lowest)} – ${noteName(highest)}`}</dd>
              </div>
              <div>
                <dt>Velocity</dt>
                <dd>{Math.min(...velocities) === Math.max(...velocities) ? velocities[0] : `${Math.min(...velocities)} – ${Math.max(...velocities)}`}</dd>
              </div>
            </>
          )}
          <div>
            <dt>Length</dt>
            <dd>
              {Math.floor(clipLength / bpb)} bars{clipLength % bpb ? ` ${+(clipLength % bpb).toFixed(2)} beats` : ""}
            </dd>
          </div>
        </dl>
        <span className="hint" title="Double-click adds or deletes a note · drag to move, Ctrl-drag to copy · drag an edge to resize · right-click deletes · Alt places freely">
          <Icon name="info" size={14} />
          <span className="hint-text">
            {mode === "steps"
              ? "Click or drag across cells to switch steps on and off · click a row name to hear it · the lane sets the selected row's steps"
              : "Double-click adds or deletes a note · drag to move, Ctrl-drag to copy · drag an edge to resize · right-click deletes · Alt places freely"}
          </span>
        </span>
      </div>
    </div>
  );
}
