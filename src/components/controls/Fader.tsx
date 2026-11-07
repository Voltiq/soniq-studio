import { useRef, type KeyboardEvent, type PointerEvent } from "react";
import { formatDb, MIN_DB } from "../../state/format";

const MAX_DB = 6;
const CAP = 16;
const TICKS = [6, 0, -6, -12, -24, -48];

/** Fader taper: more travel near unity gain, where mixing happens. */
export function dbToFrac(db: number): number {
  if (db <= MIN_DB) return 0;
  return Math.pow(Math.min(1, (db - MIN_DB) / (MAX_DB - MIN_DB)), 1.5);
}

function fracToDb(frac: number): number {
  const f = Math.min(1, Math.max(0, frac));
  if (f <= 0) return MIN_DB;
  return Math.round((MIN_DB + (MAX_DB - MIN_DB) * Math.pow(f, 1 / 1.5)) * 10) / 10;
}

interface FaderProps {
  label: string;
  value: number;
  onChange: (db: number) => void;
  height?: number;
}

/** Vertical volume fader in dB. Drag (Shift for fine), arrow keys, double-click for 0 dB. */
export function Fader({ label, value, onChange, height = 180 }: FaderProps) {
  const drag = useRef<{ y: number; frac: number } | null>(null);
  const travel = height - CAP;
  const frac = dbToFrac(value);

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { y: e.clientY, frac };
  };
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return;
    const dy = drag.current.y - e.clientY;
    onChange(fracToDb(drag.current.frac + (dy / travel) * (e.shiftKey ? 0.2 : 1)));
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.shiftKey ? 0.1 : 0.5;
    let next: number | null = null;
    if (e.key === "ArrowUp" || e.key === "ArrowRight") next = value + step;
    else if (e.key === "ArrowDown" || e.key === "ArrowLeft") next = value - step;
    else if (e.key === "PageUp") next = value + 3;
    else if (e.key === "PageDown") next = value - 3;
    else if (e.key === "Home") next = MIN_DB;
    else if (e.key === "End") next = MAX_DB;
    if (next === null) return;
    e.preventDefault();
    e.stopPropagation();
    onChange(Math.min(MAX_DB, Math.max(MIN_DB, Math.round(next * 10) / 10)));
  };

  return (
    <div className="fader-wrap" style={{ height }}>
      <div className="fader-scale" aria-hidden="true">
        {TICKS.map((db) => (
          <span key={db} style={{ bottom: dbToFrac(db) * travel + CAP / 2 }}>
            {db > 0 ? `+${db}` : db}
          </span>
        ))}
      </div>
      <div
        className="fader"
        role="slider"
        tabIndex={0}
        aria-label={label}
        aria-orientation="vertical"
        aria-valuemin={MIN_DB}
        aria-valuemax={MAX_DB}
        aria-valuenow={value}
        aria-valuetext={`${formatDb(value)} dB`}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={() => (drag.current = null)}
        onPointerCancel={() => (drag.current = null)}
        onDoubleClick={() => onChange(0)}
        onKeyDown={onKeyDown}
      >
        <span className="fader-track" />
        <span className="fader-fill" style={{ height: frac * travel + CAP / 2 }} />
        <span className="fader-unity" style={{ bottom: dbToFrac(0) * travel + CAP / 2 }} />
        <span className="fader-cap" style={{ bottom: frac * travel }} />
      </div>
    </div>
  );
}
