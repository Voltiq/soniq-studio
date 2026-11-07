import { useState, type ReactNode } from "react";
import { isMidiFx, isModulator } from "../state/devices";
import { useStore } from "../state/store";
import { sameTarget, type AutoTarget, type MappingTarget } from "../state/types";
import { Popover } from "./Menu";

interface LearnableProps {
  target: MappingTarget;
  /** What the control is, for menus and messages ("Cutoff", "Bass volume"). */
  label: string;
  children: ReactNode;
  /** Menu items above the MIDI ones (a rack chain's macros); `close` shuts the menu. */
  extra?: (close: () => void) => ReactNode;
  /** A small badge in the corner (the macro that turns a rack chain's knob). */
  tag?: string;
  className?: string;
  /** False renders the control alone (returns and the master have no mappings yet). */
  on?: boolean;
}

/**
 * Wraps a knob or fader so a right-click can map a MIDI controller to it:
 * "Learn MIDI controller" waits for the next knob, fader or button that
 * moves; a mapped control shows its CC number.
 */
export function Learnable({ on = true, children, ...rest }: LearnableProps) {
  return on ? <LearnableControl {...rest}>{children}</LearnableControl> : <>{children}</>;
}

/** The automation lane for a control: the track's mixer, or a device on the track (not inside a rack chain). */
function autoTargetOf(target: MappingTarget): AutoTarget {
  return target.kind === "param" ? { kind: "param", device: target.device, param: target.param } : { kind: target.kind };
}

function LearnableControl({ target, label, children, extra, tag, className }: Omit<LearnableProps, "on">) {
  const mapping = useStore((s) => s.project.mappings?.find((m) => sameTarget(m.target, target)));
  const learning = useStore((s) => !!s.midiLearn && sameTarget(s.midiLearn.target, target));
  const automatable = useStore((s) => {
    const track = s.project.tracks.find((t) => t.id === target.track);
    return !!track && (target.kind !== "param" || track.devices.some((d) => d.id === target.device && !isModulator(d.type) && !isMidiFx(d.type)));
  });
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const close = () => setAnchor(null);
  const { learnMidi, run, showAutomation } = useStore.getState();
  const classes = ["learnable", className, mapping && "cc-mapped", learning && "learning"].filter(Boolean).join(" ");
  const tip = [tag && `Macro ${tag.slice(1)} turns this`, mapping && `MIDI CC ${mapping.cc}`, "right-click to map"].filter(Boolean).join(" · ");
  return (
    <div
      className={classes}
      data-cc={mapping ? `CC${mapping.cc}` : undefined}
      data-tag={tag}
      title={tip.charAt(0).toUpperCase() + tip.slice(1)}
      onContextMenu={(e) => {
        e.preventDefault();
        setAnchor(e.currentTarget);
      }}
    >
      {children}
      {anchor && (
        <Popover anchor={anchor} onClose={close} label={`${label} mapping`}>
          {extra?.(close)}
          <button
            type="button"
            role="menuitem"
            className="mi"
            onClick={() => {
              close();
              learnMidi(learning ? null : target, label);
            }}
          >
            <span className="mi-check" />
            <span className="mi-label">{learning ? "Stop learning" : mapping ? "Learn another MIDI controller" : "Learn MIDI controller"}</span>
          </button>
          {mapping && (
            <button
              type="button"
              role="menuitem"
              className="mi"
              onClick={() => {
                close();
                void run({ op: "removeMapping", id: mapping.id });
              }}
            >
              <span className="mi-check" />
              <span className="mi-label">Forget CC {mapping.cc}</span>
            </button>
          )}
          {automatable && (
            <>
              <div className="msep" role="separator" />
              <button
                type="button"
                role="menuitem"
                className="mi"
                onClick={() => {
                  close();
                  showAutomation(target.track, autoTargetOf(target));
                }}
              >
                <span className="mi-check" />
                <span className="mi-label">Show automation</span>
              </button>
            </>
          )}
        </Popover>
      )}
    </div>
  );
}
