import { useCallback, useEffect, useState } from "react";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { backend } from "../engine/backend";
import { inTauri } from "../engine/bridge";
import { REPO_URL, useStore } from "../state/store";
import type { CrashStatus } from "../state/types";
import { Icon } from "./Icon";

const when = (secs: number) => new Date(secs * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });

/** Updates: the version, a check, and whether to look by itself. */
export function UpdateSettings() {
  const version = useStore((s) => s.appVersion);
  const update = useStore((s) => s.update);
  const status = useStore((s) => s.updateStatus);
  const progress = useStore((s) => s.updateProgress);
  const auto = useStore((s) => s.autoUpdate);
  const { checkForUpdates, installUpdate, setAutoUpdate } = useStore.getState();
  return (
    <section className="rel-section" aria-labelledby="rel-updates">
      <div className="ap-head">
        <h3 id="rel-updates">Updates</h3>
        <span className="muted mono">{version ? `Version ${version}` : ""}</span>
      </div>
      {update ? (
        <div className="rel-update">
          <div>
            <b>Soniq Studio {update.version} is ready</b>
            {update.date && <span className="muted"> · {new Date(update.date).toLocaleDateString()}</span>}
            {update.notes && <p className="rel-notes">{update.notes}</p>}
          </div>
          <button className="btn btn-primary sm" type="button" disabled={status !== "idle"} onClick={() => void installUpdate()}>
            <Icon name="export" size={14} />
            {status === "downloading" ? `Downloading ${progress != null ? `${Math.round(progress * 100)} %` : "…"}` : status === "installing" ? "Installing…" : "Install and restart"}
          </button>
        </div>
      ) : (
        <div className="rel-row">
          <button className="btn sm" type="button" disabled={status !== "idle"} onClick={() => void checkForUpdates(true)}>
            {status === "checking" ? "Checking…" : "Check for updates"}
          </button>
          <span className="muted">New versions come from the project’s releases on GitHub.</span>
        </div>
      )}
      <div className="setting switch-row">
        <button className="switch sm" type="button" role="switch" aria-checked={auto} aria-labelledby="auto-update-label" onClick={() => setAutoUpdate(!auto)} />
        <span id="auto-update-label">Look for updates by itself, once a day (asks GitHub; nothing about you is sent)</span>
      </div>
    </section>
  );
}

/** Crash reports: whether to keep them, and the ones kept. */
export function CrashSettings({ open }: { open?: string | null }) {
  const [status, setStatus] = useState<CrashStatus | null>(null);
  const [shown, setShown] = useState<{ file: string; text: string } | null>(null);
  const { toast } = useStore.getState();
  const refresh = useCallback(() => {
    if (inTauri) void backend.crashStatus().then(setStatus).catch(() => setStatus(null));
  }, []);
  useEffect(refresh, [refresh]);

  const show = async (file: string) => {
    if (shown?.file === file) return setShown(null);
    try {
      setShown({ file, text: await backend.crashRead(file) });
    } catch (err) {
      toast({ tone: "danger", title: "Could not read the report", detail: String(err) });
    }
  };
  // Opened from "Show report": the newest one, read out.
  useEffect(() => {
    if (open === "crashes" && status?.reports[0] && !shown) void show(status.reports[0].file);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, status]);
  // Looking at them counts as having seen them.
  useEffect(() => {
    if (status?.reports.some((r) => !r.seen)) void backend.crashSeen();
  }, [status]);

  const enable = async (on: boolean) => {
    await backend.crashEnable(on);
    refresh();
  };
  const remove = async (file: string) => {
    await backend.crashDelete(file);
    if (shown?.file === file) setShown(null);
    refresh();
  };
  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      toast({ tone: "success", title: "Report copied", detail: "Paste it into an issue on GitHub, or an email." });
    } catch {
      toast({ tone: "warning", title: "Could not copy the report" });
    }
  };

  return (
    <section className="rel-section" aria-labelledby="rel-crashes">
      <div className="ap-head">
        <h3 id="rel-crashes">Crash reports</h3>
        {status && status.reports.length > 0 && (
          <button className="btn sm btn-ghost" type="button" onClick={() => void revealItemInDir(`${status.folder}`)}>
            <Icon name="folder" size={13} />
            Show the folder
          </button>
        )}
      </div>
      <div className="setting switch-row">
        <button
          className="switch sm"
          type="button"
          role="switch"
          aria-checked={!!status?.enabled}
          disabled={!status}
          aria-labelledby="crash-label"
          onClick={() => void enable(!status?.enabled)}
        />
        <span id="crash-label">Keep a report when Soniq crashes: what went wrong, the version and the system, saved on this computer. Nothing is sent, and nothing from your songs is in it.</span>
      </div>
      {!inTauri ? (
        <p className="muted">Crash reports need the desktop app.</p>
      ) : status && status.reports.length === 0 ? (
        <p className="muted">{status.enabled ? "No crashes so far." : "None kept."}</p>
      ) : (
        <ul className="rel-reports">
          {status?.reports.map((r) => (
            <li key={r.file} className={shown?.file === r.file ? "on" : undefined}>
              <div className="rel-report">
                <button className="rel-report-main" type="button" aria-expanded={shown?.file === r.file} onClick={() => void show(r.file)}>
                  <b>{when(r.time)}</b>
                  <span className="muted">{r.summary || "No message"}</span>
                </button>
                <button className="ib sm" type="button" aria-label={`Delete the report from ${when(r.time)}`} onClick={() => void remove(r.file)}>
                  <Icon name="trash" size={13} />
                </button>
              </div>
              {shown?.file === r.file && (
                <div className="rel-text">
                  <pre>{shown.text}</pre>
                  <div className="tgroup">
                    <button className="btn sm" type="button" onClick={() => void copy(shown.text)}>
                      Copy the report
                    </button>
                    <button className="btn sm btn-ghost" type="button" onClick={() => void openUrl(`${REPO_URL}/issues/new`)}>
                      Report it on GitHub
                    </button>
                  </div>
                </div>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/** Help › About Soniq Studio: the version, updates and crash reports. */
function About({ focus, onClose }: { focus: string | null; onClose: () => void }) {
  const version = useStore((s) => s.appVersion);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div className="scrim" onPointerDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog shine about-dialog" role="dialog" aria-modal="true" aria-labelledby="about-title">
        <header className="dialog-head about-head">
          <span className="mark" aria-hidden="true">
            <svg viewBox="0 0 24 24" width="20" height="20">
              <path d="M3 12h2.5l2-5 3 10 3-8 2 5 1.5-2H21" />
            </svg>
          </span>
          <div>
            <h2 id="about-title">Soniq Studio</h2>
            <p className="muted">{version ? `Version ${version} · ` : ""}an audio editor and music workstation</p>
          </div>
          <button className="ib" type="button" aria-label="Close" onClick={onClose} autoFocus>
            <Icon name="close" size={16} />
          </button>
        </header>
        <UpdateSettings />
        <CrashSettings open={focus} />
        <footer className="dialog-foot">
          <button className="btn btn-ghost sm" type="button" onClick={() => void openUrl(REPO_URL)}>
            Soniq Studio on GitHub
          </button>
          <button className="btn" type="button" onClick={onClose}>
            Close
          </button>
        </footer>
      </div>
    </div>
  );
}

export function AboutDialog() {
  const open = useStore((s) => s.aboutOpen);
  const setOpen = useStore((s) => s.setAboutOpen);
  return open ? <About focus={open} onClose={() => setOpen(null)} /> : null;
}
