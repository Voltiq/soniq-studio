import { useEffect, useRef, useState, type PointerEvent } from "react";
import { EQ_BANDS, bandResponse, eqBands, eqResponse, hasGain } from "../state/eqCurve";
import { engine } from "../engine/bridge";
import { deviceReduction, deviceSpectrum } from "../state/meters";
import { useStore } from "../state/store";
import type { Device } from "../state/types";
import { ParamControl, paramOf, type Owner } from "./DeviceControls";

const W = 340;
const H = 128;
const DB = 24;
const F_MIN = 20;
const F_MAX = 20_000;
const FREQS = Array.from({ length: 161 }, (_, i) => F_MIN * Math.pow(F_MAX / F_MIN, i / 160));
const GRID = [50, 100, 200, 500, 1000, 2000, 5000, 10_000];
const TYPE_GLYPH = ["⟋", "⌊", "∩", "∨", "⌉", "⟍"];

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
const xOf = (f: number) => (Math.log(f / F_MIN) / Math.log(F_MAX / F_MIN)) * W;
const fOf = (x: number) => F_MIN * Math.pow(F_MAX / F_MIN, clamp(x / W, 0, 1));
const yOf = (db: number) => H / 2 - (clamp(db, -DB, DB) / DB) * (H / 2 - 6);
const dbOf = (y: number) => clamp(((H / 2 - y) / (H / 2 - 6)) * DB, -DB, DB);
const path = (dbs: number[]) => FREQS.map((f, i) => `${i ? "L" : "M"}${xOf(f).toFixed(1)},${yOf(dbs[i]).toFixed(1)}`).join(" ");
/** Three significant figures: 1234 Hz → 1230 Hz. */
const round3 = (f: number) => {
  const p = Math.pow(10, Math.floor(Math.log10(f)) - 2);
  return Math.round(f / p) * p;
};

/**
 * The EQ's curve, with a point for each band that is on: drag it to move the
 * band (left and right for frequency, up and down for gain), turn the wheel
 * over it for Q, double-click it to switch it off, double-click empty space
 * to add a band there. The selected band's controls sit underneath.
 */
/** How far a dynamics device turns the sound down, live: a bar and the dB. */
export function ReductionMeter({ id }: { id: string }) {
  const bar = useRef<HTMLSpanElement>(null);
  const text = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    let raf = 0;
    let shown = "";
    const loop = () => {
      const db = deviceReduction.get(id) ?? 0;
      if (bar.current) bar.current.style.width = `${Math.min(100, (db / 24) * 100).toFixed(1)}%`;
      const label = db >= 0.1 ? `−${db.toFixed(1)}` : "0.0";
      if (text.current && label !== shown) text.current.textContent = shown = label;
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, [id]);
  return (
    <span className="gr-meter" title="Gain reduction: how far it turns the sound down right now (dB)">
      <span className="gr-label">GR</span>
      <span className="gr-well">
        <span ref={bar} className="gr-fill" />
      </span>
      <span ref={text} className="gr-db mono">
        0.0
      </span>
    </span>
  );
}

/** What the EQ puts out, live, behind its curve (while the panel is open). */
function LiveSpectrum({ id }: { id: string }) {
  const area = useRef<SVGPathElement>(null);
  useEffect(() => {
    void engine.deviceSpectrum(id);
    let raf = 0;
    const loop = () => {
      const bands = deviceSpectrum.id === id ? deviceSpectrum.bands : [];
      if (area.current) {
        if (bands.length) {
          // −84 dB at the bottom of the plot, 0 dB at the top.
          const pts = bands.map((db, k) => `${xOf(20 * 2 ** (k / 6)).toFixed(1)},${(H * (1 - clamp((db + 84) / 84, 0, 1))).toFixed(1)}`);
          area.current.setAttribute("d", `M0,${H} L${pts.join(" L")} L${W},${H} Z`);
        } else area.current.setAttribute("d", "");
      }
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => {
      cancelAnimationFrame(raf);
      void engine.deviceSpectrum(null);
    };
  }, [id]);
  return <path ref={area} className="eq-spectrum" />;
}

export function EqEditor({ track, device }: { track: Owner; device: Device }) {
  const setParam = useStore((s) => s.setParam);
  const bands = eqBands(device);
  const [selected, setSelected] = useState(3);
  const svg = useRef<SVGSVGElement>(null);
  const dragging = useRef<number | null>(null);
  const set = (band: number, key: string, value: number) => setParam(track.id, device.id, `b${band + 1}${key}`, value);
  const curve = eqResponse(bands, FREQS, paramOf(device, "output", 0));
  const band = bands[selected];

  // Q on the wheel, over the plot: a native listener, so the panel does not scroll.
  const latest = useRef({ band, selected, set });
  latest.current = { band, selected, set };
  useEffect(() => {
    const el = svg.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      const { band, selected, set } = latest.current;
      if (!band.on || !device.on) return;
      e.preventDefault();
      set(selected, "q", clamp(band.q * (e.deltaY < 0 ? 1.12 : 1 / 1.12), 0.1, 18));
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [device.on]);

  const local = (e: PointerEvent) => {
    const r = svg.current!.getBoundingClientRect();
    return { x: ((e.clientX - r.left) * W) / r.width, y: ((e.clientY - r.top) * H) / r.height };
  };
  const move = (e: PointerEvent) => {
    const i = dragging.current;
    if (i === null) return;
    const { x, y } = local(e);
    set(i, "freq", round3(fOf(x)));
    if (hasGain(bands[i].type)) set(i, "gain", Math.round(dbOf(y) * 10) / 10);
  };
  const addBand = (e: React.MouseEvent) => {
    const free = bands.findIndex((b) => !b.on);
    if (free < 0 || !device.on) return;
    const r = svg.current!.getBoundingClientRect();
    const x = ((e.clientX - r.left) * W) / r.width;
    const y = ((e.clientY - r.top) * H) / r.height;
    set(free, "type", 2);
    set(free, "freq", round3(fOf(x)));
    set(free, "gain", Math.round(dbOf(y) * 10) / 10);
    set(free, "on", 1);
    setSelected(free);
  };

  return (
    <div className="eq-editor">
      <div className="eq-main">
        <svg
          ref={svg}
          className={device.on ? "eq-plot" : "eq-plot off"}
          viewBox={`0 0 ${W} ${H}`}
          role="img"
          aria-label={`${device.name} curve: drag a point to move a band`}
          onPointerMove={move}
          onPointerUp={() => (dragging.current = null)}
          onDoubleClick={addBand}
        >
          {GRID.map((f) => (
            <line key={f} x1={xOf(f)} x2={xOf(f)} y1={0} y2={H} className="eq-grid" />
          ))}
          {device.on && <LiveSpectrum id={device.id} />}
          {[-12, 12].map((db) => (
            <line key={db} x1={0} x2={W} y1={yOf(db)} y2={yOf(db)} className="eq-grid" />
          ))}
          <line x1={0} x2={W} y1={H / 2} y2={H / 2} className="eq-zero" />
          {[100, 1000, 10_000].map((f) => (
            <text key={f} x={xOf(f) + 3} y={H - 4} className="eq-label">
              {f >= 1000 ? `${f / 1000}k` : f}
            </text>
          ))}
          <path d={`${path(curve)} L${W},${H / 2} L0,${H / 2} Z`} className="eq-fill" />
          {band.on && <path d={path(bandResponse(band, FREQS))} className="eq-band" />}
          <path d={path(curve)} className="eq-line" />
          {bands.map((b, i) =>
            b.on ? (
              <g
                key={i}
                className={i === selected ? "eq-node sel" : "eq-node"}
                transform={`translate(${xOf(b.freq).toFixed(1)},${yOf(hasGain(b.type) ? b.gain : 0).toFixed(1)})`}
                onPointerDown={(e) => {
                  if (e.button !== 0 || !device.on) return;
                  e.stopPropagation();
                  setSelected(i);
                  dragging.current = i;
                  svg.current?.setPointerCapture(e.pointerId);
                }}
                onDoubleClick={(e) => {
                  e.stopPropagation();
                  set(i, "on", 0);
                }}
              >
                <circle r={8} />
                <text y={3.5}>{i + 1}</text>
              </g>
            ) : null,
          )}
        </svg>
        <div className="eq-bands" role="group" aria-label="Bands">
          {Array.from({ length: EQ_BANDS }, (_, i) => (
            <button
              key={i}
              type="button"
              className={`eq-chip${bands[i].on ? " on" : ""}`}
              aria-pressed={i === selected}
              title={`Band ${i + 1}${bands[i].on ? "" : " (off)"}`}
              onClick={() => setSelected(i)}
            >
              <span className="eq-glyph" aria-hidden="true">
                {TYPE_GLYPH[bands[i].type] ?? ""}
              </span>
              {i + 1}
            </button>
          ))}
        </div>
      </div>
      <div className="dev-grid eq-knobs">
        {["on", "type", "freq", "gain", "q"].map((key) => {
          const p = device.params.find((x) => x.id === `b${selected + 1}${key}`);
          if (!p || (key === "gain" && !hasGain(band.type))) return null;
          return <ParamControl key={p.id} track={track} device={device} param={p} size={30} />;
        })}
        {device.params
          .filter((p) => p.id === "output")
          .map((p) => (
            <ParamControl key={p.id} track={track} device={device} param={p} size={30} />
          ))}
        <EqPhase track={track} device={device} />
      </div>
    </div>
  );
}

/**
 * Minimum phase (no delay) or linear phase (no frequency shifted in time, at
 * the price of a fixed delay the engine makes up for). Not inside a Drum
 * Rack, where nothing can make up for a delay.
 */
function EqPhase({ track, device }: { track: Owner; device: Device }) {
  const rate = useStore((s) => s.engineStatus?.sampleRate ?? 48_000);
  const p = device.params.find((x) => x.id === "phase");
  if (!p || track.id.includes("/")) return null;
  const linear = Math.round(p.value) === 1;
  // Mirrors `Linear::latency`: one block and half the filter.
  const taps = rate <= 50_000 ? 8192 : rate <= 100_000 ? 16_384 : 32_768;
  const ms = Math.round(((taps / 8 + taps / 2) / rate) * 1000);
  return (
    <div
      className="eq-phase tip-t"
      data-tip={
        linear
          ? `Linear phase: the curve without shifting any frequency in time. ${ms} ms late; the other tracks wait for it, but playing live through it feels late.`
          : "Minimum phase: no delay. Linear keeps every frequency in time (for mastering and parallel tracks) at the price of a delay."
      }
    >
      <ParamControl track={track} device={device} param={p} size={30} />
      {linear && <span className="eq-late mono">{ms} ms</span>}
    </div>
  );
}

/** Gain change in dB of the compressor's static curve (mirrors `Compressor::curve`). */
function compressorCurve(level: number, thresh: number, ratio: number, knee: number): number {
  const over = level - thresh;
  const slope = 1 / Math.max(ratio, 1) - 1;
  if (knee > 0 && 2 * Math.abs(over) <= knee) return (slope * Math.pow(over + knee / 2, 2)) / (2 * knee);
  return over > 0 ? slope * over : 0;
}

/** Input level against output level for a compressor or gate, with its threshold. */
export function DynamicsCurve({ device }: { device: Device }) {
  const S = 72;
  const MIN = -60;
  const v = (id: string, fallback: number) => paramOf(device, id, fallback);
  const thresh = v("thresh", -20);
  const pos = (db: number) => ((clamp(db, MIN, 0) - MIN) / -MIN) * S;
  const points: string[] = [];
  for (let i = 0; i <= 60; i++) {
    const input = MIN + (-MIN * i) / 60;
    const output =
      device.type === "comp"
        ? input + compressorCurve(input, thresh, v("ratio", 4), v("knee", 6)) + v("makeup", 0)
        : input > thresh
          ? input
          : input + v("range", -80);
    points.push(`${pos(input).toFixed(1)},${(S - pos(output)).toFixed(1)}`);
  }
  return (
    <svg className={device.on ? "dyn-curve" : "dyn-curve off"} viewBox={`0 0 ${S} ${S}`} role="img" aria-label={`${device.name} curve`}>
      <line x1={0} y1={S} x2={S} y2={0} className="eq-grid" />
      <line x1={pos(thresh)} x2={pos(thresh)} y1={0} y2={S} className="dyn-thresh" />
      <polyline points={points.join(" ")} className="eq-line" />
    </svg>
  );
}
