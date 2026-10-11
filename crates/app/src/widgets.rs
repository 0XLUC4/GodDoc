//! Small shared widgets: fixed-size icon buttons.

use egui::{Button, Response, RichText, Ui, Vec2, vec2};

/// Phosphor icon glyphs (regular weight), installed as a fallback of the proportional font.
pub use egui_phosphor::regular as icon;

/// Side of a standard icon-only button.
pub const ICON_BUTTON: Vec2 = vec2(28.0, 28.0);
/// Side of a compact icon-only button (tab close, steppers).
pub const SMALL_ICON_BUTTON: Vec2 = vec2(22.0, 22.0);

/// Frameless-until-hovered square button showing one glyph. Use with `ui.add_sized` / `ui.put`.
pub fn icon_widget(glyph: &str, size: f32) -> Button<'static> {
    Button::new(RichText::new(glyph.to_owned()).size(size))
        .frame_when_inactive(false)
        .stroke(egui::Stroke::NONE)
}

/// Standard 28×28 icon button with a tooltip.
pub fn icon_button(ui: &mut Ui, glyph: &str, tooltip: &str) -> Response {
    ui.add_sized(ICON_BUTTON, icon_widget(glyph, 16.0))
        .on_hover_text(tooltip)
}

/// Compact 22×22 icon button with a tooltip.
pub fn small_icon_button(ui: &mut Ui, glyph: &str, tooltip: &str) -> Response {
    ui.add_sized(SMALL_ICON_BUTTON, icon_widget(glyph, 13.0))
        .on_hover_text(tooltip)
}

/// 28×28 icon toggle; `on` shows a light grey square.
pub fn toggle_button(ui: &mut Ui, glyph: &str, tooltip: &str, on: bool) -> Response {
    let text = RichText::new(glyph.to_owned()).size(16.0);
    ui.add_sized(
        ICON_BUTTON,
        Button::selectable(on, text)
            .frame_when_inactive(on)
            .stroke(egui::Stroke::NONE),
    )
    .on_hover_text(tooltip)
}

/// Icon followed by a label, for menus and text buttons.
pub fn with_icon(glyph: &str, label: &str) -> String {
    format!("{glyph}  {label}")
}

/// Padding of text fields: as tall as a button, so the shared corner radius reads as a
/// rounded rectangle and not as a pill.
pub const FIELD_MARGIN: egui::Margin = egui::Margin::symmetric(8, 4);
