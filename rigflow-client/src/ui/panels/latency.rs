use crate::UiState;
use crate::ui::app::RigflowApp;
use eframe::egui;

/// Approximate fixed server-side capture + DSP latency (block size + FIR /
/// decimation group delay), which sits *before* the audio send-stamp and so is
/// not part of the measured network one-way. Rough constant; see `11-dsp-...`.
const RX_PIPELINE_MS: f32 = 25.0;
/// Approximate fixed HL2 TX-FIFO depth ahead of the modulator (not separately
/// reported), added to the measured client-ring + server-queue depths.
const TX_HL2_FIFO_MS: f32 = 10.0;

const SAMPLE_RATE_HZ: f32 = 48_000.0;

impl RigflowApp {
    /// "Latency / Audio" diagnostics panel — a Quisk-style live view of the buffer
    /// occupancy at each stage plus the measured network one-way latency. Displayed
    /// values are producer-smoothed (slow EMA + decaying peak) so they read steadily.
    pub(crate) fn draw_latency_panel(&mut self, ui: &mut egui::Ui, snapshot: &UiState) {
        let m = &snapshot.audio_metrics;
        let diag = snapshot.tx_audio_diag;
        // Measured server→client one-way; applied to both directions below as a
        // symmetric estimate (client→server is not separately measured).
        let net_ms = m.rx_one_way_ms().unwrap_or(0.0);
        let mic_ring_ms = m.tx_ring_peak_ms();
        let server_q_ms = diag.mic_queue_samples as f32 / SAMPLE_RATE_HZ * 1000.0;
        let tx_total = net_ms + mic_ring_ms + server_q_ms + TX_HL2_FIFO_MS;

        // Hold the TX-total value + peak across an over: reset on the key-down
        // edge, update live while keyed, and freeze after unkey — so the readout
        // shows the last over's value (not drifting RX-side estimates) and only
        // changes again on the next keying.  Any TX path keys it, incl. the
        // voice keyer (which keys the server directly via StartMicTx).
        let keyed = snapshot.ssb_ptt_down
            || snapshot.cw_key_down
            || snapshot.tx_tone_running
            || snapshot.tx_tune_running
            || snapshot.cat_ptt
            || snapshot.voice_keyer.is_playing();
        if keyed && !self.latency_tx_keyed {
            self.latency_tx_peak_ms = 0.0;
            self.latency_tx_total_ms = 0.0;
        }
        self.latency_tx_keyed = keyed;
        if keyed {
            self.latency_tx_total_ms = tx_total;
            self.latency_tx_peak_ms = self.latency_tx_peak_ms.max(tx_total);
        }
        let tx_total = self.latency_tx_total_ms;
        let tx_peak = self.latency_tx_peak_ms;

        ui.collapsing(super::panel_header("Latency / Audio"), |ui| {
            // ---- Receive -------------------------------------------------
            ui.label(egui::RichText::new("Receive").strong());
            egui::Grid::new("latency_rx_grid")
                .num_columns(2)
                .spacing([10.0, 2.0])
                .show(ui, |ui| {
                    ui.label("Jitter buffer");
                    ui.label(format!(
                        "{:.0} ms  (peak {:.0})",
                        m.jitter_ms(),
                        m.jitter_peak_ms()
                    ));
                    ui.end_row();

                    ui.label("Server pipeline");
                    ui.label(format!("≈ {RX_PIPELINE_MS:.0} ms (fixed)"));
                    ui.end_row();

                    ui.label("Network one-way");
                    ui.label(net_label(m.rx_one_way_ms()));
                    ui.end_row();

                    ui.label(egui::RichText::new("RX total").strong());
                    let rx_total = RX_PIPELINE_MS + net_ms + m.jitter_ms();
                    let rx_total_peak = RX_PIPELINE_MS + net_ms + m.jitter_peak_ms();
                    ui.label(
                        egui::RichText::new(format!(
                            "≈ {rx_total:.0} ms  (peak {rx_total_peak:.0})"
                        ))
                        .strong(),
                    );
                    ui.end_row();
                });

            let (c, l, o, r) = (m.conceals(), m.late(), m.overflow(), m.resyncs());
            if (c | l | o | r) != 0 {
                ui.label(super::note_text(format!(
                    "conceal {c}  ·  late {l}  ·  overflow {o}  ·  resync {r}"
                )));
            }

            ui.add_space(4.0);

            // ---- Transmit (TX-capable sources only) ----------------------
            if snapshot.source_capabilities.supports_transmit {
                ui.label(egui::RichText::new("Transmit").strong());
                egui::Grid::new("latency_tx_grid")
                    .num_columns(2)
                    .spacing([10.0, 2.0])
                    .show(ui, |ui| {
                        ui.label("Mic ring (client)");
                        ui.label(format!("{mic_ring_ms:.0} ms"));
                        ui.end_row();

                        ui.label("Server queue");
                        ui.label(format!("{server_q_ms:.0} ms"));
                        ui.end_row();

                        ui.label("Network one-way");
                        ui.label(net_label(m.rx_one_way_ms()));
                        ui.end_row();

                        ui.label(egui::RichText::new("TX total").strong());
                        ui.label(
                            egui::RichText::new(format!("≈ {tx_total:.0} ms  (peak {tx_peak:.0})"))
                                .strong(),
                        );
                        ui.end_row();
                    });

                if (diag.underruns | diag.overruns) != 0 {
                    ui.label(
                        egui::RichText::new(format!(
                            "underruns {}  ·  overruns {}",
                            diag.underruns, diag.overruns
                        ))
                        .small()
                        .weak(),
                    );
                }

                ui.add_space(4.0);
            }

            // ---- Network -------------------------------------------------
            ui.label(egui::RichText::new("Network").strong());
            egui::Grid::new("latency_net_grid")
                .num_columns(2)
                .spacing([10.0, 2.0])
                .show(ui, |ui| {
                    ui.label("One-way");
                    ui.label(net_label(m.rx_one_way_ms()));
                    ui.end_row();

                    if let Some(rtt) = m.rtt_ms() {
                        ui.label("Round-trip");
                        ui.label(format!("{rtt:.1} ms"));
                        ui.end_row();
                    }
                    if let Some(off) = m.clock_offset_ms() {
                        ui.label("Clock offset");
                        ui.label(format!("{off:.1} ms"));
                        ui.end_row();
                    }
                });

            ui.label(super::note_text(
                "One-way is measured server»client and applied to both directions \
                 (symmetric estimate). CPAL device buffers are not included. For \
                 FT8/digital (PipeWire or TCI) the network and TX figures apply, but \
                 FT8 bypasses the jitter buffer and WSJT-X's own audio buffering isn't \
                 measured.",
            ));
        });
    }
}

/// Format a network one-way reading, or a placeholder while the first probe is
/// still converging.
fn net_label(one_way_ms: Option<f32>) -> String {
    match one_way_ms {
        Some(ms) => format!("{ms:.1} ms"),
        None => "— (measuring…)".to_string(),
    }
}
