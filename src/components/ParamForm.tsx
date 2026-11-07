import type { CSSProperties } from "react";
import { getParam, setParam, type ParamSpec } from "../state/effects";
import type { Effect, Generator } from "../state/types";
import { Select } from "./Select";

const STEPS = 1000;

/** Slider position (0…STEPS) for a value, logarithmic when the spec says so. */
function toSlider(spec: ParamSpec, value: number) {
  const min = spec.min ?? 0;
  const max = spec.max ?? 1;
  const t = spec.log && min > 0 ? Math.log(value / min) / Math.log(max / min) : (value - min) / (max - min);
  return Math.round(Math.min(1, Math.max(0, t)) * STEPS);
}

function fromSlider(spec: ParamSpec, pos: number) {
  const min = spec.min ?? 0;
  const max = spec.max ?? 1;
  const t = pos / STEPS;
  const raw = spec.log && min > 0 ? min * Math.pow(max / min, t) : min + (max - min) * t;
  const step = spec.step ?? 0.01;
  return Number((Math.round(raw / step) * step).toFixed(6));
}

function NumberParam({ spec, value, onChange }: { spec: ParamSpec; value: number; onChange: (v: number) => void }) {
  const pos = toSlider(spec, value);
  return (
    <div className="param-num">
      <input
        className="range"
        type="range"
        min={0}
        max={STEPS}
        value={pos}
        aria-label={spec.label}
        style={{ "--pct": `${(pos / STEPS) * 100}%` } as CSSProperties}
        onChange={(e) => onChange(fromSlider(spec, Number(e.target.value)))}
      />
      <label className="field mono">
        <input
          type="number"
          aria-label={`${spec.label} value`}
          min={spec.min}
          max={spec.max}
          step={spec.step}
          value={value}
          onChange={(e) => {
            const v = Number(e.target.value);
            if (Number.isFinite(v)) onChange(Math.min(spec.max ?? v, Math.max(spec.min ?? v, v)));
          }}
        />
        {spec.unit && <span className="fix">{spec.unit}</span>}
      </label>
    </div>
  );
}

/** Controls for every parameter of an effect, generator or note tool (any flat settings object). */
export function ParamForm<T extends object>({ specs, target, onChange, compact }: { specs: ParamSpec[]; target: T; onChange: (next: T) => void; compact?: boolean }) {
  if (!specs.length) return <p className="muted">There is nothing to set.</p>;
  // getParam and setParam only read and copy plain fields.
  const record = target as unknown as Effect | Generator;
  return (
    <div className={compact ? "params compact" : "params"}>
      {specs.map((spec) => {
        const value = getParam(record, spec.id);
        const update = (v: number | string | boolean) => onChange(setParam(record, spec.id, v) as unknown as T);
        return (
          <div key={spec.id} className="param" title={spec.hint}>
            <span className="lab">{spec.label}</span>
            {spec.kind === "number" && <NumberParam spec={spec} value={Number(value)} onChange={update} />}
            {spec.kind === "select" && (
              <span className="selwrap">
                <Select
                  className="select"
                  aria-label={spec.label}
                  value={String(value)}
                  onChange={(e) => {
                    const hit = spec.options?.find(([v]) => String(v) === e.target.value);
                    if (hit) update(hit[0]);
                  }}
                >
                  {spec.options?.map(([v, label]) => (
                    <option key={String(v)} value={String(v)}>
                      {label}
                    </option>
                  ))}
                </Select>
              </span>
            )}
            {spec.kind === "toggle" && (
              <button className="switch" type="button" role="switch" aria-checked={!!value} aria-label={spec.label} onClick={() => update(!value)} />
            )}
            {spec.kind === "text" && (
              <label className="field mono">
                <input aria-label={spec.label} value={String(value)} onChange={(e) => update(e.target.value)} />
              </label>
            )}
          </div>
        );
      })}
    </div>
  );
}
