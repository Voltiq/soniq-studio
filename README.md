# Soniq Studio

[![CI](https://github.com/Voltiq/soniq-studio/actions/workflows/ci.yml/badge.svg)](https://github.com/Voltiq/soniq-studio/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/Voltiq/soniq-studio)](https://github.com/Voltiq/soniq-studio/releases/latest)

A desktop audio editor and music workstation: Tauri 2, a real-time Rust audio engine, and a React UI
in the Graphiq / Magiq Studio design language. It is designed to replace Audacity and
Ableton Live Lite, and borrows the best workflow ideas from FL Studio.

**What's planned and in what order → [ROADMAP.md](ROADMAP.md)**

## Download

Get the latest Windows setup (`Soniq Studio_…_x64-setup.exe`) from
[Releases](https://github.com/Voltiq/soniq-studio/releases/latest). It installs for your user only (no
admin prompt) and updates itself: Help › Check for updates, or once a day on its own (Settings).
The setup isn't code-signed yet, so Windows SmartScreen may warn: choose More info › Run anyway.
macOS and Linux builds come from the next release on; they are untested so far, and CLAP and VST3 plugins
only load on Windows for now (the plugin sandbox is Windows-only).

Crash reports are off unless you switch them on (Help › Crash reports). They stay on your computer;
copy one into an [issue](https://github.com/Voltiq/soniq-studio/issues) if you'd like it looked at.

## Run it

Prerequisites: Node 20+, Rust (stable), and the [Tauri prerequisites](https://tauri.app/start/prerequisites/)
for your OS (on Windows: the MSVC build tools and WebView2).

```bash
npm install
npm run tauri dev      # desktop app with the Rust audio engine
npm run dev            # UI only, in a browser (no audio engine; the UI runs on its own clock)
npm run tauri build    # installer in target/release/bundle (needs the update key: RELEASING.md)
```

Releases are built by GitHub Actions from a version tag: see [RELEASING.md](RELEASING.md).

> **Windows path length:** if the project sits in a very deep folder, the MSVC linker can fail with
> `LNK1104 cannot open file`. Move the project to a shorter path or set `CARGO_TARGET_DIR` to one,
> for example `C:\t\soniq`.

## What works today (phases 0–6)

- **Real playback.** The Rust engine mixes audio clips and MIDI clips sample-accurately, with track volume,
  pan, mute, solo, master fader, a sample-accurate loop and a metronome. Meters and the playhead are
  streamed from the engine.
- **Instruments.** MIDI tracks play through a built-in instrument: **Soniq Synth** (two oscillators and a
  sub, unison, filter with drive, two envelopes, LFO and vibrato, poly/mono/legato with glide), **Electric
  Piano**, **Sampler** (classic, one-shot, loop, or sliced: equal pieces or at the transients, one key
  per slice, "To clip" writes the slices out as a MIDI clip; plus an LFO) and **Drum Rack** (64 pads in four
  pages with a built-in kit and percussion; drop your own samples on any pad, give each pad its own
  effects and sends into the rack's two returns). The **Instrument Rack** layers instruments or splits
  them by key and velocity, with eight macros: right-click a chain's knob to map it to one. Add them from
  the browser; their controls are in the device chain.
- **MIDI effects.** Arpeggiator, Chord, Scale, Note Length, Random and Velocity sit before the instrument
  and change the notes as they play.
- **Mixing.** Every track, both return tracks and the master have an effect chain: an 8-band EQ (drag
  the points on its curve), compressor, gate, limiter, reverb, delay (free or synced, ping-pong), chorus,
  phaser/flanger, auto filter, saturator and utility, plus a glue compressor, three-band multiband
  dynamics, de-esser, transient shaper, auto-pan, redux, convolution reverb, pitch shifter and vocoder.
  Sends reach the returns before or after the fader (the Pre/Post switch on each return). A compressor
  or gate can listen to another track (Sidechain), so the bass ducks under the kick; a vocoder can speak
  through another track (Carrier track, with Carrier set to "Sidechain track"). In the mixer, click a
  return's or the master's name to open its chain.
- **Library.** Browser › Library indexes your sample and preset folders: loop or one-shot, tempo, key
  and tags from the names; search, favourites, your own tags and recent items. Click to hear a sample in
  time with the song (stretched to its tempo, moved to its key), drag it onto a track or a pad. Any
  built-in device saves as a preset (the book button on it).
- **Plugins (CLAP and VST3).** Browser › Plugins lists the CLAP and VST3 plugins in the standard folders and any you add
  (Add a folder…, or Settings). Click one to add it to the selected track, a return or the master; an
  instrument plugin replaces the track's instrument. Each plugin runs in a sandbox of its own: if it
  crashes or hangs, the song plays on and the panel offers **Start again**. The panel opens the plugin's
  window and shows its controls as knobs (automatable and MIDI-mappable); **Controls…** picks which.
  Turning a control in the plugin's own window moves its knob too. Plugin settings are saved with the
  song. Plugins that delay their sound are made up for on every other track (the engine chip shows by
  how much, "+5.8 ms PDC"). Windows only for now.
- **Meters, mixes and keys.** The meter button at the end of the view tabs (or `Ctrl+Alt+M`) opens the
  Meters panel: integrated, momentary and short-term loudness against a target (−14 for streaming,
  −16, −23 or −24 LUFS), loudness range and 20 s of history, true peak (flagged over −1 dBTP), RMS,
  stereo correlation and a spectrum of the master. Above the mixer, **Save mix** keeps the faders,
  pans, mutes, solos and sends to come back to, and **Compare A/B** switches between two mixes.
  Settings › Keyboard shortcuts offers Audacity's or Live's keys and lets you change any of them.
  The mix is shared over several cores (Settings › Cores for the mix).
- **Freeze and render.** Right-click a track header (or the Track menu) and **Freeze** it: its clips are
  rendered through its devices and it plays that while they rest, saving CPU and plugin latency;
  **Unfreeze** brings the devices back, **Flatten** turns it into an audio track. Clip › **Render to a
  new audio track** renders one clip through its track's devices onto a new track below it.
- **Tempo changes and warping.** The **Tempo** button by the markers (or View › Tempo track) opens the
  tempo lane: click to add points; the tempo glides from one to the next, and two points on one beat
  make a jump. The BPM field shows the tempo at the playhead. Select an audio clip and switch on
  **Warp** in the bar under the tracks (or Clip › Warp to project tempo): Soniq finds the clip's own
  tempo and from then on it keeps its beats on the song's, in **Beats** (drums), **Tones** (melodies,
  voices) or **Repitch** (turntable) mode. Type its tempo or halve or double it if the guess is off.
  **Slice to MIDI** in the same bar puts the clip on a new MIDI track: a Sampler cut at its hits (or
  every 1/16, 1/8 or beat) and a clip that plays them, ready to rearrange.
- **Session view** (`F2`). Click a clip to launch it on the next bar (the Launch menu sets the grid);
  click a scene to launch its row; an empty slot or a stop button stops its track. Clips loop until
  something else plays. A launched track leaves the arrangement until **Back to arrangement**. Select a
  clip to give it a follow action (after a few bars: next, previous, any, again, stop…). Double-click an
  empty slot on a MIDI track to make a clip, or put a copy of an arrangement clip into a slot (Clip ›
  Put a copy in the session, or the panel under the grid). Press record while the session plays, with
  or without armed tracks, and what it plays is written into the arrangement.
- **Automation and modulation.** The wave button on a track header shows its automation lanes; the
  Automation tool (`A`) shows every track's. Click a lane to add a point, drag points, double-click one
  to delete it; the + menu adds lanes for pan, sends or any knob, and "Show automation" on any knob's
  right-click menu opens its lane. An **LFO** or **Envelope Follower** (Browser › Modulators) moves a
  knob of another device on its track: pick it under "Moves".
- **Groups.** Ctrl-click track headers to pick them and press `Ctrl+G`: they play through the group's
  effects and fader. Fold a group with its arrow; right-click a header to move a track in or out.
- **Demo song.** Drums, bass and keys are MIDI played by the Drum Rack, Synth and Electric Piano; the pad
  is rendered to audio so the audio editor has something to work on.
- **Import.** Use `Ctrl+I`, or drag WAV, AIFF, FLAC, MP3, OGG, Opus or M4A files from Explorer onto a track.
  Files are resampled to the device rate. MIDI files (File › Import MIDI file, or drag `.mid` files in)
  become one MIDI track per part; drums land on a Drum Rack.
- **Recording.** Arm one or more tracks and press `R`. Audio tracks record their input (In 1, In 2 or
  In 1/2 for stereo, picked in the device chain); MIDI tracks record what you play on a MIDI keyboard or
  the computer keyboard. Count-in plays a bar of clicks first; Punch records only inside the loop range,
  starting a bar early. Each audio track can monitor its input (off, while armed, or always; use
  headphones). Audio takes are moved earlier by the measured latency, plus an offset you can set in the
  audio settings, and stream to disk; if Soniq crashes mid-take, the next launch offers to import them.
  MIDI recording makes a new clip, overdubs into the clip under the playhead, or replaces its notes
  (Transport menu); with the loop on, every pass lands in the loop.
- **Playing live.** Every MIDI keyboard plugged in plays the armed MIDI tracks, or the selected one
  (switch inputs off in Audio & MIDI settings). `Ctrl+Shift+K` turns the computer keyboard into one:
  `A`…`'` play two octaves, `Z`/`X` change the octave, `C`/`V` the velocity. Forgot to record?
  **Capture** (`Ctrl+Shift+C`) turns what you just played into a clip.
- **MIDI controllers.** Right-click a device knob, a rack macro, or a track's volume or pan in the mixer,
  choose Learn MIDI controller and move a knob or fader on your keyboard. Mappings are saved with the
  song and listed in Audio & MIDI settings.
- **Piano roll** (`F5`, or double-click a MIDI clip). Double-click to add or delete notes, drag to move,
  Ctrl-drag to copy, drag edges to resize, paint velocities in the lane below. Quantize with strength and
  swing (`Ctrl+U`), legato, transpose with the arrows, duplicate, cut, copy and paste. Pick a key and
  scale to highlight it, fold the rows to it, or snap notes to it. Ghost notes show the track's other
  clips. The Tools menu chops, arpeggiates, strums, flams, humanizes, stacks chords, reverses and more,
  and generates Euclidean rhythms or random notes in key. Switch to **Steps** for a step sequencer with
  velocity, length and shift lanes and swing. Double-click an empty spot on a MIDI track to make a new clip.
- **Linked clips.** `Ctrl+Alt+D` duplicates a MIDI clip as a linked copy, like an FL Studio pattern: edit
  one and every copy changes. Clip › Unlink clip lets one go its own way.
- **Clip editing.** Drag a clip to move it (also to another track), drag its edges to trim, drag the
  top corners to fade. Overlapping clips crossfade by themselves; Clip › Fades picks the shape (linear,
  equal power, S-curve). The Split tool or `Ctrl+E` splits at the playhead, `Ctrl+D` duplicates.
  Ctrl- or Shift-click picks several clips to move, delete or duplicate together, and Edit › Ripple
  editing closes the gaps they leave. Edits snap to the grid; hold Alt to place freely.
- **Markers.** The lane under the ruler holds markers (`Ctrl+M` at the playhead) and regions
  (`Ctrl+Shift+M` from the loop range). Drag them, double-click to rename.
- **Audio editor.** Cut, copy, paste, delete and trim on the selection, snapped to zero crossings.
  Zoom down to single samples (Ctrl + mouse wheel) and redraw them with the Draw tool (`D`).
  The Spectrogram view (log, mel or linear) selects frequency bands to delete, lower or keep.
- **Fix my audio.** One click in the editor (or Clip › Fix my audio…) listens to a recording and lists
  what it needs: centring, a rumble cut, click removal, noise reduction learned from its own quiet
  moments, trimming the silence at the ends, shorter pauses (yours to tick) and a loudness target with
  peaks held under −1 dBFS. Untick anything, preview, fix in one undo step.
- **Effects and generators.** The Effects menu in the editor has 24 effects with settings and a
  preview: noise reduction (learn the noise first, or automatic), click removal, filters, EQ,
  compressor, limiter, truncate and trim silence, voice leveling, tempo, pitch and speed changes, echo,
  reverb, normalize, loudness, a drawn filter curve, Auto Duck and more; Repair redraws a click or a
  dropout, and Duplicate or Split New (`Ctrl+Alt+I`) puts the selection on a new track. Generate makes
  tones, sweeps, noise, DTMF, rhythm tracks and silence.
- **Analyze.** Plot the spectrum of a selection (frequency, note and level under the pointer), check
  speech against its background for WCAG contrast (20 dB), and mark clipping, beats or separate sounds
  on the timeline. Compressors, gates, glue, limiters and de-essers show their gain reduction live, and
  the EQ draws a live spectrum of what it puts out behind its curve.
- **Macros and batch.** Run a chain of effects on a clip, or on a whole folder of files
  (File › Batch process files). Five macros are built in, Fix my audio among them; make your own.
- **Export.** `Ctrl+Shift+E` renders the song, the loop region, each region or each track (stems: each
  track through its own effects, dry of the returns and the master chain) to
  WAV, FLAC, MP3, Ogg Vorbis or Opus, with tags, dither, an optional sample rate and an optional loudness
  target that keeps peaks under −1 dBFS (or reaches it with a limiter). Markers can go in as chapters
  (ID3 chapters in MP3, chapter comments in FLAC, Ogg and Opus, cue points in WAV), with a chapters file
  beside it. The report shows the measured LUFS and peak. File › Export MIDI writes every MIDI track to
  a `.mid` file; the piano roll exports a single clip.
- **Podcast tools** (View › Podcast tools, open with the Podcast template). Mark tracks as speakers or
  music and see how loud each is; level every speaker to the same loudness; duck the music under speech
  as a volume lane you can edit; keep chapters as markers; export the episode as an MP3 at −16 LUFS
  with its chapters.
- **Sketch mode.** Sketch an idea (start page) or View › Sketch mode: one big record button over a loop
  of 1–8 bars. Each press records one pass round the loop as a new layer, and the loop plays on.
  The input opens only while a layer records.
- **Undo and redo** for every change, kept in Rust. A fader drag counts as one step. The clock by the
  undo button (`Ctrl+H`) lists every step; click one to jump back (or forward) to it.
- **Projects.** New, open, save and save as. A `.soniq` file sits next to a `Samples/` folder that holds
  copies of everything it uses. Autosave runs every 30 s, and after a crash Soniq offers to restore your work.
  If a song's audio has moved, File › Find missing files… searches a folder for it.
- **Start page and lessons.** At launch (or File › Start page): start from a template (Empty song, Beat,
  Band recording, Podcast, Audio editing, or one you saved with File › Save as template…), reopen a recent
  song or the demo, or follow a lesson: a card points at the control each step needs and moves on once
  you have done it.
- **Audio settings.** Choose the driver, output, sample rate, buffer size and how many cores mix. The
  choice is remembered. A safety limiter keeps anything over −0.3 dBFS from reaching your ears (LIM on
  the master meter), and if a device is unplugged Soniq stops, carries on with the system default and
  goes back to yours when it returns.
- **UI.** Graphiq design tokens, Arrangement, Session, Mixer, Audio editor and Piano roll views, device
  chains, Essentials and Pro modes, a command palette (`Ctrl+K`) and shortcuts. Settings › Appearance
  picks the theme, the accent (or your own colour), one of four track colour sets, and comfortable or
  compact density.

Noise Reduction in the device chain is still a placeholder (marked "Not yet audible"); use it from the
audio editor's Effects menu for now.

Commands that are planned but not built yet appear dimmed in the menus and the palette, tagged with
the roadmap phase that delivers them.

## Layout

```
crates/soniq-project   project model, ops, markers, notes, tempo map, undo history (no audio data, cheap
                       to snapshot)
crates/soniq-io        decode, resample, peaks, edits, effects, generators, spectral tools, loudness,
                       transient and tempo detection, export to WAV, FLAC, MP3, OGG and Opus
crates/soniq-dsp       instruments (synth, electric piano, sampler, drum rack, instrument rack), MIDI
                       effects and audio effects, all retunable in place; warping (time-stretch)
crates/soniq-midi      MIDI inputs and controllers (midir) and Standard MIDI Files (midly)
crates/soniq-host      CLAP and VST3 plugin hosting: each plugin in a sandbox process (the app started
                       with --plugin-sandbox), audio through shared memory; four test plugins as examples
crates/soniq-engine    real-time renderer, tempo map, warped clips, instruments, live notes, automation
                       and modulators, groups, session clip launching, transport, meters, devices,
                       recorder, monitoring, offline render
src-tauri/             Tauri app: session, graph building, launching, warping and slicing, plugins,
                       freezing, commands, processing, MIDI, files, autosave
shared/                device catalog read by both Rust and the UI
src/                   React UI; src/engine/backend.ts talks to Rust
```

`npm run dev` without Tauri uses a small in-browser mock of the backend (`src/engine/previewBackend.ts`),
so you can work on the UI without building Rust. File access and real audio need the desktop app.

```bash
cargo test --workspace
```

MP3 export links the LAME encoder (LGPL). That is fine for personal use and open-source releases; a
closed-source release would need to ship LAME as a separate, replaceable library (ROADMAP §9).
Everything builds without CMake: libvorbis and LAME compile with the MSVC tools, and Opus comes from a
pure-Rust transpilation of libopus.
