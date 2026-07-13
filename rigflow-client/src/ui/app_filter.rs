//! The contact **filter** window, opened from the contact view's `Filter…`
//! button.
//!
//! This edits the *shared* [`QsoFilterDraft`] — the one filter that decides both
//! which contacts the view lists and which an export writes. Editing it here
//! re-queries the list (debounced) so the operator watches the match count move
//! as they narrow it down, and then exports exactly that.
//!
//! Incremental ("only what's new since the last run") is deliberately **not**
//! here: it isn't a property of a contact, it's a property of export progress,
//! so it lives in the export window. See `logging::export`.

use eframe::egui;

use crate::logging::export::QsoFilterDraft;
use rigflow_log::export::GridPrecision;
use rigflow_log::normalize::ModeClass;

impl crate::ui::app::RigflowApp {
    pub(crate) fn draw_filter_window(&mut self, ctx: &egui::Context) {
        if !self.show_filter {
            return;
        }

        let mut open = true;
        let mut clear = false;

        egui::Window::new("Filter contacts")
            .open(&mut open)
            .default_width(470.0)
            .show(ctx, |ui| {
                let d = &mut self.qso_filter;

                egui::CollapsingHeader::new("Date")
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("From");
                            ui.add(
                                egui::TextEdit::singleline(&mut d.date_from)
                                    .hint_text("YYYYMMDD")
                                    .desired_width(90.0),
                            );
                            ui.label("To");
                            ui.add(
                                egui::TextEdit::singleline(&mut d.date_to)
                                    .hint_text("YYYYMMDD")
                                    .desired_width(90.0),
                            );
                        });
                    });

                egui::CollapsingHeader::new("Band & mode")
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.label("Bands (none = all)");
                        egui::Grid::new("filter_bands")
                            .num_columns(6)
                            .show(ui, |ui| {
                                for (i, band) in QsoFilterDraft::all_bands().into_iter().enumerate()
                                {
                                    let mut on = d.bands.contains(band);
                                    if ui.checkbox(&mut on, band).changed() {
                                        if on {
                                            d.bands.insert(band.to_string());
                                        } else {
                                            d.bands.remove(band);
                                        }
                                    }
                                    if i % 6 == 5 {
                                        ui.end_row();
                                    }
                                }
                            });
                        ui.add_enabled(
                            !d.bands.is_empty(),
                            egui::Checkbox::new(
                                &mut d.match_either_band,
                                "also match the RX band (split across bands)",
                            ),
                        );

                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.label("Mode class");
                            for c in ModeClass::ALL {
                                let mut on = d.mode_classes.contains(c);
                                if ui.checkbox(&mut on, c.as_str()).changed() {
                                    if on {
                                        d.mode_classes.insert(*c);
                                    } else {
                                        d.mode_classes.remove(c);
                                    }
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Modes");
                            ui.add(
                                egui::TextEdit::singleline(&mut d.modes)
                                    .hint_text("SSB, CW, FT8")
                                    .desired_width(150.0),
                            );
                            ui.label("Submodes");
                            ui.add(
                                egui::TextEdit::singleline(&mut d.submodes)
                                    .hint_text("FT4")
                                    .desired_width(100.0),
                            );
                        });
                    });

                egui::CollapsingHeader::new("Station").show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Call");
                        ui.add(
                            egui::TextEdit::singleline(&mut d.call_pattern)
                                .hint_text("W1* or K?ZD")
                                .desired_width(120.0),
                        );
                        ui.label("DXCC");
                        ui.add(
                            egui::TextEdit::singleline(&mut d.dxcc)
                                .hint_text("291, 339")
                                .desired_width(90.0),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Their grid");
                        ui.add(
                            egui::TextEdit::singleline(&mut d.gridsquare)
                                .hint_text("EM12")
                                .desired_width(80.0),
                        );
                        egui::ComboBox::from_id_salt("grid_precision")
                            .selected_text(match d.grid_precision {
                                GridPrecision::Field => "field (2)",
                                GridPrecision::Square => "square (4)",
                                GridPrecision::Full => "exact",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut d.grid_precision,
                                    GridPrecision::Field,
                                    "field (2)",
                                );
                                ui.selectable_value(
                                    &mut d.grid_precision,
                                    GridPrecision::Square,
                                    "square (4)",
                                );
                                ui.selectable_value(
                                    &mut d.grid_precision,
                                    GridPrecision::Full,
                                    "exact",
                                );
                            });
                    });
                    ui.horizontal(|ui| {
                        ui.label("My grid");
                        ui.add(
                            egui::TextEdit::singleline(&mut d.my_gridsquare)
                                .hint_text("the grid you worked FROM")
                                .desired_width(160.0),
                        );
                    })
                    .response
                    .on_hover_text(
                        "Matches the grid recorded on each contact, so QSOs made from a \
                         previous QTH are still found after you move.",
                    );
                    ui.horizontal(|ui| {
                        ui.label("Contest");
                        ui.add(
                            egui::TextEdit::singleline(&mut d.contest_id)
                                .hint_text("CQ-WW-SSB")
                                .desired_width(140.0),
                        );
                    });
                });

                egui::CollapsingHeader::new("Confirmation").show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(
                            "Online-service state isn't tracked yet, so these match nothing \
                             (or everything) until service sync lands.",
                        )
                        .small()
                        .weak(),
                    );
                    ui.horizontal(|ui| {
                        ui.label("Not uploaded to");
                        ui.add(
                            egui::TextEdit::singleline(&mut d.not_uploaded_to)
                                .hint_text("lotw")
                                .desired_width(80.0),
                        );
                        ui.label("Confirmed by");
                        ui.add(
                            egui::TextEdit::singleline(&mut d.confirmed_by)
                                .hint_text("lotw")
                                .desired_width(80.0),
                        );
                    });
                    ui.checkbox(&mut d.qsl_rcvd_yes, "QSL received (QSL_RCVD = Y)");
                });

                ui.separator();

                // The live result of the filter, in the window where it's being
                // edited — so narrowing it down is a tight loop, not a hunt back
                // to the contact list.
                ui.horizontal(|ui| {
                    if ui.button("Clear all").clicked() {
                        clear = true;
                    }
                    if !self.filter_error.is_empty() {
                        ui.colored_label(egui::Color32::LIGHT_RED, &self.filter_error);
                    } else {
                        ui.label(format!(
                            "{} contact{} match",
                            self.contacts_total,
                            if self.contacts_total == 1 { "" } else { "es" }
                        ));
                    }
                });
            });

        if clear {
            self.qso_filter = QsoFilterDraft::default();
        }
        if !open {
            self.show_filter = false;
        }
    }
}
