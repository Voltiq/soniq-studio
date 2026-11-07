use std::time::{Duration, Instant};

use crate::Project;

/// Continuous gestures (a fader drag, a knob turn) with the same key inside
/// this window become one undo step.
const COALESCE_WINDOW: Duration = Duration::from_millis(1500);
const LIMIT: usize = 200;

struct Entry {
    label: String,
    project: Project,
    key: Option<String>,
    at: Instant,
}

/// Snapshot-based undo. Each entry stores the project as it was *before* the
/// labelled action, so undo swaps it back in and keeps the current state for redo.
#[derive(Default)]
pub struct History {
    past: Vec<Entry>,
    future: Vec<Entry>,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records `before` as the state preceding an action called `label`.
    pub fn record(&mut self, before: Project, label: &str, key: Option<&str>) {
        self.future.clear();
        if let (Some(key), Some(last)) = (key, self.past.last_mut())
            && last.key.as_deref() == Some(key)
            && last.at.elapsed() < COALESCE_WINDOW
        {
            last.at = Instant::now();
            return;
        }
        if self.past.len() >= LIMIT {
            self.past.remove(0);
        }
        self.past.push(Entry { label: label.to_string(), project: before, key: key.map(str::to_string), at: Instant::now() });
    }

    /// Restores the previous state; returns the label of the undone action.
    pub fn undo(&mut self, current: &mut Project) -> Option<String> {
        let entry = self.past.pop()?;
        let label = entry.label.clone();
        let now = std::mem::replace(current, entry.project);
        self.future.push(Entry { label: entry.label, project: now, key: None, at: Instant::now() });
        Some(label)
    }

    pub fn redo(&mut self, current: &mut Project) -> Option<String> {
        let entry = self.future.pop()?;
        let label = entry.label.clone();
        let now = std::mem::replace(current, entry.project);
        self.past.push(Entry { label: entry.label, project: now, key: None, at: Instant::now() });
        Some(label)
    }

    /// What can be undone (oldest first) and redone (next first).
    pub fn labels(&self) -> (Vec<String>, Vec<String>) {
        (self.past.iter().map(|e| e.label.clone()).collect(), self.future.iter().rev().map(|e| e.label.clone()).collect())
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.past.last().map(|e| e.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.future.last().map(|e| e.label.as_str())
    }

    pub fn clear(&mut self) {
        self.past.clear();
        self.future.clear();
    }

    /// Every stored snapshot, so the pool can keep the audio they refer to.
    pub fn snapshots(&self) -> impl Iterator<Item = &Project> {
        self.past.iter().chain(self.future.iter()).map(|e| &e.project)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_and_redo_swap_states() {
        let mut history = History::new();
        let mut project = Project::empty("A");
        let before = project.clone();
        project.name = "B".into();
        history.record(before, "Rename", None);

        assert_eq!(history.undo(&mut project).as_deref(), Some("Rename"));
        assert_eq!(project.name, "A");
        assert_eq!(history.redo(&mut project).as_deref(), Some("Rename"));
        assert_eq!(project.name, "B");
    }

    #[test]
    fn same_key_coalesces_into_one_step() {
        let mut history = History::new();
        let mut project = Project::empty("A");
        for v in [-1.0, -2.0, -3.0] {
            let before = project.clone();
            project.master_volume = v;
            history.record(before, "Master volume", Some("master"));
        }
        history.undo(&mut project);
        assert_eq!(project.master_volume, 0.0);
        assert!(history.undo_label().is_none());
    }

    #[test]
    fn labels_list_what_undo_and_redo_reach() {
        let mut history = History::new();
        let mut project = Project::empty("A");
        for name in ["B", "C", "D"] {
            let before = project.clone();
            project.name = name.into();
            history.record(before, &format!("Rename to {name}"), None);
        }
        history.undo(&mut project);
        history.undo(&mut project);
        let (past, future) = history.labels();
        assert_eq!(past, ["Rename to B"]);
        assert_eq!(future, ["Rename to C", "Rename to D"], "the next redo first");
    }

    #[test]
    fn new_action_clears_redo() {
        let mut history = History::new();
        let mut project = Project::empty("A");
        history.record(project.clone(), "One", None);
        history.undo(&mut project);
        assert!(history.redo_label().is_some());
        history.record(project.clone(), "Two", None);
        assert!(history.redo_label().is_none());
    }
}
