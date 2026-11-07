//! Songs to start from: Soniq's own templates, and the user's (a song saved
//! without its clips and audio, in the config folder).

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use soniq_project::{Clip, Device, Monitor, Note, Project, Track, TrackKind, TrackRole, catalog, new_id};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    /// One of Soniq's own (the user's can be deleted).
    pub builtin: bool,
    /// The view the song opens in.
    pub view: String,
}

const BUILTIN: [(&str, &str, &str, &str); 5] = [
    ("empty", "Empty song", "Two audio and two MIDI tracks, nothing on them.", "arrange"),
    ("beat", "Beat", "Drums with a pattern to start from, a bass synth and keys, at 96 BPM.", "arrange"),
    ("band", "Band recording", "Vocals, guitar, bass and drums ready to record, the vocal with a cleanup chain.", "arrange"),
    ("podcast", "Podcast", "Host and guest with a voice chain, music that ducks under either of them, a limiter on the master.", "arrange"),
    ("editing", "Audio editing", "One track to record into or import a file, opened in the audio editor.", "editor"),
];

/// A user template's id: `user:` and its file name.
const USER: &str = "user:";

pub fn list(dir: &Path) -> Vec<TemplateInfo> {
    let builtin = BUILTIN.iter().map(|(id, name, description, view)| TemplateInfo {
        id: id.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        builtin: true,
        view: view.to_string(),
    });
    let mut own: Vec<TemplateInfo> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            (path.extension()? == "soniq").then_some(())?;
            let file = path.file_name()?.to_str()?.to_string();
            Some(TemplateInfo {
                id: format!("{USER}{file}"),
                name: path.file_stem()?.to_string_lossy().into_owned(),
                description: "Your template: its tracks, devices, sends and tempo.".into(),
                builtin: false,
                view: "arrange".into(),
            })
        })
        .collect();
    own.sort_by_key(|t| t.name.to_lowercase());
    builtin.chain(own).collect()
}

fn user_file(dir: &Path, id: &str) -> Option<PathBuf> {
    let file = id.strip_prefix(USER)?;
    // A file name only: nothing outside the templates folder.
    (Path::new(file).file_name()? == file).then(|| dir.join(file))
}

/// A new song from a template, called "Untitled".
pub fn build(id: &str, dir: &Path) -> Result<Project, String> {
    let mut project = match id {
        "empty" => Project::empty("Untitled"),
        "beat" => beat(),
        "band" => band(),
        "podcast" => podcast(),
        "editing" => editing(),
        // Sketch mode's song: no tracks yet, each layer adds one.
        "sketch" => {
            let mut p = blank();
            p.bpm = 100.0;
            p
        }
        _ => {
            let path = user_file(dir, id).ok_or("There is no such template.")?;
            crate::files::read_project(&path)?
        }
    };
    project.id = new_id("p");
    project.name = "Untitled".into();
    Ok(project)
}

/// Saves `project` as a template called `name`: its tracks, devices,
/// routing, mixer and tempo, without clips, audio, markers or saved mixes.
pub fn save(dir: &Path, name: &str, project: &Project) -> Result<TemplateInfo, String> {
    let clean: String = name.chars().filter(|c| !"\\/:*?\"<>|".contains(*c)).collect();
    let clean = clean.trim();
    if clean.is_empty() {
        return Err("Give the template a name.".into());
    }
    let mut template = project.clone();
    for track in &mut template.tracks {
        track.clips.clear();
        track.slots.iter_mut().for_each(|s| *s = None);
        track.frozen = None;
        track.arm = false;
        clear_samples(&mut track.devices);
    }
    template.assets.clear();
    template.markers.clear();
    template.mixes.clear();
    template.other_mix = None;
    fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let path = dir.join(format!("{clean}.soniq"));
    let json = serde_json::to_string_pretty(&template).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    Ok(TemplateInfo {
        id: format!("{USER}{clean}.soniq"),
        name: clean.to_string(),
        description: "Your template: its tracks, devices, sends and tempo.".into(),
        builtin: false,
        view: "arrange".into(),
    })
}

pub fn delete(dir: &Path, id: &str) -> Result<(), String> {
    let path = user_file(dir, id).ok_or("Only your own templates can be deleted.")?;
    fs::remove_file(&path).map_err(|e| format!("Could not delete {}: {e}", path.display()))
}

/// Samples loaded into samplers and drum pads go with the audio.
fn clear_samples(devices: &mut [Device]) {
    for device in devices {
        device.samples.clear();
        for chain in &mut device.chains {
            clear_samples(std::slice::from_mut(&mut chain.device));
        }
    }
}

fn devices(list: &[(&str, &[(&str, f64)])]) -> Vec<Device> {
    list.iter().filter_map(|(kind, params)| catalog::device(kind, params)).collect()
}

fn track(name: &str, kind: TrackKind, color: u8) -> Track {
    Track::new(new_id("t"), name, kind, color)
}

/// An empty song with no tracks yet.
fn blank() -> Project {
    let mut project = Project::empty("Untitled");
    project.tracks.clear();
    project
}

fn beat() -> Project {
    let mut project = blank();
    project.bpm = 96.0;
    let hit = |start: f64, pitch: u8, velocity: f64| Note { start, length: 0.25, pitch, velocity };
    let mut pattern: Vec<Note> = Vec::new();
    for bar in 0..2 {
        let o = f64::from(bar) * 4.0;
        pattern.extend([0.0, 1.75, 2.5].map(|k| hit(o + k, 36, 0.95)));
        pattern.extend([1.0, 3.0].map(|s| hit(o + s, 38, 0.85)));
        pattern.extend((0..8).map(|h| hit(o + f64::from(h) * 0.5, 42, if h % 2 == 1 { 0.45 } else { 0.7 })));
    }
    let mut drums = track("Drums", TrackKind::Midi, 2);
    drums.volume = -6.0;
    drums.devices = devices(&[("drum-rack", &[]), ("comp", &[("thresh", -14.0), ("ratio", 3.0)])]);
    drums.clips = vec![Clip::midi("Beat", 0.0, 8.0, pattern)];
    let mut bass = track("Bass", TrackKind::Midi, 1);
    bass.volume = -6.0;
    let bass_sound: &[(&str, f64)] = &[("osc2", 1.0), ("osc2Semi", -12.0), ("mix", 35.0), ("sub", 30.0), ("cutoff", 480.0), ("res", 25.0), ("envAmt", 50.0), ("mode", 1.0)];
    bass.devices = devices(&[("synth", bass_sound), ("saturator", &[("drive", 8.0)])]);
    let mut keys = track("Keys", TrackKind::Midi, 6);
    keys.volume = -9.0;
    keys.devices = devices(&[("epiano", &[])]);
    keys.sends = [0.25, 0.1];
    let audio = track("Audio", TrackKind::Audio, 3);
    project.tracks = vec![drums, bass, keys, audio];
    project
}

/// A vocal chain: low cut, compressor, de-esser.
fn voice_chain() -> Vec<Device> {
    devices(&[
        ("eq", &[("b1on", 1.0), ("b1freq", 90.0)]),
        ("comp", &[("thresh", -18.0), ("ratio", 3.0), ("attack", 5.0), ("release", 120.0)]),
        ("deesser", &[]),
    ])
}

fn recording(name: &str, color: u8, input: &str) -> Track {
    let mut t = track(name, TrackKind::Audio, color);
    t.input = input.into();
    // Heard while armed; arming is still the user's choice.
    t.monitor = Monitor::Auto;
    t
}

fn band() -> Project {
    let mut project = blank();
    let mut vocals = recording("Vocals", 5, "1");
    vocals.devices = voice_chain();
    vocals.sends = [0.2, 0.1];
    let mut guitar = recording("Guitar", 3, "2");
    guitar.devices = devices(&[("eq", &[("b1on", 1.0), ("b1freq", 100.0)]), ("comp", &[("thresh", -16.0), ("ratio", 2.5)])]);
    guitar.sends = [0.12, 0.0];
    let mut bass = recording("Bass", 1, "2");
    bass.devices = devices(&[("comp", &[("thresh", -18.0), ("ratio", 4.0)])]);
    let mut drums = recording("Drums", 2, "1/2");
    drums.devices = devices(&[("glue", &[])]);
    project.tracks = vec![vocals, guitar, bass, drums];
    project
}

fn podcast() -> Project {
    let mut project = blank();
    let mut host = recording("Host", 5, "1");
    host.devices = voice_chain();
    host.role = TrackRole::Speaker;
    let mut guest = recording("Guest", 6, "2");
    guest.devices = voice_chain();
    guest.role = TrackRole::Speaker;
    let mut music = track("Music", TrackKind::Audio, 3);
    music.volume = -10.0;
    music.role = TrackRole::Music;
    // One compressor listening to each voice: the music ducks under either.
    let duck: &[(&str, f64)] = &[("thresh", -36.0), ("ratio", 6.0), ("attack", 15.0), ("release", 450.0)];
    music.devices = devices(&[("comp", duck), ("comp", duck)]);
    music.devices[0].sidechain = Some(host.id.clone());
    music.devices[1].sidechain = Some(guest.id.clone());
    project.master_devices = devices(&[("limiter", &[("ceiling", -1.0)])]);
    project.tracks = vec![host, guest, music];
    project
}

fn editing() -> Project {
    let mut project = blank();
    project.tracks = vec![recording("Recording", 4, "1")];
    project
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_template_builds_with_its_devices() {
        let dir = std::env::temp_dir().join("soniq-templates-test-none");
        for (id, ..) in BUILTIN {
            let p = build(id, &dir).unwrap();
            assert_eq!(p.name, "Untitled");
            assert!(!p.tracks.is_empty(), "{id}");
            // Every device asked for exists in the catalog.
            for t in &p.tracks {
                assert!(t.devices.iter().all(|d| !d.params.is_empty()), "{id}/{}", t.name);
            }
        }
        let podcast = build("podcast", &dir).unwrap();
        let music = &podcast.tracks[2];
        assert_eq!(music.devices.iter().filter_map(|d| d.sidechain.as_deref()).collect::<Vec<_>>(), [podcast.tracks[0].id.as_str(), podcast.tracks[1].id.as_str()]);
        assert_eq!(build("beat", &dir).unwrap().tracks[0].clips.len(), 1);
        let roles: Vec<TrackRole> = podcast.tracks.iter().map(|t| t.role).collect();
        assert_eq!(roles, [TrackRole::Speaker, TrackRole::Speaker, TrackRole::Music]);
    }

    #[test]
    fn a_saved_template_keeps_the_setup_but_not_the_audio() {
        let dir = std::env::temp_dir().join(format!("soniq-templates-test-{}", new_id("x")));
        let mut song = build("beat", &dir).unwrap();
        song.bpm = 128.0;
        let info = save(&dir, "My: beat", &song).unwrap();
        assert_eq!(info.name, "My beat");
        assert!(list(&dir).iter().any(|t| t.id == info.id && !t.builtin));
        let again = build(&info.id, &dir).unwrap();
        assert_eq!(again.bpm, 128.0);
        assert_eq!(again.tracks.len(), song.tracks.len());
        assert!(again.tracks.iter().all(|t| t.clips.is_empty()));
        assert_ne!(again.id, song.id);
        assert!(build("user:../escape.soniq", &dir).is_err(), "only files in the folder");
        delete(&dir, &info.id).unwrap();
        assert!(list(&dir).iter().all(|t| t.builtin));
        let _ = fs::remove_dir_all(&dir);
    }
}
