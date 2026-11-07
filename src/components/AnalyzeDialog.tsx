import { useEffect, useMemo, useState, type PointerEvent } from "react";
import { backend } from "../engine/backend";
import { formatClock } from "../state/format";
import { noteName } from "../state/music";
import { editorTarget, useStore } from "../state/store";
import type { SpectrumPlot } from "../state/types";
import { Icon } from "./Icon";
import { Select } from "./Select";

const W = 600;
const H = 240;
const LO = 20;
const SIZES = [1024, 2048, 4096, 8192, 16384];
const FLOOR = -100;

/** A frequency's nearest note and how far off it is, in cents. */
function nearestNote(hz: number): string {
  const midi = 69 + 12 * Math.log2(hz / 440);
  const n = Math.round(midi);
  if (n < 0 || n > 127) return "";
  const cents = Math.round((midi - n) * 100);
  return `${noteName(n)}${cents ? ` ${cents > 0 ? "+" : "−"}${Math.abs(cents)} ¢` : ""}`;
}

const hzLabel = (hz: number) => (hz >= 1000 ? `${(hz / 1000).toFixed(hz >= 10_000 ? 0 : 1)} kHz` : `${Math.round(hz)} Hz`);

/** Analyze › Plot spectrum: the selection's average spectrum, with the frequency and note under the pointer. */
function SpectrumPanel() {
  const clip = useStore((s) => editorTarget(s)?.clip.id ?? null);
  const selection = useStore((s) => s.editorSelection);
  const sel = selection && selection.end - selection.start > 0.001 ? selection : null;
  const [size, setSize] = useState(4096);
  const [plot, setPlot] = useState<SpectrumPlot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [hover, setHover] = useState<number | null>(null);

  useEffect(() => {
    if (!clip) return;
    let live = true;
    setError(null);
    backend
      .spectrumPlot(clip, sel?.start, sel?.end, size)
      .then((p) => live && setPlot(p))
      .catch((err) => live && setError(err instanceof Error ? err.message : String(err)));
    return () => {
      live = false;
    };
  }, [clip, sel?.start, sel?.end, size]);

  const nyquist = (plot?.sampleRate ?? 48_000) / 2;
  const xOf = (hz: number) => (Math.log(hz / LO) / Math.log(nyquist / LO)) * W;
  const hzAt = (x: number) => LO * (nyquist / LO) ** (x / W);
  const yOf = (db: number) => (Math.min(0, Math.max(FLOOR, db)) / FLOOR) * H;
  const binHz = plot ? plot.sampleRate / plot.size : 1;
  const line = useMemo(() => {
    if (!plot) return { d: "", x0: 0 };
    const pts: string[] = [];
    for (let k = 1; k < plot.db.length; k++) {
      const hz = k * binHz;
      if (hz < LO) continue;
      pts.push(`${pts.length ? "L" : "M"}${xOf(hz).toFixed(1)},${yOf(plot.db[k]).toFixed(1)}`);
    }
    const x0 = xOf(Math.ceil(LO / binHz) * binHz);
    return { d: pts.join(" "), x0 };
    // xOf and yOf follow plot.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [plot]);
  const at = hover !== null && plot ? { hz: hover, db: plot.db[Math.min(plot.db.length - 1, Math.round(hover / binHz))] } : null;
  const onMove = (e: PointerEvent<SVGSVGElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    setHover(hzAt(((e.clientX - r.left) / r.width) * W));
  };

  return (
    <>
      <p className="muted fx-desc">
        The average spectrum of {sel ? `the selection (${formatClock(sel.end - sel.start)})` : "the whole clip"}: each window's power, over every window and
        channel. A full-scale sine reads 0 dB.
      </p>
      {error ? (
        <div className="status-line warn">
          <Icon name="warning" size={15} />
          <span>{error}</span>
        </div>
      ) : (
        <svg className="plot" viewBox={`0 0 ${W} ${H}`} role="img" aria-label="Spectrum plot" onPointerMove={onMove} onPointerLeave={() => setHover(null)}>
          {[50, 100, 200, 500, 1000, 2000, 5000, 10_000, 20_000].filter((f) => xOf(f) < W - 24).map((f) => (
            <g key={f}>
              <line x1={xOf(f)} x2={xOf(f)} y1={0} y2={H} className="curve-grid" />
              <text x={xOf(f) + 3} y={H - 4} className="curve-label">
                {f >= 1000 ? `${f / 1000}k` : f}
              </text>
            </g>
          ))}
          {[-20, -40, -60, -80].map((db) => (
            <g key={db}>
              <line x1={0} x2={W} y1={yOf(db)} y2={yOf(db)} className="curve-grid" />
              <text x={3} y={yOf(db) - 3} className="curve-label">
                {db}
              </text>
            </g>
          ))}
          {line.d && <path d={`${line.d} L${W},${H} L${line.x0.toFixed(1)},${H} Z`} className="plot-fill" />}
          {line.d && <path d={line.d} className="curve-line" />}
          {at && <line x1={xOf(at.hz)} x2={xOf(at.hz)} y1={0} y2={H} className="plot-cursor" />}
        </svg>
      )}
      <div className="plot-readout mono" aria-live="polite">
        {at ? (
          <span>
            {hzLabel(at.hz)} · {nearestNote(at.hz)} · {at.db.toFixed(1)} dB
          </span>
        ) : plot ? (
          <span>
            Peak at {hzLabel(plot.peakHz)} · {nearestNote(plot.peakHz)}
          </span>
        ) : (
          <span className="muted">Measuring…</span>
        )}
        <span className="selwrap sm">
          <Select className="select" aria-label="Window size" value={size} onChange={(e) => setSize(Number(e.target.value))}>
            {SIZES.map((n) => (
              <option key={n} value={n}>
                {n} points · {((plot?.sampleRate ?? 48_000) / n).toFixed(1)} Hz apart
              </option>
            ))}
          </Select>
        </span>
      </div>
    </>
  );
}

interface Side {
  clip: string;
  start: number;
  end: number;
  db: number;
}

/** Analyze › Contrast: is speech at least 20 dB above the background (WCAG 2, 1.4.7)? */
function ContrastPanel() {
  const clip = useStore((s) => editorTarget(s)?.clip.id ?? null);
  const selection = useStore((s) => s.editorSelection);
  const sel = selection && selection.end - selection.start > 0.001 ? selection : null;
  const [fg, setFg] = useState<Side | null>(null);
  const [bg, setBg] = useState<Side | null>(null);
  const [error, setError] = useState<string | null>(null);

  const measure = async (set: (side: Side) => void) => {
    if (!clip || !sel) return;
    try {
      setError(null);
      set({ clip, start: sel.start, end: sel.end, db: await backend.clipLevel(clip, sel.start, sel.end) });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };
  const diff = fg && bg ? fg.db - bg.db : null;
  const row = (label: string, hint: string, side: Side | null, set: (s: Side) => void) => (
    <div className="contrast-row">
      <div>
        <b>{label}</b>
        <span className="muted">{side ? `${formatClock(side.start)} – ${formatClock(side.end)}` : hint}</span>
      </div>
      <span className="mono contrast-db">{side ? `${side.db.toFixed(1)} dB` : "–"}</span>
      <button className="btn sm" type="button" disabled={!sel} onClick={() => void measure(set)}>
        Use the selection
      </button>
    </div>
  );

  return (
    <>
      <p className="muted fx-desc">
        For speech to be understood over its background, WCAG 2 (1.4.7) asks for the background to be at least 20 dB quieter. Select a stretch of speech, then
        one of background only, and measure each.
      </p>
      {row("Foreground", "Select speech and press the button", fg, setFg)}
      {row("Background", "Select background only (no speech)", bg, setBg)}
      {error && (
        <div className="status-line warn">
          <Icon name="warning" size={15} />
          <span>{error}</span>
        </div>
      )}
      <div className={`status-line contrast-result${diff === null ? "" : diff >= 20 ? " pass" : " warn"}`}>
        <Icon name={diff === null ? "info" : diff >= 20 ? "check" : "warning"} size={15} />
        <span>
          {diff === null
            ? "Measure both to see the difference."
            : diff >= 20
              ? `${diff.toFixed(1)} dB apart: passes (20 dB or more).`
              : `${diff.toFixed(1)} dB apart: fails, by ${(20 - diff).toFixed(1)} dB. Lower the background (noise reduction, ducking) or raise the speech.`}
        </span>
      </div>
    </>
  );
}

export function AnalyzeDialog() {
  const mode = useStore((s) => s.analyzeDialog);
  const setMode = useStore((s) => s.setAnalyzeDialog);
  useEffect(() => {
    if (!mode) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setMode(null);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [mode, setMode]);
  if (!mode) return null;
  const title = mode === "spectrum" ? "Plot spectrum" : "Contrast";
  return (
    <div className="scrim" onPointerDown={(e) => e.target === e.currentTarget && setMode(null)}>
      <div className="dialog shine fx-dialog analyze-dialog" role="dialog" aria-modal="true" aria-labelledby="analyze-title">
        <header className="dialog-head">
          <h2 id="analyze-title">{title}</h2>
          <button className="ib" type="button" aria-label="Close" onClick={() => setMode(null)} autoFocus>
            <Icon name="close" size={16} />
          </button>
        </header>
        {mode === "spectrum" ? <SpectrumPanel /> : <ContrastPanel />}
        <footer className="dialog-foot">
          <span className="muted">Nothing is changed: this only measures.</span>
          <button className="btn" type="button" onClick={() => setMode(null)}>
            Close
          </button>
        </footer>
      </div>
    </div>
  );
}
