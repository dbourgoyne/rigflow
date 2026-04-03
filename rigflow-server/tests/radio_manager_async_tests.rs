use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use rigflow_core::radio::{RadioId};
use rigflow_server::server::discovery::discover_radios;
use rigflow_server::server::radio_manager::{lease_expiry_loop, RadioManager};
use rigflow_server::server::radio_types::{
    AcquireRequest, ClientId, RadioManagerConfig, RadioManagerError,
};
use rigflow_server::server::config::ServerConfig;

fn acquire_request() -> AcquireRequest {
    AcquireRequest {
        center_freq_hz: 101_100_000,
        target_freq_hz: 101_100_000,
        audio_udp_peer: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9001),
        waterfall_udp_peer: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9002),
    }
}

/*
#[tokio::test]
async fn acquire_and_release_radio_starts_and_stops_worker() {
    let cfg = ServerConfig::default();
    let manager = Arc::new(RadioManager::new(
        discover_radios(&cfg),
        RadioManagerConfig {
            lease_ttl: Duration::from_secs(10),
            startup_timeout: Duration::from_secs(2),
            shutdown_timeout: Duration::from_secs(2),
        },
    ));

    let radio_id = RadioId("rtl:0".to_string());
    let client_id = ClientId("client-1".to_string());

    let acquired = manager
        .acquire_radio(client_id.clone(), &radio_id, acquire_request())
        .await
        .unwrap();

    let radios = manager.list_radios().await;
    assert_eq!(radios.len(), 1);
    assert!(radios[0].is_leased);

    manager
        .send_command(
            &client_id,
            &radio_id,
            &acquired.lease_id,
            WorkerCommand::SetTargetFrequency { hz: 101_700_000 },
        )
        .await
        .unwrap();

    manager
        .release_radio(
            &client_id,
            &radio_id,
            &acquired.lease_id,
            StopReason::ClientRelease,
        )
        .await
        .unwrap();

    let radios = manager.list_radios().await;
    assert!(!radios[0].is_leased);
}
 */

#[tokio::test]
async fn second_acquire_fails_while_radio_is_leased() {
    let cfg = ServerConfig::default();
    let manager = Arc::new(RadioManager::new(
        discover_radios(&cfg),
        RadioManagerConfig {
            lease_ttl: Duration::from_secs(10),
            startup_timeout: Duration::from_secs(2),
            shutdown_timeout: Duration::from_secs(2),
        },
    ));

    let radio_id = RadioId("rtl:0".to_string());

    let client1 = ClientId("client-1".to_string());
    let client2 = ClientId("client-2".to_string());

    let _acquired = manager
        .acquire_radio(client1.clone(), &radio_id, acquire_request())
        .await
        .unwrap();

    let err = manager
        .acquire_radio(client2.clone(), &radio_id, acquire_request())
        .await
        .unwrap_err();

    assert!(matches!(err, RadioManagerError::RadioBusy));
}

#[tokio::test]
async fn lease_expiry_releases_radio() {
    let cfg = ServerConfig::default();
    let manager = Arc::new(RadioManager::new(
        discover_radios(&cfg),
        RadioManagerConfig {
            lease_ttl: Duration::from_millis(300),
            startup_timeout: Duration::from_secs(2),
            shutdown_timeout: Duration::from_secs(2),
        },
    ));

    tokio::spawn(lease_expiry_loop(manager.clone()));

    let radio_id = RadioId("rtl:0".to_string());
    let client_id = ClientId("client-1".to_string());

    let _acquired = manager
        .acquire_radio(client_id, &radio_id, acquire_request())
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(1500)).await;

    let radios = manager.list_radios().await;
    assert!(!radios[0].is_leased);
}
