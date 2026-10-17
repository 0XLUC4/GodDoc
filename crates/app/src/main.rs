//! GodDoc: desktop entry point.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod branding;
mod editor;
mod layout;
mod pdf;
mod platform;
#[cfg(windows)]
mod print;
mod render;
mod spell;
mod theme;
mod toasts;
mod updates;
mod widgets;
mod window;

pub const APP_NAME: &str = "GodDoc";

fn main() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_app_id("goddoc")
        .with_inner_size([1180.0, 860.0])
        .with_min_inner_size([760.0, 480.0])
        .with_drag_and_drop(true);
    if let Some(icon) = branding::icon_data() {
        viewport = viewport.with_icon(icon);
    }
    let files = std::env::args_os().skip(1).map(Into::into).collect();
    let remembered = window::load(egui::vec2(760.0, 480.0));
    if let Some(size) = remembered.size {
        viewport = viewport.with_inner_size(size);
    }
    viewport = viewport.with_maximized(remembered.maximized);
    let options = eframe::NativeOptions {
        viewport,
        // Kept by `window`, which never brings back the size of a minimized window.
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|cc| Ok(Box::new(app::GodApp::new(cc, files)))),
    )
}
