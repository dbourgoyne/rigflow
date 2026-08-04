use eframe::NativeOptions;
use eframe::egui::{Align, Color32, Frame, Id, Layout, Modal, RichText, Stroke};

use crate::persistence::StartupError;

struct StartupErrorWindow {
    error: String,
}

impl eframe::App for StartupErrorWindow {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        eframe::egui::CentralPanel::default().show(ctx, |_ui| {});

        let dialog_width = (ctx.screen_rect().width() - 48.0).clamp(320.0, 520.0);
        Modal::new(Id::new("startup_error_modal"))
            .frame(
                Frame::popup(&ctx.style())
                    .inner_margin(20.0)
                    .corner_radius(8.0),
            )
            .show(ctx, |ui| {
                ui.set_width(dialog_width);

                ui.horizontal(|ui| {
                    ui.label(RichText::new("⚠").size(28.0).color(Color32::LIGHT_RED));
                    ui.vertical(|ui| {
                        ui.heading(
                            RichText::new("Rigflow could not start").color(Color32::LIGHT_RED),
                        );
                        ui.label("The configuration could not be loaded safely.");
                    });
                });

                ui.add_space(12.0);
                Frame::new()
                    .fill(ui.visuals().faint_bg_color)
                    .stroke(Stroke::new(
                        1.0_f32,
                        ui.visuals().widgets.noninteractive.bg_stroke.color,
                    ))
                    .corner_radius(4.0)
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        ui.set_width(dialog_width - 24.0);
                        ui.label(&self.error);
                    });

                ui.add_space(12.0);
                ui.label(
                    "Check the path and its permissions, or set RIGFLOW_CONFIG_DIR \
                    to a usable directory before trying again.",
                );
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(8.0);

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add_sized([80.0, 28.0], eframe::egui::Button::new("Close"))
                        .clicked()
                    {
                        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
                    }
                });
            });

        if ctx.input(|input| {
            input.key_pressed(eframe::egui::Key::Escape)
                || input.key_pressed(eframe::egui::Key::Enter)
        }) {
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
        }
    }
}

pub(crate) fn exit_with_startup_error(error: &StartupError) -> ! {
    let error_message = error.to_string();

    log::error!("client startup failed: {error}");
    eprintln!("client startup failed: {error}");

    let options = NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([600.0, 340.0])
            .with_resizable(false)
            .with_minimize_button(false)
            .with_maximize_button(false),
        ..NativeOptions::default()
    };
    if let Err(dialog_error) = eframe::run_native(
        "Rigflow could not start",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_theme(eframe::egui::ThemePreference::Dark);
            Ok(Box::new(StartupErrorWindow {
                error: error_message,
            }))
        }),
    ) {
        log::error!("could not open the startup error window: {dialog_error}");
        eprintln!("could not open the startup error window: {dialog_error}");
    }

    std::process::exit(1);
}
