import { useEffect, useRef } from "react";
import type { Note } from "../../state/types";

// Browsers refuse canvases wider than ~16k device pixels, so very zoomed-in
// clips render at a reduced pixel ratio instead of failing.
const MAX_CANVAS_PX = 16000;

function setup(canvas: HTMLCanvasElement, width: number, height: number) {
  const ratio = Math.min(window.devicePixelRatio || 1, MAX_CANVAS_PX / Math.max(1, width));
  canvas.width = Math.max(1, Math.round(width * ratio));
  canvas.height = Math.max(1, Math.round(height * ratio));
  canvas.style.width = `${width}px`;
  canvas.style.height = `${height}px`;
  const ctx = canvas.getContext("2d");
  ctx?.setTransform(ratio, 0, 0, ratio, 0, 0);
  return ctx;
}

function peakAt(peaks: Float32Array, from: number, to: number) {
  let m = 0;
  for (let i = from; i < Math.max(to, from + 1) && i < peaks.length; i++) if (peaks[i] > m) m = peaks[i];
  return m;
}

interface WaveformProps {
  peaks: Float32Array;
  /** Second channel; when set, the max of both is drawn. */
  peaksR?: Float32Array;
  width: number;
  height: number;
  color: string;
  /** Draw the 0 line and the −6 dB guides (editor view). */
  guides?: boolean;
}

/** Mirrored peak waveform, one column per CSS pixel. */
export function Waveform({ peaks, peaksR, width, height, color, guides }: WaveformProps) {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const ctx = setup(canvas, width, height);
    if (!ctx) return;
    ctx.clearRect(0, 0, width, height);
    const mid = height / 2;
    const n = peaks.length;
    const cols = Math.max(1, Math.floor(width));

    if (guides) {
      ctx.fillStyle = color;
      ctx.globalAlpha = 0.18;
      ctx.fillRect(0, Math.round(mid), cols, 1);
      ctx.globalAlpha = 0.08;
      const six = (mid - 2) * 0.5;
      ctx.fillRect(0, Math.round(mid - six), cols, 1);
      ctx.fillRect(0, Math.round(mid + six), cols, 1);
    }

    ctx.globalAlpha = 1;
    ctx.fillStyle = color;
    ctx.beginPath();
    for (let x = 0; x < cols; x++) {
      const i0 = Math.floor((x / cols) * n);
      const i1 = Math.floor(((x + 1) / cols) * n);
      const p = Math.max(peakAt(peaks, i0, i1), peaksR ? peakAt(peaksR, i0, i1) : 0);
      const h = Math.max(0.5, p * (mid - 1));
      ctx.rect(x, mid - h, 1, h * 2);
    }
    ctx.fill();
  }, [peaks, peaksR, width, height, color, guides]);

  return <canvas ref={ref} className="clip-canvas" aria-hidden="true" />;
}

interface NotesProps {
  notes: Note[];
  lengthBeats: number;
  width: number;
  height: number;
  color: string;
}

/** Piano-roll style preview of a MIDI clip, fitted to the clip's pitch range. */
export function NotePreview({ notes, lengthBeats, width, height, color }: NotesProps) {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const ctx = setup(canvas, width, height);
    if (!ctx) return;
    ctx.clearRect(0, 0, width, height);
    if (!notes.length) return;
    let lo = 127;
    let hi = 0;
    for (const n of notes) {
      lo = Math.min(lo, n.pitch);
      hi = Math.max(hi, n.pitch);
    }
    const rows = hi - lo + 3;
    const rowH = Math.max(1.5, (height - 6) / rows);
    const pxPerBeat = width / lengthBeats;
    ctx.fillStyle = color;
    for (const n of notes) {
      const x = n.start * pxPerBeat;
      const w = Math.max(1.5, n.length * pxPerBeat - 1);
      const y = 3 + (hi + 1 - n.pitch) * rowH;
      ctx.globalAlpha = 0.45 + 0.55 * n.velocity;
      ctx.beginPath();
      ctx.roundRect(x, y, w, Math.max(1.5, rowH - 1), Math.min(2, rowH / 2));
      ctx.fill();
    }
  }, [notes, lengthBeats, width, height, color]);

  return <canvas ref={ref} className="clip-canvas" aria-hidden="true" />;
}
