use super::app::RigflowApp;
use eframe::egui;
use egui_plot::{Line, Plot, PlotPoints, Points};
use rigflow_core::radio::swr_sweep::SwrSweepResult;
use std::fmt::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

impl RigflowApp {
    /// SWR Sweep results popup: header summary, Min-SWR readout, an
    /// SWR-vs-frequency plot (hover shows frequency + SWR), and CSV export.
    ///
    /// Results are not persisted — closing the window only hides it; the data
    /// stays in `UiState` until the next sweep overwrites it.
    pub(crate) fn draw_swr_sweep_window(&mut self, ctx: &egui::Context) {
        let mut open = {
            let state = self.state.lock().unwrap();
            state.show_swr_sweep_window
        };

        if !open {
            return;
        }

        let mut export_requested = false;

        egui::Window::new("SWR Sweep Results")
            .open(&mut open)
            .resizable(true)
            .default_width(520.0)
            .show(ctx, |ui| {
                let state = self.state.lock().unwrap();
                let Some(result) = state.swr_sweep_result.clone() else {
                    ui.label("No sweep results yet.");
                    return;
                };
                let csv_status = state.swr_sweep_csv_status.clone();
                drop(state);

                ui.horizontal(|ui| {
                    ui.label(format!(
                        "Start: {:.6} MHz",
                        result.start_hz as f64 / 1_000_000.0
                    ));
                    ui.separator();
                    ui.label(format!(
                        "Stop: {:.6} MHz",
                        result.stop_hz as f64 / 1_000_000.0
                    ));
                    ui.separator();
                    ui.label(format!("Points: {}", result.points.len()));
                });

                match result.min_swr_point() {
                    Some(p) => {
                        ui.label(format!(
                            "Min SWR: {:.2} @ {:.6} MHz",
                            p.swr.unwrap_or(0.0),
                            p.frequency_hz as f64 / 1_000_000.0
                        ));
                    }
                    None => {
                        ui.colored_label(
                            egui::Color32::from_rgb(220, 80, 80),
                            "No valid SWR readings in this sweep.",
                        );
                    }
                }

                ui.separator();

                // SWR-vs-frequency plot (X = MHz, Y = SWR).  Auto-scaled.
                let points: Vec<[f64; 2]> = result
                    .points
                    .iter()
                    .filter_map(|p| {
                        p.swr
                            .map(|swr| [p.frequency_hz as f64 / 1_000_000.0, swr as f64])
                    })
                    .collect();

                Plot::new("swr_sweep_plot")
                    .height(240.0)
                    .x_axis_label("Frequency (MHz)")
                    .y_axis_label("SWR")
                    .label_formatter(|_name, value| {
                        format!("{:.6} MHz\nSWR {:.2}", value.x, value.y)
                    })
                    .show(ui, |plot_ui| {
                        if !points.is_empty() {
                            plot_ui.line(Line::new(PlotPoints::from(points.clone())).name("SWR"));
                            plot_ui.points(
                                Points::new(PlotPoints::from(points.clone()))
                                    .radius(3.0)
                                    .name("SWR"),
                            );
                        }
                    });

                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Export CSV").clicked() {
                        export_requested = true;
                    }
                    if let Some(status) = &csv_status {
                        ui.label(status);
                    }
                });
            });

        if export_requested {
            self.export_swr_sweep_csv();
        }

        // Reflect the window's close affordance back into state.
        if let Ok(mut state) = self.state.lock() {
            state.show_swr_sweep_window = open;
        }
    }

    /// Write the current sweep to `<config_dir>/sweeps/swr_sweep_<timestamp>.csv`
    /// and record the saved path (or an error) in `swr_sweep_csv_status`.  No
    /// filename prompt; the timestamp makes each export unique.
    fn export_swr_sweep_csv(&self) {
        let result = {
            let state = self.state.lock().unwrap();
            state.swr_sweep_result.clone()
        };
        let Some(result) = result else {
            return;
        };

        let status = match write_sweep_csv(&result) {
            Ok(path) => format!("Saved: {}", path),
            Err(e) => format!("Export failed: {e}"),
        };

        if let Ok(mut state) = self.state.lock() {
            state.swr_sweep_csv_status = Some(status);
        }
    }
}

/// Render and write the CSV, returning the saved path as a string.
///
/// Power columns carry the *raw* detector counts (`forward_raw`/`reverse_raw`),
/// because forward/reverse watts are not yet calibrated on the HL2.
fn write_sweep_csv(result: &SwrSweepResult) -> Result<String, String> {
    let config_dir = crate::persistence::resolve_config_dir(None)
        .map_err(|e| format!("no config dir: {e:?}"))?;
    let sweeps_dir = config_dir.join("sweeps");
    std::fs::create_dir_all(&sweeps_dir).map_err(|e| e.to_string())?;

    let file_name = format!("swr_sweep_{}.csv", local_timestamp_compact());
    let path = sweeps_dir.join(&file_name);

    let mut body = String::new();
    body.push_str("frequency_hz,swr,forward_raw,reverse_raw\n");
    for p in &result.points {
        let swr = p.swr.map(|v| format!("{v:.4}")).unwrap_or_default();
        let fwd = p.forward_raw.map(|v| v.to_string()).unwrap_or_default();
        let rev = p.reverse_raw.map(|v| v.to_string()).unwrap_or_default();
        let _ = writeln!(body, "{},{},{},{}", p.frequency_hz, swr, fwd, rev);
    }

    std::fs::write(&path, body).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

/// `YYYY-MM-DD_HHMMSS` in UTC, with no external date dependency.
fn local_timestamp_compact() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let days = (secs / 86_400) as i64;
    let secs_of_day = secs % 86_400;
    let (hh, mm, ss) = (
        secs_of_day / 3_600,
        (secs_of_day % 3_600) / 60,
        secs_of_day % 60,
    );
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}_{hh:02}{mm:02}{ss:02}")
}

/// Convert days since the Unix epoch to a civil (year, month, day).
/// Howard Hinnant's `civil_from_days` algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as i64; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}
