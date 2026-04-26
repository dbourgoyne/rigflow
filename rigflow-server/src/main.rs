#![doc = include_str!("../README.md")]
//! # rigflow-server
//!
//! `rigflow-server` is the backend service for the rigflow SDR system.
//!
//! It discovers SDR sources, manages radio leases, runs per-radio worker
//! tasks, performs DSP processing, and exposes control/status over WebSocket.
//! Audio and waterfall/spectrum data are streamed to clients over UDP.
//!
//! ## Responsibilities
//!
//! - Discover available radio sources
//!   - RTL-SDR hardware
//!   - WAV IQ files
//!   - fake/test signal source
//! - Manage radio acquisition and lease ownership
//! - Start radio workers lazily when a client acquires a radio
//! - Route control commands to the active worker
//! - Stream audio and waterfall data over UDP
//! - Publish runtime state over WebSocket
//!
//! ## Network Interfaces
//!
//! - WebSocket control endpoint:
//!
//!   ```text
//!   ws://<server-ip>:9000/ws
//!   ```
//!
//! - UDP registration listener:
//!
//!   ```text
//!   0.0.0.0:9001
//!   ```
//!
//! Clients connect over WebSocket for control and lease management, then
//! provide UDP endpoints for audio and waterfall streaming.
//!
//! ## Protocol Model
//!
//! The server uses the shared `rigflow-protocol` crate.
//!
//! - Client → server:
//!   - `ClientRadioMessage`
//!   - examples: list radios, acquire radio, tune, change demod mode
//!
//! - Server → client:
//!   - `ServerRadioMessage`
//!   - examples: radio list, lease updates, runtime snapshots, runtime deltas
//!
//! `RuntimeSnapshot` is a full state sync.
//! `RuntimeChanged` is a sparse delta containing only changed fields.
//!
//! ## Runtime Model
//!
//! The server uses a lazy worker model:
//!
//! 1. Radios are discovered at startup.
//! 2. A client requests a radio lease.
//! 3. The `RadioManager` starts or attaches to a worker for that radio.
//! 4. The worker owns the source, DSP pipeline, and UDP streaming.
//! 5. When the lease is released or expires, the worker is shut down.
//!
//! ## Example Usage
//!
//! Start with the fake source:
//!
//! ```bash
//! cargo run -p rigflow-server -- --source fake
//! ```
//!
//! Start with RTL-SDR:
//!
//! ```bash
//! cargo run -p rigflow-server -- --source rtlsdr --rtl-device 0
//! ```
//!
//! Start with WAV IQ input:
//!
//! ```bash
//! cargo run -p rigflow-server -- --source wav --wav-file input_iq.wav
//! ```
//!
//! ## Common Options
//!
//! ```text
//! --source fake|wav|rtlsdr
//! --center HZ
//! --target HZ
//! --demod wfm|nfm|am|usb|lsb|cw
//!
//! RTL-SDR:
//! --rtl-device INDEX
//! --rtl-sample-rate HZ
//! --rtl-gain TENTHS_DB
//! --rtl-auto-gain
//! --rtl-ppm PPM
//! --rtl-direct-sampling
//!
//! WAV:
//! --wav-file PATH
//! --wav-dir PATH
//!
//! Fake:
//! --fake-sample-rate HZ
//! --fake-tone HZ
//! ```
//!
//! ## Related Crates
//!
//! - `rigflow-client` — egui desktop client
//! - `rigflow-core` — shared DSP, radio, audio, and network utilities
//! - `rigflow-protocol` — shared WebSocket protocol types

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::{routing::get, Router};
use log::{error, info};

use rigflow_server::{
    app_state::AppState,
    config::ServerConfig,
    net::websocket::ws_handler,
    radio::{
	types::RadioManagerConfig,
	discovery::{debug_print_discovered_radios, discover_radios},
	manager::RadioManager,
    },
    net::udp::udp_registration::run_udp_registration_listener,
};

/// WebSocket endpoint for rigflow control.
const WS_ADDR: &str = "0.0.0.0:9000";

/// UDP listener used by clients to register their audio destination.
const UDP_REGISTRATION_ADDR: &str = "0.0.0.0:9001";

/// Default lease-management timings.
const LEASE_TTL_SECS: u64 = 30;
const STARTUP_TIMEOUT_SECS: u64 = 5;
const SHUTDOWN_TIMEOUT_SECS: u64 = 3;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging first so startup failures are visible.
    env_logger::init();

    // Parse command-line configuration.
    let cfg = parse_config_or_exit();
    info!("rigflow_server config: {:?}", cfg);

    // Discover available radios at startup.
    let descriptors = discover_radios(&cfg);
    debug_print_discovered_radios(&descriptors);

    // Build the radio manager that owns worker lifecycle and leasing.
    let radio_manager = Arc::new(RadioManager::new(
        descriptors,
        RadioManagerConfig {
            lease_ttl: Duration::from_secs(LEASE_TTL_SECS),
            startup_timeout: Duration::from_secs(STARTUP_TIMEOUT_SECS),
            shutdown_timeout: Duration::from_secs(SHUTDOWN_TIMEOUT_SECS),
        },
        cfg.clone(),
    ));

    // Periodically expire stale leases.
    tokio::spawn(RadioManager::lease_expiry_loop(Arc::clone(&radio_manager)));

    // Build shared application state used by Axum handlers.
    let app_state = build_app_state(Arc::clone(&radio_manager));

    // Start UDP registration listener in the background.
    spawn_udp_registration_listener(&app_state);

    // Build the Axum router.
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(app_state);

    // Start the WebSocket server.
    let ws_addr: SocketAddr = WS_ADDR.parse()?;
    info!("rigflow_server listening on ws://{ws_addr}/ws");
    info!("UDP registration listener on {UDP_REGISTRATION_ADDR}");

    let listener = tokio::net::TcpListener::bind(ws_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Parse server configuration, printing a friendly error and exiting on failure.
fn parse_config_or_exit() -> ServerConfig {
    match ServerConfig::from_args() {
        Ok(cfg) => cfg,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(2);
        }
    }
}

/// Construct the shared Axum application state.
///
/// This keeps startup wiring in one place and makes `main()` easier to scan.
fn build_app_state(radio_manager: Arc<RadioManager>) -> AppState {
    AppState::new(
        radio_manager,
    )
}

/// Spawn the UDP registration listener used by clients to advertise where
/// they want audio packets delivered.
fn spawn_udp_registration_listener(state: &AppState) {
    let udp_audio_target = state.udp_audio_target.clone();

    tokio::spawn(async move {
        if let Err(err) =
            run_udp_registration_listener(UDP_REGISTRATION_ADDR, udp_audio_target).await
        {
            error!("UDP registration listener failed: {err}");
        }
    });
}
