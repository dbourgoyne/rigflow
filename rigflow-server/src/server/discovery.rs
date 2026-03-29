use crate::server::radio_types::{
    HardwareKind, RadioCapabilities, RadioDescriptor, RadioId,
};

pub fn discover_radios() -> Vec<RadioDescriptor> {
    vec![RadioDescriptor {
        id: RadioId("rtl:0".to_string()),
        display_name: "RTL-SDR #0".to_string(),
        hardware_kind: HardwareKind::RtlSdr,
        index: 0,
        serial: None,
        capabilities: RadioCapabilities {
            min_freq_hz: 500_000,
            max_freq_hz: 1_700_000_000,
            max_sample_rate_hz: 2_400_000,
            supports_wfm: true,
            supports_nfm: true,
            supports_usb: true,
            supports_lsb: true,
        },
    }]
}
