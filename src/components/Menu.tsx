import { useEffect, useLayoutEffect, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { COMMANDS, isEnabled, shortcutOf, type Command, type MenuId } from "../commands";
import { useStore } from "../state/store";
import { Icon } from "./Icon";

interface PopoverProps {
  anchor: HTMLElement | null;
  onClose: () => void;
  children: ReactNode;
  label: string;
  align?: "left" | "right";
}

/** A floating menu surface anchored under a button. Closes on outside click and Escape. */
export function Popover({ anchor, onClose, children, label, align = "left" }: PopoverProps) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onDown = (e: PointerEvent) => {
      const t = e.target as Node;
      if (!ref.current?.contains(t) && !anchor?.contains(t)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
        anchor?.focus();
      }
    };
    document.addEventListener("pointerdown", onDown, true);
    document.addEventListener("keydown", onKey, true);
    // A field that focused itself keeps it; otherwise the first button gets it.
    if (!ref.current?.contains(document.activeElement)) ref.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    return () => {
      document.removeEventListener("pointerdown", onDown, true);
      document.removeEventListener("keydown", onKey, true);
    };
  }, [anchor, onClose]);

  // No room below the anchor (a track near the bottom): open above it instead.
  // Measured again when the content changes size (a list that arrives later).
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || !anchor) return;
    const place = () => {
      const r = anchor.getBoundingClientRect();
      const height = el.getBoundingClientRect().height;
      el.style.top = r.bottom + 6 + height > window.innerHeight - 8 ? `${Math.max(8, r.top - 6 - height)}px` : `${r.bottom + 6}px`;
    };
    place();
    const watch = new ResizeObserver(place);
    watch.observe(el);
    return () => watch.disconnect();
  }, [anchor]);

  const rect = anchor?.getBoundingClientRect();
  const style = rect
    ? align === "left"
      ? { top: rect.bottom + 6, left: rect.left }
      : { top: rect.bottom + 6, right: window.innerWidth - rect.right }
    : undefined;

  // On the page itself: inside frosted glass or a container, a fixed menu would
  // be placed (and clipped) by that ancestor instead of the window.
  return createPortal(
    <div ref={ref} className="menu shine" role="menu" aria-label={label} style={style}>
      {children}
    </div>,
    document.body,
  );
}

export function MenuItem({ cmd, onDone }: { cmd: Command; onDone: () => void }) {
  // Selectors return booleans, so the item only re-renders when its own state changes.
  const enabled = useStore(() => isEnabled(cmd));
  const checked = useStore(() => cmd.checked?.() ?? false);
  return (
    <button
      type="button"
      role={cmd.checked ? "menuitemcheckbox" : "menuitem"}
      aria-checked={cmd.checked ? !!checked : undefined}
      className="mi"
      disabled={!enabled}
      onClick={() => {
        onDone();
        cmd.run?.();
      }}
    >
      <span className="mi-check">{checked && <Icon name="check" size={15} />}</span>
      <span className="mi-label">{cmd.label}</span>
      {cmd.phase && !cmd.run ? <span className="tag">Phase {cmd.phase}</span> : shortcutOf(cmd) && <span className="mi-k">{shortcutOf(cmd)}</span>}
    </button>
  );
}

export function CommandMenu({ menu, anchor, onClose }: { menu: MenuId; anchor: HTMLElement | null; onClose: () => void }) {
  const items = COMMANDS.filter((c) => c.menu === menu);
  return (
    <Popover anchor={anchor} onClose={onClose} label={menu}>
      {items.map((cmd, i) => (
        <div key={cmd.id} style={{ display: "contents" }}>
          {i > 0 && items[i - 1].group !== cmd.group && <div className="msep" role="separator" />}
          <MenuItem cmd={cmd} onDone={onClose} />
        </div>
      ))}
    </Popover>
  );
}
