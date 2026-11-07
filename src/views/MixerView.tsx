import { useState, type CSSProperties } from "react";
import { Fader } from "../components/controls/Fader";
import { Icon } from "../components/Icon";
import { Popover } from "../components/Menu";
import { Knob } from "../components/controls/Knob";
import { Meter } from "../components/controls/Meter";
import { Learnable } from "../components/Learnable";
import { trackVar } from "../components/trackColor";
import { formatDb, formatPan } from "../state/format";
import { useStore } from "../state/store";
import { MASTER, type Device, type Mix } from "../state/types";

interface StripProps {
  id: string;
  name: string;
  color?: number;
  volume: number;
  pan: number;
  mute: boolean;
  solo?: boolean;
  arm?: boolean;
  sends?: [number, number];
  devices: Device[];
  kind: "track" | "return" | "master";
  selected?: boolean;
  onSelect?: () => void;
  onVolume: (db: number) => void;
  onPan?: (pan: number) => void;
  onMute?: () => void;
  onSolo?: () => void;
  onArm?: () => void;
  onSend?: (index: 0 | 1, value: number) => void;
  /** Returns: whether tracks send before their fader. */
  preFader?: boolean;
  onPreFader?: () => void;
  /** A group track: its tracks play through it. */
  group?: boolean;
  /** Where a track in a group plays: the group's name (the master otherwise). */
  output?: string;
}

function Strip(p: StripProps) {
  const color = p.color ? trackVar(p.color) : "var(--text-3)";
  return (
    <section
      className={`strip shine ${p.kind}${p.selected ? " on" : ""}${p.group ? " group" : ""}${p.output ? " grouped" : ""}`}
      aria-label={`${p.name} channel`}
      style={{ "--c": color } as CSSProperties}
    >
      <button className="strip-name" type="button" onClick={p.onSelect} disabled={!p.onSelect}>
        <span className="tdot" />
        <span>{p.name}</span>
      </button>
      {/* Where a track plays; every track strip has the line, so their controls line up. */}
      {p.kind === "track" && <span className="strip-out">{p.group ? "Group → Master" : `→ ${p.output ?? "Master"}`}</span>}

      <div className="inserts" aria-label="Inserts">
        {p.devices.slice(0, 4).map((d) => (
          <span key={d.id} className={d.on ? "ins" : "ins off"}>
            <span className="ins-led" aria-hidden="true" />
            {d.name}
          </span>
        ))}
        {p.devices.length > 4 && <span className="ins more">+{p.devices.length - 4} more</span>}
        {p.devices.length === 0 && <span className="ins empty">No inserts</span>}
      </div>

      {p.onPreFader && (
        <button
          className="chipb pre-post tip-t"
          type="button"
          aria-pressed={!!p.preFader}
          data-tip={p.preFader ? "Tracks send here before their fader: it hears them whatever the fader does" : "Tracks send here after their fader"}
          onClick={p.onPreFader}
        >
          {p.preFader ? "Pre" : "Post"}
        </button>
      )}

      {p.sends && p.onSend && (
        <div className="sends pro-only">
          {(["A", "B"] as const).map((label, i) => (
            <Knob
              key={label}
              label={`Send ${label}`}
              size={30}
              value={p.sends![i]}
              min={0}
              max={1}
              defaultValue={0}
              format={(v) => (v <= 0.001 ? "−∞" : `${formatDb(20 * Math.log10(v))}`)}
              onChange={(v) => p.onSend!(i as 0 | 1, v)}
            />
          ))}
        </div>
      )}

      {p.onPan ? (
        <Learnable on={p.kind === "track"} target={{ kind: "pan", track: p.id }} label={`${p.name} pan`}>
          <Knob label="Pan" size={34} value={p.pan} min={-1} max={1} defaultValue={0} bipolar format={formatPan} onChange={p.onPan} />
        </Learnable>
      ) : (
        <div className="pan-spacer" />
      )}

      <div className="fader-row">
        <Learnable on={p.kind === "track"} target={{ kind: "volume", track: p.id }} label={`${p.name} volume`}>
          <Fader label={`${p.name} volume`} value={p.volume} onChange={p.onVolume} height={196} />
        </Learnable>
        <Meter id={p.id} label={p.name} width={p.kind === "master" ? 10 : 7} showClip={p.kind === "master"} />
      </div>
      <output className="strip-db">{formatDb(p.volume)} dB</output>

      <div className="tbtns">
        {p.onMute && (
          <button className="tg mute" type="button" aria-pressed={p.mute} aria-label={`Mute ${p.name}`} onClick={p.onMute}>
            M
          </button>
        )}
        {p.onSolo && (
          <button className="tg solo" type="button" aria-pressed={!!p.solo} aria-label={`Solo ${p.name}`} onClick={p.onSolo}>
            S
          </button>
        )}
        {p.onArm && (
          <button className="tg arm" type="button" aria-pressed={!!p.arm} aria-label={`Arm ${p.name}`} onClick={p.onArm}>
            <span className="rec-dot" aria-hidden="true" />
          </button>
        )}
      </div>
    </section>
  );
}

/** One empty list, so the store selector below returns the same value each time. */
const NO_MIXES: Mix[] = [];

/** Saved mixes, and comparing two mixes (A/B), above the strips. */
function MixBar() {
  const mixes = useStore((s) => s.project.mixes ?? NO_MIXES);
  const other = useStore((s) => s.project.otherMix ?? null);
  const run = useStore((s) => s.run);
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const playing = other ? (other.name === "A" ? "B" : "A") : null;
  const rename = (id: string, name: string) => {
    setRenaming(null);
    const mix = mixes.find((m) => m.id === id);
    if (mix && name.trim() && name.trim() !== mix.name) void run({ op: "renameMix", id, name });
  };

  return (
    <div className="mix-bar" role="toolbar" aria-label="Mixes">
      {other ? (
        <>
          <div className="seg" role="group" aria-label="Compare two mixes">
            {(["A", "B"] as const).map((name) => (
              <button key={name} type="button" aria-pressed={playing === name} onClick={() => playing !== name && void run({ op: "swapMix" })}>
                Mix {name}
              </button>
            ))}
          </div>
          <span className="muted mix-hint">
            Changes go to {playing}. Click {other.name} to hear it instead.
          </span>
          <button className="btn sm" type="button" onClick={() => void run({ op: "keepMix" })}>
            Keep {playing}
          </button>
        </>
      ) : (
        <button className="btn sm" type="button" onClick={() => void run({ op: "swapMix" })} title="Keep the mix as A, then try changes as B and switch between them">
          Compare A/B
        </button>
      )}
      <span className="mix-gap" />
      <button className="btn sm" type="button" aria-haspopup="menu" aria-expanded={!!anchor} onClick={(e) => setAnchor(anchor ? null : e.currentTarget)}>
        Saved mixes{mixes.length ? ` · ${mixes.length}` : ""}
        <Icon name="chevronDown" size={13} />
      </button>
      <button className="btn sm" type="button" onClick={() => void run({ op: "saveMix" })} title="Keep the faders, pans, mutes, solos and sends as they are">
        <Icon name="plus" size={14} />
        Save mix
      </button>
      {anchor && (
        <Popover anchor={anchor} onClose={() => setAnchor(null)} label="Saved mixes" align="right">
          {mixes.length === 0 && <p className="muted mix-empty">No saved mixes yet. Save mix keeps the faders, pans, mutes, solos and sends as they are, to come back to.</p>}
          {mixes.map((m) => (
            <div key={m.id} className="mix-row">
              {renaming === m.id ? (
                <input
                  className="mix-name"
                  aria-label="Mix name"
                  defaultValue={m.name}
                  autoFocus
                  onBlur={(e) => rename(m.id, e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") rename(m.id, e.currentTarget.value);
                    if (e.key === "Escape") setRenaming(null);
                  }}
                />
              ) : (
                <button
                  type="button"
                  role="menuitem"
                  className="mi"
                  title="Set the mixer as this mix has it"
                  onClick={() => {
                    setAnchor(null);
                    void run({ op: "recallMix", id: m.id });
                  }}
                >
                  <span className="mi-label">{m.name}</span>
                </button>
              )}
              <button className="ib xs" type="button" aria-label={`Save the mixer into ${m.name}`} title="Save the mixer as it is into this mix" onClick={() => void run({ op: "updateMix", id: m.id })}>
                <Icon name="upload" size={13} />
              </button>
              <button className="ib xs" type="button" aria-label={`Rename ${m.name}`} title="Rename" onClick={() => setRenaming(m.id)}>
                <Icon name="draw" size={13} />
              </button>
              <button className="ib xs" type="button" aria-label={`Delete ${m.name}`} title="Delete" onClick={() => void run({ op: "removeMix", id: m.id })}>
                <Icon name="trash" size={13} />
              </button>
            </div>
          ))}
        </Popover>
      )}
    </div>
  );
}

export function MixerView() {
  const tracks = useStore((s) => s.project.tracks);
  const returns = useStore((s) => s.project.returns);
  const master = useStore((s) => s.project.masterVolume);
  const masterDevices = useStore((s) => s.project.masterDevices);
  const selectedTrackId = useStore((s) => s.selectedTrackId);
  const chainOwner = useStore((s) => s.chainOwner);
  const { updateTrack, updateReturn, setMasterVolume, selectTrack, selectChain } = useStore.getState();

  return (
    <div className="mixer-view">
      <MixBar />
      <div className="mixer">
        <div className="strips">
          {tracks.map((t) => (
            <Strip
              key={t.id}
              id={t.id}
              name={t.name}
              color={t.color}
              kind="track"
              volume={t.volume}
              pan={t.pan}
              mute={t.mute}
              solo={t.solo}
              arm={t.arm}
              sends={t.sends}
              devices={t.devices}
              group={t.kind === "group"}
              output={t.group ? tracks.find((g) => g.id === t.group)?.name : undefined}
              selected={chainOwner === null && t.id === selectedTrackId}
              onSelect={() => selectTrack(t.id)}
              onVolume={(volume) => updateTrack(t.id, { volume })}
              onPan={(pan) => updateTrack(t.id, { pan })}
              onMute={() => updateTrack(t.id, { mute: !t.mute })}
              onSolo={() => updateTrack(t.id, { solo: !t.solo })}
              onArm={t.kind === "group" ? undefined : () => updateTrack(t.id, { arm: !t.arm })}
              onSend={(i, v) => updateTrack(t.id, { sends: (i === 0 ? [v, t.sends[1]] : [t.sends[0], v]) as [number, number] })}
            />
          ))}
          <span className="strip-gap pro-only" aria-hidden="true" />
          {returns.map((r) => (
            <div key={r.id} className="strip-wrap pro-only">
              <Strip
                id={r.id}
                name={r.name}
                kind="return"
                volume={r.volume}
                pan={r.pan}
                mute={r.mute}
                devices={r.devices}
                selected={chainOwner === r.id}
                onSelect={() => selectChain(r.id)}
                onVolume={(volume) => updateReturn(r.id, { volume })}
                onPan={(pan) => updateReturn(r.id, { pan })}
                onMute={() => updateReturn(r.id, { mute: !r.mute })}
                preFader={r.preFader}
                onPreFader={() => updateReturn(r.id, { preFader: !r.preFader })}
              />
            </div>
          ))}
          <span className="strip-gap" aria-hidden="true" />
          <Strip
            id="master"
            name="Master"
            kind="master"
            volume={master}
            pan={0}
            mute={false}
            devices={masterDevices ?? []}
            selected={chainOwner === MASTER}
            onSelect={() => selectChain(MASTER)}
            onVolume={setMasterVolume}
          />
        </div>
      </div>
    </div>
  );
}
