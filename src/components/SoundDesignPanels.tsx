import { useEffect, useState } from "react";
import { backend } from "../engine/backend";
import { chainHost } from "../state/devices";
import { usePeaks } from "../state/peaks";
import { useStore } from "../state/store";
import type { ChainPatch, Device, DeviceParam, FxChain, MacroMap } from "../state/types";
import { Waveform } from "./controls/ClipCanvas";
import { Knob } from "./controls/Knob";
import { ParamControl, ParamGroups, panelParams, paramOf, type Mapping, type Owner } from "./DeviceControls";
import { Icon } from "./Icon";
import { Select } from "./Select";
import { trackColor } from "./trackColor";

/** A table's waveforms, fetched once per table: 16 curves of `POINTS` values. */
const POINTS = 96;
const shapes = new Map<number, Promise<number[][]>>();

function useShapes(table: number): number[][] | null {
  const [loaded, setLoaded] = useState<{ table: number; frames: number[][] } | null>(null);
  useEffect(() => {
    let live = true;
    let pending = shapes.get(table);
    if (!pending) {
      pending = backend.wavetableShapes(table, POINTS);
      shapes.set(table, pending);
      // A failed fetch is tried again the next time the table shows.
      pending.catch(() => shapes.delete(table));
    }
    pending.then((frames) => live && setLoaded({ table, frames })).catch(() => undefined);
    return () => {
      live = false;
    };
  }, [table]);
  return loaded?.table === table ? loaded.frames : null;
}

/** The waveform an oscillator plays at its position: between two of the table's waveforms, as the engine reads it. */
function WaveView({ device, osc }: { device: Device; osc: "A" | "B" }) {
  const table = Math.round(paramOf(device, `table${osc}`, 0));
  const pos = paramOf(device, `pos${osc}`, 0) / 100;
  const level = paramOf(device, `level${osc}`, osc === "A" ? 100 : 0);
  const frames = useShapes(table);
  const name = device.params.find((p) => p.id === `table${osc}`)?.options?.[table] ?? "";
  const [w, h] = [128, 46];
  let path = "";
  if (frames?.length) {
    const at = Math.min(1, Math.max(0, pos)) * (frames.length - 1);
    const low = Math.min(frames.length - 2, Math.floor(at));
    const f = at - low;
    const a = frames[low];
    const b = frames[low + 1] ?? a;
    const top = 1.7;
    path = a.map((v, i) => `${i === 0 ? "M" : "L"}${((i / (a.length - 1)) * w).toFixed(1)} ${(h / 2 - ((v + (b[i] - v) * f) / top) * (h / 2 - 2)).toFixed(1)}`).join(" ");
  }
  return (
    <div className={level > 0 ? "wt-wave" : "wt-wave is-silent"} title={`Oscillator ${osc}: ${name}, ${Math.round(pos * 100)} % through the table${level > 0 ? "" : " · its level is 0"}`}>
      <svg width={w} height={h} aria-hidden="true">
        <line x1={0} x2={w} y1={h / 2} y2={h / 2} className="wt-zero" />
        {path && <path d={path} className="wt-line" />}
      </svg>
      <span className="wt-tag">{osc}</span>
      <span className="wt-pos" aria-hidden="true">
        <span style={{ left: `${Math.min(1, Math.max(0, pos)) * 100}%` }} />
      </span>
    </div>
  );
}

/** How far a slot's amount moves its destination, in the destination's own terms. */
function reach(dest: number, amount: number): string {
  const a = Math.abs(amount) / 100;
  if (dest === 2 || dest === 3) return `${(a * 12).toFixed(a * 12 < 1 ? 2 : 1)} st`;
  if (dest === 4) return `${(a * 5).toFixed(1)} oct`;
  return `${Math.round(a * 100)} %`;
}

/** The modulation matrix: each row sends a source to a destination by an amount. */
function Matrix({ track, device }: { track: Owner; device: Device }) {
  const setParam = useStore((s) => s.setParam);
  const param = (id: string) => device.params.find((p) => p.id === id);
  const rows = Array.from({ length: 8 }, (_, i) => i + 1).filter((n) => param(`m${n}src`));
  const choice = (p: DeviceParam, label: string) => (
    <Select className="select sm" aria-label={label} value={Math.round(p.value)} disabled={!device.on} onChange={(e) => setParam(track.id, device.id, p.id, Number(e.target.value))}>
      {(p.options ?? []).map((o, i) => (
        <option key={o} value={i}>
          {o}
        </option>
      ))}
    </Select>
  );
  return (
    <div className="dev-group wt-matrix" role="group" aria-label="Modulation matrix">
      <span className="dev-group-l">Matrix · source → destination, amount</span>
      <div className="wt-rows">
        {rows.map((n) => {
          const [src, dst, amt] = [param(`m${n}src`)!, param(`m${n}dst`)!, param(`m${n}amt`)!];
          const off = Math.round(src.value) === 0;
          return (
            <div key={n} className={off ? "wt-slot is-off" : "wt-slot"}>
              <span className="selwrap">{choice(src, `Slot ${n} source`)}</span>
              <Icon name="chevronRight" size={12} className="dim" />
              <span className="selwrap">{choice(dst, `Slot ${n} destination`)}</span>
              <input
                className="range mini"
                type="range"
                min={-100}
                max={100}
                step={1}
                value={Math.round(amt.value)}
                disabled={!device.on || off}
                aria-label={`Slot ${n} amount`}
                title="Drag to set how far, and which way; double-click for none"
                onChange={(e) => setParam(track.id, device.id, amt.id, Number(e.target.value))}
                onDoubleClick={() => setParam(track.id, device.id, amt.id, 0)}
              />
              <span className="wt-amt mono">
                {amt.value > 0 ? "+" : amt.value < 0 ? "−" : ""}
                {reach(Math.round(dst.value), amt.value)}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

/** The Wavetable synth: both oscillators' waveforms, its knobs by section, and the modulation matrix. */
export function WavetableBody({ track, device, mapping }: { track: Owner; device: Device; mapping?: Mapping }) {
  return (
    <div className="dev-row wt">
      <div className="wt-side">
        <WaveView device={device} osc="A" />
        <WaveView device={device} osc="B" />
      </div>
      <ParamGroups track={track} device={device} params={panelParams(device).filter((p) => p.group !== "Matrix")} mapping={mapping} />
      <Matrix track={track} device={device} />
    </div>
  );
}

/** A convolution reverb's own impulse response: an audio file that takes the place of the built-in space. */
export function IrSlot({ track, device }: { track: Owner; device: Device }) {
  const ref = device.samples?.find((s) => s.key === 0);
  const asset = useStore((s) => (ref ? s.project.assets.find((a) => a.id === ref.asset) : undefined));
  const data = usePeaks(asset && !asset.missing ? asset.id : undefined);
  const { loadSample, clearSample } = useStore.getState();
  return (
    <div className={asset ? "smp-slot ir-slot" : "smp-slot ir-slot empty-slot"} data-sample-target={`${track.id}|${device.id}|0`}>
      <div className="smp-wave">
        {data ? (
          <Waveform peaks={data.channels[0]} peaksR={data.channels[1]} width={200} height={40} color={trackColor(track.color || 6)} />
        ) : (
          <span className="muted">{asset?.missing ? "The file is missing: the space below plays" : "Drop an impulse response here to use it instead of the space"}</span>
        )}
      </div>
      <div className="smp-row">
        <span className="smp-name" title={asset ? `${asset.name} plays instead of the space; Length shortens it` : undefined}>
          {asset?.name ?? "Built-in space"}
        </span>
        <button className="btn xs" type="button" onClick={() => void loadSample(track.id, device.id, 0)}>
          Load…
        </button>
        {asset && (
          <button className="ib xs" type="button" aria-label="Back to the built-in space" onClick={() => clearSample(track.id, device.id, 0)}>
            <Icon name="close" size={13} />
          </button>
        )}
      </div>
    </div>
  );
}

/** What a chain's effects are, in a few words. */
function chainSummary(chain: FxChain): string {
  if (!chain.devices.length) return "Dry · no effects";
  const names = chain.devices.map((d) => d.name);
  return names.length > 2 ? `${names.slice(0, 2).join(", ")} +${names.length - 2}` : names.join(", ");
}

/**
 * An audio effect rack: eight macros and its Dry/Wet, and the chains that
 * run side by side. Opening a chain shows its effects in the devices panel,
 * where a right-click on a knob maps it to a macro.
 */
export function FxRackBody({ track, device }: { track: Owner; device: Device }) {
  const run = useStore((s) => s.run);
  const selectChain = useStore((s) => s.selectChain);
  const chains = device.fxChains ?? [];
  const macros = device.macros ?? [];
  const [selected, setSelected] = useState<string | null>(null);
  const chain = chains.find((c) => c.id === selected) ?? chains[0];
  const macroParams = device.params.filter((p) => p.id.startsWith("macro"));
  const wet = device.params.find((p) => p.id === "wet");
  const set = (id: string, patch: ChainPatch) => void run({ op: "setChain", track: track.id, device: device.id, chain: id, patch });
  const open = (id: string) => selectChain(`${track.id}/${device.id}/${id}`);

  const addChain = async () => {
    const before = new Set(chains.map((c) => c.id));
    await run({ op: "addChain", track: track.id, device: device.id });
    const added = chainHost(useStore.getState().project, track.id)
      ?.devices.find((d) => d.id === device.id)
      ?.fxChains?.find((c) => !before.has(c.id));
    if (added) setSelected(added.id);
  };
  const mapsOf = (index: number) => macros.filter((m) => m.index === index);
  const describe = (m: MacroMap) => {
    const d = chains.find((c) => c.id === m.chain)?.devices.find((x) => x.id === m.device);
    return `${d?.name ?? "?"} · ${d?.params.find((p) => p.id === m.param)?.label ?? m.param}`;
  };

  return (
    <div className="rack fx-rack">
      <div className="dev-group rack-macros" role="group" aria-label="Macros">
        <span className="dev-group-l">Macros</span>
        <div className="rack-macros-body">
          <div className="macro-grid">
            {macroParams.map((p, i) => (
              <div key={p.id} className={mapsOf(i).length ? "macro has-maps" : "macro"} title={mapsOf(i).map(describe).join("\n") || "Open a chain and right-click a knob to map it here"}>
                <ParamControl track={track} device={device} param={p} size={30} />
                {mapsOf(i).length > 0 && <span className="macro-n">{mapsOf(i).length}</span>}
              </div>
            ))}
          </div>
          {wet && <ParamControl track={track} device={device} param={wet} size={30} />}
        </div>
      </div>

      <div className="rack-chains" role="group" aria-label="Chains">
        <div className="rack-list" role="list">
          {chains.map((c) => (
            <div key={c.id} role="listitem" className={`rack-chain${c.id === chain?.id ? " sel" : ""}${c.on ? "" : " is-off"}`}>
              <button className="switch sm" type="button" role="switch" aria-checked={c.on} aria-label={`${c.name} on`} onClick={() => set(c.id, { on: !c.on })} />
              <button className="rack-chain-name" type="button" aria-pressed={c.id === chain?.id} title="Click to select · double-click to open its effects" onClick={() => setSelected(c.id)} onDoubleClick={() => open(c.id)}>
                <span className="rack-chain-title">{c.name}</span>
                <span className="rack-chain-zone">
                  {c.volume > 0 ? "+" : ""}
                  {c.volume.toFixed(1)} dB
                </span>
                <span className="fx-chain-sum">{chainSummary(c)}</span>
              </button>
              <button className="ib xs tip-t" type="button" data-tip="Open its effects" aria-label={`Open the effects of ${c.name}`} onClick={() => open(c.id)}>
                <Icon name="chevronRight" size={13} />
              </button>
              <button className="ib xs" type="button" aria-label={`Remove ${c.name}`} onClick={() => void run({ op: "removeChain", track: track.id, device: device.id, chain: c.id })}>
                <Icon name="close" size={13} />
              </button>
            </div>
          ))}
          <button className="btn xs rack-add" type="button" onClick={() => void addChain()}>
            <Icon name="plus" size={13} />
            Add chain
          </button>
        </div>
      </div>

      {chain ? (
        <div className="fx-chain-edit" role="group" aria-label={`${chain.name}: name, level and pan`}>
          <input
            className="mk-input fx-chain-name"
            key={`${chain.id}-${chain.name}`}
            defaultValue={chain.name}
            aria-label="Chain name"
            onBlur={(e) => e.currentTarget.value.trim() && e.currentTarget.value.trim() !== chain.name && set(chain.id, { name: e.currentTarget.value })}
            onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          />
          <div className="dev-grid rack-zone">
            <Knob label="Level" value={chain.volume} min={-36} max={6} defaultValue={0} size={30} format={(v) => `${v > 0 ? "+" : ""}${v.toFixed(1)} dB`} onChange={(v) => set(chain.id, { volume: Math.round(v * 10) / 10 })} />
            <Knob
              label="Pan"
              value={chain.pan}
              min={-1}
              max={1}
              defaultValue={0}
              size={30}
              bipolar
              format={(v) => (Math.abs(v) < 0.005 ? "C" : `${Math.round(Math.abs(v) * 100)}${v < 0 ? "L" : "R"}`)}
              onChange={(v) => set(chain.id, { pan: Math.round(v * 100) / 100 })}
            />
          </div>
          <button className="btn xs" type="button" onClick={() => open(chain.id)}>
            Open effects
            <Icon name="chevronRight" size={12} />
          </button>
        </div>
      ) : (
        <p className="rack-empty muted">Add a chain for each path the sound should take: they run side by side and are added up. A chain without effects is the dry sound.</p>
      )}
    </div>
  );
}
