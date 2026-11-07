import { useRef, useState, type PointerEvent } from "react";

/** (Hz, dB) points of a drawn EQ curve. */
export type CurvePoints = [number, number][];

const W = 520;
const H = 200;
const LO = 20;
const HI = 20_000;
const RANGE = 24;
const GRID_HZ = [50, 100, 200, 500, 1000, 2000, 5000, 10_000];
const GRID_DB = [-18, -12, -6, 0, 6, 12, 18];

export const CURVE_PRESETS: [string, CurvePoints][] = [
  ["Flat", [[1000, 0]]],
  ["Low cut (rumble)", [[40, -24], [90, 0]]],
  ["Telephone", [[250, -24], [500, 0], [3000, 0], [4500, -24]]],
  ["Less mud", [[120, 0], [300, -4], [700, 0]]],
  ["Presence", [[1500, 0], [4000, 4], [9000, 0]]],
  ["Air", [[6000, 0], [14_000, 5]]],
];

const xOf = (hz: number) => (Math.log10(hz / LO) / Math.log10(HI / LO)) * W;
const hzAt = (x: number) => LO * 10 ** ((x / W) * Math.log10(HI / LO));
const yOf = (db: number) => H / 2 - (db / RANGE) * (H / 2);
const dbAt = (y: number) => ((H / 2 - y) / (H / 2)) * RANGE;
const label = (hz: number) => (hz >= 1000 ? `${hz / 1000}k` : String(hz));

/** Draw an EQ curve: click to add a point, drag to move it, double-click to remove it. */
export function CurveEditor({ points, onChange }: { points: CurvePoints; onChange: (next: CurvePoints) => void }) {
  const svg = useRef<SVGSVGElement>(null);
  const [drag, setDrag] = useState<number | null>(null);
  const sorted = [...points].sort((a, b) => a[0] - b[0]);

  const at = (e: PointerEvent): [number, number] => {
    const r = svg.current!.getBoundingClientRect();
    const x = Math.min(W, Math.max(0, ((e.clientX - r.left) / r.width) * W));
    const y = Math.min(H, Math.max(0, ((e.clientY - r.top) / r.height) * H));
    return [Math.round(hzAt(x)), Math.round(dbAt(y) * 2) / 2];
  };

  const line = sorted.length
    ? [`M0,${yOf(sorted[0][1])}`, ...sorted.map(([hz, db]) => `L${xOf(hz).toFixed(1)},${yOf(db).toFixed(1)}`), `L${W},${yOf(sorted[sorted.length - 1][1])}`].join(" ")
    : `M0,${H / 2} L${W},${H / 2}`;

  return (
    <div className="curve-editor">
      <svg
        ref={svg}
        viewBox={`0 0 ${W} ${H}`}
        role="img"
        aria-label={`EQ curve with ${sorted.length} points`}
        onPointerDown={(e) => {
          if (e.target !== e.currentTarget && (e.target as Element).tagName !== "rect") return;
          onChange([...points, at(e)]);
        }}
        onPointerMove={(e) => {
          if (drag === null) return;
          const next = points.slice();
          next[drag] = at(e);
          onChange(next);
        }}
        onPointerUp={() => setDrag(null)}
      >
        <rect x={0} y={0} width={W} height={H} className="curve-bg" />
        {GRID_HZ.map((hz) => (
          <g key={hz}>
            <line x1={xOf(hz)} x2={xOf(hz)} y1={0} y2={H} className="curve-grid" />
            <text x={xOf(hz) + 3} y={H - 4} className="curve-label">
              {label(hz)}
            </text>
          </g>
        ))}
        {GRID_DB.map((db) => (
          <g key={db}>
            <line x1={0} x2={W} y1={yOf(db)} y2={yOf(db)} className={db === 0 ? "curve-grid zero" : "curve-grid"} />
            <text x={3} y={yOf(db) - 3} className="curve-label">
              {db > 0 ? `+${db}` : db}
            </text>
          </g>
        ))}
        <path d={line} className="curve-line" />
        {points.map(([hz, db], i) => (
          <circle
            key={i}
            cx={xOf(hz)}
            cy={yOf(db)}
            r={6}
            className="curve-point"
            onPointerDown={(e) => {
              e.stopPropagation();
              (e.currentTarget.ownerSVGElement as SVGSVGElement).setPointerCapture(e.pointerId);
              setDrag(i);
            }}
            onDoubleClick={() => onChange(points.filter((_, j) => j !== i))}
          >
            <title>{`${label(Math.round(hz))} Hz, ${db > 0 ? "+" : ""}${db} dB · drag to move, double-click to remove`}</title>
          </circle>
        ))}
      </svg>
      <div className="curve-presets" role="group" aria-label="Curve presets">
        {CURVE_PRESETS.map(([name, preset]) => (
          <button key={name} type="button" className="btn sm btn-ghost" onClick={() => onChange(preset.map((p) => [...p] as [number, number]))}>
            {name}
          </button>
        ))}
      </div>
      <p className="muted curve-hint">Click to add a point, drag to move it, double-click to take it away. Between points the curve is straight on this log scale.</p>
    </div>
  );
}
