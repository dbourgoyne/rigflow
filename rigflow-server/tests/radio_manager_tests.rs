use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use radio_server::server::discovery::discover_radios;
use radio_server::server::radio_manager::RadioManager;
use radio_server::server::radio_types::{
    AcquireRequest, ClientId, RadioId, RadioManagerConfig, StopReason,
};

#[test]
fn test_acquire_and_release_radio() {
    let descriptors = discover_radios();

    let mut manager = RadioManager::new(
        descriptors,
        RadioManagerConfig {
            lease_ttl: Duration::from_secs(10),
            startup_timeout: Duration::from_secs(5),
            shutdown_timeout: Duration::from_secs(3),
        },
    );

    let radio_id = RadioId("rtl:0".to_string());
    let client_id = ClientId("client-1".to_string());

    let acquire = manager
        .acquire_radio(
            client_id.clone(),
            &radio_id,
            AcquireRequest {
                center_freq_hz: 101_100_000,
                target_freq_hz: 101_100_000,
                audio_udp_peer: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9001),
                waterfall_udp_peer: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9002),
            },
        )
        .unwrap();

    assert_eq!(acquire.radio_id.0, "rtl:0");

    let radios = manager.list_radios();
    assert_eq!(radios.len(), 1);
    assert!(radios[0].is_leased);

    manager
        .release_radio(
            &client_id,
            &radio_id,
            &acquire.lease_id,
            StopReason::ClientRelease,
        )
        .unwrap();

    let radios = manager.list_radios();
    assert!(!radios[0].is_leased);
}
