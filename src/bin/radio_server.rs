use std::net::SocketAddr;
use std::time::Duration;

use axum::{routing::get, Router};
use tokio::time::Instant;

use radio_server::{
    api::{protocol::ServerMessage, websocket::ws_handler},
    dsp::{demod::Sideband, pipeline::DspPipeline},
    server::app_state::AppState,
    source::factory::{create_source, SourceConfig},
    streaming::{
        audio_binary::f32_samples_to_i16_bytes,
        udp_audio::UdpAudioSender,
        udp_registration::run_udp_registration_listener,
	udp_waterfall::UdpWaterfallSender,
    },
    waterfall::simple::WaterfallGenerator,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let center_freq_hz = 3750000.0;
    let target_freq_hz = 3690000.0;
    let sideband = Sideband::Lsb;

    let block_size = 8192;
    let waterfall_bins = 512;
    let waterfall_frame_rate_hz = 10.0;

    let ws_addr: SocketAddr = "127.0.0.1:9000".parse()?;
    let udp_registration_addr = "0.0.0.0:9001";
    let udp_registration_port = 9001;

    let state = AppState::new(center_freq_hz, target_freq_hz, sideband);

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state.clone());

    println!("radio_server listening on ws://{ws_addr}/ws");

    {
        let udp_audio_target = state.udp_audio_target.clone();
        tokio::spawn(async move {
            if let Err(e) =
                run_udp_registration_listener(udp_registration_addr, udp_audio_target).await
            {
                eprintln!("UDP registration listener failed: {e}");
            }
        });
    }

    let tx = state.tx.clone();
    let audio_tx = state.audio_tx.clone();
    let waterfall_tx = state.waterfall_tx.clone();
    let radio_state = state.radio.clone();
    let udp_audio_target = state.udp_audio_target.clone();

    tokio::spawn(async move {
        let source_config = SourceConfig::WavFile {
            path: "input_iq.wav".to_string(),
        };

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
            2_800.0,
            129,
            16,
            2_700.0,
            101,
            48_000.0,
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

        let _ = tx.send(ServerMessage::UdpAudioOffer {
            server_udp_port: udp_registration_port,
        });

        let _ = tx.send(ServerMessage::Info {
            message: format!("source initialized at {} Hz", input_sample_rate_hz),
        });

        let mut waterfall = WaterfallGenerator::new(waterfall_bins);
	let mut udp_waterfall = match UdpWaterfallSender::new() {
	    Ok(s) => s,
	    Err(e) => {
		let _ = tx.send(ServerMessage::Error {
		    message: format!("UDP waterfall init failed: {e}"),
		});
		return;
	    }
	};
        let mut udp_audio = match UdpAudioSender::new(480) {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(ServerMessage::Error {
                    message: format!("UDP audio init failed: {e}"),
                });
                return;
            }
        };

        let mut wf_counter = 0usize;
        let wf_every_n_blocks = 5usize;

        let mut next_tick = Instant::now();

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

            let block_duration =
                Duration::from_secs_f64(iq_block.len() as f64 / input_sample_rate_hz as f64);

            let audio = pipeline.process_audio(&iq_block);

            // Browser path
            let audio_bytes = f32_samples_to_i16_bytes(&audio);
            let _ = audio_tx.send(audio_bytes);

            // UDP desktop path
            let audio_i16: Vec<i16> = audio
                .iter()
                .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .collect();

            if let Some(target) = *udp_audio_target.read().await {
                udp_audio.send_audio_to(target, &audio_i16);
            }

	    wf_counter += 1;
	    if wf_counter >= wf_every_n_blocks {
		wf_counter = 0;
		let row = waterfall.generate_row(&iq_block);

		// Browser path
		let _ = waterfall_tx.send(row.clone());

		// UDP desktop path
		if let Some(target) = *udp_audio_target.read().await {
		    udp_waterfall.send_row_to(target, &row);
		}
	    }

            next_tick += block_duration;
            tokio::time::sleep_until(next_tick).await;
        }
    });

    let listener = tokio::net::TcpListener::bind(ws_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
