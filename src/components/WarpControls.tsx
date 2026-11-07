import { useState } from "react";
import { useStore } from "../state/store";
import type { AudioClip, WarpMode } from "../state/types";
import { Icon } from "./Icon";
import { Popover } from "./Menu";
import { Select } from "./Select";

const MODES: [WarpMode, string, string][] = [
  ["beats", "Beats", "For drums and loops: every hit keeps its own speed, so the groove stays tight"],
  ["tones", "Tones", "For melodies and voices: stretched smoothly, the pitch stays"],
  ["repitch", "Repitch", "Like a turntable: faster also means higher"],
];

const SLICES: [number | null, string][] = [
  [null, "At its transients"],
  [0.25, "Every 1/16"],
  [0.5, "Every 1/8"],
  [1, "Every beat"],
];

/** Warping and slicing for an audio clip: whether it follows the song, its own tempo, how it stretches, and a way into a MIDI track. */
export function WarpControls({ clip }: { clip: AudioClip }) {
  const busy = useStore((s) => s.busy !== null);
  const { warpClip, setWarp, sliceToMidi } = useStore.getState();
  const [sliceMenu, setSliceMenu] = useState<HTMLElement | null>(null);
  const warp = clip.warp;
  const mode = MODES.find(([m]) => m === (warp?.mode ?? "beats")) ?? MODES[0];
  const setBpm = (bpm: number) => {
    if (warp && Number.isFinite(bpm) && bpm > 0 && Math.abs(bpm - warp.bpm) > 0.004) setWarp(clip.id, { ...warp, bpm });
  };

  return (
    <div className="warp-controls" role="group" aria-label={`Warping of ${clip.name}`}>
      <button
        className="switch sm tip-t"
        type="button"
        role="switch"
        aria-checked={!!warp}
        aria-label="Warp: follow the song's tempo"
        data-tip={warp ? "Warped: follows the song's tempo. Off plays the audio at its own speed." : "Follow the song's tempo: finds the clip's own tempo first"}
        disabled={busy}
        onClick={() => (warp ? setWarp(clip.id, null) : void warpClip(clip.id))}
      />
      <span className="warp-label">Warp</span>
      <span className="selwrap tip-t" data-tip={mode[2]}>
        <Select
          className="select sm"
          aria-label="Warp mode"
          value={mode[0]}
          disabled={!warp}
          onChange={(e) => warp && setWarp(clip.id, { ...warp, mode: e.target.value as WarpMode })}
        >
          {MODES.map(([m, label]) => (
            <option key={m} value={m}>
              {label}
            </option>
          ))}
        </Select>
      </span>
      {warp && (
        <>
          <label className="cp-field warp-bpm tip-t" data-tip="The clip's own tempo: it plays faster or slower to fit the song's">
            <input
              className="num-input"
              type="number"
              min={20}
              max={999}
              step={0.01}
              defaultValue={warp.bpm}
              key={`${clip.id}-${warp.bpm}`}
              aria-label="The clip's own tempo in BPM"
              onBlur={(e) => setBpm(Number(e.target.value))}
              onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
            />
            <span>BPM</span>
          </label>
          <span className="seg warp-half" role="group" aria-label="Halve or double the clip's tempo">
            <button type="button" className="tip-t" data-tip="Half the tempo: the clip plays twice as fast" onClick={() => setBpm(warp.bpm / 2)}>
              ½
            </button>
            <button type="button" className="tip-t" data-tip="Double the tempo: the clip plays half as fast" onClick={() => setBpm(warp.bpm * 2)}>
              ×2
            </button>
          </span>
          <button className="ib xs tip-t" type="button" aria-label="Find the clip's tempo again" data-tip="Find the clip's tempo again" disabled={busy} onClick={() => void warpClip(clip.id, warp.mode)}>
            <Icon name="sparkle" size={13} />
          </button>
        </>
      )}
      <span className="vsep" />
      <button
        className="btn xs"
        type="button"
        aria-haspopup="menu"
        aria-expanded={!!sliceMenu}
        disabled={busy}
        onClick={(e) => setSliceMenu(sliceMenu ? null : e.currentTarget)}
      >
        <Icon name="split" size={13} />
        <span className="slice-label">Slice to MIDI</span>
        <Icon name="chevronDown" size={12} />
      </button>
      {sliceMenu && (
        <Popover anchor={sliceMenu} onClose={() => setSliceMenu(null)} label="Slice to MIDI">
          <div className="mi-menu mi-group">A new track with a sampler, one note per slice</div>
          {SLICES.map(([division, label]) => (
            <button
              key={label}
              type="button"
              role="menuitem"
              className="mi"
              onClick={() => {
                setSliceMenu(null);
                void sliceToMidi(clip.id, division);
              }}
            >
              <span className="mi-check" />
              <span className="mi-label">{label}</span>
            </button>
          ))}
        </Popover>
      )}
    </div>
  );
}
