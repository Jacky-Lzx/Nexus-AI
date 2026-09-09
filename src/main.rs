mod app;

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ClipKeep")
            .with_inner_size([900.0, 570.0])
            .with_min_inner_size([740.0, 480.0])
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top(),
        centered: true,
        persist_window: false,
        ..Default::default()
    };

    eframe::run_native(
        "ClipKeep",
        options,
        Box::new(|cc| Ok(Box::new(app::LauncherApp::new(cc)))),
    )
}
