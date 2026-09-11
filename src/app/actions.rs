use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use crossbeam_channel::Receiver;

use super::dialogs::{ExportDialog, FadeDialog, FadeScope, NormalizeDialog, ResampleDialog};
use super::{format, SwitchbladeApp};
use crate::analysis::report::AnalysisReport;
use crate::audio::encode::{self, BitDepth, ExportFormat, SaveOptions};
use crate::audio::fade::{apply_fade, FadeCurve, FadeDirection};
use crate::audio::loop_fade::{crossfade_loop, DEFAULT_FADE_MS};
use crate::audio::normalize::normalize_peak;
use crate::audio::repair::{count_discontinuities, repair_discontinuities, DEFAULT_WINDOW_MS};
use crate::audio::resample::{resample, ResampleQuality};
use crate::audio::{decode, AudioClip};
use crate::cache;
use crate::document::Document;
use crate::engine::EngineCommand;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Open,
    Save,
    SaveAs,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Delete,
    Trim,
    Silence,
    SelectAll,
    TogglePlay,
    Stop,
    ToggleRecord,
    ToggleLoop,
    ToggleBeatSnap,
    ZoomFit,
    ZoomSelection,
    GoToStart,
    GoToEnd,
    Normalize,
    RemoveDc,
    RepairDiscontinuities,
    CrossfadeLoop,
    FadeIn,
    FadeOut,
    Resample,
    ApplyStack,
    HighlightSettings,
    ToggleSidePanel,
    About,
    Quit,
}

pub type LoadResult = (PathBuf, Result<(AudioClip, Option<AnalysisReport>)>);

impl SwitchbladeApp {
    pub fn perform(&mut self, action: Action) {
        match action {
            Action::Open => self.open_file_dialog(),
            Action::Save => self.save(),
            Action::SaveAs => {
                self.dialogs.export = Some(ExportDialog {
                    format: self.export_format,
                    depth: self.export_depth,
                })
            }
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::Cut => self.cut(),
            Action::Copy => self.copy(),
            Action::Paste => self.paste(),
            Action::Delete => self.delete_selection(),
            Action::Trim => self.trim_to_selection(),
            Action::Silence => self.silence_selection(),
            Action::SelectAll => self.doc.set_selection(self.doc.clip.full_range()),
            Action::TogglePlay => self.toggle_play(),
            Action::Stop => self.stop(),
            Action::ToggleRecord => self.toggle_record(),
            Action::ToggleLoop => self.loop_playback = !self.loop_playback,
            Action::ToggleBeatSnap => self.toggle_beat_snap(),
            Action::ZoomFit => self.view.zoom_to_fit(),
            Action::ZoomSelection => self.zoom_to_selection(),
            Action::GoToStart => self.doc.set_cursor(0),
            Action::GoToEnd => self.doc.set_cursor(self.doc.clip.frames()),
            Action::Normalize => self.dialogs.normalize = Some(NormalizeDialog::default()),
            Action::RemoveDc => self.remove_dc(),
            Action::RepairDiscontinuities => self.repair_discontinuities(),
            Action::CrossfadeLoop => self.crossfade_loop(),
            Action::FadeIn => {
                self.dialogs.fade =
                    Some(FadeDialog::new(FadeDirection::In, self.doc.has_selection()))
            }
            Action::FadeOut => {
                self.dialogs.fade = Some(FadeDialog::new(
                    FadeDirection::Out,
                    self.doc.has_selection(),
                ))
            }
            Action::Resample => {
                self.dialogs.resample = Some(ResampleDialog::new(self.doc.clip.sample_rate))
            }
            Action::ApplyStack => self.apply_stack(),
            Action::HighlightSettings => self.dialogs.anomaly_settings_open = true,
            Action::ToggleSidePanel => self.show_side_panel = !self.show_side_panel,
            Action::About => self.dialogs.about_open = true,
            Action::Quit => self.quit_requested = true,
        }
    }

    pub fn set_status(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.status_is_error = false;
    }

    pub fn set_error(&mut self, message: impl Into<String>) {
        let message = message.into();
        log::error!("{message}");
        self.status = message;
        self.status_is_error = true;
    }

    fn open_file_dialog(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Audio", decode::SUPPORTED_EXTENSIONS)
            .pick_file()
        else {
            return;
        };
        self.open_path(&path);
    }

    /// Decoding runs on a worker thread; `poll_load` swaps the document in when it finishes.
    pub fn open_path(&mut self, path: &Path) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let name = path_name(path);
        let path = path.to_path_buf();
        std::thread::spawn(move || {
            let result = decode::load(&path).map(|clip| (clip, cache::load(&path)));
            let _ = tx.send((path, result));
        });
        self.pending_load = Some(rx);
        self.set_status(format!("Loading {name}…"));
    }

    pub fn poll_load(&mut self) {
        let Some(rx) = &self.pending_load else { return };
        let Ok((path, result)) = rx.try_recv() else {
            return;
        };
        self.pending_load = None;
        match result {
            Ok((clip, cached)) => {
                let doc = Document::from_clip(clip, Some(path.clone()));
                if let Some(report) = cached {
                    self.analysis.seed(doc.version, report);
                }
                self.install_document(doc, &format!("Opened {}", path_name(&path)));
            }
            Err(error) => self.set_error(format!("Could not open {}: {error:#}", path_name(&path))),
        }
    }

    fn install_document(&mut self, doc: Document, status: &str) {
        self.stop();
        self.doc = doc;
        self.view.zoom_to_fit();
        self.set_status(status);
    }

    fn save(&mut self) {
        let target = self
            .doc
            .path
            .clone()
            .and_then(|p| ExportFormat::from_path(&p).map(|f| (p, f)));
        match target {
            Some((path, format)) => self.write_file(&path, format, self.export_depth),
            None => self.perform(Action::SaveAs),
        }
    }

    pub fn export(&mut self, format: ExportFormat, depth: BitDepth) {
        self.export_format = format;
        self.export_depth = depth;
        let suggested = self
            .doc
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or("untitled");
        let Some(path) = rfd::FileDialog::new()
            .add_filter(format.label(), &[format.extension()])
            .set_file_name(format!("{suggested}.{}", format.extension()))
            .save_file()
        else {
            return;
        };
        self.write_file(&path, format, depth);
    }

    fn write_file(&mut self, path: &Path, format: ExportFormat, depth: BitDepth) {
        // The loop the user is hearing is the loop that gets written into the file.
        let options = SaveOptions {
            loop_points: self.active_loop_range(),
        };
        match encode::save(path, &self.doc.clip, format, depth, &options) {
            Ok(()) => {
                self.doc.mark_saved(path.to_path_buf());
                self.analysis.persist(self.doc.version, path.to_path_buf());
                self.set_status(format!("Saved {}", path_name(path)));
            }
            Err(error) => self.set_error(format!("Save failed: {error:#}")),
        }
    }

    /// Clones the clip, applies `edit`, and commits it as an undoable step.
    fn apply_edit(&mut self, label: &str, edit: impl FnOnce(&mut AudioClip)) {
        let mut clip = (*self.doc.clip).clone();
        edit(&mut clip);
        self.doc.commit(label, clip);
        self.set_status(label.to_string());
    }

    fn undo(&mut self) {
        match self.doc.undo() {
            Some(label) => self.set_status(format!("Undid {label}")),
            None => self.set_status("Nothing to undo"),
        }
    }

    fn redo(&mut self) {
        match self.doc.redo() {
            Some(label) => self.set_status(format!("Redid {label}")),
            None => self.set_status("Nothing to redo"),
        }
    }

    fn copy(&mut self) {
        if let Some(selection) = self.doc.selection.clone() {
            self.clipboard = Some(self.doc.clip.slice(&selection));
            self.set_status("Copied");
        }
    }

    fn cut(&mut self) {
        self.copy();
        self.delete_selection();
    }

    fn delete_selection(&mut self) {
        let Some(selection) = self.doc.selection.clone() else {
            return;
        };
        self.apply_edit("Delete", |clip| clip.remove(&selection));
        self.doc.set_cursor(selection.start);
    }

    fn paste(&mut self) {
        let Some(pasted) = self.clipboard.clone() else {
            return;
        };
        let pasted = match resample(&pasted, self.doc.clip.sample_rate, ResampleQuality::High) {
            Ok(clip) => clip,
            Err(error) => return self.set_error(format!("Paste failed: {error:#}")),
        };
        if let Some(selection) = self.doc.selection.clone() {
            self.apply_edit("Paste", |clip| clip.remove(&selection));
        }
        let at = self.doc.cursor;
        let length = pasted.frames();
        self.apply_edit("Paste", |clip| clip.insert(at, &pasted));
        self.doc.set_selection(at..at + length);
    }

    fn trim_to_selection(&mut self) {
        let Some(selection) = self.doc.selection.clone() else {
            return;
        };
        self.apply_edit("Trim", |clip| *clip = clip.slice(&selection));
        self.doc.set_cursor(0);
    }

    fn silence_selection(&mut self) {
        let Some(selection) = self.doc.selection.clone() else {
            return;
        };
        self.apply_edit("Silence", |clip| clip.silence_range(&selection));
    }

    pub fn normalize(&mut self, target_dbfs: f32) {
        let range = self.doc.edit_range();
        let mut applied = None;
        self.apply_edit("Normalize", |clip| {
            applied = normalize_peak(clip, &range, target_dbfs)
        });
        match applied {
            Some(gain_db) => self.set_status(format!(
                "Normalized to {target_dbfs:+.2} dBFS ({gain_db:+.2} dB)"
            )),
            None => self.set_status("Nothing to normalize: the range is silent"),
        }
    }

    fn remove_dc(&mut self) {
        let range = self.doc.edit_range();
        let rate = self.doc.clip.sample_rate;
        let mut offsets = Vec::new();
        self.apply_edit("Remove DC", |clip| {
            offsets = crate::audio::dc::remove(clip, &range)
        });
        let worst = crate::audio::dc::largest(&offsets);
        if worst == 0.0 {
            return self.set_status("No DC offset to remove");
        }
        let summary: Vec<String> = offsets.iter().map(|o| format!("{:+.5}", o)).collect();
        self.set_status(format!(
            "Removed DC offset over {}: {}",
            format::time(range.len(), rate),
            summary.join(", ")
        ));
    }

    fn repair_discontinuities(&mut self) {
        let range = self.doc.edit_range();
        let threshold = self.anomaly_settings.discontinuity_jump;
        let found = count_discontinuities(&self.doc.clip, &range, threshold);
        if found == 0 {
            return self.set_status("No discontinuities in range");
        }
        let mut repaired = 0;
        self.apply_edit("Repair discontinuities", |clip| {
            repaired = repair_discontinuities(clip, &range, threshold, DEFAULT_WINDOW_MS)
        });
        self.set_status(format!(
            "Repaired {repaired} discontinuit{} over a {DEFAULT_WINDOW_MS} ms window",
            if repaired == 1 { "y" } else { "ies" }
        ));
    }

    fn crossfade_loop(&mut self) {
        let Some(range) = self.active_loop_range().or_else(|| self.doc.selection.clone()) else {
            return self.set_error("Select the loop region first");
        };
        let mut result = None;
        self.apply_edit("Crossfade loop", |clip| {
            result = Some(crossfade_loop(clip, &range, DEFAULT_FADE_MS))
        });
        match result {
            Some(Ok(fade)) => {
                let rate = self.doc.clip.sample_rate;
                self.set_status(format!(
                    "Crossfaded the loop over {}",
                    format::time(fade.frames, rate)
                ));
            }
            Some(Err(error)) => {
                // The clip was cloned and committed before the fade failed, so step back.
                self.doc.undo();
                self.set_error(format!("Crossfade failed: {error}"));
            }
            None => {}
        }
    }

    pub fn fade(&mut self, curve: FadeCurve, direction: FadeDirection, scope: FadeScope) {
        let frames = self.doc.clip.frames();
        let range = match scope {
            FadeScope::Selection => self.doc.edit_range(),
            FadeScope::Seconds(seconds) => {
                let length = ((seconds * self.doc.clip.sample_rate as f64) as usize).min(frames);
                match direction {
                    FadeDirection::In => 0..length,
                    FadeDirection::Out => frames - length..frames,
                }
            }
        };
        let label = match direction {
            FadeDirection::In => "Fade in",
            FadeDirection::Out => "Fade out",
        };
        self.apply_edit(label, |clip| apply_fade(clip, &range, curve, direction));
    }

    pub fn resample_to(&mut self, target_rate: u32, quality: ResampleQuality) {
        let source_rate = self.doc.clip.sample_rate;
        match resample(&self.doc.clip, target_rate, quality) {
            Ok(clip) => {
                self.doc.commit("Resample", clip);
                self.view.zoom_to_fit();
                self.set_status(format!("Resampled {source_rate} Hz → {target_rate} Hz"));
            }
            Err(error) => self.set_error(format!("Resample failed: {error:#}")),
        }
    }

    fn apply_stack(&mut self) {
        self.stop();
        let rendered = self
            .engine
            .shared
            .stack
            .lock()
            .render_offline(&self.doc.clip);
        match rendered {
            Ok(clip) => {
                self.doc.commit("Apply plugins", clip);
                self.set_status("Applied plugin stack");
            }
            Err(error) => self.set_error(format!("Applying plugins failed: {error:#}")),
        }
    }

    fn toggle_beat_snap(&mut self) {
        self.snap_to_beats = !self.snap_to_beats;
        if !self.snap_to_beats {
            return self.set_status("Beat snapping off");
        }
        match self.analysis.for_version(self.doc.version) {
            Some(report) if !report.beats.beats.is_empty() => {
                let count = report.beats.beats.len();
                match report.beats.bpm {
                    Some(bpm) => self.set_status(format!("Snapping to {count} hits · {bpm:.1} BPM")),
                    None => self.set_status(format!("Snapping to {count} hits")),
                }
            }
            Some(_) => self.set_status("No hits detected to snap to"),
            None => self.set_status("Snapping to beats once the analysis finishes"),
        }
    }

    fn zoom_to_selection(&mut self) {
        if let Some(selection) = self.doc.selection.clone() {
            self.view.zoom_to_range(&selection, self.last_wave_width);
        }
    }

    fn toggle_play(&mut self) {
        if self.engine.shared.is_playing() {
            self.pause();
        } else {
            self.play();
        }
    }

    /// The selection acts as the loop region whenever looping is on.
    pub fn active_loop_range(&self) -> Option<std::ops::Range<usize>> {
        self.doc.selection.clone().filter(|_| self.loop_playback)
    }

    fn play(&mut self) {
        if self.doc.clip.is_empty() {
            return self.set_status("Nothing to play");
        }
        let loop_range = self.active_loop_range();
        let start_frame = if self.doc.cursor >= self.doc.clip.frames() {
            0
        } else {
            self.doc.cursor
        };
        self.engine.send(EngineCommand::Play {
            clip: Arc::clone(&self.doc.clip),
            start_frame,
            loop_range,
        });
    }

    fn pause(&mut self) {
        let position = self
            .engine
            .shared
            .play_position
            .load(std::sync::atomic::Ordering::Relaxed);
        self.engine.send(EngineCommand::Stop);
        self.doc.cursor = position.min(self.doc.clip.frames());
    }

    pub fn stop(&mut self) {
        if self.engine.shared.is_recording() {
            self.engine.send(EngineCommand::StopRecording);
        }
        self.engine.send(EngineCommand::Stop);
    }

    fn toggle_record(&mut self) {
        if self.engine.shared.is_recording() {
            self.engine.send(EngineCommand::StopRecording);
            self.set_status("Finishing recording…");
        } else {
            self.engine.send(EngineCommand::Stop);
            self.engine.send(EngineCommand::StartRecording);
            self.set_status("Recording");
        }
    }

    pub fn finish_recording(&mut self, recorded: AudioClip) {
        if recorded.is_empty() {
            return self.set_status("Recording was empty");
        }
        let seconds = recorded.duration_seconds();
        if self.doc.clip.is_empty() {
            let path = self.doc.path.clone();
            self.install_document(Document::from_clip(recorded, path), "Recorded");
            self.doc.dirty = true;
        } else {
            self.insert_recording(recorded);
        }
        self.set_status(format!("Recorded {seconds:.1} s"));
    }

    fn insert_recording(&mut self, recorded: AudioClip) {
        let recorded = match resample(&recorded, self.doc.clip.sample_rate, ResampleQuality::High) {
            Ok(clip) => clip,
            Err(error) => return self.set_error(format!("Could not conform recording: {error:#}")),
        };
        let at = self.doc.cursor;
        let length = recorded.frames();
        self.apply_edit("Record", |clip| clip.insert(at, &recorded));
        self.doc.set_selection(at..at + length);
    }
}

fn path_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string()
}

pub type PendingLoad = Option<Receiver<LoadResult>>;
