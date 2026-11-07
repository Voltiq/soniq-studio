import { useEffect, useMemo, useState } from "react";
import { ACCENTS, COLOR_SETS, DEFAULT_ACCENT } from "../state/appearance";
import { COMMANDS, MENUS, shortcutOf } from "../commands";
import { engine, inTauri, type DeviceInfo, type DeviceOptions, type EngineConfig } from "../engine/bridge";
import { useStore } from "../state/store";
import type { MappingTarget, MidiPortStatus, Project } from "../state/types";
import { KEYMAP_PRESETS, eventShortcut, keyFor, presetKey, type Keymap } from "../state/keymap";
import { CrashSettings, UpdateSettings } from "./AboutDialog";
import { Icon } from "./Icon";
import { Select as Dropdown } from "./Select";

/** Logical cores on this computer. */
const CORES = Math.max(1, navigator.hardwareConcurrency || 1);
const DEFAULT = "";

function Select({ label, value, onChange, children }: { label: string; value: string; onChange: (v: string) => void; children: React.ReactNode }) {
  return (
    <label className="setting">
      <span className="lab">{label}</span>
      <span className="selwrap">
        <Dropdown className="select" value={value} onChange={(e) => onChange(e.target.value)}>
          {children}
        </Dropdown>
      </span>
    </label>
  );
}

/** MIDI keyboards and controllers: every one found is used unless switched off here. */
function MidiInputs() {
  const [ports, setPorts] = useState<MidiPortStatus[] | null>(null);
  const computerKeys = useStore((s) => s.computerKeys);
  const { toggleComputerKeys, toast } = useStore.getState();
  useEffect(() => {
    let live = true;
    const scan = () => void engine.midiInputs().then((p) => live && setPorts(p));
    scan();
    // Keyboards plugged in while the dialog is open show up too.
    const id = window.setInterval(scan, 2000);
    return () => {
      live = false;
      window.clearInterval(id);
    };
  }, []);
  const toggle = async (port: MidiPortStatus) => {
    try {
      setPorts(await engine.midiInputEnable(port.name, !port.enabled));
    } catch (err) {
      toast({ tone: "danger", title: "Could not change the MIDI input", detail: err instanceof Error ? err.message : String(err) });
    }
  };
  return (
    <div className="midi-settings">
      <span className="lab">MIDI inputs</span>
      {ports === null ? (
        <p className="muted">Looking for MIDI devices…</p>
      ) : ports.length === 0 ? (
        <p className="muted">No MIDI keyboard found. Plug one in and it appears here within a few seconds.</p>
      ) : (
        <ul className="midi-ports">
          {ports.map((p) => (
            <li key={p.name}>
              <button className="switch sm" type="button" role="switch" aria-checked={p.enabled} aria-label={`Use ${p.name}`} onClick={() => void toggle(p)} />
              <span className="devname">{p.name}</span>
              <span className={p.connected ? "tag ok" : "tag"}>{p.connected ? "Connected" : p.enabled ? "Not connected" : "Off"}</span>
            </li>
          ))}
        </ul>
      )}
      <label className="midi-keys">
        <button className="switch sm" type="button" role="switch" aria-checked={computerKeys} aria-label="Computer keyboard plays notes" onClick={toggleComputerKeys} />
        <span>
          Computer keyboard plays notes <span className="kbd">Ctrl Shift K</span>
        </span>
      </label>
      <p className="muted setting-note">Keyboards play the armed MIDI tracks, or the selected one when none is armed. Arm a MIDI track and press record to capture what you play.</p>
      <MidiMappings />
    </div>
  );
}

/** Where Soniq looks for CLAP plugins, and the files it could not load. */
function PluginSettings() {
  const list = useStore((s) => s.pluginList);
  const scanning = useStore((s) => s.scanningPlugins);
  const { loadPlugins, setPluginFolders, addPluginFolder } = useStore.getState();
  useEffect(() => {
    if (!useStore.getState().pluginList) void loadPlugins(false);
  }, [loadPlugins]);
  return (
    <div className="plugin-settings">
      <span className="lab">Plugins (CLAP and VST3)</span>
      <p className="muted setting-note">
        Each plugin runs in a sandbox of its own: if one crashes or hangs, the song plays on without it. A file that crashes while being looked at is
        skipped until you look again.
      </p>
      <ul className="plugin-folders">
        {list?.standard.map((f) => (
          <li key={f}>
            <Icon name="folder" size={14} className="dim" />
            <span className="devname">{f}</span>
            <span className="tag">Standard</span>
          </li>
        ))}
        {list?.folders.map((f) => (
          <li key={f}>
            <Icon name="folder" size={14} className="dim" />
            <span className="devname">{f}</span>
            <button className="ib xs" type="button" aria-label={`Stop looking in ${f}`} onClick={() => void setPluginFolders(list.folders.filter((x) => x !== f))}>
              <Icon name="close" size={13} />
            </button>
          </li>
        ))}
      </ul>
      <div className="plugin-actions">
        <button className="btn sm" type="button" onClick={() => void addPluginFolder()}>
          Add a folder…
        </button>
        <button className="btn sm" type="button" disabled={scanning} onClick={() => void loadPlugins(true)}>
          {scanning ? "Looking…" : "Look again"}
        </button>
        <span className="muted">{list ? `${list.plugins.length} ${list.plugins.length === 1 ? "plugin" : "plugins"} found` : "Looking…"}</span>
      </div>
      {list && list.failed.length > 0 && (
        <details className="plugin-failed">
          <summary>
            {list.failed.length} {list.failed.length === 1 ? "file" : "files"} could not be loaded
          </summary>
          <ul>
            {list.failed.map(([path, why]) => (
              <li key={path}>
                <span className="devname">{path}</span>
                <span className="muted">{why}</span>
              </li>
            ))}
          </ul>
        </details>
      )}
    </div>
  );
}

/** What a mapping moves, in words: "Bass · Soniq Synth · Cutoff". */
function describeTarget(project: Project, target: MappingTarget): string {
  const track = project.tracks.find((t) => t.id === target.track);
  if (!track) return "A deleted track";
  if (target.kind === "volume") return `${track.name} · Volume`;
  if (target.kind === "pan") return `${track.name} · Pan`;
  const devices = track.devices.flatMap((d) => [d, ...(d.chains ?? []).map((c) => c.device)]);
  const device = devices.find((d) => d.id === target.device);
  return [track.name, device?.name ?? "A removed device", device?.params.find((p) => p.id === target.param)?.label ?? target.param].join(" · ");
}

/** The song's MIDI controller mappings: what each knob, fader or button moves. */
function MidiMappings() {
  const project = useStore((s) => s.project);
  const run = useStore((s) => s.run);
  const mappings = project.mappings ?? [];
  return (
    <>
      <span className="lab">Controller mappings</span>
      {mappings.length === 0 ? (
        <p className="muted">None yet. Right-click a knob or fader and choose Learn MIDI controller, then move a control on your keyboard.</p>
      ) : (
        <ul className="midi-maps">
          {mappings.map((m) => (
            <li key={m.id}>
              <span className="map-what">{describeTarget(project, m.target)}</span>
              <span className="map-cc">
                CC {m.cc} · {m.channel === undefined ? "any channel" : `channel ${m.channel + 1}`}
              </span>
              <button className="ib xs" type="button" aria-label={`Forget CC ${m.cc}`} onClick={() => void run({ op: "removeMapping", id: m.id })}>
                <Icon name="close" size={13} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

/** The keyboard shortcuts: a set to start from, and any command's key changed by pressing a new one. */
function KeyboardSettings() {
  const keymap = useStore((s) => s.keymap);
  const setKeymap = useStore((s) => s.setKeymap);
  const [query, setQuery] = useState("");
  const [recording, setRecording] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const preset = KEYMAP_PRESETS.find((p) => p.id === keymap.preset) ?? KEYMAP_PRESETS[0];
  const commands = useMemo(() => {
    const q = query.trim().toLowerCase();
    return COMMANDS.filter((c) => c.run && c.label && (!q || `${c.menu} ${c.label} ${shortcutOf(c) ?? ""}`.toLowerCase().includes(q))).sort(
      (a, b) => MENUS.indexOf(a.menu) - MENUS.indexOf(b.menu),
    );
    // The keymap changes the keys searched.
  }, [query, keymap]);

  /** Gives `id` the key `key`; any other command with that key loses it. */
  const assign = (id: string, key: string | null) => {
    const custom = { ...keymap.custom, [id]: key };
    const next: Keymap = { ...keymap, custom };
    const moved = key ? COMMANDS.filter((c) => c.id !== id && keyFor(c.id, c.shortcut, next) === key) : [];
    for (const c of moved) custom[c.id] = null;
    const name = COMMANDS.find((c) => c.id === id)?.label;
    setNote(moved.length ? `${key} moved from ${moved.map((c) => c.label).join(", ")} to ${name}.` : null);
    setKeymap({ ...next, custom });
  };

  // While recording, the next key press (with its modifiers) is the new key; Escape cancels.
  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") return setRecording(null);
      const key = eventShortcut(e);
      if (!key) return;
      setRecording(null);
      assign(recording, key);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  });

  return (
    <div className="key-settings">
      <span className="lab">Keyboard shortcuts</span>
      <div className="seg" role="group" aria-label="Shortcut set">
        {KEYMAP_PRESETS.map((p) => (
          <button
            key={p.id}
            type="button"
            aria-pressed={keymap.preset === p.id}
            onClick={() => {
              setNote(null);
              setKeymap({ preset: p.id, custom: {} });
            }}
          >
            {p.name}
          </button>
        ))}
      </div>
      <p className="muted setting-note">
        {preset.description} Click a key to change it, then press the new one (Escape cancels).
        {Object.keys(keymap.custom).length > 0 && ` ${Object.keys(keymap.custom).length} changed by you.`}
      </p>
      <label className="field">
        <Icon name="search" size={15} className="dim" />
        <input type="search" placeholder="Search commands and keys" aria-label="Search commands and keys" value={query} onChange={(e) => setQuery(e.target.value)} />
      </label>
      {note && <p className="setting-note key-note">{note}</p>}
      <ul className="key-list">
        {commands.map((c) => {
          const key = shortcutOf(c);
          const changed = c.id in keymap.custom;
          return (
            <li key={c.id}>
              <span className="key-what">
                <span className="muted">{c.menu}</span> {c.label}
              </span>
              <button
                type="button"
                className={`key-btn${recording === c.id ? " rec" : ""}${changed ? " changed" : ""}`}
                aria-label={`Shortcut for ${c.label}: ${key ?? "none"}. Click to change it.`}
                onClick={() => setRecording(recording === c.id ? null : c.id)}
              >
                {recording === c.id ? "Press keys…" : (key ?? "—")}
              </button>
              <span className="key-actions">
                {changed && (
                  <button
                    className="ib xs"
                    type="button"
                    aria-label={`Back to ${preset.name}'s key (${presetKey(c.id, c.shortcut, keymap.preset) ?? "none"})`}
                    title={`Back to ${preset.name}'s key`}
                    onClick={() => {
                      const custom = { ...keymap.custom };
                      delete custom[c.id];
                      setKeymap({ ...keymap, custom });
                    }}
                  >
                    <Icon name="undo" size={13} />
                  </button>
                )}
                {key && (
                  <button className="ib xs" type="button" aria-label={`No shortcut for ${c.label}`} title="No shortcut" onClick={() => assign(c.id, null)}>
                    <Icon name="close" size={13} />
                  </button>
                )}
              </span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

/** The theme editor: light or dark, the accent, the track colours and the density. */
function AppearanceSettings() {
  const theme = useStore((s) => s.theme);
  const accent = useStore((s) => s.accent);
  const colorSet = useStore((s) => s.colorSet);
  const density = useStore((s) => s.density);
  const { setTheme, setAppearance } = useStore.getState();
  const custom = !ACCENTS.some(([value]) => value === accent.toLowerCase());
  const changed = accent.toLowerCase() !== DEFAULT_ACCENT || colorSet !== "graphiq" || density !== "comfortable";
  return (
    <section className="appearance" aria-labelledby="appearance-title">
      <div className="ap-head">
        <h3 id="appearance-title">Appearance</h3>
        <button className="btn sm btn-ghost" type="button" disabled={!changed} onClick={() => setAppearance({ accent: DEFAULT_ACCENT, colorSet: "graphiq", density: "comfortable" })}>
          Back to Graphiq
        </button>
      </div>
      <div className="settings-grid">
        <div className="setting">
          <span className="lab">Theme</span>
          <div className="seg" role="group" aria-label="Theme">
            <button type="button" aria-pressed={theme === "dark"} onClick={() => setTheme("dark")}>
              <Icon name="moon" size={13} />
              Graphite
            </button>
            <button type="button" aria-pressed={theme === "light"} onClick={() => setTheme("light")}>
              <Icon name="sun" size={13} />
              Porcelain
            </button>
          </div>
        </div>
        <div className="setting">
          <span className="lab">Density</span>
          <div className="seg" role="group" aria-label="Density">
            <button type="button" aria-pressed={density === "comfortable"} onClick={() => setAppearance({ density: "comfortable" })}>
              Comfortable
            </button>
            <button type="button" aria-pressed={density === "compact"} onClick={() => setAppearance({ density: "compact" })}>
              Compact
            </button>
          </div>
        </div>
        <div className="setting wide">
          <span className="lab">Accent</span>
          <div className="swatches" role="radiogroup" aria-label="Accent colour">
            {ACCENTS.map(([value, name]) => (
              <button
                key={value}
                type="button"
                role="radio"
                aria-checked={accent.toLowerCase() === value}
                aria-label={name}
                title={name}
                className="swatch"
                style={{ background: value }}
                onClick={() => setAppearance({ accent: value })}
              />
            ))}
            <label className={custom ? "swatch custom on" : "swatch custom"} title="Your own colour" style={custom ? { background: accent } : undefined}>
              <input type="color" aria-label="Your own accent colour" value={accent} onChange={(e) => setAppearance({ accent: e.target.value })} />
              {!custom && <Icon name="plus" size={13} />}
            </label>
          </div>
        </div>
        <div className="setting wide">
          <span className="lab">Track colours</span>
          <div className="color-sets" role="radiogroup" aria-label="Track colours">
            {COLOR_SETS.map((set) => (
              <button key={set.id} type="button" role="radio" aria-checked={colorSet === set.id} className="color-set" onClick={() => setAppearance({ colorSet: set.id })}>
                <span className="color-row" aria-hidden="true">
                  {set.colors.map((c) => (
                    <span key={c} style={{ background: c }} />
                  ))}
                </span>
                {set.name}
              </button>
            ))}
          </div>
        </div>
      </div>
    </section>
  );
}

function Settings({ onClose }: { onClose: () => void }) {
  const status = useStore((s) => s.engineStatus);
  const { setEngineStatus, toast } = useStore.getState();
  const [devices, setDevices] = useState<DeviceInfo[] | null>(null);
  const input = useStore((s) => s.inputStatus);
  const [config, setConfig] = useState<EngineConfig>({ host: null, output: null, input: null, sampleRate: null, bufferFrames: null, recordOffsetMs: 0 });
  const [options, setOptions] = useState<DeviceOptions | null>(null);
  const [applying, setApplying] = useState(false);

  useEffect(() => {
    let live = true;
    void engine.devices().then((d) => live && setDevices(d));
    void engine.config().then((c) => live && c && setConfig(c));
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => {
      live = false;
      window.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  // What the chosen output supports.
  useEffect(() => {
    if (!inTauri) return;
    let live = true;
    engine
      .deviceOptions(config.host, config.output)
      .then((o) => live && setOptions(o))
      .catch(() => live && setOptions(null));
    return () => {
      live = false;
    };
  }, [config.host, config.output]);

  const hosts = useMemo(() => [...new Set((devices ?? []).map((d) => d.host))], [devices]);
  const host = config.host ?? status?.host ?? hosts[0] ?? "";
  const outputs = (devices ?? []).filter((d) => d.direction === "output" && d.host === host);
  const inputs = (devices ?? []).filter((d) => d.direction === "input" && d.host === host);
  const latency = status?.running && status.bufferFrames ? `${((status.bufferFrames / status.sampleRate) * 1000).toFixed(1)} ms` : null;

  const apply = async () => {
    setApplying(true);
    try {
      const next = await engine.configure(config);
      setEngineStatus(next);
      toast({ tone: "success", title: "Audio settings applied", detail: `${next.device} · ${next.sampleRate / 1000} kHz` });
    } catch (err) {
      toast({ tone: "danger", title: "Could not use those settings", detail: err instanceof Error ? err.message : String(err) });
    } finally {
      setApplying(false);
    }
  };

  return (
    <div className="scrim" onPointerDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog shine" role="dialog" aria-modal="true" aria-labelledby="settings-title">
        <header className="dialog-head">
          <h2 id="settings-title">Settings</h2>
          <button className="ib" type="button" aria-label="Close" onClick={onClose} autoFocus>
            <Icon name="close" size={16} />
          </button>
        </header>

        <AppearanceSettings />

        {!inTauri ? (
          <div className="empty">
            <Icon name="headphones" size={22} />
            <p>
              <b>No audio engine in the browser preview</b>
              Run <code>npm run tauri dev</code> to start the desktop app with the Rust engine.
            </p>
          </div>
        ) : (
          <>
            <div className="status-line">
              <span className={status?.running ? "dot ok" : "dot bad"} aria-hidden="true" />
              <span>
                {status?.running ? (
                  <>
                    <b>{status.device}</b> · {status.host} · {status.sampleRate / 1000} kHz · {status.channels} ch
                    {latency && ` · ${status.bufferFrames} samples (${latency})`}
                    {status.cores && status.cores > 1 ? ` · mixing on ${status.cores} cores` : ""}
                  </>
                ) : (
                  (status?.error ?? "The audio engine is not running.")
                )}
              </span>
            </div>

            <div className="settings-grid">
              <Select
                label="Driver"
                value={config.host ?? DEFAULT}
                onChange={(v) => setConfig({ ...config, host: v || null, output: null, input: null, sampleRate: null, bufferFrames: null })}
              >
                <option value={DEFAULT}>System default</option>
                {hosts.map((h) => (
                  <option key={h}>{h}</option>
                ))}
              </Select>
              <Select label="Output device" value={config.output ?? DEFAULT} onChange={(v) => setConfig({ ...config, output: v || null, sampleRate: null, bufferFrames: null })}>
                <option value={DEFAULT}>Default output</option>
                {outputs.map((d) => (
                  <option key={d.name} value={d.name}>
                    {d.name}
                    {d.isDefault ? " (default)" : ""}
                  </option>
                ))}
              </Select>
              <Select label="Sample rate" value={String(config.sampleRate ?? DEFAULT)} onChange={(v) => setConfig({ ...config, sampleRate: v ? Number(v) : null })}>
                <option value={DEFAULT}>Device default{options ? ` (${options.defaultSampleRate / 1000} kHz)` : ""}</option>
                {options?.sampleRates.map((r) => (
                  <option key={r} value={r}>
                    {r / 1000} kHz
                  </option>
                ))}
              </Select>
              <Select label="Buffer size" value={String(config.bufferFrames ?? DEFAULT)} onChange={(v) => setConfig({ ...config, bufferFrames: v ? Number(v) : null })}>
                <option value={DEFAULT}>Driver default</option>
                {options?.bufferSizes.map((b) => {
                  const rate = config.sampleRate ?? options.defaultSampleRate;
                  return (
                    <option key={b} value={b}>
                      {b} samples · {((b / rate) * 1000).toFixed(1)} ms
                    </option>
                  );
                })}
              </Select>
              <Select label="Cores for the mix" value={String(config.cores ?? DEFAULT)} onChange={(v) => setConfig({ ...config, cores: v ? Number(v) : null })}>
                <option value={DEFAULT}>Automatic · {Math.min(Math.max(CORES - 1, 1), 8)} of {CORES}</option>
                {Array.from({ length: CORES }, (_, i) => i + 1).map((c) => (
                  <option key={c} value={c}>
                    {c === 1 ? "1 · the audio thread alone" : `${c} cores`}
                  </option>
                ))}
              </Select>
            </div>
            <p className="muted">Tracks are shared out over the cores while they play; more cores help songs with many tracks and heavy plugins.</p>
            <div className="setting switch-row">
              <button
                className="switch sm"
                type="button"
                role="switch"
                aria-checked={!config.safetyLimiterOff}
                aria-labelledby="safety-label"
                onClick={() => {
                  const on = !!config.safetyLimiterOff;
                  setConfig({ ...config, safetyLimiterOff: !on });
                  void engine.safety(on);
                }}
              />
              <span id="safety-label">
                Safety limiter: nothing reaches your speakers or headphones above −0.3 dBFS, whatever a plugin or a feedback loop does. Exports are not
                touched. The <b>lim</b> light on the master meter shows when it works.
              </span>
            </div>
            <p className="muted">If the output is unplugged, Soniq stops and carries on with the system default; it goes back to your choice when it returns.</p>
            {options && options.bufferSizes.length === 0 && (
              <p className="muted">This driver picks its own buffer size. ASIO and WASAPI exclusive mode for lower latency are planned.</p>
            )}

            <div className="settings-grid">
              <Select label="Input device (recording)" value={config.input ?? DEFAULT} onChange={(v) => setConfig({ ...config, input: v || null })}>
                <option value={DEFAULT}>Default input</option>
                {inputs.map((d) => (
                  <option key={d.name} value={d.name}>
                    {d.name}
                    {d.channels ? ` · ${d.channels} ch` : ""}
                    {d.isDefault ? " (default)" : ""}
                  </option>
                ))}
              </Select>
              <p className="muted setting-note">
                The input opens only while a track is armed or monitoring. Pick which channels each track records, and whether you hear them, in its device
                chain header.
              </p>
              <label className="setting">
                <span className="lab">Recording offset</span>
                <span className="field mono">
                  <input
                    type="number"
                    step={0.5}
                    min={-200}
                    max={200}
                    aria-label="Recording offset in milliseconds"
                    value={config.recordOffsetMs}
                    onChange={(e) => setConfig({ ...config, recordOffsetMs: Number(e.target.value) || 0 })}
                  />
                  <span className="fix">ms</span>
                </span>
              </label>
              <p className="muted setting-note">
                Takes are moved earlier by the measured round trip
                {status?.running ? ` (${(status.outputLatencyMs + (input?.open ? input.latencyMs : 0)).toFixed(1)} ms now)` : ""} so they line up
                with what you heard. If they still land late, add the difference here.
              </p>
            </div>
            <MidiInputs />
            <PluginSettings />
          </>
        )}
        <KeyboardSettings />
        <UpdateSettings />
        <CrashSettings />
        <footer className="dialog-foot">
          <span className="muted">Settings are remembered for next time.</span>
          <div className="tgroup">
            <button className="btn btn-ghost" type="button" onClick={onClose}>
              Close
            </button>
            {inTauri && (
              <button className="btn btn-primary" type="button" disabled={applying} onClick={() => void apply()}>
                {applying ? "Applying…" : "Apply"}
              </button>
            )}
          </div>
        </footer>
      </div>
    </div>
  );
}

export function SettingsDialog() {
  const open = useStore((s) => s.settingsOpen);
  const setOpen = useStore((s) => s.setSettingsOpen);
  return open ? <Settings onClose={() => setOpen(false)} /> : null;
}
