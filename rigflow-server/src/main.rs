use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::{routing::get, Router};
use log::info;
use tokio::sync::mpsc as tokio_mpsc;

use rigflow_core::dsp::demod::Sideband;
use rigflow_server::{
    api::websocket::ws_handler,
    server::{
        app_state::AppState,
        config::ServerConfig,
        control::RadioCommand,
    },
    streaming::udp_registration::run_udp_registration_listener,
};

use rigflow_server::server::discovery::{debug_print_discovered_radios, discover_radios};
use rigflow_server::server::radio_manager::RadioManager;
use rigflow_server::server::radio_types::RadioManagerConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let cfg = match ServerConfig::from_args() {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(2);
        }
    };

    info!("rigflow_server config: {:?}", cfg);

    let center_freq_hz = cfg.center_freq_hz;
    let target_freq_hz = cfg.target_freq_hz;
    let sideband = Sideband::Lsb;
    let demod_mode = cfg.demod;
    let pitch_hz = 0.0;

    let ws_addr: SocketAddr = "0.0.0.0:9000".parse()?;
    let udp_registration_addr = "0.0.0.0:9001";

    let (radio_cmd_tx, _radio_cmd_rx) = tokio_mpsc::unbounded_channel::<RadioCommand>();

    let descriptors = discover_radios(&cfg);
    debug_print_discovered_radios(&descriptors);

    let radio_manager = Arc::new(RadioManager::new(
        descriptors,
        RadioManagerConfig {
            lease_ttl: Duration::from_secs(30),
            startup_timeout: Duration::from_secs(5),
            shutdown_timeout: Duration::from_secs(3),
        },
	cfg.clone(),
    ));

    tokio::spawn(RadioManager::lease_expiry_loop(radio_manager.clone()));

    let state = AppState::new(
        center_freq_hz,
        target_freq_hz,
        sideband,
        demod_mode,
        pitch_hz,
        radio_cmd_tx,
        radio_manager.clone(),
    );

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state.clone());

    info!("rigflow_server listening on ws://{ws_addr}/ws");
    info!("UDP registration listener on {}", udp_registration_addr);

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

    let listener = tokio::net::TcpListener::bind(ws_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
