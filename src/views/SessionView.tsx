import { useEffect, useMemo, useRef, useState, type CSSProperties, type MouseEvent } from "react";
import { Icon } from "../components/Icon";
import { Popover } from "../components/Menu";
import { trackVar } from "../components/trackColor";
import { WarpControls } from "../components/WarpControls";
import { inTauri, type ClipState } from "../engine/bridge";
import { clipProgress } from "../state/meters";
import { positionBeats, useStore } from "../state/store";
import { loopTempo, tempoOf } from "../state/tempo";
import { clipLength, slotClip, type Clip, type Follow, type Track } from "../state/types";
import { Select } from "../components/Select";

const FOLLOWS: [Follow, string][] = [
  ["next", "Next clip"],
  ["previous", "Previous clip"],
  ["first", "First clip"],
  ["last", "Last clip"],
  ["any", "Any clip"],
  ["other", "Any other clip"],
  ["again", "Play it again"],
  ["stop", "Stop"],
];

/** Launch quantisation choices in beats; bars follow the time signature. */
function quantizeChoices(beatsPerBar: number): [number, string][] {
  const all: [number, string][] = [
    [0, "None"],
    [0.25, "1/16"],
    [0.5, "1/8"],
    [1, "1/4"],
    [beatsPerBar / 2, "½ bar"],
    [beatsPerBar, "1 bar"],
    [beatsPerBar * 2, "2 bars"],
    [beatsPerBar * 4, "4 bars"],
    [beatsPerBar * 8, "8 bars"],
  ];
  return all.filter(([v], i) => all.findIndex(([w]) => w === v) === i);
}

/** How far through its loop a playing clip is: from the engine in the desktop app, from the playhead in the browser. */
function SlotProgress({ trackId, lengthBeats }: { trackId: string; lengthBeats: number }) {
  const ref = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    let raf = 0;
    const draw = () => {
      const s = useStore.getState();
      const progress = inTauri ? (clipProgress.get(trackId) ?? 0) : s.playing ? (positionBeats(s) % lengthBeats) / lengthBeats : 0;
      if (ref.current) ref.current.style.width = `${progress * 100}%`;
      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [trackId, lengthBeats]);
  return <span ref={ref} className="slot-progress" aria-hidden="true" />;
}

function SlotMenu({ track, scene, clip, anchor, onClose }: { track: Track; scene: number; clip: Clip | undefined; anchor: HTMLElement; onClose: () => void }) {
  const selected = useStore((s) => track.clips.find((c) => c.id === s.selectedClipId));
  const { launchSlot, stopTrack, newSlotClip, putInSlot, clearSlot, openPianoRoll, selectClip, setView } = useStore.getState();
  const item = (label: string, run: () => void) => (
    <button
      type="button"
      role="menuitem"
      className="mi"
      onClick={() => {
        onClose();
        run();
      }}
    >
      <span className="mi-check" />
      <span className="mi-label">{label}</span>
    </button>
  );
  return (
    <Popover anchor={anchor} onClose={onClose} label="Slot">
      {clip ? (
        <>
          {item("Launch", () => launchSlot(track.id, scene))}
          {item(clip.kind === "midi" ? "Edit notes" : "Edit audio", () => {
            selectClip(track.id, clip.id);
            if (clip.kind === "midi") openPianoRoll(clip.id);
            else setView("editor");
          })}
          <div className="msep" role="separator" />
          {item("Delete clip", () => clearSlot(track.id, scene))}
        </>
      ) : (
        <>
          {track.kind === "midi" && item("New MIDI clip", () => void newSlotClip(track.id, scene))}
          {selected && selected.slot !== scene && item(`Put “${selected.name}” here`, () => putInSlot(selected.id, scene))}
          {item(`Stop ${track.name}`, () => stopTrack(track.id))}
        </>
      )}
    </Popover>
  );
}

function SlotCell({ track, scene, state, sceneName }: { track: Track; scene: number; state: ClipState | undefined; sceneName: string }) {
  const clip = slotClip(track, scene);
  const tempo = useStore((s) => loopTempo(s.project));
  const selected = useStore((s) => !!clip && s.selectedClipId === clip.id);
  const [menu, setMenu] = useState<HTMLElement | null>(null);
  const { launchSlot, stopTrack, selectClip, newSlotClip, openPianoRoll, setView } = useStore.getState();
  const playing = state?.playing === scene;
  const queued = state?.queued === scene;
  const onMenu = (e: MouseEvent<HTMLElement>) => {
    e.preventDefault();
    setMenu(e.currentTarget);
  };
  if (!clip) {
    return (
      <>
        <button
          type="button"
          className="slot empty"
          aria-label={`Empty slot, ${track.name}, scene ${sceneName}. Stops the track.`}
          title={track.kind === "midi" ? "Click stops the track · double-click makes a MIDI clip · right-click for more" : "Click stops the track · right-click for more"}
          onClick={() => stopTrack(track.id)}
          onDoubleClick={() => track.kind === "midi" && void newSlotClip(track.id, scene)}
          onContextMenu={onMenu}
        >
          <span className="slot-stop" aria-hidden="true" />
        </button>
        {menu && <SlotMenu track={track} scene={scene} clip={undefined} anchor={menu} onClose={() => setMenu(null)} />}
      </>
    );
  }
  const classes = ["slot filled", playing && "playing", queued && "queued", selected && "sel"].filter(Boolean).join(" ");
  return (
    <>
      <button
        type="button"
        className={classes}
        style={{ "--c": trackVar(track.color) } as CSSProperties}
        aria-pressed={playing}
        aria-label={`${playing ? "Playing" : queued ? "Starting" : "Launch"} ${clip.name} on ${track.name}`}
        title={`${clip.name} · click to launch · double-click to edit · right-click for more`}
        onClick={() => {
          selectClip(track.id, clip.id);
          launchSlot(track.id, scene);
        }}
        onDoubleClick={() => (clip.kind === "midi" ? openPianoRoll(clip.id) : setView("editor"))}
        onContextMenu={onMenu}
      >
        <Icon name={playing ? "stop" : "play"} size={11} />
        <span className="slot-name">{clip.name}</span>
        {clip.follow && (
          <span className="slot-follow" title={`Then: ${FOLLOWS.find(([f]) => f === clip.follow?.action)?.[1]} after ${clip.follow.bars} bars`}>
            <Icon name="chevronRight" size={11} />
          </span>
        )}
        {playing && <SlotProgress trackId={track.id} lengthBeats={clipLength(clip, tempo)} />}
      </button>
      {menu && <SlotMenu track={track} scene={scene} clip={clip} anchor={menu} onClose={() => setMenu(null)} />}
    </>
  );
}

/** The selected clip: a session clip's follow action, or a timeline clip's way into a slot. */
function ClipPanel() {
  const allTracks = useStore((s) => s.project.tracks);
  const selectedClipId = useStore((s) => s.selectedClipId);
  const found = useMemo(() => {
    for (const track of allTracks) {
      const clip = track.clips.find((x) => x.id === selectedClipId);
      if (clip) return { track, clip };
    }
    return null;
  }, [allTracks, selectedClipId]);
  const scenes = useStore((s) => s.project.scenes);
  const bpb = useStore((s) => s.project.beatsPerBar);
  const project = useStore((s) => s.project);
  const { setFollow, putInSlot, clearSlot } = useStore.getState();
  if (!found) return <p className="session-note">Select a clip to set what happens after it plays. Right-click a slot for more.</p>;
  const { track, clip } = found;
  const bars = Math.round((clipLength(clip, clip.slot == null ? tempoOf(project) : loopTempo(project)) / bpb) * 100) / 100;
  if (clip.slot == null) {
    const free = scenes.findIndex((_, i) => !slotClip(track, i));
    return (
      <div className="clip-panel card">
        <span className="tdot" style={{ background: trackVar(track.color) }} />
        <b>{clip.name}</b>
        <span className="muted">
          on the timeline of {track.name} · {bars} {bars === 1 ? "bar" : "bars"}
        </span>
        <span className="selwrap">
          <Select className="select sm" aria-label="Put a copy in a scene" value="" onChange={(e) => putInSlot(clip.id, Number(e.target.value))}>
            <option value="" disabled>
              Put a copy in…
            </option>
            {scenes.map((name, i) => (
              <option key={i} value={i}>
                {name}
                {slotClip(track, i) ? " (replaces its clip)" : ""}
              </option>
            ))}
            <option value={scenes.length}>A new scene</option>
          </Select>
        </span>
        {free >= 0 && (
          <button className="btn xs" type="button" onClick={() => putInSlot(clip.id, free)}>
            Into {scenes[free]}
          </button>
        )}
        {clip.kind === "audio" && <WarpControls clip={clip} />}
      </div>
    );
  }
  const follow = clip.follow;
  return (
    <div className="clip-panel card">
      <span className="tdot" style={{ background: trackVar(track.color) }} />
      <b>{clip.name}</b>
      <span className="muted">
        {track.name} · {scenes[clip.slot] ?? `Scene ${clip.slot + 1}`} · {bars} {bars === 1 ? "bar" : "bars"} · loops
      </span>
      <label className="cp-field">
        <span>Then</span>
        <span className="selwrap">
          <Select
            className="select sm"
            value={follow?.action ?? ""}
            onChange={(e) => setFollow(clip.id, e.target.value ? { action: e.target.value as Follow, bars: follow?.bars ?? Math.max(1, Math.ceil(bars)) } : null)}
          >
            <option value="">Keep looping</option>
            {FOLLOWS.map(([f, label]) => (
              <option key={f} value={f}>
                {label}
              </option>
            ))}
          </Select>
        </span>
      </label>
      {follow && (
        <label className="cp-field">
          <span>after</span>
          <input
            className="num-input"
            type="number"
            min={0.25}
            max={128}
            step={0.25}
            defaultValue={follow.bars}
            key={`${clip.id}-${follow.bars}`}
            aria-label="Bars before the follow action"
            onBlur={(e) => {
              const v = Number(e.target.value);
              if (Number.isFinite(v) && v > 0 && v !== follow.bars) setFollow(clip.id, { ...follow, bars: v });
            }}
            onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          />
          <span>bars</span>
        </label>
      )}
      {clip.kind === "audio" && <WarpControls clip={clip} />}
      <button className="ib xs" type="button" aria-label={`Delete ${clip.name} from its slot`} title="Delete from the slot" onClick={() => clearSlot(track.id, clip.slot ?? 0)}>
        <Icon name="trash" size={13} />
      </button>
    </div>
  );
}

export function SessionView() {
  const allTracks = useStore((s) => s.project.tracks);
  const scenes = useStore((s) => s.project.scenes);
  const states = useStore((s) => s.clipStates);
  const selectedTrackId = useStore((s) => s.selectedTrackId);
  const quantize = useStore((s) => s.project.launchQuantize);
  const bpb = useStore((s) => s.project.beatsPerBar);
  const recording = useStore((s) => s.recording);
  const { launchScene, stopTrack, stopAllClips, selectTrack, backToArrangement, setLaunchQuantize, addScene } = useStore.getState();
  // Group tracks play what their tracks send them: they have no clips of their own.
  const tracks = allTracks.filter((t) => t.kind !== "group");
  const groupName = (t: Track) => (t.group ? allTracks.find((g) => g.id === t.group)?.name : undefined);
  const leftArrangement = Object.values(states).some((c) => !c.arrangement);
  const choices = quantizeChoices(bpb);
  if (!choices.some(([v]) => v === quantize)) choices.push([quantize, `${quantize} beats`]);

  return (
    <div className="session">
      <div className="session-bar">
        <label className="cp-field">
          <span>Launch</span>
          <span className="selwrap tip-b" data-tip="Clips start on the next line of this grid, so they stay in time">
            <Select className="select sm" aria-label="Launch quantisation" value={quantize} onChange={(e) => setLaunchQuantize(Number(e.target.value))}>
              {choices.map(([v, label]) => (
                <option key={v} value={v}>
                  {label}
                </option>
              ))}
            </Select>
          </span>
        </label>
        <button
          className={`btn sm${leftArrangement ? " back-on" : ""}`}
          type="button"
          disabled={!leftArrangement}
          onClick={backToArrangement}
          title="Every track plays the arrangement again, from where the playhead is"
        >
          <Icon name="arrange" size={14} />
          Back to arrangement
        </button>
        <span className="session-hint muted">
          {recording ? (
            <>
              <span className="rec-dot" aria-hidden="true" /> Recording: what the session plays goes into the arrangement
            </>
          ) : (
            "Press record (R) to write what the session plays into the arrangement"
          )}
        </span>
      </div>

      <div className="sgrid" style={{ gridTemplateColumns: `repeat(${tracks.length}, minmax(132px, 1fr)) 148px` }}>
        {tracks.map((t) => {
          const state = states[t.id];
          return (
            <button
              key={t.id}
              type="button"
              className={`shead${t.id === selectedTrackId ? " on" : ""}${state && !state.arrangement ? " in-session" : ""}`}
              style={{ "--c": trackVar(t.color) } as CSSProperties}
              onClick={() => selectTrack(t.id)}
              title={state && !state.arrangement ? `${t.name} plays the session, not the arrangement` : t.name}
            >
              <span className="tdot" />
              <span className="shead-name">
                {groupName(t) && <span className="shead-group">{groupName(t)} ›</span>}
                {t.name}
              </span>
            </button>
          );
        })}
        <div className="shead scenes">Scenes</div>

        {scenes.map((scene, si) => (
          <div key={si} style={{ display: "contents" }}>
            {tracks.map((t) => (
              <SlotCell key={t.id} track={t} scene={si} sceneName={scene} state={states[t.id]} />
            ))}
            <button type="button" className="scene" onClick={() => launchScene(si)} aria-label={`Launch scene ${scene}`}>
              <Icon name="play" size={11} />
              <span>{scene}</span>
              <span className="scene-n">{si + 1}</span>
            </button>
          </div>
        ))}

        {tracks.map((t) => {
          const state = states[t.id];
          return (
            <button
              key={t.id}
              type="button"
              className={`slot stopbtn${state?.stopping ? " queued" : ""}${state && !state.arrangement && state.playing === null ? " stopped" : ""}`}
              aria-label={`Stop ${t.name}`}
              onClick={() => stopTrack(t.id)}
            >
              <span className="slot-stop" aria-hidden="true" />
            </button>
          );
        })}
        <button type="button" className="scene stopall" onClick={stopAllClips}>
          <span className="slot-stop" aria-hidden="true" />
          <span>Stop all clips</span>
        </button>
        {tracks.map((t) => (
          <span key={t.id} aria-hidden="true" />
        ))}
        <button type="button" className="scene addscene" onClick={addScene}>
          <Icon name="plus" size={12} />
          <span>Scene</span>
        </button>
      </div>
      <ClipPanel />
    </div>
  );
}
