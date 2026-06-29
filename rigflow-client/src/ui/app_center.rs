use super::app::RigflowApp;
use crate::ControlCommand;
use crate::UiState;
use crate::ui::layout::{LEFT_GUTTER, RIGHT_GUTTER, WATERFALL_IMAGE_HEIGHT, WATERFALL_IMAGE_WIDTH};
use crate::ui::spectrum_view::{
    SpectrumInteraction, draw_spectrum_plot, x_frac_to_frequency_hz, zoomed_visible_freq_range_hz,
};
use crate::ui::view_interaction::ViewMouseResult;
use eframe::egui;
use log::warn;
use std::time::{Duration, Instant};

/// Minimum spacing between drag/momentum control-channel sends (~33/s).  Local
/// UI state still updates every frame; only the WebSocket retunes are throttled.
const PAN_SEND_INTERVAL: Duration = Duration::from_millis(30);
/// Exponential (viscous) decay time constant for flick momentum (seconds).
/// Velocity falls by `1/e` every `MOMENTUM_TAU` — this shapes the main glide.
const MOMENTUM_TAU: f32 = 0.35;
/// Constant (Coulomb) deceleration added on top of the exponential decay, in
/// Hz/s².  Subtracted from the speed every frame, it is negligible during the
/// fast glide but dominant at low speed, so momentum terminates in a fraction of
/// a second instead of crawling along the exponential asymptote.  Raise it to
/// make the tail die faster / stop sooner.
const MOMENTUM_DECEL_HZ_PER_S2: f32 = 20_000.0;
/// Momentum stops once its speed drops below this (Hz/s).
const MOMENTUM_MIN_HZ_PER_S: f32 = 100.0;

/// True while the radio is keyed by any path (CW, SSB PTT, CAT PTT, or a test
/// tone).  Drag-pan and momentum are disabled while transmitting so the band
/// filters can't be swept out from under an active transmit.
fn is_transmitting(s: &UiState) -> bool {
    s.cw_key_down || s.ssb_ptt_down || s.cat_ptt || s.tx_tone_running || s.voice_keyer.is_playing()
}

impl RigflowApp {
    pub(crate) fn draw_center_panel(&mut self, ctx: &egui::Context, snapshot: &UiState) {
        // Advance any in-flight flick-momentum pan before drawing this frame.
        self.advance_pan_momentum(ctx, snapshot);
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::Frame::NONE
                .fill(egui::Color32::BLACK)
                .inner_margin(egui::Margin {
                    left: 12,
                    right: 12,
                    top: 4,
                    bottom: 4,
                })
                .show(ui, |ui| {
                    // Top status bar: live operating telemetry (frequency, mode,
                    // S-meter, dBm, TX/RX, SWR, REC).  Allocated first so it
                    // consumes height before the spectrum/waterfall are sized.
                    let status_bar_height = 30.0;
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), status_bar_height),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            self.draw_status_bar(ui, snapshot);
                        },
                    );

                    let lo_strip_height = 34.0;
                    let gap = 6.0;
                    // Dual-watch stacks VFO B below VFO A: spectrum A shrinks and
                    // a VFO B spectrum + waterfall pane is added below.
                    let dual = snapshot.dual_watch_enabled;
                    let spectrum_height = if dual { 140.0 } else { 220.0 };
                    let spectrum_b_height = 120.0;
                    // The waterfall heights are computed later, from the *actual*
                    // remaining height just before they are drawn, so the VFO A and
                    // VFO B waterfalls come out exactly equal.

                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), lo_strip_height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| self.draw_lo_strip(ui, TuneVfo::A),
                    );

                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), spectrum_height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            let spectrum_snapshot = {
                                let guard = self.spectrum_db.lock().unwrap();
                                guard.clone()
                            };

                            let state_snapshot = {
                                let state = self.state.lock().unwrap();
                                state.clone()
                            };

                            let (spectrum_db_min, spectrum_db_max) =
                                if state_snapshot.adaptive_waterfall_normalization {
                                    let top = state_snapshot.adaptive_top_db_estimate + 3.0;
                                    (top - state_snapshot.adaptive_range_db_estimate, top)
                                } else {
                                    let top = state_snapshot.manual_waterfall_top_db;
                                    (top - state_snapshot.manual_waterfall_range_db, top)
                                };

                            let interaction: SpectrumInteraction = draw_spectrum_plot(
                                ui,
                                egui::vec2(ui.available_width(), spectrum_height),
                                &spectrum_snapshot,
                                spectrum_db_min,
                                spectrum_db_max,
                                &state_snapshot,
                            );

                            // Bookmark clicks take precedence; all other mouse
                            // behaviour (tune / recenter / wheel / zoom) goes
                            // through the shared handler so the spectrum and
                            // waterfall behave identically.
                            if let Some(bookmark_id) = interaction.clicked_bookmark_id {
                                self.apply_bookmark(&bookmark_id);
                            }
                            self.apply_view_interaction(&interaction.mouse, snapshot, TuneVfo::A);
                        },
                    );

                    // ── VFO B spectrum (dual-watch): stacked below VFO A ──────
                    if dual {
                        ui.add_space(gap);
                        // VFO B's own LO + LO-Offset spinner strip.
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), lo_strip_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| self.draw_lo_strip(ui, TuneVfo::B),
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), spectrum_b_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                let b_view = vfo_b_view(snapshot);
                                let spectrum_b = {
                                    let g = self.spectrum_db_b.lock().unwrap();
                                    g.clone()
                                };
                                let (db_min, db_max) = if b_view.adaptive_waterfall_normalization {
                                    let top = b_view.adaptive_top_db_estimate + 3.0;
                                    (top - b_view.adaptive_range_db_estimate, top)
                                } else {
                                    let top = b_view.manual_waterfall_top_db;
                                    (top - b_view.manual_waterfall_range_db, top)
                                };
                                let inter = draw_spectrum_plot(
                                    ui,
                                    egui::vec2(ui.available_width(), spectrum_b_height),
                                    &spectrum_b,
                                    db_min,
                                    db_max,
                                    &b_view,
                                );
                                // Full VFO B tuning — click / drag / wheel / recenter,
                                // identical to VFO A but on VFO B's centre/target.
                                self.apply_view_interaction(&inter.mouse, snapshot, TuneVfo::B);
                            },
                        );
                    }

                    ui.add_space(gap);
                    ui.separator();
                    ui.add_space(gap);

                    // Split the remaining height equally between the VFO A and VFO B
                    // waterfalls.  Measured here (after the spectra are laid out) so
                    // both halves are exactly equal regardless of what the panels
                    // above consumed; the `between` reserve covers the separator +
                    // gaps that sit between the two waterfalls so B is never clipped.
                    let waterfall_height;
                    let waterfall_b_height;
                    {
                        let remaining = ui.available_height();
                        if dual {
                            let item_sp = ui.spacing().item_spacing.y;
                            let between = gap * 2.0 + 2.0 + item_sp * 4.0;
                            let each = ((remaining - between) / 2.0).max(60.0);
                            waterfall_height = each;
                            waterfall_b_height = each;
                        } else {
                            waterfall_height = remaining.max(120.0);
                            waterfall_b_height = 0.0;
                        }
                    }

                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), waterfall_height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            self.update_waterfall_texture(
                                ctx,
                                WATERFALL_IMAGE_WIDTH,
                                WATERFALL_IMAGE_HEIGHT,
                            );

                            if let Some(texture) = &self.waterfall_texture {
                                let image_width =
                                    (ui.available_width() - LEFT_GUTTER - RIGHT_GUTTER).max(100.0);

                                // Shared mouse interaction (identical to the
                                // spectrum): the waterfall maps screen-x →
                                // frequency over the image rect via the same
                                // zoomed visible range.
                                let mut mouse = ViewMouseResult::default();

                                ui.horizontal(|ui| {
                                    ui.add_space(LEFT_GUTTER);

                                    let image = egui::Image::new((
                                        texture.id(),
                                        egui::vec2(image_width, waterfall_height),
                                    ))
                                    .sense(egui::Sense::click_and_drag());

                                    let response = ui.add(image);
                                    let rect = response.rect;

                                    let state_snapshot = {
                                        let state = self.state.lock().unwrap();
                                        state.clone()
                                    };
                                    let spectrum_len = {
                                        let spectrum = self.spectrum_db.lock().unwrap();
                                        spectrum.len()
                                    };

                                    mouse = crate::ui::view_interaction::handle_view_mouse(
                                        ui,
                                        &response,
                                        rect,
                                        |x| {
                                            let frac =
                                                ((x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                                            if let Some((left_hz, right_hz)) =
                                                zoomed_visible_freq_range_hz(
                                                    spectrum_len,
                                                    &state_snapshot,
                                                )
                                            {
                                                left_hz + frac * (right_hz - left_hz)
                                            } else {
                                                x_frac_to_frequency_hz(frac, &state_snapshot)
                                            }
                                        },
                                    );
                                });

                                self.apply_view_interaction(&mouse, snapshot, TuneVfo::A);
                            }
                        },
                    );

                    // ── VFO B waterfall (dual-watch): stacked below A's, with a
                    //    separator between the two waterfalls ───────────────────
                    if dual {
                        ui.add_space(gap);
                        ui.separator();
                        ui.add_space(gap);
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), waterfall_b_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                render_waterfall_texture(
                                    ctx,
                                    WATERFALL_IMAGE_WIDTH,
                                    WATERFALL_IMAGE_HEIGHT,
                                    &self.waterfall_buffer_b,
                                    &mut self.waterfall_texture_b,
                                    "waterfall_texture_b",
                                );
                                if let Some(texture) = &self.waterfall_texture_b {
                                    let image_width =
                                        (ui.available_width() - LEFT_GUTTER - RIGHT_GUTTER)
                                            .max(100.0);
                                    let b_view = vfo_b_view(snapshot);
                                    let spectrum_len = {
                                        let s = self.spectrum_db_b.lock().unwrap();
                                        s.len()
                                    };
                                    let mut wf_mouse = ViewMouseResult::default();
                                    ui.horizontal(|ui| {
                                        ui.add_space(LEFT_GUTTER);
                                        let image = egui::Image::new((
                                            texture.id(),
                                            egui::vec2(image_width, waterfall_b_height),
                                        ))
                                        .sense(egui::Sense::click_and_drag());
                                        let response = ui.add(image);
                                        let rect = response.rect;
                                        wf_mouse = crate::ui::view_interaction::handle_view_mouse(
                                            ui,
                                            &response,
                                            rect,
                                            |x| {
                                                let frac = ((x - rect.left()) / rect.width())
                                                    .clamp(0.0, 1.0);
                                                if let Some((l, r)) = zoomed_visible_freq_range_hz(
                                                    spectrum_len,
                                                    &b_view,
                                                ) {
                                                    l + frac * (r - l)
                                                } else {
                                                    x_frac_to_frequency_hz(frac, &b_view)
                                                }
                                            },
                                        );
                                    });
                                    self.apply_view_interaction(&wf_mouse, snapshot, TuneVfo::B);
                                }
                            },
                        );
                    }
                });
        });
    }

    /// Top status bar: compact, single-row live operating telemetry.  Reads
    /// only from the UI snapshot (no protocol changes); optional fields (SWR,
    /// REC) are omitted when unavailable.  Structured as a left-to-right row so
    /// future items (TX power, ALC, network status, …) just append more cells.
    fn draw_status_bar(&self, ui: &mut egui::Ui, snapshot: &UiState) {
        use crate::ui::panels::s_meter_label;
        use crate::ui::state::{ProblemSeverity, collect_problems};

        // Status indicator light — always visible (before the no-radio
        // early-return) so a failure is never hidden.  A filled circle: green =
        // all OK, amber = warnings only, red = an error.  Hover lists the
        // details (same source as the "Status / Problems" panel).
        let problems = collect_problems(snapshot);
        let has_error = problems
            .iter()
            .any(|p| p.severity == ProblemSeverity::Error);
        let color = if problems.is_empty() {
            egui::Color32::from_rgb(40, 200, 80) // green: all subsystems OK
        } else if has_error {
            egui::Color32::from_rgb(230, 60, 60) // red: error
        } else {
            egui::Color32::from_rgb(255, 170, 40) // amber: warnings only
        };
        let hover = if problems.is_empty() {
            "All subsystems OK".to_string()
        } else {
            problems
                .iter()
                .map(|p| format!("{}: {}", p.source, p.detail))
                .collect::<Vec<_>>()
                .join("\n")
        };
        // LED-style status light: a solid, color-filled circle.
        let diameter = 20.0;
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(diameter, diameter), egui::Sense::hover());
        ui.painter()
            .circle_filled(rect.center(), diameter * 0.45, color);
        response.on_hover_text(hover);
        ui.separator();

        // Operator + license first (shown whenever an operator is selected,
        // even before a radio is acquired).  Operator is coloured to stand out;
        // the license is normal text.
        if !snapshot.operator_id.is_empty() {
            ui.label(
                egui::RichText::new(&snapshot.operator_id)
                    .size(20.0)
                    .strong()
                    .color(egui::Color32::from_rgb(90, 200, 255)),
            );
            if let Some(license) = snapshot.selected_license {
                ui.label(license_label(license));
            }
            ui.separator();
        }

        if !snapshot.radio_acquired {
            ui.label(egui::RichText::new("No radio acquired").weak());
            return;
        }

        // Frequency (operating / target) — prominent.
        ui.label(
            egui::RichText::new(format_freq_dotted(snapshot.target_freq_hz.max(0.0) as u64))
                .size(18.0)
                .strong(),
        );
        // Mode.
        ui.label(egui::RichText::new(mode_label(snapshot.demod_mode)).strong());

        // VFO B + split / RIT / XIT badges (only when relevant).
        if snapshot.split_enabled || snapshot.dual_watch_enabled {
            ui.separator();
            let b_tx =
                snapshot.split_enabled && snapshot.tx_vfo == rigflow_core::radio::vfo::VfoSelect::B;
            ui.label(
                egui::RichText::new(format!(
                    "B {}  {}",
                    format_freq_dotted(snapshot.vfo_b_target_freq_hz.max(0.0) as u64),
                    mode_label(snapshot.vfo_b_demod_mode)
                ))
                .color(if b_tx {
                    egui::Color32::from_rgb(235, 80, 80)
                } else {
                    egui::Color32::from_rgb(160, 200, 235)
                }),
            );
        }
        if snapshot.split_enabled {
            let tx_letter = if snapshot.tx_vfo == rigflow_core::radio::vfo::VfoSelect::B {
                "B"
            } else {
                "A"
            };
            ui.label(
                egui::RichText::new(format!("SPLIT▶{tx_letter}"))
                    .strong()
                    .color(egui::Color32::from_rgb(235, 180, 70)),
            );
        }
        if snapshot.rit_enabled && snapshot.rit_offset_hz != 0 {
            ui.label(
                egui::RichText::new(format!("RIT {:+} Hz", snapshot.rit_offset_hz))
                    .strong()
                    .color(egui::Color32::from_rgb(120, 210, 255)),
            );
        }
        if snapshot.xit_enabled && snapshot.xit_offset_hz != 0 {
            ui.label(
                egui::RichText::new(format!("XIT {:+} Hz", snapshot.xit_offset_hz))
                    .strong()
                    .color(egui::Color32::from_rgb(235, 180, 70)),
            );
        }

        ui.separator();

        // S-meter — the most prominent item (largest text, coloured).
        ui.label(
            egui::RichText::new(s_meter_label(snapshot.signal_dbm))
                .size(20.0)
                .strong()
                .color(egui::Color32::from_rgb(120, 230, 120)),
        );
        ui.label(egui::RichText::new(format!("{:.0} dBm", snapshot.signal_dbm)).size(15.0));

        ui.separator();

        // TX / RX state.
        let transmitting = snapshot.ssb_ptt_down
            || snapshot.cw_key_down
            || snapshot.tx_tone_running
            || snapshot.tx_tune_running
            || snapshot.cat_ptt
            || snapshot.voice_keyer.is_playing();
        if transmitting {
            ui.label(
                egui::RichText::new("TX")
                    .strong()
                    .color(egui::Color32::from_rgb(235, 80, 80)),
            );
        } else {
            ui.label(egui::RichText::new("RX").weak());
        }

        // SWR — shown only when the source reports it.
        if let Some(swr) = snapshot.source_status.swr {
            ui.separator();
            ui.label(format!("SWR {swr:.1}"));
        }

        // Recording — shown only while a recording is active.
        if snapshot.iq_recording_status.recording {
            ui.separator();
            ui.label(
                egui::RichText::new("REC")
                    .strong()
                    .color(egui::Color32::from_rgb(235, 90, 90)),
            );
        }
    }

    /// Apply a shared Spectrum/Waterfall mouse result through the existing
    /// clamp/tune/zoom paths.  Used identically by both views, so their mouse
    /// behaviour is guaranteed consistent.  Tuning validation is unchanged
    /// (the server still validates every target); zoom only adjusts the local
    /// display.
    /// Move the target frequency by `delta_hz` with soft-edge LO panning: as the
    /// target nears the visible edge, shift the LO center so tuning keeps going
    /// instead of hitting the dead zone at `center ± sample_rate/2`.  Shared by
    /// the mouse wheel and the ←/→ arrow keys so both behave identically.  The
    /// caller must ensure a radio is acquired.
    pub(crate) fn tune_target_relative(&self, snapshot: &UiState, delta_hz: f32, vfo: TuneVfo) {
        use crate::ui::freq_limits::{active_freq_limits, clamp_center, clamp_target};

        if delta_hz == 0.0 {
            return;
        }
        let limits = active_freq_limits(snapshot);
        let cur_center = vfo.center(snapshot);

        // Move the target by the step, clamped only to the RF range (NOT the
        // visible band) so it can cross the soft edge; the LO follows it.
        let desired_target = clamp_center(vfo.target(snapshot) + delta_hz, &limits);

        // Soft threshold = 80% of the visible half-span (zoom-aware: the visible
        // span is sample_rate / display_zoom, centered on the LO).
        let half_span =
            (snapshot.input_sample_rate_hz / (2.0 * snapshot.display_zoom.max(1.0))).max(0.0);
        let soft = 0.8 * half_span;

        // Pan the LO by the excess past the threshold so the target settles back
        // at ~±soft (symmetric for the left and right edges).  The target itself
        // always moves by exactly one step — no jumps.
        let mut new_center = cur_center;
        let offset = desired_target - new_center;
        if half_span > 0.0 && offset.abs() > soft {
            let excess = offset.abs() - soft;
            new_center = clamp_center(new_center + offset.signum() * excess, &limits);
        }

        let new_target = clamp_target(
            desired_target,
            new_center,
            snapshot.input_sample_rate_hz,
            &limits,
        );

        if let Ok(mut state) = self.state.lock() {
            vfo.set_center(&mut state, new_center);
            vfo.set_target(&mut state, new_target);
        }
        // Retune the LO only when it actually panned.
        if (new_center - cur_center).abs() > 0.5 {
            let _ = self.ws_cmd_tx.send(ControlCommand::RadioMessage(
                vfo.center_msg(new_center as u64),
            ));
        }
        let _ = self.ws_cmd_tx.send(ControlCommand::RadioMessage(
            vfo.target_msg(new_target as u64),
        ));
    }

    /// Pan the target frequency by `delta_hz`, applying the same soft-edge LO
    /// pan as [`tune_target_relative`] but with **throttled** control-channel
    /// sends so a 60 fps drag / momentum sweep doesn't flood the server with
    /// retunes.  Local UI state is updated on every call (smooth display);
    /// `SetTargetFrequency` / `SetCenterFrequency` are sent at most once per
    /// [`PAN_SEND_INTERVAL`] unless `force_send` is set (used for the final,
    /// exact value when a gesture ends, so the server lands on the settled
    /// frequency with the correct band filters).
    ///
    /// Returns `true` if the target actually moved; `false` means it was pinned
    /// at a band edge — the caller uses that to stop momentum.
    fn pan_target_by(
        &self,
        snapshot: &UiState,
        delta_hz: f32,
        force_send: bool,
        vfo: TuneVfo,
    ) -> bool {
        use crate::ui::freq_limits::{active_freq_limits, clamp_center, clamp_target};

        if delta_hz == 0.0 && !force_send {
            return false;
        }
        let limits = active_freq_limits(snapshot);
        let cur_center = vfo.center(snapshot);
        let cur_target = vfo.target(snapshot);
        let desired_target = clamp_center(cur_target + delta_hz, &limits);

        // Soft-edge LO pan — identical math to `tune_target_relative`.
        let half_span =
            (snapshot.input_sample_rate_hz / (2.0 * snapshot.display_zoom.max(1.0))).max(0.0);
        let soft = 0.8 * half_span;
        let mut new_center = cur_center;
        let offset = desired_target - new_center;
        if half_span > 0.0 && offset.abs() > soft {
            let excess = offset.abs() - soft;
            new_center = clamp_center(new_center + offset.signum() * excess, &limits);
        }
        let new_target = clamp_target(
            desired_target,
            new_center,
            snapshot.input_sample_rate_hz,
            &limits,
        );

        let moved = (new_target - cur_target).abs() > 0.5 || (new_center - cur_center).abs() > 0.5;

        // Apply locally every frame; decide under the lock whether this frame's
        // send passes the throttle, and whether the LO changed and must be sent.
        let (do_center, do_target) = {
            let mut state = match self.state.lock() {
                Ok(s) => s,
                Err(_) => return moved,
            };
            vfo.set_center(&mut state, new_center);
            vfo.set_target(&mut state, new_target);

            let now = Instant::now();
            let allow = force_send || now.duration_since(state.last_pan_send) >= PAN_SEND_INTERVAL;
            if !allow {
                (false, false)
            } else {
                state.last_pan_send = now;
                // Send the LO only when it changed since the last sent value, so
                // a center change that lands in a throttled frame still reaches
                // the server on the next allowed send.
                let send_center = (new_center - state.last_sent_center_hz).abs() > 0.5;
                if send_center {
                    state.last_sent_center_hz = new_center;
                }
                (send_center, true)
            }
        };

        if do_center {
            let _ = self.ws_cmd_tx.send(ControlCommand::RadioMessage(
                vfo.center_msg(new_center as u64),
            ));
        }
        if do_target {
            let _ = self.ws_cmd_tx.send(ControlCommand::RadioMessage(
                vfo.target_msg(new_target as u64),
            ));
        }
        moved
    }

    /// Advance flick-momentum panning by one frame.  Called every frame from
    /// `draw_center_panel`; a no-op unless a fling is active.  Decays the
    /// velocity exponentially and stops at the band edge, on transmit, or once
    /// the speed falls below [`MOMENTUM_MIN_HZ_PER_S`], force-sending the final
    /// exact frequency so the server's band filters match before any TX.
    pub(crate) fn advance_pan_momentum(&self, ctx: &egui::Context, snapshot: &UiState) {
        let v = snapshot.pan_velocity_hz_per_s;
        if v == 0.0 {
            return;
        }

        // Never sweep while transmitting or with no radio acquired.
        if !snapshot.radio_acquired || is_transmitting(snapshot) {
            if let Ok(mut s) = self.state.lock() {
                s.pan_velocity_hz_per_s = 0.0;
            }
            return;
        }

        let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 0.1);
        // Decay first so we know whether this is the stopping frame.  Viscous
        // (exponential) decay shapes the glide; a constant Coulomb term then
        // subtracts a fixed speed each frame so the tail terminates quickly
        // instead of asymptoting toward zero.
        let mut new_v = v * (-dt / MOMENTUM_TAU).exp();
        new_v = new_v.signum() * (new_v.abs() - MOMENTUM_DECEL_HZ_PER_S2 * dt).max(0.0);
        let stopping = new_v.abs() < MOMENTUM_MIN_HZ_PER_S;

        // Move by this frame's velocity; force the exact final send when stopping
        // so the server lands precisely on the settled frequency.
        let moved = self.pan_target_by(snapshot, v * dt, stopping, TuneVfo::A);
        if stopping || !moved {
            new_v = 0.0;
        }

        if let Ok(mut s) = self.state.lock() {
            s.pan_velocity_hz_per_s = new_v;
        }
        if new_v != 0.0 {
            ctx.request_repaint();
        }
    }

    fn apply_view_interaction(&self, r: &ViewMouseResult, snapshot: &UiState, vfo: TuneVfo) {
        use crate::ui::freq_limits::{active_freq_limits, clamp_center, clamp_target};

        let send = |msg: rigflow_protocol::ClientRadioMessage| {
            let _ = self.ws_cmd_tx.send(ControlCommand::RadioMessage(msg));
        };

        // Any explicit tune / zoom / click cancels an in-flight momentum sweep,
        // and — under dual-watch — focuses this view's VFO so the Receive-panel
        // controls follow the spectrum the operator is working on.
        if r.tune_dir != 0 || r.tune_to_hz.is_some() || r.center_on_target || r.zoom_steps != 0 {
            if let Ok(mut state) = self.state.lock() {
                state.pan_velocity_hz_per_s = 0.0;
                if state.dual_watch_enabled {
                    state.active_control_vfo = match vfo {
                        TuneVfo::A => rigflow_core::radio::vfo::VfoSelect::A,
                        TuneVfo::B => rigflow_core::radio::vfo::VfoSelect::B,
                    };
                }
            }
        }

        // Ctrl+wheel zoom — display only (×1.25 per notch, clamped to the same
        // 1..4 range as the zoom slider).  Works regardless of acquisition.
        if r.zoom_steps != 0 {
            let factor = 1.25_f32.powi(r.zoom_steps);
            if let Ok(mut state) = self.state.lock() {
                state.display_zoom = (state.display_zoom * factor).clamp(1.0, 4.0);
            }
        }

        // Wheel fine-tune: resolve the mode-aware Hz step here (this code has the
        // demod mode; the shared mouse handler does not) and apply it through the
        // common relative-tune path, which also handles soft-edge LO panning.
        if r.tune_dir != 0 && snapshot.radio_acquired {
            let step =
                crate::ui::tuning_steps::target_step_hz(vfo.demod_mode(snapshot), r.tune_tier);
            self.tune_target_relative(snapshot, r.tune_dir as f32 * step, vfo);
        }

        // Single-click → tune target to the clicked frequency.
        if let Some(freq_hz) = r.tune_to_hz {
            if snapshot.radio_acquired {
                let limits = active_freq_limits(snapshot);
                let new_target = clamp_target(
                    freq_hz,
                    vfo.center(snapshot),
                    snapshot.input_sample_rate_hz,
                    &limits,
                );
                if let Ok(mut state) = self.state.lock() {
                    vfo.set_target(&mut state, new_target);
                }
                send(vfo.target_msg(new_target as u64));
            } else if let Ok(mut state) = self.state.lock() {
                state.server_status = "cannot tune: no radio acquired".to_string();
            }
        }

        // `C` key → center the LO on the current target frequency and zero the
        // LO offset (center == target == old target).  The tuned signal stays
        // put; the display just recenters on it.
        if r.center_on_target {
            if snapshot.radio_acquired {
                let limits = active_freq_limits(snapshot);
                let new_center = clamp_center(vfo.target(snapshot), &limits);
                let new_target = clamp_target(
                    new_center,
                    new_center,
                    snapshot.input_sample_rate_hz,
                    &limits,
                );
                if let Ok(mut state) = self.state.lock() {
                    vfo.set_center(&mut state, new_center);
                    vfo.set_target(&mut state, new_target);
                }
                send(vfo.center_msg(new_center as u64));
                send(vfo.target_msg(new_target as u64));
            } else if let Ok(mut state) = self.state.lock() {
                state.server_status = "cannot tune: no radio acquired".to_string();
            }
        }

        // Click-drag pan (grab-and-slide) — live, throttled.  An active drag
        // cancels any prior momentum so the two don't fight.  Disabled while
        // transmitting so the band can't be swept out from under a transmit.
        if r.drag_delta_hz != 0.0 && snapshot.radio_acquired && !is_transmitting(snapshot) {
            if let Ok(mut state) = self.state.lock() {
                state.pan_velocity_hz_per_s = 0.0;
            }
            self.pan_target_by(snapshot, r.drag_delta_hz, false, vfo);
        }

        // Flick release → seed momentum (VFO A only; the shared momentum field
        // animates VFO A's spectrum).  VFO B still pans live during the drag.
        if let Some(v) = r.fling_velocity_hz_per_s {
            if vfo == TuneVfo::A && snapshot.radio_acquired && !is_transmitting(snapshot) {
                if let Ok(mut state) = self.state.lock() {
                    state.pan_velocity_hz_per_s = v;
                }
            }
        }
    }

    /// Draw the LO + LO-Offset spinner strip for `vfo`.  Offset-preserving LO
    /// (the tuned target rides with the LO) and soft-edge LO-Offset (the LO
    /// follows the target at the passband edge) — identical for VFO A and VFO B.
    fn draw_lo_strip(&self, ui: &mut egui::Ui, vfo: TuneVfo) {
        use crate::ui::freq_limits::{active_freq_limits, clamp_center, clamp_target};

        let state_snapshot = {
            let state = self.state.lock().unwrap();
            state.clone()
        };
        let strip_rect = ui.max_rect();
        let lo_y = strip_rect.top() + 2.0;
        let lo_pos = egui::Pos2::new(strip_rect.left() + 12.0, lo_y);
        let lo_offset_pos = egui::Pos2::new(strip_rect.right() - 12.0, lo_y);

        let cur_center = vfo.center(&state_snapshot);
        let cur_target = vfo.target(&state_snapshot);
        let mut new_center_freq_hz = None;
        let mut new_target_freq_hz = None;

        if let Some(new_center_hz) = crate::widgets::lo_frequency_widget::draw_lo_widget(
            ui,
            lo_pos,
            cur_center.max(0.0) as u64,
        ) {
            let limits = active_freq_limits(&state_snapshot);
            let clamped_center = clamp_center(new_center_hz as f32, &limits);
            new_center_freq_hz = Some(clamped_center);
            // Offset-preserving LO: shift the target by the same delta so the LO
            // Offset stays constant and the target never leaves center ± sr/2.
            let delta = clamped_center - cur_center;
            new_target_freq_hz = Some(clamp_target(
                cur_target + delta,
                clamped_center,
                state_snapshot.input_sample_rate_hz,
                &limits,
            ));
        }

        let lo_offset_hz = (cur_target - cur_center).round() as i64;
        if let Some(new_offset_hz) = crate::widgets::lo_frequency_widget::draw_lo_offset_widget(
            ui,
            lo_offset_pos,
            lo_offset_hz,
        ) {
            // Reuse the soft-edge LO pan (LO follows the target at the edge).
            let delta = (new_offset_hz - lo_offset_hz) as f32;
            self.tune_target_relative(&state_snapshot, delta, vfo);
        }

        if let Some(new_center_hz) = new_center_freq_hz {
            if let Ok(mut state) = self.state.lock() {
                vfo.set_center(&mut state, new_center_hz);
            }
            let _ = self.ws_cmd_tx.send(ControlCommand::RadioMessage(
                vfo.center_msg(new_center_hz as u64),
            ));
        }
        if let Some(new_target_hz) = new_target_freq_hz {
            if let Ok(mut state) = self.state.lock() {
                vfo.set_target(&mut state, new_target_hz);
            }
            let _ = self.ws_cmd_tx.send(ControlCommand::RadioMessage(
                vfo.target_msg(new_target_hz as u64),
            ));
        }
    }

    fn update_waterfall_texture(&mut self, ctx: &egui::Context, wf_width: usize, wf_height: usize) {
        render_waterfall_texture(
            ctx,
            wf_width,
            wf_height,
            &self.waterfall_buffer,
            &mut self.waterfall_texture,
            "waterfall_texture",
        );
    }
}

/// Format a frequency in Hz with `.` thousands separators, e.g.
/// `14074000 → "14.074.000"`.
/// Which VFO a tuning gesture operates on.  Abstracts the centre/target state
/// fields and the protocol messages so the tuning helpers (relative tune, drag
/// pan, click-to-tune, recenter, LO/LO-Offset spinners) are VFO-agnostic and
/// VFO B tunes identically to VFO A.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum TuneVfo {
    A,
    B,
}

impl TuneVfo {
    fn center(self, s: &UiState) -> f32 {
        match self {
            TuneVfo::A => s.center_freq_hz,
            TuneVfo::B => s.vfo_b_center_freq_hz,
        }
    }
    fn target(self, s: &UiState) -> f32 {
        match self {
            TuneVfo::A => s.target_freq_hz,
            TuneVfo::B => s.vfo_b_target_freq_hz,
        }
    }
    fn set_center(self, s: &mut UiState, hz: f32) {
        match self {
            TuneVfo::A => s.center_freq_hz = hz,
            TuneVfo::B => s.vfo_b_center_freq_hz = hz,
        }
    }
    fn set_target(self, s: &mut UiState, hz: f32) {
        match self {
            TuneVfo::A => s.target_freq_hz = hz,
            TuneVfo::B => s.vfo_b_target_freq_hz = hz,
        }
    }
    fn demod_mode(self, s: &UiState) -> rigflow_core::dsp::modes::DemodMode {
        match self {
            TuneVfo::A => s.demod_mode,
            TuneVfo::B => s.vfo_b_demod_mode,
        }
    }
    fn center_msg(self, hz: u64) -> rigflow_protocol::ClientRadioMessage {
        use rigflow_protocol::ClientRadioMessage as M;
        match self {
            TuneVfo::A => M::SetCenterFrequency { center_freq_hz: hz },
            TuneVfo::B => M::SetVfoBCenterFrequency { center_freq_hz: hz },
        }
    }
    fn target_msg(self, hz: u64) -> rigflow_protocol::ClientRadioMessage {
        use rigflow_protocol::ClientRadioMessage as M;
        match self {
            TuneVfo::A => M::SetTargetFrequency { target_freq_hz: hz },
            TuneVfo::B => M::SetVfoBFrequency { target_freq_hz: hz },
        }
    }
}

/// A VFO-A-shaped view of VFO B's state, so the spectrum/waterfall drawing and
/// the screen-x → frequency mapping work for the second receiver unchanged.
fn vfo_b_view(snapshot: &UiState) -> UiState {
    let mut v = snapshot.clone();
    v.center_freq_hz = snapshot.vfo_b_center_freq_hz;
    v.target_freq_hz = snapshot.vfo_b_target_freq_hz;
    v.demod_mode = snapshot.vfo_b_demod_mode;
    v.sideband = snapshot.vfo_b_sideband;
    v.filter_bandwidth_hz = snapshot.vfo_b_filter_bandwidth_hz;
    v.signal_dbm = snapshot.vfo_b_signal_dbm;
    v.signal_s_units = snapshot.vfo_b_signal_s_units;
    // VFO B's own waterfall display settings (zoom / normalization / estimates),
    // so the B spectrum + waterfall render independently of VFO A.
    v.display_zoom = snapshot.vfo_b_display_zoom;
    v.adaptive_waterfall_normalization = snapshot.vfo_b_adaptive_waterfall_normalization;
    v.manual_waterfall_top_db = snapshot.vfo_b_manual_waterfall_top_db;
    v.manual_waterfall_range_db = snapshot.vfo_b_manual_waterfall_range_db;
    v.adaptive_top_db_estimate = snapshot.vfo_b_adaptive_top_db_estimate;
    v.adaptive_range_db_estimate = snapshot.vfo_b_adaptive_range_db_estimate;
    v
}

/// Render a waterfall pixel buffer (ARGB u32) into `texture`, reused for both
/// VFO A and VFO B so the two stacked waterfalls share one code path.
fn render_waterfall_texture(
    ctx: &egui::Context,
    wf_width: usize,
    wf_height: usize,
    buffer: &std::sync::Arc<std::sync::Mutex<Vec<u32>>>,
    texture: &mut Option<egui::TextureHandle>,
    name: &str,
) {
    let pixels = {
        let guard = buffer.lock().unwrap();
        guard.clone()
    };
    if pixels.len() != wf_width * wf_height {
        warn!(
            "waterfall texture size mismatch: pixels={} expected={}",
            pixels.len(),
            wf_width * wf_height
        );
        return;
    }
    let mut image = egui::ColorImage::new([wf_width, wf_height], egui::Color32::BLACK);
    for (dst, src) in image.pixels.iter_mut().zip(pixels.iter()) {
        let rgb = *src;
        *dst = egui::Color32::from_rgb(
            ((rgb >> 16) & 0xff) as u8,
            ((rgb >> 8) & 0xff) as u8,
            (rgb & 0xff) as u8,
        );
    }
    match texture {
        Some(t) => t.set(image, egui::TextureOptions::NEAREST),
        None => *texture = Some(ctx.load_texture(name, image, egui::TextureOptions::NEAREST)),
    }
}

fn format_freq_dotted(hz: u64) -> String {
    let digits = hz.to_string();
    let n = digits.len();
    let mut out = String::with_capacity(n + n / 3);
    for (i, c) in digits.chars().enumerate() {
        if i != 0 && (n - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// Compact license-class label for the status bar.
fn license_label(license: crate::ui::om_bands::LicenseClass) -> &'static str {
    use crate::ui::om_bands::LicenseClass;
    match license {
        LicenseClass::AmateurExtra => "Extra",
        LicenseClass::Advanced => "Advanced",
        LicenseClass::General => "General",
        LicenseClass::Technician => "Technician",
        LicenseClass::Novice => "Novice",
    }
}

/// Uppercase mode label for the status bar.
fn mode_label(mode: rigflow_core::dsp::modes::DemodMode) -> &'static str {
    use rigflow_core::dsp::modes::DemodMode;
    match mode {
        DemodMode::Wfm => "WFM",
        DemodMode::Nfm => "NFM",
        DemodMode::Usb => "USB",
        DemodMode::Lsb => "LSB",
        DemodMode::Am => "AM",
        DemodMode::Cwu => "CWU",
        DemodMode::Cwl => "CWL",
        DemodMode::DgtU => "DATA-U",
    }
}
