use std::{net::SocketAddr, time::Duration};

use axum::{routing::get, Router};

use radio_server::{
    api::{protocol::ServerMessage, websocket::ws_handler},
    dsp::{demod::Sideband, pipeline::DspPipeline},
    server::app_state::AppState,
    source::factory::{create_source, SourceConfig},
    streaming::audio_binary::f32_samples_to_i16_bytes,
    waterfall::simple::WaterfallGenerator,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let center_freq_hz = 0.0;
    let target_freq_hz = 0.0;
    let sideband = Sideband::Lsb;

    let block_size = 8*8192;
    let waterfall_bins = 512;
    let waterfall_frame_rate_hz = 10.0;

    let state = AppState::new(center_freq_hz, target_freq_hz, sideband);

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state.clone());

    let addr: SocketAddr = "192.168.0.225:9000".parse()?;
    println!("radio_server listening on ws://{addr}/ws");

    let tx = state.tx.clone();
    let audio_tx = state.audio_tx.clone();
    let waterfall_tx = state.waterfall_tx.clone();
    let radio_state = state.radio.clone();

    tokio::spawn(async move {
        let source_config = SourceConfig::WavFile {
            path: "input_iq.wav".to_string(),
        };

        // For quick testing, you can swap to:
        //
        // let source_config = SourceConfig::Fake {
        //     sample_rate_hz: 48_000.0,
        //     tone_hz: 1_500.0,
        // };

        let mut source = match create_source(source_config) {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(ServerMessage::Error {
                    message: format!("failed to create source: {e}"),
                });
                return;
            }
        };

        let input_sample_rate_hz = source.sample_rate();

        let mut pipeline = DspPipeline::new(
            center_freq_hz,
            target_freq_hz,
            input_sample_rate_hz,
            2_800.0, // channel cutoff
            129,     // channel filter taps
            16,      // decimation factor
            2_700.0, // audio cutoff
            101,     // audio FIR taps
            48_000.0, // client/browser output sample rate
        );

        pipeline.set_sideband(sideband);

        let _ = tx.send(ServerMessage::StreamConfig {
            audio_sample_rate_hz: pipeline.client_output_sample_rate(),
            audio_format: "i16".to_string(),
            waterfall_bins,
            waterfall_frame_rate_hz,
            center_freq_hz,
            input_sample_rate_hz,
        });

        let _ = tx.send(ServerMessage::Info {
            message: format!("source initialized at {} Hz", input_sample_rate_hz),
        });

        let mut waterfall = WaterfallGenerator::new(waterfall_bins);

        // Rough frame pacing. If your loop runs every ~20 ms, this gives ~10 FPS.
        let mut wf_counter = 0usize;
        let wf_every_n_blocks = 5usize;

        loop {
            {
                let radio = radio_state.read().await;
                pipeline.set_center_frequency(radio.center_freq_hz);
                pipeline.set_target_frequency(radio.target_freq_hz);
                pipeline.set_sideband(radio.sideband);
            }

            let iq_block = match source.read_block(block_size) {
                Ok(block) => block,
                Err(e) => {
                    let _ = tx.send(ServerMessage::Error {
                        message: format!("source read failed: {e}"),
                    });
                    break;
                }
            };

            if iq_block.is_empty() {
                let _ = tx.send(ServerMessage::Info {
                    message: "source reached end of stream".to_string(),
                });
                break;
            }

            let audio = pipeline.process_audio(&iq_block);
            let audio_bytes = f32_samples_to_i16_bytes(&audio);
            let _ = audio_tx.send(audio_bytes);
	    println!("sent audio bytes");

            wf_counter += 1;
            if wf_counter >= wf_every_n_blocks {
                wf_counter = 0;
                let row = waterfall.generate_row(&iq_block);
                let _ = waterfall_tx.send(row);
            }

            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
