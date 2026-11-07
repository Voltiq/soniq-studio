import { useEffect, useRef, useState } from "react";
import { engine, inTauri, type AnalysisReading } from "../engine/bridge";
import { HISTORY_SECONDS, bandHz, clearAnalysis, loudnessHistory, useAnalysis } from "../state/analysis";
import { useStore } from "../state/store";
import { Icon } from "./Icon";

/** Loudness targets people master to, in LUFS integrated. */
const TARGETS: [string, number][] = [
  ["Streaming · −14", -14],
  ["Apple Music, podcasts · −16", -16],
  ["Broadcast EBU R128 · −23", -23],
  ["Film, ATSC A/85 · −24", -24],
];
const TARGET_KEY = "soniq.loudnessTarget";
/** Above this the master may clip once it is encoded (MP3, AAC, Opus). */
const PEAK_CEILING = -1;

const fmt = (v: number | null | undefined, digits = 1) => (v == null ? "–" : v.toFixed(digits).replace("-", "−"));
const fraction = (v: number | null | undefined, lo: number, hi: number) => (v == null ? 0 : Math.min(1, Math.max(0, (v - lo) / (hi - lo))));

function savedTarget(): number {
  try {
    const v = Number(localStorage.getItem(TARGET_KEY));
    return TARGETS.some(([, t]) => t === v) ? v : -14;
  } catch {
    return -14;
  }
}

/** A horizontal level bar from `lo` to `hi`, with an optional mark (the target). */
function Bar({ value, lo, hi, mark, hot }: { value: number | null | undefined; lo: number; hi: number; mark?: number; hot?: boolean }) {
  return (
    <span className={`an-bar${hot ? " hot" : ""}`}>
      <span className="an-fill" style={{ width: `${fraction(value, lo, hi) * 100}%` }} />
      {mark !== undefined && <span className="an-mark" style={{ left: `${fraction(mark, lo, hi) * 100}%` }} />}
    </span>
  );
}

/** Colours for canvases, read from the theme. */
function palette(el: HTMLElement) {
  const css = getComputedStyle(el);
  const v = (name: string) => css.getPropertyValue(name).trim();
  return { accent: v("--accent"), line: v("--line-2"), text: v("--text-3"), warn: v("--warning"), info: v("--info") };
}

function prepare(canvas: HTMLCanvasElement) {
  const ratio = window.devicePixelRatio || 1;
  const { width, height } = canvas.getBoundingClientRect();
  if (canvas.width !== Math.round(width * ratio) || canvas.height !== Math.round(height * ratio)) {
    canvas.width = Math.round(width * ratio);
    canvas.height = Math.round(height * ratio);
  }
  const ctx = canvas.getContext("2d");
  ctx?.setTransform(ratio, 0, 0, ratio, 0, 0);
  ctx?.clearRect(0, 0, width, height);
  return { ctx, width, height };
}

const SPECTRUM_TOP = 0;
const SPECTRUM_BOTTOM = -90;
const LABELS: [number, string][] = [
  [50, "50"],
  [100, "100"],
  [500, "500"],
  [1000, "1k"],
  [5000, "5k"],
  [10000, "10k"],
];

function drawSpectrum(canvas: HTMLCanvasElement, bands: number[]) {
  const { ctx, width, height } = prepare(canvas);
  if (!ctx) return;
  const colors = palette(canvas);
  const x = (hz: number) => (Math.log2(hz / 20) / Math.log2(20000 / 20)) * width;
  const y = (db: number) => fraction(db, SPECTRUM_BOTTOM, SPECTRUM_TOP) * -height + height;
  ctx.strokeStyle = colors.line;
  ctx.fillStyle = colors.text;
  ctx.font = "10px var(--font, sans-serif)";
  ctx.lineWidth = 1;
  for (const [hz, label] of LABELS) {
    ctx.beginPath();
    ctx.moveTo(Math.round(x(hz)) + 0.5, 0);
    ctx.lineTo(Math.round(x(hz)) + 0.5, height);
    ctx.stroke();
    ctx.fillText(label, x(hz) + 3, height - 3);
  }
  for (const db of [-30, -60]) {
    ctx.beginPath();
    ctx.moveTo(0, Math.round(y(db)) + 0.5);
    ctx.lineTo(width, Math.round(y(db)) + 0.5);
    ctx.stroke();
    ctx.fillText(`${db}`.replace("-", "−"), 3, y(db) - 3);
  }
  if (!bands.length) return;
  ctx.beginPath();
  ctx.moveTo(0, height);
  bands.forEach((db, k) => ctx.lineTo(x(bandHz(k)), y(db)));
  ctx.lineTo(width, height);
  ctx.closePath();
  ctx.globalAlpha = 0.22;
  ctx.fillStyle = colors.accent;
  ctx.fill();
  ctx.globalAlpha = 1;
  ctx.beginPath();
  bands.forEach((db, k) => (k ? ctx.lineTo(x(bandHz(k)), y(db)) : ctx.moveTo(x(bandHz(k)), y(db))));
  ctx.strokeStyle = colors.accent;
  ctx.lineWidth = 1.5;
  ctx.stroke();
}

const HISTORY_TOP = 0;
const HISTORY_BOTTOM = -40;

function drawHistory(canvas: HTMLCanvasElement, target: number) {
  const { ctx, width, height } = prepare(canvas);
  if (!ctx) return;
  const colors = palette(canvas);
  const y = (lufs: number) => height - fraction(lufs, HISTORY_BOTTOM, HISTORY_TOP) * height;
  ctx.setLineDash([3, 3]);
  ctx.strokeStyle = colors.warn;
  ctx.beginPath();
  ctx.moveTo(0, Math.round(y(target)) + 0.5);
  ctx.lineTo(width, Math.round(y(target)) + 0.5);
  ctx.stroke();
  ctx.setLineDash([]);
  const length = HISTORY_SECONDS * 30;
  const line = (values: (number | null)[], color: string, lineWidth: number) => {
    ctx.beginPath();
    let drawing = false;
    values.forEach((v, i) => {
      const px = width - ((values.length - 1 - i) / (length - 1)) * width;
      if (v == null) {
        drawing = false;
        return;
      }
      if (drawing) ctx.lineTo(px, y(v));
      else ctx.moveTo(px, y(v));
      drawing = true;
    });
    ctx.strokeStyle = color;
    ctx.lineWidth = lineWidth;
    ctx.stroke();
  };
  line(loudnessHistory.momentary, colors.info, 1);
  line(loudnessHistory.shortTerm, colors.accent, 2);
}

function Loudness({ reading, target }: { reading: AnalysisReading | null; target: number }) {
  const integrated = reading?.integrated ?? null;
  const off = integrated == null ? null : integrated - target;
  return (
    <section className="an-section" aria-label="Loudness">
      <div className="an-big">
        <span className="an-value">{fmt(integrated)}</span>
        <span className="an-unit">LUFS integrated</span>
      </div>
      <p className={`an-verdict${off != null && Math.abs(off) > 1 ? " warn" : ""}`}>
        {off == null ? "Play the song through to measure it" : Math.abs(off) <= 1 ? "On target" : `${fmt(Math.abs(off))} LU ${off > 0 ? "louder" : "quieter"} than the target`}
      </p>
      <dl className="an-rows">
        <dt>Momentary</dt>
        <dd>
          <Bar value={reading?.momentary} lo={-40} hi={0} mark={target} />
          <span>{fmt(reading?.momentary)}</span>
        </dd>
        <dt>Short-term</dt>
        <dd>
          <Bar value={reading?.shortTerm} lo={-40} hi={0} mark={target} />
          <span>{fmt(reading?.shortTerm)}</span>
        </dd>
        <dt>Range</dt>
        <dd>
          <Bar value={reading?.range} lo={0} hi={20} />
          <span>{fmt(reading?.range)} LU</span>
        </dd>
      </dl>
    </section>
  );
}

function Levels({ reading }: { reading: AnalysisReading | null }) {
  const max = reading?.truePeak ?? null;
  const over = max != null && max > PEAK_CEILING;
  return (
    <section className="an-section" aria-label="Peaks and RMS">
      <div className="an-head">
        <span>True peak</span>
        <b className={over ? "warn" : undefined}>{fmt(max)} dBTP</b>
      </div>
      {over && <p className="an-verdict warn">Over −1 dBTP: it may clip once encoded. Lower the master or add a limiter.</p>}
      <dl className="an-rows">
        {(["L", "R"] as const).map((side, ch) => (
          <div key={side} className="an-pair">
            <dt>Peak {side}</dt>
            <dd>
              <Bar value={reading?.peak[ch]} lo={-60} hi={3} hot={(reading?.peak[ch] ?? -99) > PEAK_CEILING} />
              <span>{fmt(reading?.peak[ch])}</span>
            </dd>
          </div>
        ))}
        {(["L", "R"] as const).map((side, ch) => (
          <div key={side} className="an-pair">
            <dt>RMS {side}</dt>
            <dd>
              <Bar value={reading?.rms[ch]} lo={-60} hi={0} />
              <span>{fmt(reading?.rms[ch])}</span>
            </dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

function Correlation({ value }: { value: number | null | undefined }) {
  const at = value == null ? 50 : ((value + 1) / 2) * 100;
  return (
    <section className="an-section" aria-label="Stereo correlation">
      <div className="an-head">
        <span>Correlation</span>
        <b className={value != null && value < 0 ? "warn" : undefined}>{fmt(value, 2)}</b>
      </div>
      <div className="an-corr" role="meter" aria-valuemin={-1} aria-valuemax={1} aria-valuenow={value ?? 0} aria-label="Stereo correlation">
        <span className="an-corr-mid" />
        {value != null && <span className={`an-needle${value < 0 ? " warn" : ""}`} style={{ left: `${at}%` }} />}
      </div>
      <div className="an-scale">
        <span>−1 out of phase</span>
        <span>+1 mono</span>
      </div>
    </section>
  );
}

/** The master measured as it plays: loudness, peaks, stereo image and spectrum. */
export function AnalysisPanel() {
  const reading = useAnalysis();
  const toggleAnalysis = useStore((s) => s.toggleAnalysis);
  const [target, setTarget] = useState(savedTarget);
  const spectrum = useRef<HTMLCanvasElement>(null);
  const history = useRef<HTMLCanvasElement>(null);

  // The engine measures only while the panel is open.
  useEffect(() => {
    if (!inTauri) return;
    void engine.analysis(true);
    return () => {
      void engine.analysis(false);
      clearAnalysis();
    };
  }, []);

  useEffect(() => {
    if (spectrum.current) drawSpectrum(spectrum.current, reading?.spectrum ?? []);
    if (history.current) drawHistory(history.current, target);
  }, [reading, target]);

  const pickTarget = (value: number) => {
    setTarget(value);
    try {
      localStorage.setItem(TARGET_KEY, String(value));
    } catch {
      // Remembered for this session only.
    }
  };

  return (
    <aside className="analysis card shine" aria-label="Meters and analysis">
      <div className="ph">
        <h2 className="pt">Meters</h2>
        <span className="an-actions">
          <button
            className="btn sm"
            type="button"
            onClick={() => {
              void engine.analysisReset();
              clearAnalysis();
            }}
            title="Start the integrated loudness, range and true peak over"
          >
            Reset
          </button>
          <button className="ib xs" type="button" aria-label="Close the meters" onClick={toggleAnalysis}>
            <Icon name="close" size={14} />
          </button>
        </span>
      </div>
      {!inTauri && <p className="muted an-note">The meters measure the engine's output: open the desktop app (npm run tauri dev).</p>}
      <label className="an-target">
        <span>Target</span>
        <select value={target} onChange={(e) => pickTarget(Number(e.target.value))} aria-label="Loudness target">
          {TARGETS.map(([label, value]) => (
            <option key={value + label} value={value}>
              {label} LUFS
            </option>
          ))}
        </select>
      </label>
      <div className="an-scroll">
        <Loudness reading={reading} target={target} />
        <section className="an-section" aria-label="Loudness over time">
          <div className="an-head">
            <span>Last {HISTORY_SECONDS} s</span>
            <span className="an-legend">
              <i className="m" /> momentary <i className="s" /> short-term
            </span>
          </div>
          <canvas ref={history} className="an-canvas history" aria-hidden="true" />
        </section>
        <Levels reading={reading} />
        <Correlation value={reading?.correlation} />
        <section className="an-section" aria-label="Spectrum">
          <div className="an-head">
            <span>Spectrum</span>
            <span className="muted">dB · Hz</span>
          </div>
          <canvas ref={spectrum} className="an-canvas spectrum" aria-hidden="true" />
        </section>
      </div>
    </aside>
  );
}
