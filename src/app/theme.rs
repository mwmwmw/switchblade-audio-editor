use egui::Color32;

pub const BACKGROUND: Color32 = Color32::from_rgb(20, 21, 24);
pub const LANE_BACKGROUND: Color32 = Color32::from_rgb(28, 30, 34);
pub const LANE_SEPARATOR: Color32 = Color32::from_rgb(45, 48, 54);
pub const CENTER_LINE: Color32 = Color32::from_rgb(70, 74, 82);
pub const GRID_LINE: Color32 = Color32::from_rgb(42, 45, 51);
pub const WAVEFORM: Color32 = Color32::from_rgb(120, 190, 240);
pub const WAVEFORM_DOT: Color32 = Color32::from_rgb(200, 230, 255);
pub const RULER_BACKGROUND: Color32 = Color32::from_rgb(24, 26, 30);
pub const RULER_TEXT: Color32 = Color32::from_rgb(150, 155, 165);
pub const RULER_TICK: Color32 = Color32::from_rgb(90, 95, 105);
pub const SELECTION: Color32 = Color32::from_rgba_premultiplied(60, 90, 140, 70);
pub const SELECTION_EDGE: Color32 = Color32::from_rgb(140, 170, 220);
pub const CURSOR: Color32 = Color32::from_rgb(230, 230, 230);
pub const PLAYHEAD: Color32 = Color32::from_rgb(90, 220, 120);
pub const CLIP: Color32 = Color32::from_rgb(235, 60, 60);
pub const CLIP_FILL: Color32 = Color32::from_rgba_premultiplied(120, 20, 20, 110);
pub const ZERO_RUN: Color32 = Color32::from_rgb(80, 140, 240);
pub const ZERO_RUN_FILL: Color32 = Color32::from_rgba_premultiplied(20, 50, 120, 110);
pub const DISCONTINUITY: Color32 = Color32::from_rgb(250, 160, 40);
pub const METER_BACKGROUND: Color32 = Color32::from_rgb(30, 32, 36);
pub const METER_GREEN: Color32 = Color32::from_rgb(70, 190, 100);
pub const METER_YELLOW: Color32 = Color32::from_rgb(230, 200, 60);
pub const METER_RED: Color32 = Color32::from_rgb(235, 70, 60);
pub const METER_HOLD: Color32 = Color32::from_rgb(240, 240, 240);
pub const METER_TICK: Color32 = Color32::from_rgb(110, 115, 125);
pub const BALANCE_NEUTRAL: Color32 = Color32::from_rgb(120, 130, 145);
pub const BALANCE_HOT: Color32 = Color32::from_rgb(240, 130, 60);
pub const BALANCE_COLD: Color32 = Color32::from_rgb(80, 150, 240);
pub const STATUS_ERROR: Color32 = Color32::from_rgb(250, 110, 100);
pub const STATUS_MUTED: Color32 = Color32::from_rgb(140, 145, 155);

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.visuals.panel_fill = BACKGROUND;
        style.visuals.window_fill = Color32::from_rgb(32, 34, 39);
        style.visuals.window_stroke = egui::Stroke::NONE;
        style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
        style.visuals.collapsing_header_frame = false;
        style.visuals.indent_has_left_vline = false;
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(8.0, 3.0);
        style.spacing.indent = 8.0;
    });
}
