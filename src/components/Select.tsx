import {
  Children,
  Fragment,
  isValidElement,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { Icon } from "./Icon";

/**
 * The custom dropdown (the Graphiq one): a field-style trigger whose chevron
 * turns over while open, and a list on the app's menu surface that grows out
 * of the trigger on a spring, its rows arriving in a short cascade and the
 * chosen one's tick popping in.
 *
 * It replaces the native `select` because the open list is the one part of a
 * native control the theme cannot reach: a white system list opening out of a
 * graphite panel. Everything the native one gave is kept: arrows, Home/End,
 * type-ahead, Escape, Tab to leave, `listbox`/`option` semantics, and a width
 * that fits the longest option, so the layout does not jump when it changes.
 *
 * It takes the native one's children (`option` and `optgroup`) and calls
 * `onChange` with an event-like `{ target: { value } }`, so a native select
 * becomes one by changing its tag.
 */

interface Item {
  value: string;
  label: string;
  group?: string;
  disabled?: boolean;
}

export interface SelectChange {
  target: { value: string };
  currentTarget: { value: string };
}

/** How long a closing list stays for its exit animation. */
const EXIT_MS = 140;

/** The text of a node (`{v}%` is two children, a number and a string). */
function textOf(node: ReactNode): string {
  if (node === null || node === undefined || typeof node === "boolean") return "";
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  if (isValidElement<{ children?: ReactNode }>(node)) return textOf(node.props.children);
  return "";
}

/** The options a native select would show, from its children. */
function itemsOf(children: ReactNode, group?: string, out: Item[] = []): Item[] {
  Children.forEach(children, (child) => {
    if (!isValidElement<{ children?: ReactNode; value?: string | number; label?: string; disabled?: boolean }>(child)) return;
    const { props } = child;
    if (child.type === Fragment) itemsOf(props.children, group, out);
    else if (child.type === "optgroup") itemsOf(props.children, props.label, out);
    else if (child.type === "option") {
      const label = textOf(props.children);
      out.push({ value: String(props.value ?? label), label, group, disabled: props.disabled });
    }
  });
  return out;
}

/** Keeps the list mounted for its exit; opening again mid-exit cancels it. */
function usePresence(open: boolean) {
  const [mounted, setMounted] = useState(open);
  const [previous, setPrevious] = useState(open);
  if (open !== previous) {
    setPrevious(open);
    if (open) setMounted(true);
  }
  useEffect(() => {
    if (open || !mounted) return;
    const reduced = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
    const timer = window.setTimeout(() => setMounted(false), reduced ? 0 : EXIT_MS);
    return () => window.clearTimeout(timer);
  }, [open, mounted]);
  return { mounted: open || mounted, closing: !open && mounted };
}

export function Select({
  value,
  defaultValue,
  onChange,
  children,
  className,
  disabled = false,
  id,
  title,
  style,
  "aria-label": ariaLabel,
}: {
  value?: string | number;
  defaultValue?: string | number;
  onChange?: (e: SelectChange) => void;
  children: ReactNode;
  className?: string;
  disabled?: boolean;
  id?: string;
  title?: string;
  style?: CSSProperties;
  "aria-label"?: string;
}) {
  const items = itemsOf(children);
  const [inner, setInner] = useState(defaultValue !== undefined ? String(defaultValue) : (items[0]?.value ?? ""));
  const current = value !== undefined ? String(value) : inner;
  const chosen = items.find((i) => i.value === current);
  const shown = chosen?.label ?? current;

  const [open, setOpen] = useState(false);
  const { mounted, closing } = usePresence(open);
  const [place, setPlace] = useState<{ top: number; left: number; width: number; origin: string } | null>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const focusOnOpen = useRef(false);
  const typeahead = useRef<{ query: string; timer: number } | null>(null);

  const choose = (next: string, byKey: boolean) => {
    setOpen(false);
    if (next !== current) {
      if (value === undefined) setInner(next);
      onChange?.({ target: { value: next }, currentTarget: { value: next } });
    }
    // Back to the trigger after a keyboard choice, as a native select does.
    if (byKey) buttonRef.current?.focus();
  };
  const optionButtons = () => [...(listRef.current?.querySelectorAll<HTMLButtonElement>('[role="option"]:not(:disabled)') ?? [])];

  // Under the trigger, or above it when there is no room; kept on screen.
  useLayoutEffect(() => {
    if (!mounted || closing) return;
    const measure = () => {
      const trigger = buttonRef.current?.getBoundingClientRect();
      if (!trigger) return;
      const height = listRef.current?.offsetHeight ?? 0;
      const width = Math.max(trigger.width, listRef.current?.offsetWidth ?? 0);
      const margin = 8;
      let top = trigger.bottom + 6;
      const above = top + height > window.innerHeight - margin;
      if (above) top = Math.max(margin, trigger.top - 6 - height);
      const left = Math.min(Math.max(margin, trigger.left), window.innerWidth - width - margin);
      // It grows out of its trigger: from the trigger's middle, at the edge it hangs from.
      const origin = `${trigger.left + trigger.width / 2 - left}px ${above ? "100%" : "0px"}`;
      setPlace({ top, left, width: trigger.width, origin });
    };
    measure();
    window.addEventListener("resize", measure);
    window.addEventListener("scroll", measure, true);
    return () => {
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", measure, true);
    };
  }, [mounted, closing]);

  // Opened from the keyboard: land on the chosen option once the list is placed.
  useEffect(() => {
    if (!open || !place || !focusOnOpen.current) return;
    const options = optionButtons();
    (options.find((o) => o.getAttribute("aria-selected") === "true") ?? options[0])?.focus();
    focusOnOpen.current = false;
  }, [open, place]);

  // The chosen option starts in view in a long list.
  useLayoutEffect(() => {
    if (!open || !place) return;
    listRef.current?.querySelector<HTMLElement>('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
  }, [open, place]);

  // A press anywhere else closes it.
  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      const t = e.target as Node;
      if (buttonRef.current?.contains(t) || listRef.current?.contains(t)) return;
      setOpen(false);
    };
    document.addEventListener("pointerdown", onDown, true);
    return () => document.removeEventListener("pointerdown", onDown, true);
  }, [open]);

  // The trigger's keys stop here, so Space does not also start playback.
  const onTriggerKey = (e: ReactKeyboardEvent) => {
    if (e.key === "Enter" || e.key === " ") e.stopPropagation();
    else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      e.stopPropagation();
      focusOnOpen.current = true;
      setOpen(true);
    } else if (e.key === "Escape" && open) {
      e.preventDefault();
      e.stopPropagation();
      setOpen(false);
    } else if (!open && e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
      // Closed, a letter picks the next option starting with it, as a native
      // select does (and is not a shortcut for the rest of the app).
      e.preventDefault();
      e.stopPropagation();
      const k = e.key.toLowerCase();
      const enabled = items.filter((i) => !i.disabled);
      const from = enabled.findIndex((i) => i.value === current);
      const next = [...enabled.slice(from + 1), ...enabled.slice(0, from + 1)].find((i) => i.label.toLowerCase().startsWith(k));
      if (next) choose(next.value, false);
    }
  };

  // The list's keys stop here too: a dialog must not read Escape as "close".
  const onListKey = (e: ReactKeyboardEvent) => {
    const options = optionButtons();
    const at = options.indexOf(document.activeElement as HTMLButtonElement);
    const moves: Record<string, number> = {
      ArrowDown: Math.min(options.length - 1, at + 1),
      ArrowUp: Math.max(0, at - 1),
      Home: 0,
      End: options.length - 1,
    };
    if (e.key in moves) {
      e.preventDefault();
      options[moves[e.key]]?.focus();
    } else if (e.key === "Escape") {
      e.preventDefault();
      setOpen(false);
      buttonRef.current?.focus();
    } else if (e.key === "Tab") {
      setOpen(false);
      return;
    } else if (e.key.length === 1 && e.key !== " " && !e.ctrlKey && !e.metaKey && !e.altKey) {
      // Type-ahead over the labels.
      const previous = typeahead.current;
      if (previous) window.clearTimeout(previous.timer);
      const query = (previous?.query ?? "") + e.key.toLowerCase();
      typeahead.current = { query, timer: window.setTimeout(() => (typeahead.current = null), 700) };
      options.find((o) => o.textContent?.trim().toLowerCase().startsWith(query))?.focus();
    } else if (e.key !== "Enter" && e.key !== " ") return;
    e.stopPropagation();
  };

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        id={id}
        title={title}
        style={style}
        className={className ? `select sbox ${className}` : "select sbox"}
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel ? `${ariaLabel}: ${shown}` : undefined}
        onClick={(e) => {
          // detail 0 is Enter or Space rather than a pointer.
          focusOnOpen.current = e.detail === 0;
          setOpen((was) => !was);
        }}
        onKeyDown={onTriggerKey}
      >
        {/* Every label in one cell, only the chosen one visible: as wide as the longest. */}
        <span className="sbox-value">
          {items.map((i) => (
            <span key={i.value} className={i.value === current ? "on" : undefined} aria-hidden={i.value !== current || undefined}>
              {i.label}
            </span>
          ))}
          {!chosen && <span className="on">{shown}</span>}
        </span>
        <Icon name="chevronDown" size={13} className="sbox-chev" />
      </button>
      {mounted &&
        createPortal(
          <div
            ref={listRef}
            role="listbox"
            aria-label={ariaLabel}
            className="menu shine sel-menu"
            data-state={closing ? "closed" : "open"}
            aria-hidden={closing || undefined}
            onKeyDown={onListKey}
            onBlur={(e) => {
              const to = e.relatedTarget as Node | null;
              if (!listRef.current?.contains(to) && to !== buttonRef.current) setOpen(false);
            }}
            style={
              {
                top: place?.top ?? 0,
                left: place?.left ?? 0,
                minWidth: place?.width,
                visibility: place ? "visible" : "hidden",
                "--origin": place?.origin,
              } as CSSProperties
            }
          >
            {items.map((item, index) => (
              <Fragment key={item.value}>
                {item.group && item.group !== items[index - 1]?.group && (
                  <div className="sel-group" role="presentation">
                    {item.group}
                  </div>
                )}
                <button
                  type="button"
                  role="option"
                  tabIndex={-1}
                  disabled={item.disabled}
                  aria-selected={item.value === current}
                  className="sel-opt"
                  // A short cascade, capped so a long list never waits.
                  style={{ animationDelay: `${Math.min(index, 8) * 12}ms` }}
                  onClick={(e) => choose(item.value, e.detail === 0)}
                >
                  <span>{item.label}</span>
                  {item.value === current && <Icon name="check" size={14} className="sel-tick" />}
                </button>
              </Fragment>
            ))}
          </div>,
          document.body,
        )}
    </>
  );
}
