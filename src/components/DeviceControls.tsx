import { MACROS } from "../state/devices";
import { formatParam } from "../state/format";
import { useStore } from "../state/store";
import type { Device, DeviceParam } from "../state/types";
import { Knob } from "./controls/Knob";
import { Icon } from "./Icon";
import { Learnable } from "./Learnable";
import { Select } from "./Select";

/** What holds a device chain, as its panels need it: an id for device ops and a colour. */
export interface Owner {
  id: string;
  color: number;
}

/** Inside an instrument rack: which macro turns a parameter, how many each macro turns, and (re)mapping one. */
export interface Mapping {
  macroOf: (param: string) => number | undefined;
  count: (index: number) => number;
  map: (param: DeviceParam, index: number | undefined) => void;
}

export const paramOf = (device: Device, id: string, fallback: number) => device.params.find((p) => p.id === id)?.value ?? fallback;

/**
 * A knob, or a small menu for parameters with named choices. Right-click
 * maps a MIDI controller to it, and in a rack chain a macro too.
 */
export function ParamControl({
  track,
  device,
  param: p,
  size,
  mapping,
  text,
}: {
  track: Owner;
  device: Device;
  param: DeviceParam;
  size?: number;
  mapping?: Mapping;
  /** How the device itself writes the current value (a plugin's "-6.0 dB"). */
  text?: string;
}) {
  const setParam = useStore((s) => s.setParam);
  const macro = mapping?.macroOf(p.id);
  // A mapped parameter follows its macro.
  const locked = !device.on || macro !== undefined;
  const control = p.options ? (
    <label className="dev-opt">
      <span className="knob-l">{p.label}</span>
      <span className="selwrap">
        <Select className="select sm" value={Math.round(p.value)} disabled={locked} onChange={(e) => setParam(track.id, device.id, p.id, Number(e.target.value))}>
          {p.options.map((o, i) => (
            <option key={o} value={i}>
              {o}
            </option>
          ))}
        </Select>
      </span>
    </label>
  ) : (
    <Knob
      label={p.label}
      value={p.value}
      min={p.min}
      max={p.max}
      defaultValue={p.default}
      log={p.log}
      bipolar={p.bipolar}
      size={size}
      disabled={locked}
      format={(value) => (text !== undefined && value === p.value ? text : formatParam({ ...p, value }))}
      onChange={(value) => setParam(track.id, device.id, p.id, p.step ? Math.round(value / p.step) * p.step : value)}
    />
  );
  const macros = mapping
    ? (close: () => void) => (
        <>
          {Array.from({ length: MACROS }, (_, i) => (
            <button
              key={i}
              type="button"
              role="menuitemradio"
              aria-checked={macro === i}
              className="mi"
              onClick={() => {
                close();
                mapping.map(p, i);
              }}
            >
              <span className="mi-check">{macro === i && <Icon name="check" size={15} />}</span>
              <span className="mi-label">Macro {i + 1}</span>
              <span className="mi-k">{mapping.count(i) ? `${mapping.count(i)} mapped` : ""}</span>
            </button>
          ))}
          {macro !== undefined && (
            <button
              type="button"
              role="menuitem"
              className="mi"
              onClick={() => {
                close();
                mapping.map(p, undefined);
              }}
            >
              <span className="mi-check" />
              <span className="mi-label">Unmap from macro {macro + 1}</span>
            </button>
          )}
          <div className="msep" role="separator" />
        </>
      )
    : undefined;
  return (
    <Learnable
      target={{ kind: "param", track: track.id, device: device.id, param: p.id }}
      label={`${device.name} ${p.label}`}
      tag={macro === undefined ? undefined : `M${macro + 1}`}
      extra={macros}
    >
      {control}
    </Learnable>
  );
}

/** Parameters in their sections (an instrument's oscillators, filter, envelopes…), or one row. */
export function ParamGroups({
  track,
  device,
  params,
  mapping,
  texts,
}: {
  track: Owner;
  device: Device;
  params: DeviceParam[];
  mapping?: Mapping;
  texts?: Record<string, string>;
}) {
  const groups = new Map<string, DeviceParam[]>();
  for (const p of params) groups.set(p.group ?? "", [...(groups.get(p.group ?? "") ?? []), p]);
  if (groups.size <= 1) {
    return (
      <div className="dev-knobs">
        {params.map((p) => (
          <ParamControl key={p.id} track={track} device={device} param={p} mapping={mapping} text={texts?.[p.id]} />
        ))}
      </div>
    );
  }
  return (
    <div className="dev-groups">
      {[...groups].map(([name, list]) => (
        <div key={name} className="dev-group" role="group" aria-label={name}>
          <span className="dev-group-l">{name}</span>
          <div className="dev-grid">
            {list.map((p) => (
              <ParamControl key={p.id} track={track} device={device} param={p} size={32} mapping={mapping} text={texts?.[p.id]} />
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

/**
 * The parameters a device's panel shows as knobs: pad parameters only for
 * the selected pad, slice settings only in slice mode, and an EQ's bands
 * in its curve editor instead.
 */
export function panelParams(device: Device): DeviceParam[] {
  const slicing = Math.round(paramOf(device, "mode", 0)) === 3;
  return device.params.filter((p) => !p.group?.startsWith("pad:") && !p.group?.startsWith("band:") && (p.group !== "Slices" || slicing));
}
