use std::{net::SocketAddr, time::Duration};

use axum::{routing::get, Router};
use num_complex::Complex32;

use radio_server::{
    api::{protocol::ServerMessage, websocket::ws_handler},
    dsp::{demod::Sideband, pipeline::DspPipeline},
    server::app_state::AppState,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let center_freq_hz = 14_100_000.0;
    let target_freq_hz = 14_074_000.0;
    let sideband = Sideband::Lsb;

    let state = AppState::new(center_freq_hz, target_freq_hz, sideband);

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state.clone());

    let addr: SocketAddr = "127.0.0.1:9000".parse()?;
    println!("radio_server listening on ws://{addr}/ws");

    let tx = state.tx.clone();
    let radio_state = state.radio.clone();

    tokio::spawn(async move {
        let input_sample_rate_hz = 48_000.0;
        let mut pipeline = DspPipeline::new(
            center_freq_hz,
            target_freq_hz,
            input_sample_rate_hz,
            2_800.0,
            129,
            4,
            2_700.0,
            101,
        );

        pipeline.set_sideband(sideband);

        loop {
            // Apply any state changes from clients.
            {
                let radio = radio_state.read().await;
		pipeline.set_center_frequency(radio.center_freq_hz);
		pipeline.set_target_frequency(radio.target_freq_hz);
                pipeline.set_sideband(radio.sideband);
                // Next improvement: add set_target_frequency() to DspPipeline / tuner and apply here.
            }

            // Fake IQ source for now.
            let iq_block: Vec<Complex32> = (0..1024)
                .map(|i| {
                    let x = i as f32 * 0.01;
                    Complex32::new(x.sin(), x.cos())
                })
                .collect();

            let audio = pipeline.process_audio(&iq_block);
            let _ = tx.send(ServerMessage::AudioFrame { samples: audio });

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
