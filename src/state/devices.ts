import catalog from "../../shared/device-catalog.json";
import { MASTER, type AutoTarget, type Chain, type Device, type DeviceParam, type DeviceRole, type MacroMap, type Project } from "./types";

type ParamSpec = Omit<DeviceParam, "value">;

export interface PadSpec {
  count: number;
  /** Note of the first pad. */
  first: number;
  /** Built-in sound of each pad. */
  names: string[];
  params: ParamSpec[];
  /** Per-pad default overrides, by note and parameter id. */
  defaults?: Record<string, Record<string, number>>;
}

export interface DeviceSpec {
  type: string;
  name: string;
  role: DeviceRole;
  category: "instrument" | "audio-fx" | "midi-fx" | "modulator";
  description: string;
  /** Roadmap phase in which the device gets real DSP. */
  phase: number;
  params: ParamSpec[];
  pads?: PadSpec;
}

/** Shared with the Rust side (soniq-project reads the same file). */
export const DEVICE_CATALOG = catalog as DeviceSpec[];

/** Devices that work today: instruments, MIDI effects, modulators and the audio effects of phase 4. */
export const LIVE_TYPES = new Set(
  DEVICE_CATALOG.filter((d) => d.role === "instrument" || d.category === "midi-fx" || ((d.category === "audio-fx" || d.category === "modulator") && d.phase <= 4)).map(
    (d) => d.type,
  ),
);

export const isMidiFx = (type: string) => DEVICE_CATALOG.some((d) => d.type === type && d.category === "midi-fx");

/** LFOs and envelope followers: they move another device's parameter (mirrors `MODULATOR_TYPES`). */
export const isModulator = (type: string) => DEVICE_CATALOG.some((d) => d.type === type && d.category === "modulator");

/** Devices that can listen to another track (mirrors `SIDECHAIN_TYPES`). */
export const SIDECHAIN_TYPES = new Set(["comp", "gate", "vocoder"]);

/** A device chain and what holds it: a track, a return track or the master. */
export interface ChainHost {
  id: string;
  name: string;
  kind: "midi" | "audio" | "group" | "return" | "master" | "pad";
  devices: Device[];
  /** Track colour; 0 for returns and the master. */
  color: number;
}

/** The chain with this owner id (a track, a return, or MASTER). */
export function chainHost(project: Project, id: string | null): ChainHost | null {
  if (!id) return null;
  // A Drum Rack pad's effects, or one of its returns: "track/device/key".
  const parts = id.split("/");
  if (parts.length === 3) {
    const [t, d, key] = parts;
    const track = project.tracks.find((x) => x.id === t);
    const rack = track?.devices.find((x) => x.id === d && x.type === "drum-rack");
    if (!track || !rack) return null;
    // A pad playing a sample goes by the sample's name.
    const sample = rack.samples?.find((x) => String(x.key) === key);
    const sampleName = sample && project.assets.find((a) => a.id === sample.asset)?.name.replace(/\.[^.]+$/, "");
    const what = key === "A" || key === "B" ? `Return ${key}` : (sampleName ?? padName(Number(key)));
    return { id, name: `${track.name} · ${what}`, kind: "pad", devices: rack.padChains?.find((c) => c.key === key)?.devices ?? [], color: track.color };
  }
  if (id === MASTER) return { id, name: "Master", kind: "master", devices: project.masterDevices ?? [], color: 0 };
  const track = project.tracks.find((t) => t.id === id);
  if (track) return { id, name: track.name, kind: track.kind, devices: track.devices, color: track.color };
  const ret = project.returns.find((r) => r.id === id);
  return ret ? { id, name: ret.name, kind: "return", devices: ret.devices, color: 0 } : null;
}

/** Every parameter, with drum pads expanded to `p36.tune`, … (mirrors `DeviceSpec::all_params`). */
export function allParams(spec: DeviceSpec): ParamSpec[] {
  const params = [...spec.params];
  const pads = spec.pads;
  if (pads) {
    for (let note = pads.first; note < pads.first + pads.count; note++) {
      const overrides = pads.defaults?.[String(note)] ?? {};
      for (const p of pads.params) params.push({ ...p, id: `p${note}.${p.id}`, group: `pad:${note}`, default: overrides[p.id] ?? p.default });
    }
  }
  return params;
}

let nextDeviceId = 1;

/** A fresh device; the backend assigns the final id when it is added. */
export function createDevice(type: string, overrides: Record<string, number> = {}): Device {
  const spec = DEVICE_CATALOG.find((d) => d.type === type);
  if (!spec) throw new Error(`Unknown device type: ${type}`);
  return {
    id: `dev-new-${nextDeviceId++}`,
    type: spec.type,
    name: spec.name,
    role: spec.role,
    on: true,
    params: allParams(spec).map((param) => ({ ...param, value: overrides[param.id] ?? param.default })),
  };
}

/** The values an automation lane can take, and whether they sweep by octave (mirrors `target_range`). */
export function autoRange(track: { devices: Device[] }, target: AutoTarget): { min: number; max: number; log: boolean; param?: DeviceParam } {
  switch (target.kind) {
    case "volume":
      return { min: -60, max: 6, log: false };
    case "pan":
      return { min: -1, max: 1, log: false };
    case "send":
      return { min: 0, max: 1, log: false };
    case "param": {
      const param = track.devices.find((d) => d.id === target.device)?.params.find((p) => p.id === target.param);
      return param ? { min: param.min, max: param.max, log: !!param.log && param.min > 0, param } : { min: 0, max: 1, log: false };
    }
  }
}

/** What an automation lane or modulator target is called ("Volume", "Send A", "Reverb · Size"). */
export function targetLabel(track: { devices: Device[] }, target: { kind: string; index?: number; device?: string; param?: string }): string {
  switch (target.kind) {
    case "volume":
      return "Volume";
    case "pan":
      return "Pan";
    case "send":
      return `Send ${String.fromCharCode(65 + (target.index ?? 0))}`;
    default: {
      const device = track.devices.find((d) => d.id === target.device);
      const param = device?.params.find((p) => p.id === target.param);
      return device && param ? `${device.name} · ${param.label}` : "Missing parameter";
    }
  }
}

export function deviceSpec(type: string): DeviceSpec | undefined {
  return DEVICE_CATALOG.find((d) => d.type === type);
}

/** How many macros an instrument rack has (`macro1` … `macro8`). */
export const MACROS = 8;

/** A mapped parameter's value with its macro at `amount` (0–1): log parameters sweep by octave (mirrors `MacroMap::value`). */
export function macroValue(map: MacroMap, p: DeviceParam, amount: number): number {
  const t = Math.min(1, Math.max(0, amount));
  let v = p.log && map.min > 0 && map.max > 0 ? map.min * Math.pow(map.max / map.min, t) : map.min + (map.max - map.min) * t;
  if (p.step) v = p.min + Math.round((v - p.min) / p.step) * p.step;
  return Math.min(p.max, Math.max(p.min, v));
}

/** Where a macro (0–1) has to be for the parameter to sit at `value` (mirrors `MacroMap::amount`). */
export function macroAmount(map: MacroMap, p: DeviceParam, value: number): number {
  let t = 0;
  if (p.log && map.min > 0 && map.max > 0 && value > 0 && map.max !== map.min) t = Math.log(value / map.min) / Math.log(map.max / map.min);
  else if (map.max !== map.min) t = (value - map.min) / (map.max - map.min);
  return Math.min(1, Math.max(0, t));
}

/** A chain instrument's parameters as the rack plays them: mapped ones follow their macro. */
export function chainParams(rack: Device, chain: Chain): DeviceParam[] {
  return chain.device.params.map((p) => {
    const map = rack.macros?.find((m) => m.chain === chain.id && m.param === p.id);
    if (!map) return p;
    const amount = (rack.params.find((q) => q.id === `macro${map.index + 1}`)?.value ?? 0) / 100;
    return { ...p, value: macroValue(map, p, amount) };
  });
}

/** The built-in sound of a drum pad. */
export function padName(note: number): string {
  const pads = deviceSpec("drum-rack")?.pads;
  return pads?.names[note - pads.first] ?? `Pad ${note}`;
}
