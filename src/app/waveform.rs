use std::ops::Range;

use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui};

use super::{format, theme};
use crate::analysis::anomalies::{Anomaly, AnomalyKind};
use crate::analysis::beats::BeatReport;
use crate::analysis::peaks::{scan_min_max, PeakMipmap};
use crate::analysis::report::AnalysisReport;
use crate::document::Document;

const MIN_FRAMES_PER_PIXEL: f64 = 0.02;
const SCROLLBAR_HEIGHT: f32 = 6.0;
const RULER_HEIGHT: f32 = 20.0;
const MARKER_STRIP_HEIGHT: f32 = 5.0;
/// A thumb narrower than this is hard to grab, so it stops shrinking with the zoom.
const MIN_THUMB_WIDTH: f32 = 24.0;
const LANE_GAP: f32 = 2.0;
const AMPLITUDE_HEADROOM: f32 = 0.94;
/// Samples inspected per column while the peak mipmap is still being built.
const PLACEHOLDER_SAMPLES_PER_COLUMN: usize = 64;
const SAMPLE_DOT_THRESHOLD_FPP: f64 = 0.15;
const SAMPLE_DOT_RADIUS: f32 = 2.0;
const MIN_HIGHLIGHT_WIDTH: f32 = 2.0;
/// Pointer distance, in pixels, within which a selection edge can be grabbed.
const EDGE_GRAB_PX: f32 = 6.0;
/// Pointer distance, in pixels, within which the cursor and selection edges snap to a beat.
const BEAT_SNAP_PX: f32 = 10.0;
const BEAT_TICK_HEIGHT: f32 = 6.0;
const MAX_DRAWN_ANOMALIES: usize = 4000;
const WHEEL_ZOOM_SENSITIVITY: f64 = 0.004;
const MIN_TICK_SPACING_PX: f32 = 90.0;
const TICK_STEPS_SECONDS: &[f64] = &[
    0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0, 300.0, 600.0, 1800.0,
];
const GRID_LEVELS: &[f32] = &[0.5, 0.25];

pub struct WaveView {
    start_frame: f64,
    frames_per_pixel: f64,
    fit_pending: bool,
    drag: Option<DragMode>,
}

/// What the pointer started on decides what a drag does for the rest of the gesture.
#[derive(Clone, Copy, Debug)]
enum DragMode {
    /// Dragging in the waveform sweeps a selection out from a fixed anchor frame.
    Select { anchor: usize },
    /// Dragging the ruler grabs the waveform itself, which follows the pointer.
    PanContent,
    /// Dragging the scrollbar moves the thumb under the pointer, keeping the grab offset.
    DragThumb { grab_offset: f32 },
}

pub struct WaveResponse {
    pub seek_to: Option<usize>,
}

impl Default for WaveView {
    fn default() -> Self {
        Self {
            start_frame: 0.0,
            frames_per_pixel: 1.0,
            fit_pending: true,
            drag: None,
        }
    }
}

impl WaveView {
    pub fn zoom_to_fit(&mut self) {
        self.fit_pending = true;
    }

    pub fn zoom_to_range(&mut self, range: &Range<usize>, width_px: f32) {
        if range.is_empty() || width_px <= 0.0 {
            return;
        }
        self.frames_per_pixel = (range.len() as f64 / width_px as f64).max(MIN_FRAMES_PER_PIXEL);
        self.start_frame = range.start as f64;
        self.fit_pending = false;
    }

    fn fit(&mut self, frames: usize, width_px: f32) {
        self.frames_per_pixel =
            (frames.max(1) as f64 / width_px.max(1.0) as f64).max(MIN_FRAMES_PER_PIXEL);
        self.start_frame = 0.0;
        self.fit_pending = false;
    }

    fn frame_to_x(&self, frame: f64, rect: &Rect) -> f32 {
        rect.left() + ((frame - self.start_frame) / self.frames_per_pixel) as f32
    }

    fn x_to_frame(&self, x: f32, rect: &Rect) -> f64 {
        self.start_frame + (x - rect.left()) as f64 * self.frames_per_pixel
    }

    fn visible_range(&self, rect: &Rect) -> Range<usize> {
        let start = self.start_frame.max(0.0) as usize;
        let end = (self.start_frame + rect.width() as f64 * self.frames_per_pixel)
            .ceil()
            .max(0.0) as usize;
        start..end
    }

    fn clamp(&mut self, frames: usize, width_px: f32) {
        let max_fpp = (frames.max(1) as f64 / width_px.max(1.0) as f64).max(MIN_FRAMES_PER_PIXEL);
        self.frames_per_pixel = self.frames_per_pixel.clamp(MIN_FRAMES_PER_PIXEL, max_fpp);
        let max_start = (frames as f64 - width_px as f64 * self.frames_per_pixel).max(0.0);
        self.start_frame = self.start_frame.clamp(0.0, max_start);
    }

    /// Zooms so `anchor_frame` keeps its screen position; an off-screen anchor is centred instead.
    fn zoom_about_frame(&mut self, anchor_frame: f64, factor: f64, rect: &Rect) {
        let anchor_x = self.frame_to_x(anchor_frame, rect);
        let anchor_x = if (rect.left()..=rect.right()).contains(&anchor_x) {
            anchor_x
        } else {
            rect.center().x
        };
        self.frames_per_pixel = (self.frames_per_pixel * factor).max(MIN_FRAMES_PER_PIXEL);
        self.start_frame = anchor_frame - (anchor_x - rect.left()) as f64 * self.frames_per_pixel;
    }

    fn pan_pixels(&mut self, delta_px: f32) {
        self.start_frame -= delta_px as f64 * self.frames_per_pixel;
    }
}

pub fn show(
    ui: &mut Ui,
    view: &mut WaveView,
    doc: &mut Document,
    analysis: Option<&AnalysisReport>,
    play_position: Option<usize>,
    snap_to_beats: bool,
) -> WaveResponse {
    let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let rect = response.rect;
    let frames = doc.clip.frames();
    let peaks = analysis.map(|a| &a.peaks).filter(|p| p.matches(&doc.clip));
    if view.fit_pending {
        view.fit(frames, rect.width());
    }
    let lanes = Layout::new(&rect, doc.clip.channel_count());
    // Snapping only makes sense once the analysis pass has produced hits to snap to.
    let beats = snap_to_beats
        .then(|| analysis.map(|a| &a.beats))
        .flatten()
        .filter(|report| !report.beats.is_empty());
    let seek_to = handle_input(
        ui,
        view,
        doc,
        &response,
        &lanes,
        play_position.is_some(),
        beats,
    );
    view.clamp(frames, rect.width());

    painter.rect_filled(rect, 0.0, theme::BACKGROUND);
    paint_scrollbar(&painter, view, &lanes, frames);
    paint_ruler(&painter, view, &lanes, doc.clip.sample_rate);
    for (channel, lane) in lanes.lanes.iter().enumerate() {
        paint_lane(
            &painter,
            view,
            lane,
            &doc.clip.channels[channel],
            peaks,
            channel,
        );
    }
    if let Some(analysis) = analysis {
        paint_anomalies(&painter, view, &lanes, &analysis.anomalies.anomalies);
    }
    if let Some(beats) = beats {
        paint_beats(&painter, view, &lanes, beats);
    }
    paint_selection(&painter, view, doc, &lanes.wave_rect);
    paint_marker(&painter, view, doc.cursor, &lanes.wave_rect, theme::CURSOR);
    if let Some(position) = play_position {
        paint_marker(&painter, view, position, &lanes.wave_rect, theme::PLAYHEAD);
    }
    WaveResponse { seek_to }
}

struct Layout {
    scroll_rect: Rect,
    ruler_rect: Rect,
    marker_rect: Rect,
    wave_rect: Rect,
    lanes: Vec<Rect>,
}

impl Layout {
    fn new(rect: &Rect, channels: usize) -> Self {
        let scroll_rect = Rect::from_min_size(rect.min, vec2(rect.width(), SCROLLBAR_HEIGHT));
        let ruler_rect = Rect::from_min_size(
            pos2(rect.left(), scroll_rect.bottom()),
            vec2(rect.width(), RULER_HEIGHT),
        );
        let marker_rect = Rect::from_min_size(
            pos2(rect.left(), ruler_rect.bottom()),
            vec2(rect.width(), MARKER_STRIP_HEIGHT),
        );
        let wave_rect = Rect::from_min_max(pos2(rect.left(), marker_rect.bottom()), rect.max);
        let count = channels.max(1);
        let lane_height = (wave_rect.height() - LANE_GAP * (count as f32 - 1.0)) / count as f32;
        let lanes = (0..channels)
            .map(|index| {
                let top = wave_rect.top() + index as f32 * (lane_height + LANE_GAP);
                Rect::from_min_size(
                    pos2(wave_rect.left(), top),
                    vec2(wave_rect.width(), lane_height),
                )
            })
            .collect();
        Self {
            scroll_rect,
            ruler_rect,
            marker_rect,
            wave_rect,
            lanes,
        }
    }

    /// The strip above the waveform: dragging anywhere in it scrolls rather than selects.
    fn scroll_strip(&self) -> Rect {
        self.scroll_rect.union(self.ruler_rect)
    }
}

/// Visible window as a thumb across the whole file, or `None` when the file all fits on screen.
fn thumb_rect(view: &WaveView, frames: usize, scroll_rect: &Rect) -> Option<Rect> {
    let frames = frames as f64;
    let width = scroll_rect.width() as f64;
    let visible = width * view.frames_per_pixel;
    if frames <= 0.0 || visible >= frames {
        return None;
    }
    let thumb_width = ((visible / frames * width) as f32).max(MIN_THUMB_WIDTH);
    let span = scroll_rect.width() - thumb_width;
    let offset = (view.start_frame / (frames - visible)).clamp(0.0, 1.0) as f32 * span;
    Some(Rect::from_min_size(
        pos2(scroll_rect.left() + offset, scroll_rect.top()),
        vec2(thumb_width, scroll_rect.height()),
    ))
}

/// Inverse of `thumb_rect`: the start frame that puts the thumb's left edge at `x`.
fn start_frame_for_thumb_x(view: &WaveView, frames: usize, scroll_rect: &Rect, x: f32) -> f64 {
    let frames = frames as f64;
    let visible = scroll_rect.width() as f64 * view.frames_per_pixel;
    let thumb_width = ((visible / frames * scroll_rect.width() as f64) as f32).max(MIN_THUMB_WIDTH);
    let span = scroll_rect.width() - thumb_width;
    if span <= 0.0 {
        return 0.0;
    }
    let fraction = ((x - scroll_rect.left()) / span).clamp(0.0, 1.0) as f64;
    fraction * (frames - visible).max(0.0)
}

#[allow(clippy::too_many_arguments)]
fn handle_input(
    ui: &Ui,
    view: &mut WaveView,
    doc: &mut Document,
    response: &egui::Response,
    layout: &Layout,
    playing: bool,
    beats: Option<&BeatReport>,
) -> Option<usize> {
    let rect = &layout.wave_rect;
    if response.hovered() {
        handle_wheel(ui, view, doc, rect);
    }
    let (start_frame, frames_per_pixel) = (view.start_frame, view.frames_per_pixel);
    // The snap radius is a fixed distance on screen, so it tightens as you zoom in and a
    // close pair of hits stays separable.
    let snap_tolerance = (BEAT_SNAP_PX as f64 * frames_per_pixel).round() as usize;
    let frame_at = |pos: Pos2| {
        let frame = (start_frame + (pos.x - rect.left()) as f64 * frames_per_pixel)
            .round()
            .max(0.0) as usize;
        match beats {
            Some(report) => report.nearest(frame, snap_tolerance).unwrap_or(frame),
            None => frame,
        }
    };
    let frames = doc.clip.frames();
    let strip = layout.scroll_strip();
    let mut seek_to = None;
    if let Some(pos) = response.hover_pos() {
        if strip.contains(pos) {
            let scrollable = thumb_rect(view, frames, &layout.scroll_rect).is_some();
            if scrollable {
                ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grab);
            }
        } else if grabbed_edge(view, doc, pos.x, rect).is_some() {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::ResizeHorizontal);
        }
    }
    if response.double_clicked() {
        doc.set_selection(doc.clip.full_range());
    } else if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let frame = frame_at(pos);
            doc.set_cursor(frame);
            seek_to = playing.then_some(frame);
        }
    }
    if response.drag_started() {
        view.drag = response
            .interact_pointer_pos()
            .map(|pos| drag_mode_at(view, doc, layout, pos, frame_at(pos)));
    }
    if let (Some(mode), true) = (view.drag, response.dragged()) {
        match mode {
            DragMode::Select { anchor } => {
                if let Some(pos) = response.interact_pointer_pos() {
                    let current = frame_at(pos);
                    doc.set_selection(anchor.min(current)..anchor.max(current));
                }
            }
            // The waveform follows the pointer, so the view moves the other way.
            DragMode::PanContent => {
                ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grabbing);
                view.pan_pixels(response.drag_delta().x);
            }
            DragMode::DragThumb { grab_offset } => {
                if let Some(pos) = response.interact_pointer_pos() {
                    view.start_frame = start_frame_for_thumb_x(
                        view,
                        frames,
                        &layout.scroll_rect,
                        pos.x - grab_offset,
                    );
                }
            }
        }
    }
    if response.drag_stopped() {
        view.drag = None;
    }
    seek_to
}

/// Picks the gesture from where the press landed: scrollbar, ruler, or the waveform itself.
fn drag_mode_at(
    view: &WaveView,
    doc: &Document,
    layout: &Layout,
    pos: Pos2,
    frame: usize,
) -> DragMode {
    if layout.scroll_rect.contains(pos) {
        let grab_offset = match thumb_rect(view, doc.clip.frames(), &layout.scroll_rect) {
            // Pressing beside the thumb jumps it under the pointer, then drags from its centre.
            Some(thumb) if !thumb.contains(pos) => thumb.width() / 2.0,
            Some(thumb) => pos.x - thumb.left(),
            None => return DragMode::PanContent,
        };
        return DragMode::DragThumb { grab_offset };
    }
    if layout.ruler_rect.contains(pos) {
        return DragMode::PanContent;
    }
    // Grabbing an existing edge keeps the opposite edge as the anchor, so the edge follows the pointer.
    let anchor = grabbed_edge(view, doc, pos.x, &layout.wave_rect).unwrap_or(frame);
    DragMode::Select { anchor }
}

/// If `x` is on a selection edge, returns the *opposite* edge to use as the drag anchor.
///
/// The nearer edge wins. On a selection narrower than twice the grab distance both edges
/// match, and testing the start first would quietly turn every drag of the end into a drag
/// of the start — which is what a short loop region does at any useful zoom level.
fn grabbed_edge(view: &WaveView, doc: &Document, x: f32, rect: &Rect) -> Option<usize> {
    let selection = doc.selection.as_ref()?;
    let to_start = (x - view.frame_to_x(selection.start as f64, rect)).abs();
    let to_end = (x - view.frame_to_x(selection.end as f64, rect)).abs();
    if to_start.min(to_end) > EDGE_GRAB_PX {
        return None;
    }
    if to_start <= to_end {
        Some(selection.end)
    } else {
        Some(selection.start)
    }
}

/// Vertical wheel and pinch zoom around the selection (or the cursor); horizontal wheel pans.
/// Only the dominant axis of a diagonal trackpad swipe is used so the view does not jitter.
fn handle_wheel(ui: &Ui, view: &mut WaveView, doc: &Document, rect: &Rect) {
    let (scroll, pinch, modifiers) =
        ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers));
    let vertical_dominates = scroll.y.abs() >= scroll.x.abs();
    let mut factor = 1.0 / pinch as f64;
    if vertical_dominates && !modifiers.shift {
        factor *= (-scroll.y as f64 * WHEEL_ZOOM_SENSITIVITY).exp();
    } else {
        let pan = if modifiers.shift {
            scroll.x + scroll.y
        } else {
            scroll.x
        };
        view.pan_pixels(pan);
    }
    if (factor - 1.0).abs() > f64::EPSILON {
        view.zoom_about_frame(zoom_anchor(doc), factor, rect);
    }
}

fn zoom_anchor(doc: &Document) -> f64 {
    match &doc.selection {
        Some(selection) => (selection.start + selection.end) as f64 / 2.0,
        None => doc.cursor as f64,
    }
}

fn paint_ruler(painter: &egui::Painter, view: &WaveView, layout: &Layout, sample_rate: u32) {
    painter.rect_filled(layout.ruler_rect, 0.0, theme::RULER_BACKGROUND);
    painter.rect_filled(layout.marker_rect, 0.0, theme::RULER_BACKGROUND);
    if sample_rate == 0 {
        return;
    }
    let seconds_per_pixel = view.frames_per_pixel / sample_rate as f64;
    let step = tick_step_seconds(seconds_per_pixel);
    let first_tick = (view.start_frame / sample_rate as f64 / step).floor() * step;
    let visible_seconds = layout.ruler_rect.width() as f64 * seconds_per_pixel;
    let mut tick = first_tick;
    while tick <= first_tick + visible_seconds + step {
        let x = view.frame_to_x(tick * sample_rate as f64, &layout.ruler_rect);
        painter.vline(
            x,
            layout.ruler_rect.bottom() - 6.0..=layout.ruler_rect.bottom(),
            Stroke::new(1.0, theme::RULER_TICK),
        );
        painter.text(
            pos2(x + 3.0, layout.ruler_rect.top() + 2.0),
            Align2::LEFT_TOP,
            format::seconds(tick.max(0.0)),
            FontId::monospace(10.0),
            theme::RULER_TEXT,
        );
        tick += step;
    }
}

fn paint_scrollbar(painter: &egui::Painter, view: &WaveView, layout: &Layout, frames: usize) {
    painter.rect_filled(layout.scroll_rect, 0.0, theme::SCROLL_TRACK);
    if let Some(thumb) = thumb_rect(view, frames, &layout.scroll_rect) {
        painter.rect_filled(thumb.shrink2(vec2(0.0, 1.0)), 2.0, theme::SCROLL_THUMB);
    }
}

fn tick_step_seconds(seconds_per_pixel: f64) -> f64 {
    let min_seconds = MIN_TICK_SPACING_PX as f64 * seconds_per_pixel;
    TICK_STEPS_SECONDS
        .iter()
        .copied()
        .find(|step| *step >= min_seconds)
        .unwrap_or(3600.0)
}

fn paint_lane(
    painter: &egui::Painter,
    view: &WaveView,
    lane: &Rect,
    samples: &[f32],
    peaks: Option<&PeakMipmap>,
    channel: usize,
) {
    painter.rect_filled(*lane, 0.0, theme::LANE_BACKGROUND);
    let center_y = lane.center().y;
    let scale = lane.height() / 2.0 * AMPLITUDE_HEADROOM;
    for level in GRID_LEVELS {
        for sign in [-1.0, 1.0] {
            painter.hline(
                lane.x_range(),
                center_y - sign * level * scale,
                Stroke::new(1.0, theme::GRID_LINE),
            );
        }
    }
    painter.hline(
        lane.x_range(),
        center_y,
        Stroke::new(1.0, theme::CENTER_LINE),
    );
    if samples.is_empty() {
        return;
    }
    if view.frames_per_pixel < 1.0 {
        paint_samples(painter, view, lane, samples, center_y, scale);
    } else {
        paint_columns(
            painter, view, lane, samples, peaks, channel, center_y, scale,
        );
    }
    if channel > 0 {
        painter.hline(
            lane.x_range(),
            lane.top() - LANE_GAP / 2.0,
            Stroke::new(LANE_GAP, theme::LANE_SEPARATOR),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_columns(
    painter: &egui::Painter,
    view: &WaveView,
    lane: &Rect,
    samples: &[f32],
    peaks: Option<&PeakMipmap>,
    channel: usize,
    center_y: f32,
    scale: f32,
) {
    let width = lane.width().ceil() as usize;
    let stroke = Stroke::new(1.0, theme::WAVEFORM);
    for column in 0..width {
        let x = lane.left() + column as f32 + 0.5;
        let start = view.x_to_frame(x - 0.5, lane).max(0.0) as usize;
        let end = (view.x_to_frame(x + 0.5, lane).max(0.0) as usize).min(samples.len());
        if start >= end {
            continue;
        }
        let (min, max) = match peaks {
            Some(peaks) => peaks.min_max(samples, channel, start, end),
            None => placeholder_min_max(&samples[start..end]),
        };
        let top = center_y - max * scale;
        let bottom = center_y - min * scale;
        painter.vline(x, top.min(bottom)..=bottom.max(top + 1.0), stroke);
    }
}

fn paint_samples(
    painter: &egui::Painter,
    view: &WaveView,
    lane: &Rect,
    samples: &[f32],
    center_y: f32,
    scale: f32,
) {
    let visible = view.visible_range(lane);
    let start = visible.start.saturating_sub(1);
    let end = (visible.end + 2).min(samples.len());
    let points: Vec<Pos2> = (start..end)
        .map(|frame| {
            pos2(
                view.frame_to_x(frame as f64, lane),
                center_y - samples[frame] * scale,
            )
        })
        .collect();
    if points.len() >= 2 {
        painter.add(Shape::line(
            points.clone(),
            Stroke::new(1.0, theme::WAVEFORM),
        ));
    }
    if view.frames_per_pixel < SAMPLE_DOT_THRESHOLD_FPP {
        for point in points {
            painter.circle_filled(point, SAMPLE_DOT_RADIUS, theme::WAVEFORM_DOT);
        }
    }
}

fn paint_anomalies(
    painter: &egui::Painter,
    view: &WaveView,
    layout: &Layout,
    anomalies: &[Anomaly],
) {
    let visible = view.visible_range(&layout.wave_rect);
    let first = anomalies.partition_point(|a| a.range.end < visible.start);
    for anomaly in anomalies[first..].iter().take(MAX_DRAWN_ANOMALIES) {
        if anomaly.range.start > visible.end {
            break;
        }
        let Some(lane) = layout.lanes.get(anomaly.channel) else {
            continue;
        };
        let x0 = view.frame_to_x(anomaly.range.start as f64, lane);
        let x1 = view
            .frame_to_x(anomaly.range.end as f64, lane)
            .max(x0 + MIN_HIGHLIGHT_WIDTH);
        paint_anomaly(painter, anomaly.kind, x0, x1, lane, &layout.marker_rect);
    }
}

fn paint_anomaly(
    painter: &egui::Painter,
    kind: AnomalyKind,
    x0: f32,
    x1: f32,
    lane: &Rect,
    marker_strip: &Rect,
) {
    let (edge, fill) = match kind {
        AnomalyKind::Clip => (theme::CLIP, theme::CLIP_FILL),
        AnomalyKind::ZeroRun => (theme::ZERO_RUN, theme::ZERO_RUN_FILL),
        AnomalyKind::Discontinuity => (theme::DISCONTINUITY, theme::DISCONTINUITY),
    };
    let band = Rect::from_min_max(pos2(x0, lane.top()), pos2(x1, lane.bottom()));
    painter.rect_filled(band, 0.0, fill);
    painter.rect_stroke(band, 0.0, Stroke::new(1.0, edge), StrokeKind::Inside);
    let marker = Rect::from_min_max(
        pos2(x0, marker_strip.top()),
        pos2(x1, marker_strip.bottom()),
    );
    painter.rect_filled(marker, 0.0, edge);
}

/// Ticks hanging from the ruler mark the detected hits, so it is obvious what will be snapped to.
fn paint_beats(painter: &egui::Painter, view: &WaveView, layout: &Layout, report: &BeatReport) {
    let visible = view.visible_range(&layout.wave_rect);
    let stroke = Stroke::new(1.0, theme::BEAT_MARKER);
    for beat in &report.beats {
        if beat.frame < visible.start {
            continue;
        }
        if beat.frame > visible.end {
            break;
        }
        let x = view.frame_to_x(beat.frame as f64, &layout.wave_rect);
        let bottom = layout.ruler_rect.bottom();
        painter.vline(x, bottom - BEAT_TICK_HEIGHT..=bottom, stroke);
        painter.vline(x, layout.wave_rect.y_range(), Stroke::new(1.0, theme::BEAT_LINE));
    }
}

fn paint_selection(painter: &egui::Painter, view: &WaveView, doc: &Document, rect: &Rect) {
    let Some(selection) = &doc.selection else {
        return;
    };
    let x0 = view
        .frame_to_x(selection.start as f64, rect)
        .clamp(rect.left(), rect.right());
    let x1 = view
        .frame_to_x(selection.end as f64, rect)
        .clamp(rect.left(), rect.right());
    let area = Rect::from_min_max(pos2(x0, rect.top()), pos2(x1, rect.bottom()));
    painter.rect_filled(area, 0.0, theme::SELECTION);
    painter.vline(x0, rect.y_range(), Stroke::new(1.0, theme::SELECTION_EDGE));
    painter.vline(x1, rect.y_range(), Stroke::new(1.0, theme::SELECTION_EDGE));
}

fn paint_marker(
    painter: &egui::Painter,
    view: &WaveView,
    frame: usize,
    rect: &Rect,
    color: Color32,
) {
    let x = view.frame_to_x(frame as f64, rect);
    if x >= rect.left() && x <= rect.right() {
        painter.vline(x, rect.y_range(), Stroke::new(1.5, color));
    }
}

/// Cheap strided estimate used until the analysis thread delivers the real mipmap.
fn placeholder_min_max(samples: &[f32]) -> (f32, f32) {
    let stride = (samples.len() / PLACEHOLDER_SAMPLES_PER_COLUMN).max(1);
    let strided: Vec<f32> = samples.iter().step_by(stride).copied().collect();
    scan_min_max(&strided)
}
