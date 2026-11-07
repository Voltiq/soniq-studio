/*
 * A browser-only stand-in for the Rust backend, so `npm run dev` stays useful
 * for UI work. It mirrors the project ops and undo behaviour of soniq-project
 * on the demo song; file access and real audio need the desktop app.
 */
import { createDemoProject, emptyTrack, newId } from "../state/demoProject";
import { DEVICE_CATALOG, autoRange, createDevice, isMidiFx, macroAmount } from "../state/devices";
import { editLabel, editPeaks } from "../state/edits";
import { tempoOf } from "../state/tempo";
import {
  MASTER,
  clipLength,
  sameAutoTarget,
  sameTarget,
  sortNotes,
  type AudioClip,
  type Chain,
  type Device,
  type Macro,
  type MacroMap,
  type Mix,
  type Op,
  type PeakData,
  type Project,
  type ProjectReply,
  type Track,
} from "../state/types";
import type { Backend } from "./backend";

interface Entry {
  label: string;
  project: Project;
  key: string | null;
  at: number;
}

const COALESCE_MS = 1500;
const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

function coalesceKey(op: Op): string | null {
  switch (op.op) {
    case "setTrack":
      return op.patch.volume !== undefined || op.patch.pan !== undefined || op.patch.sends !== undefined ? `track:${op.id}:${Object.keys(op.patch)[0]}` : null;
    case "setReturn":
      return `return:${op.id}`;
    case "setMaster":
      return "master";
    case "setParam":
      return `param:${op.device}:${op.param}`;
    case "setTempo":
      return "tempo";
    case "setChain":
      return op.patch.name === undefined && op.patch.on === undefined ? `chain:${op.chain}` : null;
    default:
      return null;
  }
}

function captureMix(p: Project, id: string, name: string): Mix {
  return {
    id,
    name,
    tracks: p.tracks.map((t) => ({ track: t.id, volume: t.volume, pan: t.pan, mute: t.mute, solo: t.solo, sends: t.sends })),
    returns: p.returns.map((r) => ({ return: r.id, volume: r.volume, pan: r.pan, mute: r.mute })),
    masterVolume: p.masterVolume,
  };
}

/** Tracks added since keep their own settings. */
function applyMix(p: Project, mix: Mix): Project {
  return {
    ...p,
    tracks: p.tracks.map((t) => {
      const m = mix.tracks.find((x) => x.track === t.id);
      return m ? { ...t, volume: m.volume, pan: m.pan, mute: m.mute, solo: m.solo, sends: m.sends } : t;
    }),
    returns: p.returns.map((r) => {
      const m = mix.returns.find((x) => x.return === r.id);
      return m ? { ...r, volume: m.volume, pan: m.pan, mute: m.mute } : r;
    }),
    masterVolume: mix.masterVolume,
  };
}

function mapTrack(p: Project, id: string, fn: (t: Track) => Track): Project {
  if (!p.tracks.some((t) => t.id === id)) throw new Error(`No track with id ${id}`);
  return { ...p, tracks: p.tracks.map((t) => (t.id === id ? fn(t) : t)) };
}

/** Changes the device chain of a track, a return or the master. */
function mapChain(p: Project, owner: string, fn: (devices: Device[]) => Device[]): Project {
  if (owner === MASTER) return { ...p, masterDevices: fn(p.masterDevices ?? []) };
  if (p.returns.some((r) => r.id === owner)) return { ...p, returns: p.returns.map((r) => (r.id === owner ? { ...r, devices: fn(r.devices) } : r)) };
  return mapTrack(p, owner, (t) => ({ ...t, devices: fn(t.devices) }));
}

/** Changes a device in a chain, or the instrument of one of its rack chains. */
function mapDevice(p: Project, owner: string, deviceId: string, fn: (d: Device) => Device): Project {
  return mapChain(p, owner, (devices) =>
    devices.map((d) =>
      d.id === deviceId ? fn(d) : d.chains?.some((c) => c.device.id === deviceId) ? { ...d, chains: d.chains.map((c) => (c.device.id === deviceId ? { ...c, device: fn(c.device) } : c)) } : d,
    ),
  );
}

/** Changes an instrument rack on the track. */
function mapRack(p: Project, trackId: string, rackId: string, fn: (d: Device) => Device): Project {
  return mapDevice(p, trackId, rackId, (d) => {
    if (d.type !== "rack") throw new Error(`${d.name} has no chains`);
    return fn(d);
  });
}

function pruneAssets(p: Project): Project {
  const clips = p.tracks.flatMap((t) => t.clips.flatMap((c) => (c.kind === "audio" ? [c.asset] : [])));
  const samples = p.tracks.flatMap((t) =>
    t.devices.flatMap((d) => [d, ...(d.chains ?? []).map((c) => c.device)]).flatMap((d) => (d.samples ?? []).map((s) => s.asset)),
  );
  const used = new Set([...clips, ...samples]);
  return { ...p, assets: p.assets.filter((a) => used.has(a.id)) };
}

function reduce(p: Project, op: Op): { project: Project; label: string; created?: string } {
  switch (op.op) {
    case "setTrack": {
      const { patch } = op;
      const next = mapTrack(p, op.id, (t) => ({
        ...t,
        ...patch,
        volume: patch.volume !== undefined ? clamp(patch.volume, -60, 6) : t.volume,
        pan: patch.pan !== undefined ? clamp(patch.pan, -1, 1) : t.pan,
      }));
      const name = p.tracks.find((t) => t.id === op.id)?.name ?? "Track";
      return { project: next, label: `${name} ${Object.keys(patch)[0]}` };
    }
    case "setReturn":
      return { project: { ...p, returns: p.returns.map((r) => (r.id === op.id ? { ...r, ...op.patch } : r)) }, label: "Return change" };
    case "setMaster":
      return { project: { ...p, masterVolume: clamp(op.volume, -60, 6) }, label: "Master volume" };
    case "addTrack": {
      const count = p.tracks.filter((t) => t.kind === op.kind).length + 1;
      const t = emptyTrack(op.kind, op.kind === "audio" ? `Audio ${count}` : `MIDI ${count}`, (p.tracks.length % 8) + 1);
      return { project: { ...p, tracks: [...p.tracks, t] }, label: `Add ${op.kind === "audio" ? "audio" : "MIDI"} track`, created: t.id };
    }
    case "removeTrack": {
      const t = p.tracks.find((x) => x.id === op.id);
      if (!t) throw new Error(`No track with id ${op.id}`);
      // A group's tracks stay, outside it.
      const rest = { ...p, tracks: p.tracks.filter((x) => x.id !== op.id).map((x) => (x.group === op.id ? { ...x, group: undefined } : x)) };
      // Sidechains that listened to the track hear their own input again.
      const unhook = (devices: Device[]) => devices.map((d) => (d.sidechain === op.id ? { ...d, sidechain: undefined } : d));
      const owners = [...rest.tracks.map((x) => x.id), ...rest.returns.map((r) => r.id), MASTER];
      const project = owners.reduce((acc, owner) => mapChain(acc, owner, unhook), rest);
      return { project: pruneAssets(project), label: `Delete ${t.name}` };
    }
    case "addDevice": {
      const device = { ...op.device, id: newId("dev"), sidechain: undefined };
      const target = p.tracks.find((t) => t.id === op.track);
      // MIDI effects, then the instrument, then audio effects (mirrors soniq-project).
      const midiFx = isMidiFx(device.type);
      if ((device.role === "instrument" || midiFx) && target?.kind !== "midi") throw new Error(`${device.name} needs a MIDI track`);
      const next = mapChain(p, op.track, (devices) => {
        const lead = devices.findIndex((d) => !isMidiFx(d.type));
        const at = lead < 0 ? devices.length : lead;
        const rest = device.role === "instrument" ? devices.filter((d) => d.role !== "instrument") : devices;
        return device.role === "instrument" || midiFx ? [...rest.slice(0, at), device, ...rest.slice(at)] : [...devices, device];
      });
      return { project: next, label: `Add ${device.name}` };
    }
    case "setPluginParams":
      return { project: mapDevice(p, op.track, op.device, (d) => ({ ...d, params: op.params })), label: "Plugin controls" };
    case "saveMix": {
      const mix = captureMix(p, newId("x"), op.name?.trim() || `Mix ${(p.mixes ?? []).length + 1}`);
      return { project: { ...p, mixes: [...(p.mixes ?? []), mix] }, label: `Save mix “${mix.name}”`, created: mix.id };
    }
    case "recallMix": {
      const mix = (p.mixes ?? []).find((m) => m.id === op.id);
      if (!mix) throw new Error("That mix no longer exists.");
      return { project: applyMix(p, mix), label: `Recall mix “${mix.name}”` };
    }
    case "updateMix": {
      const mix = (p.mixes ?? []).find((m) => m.id === op.id);
      if (!mix) throw new Error("That mix no longer exists.");
      return { project: { ...p, mixes: (p.mixes ?? []).map((m) => (m.id === op.id ? captureMix(p, m.id, m.name) : m)) }, label: `Update mix “${mix.name}”` };
    }
    case "renameMix": {
      const name = op.name.trim();
      if (!name) throw new Error("Give the mix a name.");
      return { project: { ...p, mixes: (p.mixes ?? []).map((m) => (m.id === op.id ? { ...m, name } : m)) }, label: `Rename mix to “${name}”` };
    }
    case "removeMix": {
      const mix = (p.mixes ?? []).find((m) => m.id === op.id);
      if (!mix) throw new Error("That mix no longer exists.");
      return { project: { ...p, mixes: (p.mixes ?? []).filter((m) => m.id !== op.id) }, label: `Delete mix “${mix.name}”` };
    }
    case "swapMix": {
      const other = p.otherMix;
      if (!other) return { project: { ...p, otherMix: captureMix(p, newId("x"), "A") }, label: "Compare mixes A/B" };
      const now = captureMix(p, other.id, other.name === "A" ? "B" : "A");
      return { project: { ...applyMix(p, other), otherMix: now }, label: `Switch to mix ${other.name}` };
    }
    case "keepMix": {
      if (!p.otherMix) throw new Error("No A/B comparison is running.");
      return { project: { ...p, otherMix: null }, label: `Keep mix ${p.otherMix.name === "A" ? "B" : "A"}` };
    }
    case "unfreezeTrack":
      return { project: { ...p, tracks: p.tracks.map((t) => (t.id === op.track ? { ...t, frozen: undefined } : t)) }, label: "Unfreeze" };
    case "flattenTrack":
      throw new Error("Flattening needs the desktop app (npm run tauri dev).");
    case "setSidechain": {
      const source = op.source;
      if (source !== null && !p.tracks.some((t) => t.id === source)) throw new Error(`No track with id ${source}`);
      if (source === op.track) throw new Error("A track can't be its own sidechain");
      return { project: mapDevice(p, op.track, op.device, (d) => ({ ...d, sidechain: source ?? undefined })), label: "Sidechain" };
    }
    case "removeDevice":
      return { project: pruneAssets(mapChain(p, op.track, (devices) => devices.filter((d) => d.id !== op.device))), label: "Remove device" };
    case "toggleDevice":
      return { project: mapDevice(p, op.track, op.device, (d) => ({ ...d, on: !d.on })), label: "Toggle device" };
    case "setParam":
      return {
        project: mapDevice(p, op.track, op.device, (d) => ({
          ...d,
          params: d.params.map((x) => {
            if (x.id !== op.param) return x;
            const v = clamp(op.value, x.min, x.max);
            return { ...x, value: x.step ? clamp(x.min + Math.round((v - x.min) / x.step) * x.step, x.min, x.max) : v };
          }),
        })),
        label: "Device parameter",
      };
    case "addChain": {
      const spec = DEVICE_CATALOG.find((d) => d.type === op.kind);
      if (!spec || spec.role !== "instrument" || op.kind === "rack") throw new Error(`${spec?.name ?? op.kind} can't go in a rack chain`);
      const device = { ...createDevice(op.kind), id: newId("dev") };
      const chain: Chain = { id: newId("ch"), name: device.name, on: true, device, keyLow: 0, keyHigh: 127, velLow: 1, velHigh: 127, volume: 0, pan: 0 };
      return { project: mapRack(p, op.track, op.device, (d) => ({ ...d, chains: [...(d.chains ?? []), chain] })), label: `Add a ${device.name} chain`, created: chain.id };
    }
    case "removeChain": {
      const next = mapRack(p, op.track, op.device, (d) => ({
        ...d,
        chains: (d.chains ?? []).filter((c) => c.id !== op.chain),
        macros: (d.macros ?? []).filter((m) => m.chain !== op.chain),
      }));
      return { project: pruneAssets(next), label: "Remove a chain" };
    }
    case "setChain": {
      const patch = op.patch;
      const next = mapRack(p, op.track, op.device, (d) => ({
        ...d,
        chains: (d.chains ?? []).map((c) => {
          if (c.id !== op.chain) return c;
          const n = { ...c, ...patch, name: patch.name?.trim() || c.name };
          // The low end of a zone never passes the high end: moving one pushes the other (mirrors soniq-project).
          if (patch.keyLow !== undefined) n.keyHigh = Math.max(n.keyHigh, n.keyLow);
          if (patch.keyHigh !== undefined) n.keyLow = Math.min(n.keyLow, n.keyHigh);
          if (patch.velLow !== undefined) n.velHigh = Math.max(n.velHigh, n.velLow);
          if (patch.velHigh !== undefined) n.velLow = Math.min(n.velLow, n.velHigh);
          return { ...n, volume: clamp(n.volume, -60, 6), pan: clamp(n.pan, -1, 1) };
        }),
      }));
      return { project: next, label: "Change a chain" };
    }
    case "mapMacro": {
      const { index } = op;
      const next = mapRack(p, op.track, op.device, (d) => {
        const chain = d.chains?.find((c) => c.id === op.chain);
        const param = chain?.device.params.find((x) => x.id === op.param);
        if (!param) throw new Error(`No parameter ${op.param}`);
        const macros = (d.macros ?? []).filter((m) => !(m.chain === op.chain && m.param === op.param));
        if (index === undefined) return { ...d, macros };
        const map: MacroMap = { index, chain: op.chain, param: op.param, min: clamp(op.min ?? param.min, param.min, param.max), max: clamp(op.max ?? param.max, param.min, param.max) };
        // A macro's first mapping takes the parameter's place, so nothing jumps.
        const first = !macros.some((m) => m.index === index);
        const params = first ? d.params.map((x) => (x.id === `macro${index + 1}` ? { ...x, value: macroAmount(map, param, param.value) * 100 } : x)) : d.params;
        return { ...d, params, macros: [...macros, map] };
      });
      return { project: next, label: index === undefined ? "Unmap a macro" : `Map to macro ${index + 1}` };
    }
    case "setTempo":
      return { project: { ...p, bpm: Math.round(clamp(op.bpm, 20, 999) * 100) / 100 }, label: "Tempo" };
    case "setTempoPoints": {
      const points = op.points
        .filter((pt) => Number.isFinite(pt.time) && Number.isFinite(pt.bpm))
        .map((pt) => ({ time: Math.max(0, pt.time), bpm: Math.round(clamp(pt.bpm, 20, 999) * 100) / 100 }))
        .sort((a, b) => a.time - b.time);
      // The song's tempo is where the track starts (mirrors soniq-project).
      return { project: { ...p, tempo: points, bpm: points[0]?.bpm ?? p.bpm }, label: points.length ? "Tempo track" : "Clear the tempo track" };
    }
    case "setWarp": {
      const warp = op.warp ? { ...op.warp, bpm: Math.round(clamp(op.warp.bpm, 20, 999) * 100) / 100 } : undefined;
      const tracks = p.tracks.map((t) => ({ ...t, clips: t.clips.map((c) => (c.id === op.clip && c.kind === "audio" ? { ...c, warp } : c)) }));
      return { project: { ...p, tracks }, label: warp ? "Warp" : "Unwarp" };
    }
    case "setTimeSignature":
      return { project: { ...p, beatsPerBar: clamp(op.beatsPerBar, 1, 32) }, label: "Time signature" };
    case "setKey":
      return { project: { ...p, key: op.key }, label: "Key" };
    case "deleteClips": {
      // The simple form here; ripple editing is the desktop app's.
      const gone = new Set(op.clips);
      const next = { ...p, tracks: p.tracks.map((t) => ({ ...t, clips: t.clips.filter((c) => !gone.has(c.id)) })) };
      return { project: pruneAssets(next), label: op.clips.length === 1 ? "Delete clip" : `Delete ${op.clips.length} clips` };
    }
    case "setFadeCurve": {
      const picked = new Set(op.clips);
      const tracks = p.tracks.map((t) => ({
        ...t,
        clips: t.clips.map((c) => (picked.has(c.id) && c.kind === "audio" ? { ...c, curve: op.curve ?? undefined } : c)),
      }));
      return { project: { ...p, tracks }, label: "Fade shape" };
    }
    case "moveClips":
    case "duplicateClips":
    case "extractToTrack":
      throw new Error("Moving and copying several clips needs the desktop app (npm run tauri dev).");
    case "deleteClip": {
      const name = p.tracks.flatMap((t) => t.clips).find((c) => c.id === op.clip)?.name;
      if (!name) throw new Error("That clip no longer exists.");
      const next = { ...p, tracks: p.tracks.map((t) => ({ ...t, clips: t.clips.filter((c) => c.id !== op.clip) })) };
      return { project: pruneAssets(next), label: `Delete clip “${name}”` };
    }
    case "moveClip": {
      const from = p.tracks.find((t) => t.clips.some((c) => c.id === op.clip));
      const to = p.tracks.find((t) => t.id === op.track);
      const clip = from?.clips.find((c) => c.id === op.clip);
      if (!from || !to || !clip) throw new Error("That clip no longer exists.");
      if (from.kind !== to.kind) throw new Error("Audio clips go on audio tracks and MIDI clips on MIDI tracks.");
      const moved = { ...clip, start: Math.max(0, op.start) };
      const tracks = p.tracks.map((t) => {
        const rest = t.clips.filter((c) => c.id !== op.clip);
        return t.id === to.id ? { ...t, clips: [...rest, moved].sort((a, b) => a.start - b.start) } : { ...t, clips: rest };
      });
      return { project: { ...p, tracks }, label: `Move clip “${clip.name}”` };
    }
    case "duplicateClip": {
      const track = p.tracks.find((t) => t.clips.some((c) => c.id === op.clip));
      const clip = track?.clips.find((c) => c.id === op.clip);
      if (!track || !clip) throw new Error("That clip no longer exists.");
      const link = op.linked && clip.kind === "midi" ? (clip.link ?? newId("lnk")) : undefined;
      const original = link && clip.kind === "midi" ? { ...clip, link } : clip;
      const copy = { ...original, id: newId("c"), start: clip.start + clipLength(clip, tempoOf(p)), ...(clip.kind === "midi" ? { link } : {}) };
      const tracks = p.tracks.map((t) => (t.id === track.id ? { ...t, clips: [...t.clips.map((c) => (c.id === clip.id ? original : c)), copy] } : t));
      return { project: { ...p, tracks }, label: `Duplicate clip “${clip.name}”`, created: copy.id };
    }
    case "unlinkClip": {
      const tracks = p.tracks.map((t) => ({ ...t, clips: t.clips.map((c) => (c.id === op.clip && c.kind === "midi" ? { ...c, link: undefined } : c)) }));
      return { project: { ...p, tracks }, label: "Unlink clip" };
    }
    case "setClipFades":
      return {
        project: {
          ...p,
          tracks: p.tracks.map((t) => ({
            ...t,
            clips: t.clips.map((c) => (c.id === op.clip && c.kind === "audio" ? { ...c, fadeIn: Math.max(0, op.fadeIn), fadeOut: Math.max(0, op.fadeOut) } : c)),
          })),
        },
        label: "Fades",
      };
    case "addMarker": {
      const region = op.end !== undefined;
      const count = p.markers.filter((m) => (m.end !== undefined) === region).length + 1;
      const marker = { id: newId("m"), name: op.name?.trim() || `${region ? "Region" : "Marker"} ${count}`, start: Math.max(0, op.start), end: op.end };
      const markers = [...p.markers, marker].sort((a, b) => a.start - b.start);
      return { project: { ...p, markers }, label: region ? "Add region" : "Add marker", created: marker.id };
    }
    case "setMarker": {
      const markers = p.markers
        .map((m) => {
          if (m.id !== op.id) return m;
          if (op.name !== undefined) return { ...m, name: op.name.trim() || m.name };
          const start = Math.max(0, op.start ?? m.start);
          const end = m.end === undefined ? undefined : (op.end ?? m.end + (start - m.start));
          return { ...m, start, end };
        })
        .sort((a, b) => a.start - b.start);
      return { project: { ...p, markers }, label: "Marker" };
    }
    case "removeMarker":
      return { project: { ...p, markers: p.markers.filter((m) => m.id !== op.id) }, label: "Delete marker" };
    case "addMidiClip": {
      const track = p.tracks.find((t) => t.id === op.track);
      if (track?.kind !== "midi") throw new Error("MIDI clips go on MIDI tracks.");
      const slot = op.slot;
      const clip = {
        id: newId("c"),
        kind: "midi" as const,
        name: op.name?.trim() || `${track.name} ${track.clips.length + 1}`,
        start: slot === undefined ? Math.max(0, op.start) : 0,
        length: op.length,
        notes: sortNotes(op.notes ?? []),
        slot,
      };
      // A new session clip replaces the slot's.
      const next = mapTrack(p, op.track, (t) => ({ ...t, clips: [...t.clips.filter((c) => slot === undefined || c.slot !== slot), clip].sort((a, b) => a.start - b.start) }));
      return { project: next, label: "Add MIDI clip", created: clip.id };
    }
    case "setNotes": {
      const notes = sortNotes(
        op.notes.map((n) => ({ ...n, start: Math.max(0, n.start), length: Math.max(1 / 128, n.length), pitch: clamp(Math.round(n.pitch), 0, 127), velocity: clamp(n.velocity, 0.01, 1) })),
      );
      // Linked clips share their notes.
      const edited = p.tracks.flatMap((t) => t.clips).find((c) => c.id === op.clip);
      const link = edited?.kind === "midi" ? edited.link : undefined;
      const hit = (c: Track["clips"][number]) => c.kind === "midi" && (c.id === op.clip || (!!link && c.link === link));
      const tracks = p.tracks.map((t) => ({ ...t, clips: t.clips.map((c) => (hit(c) ? { ...c, notes } : c)) }));
      return { project: { ...p, tracks }, label: op.label ?? "Edit notes" };
    }
    case "renameClip": {
      const tracks = p.tracks.map((t) => ({ ...t, clips: t.clips.map((c) => (c.id === op.clip ? { ...c, name: op.name.trim() || c.name } : c)) }));
      return { project: { ...p, tracks }, label: "Rename clip" };
    }
    case "setDeviceSample": {
      const asset = op.asset;
      const next = mapDevice(p, op.track, op.device, (d) => ({
        ...d,
        samples: [...(d.samples ?? []).filter((s) => s.key !== op.key), ...(asset ? [{ key: op.key, asset }] : [])].sort((a, b) => a.key - b.key),
      }));
      return { project: pruneAssets(next), label: asset ? "Load a sample" : "Clear a sample" };
    }
    case "addMapping": {
      const { target } = op;
      const mapping = { id: newId("map"), channel: op.channel, cc: op.cc, target };
      const mappings = (p.mappings ?? []).filter((m) => !(m.cc === op.cc && m.channel === op.channel) && !sameTarget(m.target, target));
      return { project: { ...p, mappings: [...mappings, mapping] }, label: `Map CC ${op.cc}`, created: mapping.id };
    }
    case "removeMapping":
      return { project: { ...p, mappings: (p.mappings ?? []).filter((m) => m.id !== op.id) }, label: "Remove a MIDI mapping" };
    case "setAutomation": {
      const track = p.tracks.find((t) => t.id === op.track);
      if (!track) throw new Error(`No track with id ${op.track}`);
      const { min, max } = autoRange(track, op.target);
      const points = op.points
        .filter((pt) => Number.isFinite(pt.time) && Number.isFinite(pt.value))
        .map((pt) => ({ time: Math.max(0, pt.time), value: clamp(pt.value, min, max) }))
        .sort((a, b) => a.time - b.time);
      const next = mapTrack(p, op.track, (t) => {
        const rest = (t.automation ?? []).filter((l) => !sameAutoTarget(l.target, op.target));
        return { ...t, automation: points.length ? [...rest, { target: op.target, points }] : rest };
      });
      return { project: next, label: `${points.length ? "Draw" : "Clear"} automation on ${track.name}` };
    }
    case "setModTarget": {
      const target = op.target;
      const next = mapTrack(p, op.track, (t) => ({ ...t, devices: t.devices.map((d) => (d.id === op.device ? { ...d, modulates: target ?? undefined } : d)) }));
      return { project: next, label: "Point modulator" };
    }
    case "groupTracks": {
      const members = p.tracks.filter((t) => op.tracks.includes(t.id));
      if (!members.length) throw new Error("Pick tracks to group");
      if (members.some((t) => t.kind === "group")) throw new Error("Groups can't go inside groups");
      // Every track before the first member stays before the group.
      const at = p.tracks.findIndex((t) => op.tracks.includes(t.id));
      const group = emptyTrack("group", `Group ${p.tracks.filter((t) => t.kind === "group").length + 1}`, members[0].color);
      const rest = p.tracks.filter((t) => !op.tracks.includes(t.id));
      const tracks = [...rest.slice(0, at), group, ...members.map((t) => ({ ...t, group: group.id })), ...rest.slice(at)];
      return { project: { ...p, tracks }, label: "Group tracks", created: group.id };
    }
    case "ungroup": {
      const group = p.tracks.find((t) => t.id === op.group && t.kind === "group");
      if (!group) throw new Error(`No track with id ${op.group}`);
      const tracks = p.tracks.filter((t) => t.id !== op.group).map((t) => (t.group === op.group ? { ...t, group: undefined } : t));
      return { project: { ...p, tracks }, label: `Ungroup ${group.name}` };
    }
    case "setTrackGroup": {
      const moved = p.tracks.find((t) => t.id === op.track);
      if (!moved) throw new Error(`No track with id ${op.track}`);
      if (moved.kind === "group") throw new Error("Groups can't go inside groups");
      const rest = p.tracks.filter((t) => t.id !== op.track);
      // It lands right after the last track of the group it joins or leaves.
      const block = op.group ?? moved.group;
      let at = p.tracks.indexOf(moved);
      if (block) {
        const last = rest.map((t) => t.group).lastIndexOf(block);
        at = last >= 0 ? last + 1 : rest.findIndex((t) => t.id === block) + 1;
      }
      const tracks = [...rest.slice(0, at), { ...moved, group: op.group ?? undefined }, ...rest.slice(at)];
      return { project: { ...p, tracks }, label: `Move ${moved.name}` };
    }
    case "putInSlot": {
      const track = p.tracks.find((t) => t.clips.some((c) => c.id === op.clip));
      const clip = track?.clips.find((c) => c.id === op.clip);
      if (!track || !clip) throw new Error("That clip no longer exists.");
      const scenes = [...p.scenes];
      while (scenes.length <= op.scene) scenes.push(`Scene ${scenes.length + 1}`);
      const copy = { ...clip, id: newId("c"), start: 0, slot: op.scene, follow: undefined, ...(clip.kind === "midi" ? { link: undefined } : {}) };
      const next = mapTrack({ ...p, scenes }, track.id, (t) => ({ ...t, clips: [...t.clips.filter((c) => c.slot !== op.scene), copy] }));
      return { project: next, label: "Put clip in slot", created: copy.id };
    }
    case "clearSlot":
      return { project: pruneAssets(mapTrack(p, op.track, (t) => ({ ...t, clips: t.clips.filter((c) => c.slot !== op.scene) }))), label: "Clear slot" };
    case "setFollow": {
      const follow = op.follow ? { ...op.follow, bars: clamp(op.follow.bars, 0.25, 128) } : undefined;
      const tracks = p.tracks.map((t) => ({ ...t, clips: t.clips.map((c) => (c.id === op.clip && c.slot != null ? { ...c, follow } : c)) }));
      return { project: { ...p, tracks }, label: "Follow action" };
    }
    case "setLaunchQuantize":
      return { project: { ...p, launchQuantize: clamp(op.beats, 0, 32) }, label: "Launch quantisation" };
    case "addScene": {
      const name = `Scene ${p.scenes.length + 1}`;
      return { project: { ...p, scenes: [...p.scenes, name] }, label: `Add ${name}` };
    }
    case "trimClip":
    case "splitClip":
      throw new Error("Trimming and splitting clips needs the desktop app (npm run tauri dev).");
  }
}

export function createPreviewBackend(): Backend {
  const demo = createDemoProject();
  const peaks: Map<string, PeakData> = demo.peaks;
  let project = demo.project;
  let past: Entry[] = [];
  let future: Entry[] = [];
  let dirty = false;

  const reply = (extra: Partial<ProjectReply> = {}): ProjectReply => ({
    project,
    canUndo: past.length > 0,
    canRedo: future.length > 0,
    undoLabel: past[past.length - 1]?.label ?? null,
    redoLabel: future[future.length - 1]?.label ?? null,
    dirty,
    path: null,
    created: null,
    warnings: [],
    clipped: false,
    note: null,
    ...extra,
  });

  const record = (before: Project, label: string, key: string | null) => {
    future = [];
    const last = past[past.length - 1];
    if (key && last?.key === key && Date.now() - last.at < COALESCE_MS) {
      last.at = Date.now();
      return;
    }
    past = [...past.slice(-199), { label, project: before, key, at: Date.now() }];
  };

  const desktopOnly = () => Promise.reject(new Error("Opening, saving and importing files needs the desktop app (npm run tauri dev)."));
  const plugins = () => Promise.reject(new Error("Plugins run in sandboxes of the desktop app (npm run tauri dev)."));
  const needsEngine = () => Promise.reject(new Error("Effects, spectrograms and generators run in the Rust engine: use the desktop app (npm run tauri dev)."));
  let macros: Macro[] = [];

  return {
    load: async () => reply(),
    apply: async (op) => {
      const result = reduce(project, op);
      record(project, result.label, coalesceKey(op));
      project = result.project;
      dirty = true;
      return reply({ created: result.created ?? null });
    },
    undo: async () => {
      const entry = past.pop();
      if (entry) {
        future.push({ ...entry, project, key: null });
        project = entry.project;
      }
      return reply();
    },
    redo: async () => {
      const entry = future.pop();
      if (entry) {
        past.push({ ...entry, project, key: null });
        project = entry.project;
      }
      return reply();
    },
    templates: async () => [
      { id: "empty", name: "Empty song", description: "Two audio and two MIDI tracks, nothing on them.", builtin: true, view: "arrange" },
      { id: "beat", name: "Beat", description: "Drums, bass and keys (in the desktop app, with a pattern).", builtin: true, view: "arrange" },
      { id: "editing", name: "Audio editing", description: "One track, opened in the audio editor.", builtin: true, view: "editor" },
    ],
    saveTemplate: desktopOnly,
    deleteTemplate: desktopOnly,
    recent: async () => [],
    forgetRecent: async () => [],
    crashStatus: async () => ({ enabled: false, asked: true, folder: "", reports: [] }),
    crashEnable: async () => {},
    crashSeen: async () => {},
    crashRead: desktopOnly,
    crashDelete: async () => {},
    crashUi: async () => null,
    historyList: async () => ({ past: past.map((e) => e.label), future: [...future].reverse().map((e) => e.label) }),
    historyJump: async (steps) => {
      for (let i = 0; i < Math.abs(steps); i++) {
        const entry = steps < 0 ? past.pop() : future.pop();
        if (!entry) break;
        (steps < 0 ? future : past).push({ ...entry, project, key: null });
        project = entry.project;
      }
      return reply();
    },
    findAssets: desktopOnly,
    locateAsset: desktopOnly,
    newProject: async () => {
      project = {
        ...demo.project,
        name: "Untitled",
        bpm: 120,
        key: "C major",
        assets: [],
        tracks: [emptyTrack("audio", "Audio 1", 1), emptyTrack("audio", "Audio 2", 5), emptyTrack("midi", "MIDI 1", 6), emptyTrack("midi", "MIDI 2", 2)],
      };
      past = [];
      future = [];
      dirty = false;
      return reply();
    },
    openDemo: async () => {
      project = demo.project;
      past = [];
      future = [];
      dirty = false;
      return reply();
    },
    open: desktopOnly,
    save: desktopOnly,
    importFiles: desktopOnly,
    edit: async (clipId, op, start, end) => {
      if (typeof op !== "string") throw new Error("Loudness measurement needs the desktop app (npm run tauri dev).");
      const clip = project.tracks.flatMap((t) => t.clips).find((c): c is AudioClip => c.id === clipId && c.kind === "audio");
      const data = clip && peaks.get(clip.asset);
      const meta = clip && project.assets.find((a) => a.id === clip.asset);
      if (!clip || !data || !meta) throw new Error("Only audio clips can be edited this way.");
      const perSec = data.sampleRate / data.step;
      const from = (clip.offset + (start ?? 0)) * perSec;
      const to = (clip.offset + (end ?? clip.duration)) * perSec;
      const edited = editPeaks(data, op, from, to);
      const asset = { ...meta, id: newId("a") };
      peaks.set(asset.id, edited.data);
      const before = project;
      project = pruneAssets({
        ...project,
        assets: [...project.assets, asset],
        tracks: project.tracks.map((t) => ({ ...t, clips: t.clips.map((c) => (c.id === clipId ? { ...clip, asset: asset.id } : c)) })),
      });
      record(before, editLabel(op), null);
      dirty = true;
      return reply({ clipped: edited.clipped });
    },
    clipboard: desktopOnly,
    effect: needsEngine,
    previewEffect: needsEngine,
    fixAnalyse: needsEngine,
    repair: needsEngine,
    podcastMeasure: async () => [],
    podcastLevel: needsEngine,
    podcastDuck: needsEngine,
    chaptersExport: desktopOnly,
    stopPreview: async () => {},
    captureNoiseProfile: needsEngine,
    noiseProfile: async () => null,
    spectral: needsEngine,
    spectrogram: needsEngine,
    waveWindow: needsEngine,
    draw: needsEngine,
    generate: needsEngine,
    macros: async () => macros,
    saveMacros: async (next) => {
      macros = next;
    },
    batch: desktopOnly,
    peaks: async (asset) => {
      const data = peaks.get(asset);
      if (!data) throw new Error(`No audio with id ${asset}`);
      return data;
    },
    recoveryAvailable: async () => false,
    restoreRecovery: desktopOnly,
    discardRecovery: async () => {},
    closing: async () => {},
    recordStart: () => Promise.reject(new Error("Recording needs the desktop app (npm run tauri dev).")),
    recordStop: async () => reply(),
    orphanedRecordings: async () => [],
    recoverRecordings: async () => [],
    discardRecordings: async () => {},
    exportMix: () => Promise.reject(new Error("Export needs the desktop app (npm run tauri dev).")),
    midiCapture: () => Promise.reject(new Error("Capture MIDI needs the desktop app (npm run tauri dev).")),
    midiImport: desktopOnly,
    midiExport: desktopOnly,
    loadSample: desktopOnly,
    sampleOnsets: async () => [0],
    warpClip: async () => {
      throw new Error("Finding a clip's tempo needs the desktop app (npm run tauri dev).");
    },
    sliceToMidi: async () => {
      throw new Error("Slicing needs the desktop app (npm run tauri dev).");
    },
    pluginsList: async () => ({ plugins: [], failed: [], standard: [], folders: [] }),
    pluginsSetFolders: async (folders) => ({ plugins: [], failed: [], standard: [], folders }),
    pluginAdd: plugins,
    pluginWindow: plugins,
    pluginParams: plugins,
    pluginTexts: async () => [],
    pluginReload: plugins,
    trackFreeze: () => Promise.reject(new Error("Freezing renders in the Rust engine: use the desktop app (npm run tauri dev).")),
    clipRender: () => Promise.reject(new Error("Rendering clips needs the Rust engine: use the desktop app (npm run tauri dev).")),
  };
}
