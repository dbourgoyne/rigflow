use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use rigflow_core::radio::RadioId;
use rigflow_server::config::ServerConfig;
use rigflow_server::radio::discovery::discover_radios;
use rigflow_server::radio::manager::RadioManager;
use rigflow_server::radio::types::{
    AcquireRequest, ClientId, RadioManagerConfig, RadioManagerError, StopReason,
};

/// The fake tone radio is always discovered and opens without any hardware, so
/// it is the radio these integration tests acquire (they spawn a real worker).
fn fake_tone_id() -> RadioId {
    RadioId("fake:tone".to_string())
}

fn acquire_request() -> AcquireRequest {
    AcquireRequest {
        center_freq_hz: 101_100_000,
        target_freq_hz: 101_100_000,
        audio_udp_peer: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9001),
        waterfall_udp_peer: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9002),
    }
}

fn build_manager(lease_ttl: Duration) -> Arc<RadioManager> {
    let cfg = ServerConfig::default();
    Arc::new(RadioManager::new(
        discover_radios(&cfg),
        RadioManagerConfig {
            lease_ttl,
            startup_timeout: Duration::from_secs(2),
            shutdown_timeout: Duration::from_secs(2),
        },
        cfg,
    ))
}

/// Look up the fake-tone radio's `is_leased` flag by id (not by position — a
/// stray WAV in the test's working directory would otherwise shift indices).
async fn fake_tone_is_leased(manager: &RadioManager) -> bool {
    manager
        .list_radios()
        .await
        .into_iter()
        .find(|r| r.descriptor.id == fake_tone_id())
        .expect("fake:tone radio should always be discovered")
        .is_leased
}

#[tokio::test]
async fn second_acquire_fails_while_radio_is_leased() {
    let manager = build_manager(Duration::from_secs(10));
    let radio_id = fake_tone_id();

    let client1 = ClientId("client-1".to_string());
    let client2 = ClientId("client-2".to_string());

    let acquired = manager
        .acquire_radio(client1.clone(), &radio_id, acquire_request())
        .await
        .unwrap();

    let err = manager
        .acquire_radio(client2.clone(), &radio_id, acquire_request())
        .await
        .unwrap_err();

    assert!(matches!(err, RadioManagerError::RadioBusy));

    // Release the first lease so the worker shuts down cleanly.
    manager
        .release_radio(
            &client1,
            &radio_id,
            &acquired.lease_id,
            StopReason::ClientRelease,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn lease_expiry_releases_radio() {
    let manager = build_manager(Duration::from_millis(300));

    tokio::spawn(RadioManager::lease_expiry_loop(manager.clone()));

    let radio_id = fake_tone_id();
    let client_id = ClientId("client-1".to_string());

    let _acquired = manager
        .acquire_radio(client_id, &radio_id, acquire_request())
        .await
        .unwrap();

    assert!(fake_tone_is_leased(&manager).await);

    // Wait well past the 300 ms lease TTL for the expiry loop to reclaim it.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    assert!(!fake_tone_is_leased(&manager).await);
}
