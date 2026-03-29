use rigflow_core::radio::{HardwareKind, LeaseId, RadioCapabilities, RadioId};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientRadioMessage {
    ListRadios,
    AcquireRadio {
        radio_id: RadioId,
        center_freq_hz: u64,
        target_freq_hz: u64,
        audio_udp_peer: String,
        waterfall_udp_peer: String,
    },
    ReleaseRadio,
    RenewLease,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerRadioMessage {
    RadiosListed {
        radios: Vec<RadioInfo>,
    },
    RadioAcquired {
        radio_id: RadioId,
        lease_id: LeaseId,
        lease_ttl_ms: u64,
    },
    RadioReleased {
        radio_id: RadioId,
    },
    LeaseRenewed {
        radio_id: RadioId,
        lease_ttl_ms: u64,
    },
    RadioError {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RadioInfo {
    pub id: RadioId,
    pub display_name: String,
    pub hardware_kind: HardwareKind,
    pub index: u32,
    pub serial: Option<String>,
    pub capabilities: RadioCapabilities,
    pub state: RadioAvailability,
    pub is_leased: bool,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RadioAvailability {
    Available,
    Starting,
    Running,
    Stopping,
    Faulted,
}
