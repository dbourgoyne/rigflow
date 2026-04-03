mod app;
mod input;
mod net;
mod render;
mod widgets;
mod eframe_app;

use eframe::NativeOptions;
use eframe_app::RigflowApp;

fn main() -> Result<(), eframe::Error> {
    let options = NativeOptions::default();

    eframe::run_native(
        "rigflow-client",
        options,
        Box::new(|_cc| Ok(Box::new(RigflowApp::default()))),
    )
}
