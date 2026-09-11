use std::sync::Arc;

use crate::audio::AudioClip;

const MAX_HISTORY: usize = 64;

struct Snapshot {
    label: String,
    clip: Arc<AudioClip>,
}

/// Snapshot-based undo: every entry keeps the clip as it was before the edit.
#[derive(Default)]
pub struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

impl History {
    pub fn push(&mut self, label: &str, before: Arc<AudioClip>) {
        self.undo.push(Snapshot {
            label: label.to_string(),
            clip: before,
        });
        if self.undo.len() > MAX_HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn undo(&mut self, current: Arc<AudioClip>) -> Option<(String, Arc<AudioClip>)> {
        let snapshot = self.undo.pop()?;
        self.redo.push(Snapshot {
            label: snapshot.label.clone(),
            clip: current,
        });
        Some((snapshot.label, snapshot.clip))
    }

    pub fn redo(&mut self, current: Arc<AudioClip>) -> Option<(String, Arc<AudioClip>)> {
        let snapshot = self.redo.pop()?;
        self.undo.push(Snapshot {
            label: snapshot.label.clone(),
            clip: current,
        });
        Some((snapshot.label, snapshot.clip))
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|s| s.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|s| s.label.as_str())
    }
}
