import { useEffect, useRef, type KeyboardEvent, type PointerEvent } from "react";

interface KnobProps {
  label: string;
  value: number;
  min: number;
  max: number;
  defaultValue: number;
  format: (value: number) => string;
  onChange: (value: number) => void;
  log?: boolean;
  bipolar?: boolean;
  size?: number;
  disabled?: boolean;
}

const SWEEP = 135;

function polar(c: number, r: number, deg: number) {
  const a = (deg * Math.PI) / 180;
  return [c + r * Math.sin(a), c - r * Math.cos(a)] as const;
}

function arc(c: number, r: number, from: number, to: number) {
  const [x1, y1] = polar(c, r, from);
  const [x2, y2] = polar(c, r, to);
  const large = Math.abs(to - from) > 180 ? 1 : 0;
  const sweep = to > from ? 1 : 0;
  return `M${x1.toFixed(2)} ${y1.toFixed(2)}A${r} ${r} 0 ${large} ${sweep} ${x2.toFixed(2)} ${y2.toFixed(2)}`;
}

const clamp01 = (v: number) => Math.min(1, Math.max(0, v));

/**
 * Rotary control. Drag vertically (Shift for fine), scroll, use the arrow keys,
 * or double-click to reset.
 */
export function Knob({ label, value, min, max, defaultValue, format, onChange, log, bipolar, size = 40, disabled }: KnobProps) {
  const ref = useRef<HTMLDivElement>(null);
  const drag = useRef<{ y: number; frac: number } | null>(null);

  const toFrac = (v: number) => clamp01(log ? Math.log(v / min) / Math.log(max / min) : (v - min) / (max - min));
  const fromFrac = (f: number) => (log ? min * Math.pow(max / min, clamp01(f)) : min + clamp01(f) * (max - min));
  const frac = toFrac(value);

  // A latest-value ref keeps the native wheel listener free of stale closures.
  const latest = useRef({ frac, fromFrac, onChange, disabled });
  latest.current = { frac, fromFrac, onChange, disabled };

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    // Non-passive so the wheel turns the knob instead of scrolling the panel.
    const onWheel = (e: WheelEvent) => {
      const { frac, fromFrac, onChange, disabled } = latest.current;
      if (disabled) return;
      e.preventDefault();
      onChange(fromFrac(frac + (e.deltaY < 0 ? 1 : -1) * (e.shiftKey ? 0.005 : 0.025)));
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (disabled || e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { y: e.clientY, frac };
  };
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return;
    const dy = drag.current.y - e.clientY;
    onChange(fromFrac(drag.current.frac + dy / (e.shiftKey ? 800 : 160)));
  };
  const onPointerUp = () => {
    drag.current = null;
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.shiftKey ? 0.002 : 0.01;
    const moves: Record<string, number> = { ArrowUp: step, ArrowRight: step, ArrowDown: -step, ArrowLeft: -step, PageUp: 0.1, PageDown: -0.1 };
    if (e.key in moves) onChange(fromFrac(frac + moves[e.key]));
    else if (e.key === "Home") onChange(min);
    else if (e.key === "End") onChange(max);
    else return;
    e.preventDefault();
    e.stopPropagation();
  };

  const c = size / 2;
  const r = c - 3;
  const angle = -SWEEP + frac * SWEEP * 2;
  const origin = bipolar ? 0 : -SWEEP;
  const [px1, py1] = polar(c, r * 0.32, angle);
  const [px2, py2] = polar(c, r * 0.72, angle);

  return (
    <div className={disabled ? "knob is-off" : "knob"}>
      <div
        ref={ref}
        className="knob-dial"
        role="slider"
        tabIndex={disabled ? -1 : 0}
        aria-label={label}
        aria-valuemin={min}
        aria-valuemax={max}
        aria-valuenow={Number(value.toFixed(3))}
        aria-valuetext={format(value)}
        aria-disabled={disabled || undefined}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onDoubleClick={() => !disabled && onChange(defaultValue)}
        onKeyDown={onKeyDown}
        style={{ width: size, height: size }}
      >
        <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} aria-hidden="true">
          <path className="knob-track" d={arc(c, r, -SWEEP, SWEEP)} />
          {Math.abs(angle - origin) > 0.5 && <path className="knob-value" d={arc(c, r, Math.min(origin, angle), Math.max(origin, angle))} />}
          <circle className="knob-body" cx={c} cy={c} r={r - 5} />
          <line className="knob-pointer" x1={px1} y1={py1} x2={px2} y2={py2} />
        </svg>
      </div>
      <span className="knob-l">{label}</span>
      <output className="knob-v">{format(value)}</output>
    </div>
  );
}
