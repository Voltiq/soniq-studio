//! Projects on disk.
//!
//! A project is a `.soniq` JSON file plus a `Samples` folder beside it. Saving
//! copies every imported file into `Samples` and writes renders and edits as
//! 32-bit float WAV, so a project folder is self-contained and can be moved.
//! Autosaves skip the copying and point at the original files instead.

use std::fs;
use std::path::{Path, PathBuf};

use soniq_project::{Asset, Project, SCHEMA_VERSION, new_id};

use crate::session::{Pool, PoolAsset, Session};

pub const AUTOSAVE_FILE: &str = "autosave.soniq";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveMode {
    Project,
    Autosave,
}

fn file_name_safe(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c.is_alphanumeric() || " -_".contains(c) { c } else { '_' }).collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() { "audio".into() } else { trimmed.chars().take(60).collect() }
}

/// `Name-1a2b3c.ext`: readable, and the id suffix avoids collisions.
fn sample_file_name(asset: &Asset, ext: &str) -> String {
    let suffix: String = asset.id.chars().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect();
    format!("{}-{suffix}.{ext}", file_name_safe(&asset.name))
}

fn relative_to(path: &Path, dir: &Path) -> String {
    match path.strip_prefix(dir) {
        Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

/// Writes the session to `path`. Updates in-memory asset paths to the files
/// that now back them, so later saves do not copy or render again.
pub fn save(session: &mut Session, path: &Path, mode: SaveMode) -> Result<(), String> {
    let dir = path.parent().ok_or("That is not a folder Soniq can save in.")?;
    let samples = dir.join("Samples");
    let mut file_project = session.project.clone();
    let mut backing: Vec<(String, PathBuf)> = Vec::new();

    // Autosaves point at files in place, except those inside an older,
    // recovered autosave, which gets deleted later and so must be copied.
    let recovered = dir.parent().map(|data| data.join("autosave-recovered"));
    let reference_in_place = |p: &Path| mode == SaveMode::Autosave && !recovered.as_ref().is_some_and(|r| p.starts_with(r));

    for asset in &mut file_project.assets {
        let current = asset.path.as_ref().map(PathBuf::from).filter(|p| p.is_file());
        let file = match current {
            Some(p) if p.starts_with(&samples) || reference_in_place(&p) => p,
            Some(p) => {
                fs::create_dir_all(&samples).map_err(|e| format!("Could not create {}: {e}", samples.display()))?;
                let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("wav").to_lowercase();
                let target = samples.join(sample_file_name(asset, &ext));
                if !target.exists() {
                    fs::copy(&p, &target).map_err(|e| format!("Could not copy {}: {e}", p.display()))?;
                }
                target
            }
            None => {
                let Some(pooled) = session.pool.get(&asset.id) else {
                    // Missing audio keeps whatever path it had, so it can be found later.
                    continue;
                };
                fs::create_dir_all(&samples).map_err(|e| format!("Could not create {}: {e}", samples.display()))?;
                let target = samples.join(sample_file_name(asset, "wav"));
                soniq_io::write_wav(&target, &pooled.native).map_err(|e| e.to_string())?;
                target
            }
        };
        asset.path = Some(relative_to(&file, dir));
        backing.push((asset.id.clone(), file));
    }

    let json = serde_json::to_string_pretty(&file_project).map_err(|e| e.to_string())?;
    // Write next to the target, then rename: a crash mid-save never leaves half a file.
    let tmp = path.with_extension("soniq.tmp");
    fs::write(&tmp, json).map_err(|e| format!("Could not write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("Could not write {}: {e}", path.display()))?;

    for (id, file) in backing {
        if let Some(asset) = session.project.assets.iter_mut().find(|a| a.id == id) {
            asset.path = Some(file.to_string_lossy().into_owned());
        }
    }
    Ok(())
}

pub fn read_project(path: &Path) -> Result<Project, String> {
    let json = fs::read_to_string(path).map_err(|e| format!("Could not open {}: {e}", path.display()))?;
    let mut project: Project = serde_json::from_str(&json).map_err(|e| format!("{} is not a Soniq project: {e}", path.display()))?;
    if project.version > SCHEMA_VERSION {
        return Err("This project was made with a newer version of Soniq Studio.".into());
    }
    project.version = SCHEMA_VERSION;
    project.refresh_devices();
    // Older projects named their session clips; they are copies in slots now.
    project.migrate_slots();
    // Resolve sample paths against the project folder.
    let dir = path.parent().unwrap_or(Path::new("."));
    for asset in &mut project.assets {
        if let Some(p) = &asset.path {
            let p = PathBuf::from(p);
            let abs = if p.is_absolute() { p } else { dir.join(p) };
            asset.path = Some(abs.to_string_lossy().into_owned());
        }
    }
    Ok(project)
}

/// Decodes every asset of a freshly read project in parallel. Assets that
/// cannot be read are flagged `missing` and reported.
pub fn load_assets(project: &mut Project, engine_rate: u32) -> (Pool, Vec<String>) {
    let jobs: Vec<(usize, Option<PathBuf>)> =
        project.assets.iter().enumerate().map(|(i, a)| (i, a.path.as_ref().map(PathBuf::from))).collect();
    let workers = std::thread::available_parallelism().map_or(2, |n| n.get()).min(8);
    let chunk = jobs.len().div_ceil(workers).max(1);

    let results: Vec<(usize, Result<PoolAsset, String>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .chunks(chunk)
            .map(|batch| {
                scope.spawn(move || {
                    batch
                        .iter()
                        .map(|(i, path)| {
                            let result = match path {
                                Some(p) => soniq_io::decode_file(p).map(|b| PoolAsset::prepare(b, engine_rate)).map_err(|e| e.to_string()),
                                None => Err("no file was saved for it".into()),
                            };
                            (*i, result)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });

    let mut pool = Pool::default();
    let mut warnings = Vec::new();
    for (i, result) in results {
        let asset = &mut project.assets[i];
        match result {
            Ok(pooled) => {
                asset.missing = false;
                pool.insert(asset.id.clone(), pooled);
            }
            Err(err) => {
                asset.missing = true;
                warnings.push(format!("“{}” is missing: {err}", asset.name));
            }
        }
    }
    (pool, warnings)
}

/// Looks through `folder` and the folders in it (up to a limit) for files
/// with the given names (lower case); the first found for each name.
pub fn find_by_name<'a>(folder: &Path, names: impl Iterator<Item = &'a str>) -> std::collections::HashMap<String, PathBuf> {
    const MAX_ENTRIES: usize = 200_000;
    let mut wanted: std::collections::HashSet<&str> = names.collect();
    let mut found = std::collections::HashMap::new();
    let mut stack = vec![folder.to_path_buf()];
    let mut seen = 0;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_ENTRIES || wanted.is_empty() {
                return found;
            }
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(path),
                Ok(t) if t.is_file() => {
                    let name = entry.file_name().to_string_lossy().to_lowercase();
                    if wanted.remove(name.as_str()) {
                        found.insert(name, path);
                    }
                }
                _ => {}
            }
        }
    }
    found
}

/// Decodes one file for import: the asset entry and its prepared audio.
pub fn import_file(path: &Path, engine_rate: u32) -> Result<(Asset, PoolAsset), String> {
    let buffer = soniq_io::decode_file(path).map_err(|e| e.to_string())?;
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Audio").to_string();
    let asset = Asset {
        id: new_id("a"),
        name,
        channels: buffer.channel_count() as u16,
        sample_rate: buffer.sample_rate,
        frames: buffer.frames() as u64,
        path: Some(path.to_string_lossy().into_owned()),
        missing: false,
    };
    Ok((asset, PoolAsset::prepare(buffer, engine_rate)))
}

/// Moves a leftover autosave aside at startup so new autosaves cannot
/// overwrite it before the user decides. Returns whether one was found.
pub fn stash_autosave(autosave: &Path, recovered: &Path) -> bool {
    if !autosave.join(AUTOSAVE_FILE).is_file() {
        return recovered.join(AUTOSAVE_FILE).is_file();
    }
    let _ = fs::remove_dir_all(recovered);
    fs::rename(autosave, recovered).is_ok()
}

pub fn remove_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_files_are_found_by_name_in_folders_below() {
        let root = std::env::temp_dir().join(format!("soniq-find-{}", new_id("x")));
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::write(root.join("a/b/Kick.wav"), b"").unwrap();
        fs::write(root.join("a/other.wav"), b"").unwrap();
        let found = find_by_name(&root, ["kick.wav", "snare.wav"].into_iter());
        assert_eq!(found.len(), 1);
        assert_eq!(found["kick.wav"], root.join("a/b/Kick.wav"));
        let _ = fs::remove_dir_all(&root);
    }
}
