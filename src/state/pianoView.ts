import { useStore } from "./store";

export const MIN_PPB = 6;
export const MAX_PPB = 600;

/** The piano roll's zoom on screen (also when fitted), for the View menu's zoom commands. */
export const pianoView = { ppb: 40 };

export function zoomPiano(factor: number) {
  useStore.getState().setPianoZoom(Math.min(MAX_PPB, Math.max(MIN_PPB, pianoView.ppb * factor)));
}
