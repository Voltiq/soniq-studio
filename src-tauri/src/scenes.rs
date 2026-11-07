//! The session view: launching clips and scenes, and recording what the
//! session plays into the arrangement.

use std::collections::HashMap;

use soniq_engine::{Launched, track_key};
use soniq_project::{Project, record_slot};
use tauri::State;

use crate::session::{AppState, lock};

type CmdResult<T> = Result<T, String>;

/// What the session played while recording.
pub struct SessionTake {
    /// The timeline sample recording started at.
    start: u64,
    /// The clips already playing then: track key, scene, beats into the loop.
    playing: Vec<(u64, usize, f64)>,
    events: Vec<Launched>,
}

impl AppState {
    /// Moves what the session started and stopped into the running take.
    /// Called often (the meter loop does), so the engine's queue never fills.
    pub fn collect_launched(&self) {
        let events = self.engine.take_launched();
        if let Some(take) = lock(&self.session_take).as_mut() {
            take.events.extend(events);
        }
    }

    pub fn begin_session_take(&self) {
        let _stale = self.engine.take_launched();
        let session = lock(&self.session);
        let project = &session.project;
        let tempo = project.tempo_map();
        let playing = self
            .engine
            .meters()
            .clip_states()
            .into_iter()
            .filter_map(|(id, state)| {
                let scene = state.playing?;
                let clip = project.track(&id)?.slot(scene)?;
                Some((track_key(&id), scene, f64::from(state.progress) * clip.length_beats(&tempo)))
            })
            .collect();
        *lock(&self.session_take) = Some(SessionTake { start: self.engine.transport().position_samples(), playing, events: Vec::new() });
    }

    /// Ends the take at timeline sample `stop` and writes each clip the
    /// session played into the arrangement, over what was there. Returns how
    /// many clips it placed.
    pub fn end_session_take(&self, project: &mut Project, stop: u64) -> usize {
        self.collect_launched();
        let Some(take) = lock(&self.session_take).take() else { return 0 };
        let (tempo, rate) = (project.tempo_map(), f64::from(self.engine.sample_rate()));
        let beats = |s: u64| tempo.beats_at(s as f64 / rate);
        let ids: HashMap<u64, String> = project.tracks.iter().map(|t| (track_key(&t.id), t.id.clone())).collect();
        // Per track: the clip playing, since when, and how far into its loop it started.
        let mut open: HashMap<u64, (usize, u64, f64)> = take.playing.iter().map(|&(track, scene, offset)| (track, (scene, take.start, offset))).collect();
        let mut played = Vec::new();
        for e in &take.events {
            if let Some((scene, from, offset)) = open.remove(&e.track) {
                played.push((e.track, scene, from, e.at, offset));
            }
            if let Some(scene) = e.scene {
                open.insert(e.track, (scene, e.at, 0.0));
            }
        }
        played.extend(open.into_iter().map(|(track, (scene, from, offset))| (track, scene, from, stop, offset)));
        played
            .into_iter()
            .filter(|(_, _, from, to, _)| to > from)
            .filter_map(|(track, scene, from, to, offset)| Some(record_slot(project, ids.get(&track)?, scene, beats(from), beats(to), offset)))
            .sum()
    }
}

/// Starts the clip in a scene's slot on a track (or stops the track with no
/// scene) on the next launch line. Launching starts the transport (the audio
/// thread does, so the clip starts on the first sample played).
#[tauri::command]
pub fn session_launch(state: State<'_, AppState>, track: String, scene: Option<usize>) -> CmdResult<()> {
    let quantize = lock(&state.session).project.launch_quantize;
    state.engine.launch(track_key(&track), scene, quantize)
}

/// Every track plays its clip in the scene; tracks without one stop.
#[tauri::command]
pub fn session_launch_scene(state: State<'_, AppState>, scene: usize) -> CmdResult<()> {
    let quantize = lock(&state.session).project.launch_quantize;
    state.engine.launch_scene(scene, quantize)
}

#[tauri::command]
pub fn session_stop_clips(state: State<'_, AppState>) -> CmdResult<()> {
    let quantize = lock(&state.session).project.launch_quantize;
    state.engine.stop_clips(quantize)
}

#[tauri::command]
pub fn session_back_to_arrangement(state: State<'_, AppState>) -> CmdResult<()> {
    state.engine.back_to_arrangement()
}
