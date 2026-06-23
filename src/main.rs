// main.rs — App Inventory Generator
// Developed for Lean Linux by archerprojects (archer.projects@proton.me)
// https://github.com/archerprojects/app-inventory-generator
//
// Entry point. Builds the eframe NativeOptions and launches the egui window.
// The tokio runtime is owned by ui::AppState (created on construction).

mod generator;
mod resolver;
mod scanner;
mod ui;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("App Inventory Generator")
            .with_inner_size([1100.0, 750.0])
            .with_min_inner_size([800.0, 550.0]),
        ..Default::default()
    };

    eframe::run_native(
        "App Inventory Generator",
        options,
        Box::new(|_cc| Ok(Box::new(ui::AppState::default()))),
    )
}
