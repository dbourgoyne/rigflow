#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct RadioId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct LeaseId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HardwareKind {
    RtlSdr,
    Soapy,
    Unknown,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RadioCapabilities {
    pub min_freq_hz: u64,
    pub max_freq_hz: u64,
    pub max_sample_rate_hz: u32,
    pub supports_wfm: bool,
    pub supports_nfm: bool,
    pub supports_usb: bool,
    pub supports_lsb: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RadioDescriptor {
    pub id: RadioId,
    pub display_name: String,
    pub hardware_kind: HardwareKind,
    pub index: u32,
    pub serial: Option<String>,
    pub capabilities: RadioCapabilities,
}
