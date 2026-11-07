import { useEffect, useMemo, useState } from "react";
import { backend } from "../engine/backend";
import { useStore } from "../state/store";
import type { Device, DeviceParam, PluginParamInfo } from "../state/types";
import { ParamGroups, type Owner } from "./DeviceControls";
import { Icon } from "./Icon";
import { Popover } from "./Menu";

/** A plugin parameter as a device control (mirrors `plugins::control` in Rust). */
function control(p: PluginParamInfo): DeviceParam {
  return {
    id: String(p.id),
    label: p.name,
    value: p.value,
    min: p.min,
    max: Math.max(p.min, p.max),
    default: p.default,
    unit: "",
    // VST3 values run 0–1, so a stepped one moves by 1/steps.
    ...(p.stepped ? { step: p.steps > 0 ? (p.max - p.min) / p.steps : 1 } : {}),
    ...(p.module ? { group: p.module } : {}),
  };
}

/** What the plugin writes for each shown control's value, kept fresh while the panel shows. */
function usePluginTexts(device: Device, running: boolean): Record<string, string> {
  const [texts, setTexts] = useState<Record<string, string>>({});
  const ids = device.params.map((p) => Number(p.id)).filter((id) => Number.isFinite(id));
  const key = `${device.id}|${ids.join(",")}|${device.params.map((p) => p.value).join(",")}`;
  useEffect(() => {
    if (!running || !ids.length) return;
    let live = true;
    const fetch = () =>
      void backend
        .pluginTexts(device.id, ids)
        .then((t) => live && setTexts(Object.fromEntries(t.map(([id, , text]) => [String(id), text]))))
        .catch(() => {});
    fetch();
    // The plugin's own window can move a control too.
    const timer = window.setInterval(fetch, 500);
    return () => {
      live = false;
      window.clearInterval(timer);
    };
  }, [key, running]); // eslint-disable-line react-hooks/exhaustive-deps
  return texts;
}

/** Which of the plugin's parameters the device shows as controls (and automation can reach). */
function ControlsPicker({ track, device, onClose, anchor }: { track: Owner; device: Device; onClose: () => void; anchor: HTMLElement }) {
  const [params, setParams] = useState<PluginParamInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const setPluginControls = useStore((s) => s.setPluginControls);
  useEffect(() => {
    backend
      .pluginParams(device.id)
      .then(setParams)
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }, [device.id]);
  const shown = new Set(device.params.map((p) => p.id));
  const toggle = (p: PluginParamInfo) => {
    if (!params) return;
    const id = String(p.id);
    const next = shown.has(id) ? [...shown].filter((s) => s !== id) : [...shown, id];
    // Plugin order, keeping each shown control as it is.
    const list = params.filter((q) => next.includes(String(q.id))).map((q) => device.params.find((d) => d.id === String(q.id)) ?? control(q));
    setPluginControls(track.id, device.id, list);
  };
  const q = query.trim().toLowerCase();
  const visible = (params ?? []).filter((p) => !p.hidden && (!q || `${p.name} ${p.module}`.toLowerCase().includes(q)));
  return (
    <Popover anchor={anchor} onClose={onClose} label="Plugin controls">
      <div className="plugin-picker">
        <p className="mi-menu">Controls on the panel can be automated and mapped to MIDI. Hiding one removes its automation.</p>
        <label className="field">
          <Icon name="search" size={14} className="dim" />
          <input type="search" placeholder="Search parameters" aria-label="Search the plugin's parameters" value={query} onChange={(e) => setQuery(e.target.value)} autoFocus />
        </label>
        <div className="plugin-params" role="group" aria-label="Parameters">
          {error ? (
            <p className="muted">{error}</p>
          ) : !params ? (
            <p className="muted">Asking the plugin…</p>
          ) : visible.length === 0 ? (
            <p className="muted">No parameter matches.</p>
          ) : (
            visible.map((p) => (
              <label key={p.id} className={`plugin-param${p.readonly ? " ro" : ""}`}>
                <input type="checkbox" checked={shown.has(String(p.id))} disabled={p.readonly} onChange={() => toggle(p)} />
                <span className="pp-name">{p.name}</span>
                <span className="pp-module muted">{p.module}</span>
                <span className="pp-text mono">{p.text}</span>
              </label>
            ))
          )}
        </div>
      </div>
    </Popover>
  );
}

/** A plugin device: how its sandbox is doing, its window, and the controls it shows. */
export function PluginBody({ track, device }: { track: Owner; device: Device }) {
  const status = useStore((s) => s.pluginStatus[device.id]);
  const rate = useStore((s) => s.engineStatus?.sampleRate ?? 48_000);
  const { pluginWindow, reloadPlugin } = useStore.getState();
  const [picker, setPicker] = useState<HTMLElement | null>(null);
  const running = status?.state === "running";
  const texts = usePluginTexts(device, running);
  const params = useMemo(() => device.params, [device.params]);

  return (
    <div className="dev-row plugin-row">
      <div className="plugin-side">
        <span className="plugin-vendor muted" title={device.plugin?.path}>
          {device.plugin?.vendor || `${device.plugin?.format === "vst3" ? "VST3" : "CLAP"} plugin`}
        </span>
        {!status || status.state === "starting" ? (
          <span className="plugin-state">
            <span className="dot warn" aria-hidden="true" /> Starting in its sandbox…
          </span>
        ) : running ? (
          <>
            {status.gui && (
              <button className="btn sm" type="button" aria-pressed={status.window} disabled={!device.on} onClick={() => void pluginWindow(device.id, !status.window)}>
                <Icon name="sliders" size={14} />
                {status.window ? "Close window" : "Open window"}
              </button>
            )}
            <button className="btn sm" type="button" aria-haspopup="dialog" onClick={(e) => setPicker(picker ? null : e.currentTarget)}>
              Controls…
            </button>
            {status.latency > 0 && <span className="muted plugin-note">Adds {((status.latency / rate) * 1000).toFixed(1)} ms of latency</span>}
          </>
        ) : (
          <>
            <span className="plugin-state bad">
              <span className="dot bad" aria-hidden="true" /> {status.state === "crashed" ? "Crashed" : "Could not start"}
            </span>
            <span className="muted plugin-note">{status.message}</span>
            <button className="btn sm" type="button" onClick={() => void reloadPlugin(device.id)}>
              Start again
            </button>
          </>
        )}
      </div>
      {params.length ? (
        <ParamGroups track={track} device={device} params={params} texts={running ? texts : undefined} />
      ) : (
        <p className="muted plugin-empty">No controls on the panel. Pick some with Controls… to automate them or map them to MIDI.</p>
      )}
      {picker && <ControlsPicker track={track} device={device} anchor={picker} onClose={() => setPicker(null)} />}
    </div>
  );
}
