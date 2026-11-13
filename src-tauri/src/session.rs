//! The open song: project model, undo history and the pool of decoded audio,
//! plus how it is turned into an engine graph and sent to the UI.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use serde::Serialize;
use soniq_dsp::Key;
use soniq_engine::{
    AudioEngine, ClipNode, EngineConfig, Graph, NoteEvent, PeakMeter, Recorder, ReturnNode, TrackNode, crossfade_overlaps, db_to_gain, track_gains, track_key,
};
use soniq_io::{AudioBuffer, NoiseProfile, PEAK_STEP};
use soniq_midi::{MidiEvent, MidiInputs};
use soniq_project::{Clip, History, MappingTarget, Monitor, Op, Project, Track, TrackKind, controller_op};

use crate::build::{Timeline, audio_node, controls, fx_chain, instrument_node, slot_nodes};

/// One asset's audio, ready for playback and drawing.
pub struct PoolAsset {
    /// As decoded, at the file's own rate. Edits work on this.
    pub native: Arc<AudioBuffer>,
    /// Resampled to the engine rate. The audio thread plays this.
    pub playback: Arc<AudioBuffer>,
    pub peaks: Arc<Vec<Vec<f32>>>,
}

impl PoolAsset {
    /// CPU-heavy (resampling, peaks): call it without holding the session lock.
    pub fn prepare(native: AudioBuffer, engine_rate: u32) -> Self {
        let peaks = soniq_io::peaks(&native, PEAK_STEP);
        let native = Arc::new(native);
        let playback = if native.sample_rate == engine_rate {
            Arc::clone(&native)
        } else {
            Arc::new(soniq_io::resample(&native, engine_rate))
        };
        Self { native, playback, peaks: Arc::new(peaks) }
    }
}

/// Transients found in playback buffers, by asset, sensitivity (0–100) and how many at most.
type OnsetCache = HashMap<(String, u32, usize), Arc<Vec<usize>>>;

/// How sensitive warping in Beats mode is to hits.
const WARP_SENSITIVITY: u32 = 60;

#[derive(Default)]
pub struct Pool {
    assets: HashMap<String, PoolAsset>,
    onsets: Mutex<OnsetCache>,
    /// Audio made from an asset for clips that play it backwards or at
    /// another pitch (cents), at the engine rate.
    variants: Mutex<HashMap<(String, i32, bool), Arc<AudioBuffer>>>,
}

/// Variants kept at most before the ones no graph plays are let go.
const VARIANTS_KEPT: usize = 8;

impl Pool {
    pub fn insert(&mut self, id: String, asset: PoolAsset) {
        lock(&self.onsets).retain(|(asset, _, _), _| *asset != id && !asset.starts_with(&format!("{id}#")));
        lock(&self.variants).retain(|(asset, _, _), _| *asset != id);
        self.assets.insert(id, asset);
    }

    /// The asset's audio as a clip plays it: as it is, shifted in pitch by
    /// `cents` (same length), backwards, or both. Made once and kept while a
    /// graph plays it.
    pub fn variant(&self, id: &str, cents: i32, reverse: bool) -> Option<Arc<AudioBuffer>> {
        let asset = self.assets.get(id)?;
        if cents == 0 && !reverse {
            return Some(Arc::clone(&asset.playback));
        }
        let mut cache = lock(&self.variants);
        let key = (id.to_string(), cents, reverse);
        if !cache.contains_key(&key) {
            if cache.len() >= VARIANTS_KEPT {
                cache.retain(|_, audio| Arc::strong_count(audio) > 1);
            }
            let mut audio = if cents == 0 { (*asset.playback).clone() } else { soniq_io::change_pitch(&asset.playback, f64::from(cents) / 100.0) };
            if reverse {
                audio.channels.iter_mut().for_each(|c| c.reverse());
            }
            cache.insert(key.clone(), Arc::new(audio));
        }
        cache.get(&key).cloned()
    }

    /// Every hit in a variant of an asset, for warping it in Beats mode.
    pub fn variant_onsets(&self, id: &str, cents: i32, reverse: bool) -> Option<Arc<Vec<usize>>> {
        if cents == 0 && !reverse {
            return self.warp_onsets(id);
        }
        let audio = self.variant(id, cents, reverse)?;
        let mut cache = lock(&self.onsets);
        let found = cache
            .entry((format!("{id}#{cents}#{reverse}"), WARP_SENSITIVITY, usize::MAX))
            .or_insert_with(|| Arc::new(soniq_io::onsets(&audio, WARP_SENSITIVITY as f32 / 100.0, usize::MAX)));
        Some(Arc::clone(found))
    }

    pub fn get(&self, id: &str) -> Option<&PoolAsset> {
        self.assets.get(id)
    }

    /// Where the hits in an asset start, at the engine rate, as many as a
    /// sampler slices; found once per sensitivity.
    pub fn onsets(&self, id: &str, sensitivity: u32) -> Option<Arc<Vec<usize>>> {
        self.find_onsets(id, sensitivity, soniq_dsp::MAX_SLICES)
    }

    /// Every hit in an asset, for warping in Beats mode.
    pub fn warp_onsets(&self, id: &str) -> Option<Arc<Vec<usize>>> {
        self.find_onsets(id, WARP_SENSITIVITY, usize::MAX)
    }

    fn find_onsets(&self, id: &str, sensitivity: u32, max: usize) -> Option<Arc<Vec<usize>>> {
        let asset = self.assets.get(id)?;
        let mut cache = lock(&self.onsets);
        let found = cache
            .entry((id.to_string(), sensitivity, max))
            .or_insert_with(|| Arc::new(soniq_io::onsets(&asset.playback, sensitivity as f32 / 100.0, max)));
        Some(Arc::clone(found))
    }

    pub fn retain(&mut self, keep: &HashSet<&str>) {
        self.assets.retain(|id, _| keep.contains(id.as_str()));
        lock(&self.onsets).retain(|(id, _, _), _| keep.contains(id.as_str()));
    }

    /// After the device rate changes, every playback buffer is rebuilt.
    pub fn resample_all(&mut self, rate: u32) {
        lock(&self.onsets).clear();
        for asset in self.assets.values_mut() {
            asset.playback = if asset.native.sample_rate == rate {
                Arc::clone(&asset.native)
            } else {
                Arc::new(soniq_io::resample(&asset.native, rate))
            };
        }
    }
}

pub struct Session {
    pub project: Project,
    pub history: History,
    pub pool: Pool,
    /// The .soniq file this session was opened from or saved to.
    pub path: Option<PathBuf>,
    /// Unsaved changes since the last save.
    pub dirty: bool,
    /// Changes since the last autosave.
    pub autosave_pending: bool,
    /// Audio copied or cut in the editor.
    pub clipboard: Option<Arc<AudioBuffer>>,
}

impl Session {
    pub fn new(project: Project, pool: Pool, path: Option<PathBuf>) -> Self {
        Self { project, history: History::new(), pool, path, dirty: false, autosave_pending: false, clipboard: None }
    }

    pub fn mark_changed(&mut self) {
        self.dirty = true;
        self.autosave_pending = true;
    }

    /// Frees audio that neither the project nor any undo step refers to.
    pub fn collect_garbage(&mut self) {
        let Session { project, history, pool, .. } = self;
        let keep: HashSet<&str> = std::iter::once(&*project)
            .chain(history.snapshots())
            .flat_map(|p| p.assets.iter().map(|a| a.id.as_str()))
            .collect();
        pool.retain(&keep);
    }
}

pub struct Dirs {
    pub config: PathBuf,
    pub data: PathBuf,
}

impl Dirs {
    pub fn autosave(&self) -> PathBuf {
        self.data.join("autosave")
    }

    /// The user's own song templates.
    pub fn templates(&self) -> PathBuf {
        self.config.join("templates")
    }

    pub fn recovered(&self) -> PathBuf {
        self.data.join("autosave-recovered")
    }

    /// Crash reports, when the user keeps them.
    pub fn crashes(&self) -> PathBuf {
        self.data.join("crash-reports")
    }

    /// Takes in progress, and recordings of projects that were never saved.
    pub fn recordings(&self) -> PathBuf {
        self.data.join("recordings")
    }
}

/// Where live notes go: every armed MIDI track, or else the selected one.
#[derive(Default)]
pub struct MidiRouting {
    /// (track id, engine key) of armed MIDI tracks.
    pub armed: Vec<(String, u64)>,
    pub selected: Option<(String, u64)>,
    /// The tracks each held key went to, so its note-off follows even if
    /// the routing changed in between.
    held: HashMap<u8, Vec<u64>>,
}

impl MidiRouting {
    fn targets(&self) -> Vec<u64> {
        if self.armed.is_empty() { self.selected.iter().map(|(_, k)| *k).collect() } else { self.armed.iter().map(|(_, k)| *k).collect() }
    }
}

/// Notes played while recording, by transport time in seconds.
pub struct MidiTake {
    pub start: f64,
    pub tracks: Vec<String>,
    pub events: Vec<(f64, NoteEvent)>,
    /// The loop range (seconds) while recording loops over it.
    pub loop_range: Option<(f64, f64)>,
    pub mode: crate::midi::MidiRecordMode,
}

/// A note played live, kept for Capture MIDI.
#[derive(Debug, Clone, Copy)]
pub struct HeardNote {
    /// Seconds since the app started.
    pub wall: f64,
    /// The song was playing, and where (seconds) the player heard it.
    pub playing: bool,
    pub at: f64,
    pub event: NoteEvent,
}

/// How many recent live notes Capture MIDI can reach back to.
pub const HISTORY: usize = 4096;

pub struct AppState {
    pub session: Mutex<Session>,
    pub engine: AudioEngine,
    pub recorder: Recorder,
    /// Audio tracks being recorded onto, fixed when recording starts.
    pub record_tracks: Mutex<Vec<String>>,
    /// MIDI being recorded, with the armed MIDI tracks it goes to.
    pub midi_take: Mutex<Option<MidiTake>>,
    pub midi_routing: Mutex<MidiRouting>,
    /// Hardware MIDI inputs, once started.
    pub midi: Mutex<Option<MidiInputs>>,
    /// Counts incoming notes, so the UI can flash a MIDI activity light.
    pub midi_activity: AtomicU64,
    /// Recent live notes, recording or not (Capture MIDI).
    pub midi_history: Mutex<VecDeque<HeardNote>>,
    pub started: Instant,
    /// Punch range of the running recording, in seconds.
    pub punch: Mutex<Option<(f64, f64)>>,
    /// Sketch mode: the loop (seconds) whose next full pass the recording keeps.
    pub pass: Mutex<Option<(f64, f64)>>,
    /// Loop recording: the loop (seconds) whose passes become takes.
    pub record_loop: Mutex<Option<(f64, f64)>>,
    /// Session view: the slot (track, scene) the recording goes into instead of the timeline.
    pub record_slot: Mutex<Option<(String, usize)>>,
    /// Learned by "Get noise profile"; kept across projects, like Audacity.
    pub noise_profile: Mutex<Option<NoiseProfile>>,
    pub engine_config: Mutex<EngineConfig>,
    pub dirs: Dirs,
    pub recovery_available: AtomicBool,
    /// Bumped on every meter subscription so a reloaded UI stops the old stream.
    pub meter_generation: AtomicU64,
    /// What the next MIDI controller moved gets mapped to (MIDI learn).
    pub learning: Mutex<Option<MappingTarget>>,
    /// Controller moves (channel, cc, value) for the worker that applies them.
    pub controls: Sender<(u8, u8, u8)>,
    /// Bumped when the project changes without the UI asking (a MIDI
    /// controller moved something), so the UI fetches it again.
    pub revision: AtomicU64,
    /// What the session plays while recording, for the arrangement.
    pub session_take: Mutex<Option<crate::scenes::SessionTake>>,
    pub plugins: crate::plugins::Plugins,
    /// Loudness, peaks and the spectrum of the master, while the meters panel is open.
    pub analyzer: Mutex<Option<soniq_io::Analyzer>>,
    /// The device whose output its panel shows as a live spectrum, its queue, and the meter.
    pub tap: Mutex<Option<(String, rtrb::Consumer<f32>, soniq_io::SpectrumMeter)>>,
    /// The browser's folders of samples and presets.
    pub library: Arc<Mutex<crate::library::Library>>,
}

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl AppState {
    /// Rebuilds the engine graph from the project. Cheap: no audio is copied.
    pub fn sync_engine(&self, session: &Session) {
        let project = &session.project;
        // Plugin devices without a running plugin get one (in the background).
        self.plugins.ensure(project, f64::from(self.engine.sample_rate()));
        let transport = self.engine.transport();
        transport.set_bpm(project.bpm);
        transport.set_beats_per_bar(project.beats_per_bar);
        let metered = project.tracks.iter().map(|t| t.id.as_str()).chain(project.returns.iter().map(|r| r.id.as_str()));
        self.engine.meters().retain(&metered.collect());
        let devices = project.tracks.iter().flat_map(|t| &t.devices).chain(project.returns.iter().flat_map(|r| &r.devices)).chain(&project.master_devices);
        self.engine.meters().retain_devices(&devices.map(|d| d.id.as_str()).collect());
        self.engine.set_graph(self.build_graph(session, true));
        self.sync_input(project);
        let mut routing = lock(&self.midi_routing);
        routing.armed = project.tracks.iter().filter(|t| t.arm && t.kind == TrackKind::Midi).map(|t| (t.id.clone(), track_key(&t.id))).collect();
        if routing.selected.as_ref().is_some_and(|(id, _)| project.track(id).is_none_or(|t| t.kind != TrackKind::Midi)) {
            routing.selected = None;
        }
    }

    /// Points live notes at the selected track when no MIDI track is armed.
    pub fn select_track(&self, id: Option<&str>) {
        let midi = id.filter(|id| lock(&self.session).project.track(id).is_some_and(|t| t.kind == TrackKind::Midi));
        lock(&self.midi_routing).selected = midi.map(|id| (id.to_string(), track_key(id)));
    }

    /// Everything a MIDI input sends: notes play, controllers move what they are mapped to.
    pub fn midi_in(&self, event: MidiEvent) {
        match (event, crate::midi::note_event(event)) {
            (MidiEvent::Control { channel, cc, value }, _) => self.route_control(channel, cc, value),
            (_, Some(note)) => self.route_note(note),
            (_, None) => {}
        }
    }

    /// A knob, fader or button moved. While learning, it gets mapped to the
    /// learn target (on any channel); otherwise its move is queued for
    /// [`apply_controls`](Self::apply_controls).
    pub fn route_control(&self, channel: u8, cc: u8, value: u8) {
        self.midi_activity.fetch_add(1, Ordering::Relaxed);
        if let Some(target) = lock(&self.learning).take() {
            let mut session = lock(&self.session);
            let session = &mut *session;
            if soniq_project::apply(&mut session.project, &mut session.history, Op::AddMapping { channel: None, cc, target }).is_ok() {
                session.mark_changed();
                self.revision.fetch_add(1, Ordering::Relaxed);
            }
            return;
        }
        // The worker only stops when the app does.
        let _ = self.controls.send((channel, cc, value));
    }

    /// Moves what each control (channel, cc) is mapped to, to its latest
    /// value. Each move is an undo step that merges with the next move of
    /// the same target, like dragging a knob on screen.
    pub fn apply_controls(&self, moves: &HashMap<(u8, u8), u8>) {
        let mut session = lock(&self.session);
        let session = &mut *session;
        let mut changed = false;
        for (&(channel, cc), &value) in moves {
            let targets: Vec<MappingTarget> =
                session.project.mappings.iter().filter(|m| m.cc == cc && m.channel.is_none_or(|c| c == channel)).map(|m| m.target.clone()).collect();
            for target in targets {
                if let Some(op) = controller_op(&session.project, &target, value)
                    && soniq_project::apply(&mut session.project, &mut session.history, op).is_ok()
                {
                    changed = true;
                }
            }
        }
        if changed {
            session.mark_changed();
            self.sync_engine(session);
            self.revision.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Plays a note from a MIDI input or the computer keyboard on the armed
    /// MIDI tracks (or the selected one), and records it while recording.
    pub fn route_note(&self, event: NoteEvent) {
        self.midi_activity.fetch_add(1, Ordering::Relaxed);
        let targets = {
            let mut routing = lock(&self.midi_routing);
            let current = routing.targets();
            match event {
                NoteEvent::On { pitch, .. } => {
                    routing.held.insert(pitch, current.clone());
                    current
                }
                NoteEvent::Off { pitch } => routing.held.remove(&pitch).unwrap_or(current),
                _ => {
                    let mut all = current;
                    all.extend(routing.held.values().flatten());
                    all.sort_unstable();
                    all.dedup();
                    all
                }
            }
        };
        for key in targets {
            self.engine.play_note(key, event);
        }
        // What the player heard when they played: the engine renders ahead by the output latency.
        let transport = self.engine.transport();
        let at = (transport.position_seconds() - self.engine.output_latency()).max(0.0);
        if let Some(take) = lock(&self.midi_take).as_mut() {
            take.events.push((if take.loop_range.is_some() { at } else { at.max(take.start) }, event));
        }
        if matches!(event, NoteEvent::On { .. } | NoteEvent::Off { .. } | NoteEvent::Sustain(_)) {
            let mut history = lock(&self.midi_history);
            if history.len() == HISTORY {
                history.pop_front();
            }
            history.push_back(HeardNote { wall: self.started.elapsed().as_secs_f64(), playing: transport.is_playing(), at, event });
        }
    }

    /// What the master's analysis reads now, while it runs.
    pub fn analyse(&self) -> Option<soniq_io::Reading> {
        let mut slot = lock(&self.analyzer);
        let analyzer = slot.as_mut()?;
        let rate = self.engine.sample_rate();
        if analyzer.sample_rate() != rate {
            *analyzer = soniq_io::Analyzer::new(rate);
        }
        self.engine.drain_analysis(|first, second| {
            analyzer.push_interleaved(first);
            analyzer.push_interleaved(second);
        });
        Some(analyzer.reading())
    }

    /// The mixer graph for the project at the engine rate. Offline renders
    /// pass `live_meters: false` so they do not move the meters on screen.
    pub fn build_graph(&self, session: &Session, live_meters: bool) -> Graph {
        self.graph_of(&session.project, &session.pool, live_meters)
    }

    /// The graph for any version of the song (a freeze renders a changed copy).
    pub fn graph_of(&self, project: &Project, pool: &Pool, live_meters: bool) -> Graph {
        let rate = f64::from(self.engine.sample_rate());
        let tempo = project.tempo_map();
        let timeline = Timeline { tempo: &tempo, rate, beats_per_bar: project.beats_per_bar };
        let meters = self.engine.meters();
        let song_key = Key::parse(&project.key);
        let index: HashMap<&str, usize> = project.tracks.iter().enumerate().map(|(i, t)| (t.id.as_str(), i)).collect();
        let groups: HashMap<&str, usize> = project.tracks.iter().filter(|t| t.kind == TrackKind::Group).enumerate().map(|(i, t)| (t.id.as_str(), i)).collect();
        let meter = |id: &str| if live_meters { meters.track(id) } else { Arc::new(PeakMeter::default()) };
        // Soloing a group plays its tracks; soloing a track in a group plays the group.
        let soloed: HashSet<&str> = project.tracks.iter().filter(|t| t.solo).flat_map(|t| std::iter::once(t.id.as_str()).chain(t.group.as_deref())).collect();
        let any_solo = !soloed.is_empty();
        let solo = |t: &Track| soloed.contains(t.id.as_str()) || t.group.as_deref().is_some_and(|g| project.tracks.iter().any(|p| p.id == g && p.solo));
        // An export's plugin nodes have their plugins to themselves while it runs.
        let plugin = |d: &soniq_project::Device, instrument: bool| self.plugins.processor(d, instrument, live_meters);

        let tracks = project
            .tracks
            .iter()
            .map(|track| {
                // A frozen track plays its rendered sound from the start of the song while its devices rest.
                let frozen = track.frozen.as_deref().and_then(|id| project.asset(id)).map(|asset| {
                    let mut clip = Clip::audio("", asset.id.clone(), 0.0, 0.0, asset.duration());
                    clip.id = format!("{}:frozen", track.id);
                    clip
                });
                let mut clips: Vec<ClipNode> = match &frozen {
                    Some(clip) => audio_node(clip, pool, &timeline, false).into_iter().collect(),
                    None => track.timeline().filter_map(|clip| audio_node(clip, pool, &timeline, false)).collect(),
                };
                clips.sort_by_key(|c| c.start);
                crossfade_overlaps(&mut clips);
                let audible = !track.mute && (!any_solo || solo(track));
                let mut node = TrackNode::new(track_gains(track.volume, track.pan, audible), clips, meter(&track.id));
                node.key = track_key(&track.id);
                node.mixer = (track.volume, track.pan, audible);
                // Offline renders never hear the input.
                if live_meters && track.monitoring() && frozen.is_none() {
                    node.monitor = Some(track.input_channels());
                }
                if frozen.is_none() {
                    if track.kind == TrackKind::Midi {
                        node.instrument = instrument_node(track, pool, &timeline, song_key, &plugin);
                    }
                    node.fx = fx_chain(&track.devices, rate as u32, &index, &plugin, pool);
                    node.slots = slot_nodes(track, pool, &timeline);
                }
                // A silent track sends nothing, before or after its fader.
                node.sends = (0..project.returns.len()).map(|i| if audible { track.sends.get(i).copied().unwrap_or(0.0) as f32 } else { 0.0 }).collect();
                (node.lanes, node.mods) = controls(track, &node.fx, node.instrument.as_ref(), &timeline);
                node.group = groups.get(track.id.as_str()).copied();
                node.output = track.group.as_deref().and_then(|g| groups.get(g).copied());
                if live_meters {
                    node.status = meters.clip_status(&track.id);
                }
                node
            })
            .collect();

        let mut graph = Graph::new(tracks, db_to_gain(project.master_volume)).with_groups(groups.len()).with_tempo(timeline.engine_map());
        graph.returns = project
            .returns
            .iter()
            .map(|r| ReturnNode::new(track_key(&r.id), track_gains(r.volume, r.pan, !r.mute), r.pre_fader, fx_chain(&r.devices, rate as u32, &index, &plugin, pool), meter(&r.id)))
            .collect();
        graph.master_fx = fx_chain(&project.master_devices, rate as u32, &index, &plugin, pool);
        // Dynamics devices show how far they turn the sound down.
        if live_meters {
            for node in graph.fx_nodes_mut().filter(|n| n.fx.reduction().is_some()) {
                node.meter = Some(meters.device(&node.id));
            }
        }
        // Plugins that hold their sound back are made up for everywhere else.
        graph.compensate();
        // The click's accents follow the time signatures.
        if !project.meters.is_empty() {
            graph.meters = project.meter_map().segments().iter().map(|s| (s.start.round() as i64, i64::from(s.beats_per_bar))).collect();
        }
        graph
    }

    /// The input is open exactly while an audio track is armed or monitors
    /// its input, so the system's microphone indicator is only on when it
    /// should be.
    fn sync_input(&self, project: &Project) {
        let audio = || project.tracks.iter().filter(|t| t.kind == TrackKind::Audio);
        let needed = audio().any(|t| t.arm || t.monitor == Monitor::In);
        if needed && !self.recorder.is_open() {
            if let Err(err) = self.open_input() {
                eprintln!("could not open the input: {err}");
            }
        } else if !needed && self.recorder.is_open() && !self.recorder.is_recording() {
            self.close_input();
        }
        self.recorder.set_monitoring(audio().any(|t| t.monitoring()));
    }

    /// Opens the configured input and hands its monitoring feed to the
    /// engine, which brings it to the output's rate and clock.
    pub fn open_input(&self) -> Result<(), String> {
        let config = lock(&self.engine_config).clone();
        self.recorder.open(&config, self.engine.transport_handle())?;
        self.engine.set_monitor(self.recorder.take_monitor_feed().map(|(feed, _)| feed));
        Ok(())
    }

    /// Some audio track is armed or always monitors, so the input should be open.
    pub fn input_wanted(&self) -> bool {
        let session = lock(&self.session);
        session.project.tracks.iter().any(|t| t.kind == TrackKind::Audio && (t.arm || t.monitor == Monitor::In))
    }

    pub fn close_input(&self) {
        self.recorder.close();
        self.engine.set_monitor(None);
    }

    pub fn reply(&self, session: &Session) -> ProjectReply {
        ProjectReply {
            project: session.project.clone(),
            can_undo: session.history.undo_label().is_some(),
            can_redo: session.history.redo_label().is_some(),
            undo_label: session.history.undo_label().map(str::to_string),
            redo_label: session.history.redo_label().map(str::to_string),
            dirty: session.dirty,
            path: session.path.as_ref().map(|p| p.display().to_string()),
            created: None,
            warnings: Vec::new(),
            clipped: false,
            note: None,
            plugins: self.plugins.statuses(&session.project),
            plugin_latency_ms: self.engine.plugin_latency() * 1000.0,
        }
    }

    /// Swaps in a whole new session (new, open, recover, demo).
    pub fn replace_session(&self, next: Session) -> ProjectReply {
        self.plugins.clear();
        let mut session = lock(&self.session);
        *session = next;
        self.sync_engine(&session);
        self.reply(&session)
    }
}

/// Everything the UI needs after any change.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectReply {
    pub project: Project,
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    pub dirty: bool,
    pub path: Option<String>,
    pub created: Option<String>,
    pub warnings: Vec<String>,
    pub clipped: bool,
    /// What an effect found or did ("Removed 3 clicks").
    pub note: Option<String>,
    /// How each plugin device's plugin is doing, by device id.
    pub plugins: HashMap<String, crate::plugins::PluginStatus>,
    /// How far the plugins' compensation holds the whole mix back.
    pub plugin_latency_ms: f64,
}
