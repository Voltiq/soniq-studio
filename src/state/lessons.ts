/*
 * Lessons inside the app, like Live's: a card walks through a few steps,
 * points at the control each one needs, and moves on by itself once the
 * step is done in the app.
 */
import { editorTarget, type Store } from "./store";

export interface LessonStep {
  title: string;
  text: string;
  /** A control to point at: a CSS selector, and text it must contain. */
  spot?: { selector: string; text?: string };
  /** Something the card can do for you to start the step. */
  action?: { label: string; run: (s: Store) => void | Promise<void> };
  /** Done when this holds; `start` is the app as it was when the step began. */
  done?: (s: Store, start: Store) => boolean;
}

export interface Lesson {
  id: string;
  title: string;
  description: string;
  minutes: number;
  steps: LessonStep[];
}

const midiTracks = (s: Store) => s.project.tracks.filter((t) => t.kind === "midi").length;
const selected = (s: Store) => s.project.tracks.find((t) => t.id === s.selectedTrackId);
const isDemo = (s: Store) => s.project.name.includes("(demo)");

export const LESSONS: Lesson[] = [
  {
    id: "tour",
    title: "Find your way around",
    description: "Play the demo song, mute a track, and look at the mixer and the meters.",
    minutes: 2,
    steps: [
      {
        title: "Open the demo song",
        text: "“Night Drive” has drums, bass, keys and a pad to play with.",
        action: { label: "Open the demo", run: (s) => s.openDemo() },
        done: isDemo,
      },
      {
        title: "Play it",
        text: "Press Space, or click Play in the transport bar. Space again pauses.",
        spot: { selector: "button[aria-label='Play'], button[aria-label='Pause']" },
        done: (s) => s.playing,
      },
      {
        title: "Mute the drums",
        text: "Click M on the Drums track to hear the song without them. Click it again to bring them back.",
        spot: { selector: ".thead", text: "Drums" },
        done: (s) => !!s.project.tracks.find((t) => t.name === "Drums")?.mute,
      },
      {
        title: "Open the mixer",
        text: "Press F3, or click Mixer above the tracks. Each track has a fader, pan, sends and its devices.",
        spot: { selector: "[role='tab']", text: "Mixer" },
        done: (s) => s.view === "mixer",
      },
      {
        title: "Watch the meters",
        text: "Click the meter button at the end of the tabs (Ctrl+Alt+M): loudness, peaks and the spectrum of the song as it plays.",
        spot: { selector: ".tab-end" },
        done: (s) => s.analysisOpen,
      },
      {
        title: "Stop",
        text: "Press Space to stop. That's the tour: the other lessons show how to make things.",
        done: (s) => !s.playing,
      },
    ],
  },
  {
    id: "beat",
    title: "Make a beat",
    description: "A MIDI track, a Drum Rack and a pattern in the step sequencer.",
    minutes: 4,
    steps: [
      {
        title: "Start a new song",
        text: "An empty song has two audio and two MIDI tracks. (It asks first if this one has changes.)",
        action: { label: "New empty song", run: (s) => void s.newProject("empty") },
        done: (s) => s.project.name === "Untitled" && !isDemo(s),
      },
      {
        title: "Add a MIDI track",
        text: "Track › New MIDI track, or Ctrl+Shift+T. MIDI tracks play notes through an instrument.",
        spot: { selector: ".menubtn", text: "Track" },
        done: (s, start) => midiTracks(s) > midiTracks(start),
      },
      {
        title: "Give it drums",
        text: "In the browser on the left, click Drum Rack: 16 pads with built-in drums.",
        spot: { selector: ".browser [role='listitem']", text: "Drum Rack" },
        done: (s) => !!selected(s)?.devices.some((d) => d.type === "drum-rack"),
      },
      {
        title: "Add a clip",
        text: "Clip › New MIDI clip puts an empty clip on the track at the playhead.",
        spot: { selector: ".menubtn", text: "Clip" },
        done: (s) => !!selected(s)?.clips.some((c) => c.kind === "midi"),
      },
      {
        title: "Open the step sequencer",
        text: "Clip › Step sequencer shows the clip as rows of pads and steps.",
        spot: { selector: ".menubtn", text: "Clip" },
        done: (s) => s.view === "piano" && s.pianoMode === "steps",
      },
      {
        title: "Program a kick",
        text: "Click the Kick row on steps 1, 5, 9 and 13: four on the floor. Add snares on 5 and 13, hats wherever you like.",
        done: (s) => {
          const clip = selected(s)?.clips.find((c) => c.id === s.selectedClipId);
          return clip?.kind === "midi" && clip.notes.length >= 4;
        },
      },
      {
        title: "Play your beat",
        text: "Press Space. Turn on Loop (L) to hear it go round; Space again stops.",
        done: (s) => s.playing,
      },
    ],
  },
  {
    id: "edit",
    title: "Edit audio",
    description: "Open a clip in the audio editor, fade it in, make it louder, and step back through the history.",
    minutes: 3,
    steps: [
      {
        title: "Open the demo song",
        text: "Its Pad track is audio: a good clip to edit.",
        action: { label: "Open the demo", run: (s) => s.openDemo() },
        done: isDemo,
      },
      {
        title: "Open the pad in the audio editor",
        text: "Click the “Warm pad” clip on the Pad track, then press F4 (or click Audio editor).",
        spot: { selector: "[role='tab']", text: "Audio editor" },
        done: (s) => s.view === "editor" && !!editorTarget(s),
      },
      {
        title: "Select a stretch",
        text: "Drag across the start of the waveform to select a few seconds.",
        done: (s) => !!s.editorSelection,
      },
      {
        title: "Fade it in",
        text: "Clip › Fade in. The selection now rises from silence.",
        spot: { selector: ".menubtn", text: "Clip" },
        done: (s) => s.history.undoLabel?.includes("Fade in") ?? false,
      },
      {
        title: "Make it louder",
        text: "Clip › Normalize raises the clip until its loudest peak just fits.",
        spot: { selector: ".menubtn", text: "Clip" },
        done: (s) => s.history.undoLabel?.includes("Normalize") ?? false,
      },
      {
        title: "Look back",
        text: "Open the History (the clock by undo and redo, or Ctrl+H) and click a step to go back to it. Every change can be undone.",
        spot: { selector: ".history-btn" },
        done: (s) => s.historyOpen,
      },
    ],
  },
];

const DONE_KEY = "soniq.lessonsDone";

export function loadLessonsDone(): string[] {
  try {
    const done = JSON.parse(localStorage.getItem(DONE_KEY) ?? "[]");
    return Array.isArray(done) ? done.filter((d): d is string => typeof d === "string") : [];
  } catch {
    return [];
  }
}

export function saveLessonsDone(done: string[]) {
  try {
    localStorage.setItem(DONE_KEY, JSON.stringify(done));
  } catch {
    // Remembered for this session only.
  }
}
