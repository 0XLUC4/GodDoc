//! GodDoc: desktop entry point.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod branding;
mod editor;
mod layout;
mod platform;
mod theme;
mod toasts;
mod updates;
mod widgets;

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
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|cc| Ok(Box::new(app::GodApp::new(cc, files)))),
    )
}
