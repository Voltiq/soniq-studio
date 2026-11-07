import { useStore } from "../state/store";

/** What sits under a window position: a sampler or drum pad, or a lane and the beat there. */
export function dropTarget(x: number, y: number): { el: HTMLElement | null; sample?: HTMLElement; lane?: HTMLElement; beats?: number } {
  const el = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-sample-target], [data-track-id], [data-new-track]") ?? null;
  if (!el) return { el };
  if (el.dataset.sampleTarget) return { el, sample: el };
  const s = useStore.getState();
  const beats = Math.max(0, Math.round((x - el.getBoundingClientRect().left) / s.pxPerBeat));
  return { el, lane: el, beats };
}

/** A device chain under a window position (a track's, or a drum pad's), for presets. */
function chainTarget(x: number, y: number): HTMLElement | null {
  return document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-chain-owner]") ?? null;
}

let highlighted: HTMLElement | null = null;
function highlight(el: HTMLElement | null) {
  if (el === highlighted) return;
  highlighted?.classList.remove("drop-target");
  el?.classList.add("drop-target");
  highlighted = el;
}

/**
 * Dragging a library item by pointer (HTML drag and drop is off in the
 * desktop window, where Explorer drops arrive instead): a sample lands on a
 * pad, a sampler or the timeline, a preset on a device chain. A press that
 * hardly moves is a click and does nothing here.
 */
export function startLibraryDrag(e: PointerEvent | React.PointerEvent, item: { path: string; name: string; preset: boolean }) {
  if (e.button !== 0) return;
  const x0 = e.clientX;
  const y0 = e.clientY;
  let ghost: HTMLDivElement | null = null;
  const target = (x: number, y: number) => (item.preset ? chainTarget(x, y) : dropTarget(x, y).el);
  const move = (ev: PointerEvent) => {
    if (!ghost) {
      if (Math.hypot(ev.clientX - x0, ev.clientY - y0) < 6) return;
      ghost = document.createElement("div");
      ghost.className = "drag-ghost";
      ghost.textContent = item.name;
      document.body.appendChild(ghost);
      document.body.classList.add("dragging-item");
    }
    ghost.style.transform = `translate(${ev.clientX + 12}px, ${ev.clientY + 8}px)`;
    highlight(target(ev.clientX, ev.clientY));
  };
  const end = (ev: PointerEvent) => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", end);
    window.removeEventListener("pointercancel", cancel);
    highlight(null);
    if (!ghost) return;
    ghost.remove();
    document.body.classList.remove("dragging-item");
    drop(ev.clientX, ev.clientY, item);
  };
  const cancel = () => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", end);
    window.removeEventListener("pointercancel", cancel);
    highlight(null);
    ghost?.remove();
    document.body.classList.remove("dragging-item");
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", end);
  window.addEventListener("pointercancel", cancel);
}

function drop(x: number, y: number, item: { path: string; name: string; preset: boolean }) {
  const store = useStore.getState();
  if (item.preset) {
    const chain = chainTarget(x, y);
    if (chain?.dataset.chainOwner) void store.addPreset(chain.dataset.chainOwner, item.path);
    else store.toast({ tone: "info", title: "Drop a preset on a device chain", detail: "The devices panel below the tracks, or a drum pad's effects." });
    return;
  }
  const { sample, lane, beats } = dropTarget(x, y);
  if (sample) {
    const [track, device, key] = (sample.dataset.sampleTarget ?? "").split("|");
    void store.loadSample(track, device, Number(key), item.path).then(() => store.libraryUsed(item.path));
    return;
  }
  if (lane) {
    const kind = lane.dataset.trackKind;
    void store.importAudio([item.path], kind === "audio" ? lane.dataset.trackId : undefined, beats).then(() => store.libraryUsed(item.path));
    return;
  }
  store.toast({ tone: "info", title: "Drop samples on the timeline, a pad or a sampler" });
}
