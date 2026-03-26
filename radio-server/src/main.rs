use std::net::SocketAddr;
use std::sync::mpsc::sync_channel;

use axum::{routing::get, Router};
use num_complex::Complex32;
use tokio::sync::mpsc as tokio_mpsc;

use rigflow_core::dsp::demod::Sideband;
use rigflow_server::{
    api::websocket::ws_handler,
    server::{
        app_state::AppState,
        control::RadioCommand,
    },
    streaming::udp_registration::run_udp_registration_listener,
};
use rigflow_server::server::config::{
    choose_block_size,
    ServerConfig,
    SourceKind,
};
use rigflow_server::server::workers::{
    spawn_dsp_worker,
    spawn_nonrealtime_worker,
    spawn_realtime_capture_worker,
    spawn_waterfall_worker,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = match ServerConfig::from_args() {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(2);
        }
    };

    println!("rigflow_server config: {:?}", cfg);

    let center_freq_hz = cfg.center_freq_hz;
    let target_freq_hz = cfg.target_freq_hz;
    let sideband = Sideband::Lsb;
    let demod_mode = cfg.demod;
    let pitch_hz = 0.0;

    let block_size = choose_block_size(&cfg.source);
    let waterfall_bins = 1024;
    let ws_addr: SocketAddr = "0.0.0.0:9000".parse()?;
    let udp_registration_addr = "0.0.0.0:9001";

    let (radio_cmd_tx, radio_cmd_rx) = tokio_mpsc::unbounded_channel::<RadioCommand>();

    let state = AppState::new(
        center_freq_hz,
        target_freq_hz,
        sideband,
        demod_mode,
        pitch_hz,
        radio_cmd_tx,
    );

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state.clone());

    println!("rigflow_server listening on ws://{ws_addr}/ws");
    println!("UDP registration listener on {}", udp_registration_addr);

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

    let (wf_tx, wf_rx) = sync_channel::<Vec<Complex32>>(2);

    spawn_waterfall_worker(
        wf_rx,
        state.udp_audio_target.clone(),
        state.tx.clone(),
        state.waterfall_tx.clone(),
        waterfall_bins,
    );

    match cfg.source {
        SourceKind::RtlSdr => {
            let (iq_tx, iq_rx) = sync_channel::<Vec<Complex32>>(4);

            spawn_realtime_capture_worker(
                cfg.clone(),
                block_size,
                iq_tx,
                state.radio.clone(),
                state.stream.clone(),
                state.tx.clone(),
            );

            spawn_dsp_worker(
                cfg.clone(),
                block_size,
                10,
                state.radio.clone(),
                state.stream.clone(),
                state.udp_audio_target.clone(),
                state.tx.clone(),
                state.audio_tx.clone(),
                iq_rx,
                wf_tx,
                radio_cmd_rx,
            );
        }

        SourceKind::Fake | SourceKind::Wav => {
            spawn_nonrealtime_worker(
                cfg.clone(),
                block_size,
                10,
                state.radio.clone(),
                state.stream.clone(),
                state.udp_audio_target.clone(),
                state.tx.clone(),
                state.audio_tx.clone(),
                wf_tx,
                radio_cmd_rx,
            );
        }
    }

    let listener = tokio::net::TcpListener::bind(ws_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
