# Soniq Studio — Product spec & roadmap

> Working name: **Soniq Studio**, a sibling of Graphiq / Magiq Studio. Rename by changing
> `productName` in `src-tauri/tauri.conf.json`, `name` in `package.json`, and the logo text in
> `src/components/TopBar.tsx`.

Soniq Studio is a desktop audio editor and music workstation built with Tauri 2, a Rust
real-time engine and a React UI. It should replace **Audacity 4** for recording and precise
audio editing, and **Ableton Live 12 Lite** for music production. It also takes the best
workflow ideas from **FL Studio**: the step sequencer, the piano roll tools and pattern-based
arranging.

Status legend used throughout: `[x]` done in the current scaffold · `[ ]` to do.
Priority tags: **P0** = needed to replace the current apps · **P1** = pro feature expected by
serious users · **P2** = nice to have / differentiator.

---

## 1. Vision

| Goal | What it means in practice |
| --- | --- |
| One app, two jobs | Open a voice memo and clean it up in 30 seconds, or produce a full track. The tasks share one engine, one file format and one UI. |
| Beginner-friendly by default | **Essentials** mode shows only what you need. Inline hints, sensible defaults, templates, undo for everything. |
| Pro-grade when you ask | **Pro** mode adds routing, sidechains, automation lanes, spectral tools, plugin hosting and detailed meters. Nothing is hidden behind a paywall or an edition limit. |
| Quiet chrome, loud canvas | The Graphiq rule: the interface recedes so waveforms, clips and notes are the brightest thing on screen. |
| Never lose work | Autosave, crash recovery, unlimited undo with a history panel, and non-destructive editing unless you choose destructive mode. |
| Real-time safe | The audio thread never locks, allocates or blocks. UI hiccups must never cause audio dropouts. |

### Who it is for
- **Beginners**: podcasters, students, first-time beat makers, people coming from GarageBand or Audacity.
- **Producers**: people who make songs with clips, MIDI, instruments and samples (Live and FL users).
- **Editors and engineers**: dialogue cleanup, mastering, sound design, restoration (Audacity, RX-style users).

---

## 2. Design language (Graphiq → Soniq)

The UI uses the Graphiq Studio design language from the design canvas. Every value is a token in
[`src/styles/tokens.css`](src/styles/tokens.css); components only reference tokens.

### Tokens (implemented)
- **Themes**: *Graphite* (dark, default) and *Porcelain* (light), switched with `data-theme` on `<html>`.
- **Neutrals**: `--bg`, `--bg-2`, `--surface-1…4`, `--text-1…3`, `--line`, `--line-2`. Elevation gets lighter in dark mode and whiter in light mode.
- **One accent**: `--accent` (default `#6457F0`), with derived `--accent-hi`, `--accent-lo`, `--accent-soft`, `--accent-line` and `--accent-text`, all made with `color-mix(in oklab)`. Users can pick another accent (Settings › Appearance); `--on-accent` is the text on it, white or graphite, whichever has more contrast.
- **Density**: comfortable (default) or compact, switched with `data-density` on `<html>`: smaller buttons, tabs, bars, mixer strips and 56 px arrangement lanes instead of 76.
- **Semantic colours, for status only**: `--success`, `--warning`, `--danger`, `--info`. Each always comes with an icon or a word.
- **Type**: Geist for UI, Geist Mono for every number (time, BPM, dB, Hz, sample counts) with `tabular-nums`. 13 px base. Scale: display 32/38, title 20/26, heading 15/20, body 13/20, label 12/16, caption 11/14 uppercase +14 %.
- **Space**: 4 px base (4, 8, 12, 16, 20, 24, 32, 40, 48, 64).
- **Radius**: xs 6 · sm 8 · md 12 · lg 16 · xl 22 · pill. Inner radius = outer radius − padding. Panels switch to a 34 px squircle where `corner-shape: squircle` is supported.
- **Depth from light, not lines**: `--sh-1/2/3` soft shadows, plus a 1 px gradient *edge shine* border (`.shine`).
- **Glass**: only for things that float over the canvas (zoom HUD, status chips, toasts, the command palette).
- **Motion**: `--ease` (.2,.8,.2,1), `--spring` (.34,1.56,.64,1), `--d1` 120 ms, `--d2` 220 ms, `--d3` 360 ms. Respect `prefers-reduced-motion`.
- **Touch floor**: desktop controls are 32–36 px; with a pen or finger, targets grow to 44 px.

### Audio-specific extensions (new tokens, same rules)
- **Track colours** are content, not chrome (like the artwork in Graphiq): an 8-colour palette (`--track-1…8`), tuned so waveforms stay readable in both themes. Four sets to choose from: Graphiq, Vivid, Soft and Earth.
- **Meters** use the semantic colours: success below −18 dBFS, warning from −18 to −6, danger above −6. Peak-hold ticks and a clip LED that has a word label.
- **Record** is the only control that uses `--danger` as a fill, and only while armed or recording.
- **Playhead and selection** use the accent. A time selection is `--accent-soft` with an `--accent-line` edge.
- **Knobs and faders** follow the slider spec: track `--surface-4` with an inset shadow, accent gradient fill, white thumb with a soft halo on hover or drag. Fills grow out from zero for bipolar controls such as pan.

### Components to build (Graphiq set + audio)
- [x] Buttons (primary, default, ghost, danger), icon buttons, segmented control, switch, field, select, kbd, chip, menu, dialog, toast, glass HUD
- [x] Knob (drag, wheel, keyboard, double-click to reset)
- [x] Vertical fader with dB scale
- [x] Level meter (peak and hold)
- [x] Waveform canvas (mirrored peaks, clip-coloured)
- [x] MIDI clip preview (note bars)
- [ ] Piano-roll grid with velocity lane
- [ ] Step-sequencer row (16/32/64 steps, per-step velocity, pan and pitch)
- [ ] Spectrogram canvas (WebGL2)
- [ ] Automation lane with breakpoints and curves
- [ ] Device and rack containers with macro knobs
- [ ] XY pad, envelope editor (ADSR) and wavetable view
- [ ] Docking or splitting layout manager with saved workspaces

---

## 3. UX principles

1. **Essentials ↔ Pro mode** (topbar switch, `[x]` scaffolded). Essentials hides sends, routing, the CPU meter and advanced tools, and shows contextual tips. Pro shows everything. Your choice is remembered for each project and as a global default.
2. **Command palette** (`Ctrl+K`, `[x]` scaffolded). Every action is a command with a name, a shortcut and a menu location. Menus, the palette and shortcuts all read from one registry (`src/commands.ts`).
3. **Everything is undoable**, including mixer moves, plugin parameter changes and view changes you opt into. The History panel lets you jump to any step.
4. **Drag and drop everywhere**: files from Explorer to a track, browser items to a track or device chain, clips between tracks, effects between tracks.
5. **Hover help**: each control has a tooltip with its name and shortcut. An optional *Info view* (like Live's) explains the control under the pointer.
6. **Start screen**: recent projects, templates (Podcast, Voice-over, Beat, Song, Mastering, Sound design), "Open audio file" and "Record now".
7. **Keyboard-first** but never keyboard-only. Shortcuts can be remapped, with Audacity and Live preset keymaps for people switching over.
8. **Accessible**: real buttons and inputs, visible focus rings, ARIA roles on sliders and knobs, 4.5:1 text contrast, screen-reader announcements for transport state, and no colour-only status.

---

## 4. Architecture

```
┌──────────────────────── Tauri 2 window (WebView2 / WKWebView / WebKitGTK) ─────────────────────────┐
│ React 19 + TypeScript + Vite                                                                       │
│  views/ (Arrange, Session, Mixer, Editor, Piano roll, Channel rack*)  components/  state/ (zustand) │
│  Canvas2D now → WebGL2 for waveforms, spectrograms and big piano rolls                             │
└───────────────▲──────────────────────────────── IPC ───────────────────────────────▲───────────────┘
                │ commands (invoke) · events · Tauri Channels for meters/peaks       │
┌───────────────┴──────────────────────────── Rust core ─────────────────────────────┴───────────────┐
│ commands.rs ── project model (undo/redo, serde) ── engine control (lock-free queues)               │
│ engine/  audio thread: transport · graph · mixer · plugin slots · metronome · recorder             │
│ dsp/     EQ, dynamics, reverb, delay, filters, synths, sampler, time-stretch, pitch                │
│ io/      decode (symphonia, opus) · encode (WAV, FLAC, MP3, OGG, Opus) · resample · peaks         │
│ midi/    midir inputs, Standard MIDI Files · later: MIDI out, clock, MPE                           │
│ host/    CLAP and VST3 plugin hosting, one sandbox process per plugin                              │
└────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

### Key decisions
- **Engine in Rust, UI in the WebView.** The UI never touches audio buffers. It sends commands and receives meter and peak data at 30–60 Hz through Tauri Channels.
- **Lock-free communication** *(built in phase 1)*: transport state is atomics. Every project change compiles a new mixer `Graph` on the control thread (positions in samples, gains resolved, scratch buffers allocated) and sends it to the audio thread through an `rtrb` queue; replaced graphs travel back through a second queue so they are freed off the audio thread. Meters are `fetch_max` atomics read by a 30 Hz thread that streams them, with the playhead, over a Tauri Channel.
- **Undo** *(built in phase 1)*: snapshot-based in `soniq-project`. A project holds no sample data, so a snapshot is cheap; continuous gestures (a fader drag) coalesce into one step. Destructive edits create a new pool asset instead of overwriting one, so undo just points the clip back. Unreferenced audio is freed once no snapshot needs it.
- **Resampling** *(built in phase 1)*: a 32-tap polyphase windowed-sinc resampler of our own, run once per file on import or device-rate change. Warping (phase 4C) reads the file at a moving speed with cubic interpolation instead.
- **Tempo track** *(built in phase 4C)*: tempo points glide in a straight line per beat between them, two points at one beat make a jump, and the tempo holds before the first and after the last. The same map exists three times (beats ↔ seconds in `soniq-project` and the UI, beats ↔ samples in the engine) with closed-form conversions both ways. Unwarped audio keeps its seconds (its length in beats depends on where it sits); warped audio keeps its beats. Session clips loop at the song's starting tempo.
- **Instruments** *(built in phase 3)*: each MIDI track's instrument lives in the graph with every note its clips play, as sample-exact note-on/off events. When a project change rebuilds the graph, the new instrument adopts the sounding voices of the old one (voices are fixed arrays, so the swap allocates nothing) and notes that no longer exist are released. Live notes (MIDI keyboards, the computer keyboard, auditioning) reach the audio thread through their own lock-free queue. A seek, loop wrap or stop releases the notes the timeline started but leaves held keys alone.
- **Audio graph**: a pull-based DAG compiled to a flat process order, with sidechain inputs.
- **Multicore** *(built in phase 5C)*: a pool of worker threads (all cores but one, up to eight; or as set in Settings) shares each block's tracks with the audio thread. A dispatch hands out tracks one at a time through a single atomic (round, next job, job count), so a worker that wakes late only means the audio thread does more itself, and nothing locks or allocates. Each track's own sound and effects run in one job; when an effect listens to another track, the own sounds come first and the effects after. Group tracks run once their tracks have played into them; summing into groups, returns and the master stays on the audio thread in track order, so the mix is bit for bit what one core makes. Workers run at the audio thread's priority and spin briefly before sleeping between dispatches.
- **Live analysis** *(built in phase 5C)*: while the Meters panel is open, the audio thread copies the master (after its fader, without the click) into a lock-free queue; the 30 Hz meter thread feeds an `Analyzer` in `soniq-io` (EBU R128 momentary, short-term, integrated and loudness range from 100 ms steps and 0.1 LU histograms; 4× oversampled true peak; 300 ms RMS and correlation; an 8192-point spectrum in sixth-octave bands) and sends its reading with each meter frame.
- **Plugin delay compensation** *(built in phase 5B)*: worked out each time the graph is built. A track's path is its own devices' latency plus its group's; every track outside a group waits (a delay line after its effects) for the slowest path, and a group's tracks arrive already lined up. Returns wait for the slowest return, and the dry mix waits with them. Mixer automation is read that much earlier, so a fade lands on the sound it was drawn under. The graph's total latency is what recording adds to its compensation and what offline renders (export, freeze, render in place) render on past the end and cut from the start, so they come out in time with the song. Delay lines are carried over when the graph is rebuilt.
- **Sample format**: 32-bit float internally, with 64-bit summing on the master. Sample rates from 44.1 to 192 kHz. Projects keep their rate; the device is resampled if needed.
- **Drivers**: WASAPI shared and exclusive (Windows default), ASIO (cpal `asio` feature, needs the Steinberg ASIO SDK at build time), CoreAudio (macOS), ALSA and JACK/PipeWire (Linux).
- **Project format** *(built in phase 1)*: a `.soniq` file (pretty-printed, versioned JSON) with a `Samples/` folder beside it. Saving copies imported files into `Samples/` and writes renders and edits as 32-bit float WAV, so a project folder is self-contained. Writes go to a temp file and are renamed into place, so a crash mid-save never corrupts a project. Still planned: a zipped `.soniqz` for sharing, cached peak files, `backups/`.
- **Time-stretch and pitch** *(real-time warping built in phase 4C)*: our own warper in `soniq-dsp`, with no library: Beats plays each transient-to-transient piece at its own speed and fades it before the next hit; Tones overlaps WSOLA grains (about 43 ms, aligned by cross-correlation, kept across graph rebuilds); Repitch reads faster or slower like a turntable. For higher-quality stretching later, a permissive library such as Signalsmith Stretch (MIT) is the candidate; Rubber Band is high quality but GPL or commercial. **The licensing choice is an open decision**, see §9.
- **Plugins** *(CLAP built in phase 5A, VST3 in 5B)*: each plugin device's plugin runs in a sandbox process of its own (the app started again with `--plugin-sandbox`), so a crash or a hang cannot take the song down. Every engine block goes to the sandbox and back through shared memory, handed over with two Windows events; everything else (loading, parameters, state, the window) travels as JSON frames over its stdin and a private copy of its stdout. An effect that misses a block is passed by and an instrument is silent; after three misses in a row it is skipped until it catches up, and a sandbox that dies is noticed at once. The plugin's window is a top-level window of its sandbox, with the plugin's view inside. An export takes the plugin over (the live engine passes it by meanwhile) and starts it afresh. VST3 runs in the same sandbox behind the same commands: the `vst3` crate's bindings (MIT/Apache), a component and its controller kept in step (the controller hears what the processor changes, the processor what the editor edits), parameters normalised to 0…1 as VST3 has them, and the editor attached to the sandbox's window.
- **Freezing** *(built in phase 5B)*: a frozen track keeps its clips and devices but plays an asset rendered offline from the start of the song: the song copied with only that track heard, at unity gain, straight into a bare master. Its fader, pan, sends and mixer automation still apply live. The render lives in the pool like any other audio, so undo and saving need nothing new; flattening makes the render the track's only clip.
- **Testing**: DSP unit tests with golden files and null tests, fuzzing for decoders and project loading, `tauri-driver` WebDriver end-to-end tests, and a CI performance budget (see §8).

### Rust workspace
```
src-tauri/            ✔ Tauri app crate: commands, session (project + history + pool), files, autosave, MIDI, demo song
crates/soniq-engine   ✔ audio thread, transport, tempo map, graph, mixer, meters, devices, metronome, recorder, instruments in the graph
crates/soniq-io       ✔ decode, resample, peaks, edits, offline effects, tempo detection, encoders (WAV, FLAC, MP3, OGG, Opus)
crates/soniq-project  ✔ data model, serde, ops, undo/redo, tempo track, shared device catalog, device refresh on load
crates/soniq-dsp      ✔ instruments (synth, electric piano, sampler, drum rack, instrument rack), MIDI effects, audio effects, warping, no allocation in process()
crates/soniq-midi     ✔ MIDI inputs (midir), Standard MIDI File import/export (midly) · later: MIDI out, clock, MPE
crates/soniq-host     ✔ CLAP and VST3 hosting in sandbox processes (shared-memory audio, pipes for the rest), the sandbox itself, four test plugins
```
Dependencies and the engine/IO crates build at `opt-level = 2` even in dev, because unoptimised audio code cannot keep up in real time.

---

## 5. Feature specification

### 5.1 Audio engine & devices
- [x] **P0** Open the default output device, run a real-time callback and a lock-free transport (play, stop, seek, BPM, time signature)
- [x] **P0** Metronome click with an accented downbeat
- [x] **P0** List the input and output devices of every host API
- [x] **P0** Audio settings: choose the driver, output device, sample rate and buffer size, see output latency in ms; remembered across launches *(input choice comes with recording in phase 2)*
- [x] **P0** Mix tracks and play audio clips from the pool: sample-accurate clip playback, gain and balance pan with click-free smoothing, mute and solo, master fader, sample-accurate loop
- [x] **P0** Master safety limiter (Phase 7A)
- [x] **P0** Automatic device recovery when a device is unplugged or the default device changes (Phase 7A)
- [ ] **P1** ASIO and WASAPI exclusive mode for low latency; JACK and PipeWire on Linux
- [x] **P1** Plugin delay compensation across the whole graph: tracks, groups, returns and the master's chain; mixer automation, recording and offline renders follow it *(sends from inside a group and sidechain keys are not compensated yet)*
- [x] **P1** Multicore graph processing: tracks shared over a worker pool, bit-identical to one core; the number of cores in Settings
- [x] **P1** Freeze and flatten tracks; render in place
- [ ] **P2** Ableton Link and MIDI clock sync (send and receive)
- [ ] **P2** Real-time CPU usage for each track and device

### 5.2 Recording
- [x] **P0** Record mono or stereo from any input onto every armed audio track at once; each track picks its input channels (In 1, In 2, In 1/2, …), so a multi-input interface records several tracks in one pass
- [x] **P0** Input meters on armed tracks; the input opens only while a track is armed, so the system microphone indicator is honest
- [x] **P0** One-bar count-in (the playhead waits while it clicks); recording can also start on the fly during playback
- [x] **P0** Crash-safe recording: takes stream to disk and the WAV header is rewritten every 0.5 s; after a crash the next launch offers to import or discard the unfinished take
- [x] **P0** Input monitoring for each audio track: off (the default, so a laptop mic never howls), while armed, or always; played through the track's fader and pan
- [x] **P0** Punch in and out at the loop range, with one bar of pre-roll; recording stops by itself after the punch-out point
- [x] **P0** Recording latency compensation: takes move earlier by the round trip the driver reports, plus a manual offset in the audio settings; the file keeps the whole take so the clip edge can be dragged back
- [x] **P0** Monitoring when the input and output run at different sample rates, or on two devices whose clocks drift apart (Phase 7A)
- [x] **P1** Loop recording into take lanes and comping (swipe to comp), for audio (Phase 8 Part B)
- [x] **P1** MIDI recording onto every armed MIDI track at once (with count-in and punch); notes held by the sustain pedal last until it lifts; the new clip layers over what is already there
- [x] **P1** MIDI recording modes (Transport menu): a new clip, overdub into the clip under the playhead, or replace its notes in the recorded range; with the loop on, every pass is folded into the loop range
- [x] **P1** Capture MIDI (`Ctrl+Shift+C` or the Capture button): turns what you just played into a clip on the armed (or selected) MIDI tracks, even without recording; at its song position if the transport was running, else bar-aligned at the playhead
- [ ] **P1** Sound-activated recording and timer recording (Audacity)
- [ ] **P2** Retrospective audio recording buffer

### 5.3 Arrangement view (timeline)
- [x] **P0** Track list (audio and MIDI), ruler in bars and time, clips with waveforms and note previews, playhead, loop region, zoom HUD
- [x] **P0** Mute, solo and arm for each track; select a track to see its device chain
- [x] **P0** Delete clips and tracks (undoable)
- [x] **P0** Move clips (also to another track of the same kind), trim either edge (audio never runs past its file), split (Split tool or `Ctrl+E` at the playhead), duplicate (`Ctrl+D`), fade in and out with corner handles; snap to the grid, hold Alt to bypass it
- [x] **P0** Crossfades: where a clip overlaps the one before it, both crossfade with equal power across the overlap (shown as an X); a clip that sits entirely inside another is layered
- [x] **P0** Ripple edit; multi-clip selection; adjustable crossfade curves (Phase 7B)
- [x] **P0** Import audio from a file dialog or by dragging files from Explorer onto a track (at the drop position) or below the tracks (new tracks); files are resampled to the engine rate
- [x] **P0** Import MIDI files (dialog or drag from Explorer, at the drop position); new MIDI clips by double-clicking a MIDI track or with the Draw tool; double-click a MIDI clip to open the piano roll
- [x] **P0** Marker lane (Audacity labels): point markers and regions, added at the playhead (`Ctrl+M`), from the loop range (`Ctrl+Shift+M`) or by double-clicking the lane; drag to move, drag region edges, double-click to rename, Delete removes; undoable and saved with the project
- [x] **P1** Group tracks, coloured and foldable: Ctrl-click headers to pick tracks and press `Ctrl+G`; right-click a header to move a track into or out of a group, or ungroup (`Ctrl+Shift+G`); a group's lane shows its tracks' clips
- [x] **P1** Automation lanes under each track for volume, pan, sends and any instrument or audio-effect knob: click to add a point, drag to move it, double-click or right-click to delete; open them from a track header, the Automation tool (`A`), or "Show automation" on any knob's right-click menu
- [ ] **P1** Automation still to come: curved segments, line and curve drawing tools, selecting and moving many points
- [x] **P1** Tempo changes: a tempo lane under the markers (the Tempo button, or View › Tempo track) with points that glide or jump; the BPM field shows the tempo at the playhead and, with a tempo track, a new value moves the whole track; clicks, launch quantisation, LFO sync, the arpeggiator, recording, MIDI capture and export all follow it
- [ ] **P1** Time-signature changes
- [ ] **P1** Clip gain envelopes, reverse, clip-level pitch and stretch
- [ ] **P1** Arrangement sections (intro, verse, chorus) that you can move as blocks
- [x] **P2** Linked MIDI clips (FL patterns): duplicate linked (`Ctrl+Alt+D`) and every copy shares its notes, edit one and all change; unlink to go separate (trimming the start or splitting unlinks too)
- [ ] **P2** Video track for scoring, with frame-accurate sync

### 5.4 Session view (clip launcher)
- [x] **P1** Grid of scenes × tracks with clip slots and scene launch buttons; session clips are clips of their own (edit them in the piano roll or audio editor), made by double-clicking an empty slot on a MIDI track or by putting a copy of an arrangement clip into a slot; add scenes
- [x] **P1** Launch clips and scenes with quantisation (none, 1/16 … 8 bars), sample-accurately; clips loop; a launched or stopped track leaves the arrangement until "Back to arrangement"; launching starts the transport and stopping it stops the session
- [x] **P1** Follow actions: after a number of bars, play the next, previous, first, last, any or any other clip, play it again, or stop
- [ ] **P1** Launch modes (trigger, gate, toggle, repeat), legato
- [x] **P1** Record into empty slots; write a scene into the arrangement (Phase 8 Part B)
- [x] **P1** Arrangement record of a live session performance: press record and every clip the session plays is written into the arrangement (one copy per loop), over what was there
- [ ] **P2** Clip launching from MIDI controllers (Launchpad-style grid mapping)

### 5.5 Audio editor (Audacity replacement)
- [x] **P0** Large stereo waveform editor with drag-to-select time ranges
- [x] **P0** Destructive edits on a selection of real samples, with undo: Normalize (to −1 dBFS), Amplify +3 dB (warns about overs), Fade in, Fade out, Reverse, Silence
- [x] **P0** Cut, copy, paste (at the cursor or over the selection), delete and trim to selection, with optional zero-crossing snapping (on by default) so edits never click
- [x] **P0** Loudness normalisation to a LUFS target (BS.1770 integrated loudness; −14, −16, −19 or −23)
- [x] **P0** Duplicate and split new inside the editor (Phase 7B)
- [x] **P0** Zoom down to single samples (Ctrl + wheel, buttons, zoom to selection, an overview strip to scroll) with exact min/max waveforms from Rust at every level, and a Draw tool that redraws samples by hand
- [x] **P0** Effects, each with a dialog: Noise Reduction (profile-based spectral gating), Click Removal, Remove DC Offset, High-pass and Low-pass (Butterworth, 12–48 dB/oct), Bass and Treble, 10-band Graphic EQ, Compressor (soft knee, make-up gain), look-ahead Limiter, Truncate Silence, Change Tempo and Change Pitch (WSOLA time-stretch), Change Speed, Echo, Reverb (Freeverb), Normalize, Loudness Normalize, Amplify
- [x] **P0** Effects: EQ filter curve, Repair, Auto Duck (Phase 7B)
- [x] **P0** Generators: tone and chirp (sine, square, sawtooth, triangle, band-limited), noise (white, pink, brown), DTMF, silence; into the clip at the cursor or as a new clip at the playhead
- [x] **P0** Generators: rhythm track (Phase 7B)
- [x] **P0** Analysis: plot spectrum, contrast (WCAG audio), find clipping, beat finder, label sounds (Analyze menu; Phase 7 Part C)
- [x] **P0** Preview for every effect: up to 10 s of the selection is processed and played through the engine, without changing anything
- [ ] **P1** Real-time (non-destructive) effect stacks on clips
- [x] **P1** Spectrogram view (log, mel or linear) with time-and-frequency selection and spectral editing: delete a band, lower it by 12 dB, or keep only the band
- [ ] **P1** Spectral healing (fill a deleted band from its surroundings)
- [x] **P1** Macros (four built in, plus your own: named effect chains with editable steps) applied to a clip, and batch processing of many files into a folder in any export format
- [ ] **P1** Multiple clips per track with clip handles; clip stretching
- [ ] **P2** AI-assisted tools, run locally: speech denoise, de-reverb, stem separation (ONNX Runtime via the `ort` crate), transcription for editing speech by text

### 5.6 MIDI, piano roll and step sequencer (Live + FL)
- [x] **P0** MIDI clips rendered as note previews, and played by the track's instrument, sample-accurately
- [x] **P0** Piano roll: draw (double-click or the Draw tool), select (click, Shift-click, marquee, Ctrl+A), move, copy (Ctrl-drag), resize either edge, delete (Delete, right-click, Erase tool), velocity lane (drag a bar or paint across), quantise with strength, swing and note ends (Ctrl+U), legato, transpose (arrows; Shift for octaves), nudge and lengthen with the arrows, duplicate (Ctrl+D), cut/copy/paste, clip length handle, notes auditioned while editing, export a clip as MIDI
- [x] **P0** Scale awareness: pick a key and one of 12 scales; in-key rows highlighted (root stronger); fold to the scale or to the keys in use; snap drawn, moved and transposed notes to the scale
- [x] **P0** Play MIDI live: hardware keyboards (every input, hot-plugged, switchable in settings) and the computer keyboard (A–' as two octaves, Z/X octave, C/V velocity) play the armed MIDI tracks, or the selected one; a MIDI activity light
- [x] **P1** FL-style tools: ghost notes from the track's other clips (and drums beside drums), chop, arpeggiate, strum, flam, humanize (timing and velocity), each a dialog with remembered settings and one undo step
- [ ] **P1** FL-style tools still to come: slide and portamento notes, "riff machine"
- [x] **P1** MIDI transformations and generators (Live 12 style): chords (stack a chord on each note, in key), span, reverse, invert, glue; Euclidean rhythm and seed (random notes in key) generators
- [ ] **P1** Transformations still to come: ornament, recombine
- [x] **P1** Step sequencer (a mode of the piano roll): 16–64 steps, drum pads or the notes in use as rows, paint steps on and off, per-step velocity, length and shift lanes, swing, fill every 2/4/8, clear a row
- [ ] **P1** Step sequencer still to come: per-step pan and pitch, a graph editor
- [x] **P1** MIDI effects in the chain, before the instrument: arpeggiator (seven styles, 1/4 to 1/32 with triplets, gate, octaves; on the song grid while playing, on its own clock while stopped), chord (up to three intervals), scale (the song key or its own), note length, random (shifts some notes, optionally in key), velocity
- [ ] **P1** MPE support (per-note pitch bend, slide, pressure) in the editor and engine — **deferred**: it needs per-note expression in the note model, the engine's voices and the piano roll, so it moves to a later phase
- [x] **P1** MIDI learn: right-click any device knob, a rack macro, or a track's volume or pan in the mixer, choose Learn MIDI controller and move a knob, fader or button; controllers move what they are mapped to (batched, one undo step per gesture); mappings are saved with the song and listed in Settings › MIDI
- [ ] **P1** Controller auto-mapping for common keyboards; MIDI learn for transport and clip launching
- [ ] **P2** Chord track and chord detection from audio

### 5.7 Instruments
- [x] **P0** **Sampler**: classic, one-shot and loop modes, root note and tune, start and end points, ADSR, filter, velocity sensitivity; load a sample from a dialog or by dropping a file on it
- [x] **P0** Sampler slicing: equal pieces or at the sample's transients (with a sensitivity), one key per slice from the root up, slice markers on the waveform (click one to hear it), and "To clip" writes a MIDI clip that plays the slices in their original timing
- [x] **P0** Sampler LFO on pitch, filter cutoff or volume (rate and amount)
- [ ] **P1** Sampler still to come: slicing on the beat grid and by hand
- [x] **P0** **Drum Rack**: 16 pads on the General MIDI drum map, each with a built-in synthesized drum (kick, snare, clap, rim, toms, hats, crash, ride) or a dropped-in sample, its own tune, level, pan and decay, and choke groups (closed hat cuts the open one)
- [x] **P0** Drum Rack: 64 pads in four pages (the kit, General MIDI percussion to note 81, then empty pads for samples), audio effects per pad, and Send A and B per pad into the rack's two returns, each with its own effects (Phase 7 Part D)
- [x] **P0** **Synth**: two oscillators plus a sub, saw/square/triangle/sine (band-limited), unison with spread, a 12/24 dB low-pass, band-pass and high-pass filter with drive, two ADSR envelopes, an LFO on the cutoff and vibrato, poly, mono and legato with glide
- [ ] **P1** Wavetable synth with a modulation matrix
- [x] **P1** A simple electric piano (two-operator FM tines, tremolo)
- [ ] **P1** FM synth (4-operator) and an organ
- [x] **P1** **Instrument Rack**: chains of instruments layered or split by key and velocity zones, each with level and pan; eight macros, each turning any chain parameters (right-click a knob to map it; log parameters sweep by octave); voices carry across edits
- [ ] **P1** Instrument Rack still to come: MIDI and audio effects inside chains, a chain selector, macro ranges in the panel, racks inside racks
- [ ] **P2** Granular sampler; physical-modelling plucks and drums

### 5.8 Effects (built-in, all real-time)
- [x] **P0** Device chain UI with on/off switch, knobs, add from the browser and remove (UI)
- [x] **P0** EQ (8 bands: cuts at 24 dB/oct, shelves, bells, notches; a curve editor), compressor (soft knee, dry/wet, sidechain), gate (hold, range, sidechain), limiter (no look-ahead, so no latency), reverb (algorithmic, with pre-delay and damping), delay (free or synced, ping-pong, filtered feedback), chorus, flanger and phaser, saturator (soft, hard, tube), utility (gain, pan, width, mono, phase), and an auto filter
- [x] **P0** A live spectrum in the EQ, gain-reduction meters on the compressor, gate, glue, limiter and de-esser (Phase 7 Part C)
- [x] **P0** Linear-phase mode in the EQ (Phase: Minimum or Linear; Phase 8 Part A)
- [x] **P1** Glue (bus) compressor (fixed ratios and attacks, auto release, range, clip), three-band multiband dynamics (Linkwitz-Riley crossovers that sum flat), de-esser (turns down the highs only or the whole signal; listen to what it hears), transient shaper, auto-pan (free or synced, three shapes, phase), redux (bit depth and sample-rate reduction), convolution reverb (five synthetic spaces, FFT convolution), pitch shifter (±24 semitones, two crossfading taps) and a vocoder (8–24 bands, formant shift; noise, saw or another track as the carrier); auto-filter came in phase 4A
- [ ] **P1** Effects still to come: convolution with your own impulse responses, a fourth multiband band and per-band solo
- [ ] **P1** Audio effect racks (parallel chains, macros), dry/wet on every device
- [ ] **P2** Sidechain anywhere (any device can listen to any track), spectral effects, Gross Beat-style time and volume gating

### 5.9 Mixer & routing
- [x] **P0** Channel strips: fader, pan, mute, solo, arm, meters, inserts list, sends (UI)
- [x] **P0** Real gain, pan and summing in the engine; master bus with its own effect chain
- [x] **P0** Return (send) tracks with pre/post sends, effects, faders and meters
- [x] **P1** Sidechain routing: a compressor or gate on any track, return or the master listens to any track
- [x] **P1** Group tracks: their tracks play through the group's effects and fader (sends still go straight to the returns); soloing a track plays its group, soloing a group plays its tracks
- [ ] **P1** Bus tracks, any-to-any routing, groups inside groups, external I/O
- [ ] **P1** Crossfader (DJ-style assignment, from Live)
- [x] **P1** Mixer snapshots and A/B compare: saved mixes (faders, pans, mutes, solos, sends, the master) to recall, update, rename and delete; A/B keeps the mix as A, tries changes as B and switches between them
- [ ] **P2** Surround and Atmos-ready bussing (stretch goal)

### 5.10 Automation & modulation
- [x] **P1** Automation lanes in the arrangement, played every 64 samples; devices retune without allocating, so a moving knob never glitches
- [ ] **P1** Envelopes inside clips
- [ ] **P1** Automation modes: read, write, touch, latch
- [x] **P1** LFO (six shapes, free or synced to the song, phase) and envelope-follower modulators that move a knob of another device on their track, around where it is set (or where its automation is)
- [ ] **P1** Modulators still to come: targets on other tracks and inside rack chains, several targets per modulator
- [ ] **P2** Macro variations and snapshots with morphing

### 5.11 Warping, tempo and pitch
- [x] **P0** Tempo detection and following the project tempo: Warp (the clip bar under a selected audio clip, the session clip panel, or Clip › Warp to project tempo) finds the clip's own tempo (a loop of up to 16 bars gets the one that makes it whole bars) and from then on it keeps its beats on the song's, through tempo changes too; without a steady beat it takes the song's tempo where it starts. The clip's tempo can be typed, halved or doubled. Detection runs when you warp, not on import, so a song never changes by itself
- [x] **P1** Warp modes: Beats (drums and loops), Tones (melodies and voices), Repitch (turntable)
- [ ] **P1** Warp markers; texture and complex modes; detection that tells a dotted rhythm from the beat (a dotted bass line can read as ⅔ of the tempo)
- [x] **P1** Slice to a new MIDI track: a Sampler holding the clip, cut at its transients or every 1/16, 1/8 or beat, and a MIDI clip in the audio's place that plays the slices where they were (Drum Rack slicing to come)
- [ ] **P1** Audio-to-MIDI (melody, harmony, drums)
- [ ] **P2** Pitch editing for vocals (note blobs, Melodyne-lite)

### 5.12 Browser & library
- [x] **P0** Browser panel with search, category chips and items; click an effect to add it to the selected track (UI)
- [x] **P0** Index folders of samples and presets; tags; favourites; recent items (Browser › Library; Phase 7 Part D)
- [x] **P0** Audition with preview playback synced to the project tempo and key (Phase 7 Part D)
- [ ] **P1** Similarity search ("find similar samples") using local audio embeddings
- [x] **P1** User presets for every built-in device, saved as plain files (the book button on a device; Documents › Soniq Studio › Presets; Phase 7 Part D)
- [ ] **P2** Pack manager for bundled and downloadable content

### 5.13 Plugins
- [x] **P1** CLAP hosting of instruments and effects: on tracks, returns and the master; the controls on a plugin's panel can be automated, modulated and mapped to MIDI; its state is saved with the song; its own window opens from the panel, and turning a control there moves the panel's knob (one undo step when let go)
- [ ] **P1** CLAP note effects; sidechain and extra audio ports (they get silence for now)
- [x] **P1** VST3 hosting of instruments and effects, everything CLAP has: the standard VST3 folders, single-file and bundle modules, state, the editor, edits in the editor reaching the song, latency
- [x] **P1** Plugin scanner with a blocklist (each file is opened once in a sandbox; one that crashes or hangs is skipped until "Look again"), the standard CLAP folders plus your own, results cached until a file changes; one sandbox process per plugin; crash recovery ("Start again" from the state saved with the song)
- [ ] **P2** LV2 on Linux; AU on macOS
- [ ] **P2** Soniq's own devices also exported as CLAP plugins (shared DSP crate)

### 5.14 Metering & analysis
- [x] **P0** Peak meters for each track and the master, streamed from the engine (post-fader, with peak hold and a master clip indicator)
- [x] **P0** True-peak and RMS meters, clip indicators: the Meters panel shows each channel's true peak (4× oversampled) and RMS, and the highest true peak, flagged over −1 dBTP
- [x] **P0** Integrated loudness measurement (BS.1770-4 K-weighting with absolute and relative gating; verified against the EBU −23 LUFS reference tone)
- [x] **P0** Live loudness meter: short-term and momentary LUFS, integrated, LRA, true peak, against a target (streaming −14, Apple Music and podcasts −16, EBU R128 −23, film −24), with 20 s of history
- [x] **P1** Spectrum analyser and correlation meter *(phase scope and stereo vectorscope still to come)*
- [x] **P1** Loudness targets on export and in the editor: streaming (−14), podcast (−16), audiobook (−19, editor) and broadcast (−23 LUFS)

### 5.15 Import / export
- [x] **P0** Import WAV, AIFF, FLAC, MP3, OGG Vorbis, M4A/AAC/ALAC and CAF (Symphonia)
- [x] **P0** Import Opus (decoded with the same libopus as export; sample-exact length)
- [x] **P0** Import MIDI files (type 0 and 1; one track per part, drums onto a Drum Rack, pianos onto the Electric Piano; an empty song takes the file's tempo and time signature)
- [x] **P0** Export a mixdown (whole song or loop region, faster than real time) to WAV 16/24-bit or 32-bit float and FLAC 16/24-bit, with TPDF dither, an optional sample-rate change and a loudness target that never pushes peaks over −1 dBFS; FLAC files are sample-exact in length
- [x] **P0** MP3 (LAME, 128–320 kbps), Ogg Vorbis (quality 2–10) and Opus (64–256 kbps) export
- [x] **P0** Export each region, or each track as a stem (alone, at its own volume and pan), into a folder
- [x] **P1** Tags in every format: title, artist, album, year, genre, track number, comment (ID3v2.4, Vorbis comments, FLAC VORBIS_COMMENT, RIFF INFO)
- [ ] **P1** Cover art; gapless MP3 (LAME tag), so encoder padding is trimmed on playback
- [x] **P1** Export MIDI: the whole song (one track per MIDI track, drums on channel 10, a General MIDI program per instrument) or one clip
- [ ] **P1** Export a project as a shareable `.soniqz` bundle
- [ ] **P2** Import Ableton Live sets (audio and MIDI clips, tempo) and FL projects at a basic level; import Audacity `.aup3` projects (they are SQLite files)

### 5.16 Project management & safety
- [x] **P0** New, open, save and save as; "unsaved changes" prompts on new, open and closing the window; the open demo song
- [x] **P0** Recent projects and templates: a start page with five templates (Empty song, Beat, Band recording, Podcast, Audio editing) and your own (a song saved without its clips and audio), the last twelve songs opened or saved, the demo and the lessons
- [x] **P0** Autosave every 30 s plus crash recovery on next launch (Restore or Discard)
- [x] **P0** Undo and redo of every project change (200 steps), with labels in the tooltips
- [x] **P0** History panel: every step that can be undone or redone, by the undo button (`Ctrl+H`); a click goes back, or forward, to just after it
- [x] **P1** Collect and save: imported files are always copied into `Samples/`; missing files are flagged on load
- [x] **P1** Find missing files: search a folder and the folders in it by file name, or point at each file; offered when a song opens with audio missing
- [ ] **P1** Project versions (snapshots) with notes; compare and restore
- [ ] **P2** Cloud-folder friendly (no lock files that break OneDrive or Dropbox sync)

### 5.17 Nice-to-have & delight features (P2)
- [x] Podcast mode: per-speaker tracks, auto-leveling, ducking music under voice, chapter export (View › Podcast tools; Phase 6 Part B)
- [x] "Fix my audio" one-click assistant: denoise, de-click, normalise loudness, trim silence (and DC, rumble; Phase 6 Part B)
- [ ] Stem separation of any audio clip (vocals, drums, bass, other)
- [x] Sketch mode: a minimal UI that is one big record button and a loop (Phase 6 Part B)
- [x] Theme editor: accent picker, track colour sets, compact or comfortable density (Settings › Appearance; Phase 6 Part B)
- [x] Tutorials inside the app (interactive lessons, like Live's built-in lessons): a card points at the control each step needs and moves on once it is done; three lessons so far
- [ ] Touch and pen support on Windows tablets, using the 44 px touch floor
- [ ] Localisation (English and German first; strings externalised from day one)

---

## 6. Roadmap

### Phase 0 — Foundation (this scaffold) ✅
- [x] Tauri 2 + React 19 + TypeScript + Vite project
- [x] Graphiq design tokens (dark and light, accent) and base components in CSS
- [x] App shell: topbar with menus, command search, Essentials/Pro switch, theme toggle, undo/redo, export; transport bar; tool rail; browser; tabbed stage with four views; device chain; zoom HUD; toasts
- [x] Arrange, Session, Mixer and Editor views populated with a demo project
- [x] Command registry driving the menus, the palette and the shortcuts
- [x] Rust engine: cpal output stream, lock-free transport, metronome, device listing, Tauri commands
- [x] UI ↔ engine bridge with a browser-only fallback (`npm run dev` works without Tauri)

### Phase 1 — Engine core & playback ✅
- [x] Workspace split into `soniq-project`, `soniq-io` and `soniq-engine` (the other crates from §4 arrive with their phases)
- [x] Project model in Rust (serde), sent to the UI after every change; the UI reconciles it so unchanged parts keep their identity and do not redraw
- [x] Undo and redo in the Rust project model (snapshots with gesture coalescing)
- [x] Decode and import audio files (Symphonia), audio pool, resampling to the engine rate, waveform peaks streamed to the UI as binary
- [x] Audio clip playback, track gain and pan, mute and solo, master bus, sample-accurate loop
- [x] Meters and playhead streamed to the UI over a Tauri Channel
- [x] Audio settings dialog: driver, output device, sample rate and buffer size, saved for next launch
- [x] Save and load `.soniq` projects with a `Samples/` folder; autosave; crash recovery
- [x] Demo song rendered to real audio on startup, so playback works before instruments exist
- [x] 22 Rust tests: renderer (placement, mute, pan, loop wrap), resampler (level, duration, anti-aliasing), WAV round trip, edits, ops, undo history
- [ ] Deferred: peak files cached on disk with mipmap levels (peaks are computed on load, one level), master safety limiter, automatic device recovery, recent-projects list

### Phase 2 — Recording & editing → **Audacity replacement milestone**

**Part A ✅**
- [x] Recording: multitrack from per-track input channels, count-in, crash-safe takes with recovery
- [x] Clip editing in the arrangement: move, trim, split, duplicate, fades, snapping
- [x] Audio editor: cut, copy, paste, delete, trim, zero-crossing snapping
- [x] Loudness normalisation (BS.1770) in the editor and on export
- [x] Export WAV (16/24/32f) and FLAC, dithered, with loudness targets
- [x] Rust tests (36 in total) now also cover clip ops, fades and count-in in the renderer, offline render, loudness reference tone, cut and paste, zero crossings, WAV and FLAC round trips

**Part B ✅**
- [x] Spectrogram view (log, mel, linear) and spectral editing of a time-and-frequency selection
- [x] Noise reduction (profile-based) and click removal
- [x] Generators and 18 offline effects from §5.5, each with a parameter dialog and a preview through the engine
- [x] Markers and regions; export each region or each track (stems); macros and batch processing
- [x] MP3, Ogg Vorbis and Opus export, and tags in every format; Opus import
- [x] Punch in/out with pre-roll, input monitoring, recording latency compensation, automatic crossfades, sample-level zoom with a draw tool
- [x] Rust tests (67 in total) now also cover filters, STFT reconstruction, the spectrogram, spectral band removal, noise reduction, click detection (including no false alarms on a sawtooth pad), compressor, limiter, truncate silence, time-stretch and pitch shift, generators, MP3/OGG/Opus round trips, tags, crossfades, preview, monitoring, markers and take placement
- [ ] Not tried on real hardware yet: input monitoring, punch recording and the latency offset (they need a microphone or interface)

### Phase 3 — MIDI, instruments & sequencing

**Part A ✅**
- [x] Instruments in the engine (new `soniq-dsp` crate): Synth, Electric Piano, Sampler, Drum Rack with a built-in kit; MIDI clips play sample-accurately; voices carry across project changes
- [x] MIDI inputs (new `soniq-midi` crate, midir), hot-plugged and switchable; the computer keyboard as a MIDI keyboard; MIDI recording onto armed MIDI tracks
- [x] Piano roll with scale tools, quantise (strength, swing), velocity lane, legato, transpose, copy and paste
- [x] Import and export Standard MIDI Files; samples for the sampler and drum pads (dialog or drag and drop), saved with the project
- [x] Instrument panels: grouped controls, named choices, drum pads, sample slot; devices saved by older versions gain new parameters on load
- [x] The demo song's drums, bass and keys are MIDI now, played by the Drum Rack, Synth and Electric Piano
- [x] Rust tests (96 in total) now also cover every instrument (pitch, release, sustain pedal, voice stealing, mono fallback, choke, sample speed, one-shot and loop), sample-accurate notes in the renderer, live notes while stopped, releasing clip notes on a jump, voice handover on graph rebuilds, note ops, drum pad parameters and device refresh, MIDI parsing, MIDI file round trips and MIDI take placement
- [ ] Not tried yet: live instrument playback through speakers, MIDI recording and a hardware MIDI keyboard. The test machine had no active audio output (and no MIDI keyboard), so instruments were checked through offline export, and live notes as far as the backend

**Part B ✅**
- [x] MIDI effects in the chain: arpeggiator, chord, scale, note length, random, velocity (engine, sample-accurate, before the instrument)
- [x] Piano roll: ghost notes; note tools (chop, arpeggiate, strum, flam, humanize, chords, span, reverse, invert, glue) and generators (Euclidean rhythm, seed)
- [x] Step sequencer mode in the piano roll, and linked MIDI clips (FL patterns)
- [x] Capture MIDI, and MIDI recording modes (new, overdub, replace; loop recording folds passes into the loop)
- [x] Sampler slicing (equal or transients) with "To clip", and a sampler LFO
- [x] Instrument Rack: chains, key and velocity zones, eight macros
- [x] MIDI learn: device parameters, rack macros, track volume and pan; mappings saved with the song
- [x] Rust tests (118 in total) now also cover every MIDI effect, the arpeggiator in both clock modes, MIDI effects in the renderer, linked clips, loop and overdub recording, transient detection, slices (equal and by transients), the sampler LFO, rack zones and voice handover, macro mapping, and controller mapping and values
- [ ] Deferred: MPE (see §5.6); drum pad chains and sends (§5.7)
- [ ] Not tried yet with hardware: a MIDI controller's knobs (mappings were tested with simulated controller messages through a debug-only `midi_simulate` command) and a MIDI keyboard. Live checks in the desktop app ran with every track muted; instruments, slices and racks were checked through offline exports
- Note: in debug builds WebView2 may claim `Ctrl+Shift+C` for its DevTools inspector; release builds have DevTools off, and the Capture button always works

### Phase 4 — Production workflow → **Live Lite replacement milestone**

**Part A ✅ — mixing**
- [x] Real-time audio effects in the engine (`soniq-dsp`'s `fx` module): 8-band EQ, compressor, gate, limiter, reverb, delay, chorus, phaser/flanger, auto filter, saturator, utility; rebuilt effects keep their tails and envelopes, so turning a knob never clicks
- [x] Effect chains on every track (after the instrument, before the fader), on the returns and on the master
- [x] Return tracks heard at last: sends before or after the fader (a Pre/Post switch on each return), with their own effects, fader, pan, mute and meters
- [x] Sidechains: a compressor or gate listens to another track's own sound (before its effects and fader, so a muted kick still ducks)
- [x] Panels: an EQ curve editor (drag a band, the wheel sets Q, double-click adds or removes a band), compressor and gate curves, a sidechain picker; return and master chains open from the mixer
- [x] Rust tests (137 in total) now also cover every audio effect (EQ curves, compression ratio, sidechain ducking, gate, limiter ceiling, reverb decay, echo timing, ping-pong, modulation, saturation, utility), inserts and the master chain in the renderer, pre- and post-fader sends, sidechains across tracks, and chains on returns and the master
- [ ] Not tried yet: listening on speakers. Effects were checked through offline exports (a −12 dB utility lowers a stem by exactly 12 LU, a −3 dB master limiter holds the mix at −3.0 dBFS, a pre-fader send still reaches the reverb with the fader down)
- Note: stems are dry: each track through its own effects, without the returns or the master chain (exporting the returns as stems of their own comes later)

**Part B ✅ — performance and automation**
- [x] Session view clip launching: session clips are clips of their own in a scene's slot (older songs' named slots become copies on load), launched on the quantisation grid sample-accurately, looping, with follow actions; scene launch, stop per track and for all, "Back to arrangement", add scenes; launch states, queued blinking and loop progress come from the engine
- [x] Arrangement recording of the session: with session clips in the song, record works without an armed track and writes what the session played into the arrangement, over what was there (`soniq_project::record_slot`)
- [x] Automation lanes (volume, pan, sends, instrument and audio-effect parameters) and LFO and envelope-follower modulators; the engine applies them every 64 samples, and devices retune in place (`configure`) instead of being rebuilt
- [x] Group tracks: grouping, folding, routing through the group's effects and fader, solo that follows groups; group strips and routing labels in the mixer
- [x] Rust tests (152 in total) now also cover retuning every effect and instrument in place, rack macros on a playing chain, automation and modulators in the renderer, group routing, launch quantisation, follow actions, looping MIDI session clips that survive edits, transport stop, and writing a session clip into the arrangement
- [ ] Not tried yet: listening on speakers, and launching from a MIDI controller. Live checks in the desktop app ran with every track muted (launch, scene launch on the bar, a follow action after one bar, session recording into the arrangement); automation, the LFO and groups were checked through offline exports (a −60 dB volume lane silences the Pad; an LFO on a Utility lifts its peak by 5.4 dB; a group at −12 dB lowers its tracks)
- Note: a track's sends go straight to the returns, also when it is in a group; muting a group leaves its tracks' reverb sends playing, as in Live

**Part C ✅ — time**
- [x] Tempo track: a tempo lane under the markers with gliding and jumping points; one tempo map (`soniq_project::TempoMap`, `soniq_engine::TempoMap`, `src/state/tempo.ts`) behind clip lengths, trims and splits, the metronome, launch quantisation, synced LFOs and arpeggios, recording, MIDI capture, export, the BPM field and the playhead
- [x] Warping: audio clips follow the song's tempo in three modes (Beats, Tones, Repitch); a warped clip keeps its beats when the tempo changes, an unwarped one keeps its seconds; warpers keep their state across graph rebuilds, so editing never glitches a playing clip
- [x] Tempo detection (`soniq_io::detect_tempo`): onset autocorrelation across 60–200 BPM with the bar folded in, refined where the same peak recurs a few beats on, ignoring slow swells; it says so when there is no steady beat; loops of up to 16 bars snap to whole bars
- [x] Slice to MIDI: a new MIDI track under the clip with a Sampler cut at the transients (or every 1/16, 1/8 or beat) and a MIDI clip that plays the slices in place; one undo step
- [x] Nine new real-time effects (§5.8): glue compressor, multiband dynamics, de-esser, transient shaper, auto-pan, redux, convolution reverb, pitch shifter and vocoder; the vocoder's carrier can be another track (its sidechain), pink noise or a saw
- [x] UI: the tempo lane, a clip bar for the selected audio clip (Warp, mode, its own BPM, ½ and ×2, find the tempo again, Slice to MIDI), the same controls in the session clip panel, a warp mark on warped clips, Clip › Warp to project tempo and Clip › Slice to new MIDI track, View › Tempo track, and a carrier picker on the vocoder
- [x] Rust tests (175 in total) now also cover the tempo map both ways (ramps and jumps), warped and unwarped clips through tempo changes (in the model and in the renderer), launch quantisation across a tempo change, the three warp modes (Beats keeps hits in place, Tones keeps the pitch, Repitch shifts it), tempo detection (click tracks, loops, long recordings, pads and silence) and every new effect
- [ ] Not tried yet: listening on speakers. Checked in the desktop app through offline exports with every other track muted: four bars of the demo drums exported and re-imported are found to be 112 BPM; at 140 BPM the warped loop ends on beat 16 (silent after it) while unwarped it runs to beat 20; with a tempo track (140, a jump to 90, a ramp to 160) the warped loop still ends on beat 16; the demo pad has "no steady beat"; slicing gives 27 slices at the transients and 32 at 1/8, and the sliced track plays; each new effect renders and changes the drums' level as expected
- Note: session clips loop at the song's starting tempo, whatever the tempo track does later; the convolution reverb's wet sound is one block (512 samples, about 11 ms) late; changing a convolution space or length rebuilds it (everything else on it can be automated)

### Phase 5 — Plugins & pro polish

**Part A ✅ — CLAP plugins**
- [x] New `soniq-host` crate: CLAP loading (`clap-sys`), the sandbox process, the app's side of it, and a `Processor` the engine runs like any other device (`soniq_dsp::External`: an instrument gets each block with its notes at their sample offsets, an effect processes in place)
- [x] Sandboxing: one process per plugin device, audio through shared memory, a hung plugin passed by after three missed blocks, a crash noticed at once (also mid-export); the song plays on
- [x] Scanner: the standard CLAP folders (`CLAP_PATH`, Common Files, Local AppData) and your own (Settings, or Add a folder… in the browser); each file opened once in a sandbox and cached; files that crash or hang are listed with why
- [x] Plugin devices (`plugin` type with a `PluginRef` and the saved state): the browser's Plugins tab adds them; an instrument plugin replaces the track's instrument; the panel shows the plugin's status, Open window, Controls… (pick which parameters are knobs; the first eight automatable ones to start), the plugin's own text for each value, and its latency
- [x] What a plugin does by itself reaches the song: a control turned in its own window moves the knob and becomes one undo step when let go; preset and other state changes mark the song as changed, so they are saved
- [x] State: fetched from every plugin when the song is saved (and autosaved), loaded back when it opens; plugins stay running while the song is open, so undoing a removal brings back the same plugin as it was
- [x] Also: popovers re-place themselves when their content grows (no more lists cut off at the bottom of the window), and dialogs taller than the window scroll between a pinned header and footer
- [x] Rust tests (185 in total) now also cover the protocol, scanning a file, an effect and an instrument through real sandbox processes, parameter text, state carried to a new instance, a plugin's own knob gesture, a crash (live and in an export), a hang that does not stall the engine, and an export taking a plugin over
- [ ] Not tried yet: third-party plugins (none are installed on the test machine) and listening on speakers. Checked in the desktop app with two CLAP plugins built for the tests (Soniq Test Gain and Soniq Test Synth), through offline exports with every other track muted: the scan finds both; −12 dB on the gain plugin lowers the Pad by exactly 12 dB and the plugin shows "−12.0 dB"; its window opens as "Soniq Test Gain · Pad"; the plugin raising its own gain by 1 dB moves the knob to −11 with one undo step; the saved song reopens with the plugin at −11 dB; a crash mid-export finishes the export in under a second with the plugin passed by, and "Start again" brings it back; the synth replaces Keys' Electric Piano and plays; stopping the app ends every sandbox
- Note: the sandbox and its shared memory use Win32, so plugins run on Windows only for now; latency was shown but not compensated until Part B; only controls on the panel can be automated or mapped, while everything else a plugin holds is in its saved state (and not undoable)

**Part B ✅ — VST3 and the engine**
- [x] VST3 hosting in the same sandbox (the `vst3` crate): modules in the standard folders (Common Files\VST3, Local AppData\Programs\Common\VST3) and yours, as single files or bundles; a component with its own controller or one object for both, connected to each other; stereo main buses; parameters (stepped ones snap), the plugin's text for each value, state (component and controller), the editor inside the sandbox's window (resizable when the plugin allows), latency changes followed. The CLAP side now shares the host callbacks (`host.rs`), and the sandbox drives either format through one `Instance`. The browser and the device's tag say which format a plugin is
- [x] Plugin delay compensation across the whole graph (see §4): the engine chip shows it (for example "+5.8 ms PDC"), recording lines up with it, and exports come out in time with the song
- [x] Freeze (Track menu or the track header's menu): the track's clips rendered through its instrument and effects, pre-fader, from the start of the song until the last clip ends and its tail dies away (up to 10 s); it plays that while its devices rest (no CPU, no latency). The header and device chain say it is frozen; Unfreeze brings the devices back with any changes made meanwhile. Flatten makes it an audio track playing the render
- [x] Render in place: Clip › Render to a new audio track renders one clip through its track's devices onto a new audio track below it (with the source's fader, pan, sends, group and mixer automation) and mutes the source track, as one undo step
- [x] Also: Effect › Browse plugins opens the browser on its plugins
- [x] Rust tests (195 in total) now also cover a VST3 module scanned, an effect processing and reporting its latency, its state carried to a new instance, an edit in its editor reaching the app and the processor, an instrument playing its notes, latency made up for across tracks and across returns, mixer automation following the delayed sound, offline renders cut in time, and freezing, unfreezing and flattening in the model (with undo)
- [ ] Not tried yet: third-party plugins and listening on speakers. Checked in the desktop app with two VST3 plugins built for the tests (Soniq Test Delay: a gain and a 256-sample delay it reports; Soniq Test VST Synth) beside the two CLAP ones, through offline exports with every other track muted: the scan lists all four with their formats; the delay on Pad reports 256 samples and the chip shows +5.8 ms PDC; Pad and Keys together export bit-identical with and without the delay on Pad; −12 dB on its gain and its editor's +1 dB nudge land exactly (one undo step); frozen Pad, frozen Keys, flattened Keys, the Chords clip rendered to its own track and a frozen track with the VST3 synth all export bit-identical to their live devices; turning the plugin up while Pad is frozen changes nothing until it is unfrozen; the compensation drops to 0 ms while Pad is frozen
- Note: sends from a track inside a group reach the returns late by the group's latency; meters and the playhead run ahead of what you hear by the compensation; sidechain keys are not compensated; a frozen track's session clips do not play (the freeze is of the arrangement), and edits to a frozen track are heard after unfreezing; VST3 note expression, extra buses and sidechain inputs are not hosted

**Part C ✅ — pro polish**
- [x] Multicore graph processing (see §4): Settings › Cores for the mix (automatic: all cores but one, up to eight); the engine reports how many it uses. A song of 32 synth tracks with four compressors each renders 10 s in 3.24 s on one core and 1.38 s on eight, with the same output
- [x] Meters panel (the meter button at the end of the view tabs, View › Meters or `Ctrl+Alt+M`): integrated loudness against a target with how far off it is, momentary, short-term and range bars, 20 s of loudness history, true peak with a warning over −1 dBTP, peak and RMS per channel, correlation and a spectrum. Reset starts the integrated loudness, range and highest peak over. It measures only while open
- [x] Saved mixes and A/B in a bar above the mixer (also in the Track menu): Save mix keeps the faders, pans, mutes, solos and sends and the master; Saved mixes recalls, updates, renames or deletes them; Compare A/B keeps the mix as A and goes on as B, the A and B buttons switch, Keep ends the comparison. Each is one undo step, and they are saved with the song
- [x] Keyboard shortcuts in Settings: Soniq's, Audacity's or Live's set (their standard Windows keys for the commands Soniq has); any command's key changed by clicking it and pressing the new one, taking the key from another command if it had it (and saying so); back to the set's key, or none. Menus and the palette show the keys in use
- [x] Rust tests (206 in total) now also cover the worker pool (every job once over 5000 dispatches; jobs really side by side), several cores mixing bit for bit what one does (with and without sidechains, a group, a return, a synth and automation), the analysis tap hearing the master but not the click, the analyser (EBU reference tone at −23 LUFS momentary, short-term and integrated, live integrated agreeing with the export's measurement, LRA of 10 LU on EBU Tech 3342 case 1, a true peak 3 dB above the samples, correlation, the spectrum), and saved mixes and A/B in the model
- [ ] Not tried yet: listening on speakers, and the meters with music playing (the checks ran silent, every track muted). Checked in the desktop app: the engine mixes on 8 of 32 cores and plays (silently) in time; with the Meters panel open a reading arrives with every meter frame, and none once it closes; a saved mix brings Pad back from −20 to −6 dB in one undo step; A/B switches between −6 (A) and −12 dB (B); the Audacity set makes Ctrl+1 and Ctrl+3 zoom, the Live set makes = zoom and B pick Draw; giving Metronome the key L takes it from Loop and says so, and L then toggles the metronome but not the loop
- Note: summing (groups, returns, the master) and the returns' and master's effects stay on the audio thread; offline renders (export, freeze) still use one core; a group track can't take a sidechain from another group track in parallel (groups with sidechains run one at a time); the meters measure the master only

### Phase 6 — Delight & release

**Part A ✅ — start here**
- [x] Start page (at launch, or File › Start page): a new song from a template, Open…, the recent songs (missing ones dimmed, removable), the demo, and the lessons; "Show this when Soniq starts" can be switched off
- [x] Templates: Empty song; Beat (a two-bar drum pattern on a Drum Rack, a bass synth, keys, at 96 BPM); Band recording (vocals with low cut, compressor and de-esser, guitar, bass and stereo drums on inputs 1, 2 and 1/2, heard while armed); Podcast (host and guest with the voice chain, music with two compressors keyed by each voice so it ducks under either, a limiter at −1 dB on the master); Audio editing (one track, opening in the audio editor). File › Save as template… keeps the song's tracks, devices, sends, master chain and tempo (and plugin settings) without clips, audio, markers or mixes; your templates can be deleted
- [x] Recent songs: the last twelve opened or saved, remembered in the config folder
- [x] History (the clock by undo and redo, Edit › History… or `Ctrl+H`): every step back and ahead, newest first; a click jumps there in one go
- [x] Find missing files (File menu, or the toast when a song opens with audio missing): search a folder and the folders below it by file name, or Locate… each one; a file of a different length is used but flagged; saving copies them into the song's Samples folder
- [x] Lessons: "Find your way around" (play the demo, mute a track, the mixer, the meters), "Make a beat" (a MIDI track, a Drum Rack, a clip, the step sequencer, four kicks) and "Edit audio" (the editor, a selection, fade in, normalize, the History). Each step points at its control and moves on when it is done; finished lessons are ticked on the start page
- [x] Rust tests (210 in total) now also cover every built-in template (devices that exist, the podcast's sidechains, the beat's pattern), a saved template coming back without its clips and keeping out paths outside its folder, the history's labels, and finding files by name in folders below
- [ ] Not tried yet: listening on speakers (the checks ran silent). Checked in the desktop app: the start page opens at launch with five templates and three lessons; Podcast, Beat and Audio editing build as described (the podcast's music compressors keyed by Host and Guest, Audio editing opening in the editor); a saved template brings back the Beat's tracks and devices without the pattern, and deletes; three renames, back two and forward one land on the right names, and the History shows them with "Now"; a saved song shows in the recent list; with its sample moved away it reopens with the file missing, the toast offers Find missing files…, a search of another folder finds it one folder down, and saving copies it back; the tour lesson runs to the end (play skipped), and "Make a beat" moves on by itself through every step but the last (play)
- Note: templates and recent songs live in the app's config folder; lessons can't check that something was heard, only that it was done

**Part B ✅ — delight** (§5.17)
- [x] "Fix my audio" (Effect and Clip menus, the editor's toolbar, and a built-in macro for batch processing): listens to the selection or the clip, then lists seven repairs, each ticked when needed and saying what it found: centre the waveform (DC), cut rumble below 70 Hz, remove clicks, reduce the background noise (learned from the recording's own quietest moments, no profile needed, by 6–18 dB depending on how loud it will be once levelled), trim silence at the ends to a quarter second, shorten long pauses (found, never ticked for you: it changes the timing), and set the loudness (−16, −14, −19 or −23 LUFS) with peaks held under −1 dBFS. One undo step; preview plays 10 s. Silence and noise are judged above 70 Hz, so rumble cannot hide them; a second run on a fixed recording finds nothing more to do
- [x] New effects beside it: Reduce noise (automatic), Trim silence, Level a voice (rides the level towards a loudness, by at most a set amount, holding still through pauses so the room is not pulled up)
- [x] Podcast mode (View › Podcast tools; opens with the Podcast template): mark each audio track as a speaker or music and see how loud each is; "Level speakers" evens every speaker's clips out to the same loudness (−18, −20 or −16 LUFS) in one undo step; "Duck the music" writes a volume lane on the music that dips 8, 12 or 18 dB wherever anyone speaks (speech closer than 2.5 s keeps it down), as points you can move; chapters are the song's markers (add one at the playhead, rename, jump to it) and "Export the episode…" opens Export as an MP3 at −16 LUFS with a limiter, its chapters inside and a chapters file
- [x] Chapters in exports: ID3 chapter frames with a table of contents in MP3, CHAPTERnnn comments in FLAC, Ogg and Opus, cue points with labels in WAV; and a chapters file beside it (Podcasting 2.0 JSON, `00:00 Title` text, or a CUE sheet), or on its own from File › Export chapters from markers…. Export can also reach a loudness target with a limiter instead of stopping short of it
- [x] Theme editor (Settings › Appearance): Graphite or Porcelain, seven accents or your own colour (text on it turns graphite when white would be hard to read), four track colour sets, comfortable or compact density; remembered, applied before the first paint, "Back to Graphiq" resets it
- [x] Sketch mode (Sketch an idea on the start page, or View › Sketch mode over any song): one big record button over a loop of 1, 2, 4 or 8 bars, the tempo, the metronome and the input. From a stop it counts in a bar and records one pass; while playing it waits for the top of the loop. The pass is kept exactly (the transport keeps looping; the take is trimmed to one pass round the loop, latency included), the layer is disarmed so the input closes, and the loop plays on with it. Layers can be muted or removed; Space plays, R records; leaving gives the song its own loop back
- [x] Rust tests (223 in total) now also cover the diagnosis (noise, clicks, silence, pauses, with and without rumble and DC), the plan for a clean and a noisy recording, fixing doing what the plan says and a second fix finding nothing, trimming, leveling (quiet and loud speech brought together, pauses not lifted), learning noise from quiet moments, chapters in MP3, WAV, FLAC and Ogg, chapters from markers and the three chapters files, the ducking points, the podcast template's roles, and keeping one pass of a loop recording
- [ ] Not tried yet: listening on speakers, and recording a sketch layer from a real microphone (the checks never open the input). Checked in the desktop app: the theme editor's accent, colour set and density apply everywhere and are remembered; the Podcast template opens its panel with Host and Guest as speakers and Music as music; two recordings 17 LU apart measure −18.0 LUFS each after leveling; ducking writes a volume lane that dips under the speech; three chapters export into an MP3 (three ID3 chapters with the right times and titles, read back from the file) with a chapters.json and a text list; on a recording with hiss, rumble, DC, three clicks and silence at both ends, Fix my audio ticks all of them, leaves the 2.2 s pause for you, and after fixing measures −16.0 LUFS, peaks at −1.0 dBFS, no clicks, no rumble, a quarter second at each end, and a second look finds nothing; sketch mode opens with a 4-bar loop at 100 BPM, changes bars, lists layers, and gives the song's loop back; the export dialog opens with the episode's choices; no console errors
- Stem separation waits on the AI decision (§9.5); touch and pen, and localisation, come after

**Part C — release**
- [x] Windows installer: an NSIS setup (`Soniq Studio_<version>_x64-setup.exe`, about 5 MB), per-user (no admin prompt), with Start menu entry and uninstaller; app icons from Soniq's own mark (`src-tauri/icons/source.svg`)
- [x] Releases on GitHub ([Voltiq/soniq-studio](https://github.com/Voltiq/soniq-studio)): CI builds, lints (warnings fail) and tests every push; pushing a `v*` tag builds Windows, macOS (`.app`, `.dmg`) and Linux (AppImage, `.deb`) into one release with signed update files and `latest.json`, published once Windows has built. `scripts/set-version.mjs` sets the version everywhere; RELEASING.md walks through it
- [x] Auto-update (Tauri updater): Help › Check for updates…, or once a day by itself (Settings, Help › About; on by default, it only asks GitHub); a found version offers "Install and restart", asks about unsaved changes first, downloads with progress and restarts. Updates must carry the project's signature (the public key is built in)
- [x] Crash reports, opt-in and on this computer only: asked once at first launch; a panic anywhere (and an interface error nobody caught, a few per run) is written to a text file with what happened, where, the version and the system, nothing from songs or audio. The next launch says so and shows it; Help › Crash reports lists, shows, copies and deletes them, with a link to file an issue. If drawing the interface fails, a "Something went wrong" page offers a reload (the song lives in Rust and survives)
- [x] Help menu: start page and lessons, check for updates, crash reports, report a problem on GitHub, About Soniq Studio (the version)
- [x] Rust tests (225 in total) now also cover crash reports (written only while switched on, with thread, place and version; nothing outside their folder can be read or deleted) and their dates
- [x] v0.1.0 released on GitHub with the Windows setup, its update signature and `latest.json` (macOS and Linux failed to compile in that run; fixed since, and CI now checks the workspace builds on macOS and Linux on every push)
- [ ] macOS and Linux: they build, but CLAP and VST3 plugins only load on Windows (the sandbox uses Win32); a POSIX sandbox (shared memory, a child process, the plugin's window) is the work left
- [ ] Code signing: not set up (no certificate yet), so SmartScreen warns on the unsigned setup; RELEASING.md says how to add a certificate or Azure Trusted Signing, and Apple's Developer ID for macOS (also needed to notarise)
- [ ] Not tried yet: installing the setup (built and signed for updates here, not installed on this machine), an update from one release to the next, and the macOS and Linux builds (best effort on CI until tested). Checked in the desktop app: the Help menu; the one-time "Keep crash reports?" question; an interface error kept as a report, listed and shown in Help › Crash reports; About shows the version; the daily check stays quiet when it cannot reach a release, and a manual check says so plainly

### Phase 7 — The P0 gaps
Every P0 item the earlier phases left open, so nothing a first-time user leans on is missing.

**Part A ✅ — safety** (§5.1, §5.2)
- [x] Safety limiter: the last stage before the device (after the master chain, the click and previews): peaks over −0.3 dBFS are turned down at once and let back up over about 100 ms; a block holding NaN or infinity (a broken plugin) becomes silence, with a message. On by default (Settings › Audio, at once, remembered); **LIM** lights on the master meter while it works, CLIP still shows the overs it caught. Offline renders and exports are never limited
- [x] Device recovery: once a second a watch notices an output the driver reports gone or whose callbacks stopped, the device you chose coming back while Soniq plays elsewhere, or (following the system default) a new default. It reopens the right device and sends the song, input, plugins' and pool's sample rate over again. Playback stops on any switch, as a phone does when headphones come out, and a message says where the sound went; with no output at all it keeps trying. A lost input closes (a recording stops and keeps what it got) and opens again when it is back and still wanted
- [x] Monitoring across rates and clocks: the monitored input is resampled to the output rate (cubic) with a small, steady backlog kept by nudging the ratio up to ±0.3 %, so a USB microphone at 44.1 kHz monitors through 48 kHz headphones, and two devices whose clocks drift stay together for hours without clicks or growing delay
- [x] Rust tests (229 in total) now also cover the limiter (never over the ceiling, right up to it, off lets overs through, a broken block silenced, exports untouched) and monitoring (44.1 into 48 kHz at the right pitch without gaps; clocks 0.2 % apart for 30 s with a steady backlog)
- [ ] Not tried yet: unplugging a device, changing the default, and monitoring a real microphone (the checks never make a sound or open the input). Checked in the desktop app: the watch leaves a healthy device alone (no restarts in 12 s on AirPods); the Settings switch turns the limiter off and on at once and is remembered; LIM sits under CLIP on the master meter, dark while nothing is limited

**Part B ✅ — editing** (§5.3, §5.5)
- [x] Several clips at once: Ctrl- or Shift-click picks more (again to drop one); dragging one moves them all, on screen as they go, by the same beats and tracks (refused if any would land on a track of another kind or before the song); Delete and Ctrl+D act on all of them, each one undo step. After Ctrl+D the copies are the selection, so pressing it again carries on along the timeline
- [x] Ripple editing (Edit › Ripple editing, remembered): deleting clips closes their gaps on their tracks (several at once, each counted where it was), duplicating pushes the clips after the block along, trimming a clip's end moves what follows. Other tracks, markers and automation stay put
- [x] Fade shapes (Clip menu, for the picked clips): standard (straight fades, equal-power crossfades, as before), linear, equal power or S-curve, for the fades and the crossfades alike; the arrangement draws the shape
- [x] Audio editor: Duplicate (Ctrl+D with a selection, or Edit › Duplicate to a new track) and Split New (Ctrl+Alt+I) put the selection on a new track just below, at the same time; Split New leaves a gap. No audio is copied: the new clip plays the same file
- [x] Repair (Effect menu): a short damaged stretch (a click, a dropout; a quarter second at most) is redrawn from the sound around it, by autoregressive models predicting forwards from before and backwards from after, blended across the gap
- [x] Filter curve EQ: draw the curve (click to add, drag, double-click to remove; presets: flat, low cut, telephone, less mud, presence, air), applied with no phase shift
- [x] Auto Duck: turns the clip down by a depth wherever another track (picked in the dialog, heard through its devices) is above a threshold, fading down before it and up after it, and staying down through short pauses
- [x] Generator: rhythm track (tempo, beats per bar, bars, woodblock, beep or tick, the first beat higher), sample-exact
- [x] Rust tests (237 in total) now also cover moving, deleting and duplicating several clips, ripple (one gap, two gaps, duplicate, trim), Duplicate and Split New, fade shapes saved and cleared, Repair (rms error under 0.02 on a 300-sample burst), the curve EQ at three frequencies, Auto Duck (fade down, through a breath, back up), and the rhythm track's beats
- [ ] Not tried yet: listening. Checked in the desktop app (silent): a generated rhythm track; two picked clips duplicated with ripple, deleted with ripple and moved together, landing where the tests say; S-curve fades set on the picked ones; Ctrl+D walking on; Repair on a planted click (two of three left); Duplicate and Split New onto "(copy)" and "(split)" tracks at the right times with a gap in the original; Auto Duck of the music under the host ("Ducked 2 times"); the curve EQ dialog with the Telephone preset, applied as one undo step; Auto Duck's track list leaving out the clip's own track and waiting for a pick; no console errors

**Part C ✅ — analysis and meters** (§5.5, §5.8)
- [x] A new Analyze menu, for the selection in the audio editor (or the picked clip, whole):
  - Plot spectrum: the average power spectrum (Welch: Hann windows of 1024–16384 points, half overlapped, over every channel), a full-scale sine at 0 dB, on a log scale. The pointer reads the frequency, the nearest note with its cents, and the level; with nothing under it, the peak
  - Contrast: measure a stretch of speech, then one of background (RMS each, "Use the selection"); the difference passes WCAG 2 (1.4.7) at 20 dB or more, and says by how much it falls short
  - Find clipping (0.99 of full scale for 3 samples or more, runs within 10 ms joined), Beat finder (onsets, sensitivity 0.5) and Label sounds (above −30 dB, silences of 1 s or more between, sounds of 0.1 s or more) put markers on the timeline: "Clipped" and "Sound N" regions, "Beat" points. Each is one undo step and says what it found
- [x] Gain-reduction meters in the header of each compressor, gate, glue, limiter and de-esser: a bar (to 24 dB) and the dB, rising at once and falling back about 20 dB a second. The engine reads each device's reduction after its block, at all three places effects run (tracks, returns, master); a closed gate reads its range
- [x] A live spectrum behind the EQ's curve while it is shown and on: the EQ's output, tapped on the audio thread into a lock-free queue (mono), measured in sixth-octave bands from 20 Hz and sent with the meter frames. The tap follows the device through graph rebuilds and comes back after the audio device restarts
- [x] Rust tests (240 in total) now also cover the spectrum plot (a sine's peak bin and level), RMS, clipping runs (joined and separate), labelled sounds (two bursts, short gaps bridged), and an engine test where a compressor reports its reduction and a tap hears its output
- [ ] Not tried yet: watching the meters move, which needs playback. Checked in the desktop app (silent): a generated test file with four hits, one clipped: Find clipping marked 1 stretch at 2.00–2.05 s, Beat finder 4 beats at 0.5, 2, 3.5 and 5 s, Label sounds 4 sounds; Contrast of a hit against silence passing; Plot spectrum peaking at 440 Hz with the clipping's harmonics; GR meters on the compressor, gate and limiter (none on the EQ); the EQ's spectrum layer in place; no console errors

**Part D ✅ — library and drums** (§5.7, §5.12)
- [x] Browser › Library: add folders of samples and presets; Soniq indexes them in the background (on every core, unchanged files kept, progress shown) and listens to each sample once: loop or one-shot, a loop's tempo (from its name, "120bpm" or "_124_", or heard), a key for pitched loops and a root note for pitched one-shots (chroma against key profiles; drums are never given one), and tags from the file and folder names (kick, snare, hat, bass, pad, vocal, fx, loop…). Search by name, tag, key or tempo; filter by favourites, recent, loops, one-shots, presets and the most used tags; star favourites and add your own tags. Kept in `library.json` with the app's settings
- [x] Audition: click a sample to hear it (again to stop). "In time" (on by default, remembered) stretches a loop to the song's tempo and loops it from the next bar while the song plays; a pitched sample moves to the song's key the short way (a minor loop into its relative major, and back); one-shots start on the next beat. Off, it plays as it is, at once
- [x] Library items drag by pointer (HTML drag and drop is off in the desktop window): a sample onto a track lane (at that beat), a drum pad or a sampler; a preset onto any device chain. Double-click adds a sample to the song or a preset to the chain on show; whatever goes in tops Recent
- [x] Presets: the book button on any built-in device saves it, with its settings and any pad effects, to Documents › Soniq Studio › Presets › its device (samples stay with the song); loading gives everything fresh ids and brings parameters up to date
- [x] Drum Rack: 64 pads in four pages (Kit, Perc, Perc 2, More). 30 new built-in sounds on notes 52–81, the General MIDI percussion (china, splash, ride bell, tambourine, cowbell, vibraslap, bongos, congas, timbales, agogos, cabasa, maracas, whistles, guiros, claves, wood blocks, cuícas, triangles with their own choke group); pads 82–99 play only samples
- [x] Each pad can have its own audio effects, and Send A and Send B (off at −60 dB) into the rack's two returns, which have effects too ("Pad effects", "Return A", "Return B" open the chain in the devices panel, with a way back). Sends are taken after the pad's effects; effects ring out for 8 s after the last hit and keep their state when the song changes. Pads without effects or sends cost nothing extra
- [x] Rust tests (253 in total) now also cover name tags, tempos and keys, a pitched one-shot's root, noise left keyless, a chord loop's key, moving keys the short way, a click loop's tempo, every built-in pad's level, empty pads, a pad's effects touching only it, sends through a return, a reverb ringing past the hit, pad chains in the song (only audio effects, knobs and switches, undo), and an audition looping from the next beat
- [ ] Not tried yet: hearing an audition, which needs sound. Effects inside the rack have no meters, live spectrum or automation lanes yet. Checked in the desktop app (silent): a test folder indexed (kick: one-shot tagged drums and kick; a click loop at 120 BPM; a pad named in A minor); favourites, your own tags and Recent; the kick dragged onto a pad and a loop onto Audio 1 at beat 4; the Perc page's pads and the empty More page; a compressor on one pad (a synth refused), a reverb on Return A and the pad's Send A at −6 dB; the kit saved as a preset and added to another track with its pad effects and fresh ids

### Phase 8 — Depth
Every P0 is done after Part A. The rest of this phase takes the P1 items people reach for first, a part at a time.

**Part A ✅ — the last P0 and loose ends** (§5.8, §5.9)
- [x] Linear-phase EQ: the Equalizer's new Phase choice (Minimum or Linear). Linear applies the same eight-band curve without shifting any frequency in time: the bands' combined level at every frequency becomes a symmetric filter (8192 taps at 44.1 and 48 kHz, more at higher rates; Blackman window), run by overlap-save FFT every 1024 samples, all of it allocated when built. It is 5120 samples late at 48 kHz (107 ms, shown under the choice); the engine makes up for it like a plugin's delay on tracks, returns and the master, and exports cut it off. Turning a knob remakes the filter at the next block and keeps the sound in flight. Inside a Drum Rack the EQ stays minimum phase (nothing there can make up for a delay)
- [x] The devices panel folds to its header (the arrow at its left, remembered); in the Mixer it starts folded, so every strip shows its faders in full. Dropping a device on the folded header opens it
- [x] Rust tests (256 in total) now also cover the linear EQ's curve (a bell at +12 dB, a low cut at −3 dB on its corner and under −40 dB two octaves down), an impulse coming out symmetric about the reported delay, a rebuild keeping the sound in flight, and two tracks with the same click landing on one sample when one has the linear EQ
- [ ] Not tried yet: listening. Checked in the desktop app (silent): Phase set to Linear on an EQ shows 107 ms; the Mixer opens with the devices folded and no strip cut off; the arrangement keeps its devices open

**Part B ✅ — takes** (§5.2, §5.4)
- [x] Loop recording makes takes: with the loop on, an audio recording that goes round it more than once is kept whole in one file, and every pass becomes a take lane under its track (Take 1, Take 2, …; the first keeps any run-up before the loop, the last may be cut short). The last pass goes on the track, over what was there, or the one before it when the last stopped under half way. One undo step ("Record takes"); the lanes open by themselves
- [x] Comping: swipe across a stretch of a take (on the grid; Alt ignores it) and it replaces that stretch on the track, with 5 ms fades where pieces meet so joins never click; double-click or Use takes the whole take. The stretches of each take that play are lit on it. A take can be deleted, or all of a track's takes at once (the comp stays); the recording stays in the song while any take or clip uses it. The count on the track header (and the track's menu) shows and hides the lanes
- [x] Recording into session slots: an armed track's empty slots are record buttons, and any audio or MIDI track's empty slot has "Record here" in its menu (that track alone records, armed or not, and what is played goes to it). Stopping waits for the next bar line (or takes the one just passed), so the clip is whole bars long and loops in time; it then plays on from the launch grid. The loop, punch and the session-to-arrangement write are off for it. One undo step
- [x] A scene into the arrangement: right-click a scene, "Write to the arrangement at the playhead": every clip of the scene lands at the bar the playhead is on, over what was there, each repeated for as long as the scene's longest clip. One undo step; the session keeps its clips
- [x] Rust tests (260 in total) now also cover a loop recording split into its passes (from the loop's start, with a run-up, with the round-trip delay, begun past the loop), takes placed and numbered, a swipe comped into three pieces with their fades, a recording kept alive by its takes alone, slot clips trimmed to whole bars (audio and notes), and a scene written out with its short clip repeated
- [ ] Not tried yet: recording audio (the checks never open the microphone), so loop takes from a real recording and an audio slot recording are untested end to end; MIDI loop recording still merges into one clip rather than making takes. Checked in the desktop app (silent): a song given two takes by hand shows them under its track with the playing one lit; a swipe over one beat makes three pieces and lights both takes; Use and two undos; an armed track's empty slots turn into record buttons; a slot recording on a muted MIDI track with two notes played from the interface makes a one-bar clip in that slot (another track was selected), nothing on the timeline, queued to play; Scene 1's clips written at bar 3

**Part C — clips and the arrangement** (§5.3)
- [ ] Clip gain envelopes, reverse, clip-level pitch and stretch
- [ ] Time-signature changes; arrangement sections (intro, verse, chorus) that move as blocks

**Part D — sound design** (§5.7, §5.8)
- [ ] A wavetable synth with a modulation matrix
- [ ] Audio effect racks (parallel chains, macros) and dry/wet on every device; convolution with your own impulse responses

**Part E — the project** (§5.15, §5.16)
- [ ] Project versions (snapshots) with notes, to compare and restore; a shareable `.soniqz` bundle
- [ ] Cover art and gapless MP3

---

## 7. Replacement checklists

### Can I uninstall Audacity 4?
- [ ] Open any common audio file and see it in under 1 s for a 5-minute song
- [ ] Record from a USB microphone with input monitoring and no dropouts *(built; needs a test on real hardware)*
- [x] Select, cut, paste, fade, normalise, amplify and remove silence on real samples
- [ ] Noise reduction at least as good as Audacity's profile-based tool *(built; compare on real recordings)*
- [x] Spectrogram with spectral selection
- [x] Labels and export of multiple regions
- [x] Macros and batch processing of a folder
- [x] Export MP3, WAV, FLAC and OGG with metadata
- [x] Loudness normalisation to a LUFS target
- [ ] Use existing VST3 and CLAP effects *(built in phases 5A and 5B; needs a try with third-party plugins)*

### Can I uninstall Ableton Live 12 Lite?
- [x] Session view with clip launching, scenes and follow actions *(built in phase 4B; launch modes and recording into slots to come)*
- [x] Arrangement with automation, warping and tempo changes *(automation built in phase 4B, warping and the tempo track in 4C; warp markers to come)*
- [ ] MIDI recording, piano roll, scales, quantise, Capture MIDI *(all built in phase 3; needs a test with a hardware keyboard)*
- [ ] Sampler, Drum Rack and a synth good enough to sketch songs without plugins *(built, with slicing and racks; needs a listening test)*
- [ ] Effects that cover Live Lite's essentials: EQ, compressor, reverb, delay, saturation, filter, utility *(built in phase 4A; needs a listening test)*
- [x] Return tracks and sidechain compression
- [ ] Ableton Link and MIDI controller support *(MIDI learn built in phase 3B; Link to come)*
- [ ] Import samples and MIDI from existing Live projects (manual export is acceptable at first)

---

## 8. Non-functional requirements

| Area | Target |
| --- | --- |
| Latency | Round trip under 10 ms at 128 samples / 48 kHz on ASIO or WASAPI exclusive |
| Stability | No xruns on a 64-track project with 4 effects per track on a mid-range 2023 laptop |
| Startup | Cold start to an empty project in under 2 s |
| Project load | A 50-track project with 2 GB of audio opens in under 3 s (peak files cached) |
| UI | 60 fps while playing; meters at 30–60 Hz; no main-thread work over 16 ms |
| Memory | Audio streamed from disk; only clips that play soon are cached |
| Safety | Autosave plus crash recovery; recordings survive a crash; no data loss on device change |
| Accessibility | Keyboard reachable, screen-reader labels, contrast as in the design tokens |
| Testing | DSP golden tests, null tests, decoder fuzzing, e2e smoke tests in CI on Windows, macOS and Linux |

---

## 9. Open decisions

1. **Name and branding**: is "Soniq Studio" final? It fits the Graphiq / Magiq family.
2. **Licence model**: proprietary, GPL or MIT? This decides whether GPL libraries (Rubber Band, some LV2 hosts) can be used, or whether only permissive ones can. *Decided for MP3:* LAME (LGPL) is linked in, which is fine for personal use and open-source releases; a closed-source release must ship LAME as a separate, replaceable library instead. Ogg Vorbis (libvorbis) and Opus (libopus, as a Rust transpilation) are BSD-licensed.
3. **Steinberg SDKs**: VST3 hosting uses the `vst3` crate's bindings (MIT/Apache); confirm the current VST3 SDK terms and the ASIO SDK's before shipping a release with VST3 or an ASIO build.
4. **Platforms**: Windows first (your machine), then macOS and Linux? CI for all three from phase 1 is cheap and avoids surprises.
5. **AI features**: local-only (ONNX models bundled or downloaded on demand) versus optional cloud. The recommendation is local-only by default.
6. **Project interchange**: how much effort should go into importing `.als`, `.flp` and `.aup3` files compared to plain audio and MIDI stems?

---

## 10. Repository map (current)

```
soniq-studio/
├─ ROADMAP.md                   ← this file
├─ README.md                    how to run
├─ RELEASING.md                 versions, tags, the update key, code signing
├─ .github/workflows/           CI (build, clippy, tests) and Release (installers, updates)
├─ scripts/set-version.mjs      the version in package.json, Cargo.toml and tauri.conf.json
├─ shared/device-catalog.json   device specs read by Rust and the UI
├─ crates/
│  ├─ soniq-project/            project model, ops (markers, notes, device samples, automation,
│  │                            groups, session slots, tempo, warp), tempo map, undo history,
│  │                            device catalog
│  ├─ soniq-io/                 decode (Symphonia, Opus), resample, peaks, edits, effects,
│  │                            generators, STFT and spectral tools, loudness, live analysis
│  │                            (analysis.rs), repair (Fix my audio's diagnosis and plan,
│  │                            trimming, leveling), tempo detection, encoders (with chapters)
│  ├─ soniq-dsp/                instruments, MIDI effects, audio effects (fx/: bus, character,
│  │                            convolution, vocoder…), warp (time-stretch), parameter banks
│  ├─ soniq-midi/               MIDI inputs (midir), MIDI files (midly), message parsing
│  ├─ soniq-host/               CLAP and VST3 plugins: loading (clap.rs, vst3.rs, and
│  │                            instance.rs over both), host callbacks (host.rs), the sandbox
│  │                            (sandbox.rs), the app's side and the engine node (client.rs),
│  │                            shared memory, protocol; examples/ build two CLAP and two VST3
│  │                            test plugins
│  └─ soniq-engine/             renderer (instruments, live notes, crossfades, warped clips, the
│                               safety limiter; monitor.rs: input at the output's rate and clock;
│                               preview, monitoring, groups, the analysis tap), workers (the
│                               multicore pool), tempo map, graph (automation, modulators),
│                               launch (session clips), transport, devices, recorder, meters,
│                               offline render
├─ src-tauri/src/
│  ├─ lib.rs                    Tauri builder, engine and MIDI start, command registration
│  ├─ session.rs                open song, audio pool, engine graph, input and MIDI routing
│  ├─ build.rs                  devices, notes, session slots, automation and modulators for the graph
│  ├─ scenes.rs                 clip and scene launching, recording the session into the arrangement
│  ├─ warping.rs                warping a clip (finding its tempo), slicing a clip to MIDI
│  ├─ plugins.rs                finding plugins, a sandbox per plugin device, their state,
│  │                            what plugins do by themselves
│  ├─ freeze.rs                 freezing tracks, rendering a clip to a new audio track
│  ├─ commands.rs               project, clip edits, recording (audio and MIDI), export, meters
│  ├─ midi.rs                   live notes, MIDI inputs, MIDI files, samples for instruments
│  ├─ processing.rs             effects and previews, spectral tools, drawing, generators,
│  │                            macros, batch processing
│  ├─ files.rs, settings.rs     saving, autosave, finding missing files; audio and MIDI settings,
│  │                            macros and recent songs on disk
│  ├─ templates.rs              the built-in song templates and the user's own
│  ├─ crash.rs                  opt-in crash reports (the panic hook, listing and reading them)
│  ├─ devicewatch.rs            device recovery: lost, returning and new default outputs; lost inputs
│  ├─ podcast.rs                leveling speakers, ducking music, chapters and chapters files
│  └─ demo.rs                   the demo song
└─ src/
   ├─ App.tsx, commands.ts      shell, command registry (menus, palette, shortcuts), computer keyboard
   ├─ engine/                   backend.ts (Rust calls), bridge.ts (engine, live notes), preview mock
   ├─ state/                    store, types, tempo (the tempo map), effects and generator registry,
   │                            peaks, music (keys, scales), notes (quantize, legato, transpose…),
   │                            analysis (the meters' readings), keymap (shortcut sets), lessons,
   │                            appearance (accents, track colour sets, density)
   ├─ components/               dialogs (Export, Effect, Generate, Macros, Settings with the
   │                            keyboard shortcuts), device chain with instrument panels (drum
   │                            pads, sample slot), PluginPanel, AnalysisPanel, WarpControls,
   │                            WelcomeScreen (start page), LessonCard, HistoryPopover,
   │                            MissingFilesDialog, FixDialog, PodcastPanel, SketchMode,
   │                            AboutDialog (updates, crash reports), CrashGuard…
   └─ views/                    Arrange (markers, crossfades, groups, clip bar) + AutomationLane
                                + TempoLane, Session, Mixer, Editor (+ editorCanvas: spectrogram,
                                sample detail), PianoRoll
```

## 11. Keyboard shortcuts (Soniq's set)

Settings › Keyboard shortcuts switches to Audacity's or Live's keys, and changes any command's key.

| Action | Shortcut |
| --- | --- |
| Play / pause | `Space` |
| Stop and return to start | `Enter` |
| Record | `R` (Shift+R in the editor) |
| Loop on/off | `L` |
| Metronome on/off | `M` |
| Command palette | `Ctrl+K` |
| Undo / redo | `Ctrl+Z` / `Ctrl+Shift+Z` or `Ctrl+Y` |
| Arrange / Session / Mixer / Editor / Piano roll | `F1` / `F2` / `F3` / `F4` / `F5`, or `Tab` to swap Arrange and Session |
| Zoom in / out / fit | `Ctrl+=` / `Ctrl+-` / `Ctrl+0` (in the editor and piano roll: that view; also Ctrl + mouse wheel) |
| Add marker / region from loop | `Ctrl+M` / `Ctrl+Shift+M` |
| Export | `Ctrl+Shift+E` |
| Piano roll: select all, duplicate, quantize | `Ctrl+A`, `Ctrl+D`, `Ctrl+U` |
| Piano roll: move notes up/down (scale step with Snap to scale), an octave | `↑` `↓`, `Shift+↑` `Shift+↓` |
| Piano roll: move notes earlier/later, shorten/lengthen by the grid | `←` `→`, `Shift+←` `Shift+→` |
| Computer keyboard plays notes on/off | `Ctrl+Shift+K`; then `A`…`'` play, `Z`/`X` octave, `C`/`V` velocity |
| Capture MIDI (what you just played) | `Ctrl+Shift+C` |
| Duplicate a MIDI clip as a linked copy | `Ctrl+Alt+D` |
| Map a MIDI controller, or a rack macro; show its automation | Right-click a knob, fader or pan control |
| Group the picked tracks (Ctrl-click headers to pick) / ungroup | `Ctrl+G` / `Ctrl+Shift+G` |
| Tools: pointer, range, draw, split, erase, automation (shows every track's lanes) | `V`, `S`, `D`, `C`, `E`, `A` |
| Toggle browser | `Ctrl+Alt+B` |
| Meters (loudness, true peak, spectrum) | `Ctrl+Alt+M` |
| Essentials ↔ Pro | `Ctrl+Shift+P` |
| History (every step, to go back to) | `Ctrl+H` |
| Sketch mode: play the loop / record a layer | `Space` / `R` |
| Pick several clips | Ctrl- or Shift-click |
| Audio editor: Duplicate / Split New (the selection onto a new track) | `Ctrl+D` / `Ctrl+Alt+I` |
