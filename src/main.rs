//! Courier desktop entry point.

// Release builds on Windows are GUI apps: no console window behind them.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() -> eframe::Result {
    let data_dir = courier::storage::default_location();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Courier")
            // Matches the Linux desktop entry (StartupWMClass) so the dock groups windows.
            .with_app_id("courier")
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([900.0, 560.0])
            .with_icon(courier::app::theme::app_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "Courier",
        options,
        Box::new(move |cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(courier::app::CourierApp::new(&cc.egui_ctx, data_dir)))
        }),
    )
}
