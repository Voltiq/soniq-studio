/* Stroke icons drawn to the Graphiq spec: 24 px grid, 1.6 stroke, round caps. */

const PATHS = {
  undo: "M9 14 4 9l5-5M4 9h11a5 5 0 0 1 0 10h-4",
  redo: "m15 14 5-5-5-5M20 9H9a5 5 0 0 0 0 10h4",
  export: "M12 3v12M7 10l5 5 5-5M5 21h14",
  search: "M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM20 20l-3.5-3.5",
  play: "M7 4.5v15a1 1 0 0 0 1.5.86l12-7.5a1 1 0 0 0 0-1.72l-12-7.5A1 1 0 0 0 7 4.5z",
  pause: "M7 4h3v16H7zM14 4h3v16h-3z",
  stop: "M6 6h12v12H6z",
  toStart: "M5 5v14M19 5 9 12l10 7z",
  loop: "M17 2l3 3-3 3M4 11V9a4 4 0 0 1 4-4h12M7 22l-3-3 3-3M20 13v2a4 4 0 0 1-4 4H4",
  metronome: "M9 3h6l4 18H5zM12 17l5-9M8 14h8",
  pointer: "M5 3l13 8-6 1.5L9.5 19z",
  range: "M8 4h8M8 20h8M12 4v16",
  draw: "M4 20l4-1 11-11a2.1 2.1 0 0 0-3-3L5 16zM14 7l3 3",
  split: "M6 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM6 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM20 4 8.1 15.9M14.5 14.5 20 20M8.1 8.1 12 12",
  erase: "m7 21-4.3-4.3a1 1 0 0 1 0-1.4l10-10a1 1 0 0 1 1.4 0l5.6 5.6a1 1 0 0 1 0 1.4L11 21zM22 21H7M5 11l9 9",
  automation: "M3 17c3 0 4-10 8-10s4 6 6 6 2-3 4-3M7 13.5v.01M15 12.5v.01",
  zoom: "M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM20 20l-3.5-3.5M11 8v6M8 11h6",
  hand: "M18 11V6a2 2 0 0 0-4 0M14 10V4a2 2 0 0 0-4 0v2M10 10.5V6a2 2 0 0 0-4 0v8M18 8a2 2 0 1 1 4 0v6a8 8 0 0 1-8 8h-2c-2.8 0-4.5-.9-6-2.4l-3.6-3.6a2 2 0 0 1 2.8-2.8L7 15",
  plus: "M12 5v14M5 12h14",
  minus: "M5 12h14",
  close: "M6 6l12 12M18 6 6 18",
  chevronDown: "m6 9 6 6 6-6",
  chevronUp: "m6 15 6-6 6 6",
  link: "M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1",
  steps: "M4 5h4v4H4zM10 5h4v4h-4zM16 5h4v4h-4zM4 15h4v4H4zM16 15h4v4h-4z",
  upload: "M12 16V4M7 9l5-5 5 5M4 16v3a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-3",
  chevronRight: "m9 6 6 6-6 6",
  sun: "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4",
  moon: "M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z",
  folder: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z",
  wave: "M3 12h2l2-6 3 12 3-9 2 6 2-3h4",
  piano: "M3 4h18v16H3zM8 4v9M13 4v9M18 4v9M8 13v7M13 13v7M18 13v7",
  power: "M12 3v8M6.3 7a8 8 0 1 0 11.4 0",
  trash: "M3 6h18M8 6V4h8v2M19 6l-1 14H6L5 6",
  more: "M5 12h.01M12 12h.01M19 12h.01",
  mic: "M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3zM5 11a7 7 0 0 0 14 0M12 18v3",
  grid: "M4 4h6v6H4zM14 4h6v6h-6zM4 14h6v6H4zM14 14h6v6h-6z",
  sliders: "M6 3v18M12 3v18M18 3v18M3 15h6M9 8h6M15 13h6",
  arrange: "M3 6h8M14 6h7M3 12h5M11 12h10M3 18h11M17 18h4",
  sidebar: "M4 4h16v16H4zM9 4v16",
  info: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 11v5M12 8v.01",
  warning: "M12 4 2.5 20h19zM12 10v4M12 17v.01",
  check: "m5 12 5 5 9-10",
  warp: "M3 12h18M7 8l-4 4 4 4M17 8l4 4-4 4M12 5v14",
  gear: "M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z",
  drum: "M12 5c5 0 8 1.6 8 3.5v7c0 1.9-3 3.5-8 3.5s-8-1.6-8-3.5v-7C4 6.6 7 5 12 5zM4 8.5C4 10.4 7 12 12 12s8-1.6 8-3.5M7 11.5v7M17 11.5v7",
  plug: "M9 2v6M15 2v6M6 8h12v4a6 6 0 0 1-12 0zM12 18v4",
  sparkle: "M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z",
  fx: "M4 20c4 0 5-16 9-16M7 10h7M14 12l6 8M20 12l-6 8",
  headphones: "M3 18v-6a9 9 0 0 1 18 0v6M21 19a2 2 0 0 1-2 2h-1v-6h3zM3 19a2 2 0 0 0 2 2h1v-6H3z",
  keyboard: "M3 6h18v12H3zM7 10h.01M11 10h.01M15 10h.01M7 14h10",
  meter: "M5 20v-7M10 20V5M15 20v-10M20 20v-5",
  clock: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 7v5l3 2",
  book: "M5 4.5A1.5 1.5 0 0 1 6.5 3H19v15H6.5A1.5 1.5 0 0 0 5 19.5zM5 19.5A1.5 1.5 0 0 0 6.5 21H19v-3M9 7h6",
  wand: "M15 4V2M15 16v-2M8 9h2M20 9h2M17.8 11.8 19 13M17.8 6.2 19 5M12.2 6.2 11 5M3 21l9-9",
  palette:
    "M12 3a9 9 0 0 0 0 18c1 0 1.5-.7 1.5-1.5 0-.4-.2-.8-.4-1.1-.3-.3-.4-.7-.4-1.1 0-.8.7-1.5 1.5-1.5H16a5 5 0 0 0 5-5c0-4.3-4-7.8-9-7.8zM7.5 11.5h.01M10.5 7.5h.01M15.5 8.5h.01",
} as const;

export type IconName = keyof typeof PATHS;

const FILLED: Partial<Record<IconName, true>> = { play: true, pause: true, stop: true };

export function Icon({ name, size = 18, className }: { name: IconName; size?: number; className?: string }) {
  const filled = FILLED[name];
  return (
    <svg
      className={className ? `i ${className}` : "i"}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      aria-hidden="true"
      style={filled ? { fill: "currentColor", strokeWidth: 1.2 } : undefined}
    >
      <path d={PATHS[name]} />
    </svg>
  );
}
