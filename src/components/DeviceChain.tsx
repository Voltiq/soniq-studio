import { useEffect, useMemo, useState, type DragEvent } from "react";
import { backend } from "../engine/backend";
import { engine } from "../engine/bridge";
import { DEVICE_CATALOG, LIVE_TYPES, SIDECHAIN_TYPES, chainHost, chainParams, deviceSpec, isMidiFx, isModulator, padName } from "../state/devices";
import { noteName } from "../state/music";
import { usePeaks } from "../state/peaks";
import { useStore } from "../state/store";
import type { Chain, ChainPatch, Device, MacroMap, Monitor, Track } from "../state/types";
import { DEVICE_DRAG_TYPE } from "./BrowserPanel";
import { Waveform } from "./controls/ClipCanvas";
import { Knob } from "./controls/Knob";
import { DynamicsCurve, EqEditor, ReductionMeter } from "./EffectPanels";
import { ParamControl, ParamGroups, panelParams, paramOf, type Mapping, type Owner } from "./DeviceControls";
import { Icon } from "./Icon";
import { Popover } from "./Menu";
import { PluginBody } from "./PluginPanel";
import { SavePreset } from "./LibraryPanel";
import { trackColor, trackVar } from "./trackColor";
import { Select } from "./Select";

/** Where the sampler's slices start (fractions of the sample), as the engine cuts them: equal pieces of start–end, or the transients inside it. */
function slicePoints(start: number, end: number, count: number, onsets: number[] | null): number[] {
  if (onsets) {
    const inside = onsets.filter((f) => f >= start && f < end).slice(0, count);
    return inside.length ? inside : [start];
  }
  return Array.from({ length: count }, (_, k) => start + ((end - start) * k) / count);
}

/** Plays a note on a track for a moment, as if a key were tapped. */
function tap(trackId: string, pitch: number) {
  engine.note(trackId, pitch, 0.9, true);
  window.setTimeout(() => engine.note(trackId, pitch, 0, false), 150);
}

/** The sampler's sample: its waveform and name, a file picker, and a drop zone for files from Explorer. In slice mode, the slices: click one to hear it. */
function SampleSlot({ track, device }: { track: Owner; device: Device }) {
  const ref = device.samples?.find((s) => s.key === 0);
  const asset = useStore((s) => (ref ? s.project.assets.find((a) => a.id === ref.asset) : undefined));
  const bpm = useStore((s) => s.project.bpm);
  const bar = useStore((s) => s.project.beatsPerBar);
  const data = usePeaks(asset && !asset.missing ? asset.id : undefined);
  const { loadSample, clearSample, newMidiClip } = useStore.getState();

  const start = paramOf(device, "start", 0) / 100;
  const end = Math.max(start, paramOf(device, "end", 100) / 100);
  const slicing = Math.round(paramOf(device, "mode", 0)) === 3;
  const transients = slicing && Math.round(paramOf(device, "sliceBy", 0)) === 1;
  const sensitivity = Math.round(paramOf(device, "sensitivity", 50));
  const root = Math.round(paramOf(device, "root", 60));
  const [onsets, setOnsets] = useState<{ key: string; at: number[] } | null>(null);
  const onsetKey = transients && asset && !asset.missing ? `${asset.id}|${sensitivity}` : null;
  useEffect(() => {
    if (!onsetKey || !asset) return;
    let live = true;
    backend
      .sampleOnsets(asset.id, sensitivity)
      .then((at) => live && setOnsets({ key: onsetKey, at }))
      .catch(() => live && setOnsets({ key: onsetKey, at: [0] }));
    return () => {
      live = false;
    };
  }, [onsetKey, asset, sensitivity]);
  const points = slicing ? slicePoints(start, end, Math.round(paramOf(device, "slices", 8)), transients ? (onsets?.key === onsetKey ? onsets.at : null) : null) : [];

  /** A clip that plays every slice in order, at the timing it has in the sample. */
  const toClip = () => {
    if (!asset || !points.length) return;
    const speed = Math.pow(2, paramOf(device, "tune", 0) / 12);
    const beats = (fraction: number) => Math.round(((fraction * asset.frames) / asset.sampleRate / speed) * (bpm / 60) * 1000) / 1000;
    const notes = points.map((p, k) => ({
      start: beats(p - points[0]),
      length: Math.max(0.01, beats((points[k + 1] ?? end) - p)),
      pitch: root + k,
      velocity: 0.8,
    }));
    const total = beats(end - points[0]);
    void newMidiClip(track.id, undefined, Math.max(bar, Math.ceil(total / bar) * bar), notes, `${asset.name.replace(/\.[^.]+$/, "")} slices`);
  };

  return (
    <div className={asset ? "smp-slot" : "smp-slot empty-slot"} data-sample-target={`${track.id}|${device.id}|0`}>
      <div className="smp-wave">
        {data ? (
          <>
            <Waveform peaks={data.channels[0]} peaksR={data.channels[1]} width={200} height={40} color={trackColor(track.color)} />
            {start > 0 && <span className="smp-out" style={{ left: 0, width: `${start * 100}%` }} />}
            {end < 1 && <span className="smp-out" style={{ left: `${end * 100}%`, right: 0 }} />}
            {points.map((p, k) => (
              <button
                key={k}
                type="button"
                className="smp-slice"
                style={{ left: `${p * 100}%`, width: `${((points[k + 1] ?? end) - p) * 100}%` }}
                title={`Slice ${k + 1} · ${noteName(root + k)}`}
                aria-label={`Play slice ${k + 1}, ${noteName(root + k)}`}
                onPointerDown={(e) => e.button === 0 && tap(track.id, root + k)}
                onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), tap(track.id, root + k))}
              />
            ))}
          </>
        ) : (
          <span className="muted">{asset?.missing ? "The sample file is missing" : "Drop an audio file here"}</span>
        )}
      </div>
      {slicing && asset && (
        <div className="smp-row smp-slices">
          <span className="muted">
            {points.length} slices · {noteName(root)}–{noteName(root + points.length - 1)}
          </span>
          <button className="btn xs tip-t" type="button" data-tip="A MIDI clip that plays the slices in order" onClick={toClip}>
            To clip
          </button>
        </div>
      )}
      <div className="smp-row">
        <span className="smp-name" title={asset?.name}>
          {asset?.name ?? "No sample"}
        </span>
        <button className="btn xs" type="button" onClick={() => void loadSample(track.id, device.id, 0)}>
          Load…
        </button>
        {asset && (
          <button className="ib xs" type="button" aria-label="Remove the sample" onClick={() => clearSample(track.id, device.id, 0)}>
            <Icon name="close" size={13} />
          </button>
        )}
      </div>
    </div>
  );
}

/** Four pages of 16 pads: the kit, its percussion, and empty pads for your samples. */
const PAGES = ["Kit", "Perc", "Perc 2", "More"];

/** A page's pads as they sit on a 4×4 grid: its first pad bottom left. */
function padRows(page: number): number[][] {
  const first = 36 + page * 16;
  return [12, 8, 4, 0].map((row) => [0, 1, 2, 3].map((col) => first + row + col));
}

/** 64 pads in four pages (the bottom-left one of the first is C1, the kick): click to hear one, drop a sample on it to replace its sound. */
function DrumPads({ track, device }: { track: Owner; device: Device }) {
  const [pad, setPad] = useState(36);
  const [page, setPage] = useState(0);
  const assets = useStore((s) => s.project.assets);
  const { loadSample, clearSample, selectChain } = useStore.getState();
  const effectsOf = (key: string) => device.padChains?.find((c) => c.key === key)?.devices.length ?? 0;
  const pageUsed = (p: number) => Array.from({ length: 16 }, (_, i) => 36 + p * 16 + i).some((n) => sampleOf(n) || effectsOf(String(n)));
  const sampleOf = (note: number) => device.samples?.find((x) => x.key === note);
  const nameOf = (note: number) => {
    const ref = sampleOf(note);
    return ref ? (assets.find((a) => a.id === ref.asset)?.name ?? "Sample") : padName(note);
  };
  const hit = (note: number) => {
    setPad(note);
    tap(track.id, note);
  };
  const padParams = device.params.filter((p) => p.group === `pad:${pad}`);
  return (
    <div className="drum-rack">
      <div className="pad-side">
      <div className="seg pad-pages" role="tablist" aria-label="Pad pages">
        {PAGES.map((label, p) => (
          <button
            key={label}
            type="button"
            role="tab"
            aria-selected={page === p}
            className={pageUsed(p) ? "used" : undefined}
            title={`Pads ${noteName(36 + p * 16)}–${noteName(51 + p * 16)}`}
            onClick={() => setPage(p)}
          >
            {label}
          </button>
        ))}
      </div>
      <div className="pads" role="group" aria-label={`Pads, page ${PAGES[page]}`}>
        {padRows(page).flat().map((note) => (
          <button
            key={note}
            type="button"
            className={`pad${note === pad ? " on" : ""}${sampleOf(note) ? " has-sample" : ""}${effectsOf(String(note)) ? " has-fx" : ""}${!sampleOf(note) && padName(note).startsWith("Pad ") ? " empty-pad" : ""}`}
            data-sample-target={`${track.id}|${device.id}|${note}`}
            aria-pressed={note === pad}
            title={`${noteName(note)} · ${nameOf(note)} · drop an audio file to replace it`}
            onPointerDown={(e) => e.button === 0 && hit(note)}
            onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), hit(note))}
          >
            <span className="pad-name">{nameOf(note)}</span>
            <span className="pad-note">{noteName(note)}</span>
          </button>
        ))}
      </div>
      </div>
      <div className="pad-edit">
        <div className="pad-head">
          <b>{nameOf(pad)}</b>
          <span className="muted">
            {noteName(pad)} · {sampleOf(pad) ? "sample" : padName(pad).startsWith("Pad ") ? "empty" : "built-in"}
          </span>
        </div>
        <div className="pad-chains" role="group" aria-label="Effects">
          {[
            [String(pad), "Pad effects"],
            ["A", "Return A"],
            ["B", "Return B"],
          ].map(([key, label]) => (
            <button
              key={key}
              type="button"
              className="btn xs"
              title={key === "A" || key === "B" ? `The effects every pad's Send ${key} goes through` : `Effects on ${nameOf(pad)} only`}
              onClick={() => selectChain(`${track.id}/${device.id}/${key}`)}
            >
              {label}
              {effectsOf(key) > 0 && <span className="count">{effectsOf(key)}</span>}
              <Icon name="chevronRight" size={12} />
            </button>
          ))}
        </div>
        <div className="pad-actions">
          <button className="btn xs" type="button" onClick={() => void loadSample(track.id, device.id, pad)}>
            Load sample…
          </button>
          {sampleOf(pad) && (
            <button className="btn xs" type="button" onClick={() => clearSample(track.id, device.id, pad)}>
              Use built-in
            </button>
          )}
        </div>
        <div className="dev-grid">
          {padParams.map((p) => (
            <ParamControl key={p.id} track={track} device={device} param={p} size={32} />
          ))}
        </div>
      </div>
    </div>
  );
}

/** A rack chain's zone and level controls. */
function ChainControls({ track, rack, chain }: { track: Owner; rack: Device; chain: Chain }) {
  const run = useStore((s) => s.run);
  const set = (patch: ChainPatch) => void run({ op: "setChain", track: track.id, device: rack.id, chain: chain.id, patch });
  const knob = (label: string, value: number, min: number, max: number, def: number, format: (v: number) => string, apply: (v: number) => ChainPatch, bipolar?: boolean) => (
    <Knob label={label} value={value} min={min} max={max} defaultValue={def} size={28} bipolar={bipolar} format={format} onChange={(v) => set(apply(v))} />
  );
  return (
    <div className="dev-grid rack-zone" role="group" aria-label={`${chain.name} zone and level`}>
      {knob("Key lo", chain.keyLow, 0, 127, 0, (v) => noteName(Math.round(v)), (v) => ({ keyLow: Math.round(v) }))}
      {knob("Key hi", chain.keyHigh, 0, 127, 127, (v) => noteName(Math.round(v)), (v) => ({ keyHigh: Math.round(v) }))}
      {knob("Vel lo", chain.velLow, 1, 127, 1, (v) => String(Math.round(v)), (v) => ({ velLow: Math.round(v) }))}
      {knob("Vel hi", chain.velHigh, 1, 127, 127, (v) => String(Math.round(v)), (v) => ({ velHigh: Math.round(v) }))}
      {knob("Level", chain.volume, -36, 6, 0, (v) => `${v > 0 ? "+" : ""}${v.toFixed(1)} dB`, (v) => ({ volume: Math.round(v * 10) / 10 }))}
      {knob("Pan", chain.pan, -1, 1, 0, (v) => (Math.abs(v) < 0.005 ? "C" : `${Math.round(Math.abs(v) * 100)}${v < 0 ? "L" : "R"}`), (v) => ({ pan: Math.round(v * 100) / 100 }), true)}
    </div>
  );
}

/**
 * An instrument rack: eight macros, the chains (each an instrument for a
 * range of keys and velocities), and the selected chain's instrument, whose
 * knobs map to a macro on right-click.
 */
function RackBody({ track, device }: { track: Owner; device: Device }) {
  const run = useStore((s) => s.run);
  const chains = device.chains ?? [];
  const macros = device.macros ?? [];
  const [selected, setSelected] = useState<string | null>(null);
  const [adding, setAdding] = useState<HTMLElement | null>(null);
  const chain = chains.find((c) => c.id === selected) ?? chains[0];
  const instruments = DEVICE_CATALOG.filter((d) => d.role === "instrument" && d.type !== "rack");
  const macroParams = device.params.filter((p) => p.id.startsWith("macro"));
  const volume = device.params.find((p) => p.id === "volume");

  const addChain = async (kind: string) => {
    setAdding(null);
    const before = new Set(chains.map((c) => c.id));
    await run({ op: "addChain", track: track.id, device: device.id, kind });
    const after = useStore.getState().project.tracks.find((t) => t.id === track.id)?.devices.find((d) => d.id === device.id)?.chains ?? [];
    const added = after.find((c) => !before.has(c.id));
    if (added) setSelected(added.id);
  };
  const mapsOf = (index: number) => macros.filter((m) => m.index === index);
  const describe = (m: MacroMap) => {
    const c = chains.find((x) => x.id === m.chain);
    return `${c?.name ?? "?"} · ${c?.device.params.find((p) => p.id === m.param)?.label ?? m.param}`;
  };
  const chainDevice = chain ? { ...chain.device, params: chainParams(device, chain) } : null;
  const mapping: Mapping | undefined = chain && {
    macroOf: (id) => macros.find((m) => m.chain === chain.id && m.param === id)?.index,
    count: (index) => mapsOf(index).length,
    map: (param, index) => void run({ op: "mapMacro", track: track.id, device: device.id, chain: chain.id, param: param.id, index }),
  };

  return (
    <div className="rack">
      <div className="dev-group rack-macros" role="group" aria-label="Macros">
        <span className="dev-group-l">Macros</span>
        <div className="rack-macros-body">
          <div className="macro-grid">
            {macroParams.map((p, i) => (
              <div key={p.id} className={mapsOf(i).length ? "macro has-maps" : "macro"} title={mapsOf(i).map(describe).join("\n") || "Right-click a chain's knob to map it here"}>
                <ParamControl track={track} device={device} param={p} size={30} />
                {mapsOf(i).length > 0 && <span className="macro-n">{mapsOf(i).length}</span>}
              </div>
            ))}
          </div>
          {volume && <ParamControl track={track} device={device} param={volume} size={30} />}
        </div>
      </div>

      <div className="rack-chains" role="group" aria-label="Chains">
        <div className="rack-list" role="list">
          {chains.map((c) => (
            <div key={c.id} role="listitem" className={`rack-chain${c.id === chain?.id ? " sel" : ""}${c.on ? "" : " is-off"}`}>
              <button
                className="switch sm"
                type="button"
                role="switch"
                aria-checked={c.on}
                aria-label={`${c.name} on`}
                onClick={() => void run({ op: "setChain", track: track.id, device: device.id, chain: c.id, patch: { on: !c.on } })}
              />
              <button className="rack-chain-name" type="button" aria-pressed={c.id === chain?.id} onClick={() => setSelected(c.id)}>
                <span className="rack-chain-title">{c.name}</span>
                <span className="rack-chain-zone">
                  {noteName(c.keyLow)}–{noteName(c.keyHigh)}
                  {(c.velLow > 1 || c.velHigh < 127) && ` · vel ${c.velLow}–${c.velHigh}`}
                </span>
                <span className="zone-bar" aria-hidden="true">
                  <span style={{ left: `${(c.keyLow / 128) * 100}%`, width: `${((c.keyHigh - c.keyLow + 1) / 128) * 100}%` }} />
                </span>
              </button>
              <button
                className="ib xs"
                type="button"
                aria-label={`Remove the ${c.name} chain`}
                onClick={() => void run({ op: "removeChain", track: track.id, device: device.id, chain: c.id })}
              >
                <Icon name="close" size={13} />
              </button>
            </div>
          ))}
          <button className="btn xs rack-add" type="button" aria-haspopup="menu" onClick={(e) => setAdding(adding ? null : e.currentTarget)}>
            <Icon name="plus" size={13} />
            Add chain
          </button>
        </div>
        {chain && <ChainControls track={track} rack={device} chain={chain} />}
      </div>

      {chain && chainDevice ? (
        <div className="rack-inst" role="group" aria-label={`${chain.name} instrument`}>
          <div className="dev-row">
            {chainDevice.type === "drum-rack" ? (
              <DrumPads track={track} device={chainDevice} />
            ) : chainDevice.type === "sampler" ? (
              <SampleSlot track={track} device={chainDevice} />
            ) : null}
            <ParamGroups track={track} device={chainDevice} params={panelParams(chainDevice)} mapping={mapping} />
          </div>
        </div>
      ) : (
        <p className="rack-empty muted">Add a chain for each sound: layer them, or split them by key or velocity.</p>
      )}

      {adding && (
        <Popover anchor={adding} onClose={() => setAdding(null)} label="Add chain">
          {instruments.map((d) => (
            <button key={d.type} type="button" role="menuitem" className="mi" onClick={() => void addChain(d.type)}>
              <span className="mi-check" />
              <span className="mi-label">{d.name}</span>
            </button>
          ))}
        </Popover>
      )}
    </div>
  );
}

/** Devices that turn the sound down, and show by how much. */
const REDUCING = new Set(["comp", "gate", "limiter", "glue", "deesser"]);

function DeviceCard({ track, device }: { track: Owner; device: Device }) {
  const { toggleDevice, removeDevice } = useStore.getState();
  const [presetAnchor, setPresetAnchor] = useState<HTMLElement | null>(null);
  // Effects inside a Drum Rack have no meters of their own.
  const nested = track.id.includes("/");
  const params = panelParams(device);
  const live = LIVE_TYPES.has(device.type);
  return (
    <section className={device.on ? "dev shine" : "dev shine is-off"} aria-label={device.name}>
      <header className="dev-head">
        <button
          className="switch sm"
          type="button"
          role="switch"
          aria-checked={device.on}
          aria-label={`${device.name} on`}
          onClick={() => toggleDevice(track.id, device.id)}
        />
        <span className="dev-name">{device.name}</span>
        {REDUCING.has(device.type) && device.on && !nested && <ReductionMeter id={device.id} />}
        {device.role === "instrument" && <span className="tag">Instrument</span>}
        {device.type === "plugin" ? (
          <span className="tag tip-b" data-tip={device.plugin?.path ?? ""}>
            {device.plugin?.format === "vst3" ? "VST3" : "CLAP"}
          </span>
        ) : !live && (
          <span className="tag tip-b" data-tip={`Real-time ${device.role === "effect" ? "effects" : "devices"} arrive in phase ${deviceSpec(device.type)?.phase ?? 4}`}>
            Not yet audible
          </span>
        )}
        {device.type !== "plugin" && (
          <button
            className="ib xs tip-b"
            type="button"
            data-tip="Save as a preset"
            aria-label={`Save ${device.name} as a preset`}
            onClick={(e) => setPresetAnchor(presetAnchor ? null : e.currentTarget)}
          >
            <Icon name="book" size={13} />
          </button>
        )}
        <button className="ib xs" type="button" aria-label={`Remove ${device.name}`} onClick={() => removeDevice(track.id, device.id)}>
          <Icon name="close" size={14} />
        </button>
        {presetAnchor && <SavePreset owner={track.id} deviceId={device.id} name={device.name} anchor={presetAnchor} onClose={() => setPresetAnchor(null)} />}
      </header>
      {device.type === "plugin" ? (
        <PluginBody track={track} device={device} />
      ) : device.type === "rack" ? (
        <RackBody track={track} device={device} />
      ) : device.type === "eq" ? (
        <EqEditor track={track} device={device} />
      ) : device.type === "drum-rack" || device.type === "sampler" ? (
        <div className="dev-row">
          {device.type === "drum-rack" ? <DrumPads track={track} device={device} /> : <SampleSlot track={track} device={device} />}
          <ParamGroups track={track} device={device} params={params} />
        </div>
      ) : isModulator(device.type) ? (
        <div className="dev-row dyn-row">
          <ModTargetPicker track={track} device={device} />
          <ParamGroups track={track} device={device} params={params} />
        </div>
      ) : SIDECHAIN_TYPES.has(device.type) ? (
        <div className="dev-row dyn-row">
          <div className="dyn-side">
            {device.type !== "vocoder" && <DynamicsCurve device={device} />}
            <SidechainPicker track={track} device={device} />
          </div>
          <ParamGroups track={track} device={device} params={params} />
        </div>
      ) : (
        <ParamGroups track={track} device={device} params={params} />
      )}
    </section>
  );
}

/** Which track a compressor or gate listens to (its own input, or another track's own sound), or the track a vocoder speaks through. */
function SidechainPicker({ track, device }: { track: Owner; device: Device }) {
  const tracks = useStore((s) => s.project.tracks);
  const setSidechain = useStore((s) => s.setSidechain);
  const vocoder = device.type === "vocoder";
  return (
    <label
      className="dev-opt sc-pick tip-t"
      data-tip={
        vocoder
          ? "The track whose sound the voice shapes (its own sound, before its effects and fader), e.g. a pad. Set Carrier to “Sidechain track” to hear it."
          : "Listen to another track's own sound (before its effects and fader), e.g. duck the bass under the kick"
      }
    >
      <span className="knob-l">{vocoder ? "Carrier track" : "Sidechain"}</span>
      <span className="selwrap">
        <Select
          className="select sm"
          value={device.sidechain ?? ""}
          disabled={!device.on}
          onChange={(e) => setSidechain(track.id, device.id, e.target.value || null)}
        >
          <option value="">{vocoder ? "None · built-in saw" : "Own input"}</option>
          {tracks
            .filter((t) => t.id !== track.id)
            .map((t) => (
              <option key={t.id} value={t.id}>
                {t.name}
              </option>
            ))}
        </Select>
      </span>
    </label>
  );
}

const MONITOR_LABELS: Record<Monitor, string> = { off: "Monitor off", auto: "Monitor when armed", in: "Monitor always" };

/** Which input channels an audio track records, and whether it plays them back. */
function InputPicker({ track }: { track: Track }) {
  const channels = useStore((s) => s.inputStatus?.channels ?? 2);
  const inputRate = useStore((s) => (s.inputStatus?.open ? s.inputStatus.sampleRate : null));
  const outputRate = useStore((s) => s.engineStatus?.sampleRate ?? null);
  const updateTrack = useStore((s) => s.updateTrack);
  const options: string[] = [];
  for (let c = 1; c <= channels; c++) options.push(String(c));
  for (let c = 1; c + 1 <= channels; c += 2) options.push(`${c}/${c + 1}`);
  if (!options.includes(track.input)) options.push(track.input);
  const mismatch = track.monitor !== "off" && inputRate !== null && outputRate !== null && inputRate !== outputRate;
  return (
    <div className="input-pick">
      <Icon name="mic" size={14} className="dim" />
      <span className="selwrap">
        <Select
          className="select sm"
          aria-label={`${track.name} records from`}
          value={track.input}
          onChange={(e) => updateTrack(track.id, { input: e.target.value })}
        >
          {options.map((o) => (
            <option key={o} value={o}>
              {o.includes("/") ? `In ${o} · stereo` : `In ${o} · mono`}
            </option>
          ))}
        </Select>
      </span>
      <Icon name="headphones" size={14} className="dim" />
      <span className="selwrap tip-t" data-tip="Hear the input through this track. Use headphones: speakers near a microphone howl.">
        <Select
          className="select sm"
          aria-label={`${track.name} monitoring`}
          value={track.monitor}
          onChange={(e) => updateTrack(track.id, { monitor: e.target.value as Monitor })}
        >
          {(Object.keys(MONITOR_LABELS) as Monitor[]).map((m) => (
            <option key={m} value={m}>
              {MONITOR_LABELS[m]}
            </option>
          ))}
        </Select>
      </span>
      {mismatch && (
        <span className="warn-note" title={`The input runs at ${inputRate / 1000} kHz and the output at ${outputRate / 1000} kHz.`}>
          <Icon name="warning" size={13} />
          Set input and output to the same sample rate to monitor
        </span>
      )}
    </div>
  );
}

const HOST_KINDS = { midi: "MIDI track", audio: "Audio track", group: "Group track", return: "Return track", master: "Master", pad: "Drum Rack" } as const;

/** What a modulator moves: a parameter of another device on its track. */
function ModTargetPicker({ track, device }: { track: Owner; device: Device }) {
  const devices = useStore((s) => s.project.tracks.find((t) => t.id === track.id)?.devices ?? []);
  const setModTarget = useStore((s) => s.setModTarget);
  const targets = devices.filter((d) => d.id !== device.id && !isModulator(d.type) && !isMidiFx(d.type));
  const value = device.modulates ? `${device.modulates.device}|${device.modulates.param}` : "";
  const moved = targets.find((d) => d.id === device.modulates?.device);
  return (
    <div className="mod-side">
      <span className="mod-wave" aria-hidden="true">
        <Icon name="automation" size={22} />
      </span>
      <label className="dev-opt mod-pick tip-t" data-tip="The knob this modulator moves, around where you set it">
        <span className="knob-l">Moves</span>
        <span className="selwrap">
          <Select
            className="select sm"
            value={value}
            disabled={!device.on}
            onChange={(e) => {
              const [d, p] = e.target.value.split("|");
              setModTarget(track.id, device.id, d ? { device: d, param: p } : null);
            }}
          >
            <option value="">Nothing</option>
            {targets.map((d) => (
              <optgroup key={d.id} label={d.name}>
                {d.params
                  .filter((p) => !p.options && !p.group?.startsWith("pad:"))
                  .map((p) => (
                    <option key={p.id} value={`${d.id}|${p.id}`}>
                      {p.label}
                    </option>
                  ))}
              </optgroup>
            ))}
          </Select>
        </span>
      </label>
      <span className="muted mod-note">{moved ? `${moved.name}${moved.on ? "" : " (off)"}` : targets.length ? "Pick a knob to move" : "Add an instrument or effect first"}</span>
    </div>
  );
}

const FOLD_KEY = "soniq.devicesFolded";

/** Whether the devices panel is folded, in the Mixer and elsewhere. */
function readFolded(): { mixer: boolean; other: boolean } {
  try {
    const saved = JSON.parse(localStorage.getItem(FOLD_KEY) ?? "null") as { mixer?: unknown; other?: unknown } | null;
    return { mixer: saved?.mixer !== false, other: saved?.other === true };
  } catch {
    return { mixer: true, other: false };
  }
}

/** The selected track's devices, or a return's or the master's (picked in the mixer). */
export function DeviceChain() {
  const project = useStore((s) => s.project);
  const owner = useStore((s) => s.chainOwner ?? s.selectedTrackId);
  // A drum pad's chain whose rack has gone shows its track again.
  const host = useMemo(() => chainHost(project, owner) ?? chainHost(project, owner?.split("/")[0] ?? null), [project, owner]);
  const selectChain = useStore((s) => s.selectChain);
  // Folded to its header, the views above get the room: the Mixer starts that way, for its faders.
  const where = useStore((s) => (s.view === "mixer" ? "mixer" : "other"));
  const [folded, setFolded] = useState(() => readFolded());
  const collapsed = folded[where];
  const fold = (on: boolean) => {
    const next = { ...folded, [where]: on };
    setFolded(next);
    try {
      localStorage.setItem(FOLD_KEY, JSON.stringify(next));
    } catch {
      // Remembered for this session only.
    }
  };
  const audioTrack = host?.kind === "audio" ? project.tracks.find((t) => t.id === host.id) : undefined;
  const frozen = project.tracks.find((t) => t.id === host?.id)?.frozen !== undefined;
  const addDevice = useStore((s) => s.addDevice);
  const unfreezeTrack = useStore((s) => s.unfreezeTrack);
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const [dropping, setDropping] = useState(false);
  if (!host) return null;
  // Instruments with two rows of controls (or pads) and the EQ get a taller panel.
  const tall = host.devices.some((d) => ["synth", "sampler", "drum-rack", "rack", "eq"].includes(d.type));

  // Only MIDI tracks host instruments and MIDI effects; modulators go on tracks.
  const onTrack = host.kind === "midi" || host.kind === "audio" || host.kind === "group";
  const options = DEVICE_CATALOG.filter((d) => d.category === "audio-fx" || (d.category === "modulator" ? onTrack : host.kind === "midi"));
  const onDrop = (e: DragEvent) => {
    setDropping(false);
    const type = e.dataTransfer.getData(DEVICE_DRAG_TYPE);
    if (type) {
      addDevice(host.id, type);
      if (collapsed) fold(false);
    }
  };

  return (
    <section
      className={`detail${tall ? " tall" : ""}${dropping ? " dropping" : ""}${collapsed ? " collapsed" : ""}`}
      aria-label={`Device chain for ${host.name}`}
      onDragOver={(e) => {
        if (e.dataTransfer.types.includes(DEVICE_DRAG_TYPE)) {
          e.preventDefault();
          setDropping(true);
        }
      }}
      onDragLeave={() => setDropping(false)}
      onDrop={onDrop}
    >
      <div className="detail-head">
        <button
          className="ib xs tip-t detail-fold"
          type="button"
          aria-expanded={!collapsed}
          aria-label={collapsed ? "Show the devices" : "Hide the devices"}
          data-tip={collapsed ? "Show the devices" : "Hide the devices: more room above"}
          onClick={() => fold(!collapsed)}
        >
          <Icon name={collapsed ? "chevronUp" : "chevronDown"} size={14} />
        </button>
        {host.kind === "pad" && (
          <button className="btn sm" type="button" onClick={() => selectChain(null)}>
            <Icon name="chevronRight" size={13} className="flip" />
            Back to the Drum Rack
          </button>
        )}
        <span className="tdot" style={{ background: host.color ? trackVar(host.color) : "var(--text-3)" }} />
        <h2 className="pt">{host.name}</h2>
        <span className="muted">
          {HOST_KINDS[host.kind]} · {host.devices.length} {host.devices.length === 1 ? "device" : "devices"}
        </span>
        {audioTrack && <InputPicker track={audioTrack} />}
        {frozen && (
          <span className="frozen-note" role="status">
            <span className="tag frozen">Frozen</span>
            Its devices rest while it plays their rendered sound: changes are heard after unfreezing.
            <button className="btn sm" type="button" onClick={() => unfreezeTrack(host.id)}>
              Unfreeze
            </button>
          </span>
        )}
        <span className="hint essentials-only">
          <Icon name="info" size={14} />
          Drag a knob up or down to turn it. Double-click resets it.
        </span>
      </div>
      <div className={`chain${frozen ? " resting" : ""}`} data-chain-owner={host.id} hidden={collapsed}>
        {host.kind === "pad" && host.devices.length === 0 && (
          <p className="muted pad-chain-empty">
            {host.id.endsWith("/A") || host.id.endsWith("/B")
              ? "Every pad's send goes through these effects. Add a reverb or a delay, then turn up a pad's send."
              : "Effects here change this pad only. Add one from the browser or with Add device."}
          </p>
        )}
        {host.devices.map((d, i) => (
          <div key={d.id} className="chain-item">
            {i > 0 && <Icon name="chevronRight" size={14} className="chain-arrow" />}
            <DeviceCard track={host} device={d} />
          </div>
        ))}
        <button className="dev-add" type="button" aria-haspopup="menu" onClick={(e) => setAnchor(anchor ? null : e.currentTarget)}>
          <Icon name="plus" size={16} />
          <span>Add device</span>
          <span className="muted">or drop one here</span>
        </button>
      </div>
      {anchor && (
        <Popover anchor={anchor} onClose={() => setAnchor(null)} label="Add device" align="right">
          {options.map((d) => (
            <button
              key={d.type}
              type="button"
              role="menuitem"
              className="mi"
              onClick={() => {
                setAnchor(null);
                addDevice(host.id, d.type);
              }}
            >
              <span className="mi-check" />
              <span className="mi-label">{d.name}</span>
              <span className="mi-k">{d.role === "instrument" ? "Instrument" : d.category === "midi-fx" ? "MIDI" : d.category === "modulator" ? "Modulator" : "Audio"}</span>
            </button>
          ))}
        </Popover>
      )}
    </section>
  );
}
