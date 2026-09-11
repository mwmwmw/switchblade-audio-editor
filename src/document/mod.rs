mod history;

use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::audio::AudioClip;
use history::History;

/// Versions are unique across every document ever created so caches keyed on them never collide.
static NEXT_VERSION: AtomicU64 = AtomicU64::new(1);

fn next_version() -> u64 {
    NEXT_VERSION.fetch_add(1, Ordering::Relaxed)
}

pub struct Document {
    pub clip: Arc<AudioClip>,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub selection: Option<Range<usize>>,
    pub cursor: usize,
    /// Bumped on every change so caches and analyses know when to refresh.
    pub version: u64,
    history: History,
}

impl Default for Document {
    fn default() -> Self {
        Self::from_clip(AudioClip::new(48_000, 2), None)
    }
}

impl Document {
    pub fn from_clip(clip: AudioClip, path: Option<PathBuf>) -> Self {
        Self {
            clip: Arc::new(clip),
            path,
            dirty: false,
            selection: None,
            cursor: 0,
            version: next_version(),
            history: History::default(),
        }
    }

    pub fn title(&self) -> String {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("Untitled");
        if self.dirty {
            format!("{name} •")
        } else {
            name.to_string()
        }
    }

    /// The selection if there is one, otherwise the whole clip.
    pub fn edit_range(&self) -> Range<usize> {
        self.selection
            .clone()
            .unwrap_or_else(|| self.clip.full_range())
    }

    pub fn has_selection(&self) -> bool {
        self.selection.as_ref().is_some_and(|s| !s.is_empty())
    }

    pub fn commit(&mut self, label: &str, clip: AudioClip) {
        self.history.push(label, Arc::clone(&self.clip));
        self.replace_clip(clip);
        self.dirty = true;
    }

    pub fn undo(&mut self) -> Option<String> {
        let (label, clip) = self.history.undo(Arc::clone(&self.clip))?;
        self.replace_arc(clip);
        self.dirty = true;
        Some(label)
    }

    pub fn redo(&mut self) -> Option<String> {
        let (label, clip) = self.history.redo(Arc::clone(&self.clip))?;
        self.replace_arc(clip);
        self.dirty = true;
        Some(label)
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.history.undo_label()
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.history.redo_label()
    }

    pub fn mark_saved(&mut self, path: PathBuf) {
        self.path = Some(path);
        self.dirty = false;
    }

    pub fn set_selection(&mut self, range: Range<usize>) {
        let range = self.clip.clamp_range(&range);
        self.cursor = range.start;
        self.selection = if range.is_empty() { None } else { Some(range) };
    }

    pub fn set_cursor(&mut self, frame: usize) {
        self.cursor = frame.min(self.clip.frames());
        self.selection = None;
    }

    fn replace_clip(&mut self, clip: AudioClip) {
        self.replace_arc(Arc::new(clip));
    }

    fn replace_arc(&mut self, clip: Arc<AudioClip>) {
        self.clip = clip;
        self.version = next_version();
        self.cursor = self.cursor.min(self.clip.frames());
        self.selection = self
            .selection
            .take()
            .map(|s| self.clip.clamp_range(&s))
            .filter(|s| !s.is_empty());
    }
}
