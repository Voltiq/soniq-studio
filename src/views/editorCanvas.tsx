/*
 * What the audio editor draws: spectrograms, waveform detail down to single
 * samples, and the frequency scales they share with the Rust side.
 */
import { useEffect, useRef, useState } from "react";
import type { SpectrogramData, WaveWindow } from "../engine/backend";
import type { FrequencyScale } from "../state/types";

/** Mirrors `soniq_io::FrequencyScale::hz_at`: position 0 is the bottom, 1 the top. */
export function hzAt(pos: number, nyquist: number, scale: FrequencyScale): number {
  const p = Math.min(1, Math.max(0, pos));
  if (scale === "linear") return p * nyquist;
  if (scale === "log") return 20 * Math.pow(nyquist / 20, p);
  const mel = (f: number) => 2595 * Math.log10(1 + f / 700);
  return 700 * (Math.pow(10, (p * mel(nyquist)) / 2595) - 1);
}

export function posOf(hz: number, nyquist: number, scale: FrequencyScale): number {
  const f = Math.min(nyquist, Math.max(0, hz));
  if (scale === "linear") return f / nyquist;
  if (scale === "log") return f <= 20 ? 0 : Math.log(f / 20) / Math.log(nyquist / 20);
  const mel = (x: number) => 2595 * Math.log10(1 + x / 700);
  return mel(f) / mel(nyquist);
}

/** Frequencies worth a label on each scale. */
export function frequencyTicks(nyquist: number, scale: FrequencyScale): number[] {
  const ticks =
    scale === "linear"
      ? Array.from({ length: 12 }, (_, i) => (i + 1) * (nyquist > 30000 ? 5000 : 2000))
      : scale === "log"
        ? [50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000]
        : [100, 250, 500, 1000, 2000, 4000, 8000, 16000];
  return ticks.filter((f) => f < nyquist * 0.97);
}

export function formatHz(hz: number): string {
  return hz >= 1000 ? `${Number((hz / 1000).toFixed(hz >= 10000 ? 0 : 1))}k` : String(Math.round(hz));
}

/** Debounced, latest-wins loading of data that depends on `key`; keeps the last result while the next one loads. */
export function useLatest<T>(key: string | null, load: () => Promise<T>, delay = 40): T | null {
  const [state, setState] = useState<T | null>(null);
  const loader = useRef(load);
  loader.current = load;
  useEffect(() => {
    if (!key) {
      setState(null);
      return;
    }
    let live = true;
    const id = window.setTimeout(() => {
      loader.current().then(
        (value) => live && setState(value),
        () => {},
      );
    }, delay);
    return () => {
      live = false;
      window.clearTimeout(id);
    };
  }, [key, delay]);
  return key ? state : null;
}

// ---------- Spectrogram ----------

let lut: { accent: string; colors: Uint8ClampedArray } | null = null;

function parseColor(css: string): [number, number, number] {
  const m = css.trim().match(/^#([0-9a-f]{6})$/i);
  if (!m) return [100, 87, 240];
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

/** 256 colours from near-black through the accent to warm white. */
function colormap(): Uint8ClampedArray {
  const accent = getComputedStyle(document.documentElement).getPropertyValue("--accent") || "#6457f0";
  if (lut?.accent === accent) return lut.colors;
  const stops: [number, [number, number, number]][] = [
    [0, [12, 12, 18]],
    [0.3, parseColor(accent).map((c) => c * 0.45) as [number, number, number]],
    [0.52, parseColor(accent)],
    [0.72, [232, 93, 154]],
    [0.88, [255, 182, 92]],
    [1, [255, 248, 228]],
  ];
  const colors = new Uint8ClampedArray(256 * 4);
  for (let i = 0; i < 256; i++) {
    const t = i / 255;
    const k = Math.max(0, stops.findIndex(([at]) => at >= t) - 1);
    const [a, ca] = stops[k];
    const [b, cb] = stops[Math.min(k + 1, stops.length - 1)];
    const f = b > a ? (t - a) / (b - a) : 0;
    for (let c = 0; c < 3; c++) colors[i * 4 + c] = ca[c] + (cb[c] - ca[c]) * f;
    colors[i * 4 + 3] = 255;
  }
  lut = { accent, colors };
  return colors;
}

export function SpectrogramCanvas({ data, channel, width, height }: { data: SpectrogramData; channel: number; width: number; height: number }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const canvas = ref.current;
    const bytes = data.channels[Math.min(channel, data.channels.length - 1)];
    if (!canvas || !bytes) return;
    canvas.width = data.columns;
    canvas.height = data.rows;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const image = ctx.createImageData(data.columns, data.rows);
    const colors = colormap();
    for (let i = 0; i < bytes.length; i++) image.data.set(colors.subarray(bytes[i] * 4, bytes[i] * 4 + 4), i * 4);
    ctx.putImageData(image, 0, 0);
  }, [data, channel]);
  return <canvas ref={ref} className="spectro-canvas" style={{ width, height }} aria-hidden="true" />;
}

// ---------- Waveform detail ----------

interface DetailProps {
  data: WaveWindow;
  channel: number;
  width: number;
  height: number;
  color: string;
  /** Visible range in seconds of the asset. */
  viewStart: number;
  viewEnd: number;
  /** Samples being drawn by hand, replacing this channel's raw samples. */
  override?: Float32Array | null;
}

/** Min/max columns when zoomed out, or a line through every sample (with dots when they are far apart). */
export function DetailWave({ data, channel, width, height, color, viewStart, viewEnd, override }: DetailProps) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const ratio = window.devicePixelRatio || 1;
    canvas.width = Math.max(1, Math.round(width * ratio));
    canvas.height = Math.max(1, Math.round(height * ratio));
    canvas.style.width = `${width}px`;
    canvas.style.height = `${height}px`;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
    ctx.clearRect(0, 0, width, height);
    const mid = height / 2;
    const amp = mid - 2;
    const values = override ?? data.channels[Math.min(channel, data.channels.length - 1)];
    if (!values) return;

    ctx.fillStyle = color;
    ctx.globalAlpha = 0.18;
    ctx.fillRect(0, Math.round(mid), width, 1);
    ctx.globalAlpha = 1;

    if (!data.raw) {
      ctx.beginPath();
      for (let x = 0; x < data.count; x++) {
        const lo = values[x * 2];
        const hi = values[x * 2 + 1];
        const top = mid - hi * amp;
        ctx.rect((x * width) / data.count, top, Math.max(1, width / data.count), Math.max(1, (hi - lo) * amp));
      }
      ctx.fill();
      return;
    }
    const span = viewEnd - viewStart;
    const xOf = (i: number) => (((data.first + i) / data.sampleRate - viewStart) / span) * width;
    ctx.strokeStyle = color;
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    for (let i = 0; i < values.length; i++) {
      const x = xOf(i);
      const y = mid - values[i] * amp;
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.stroke();
    // Far enough apart to grab: show each sample.
    if (values.length > 1 && xOf(1) - xOf(0) >= 6) {
      for (let i = 0; i < values.length; i++) {
        ctx.beginPath();
        ctx.arc(xOf(i), mid - values[i] * amp, 2.5, 0, Math.PI * 2);
        ctx.fill();
      }
    }
  }, [data, channel, width, height, color, viewStart, viewEnd, override]);
  return <canvas ref={ref} className="clip-canvas" aria-hidden="true" />;
}
