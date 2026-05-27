use egui::{Color32, FontId, Margin, Rounding, Stroke};

// ── Palette ───────────────────────────────────────────────────────────────

pub const SURFACE_0: Color32 = Color32::from_rgb(12, 12, 16);   // sidebar bg
pub const SURFACE_1: Color32 = Color32::from_rgb(17, 17, 22);   // main panel bg
pub const SURFACE_2: Color32 = Color32::from_rgb(24, 24, 32);   // card / input bg
pub const SURFACE_3: Color32 = Color32::from_rgb(34, 34, 46);   // hover / elevated

pub const BORDER: Color32 = Color32::from_rgb(38, 36, 54);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(72, 66, 112);

/// Primary accent — purple.
pub const ACCENT: Color32 = Color32::from_rgb(124, 106, 247);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(150, 134, 255);
/// Accent at ~16 % opacity (premultiplied: 124·40/255, 106·40/255, 247·40/255).
pub const ACCENT_SUBTLE: Color32 = Color32::from_rgba_premultiplied(19, 17, 39, 40);

pub const TEXT: Color32 = Color32::from_rgb(228, 224, 250);
pub const TEXT_2: Color32 = Color32::from_rgb(160, 155, 210);
pub const TEXT_DIM: Color32 = Color32::from_rgb(104, 98, 150);

pub const SUCCESS: Color32 = Color32::from_rgb(72, 199, 132);
pub const ERROR: Color32 = Color32::from_rgb(234, 80, 80);

// ── Entry point ───────────────────────────────────────────────────────────

pub fn setup(ctx: &egui::Context) {
    setup_visuals(ctx);
    setup_style(ctx);
}

fn setup_visuals(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();

    v.panel_fill = SURFACE_1;
    v.window_fill = SURFACE_2;
    v.faint_bg_color = SURFACE_0;
    v.extreme_bg_color = Color32::from_rgb(7, 7, 10);
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_rounding = Rounding::same(10.0);
    v.selection.bg_fill = ACCENT_SUBTLE;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT_HOVER;
    v.override_text_color = Some(TEXT);

    // noninteractive — text labels, separators, static backgrounds
    v.widgets.noninteractive.bg_fill = SURFACE_2;
    v.widgets.noninteractive.weak_bg_fill = SURFACE_1;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_2);
    v.widgets.noninteractive.rounding = Rounding::same(6.0);

    // inactive — buttons, checkboxes (not focused)
    v.widgets.inactive.bg_fill = SURFACE_2;
    v.widgets.inactive.weak_bg_fill = SURFACE_1;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT_2);
    v.widgets.inactive.rounding = Rounding::same(6.0);

    // hovered
    v.widgets.hovered.bg_fill = SURFACE_3;
    v.widgets.hovered.weak_bg_fill = SURFACE_2;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.rounding = Rounding::same(6.0);
    v.widgets.hovered.expansion = 1.0;

    // active / pressed
    v.widgets.active.bg_fill = ACCENT;
    v.widgets.active.weak_bg_fill = ACCENT_SUBTLE;
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT_HOVER);
    v.widgets.active.fg_stroke = Stroke::new(2.0, Color32::WHITE);
    v.widgets.active.rounding = Rounding::same(6.0);
    v.widgets.active.expansion = 1.0;

    // open (e.g. combo-box while open)
    v.widgets.open.bg_fill = SURFACE_3;
    v.widgets.open.weak_bg_fill = SURFACE_2;
    v.widgets.open.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
    v.widgets.open.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.open.rounding = Rounding::same(6.0);

    ctx.set_visuals(v);
}

fn setup_style(ctx: &egui::Context) {
    let mut s = (*ctx.style()).clone();
    s.spacing.item_spacing = egui::vec2(8.0, 6.0);
    s.spacing.button_padding = egui::vec2(12.0, 6.0);
    s.spacing.menu_margin = Margin::same(8.0);
    s.spacing.indent = 14.0;
    s.spacing.interact_size = egui::vec2(40.0, 28.0);
    s.spacing.slider_width = 130.0;
    s.text_styles = [
        (egui::TextStyle::Heading,   FontId::proportional(18.0)),
        (egui::TextStyle::Body,      FontId::proportional(14.0)),
        (egui::TextStyle::Monospace, FontId::monospace(13.0)),
        (egui::TextStyle::Button,    FontId::proportional(13.5)),
        (egui::TextStyle::Small,     FontId::proportional(11.5)),
    ]
    .into();
    ctx.set_style(s);
}

// ── Reusable widget helpers ────────────────────────────────────────────────

/// Card frame: slightly elevated surface, rounded border, internal padding.
pub fn card() -> egui::Frame {
    egui::Frame::none()
        .fill(SURFACE_2)
        .inner_margin(Margin::same(14.0))
        .rounding(Rounding::same(8.0))
        .stroke(Stroke::new(1.0, BORDER))
}

/// A small muted ALL-CAPS label used as a section heading inside cards.
pub fn section_header(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .size(10.5)
            .color(TEXT_DIM)
            .strong(),
    );
    ui.add_space(4.0);
}
