import { useEffect, useMemo, useState } from "react";
import { inTauri } from "../engine/bridge";
import { DEVICE_CATALOG, LIVE_TYPES, chainHost, type DeviceSpec } from "../state/devices";
import { useStore } from "../state/store";
import { Icon, type IconName } from "./Icon";
import { trackVar } from "./trackColor";

type Category = "all" | DeviceSpec["category"] | "samples" | "plugins";

const CATEGORIES: [Category, string][] = [
  ["all", "All"],
  ["instrument", "Instruments"],
  ["audio-fx", "Audio FX"],
  ["midi-fx", "MIDI FX"],
  ["modulator", "Modulators"],
  ["samples", "Samples"],
  ["plugins", "Plugins"],
];

const ICONS: Record<DeviceSpec["category"], IconName> = { instrument: "piano", "audio-fx": "fx", "midi-fx": "keyboard", modulator: "automation" };

export const DEVICE_DRAG_TYPE = "application/x-soniq-device";

/** The CLAP plugins found on disk; a click adds one to the chain on show. */
function PluginItems({ query, owner }: { query: string; owner: string | undefined }) {
  const list = useStore((s) => s.pluginList);
  const scanning = useStore((s) => s.scanningPlugins);
  const { loadPlugins, addPlugin, addPluginFolder } = useStore.getState();
  useEffect(() => {
    if (inTauri && !list) void loadPlugins(false);
  }, [list, loadPlugins]);

  if (!inTauri) {
    return (
      <div className="empty">
        <Icon name="plug" size={22} />
        <p>
          <b>Plugins run in the desktop app</b>
          Each CLAP or VST3 plugin gets a sandbox of its own there. Run <code>npm run tauri dev</code>.
        </p>
      </div>
    );
  }
  if (!list) {
    return (
      <div className="empty">
        <Icon name="plug" size={22} />
        <p>
          <b>Looking for plugins…</b>
          Each new plugin file is opened once in a sandbox to see what it holds.
        </p>
      </div>
    );
  }
  const q = query.trim().toLowerCase();
  const items = list.plugins.filter((p) => !q || `${p.name} ${p.vendor} ${p.description}`.toLowerCase().includes(q));
  // A span, so it can sit inside the empty state's paragraph too.
  const actions = (
    <span className="plugin-actions">
      <button className="btn xs" type="button" disabled={scanning} onClick={() => void loadPlugins(true)}>
        {scanning ? "Looking…" : "Look again"}
      </button>
      <button className="btn xs" type="button" onClick={() => void addPluginFolder()}>
        Add a folder…
      </button>
    </span>
  );
  if (!list.plugins.length) {
    return (
      <div className="empty">
        <Icon name="plug" size={22} />
        <p>
          <b>No plugins found</b>
          Soniq looks in {list.standard.length ? list.standard.join(", ") : "the standard CLAP folders"}
          {list.folders.length ? ` and ${list.folders.join(", ")}` : ""}.
          {list.failed.length > 0 && ` ${list.failed.length} could not be loaded.`}
          <span>{actions}</span>
        </p>
      </div>
    );
  }
  return (
    <>
      {items.map((p) => (
        <div key={`${p.path}|${p.id}`} role="listitem">
          <button
            className="bitem"
            type="button"
            disabled={scanning}
            onClick={() => owner && void addPlugin(owner, p)}
            title={`${p.description || p.name} · ${p.path}${p.version ? ` · version ${p.version}` : ""}. Click to add; it runs in a sandbox of its own.`}
          >
            <span className={`bicon ${p.instrument ? "instrument" : "audio-fx"}`}>
              <Icon name={p.instrument ? "piano" : "plug"} size={16} />
            </span>
            <span className="btext">
              <span className="bname">{p.name}</span>
              <span className="bdesc">
                {p.vendor ? `${p.vendor} · ` : ""}
                {p.format === "vst3" ? "VST3" : "CLAP"} {p.instrument ? "instrument" : "effect"}
              </span>
            </span>
          </button>
        </div>
      ))}
      {items.length === 0 && (
        <div className="empty">
          <p>
            <b>No plugin matches “{query}”</b>
          </p>
        </div>
      )}
      {actions}
    </>
  );
}

export function BrowserPanel() {
  const [query, setQuery] = useState("");
  const [cat, setCat] = useState<Category>("all");
  // Clicks add to the chain on show: the selected track, or a return or the master picked in the mixer.
  const project = useStore((s) => s.project);
  const owner = useStore((s) => s.chainOwner ?? s.selectedTrackId);
  const track = useMemo(() => chainHost(project, owner), [project, owner]);
  const addDevice = useStore((s) => s.addDevice);
  const importAudio = useStore((s) => s.importAudio);
  const pluginCount = useStore((s) => s.pluginList?.plugins.length);

  const items = useMemo(() => {
    const q = query.trim().toLowerCase();
    return DEVICE_CATALOG.filter((d) => (cat === "all" || d.category === cat) && (!q || `${d.name} ${d.description}`.toLowerCase().includes(q)));
  }, [query, cat]);

  return (
    <aside className="browser card shine" aria-label="Browser">
      <div className="ph">
        <h2 className="pt">Browser</h2>
        <span className="muted">{cat === "plugins" ? `${pluginCount ?? 0} plugins` : `${DEVICE_CATALOG.length} devices`}</span>
      </div>
      <label className="field">
        <Icon name="search" size={15} className="dim" />
        <input
          type="search"
          placeholder={cat === "plugins" ? "Search plugins" : "Search devices"}
          aria-label="Search the browser"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </label>
      <div className="chips" role="group" aria-label="Categories">
        {CATEGORIES.map(([id, label]) => (
          <button key={id} className={`chipb${id === "plugins" ? " pro-only" : ""}`} type="button" aria-pressed={cat === id} onClick={() => setCat(id)}>
            {label}
          </button>
        ))}
      </div>

      <div className="blist" role="list">
        {cat === "samples" ? (
          <div className="empty">
            <Icon name="folder" size={22} />
            <p>
              <b>Your sample folders go here</b>
              Folder indexing and previews in time with the song come next. For now, import files or drag them in from Explorer.
              <span>
                <button className="btn sm" type="button" onClick={() => void importAudio()}>
                  Import audio…
                </button>
              </span>
            </p>
          </div>
        ) : cat === "plugins" ? (
          <PluginItems query={query} owner={track?.id} />
        ) : items.length === 0 ? (
          <div className="empty">
            <p>
              <b>Nothing matches “{query}”</b>
              Try another word or category.
            </p>
          </div>
        ) : (
          items.map((d) => (
            <div key={d.type} role="listitem">
              <button
                className="bitem"
                type="button"
                draggable
                onDragStart={(e) => {
                  e.dataTransfer.setData(DEVICE_DRAG_TYPE, d.type);
                  e.dataTransfer.effectAllowed = "copy";
                }}
                onClick={() => track && addDevice(track.id, d.type)}
                title={
                  LIVE_TYPES.has(d.type)
                    ? "Click to add, or drag onto a device chain."
                    : `Click to add, or drag onto a device chain. Sound processing arrives in phase ${d.phase}.`
                }
              >
                <span className={`bicon ${d.category}`}>
                  <Icon name={ICONS[d.category]} size={16} />
                </span>
                <span className="btext">
                  <span className="bname">{d.name}</span>
                  <span className="bdesc">{d.description}</span>
                </span>
              </button>
            </div>
          ))
        )}
      </div>

      {track && (
        <div className="bfoot">
          <span className="tdot" style={{ background: track.color ? trackVar(track.color) : "var(--text-3)" }} />
          <span>
            Click adds to <b>{track.name}</b>
          </span>
        </div>
      )}
    </aside>
  );
}
