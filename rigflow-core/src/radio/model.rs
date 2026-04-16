use serde::{Deserialize, Serialize};

/// Unique identifier for a radio instance.
///
/// This is a stable, opaque identifier used across:
/// - server ↔ client communication
/// - leasing system
///
/// Wrapped in a newtype to provide type safety vs raw `String`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RadioId(pub String);

/// Unique identifier for a lease.
///
/// A lease represents exclusive access to a radio by a client.
/// This is separate from `RadioId` to prevent accidental mixing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LeaseId(pub String);

/// Type of underlying radio hardware or source.
///
/// Serialized as snake_case for stable wire format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HardwareKind {
    /// RTL-SDR USB device
    RtlSdr,

    /// Generic SoapySDR-compatible device
    Soapy,

    /// IQ data from a WAV file (offline source)
    WavFile,

    /// Synthetic tone generator (test/debug)
    FakeTone,

    /// Unknown or unsupported hardware
    Unknown,
}

/// Capabilities of a radio device.
///
/// These are used by the client to:
/// - constrain UI controls (frequency limits, modes)
/// - enable/disable demodulation options
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RadioCapabilities {
    /// Minimum tunable frequency (Hz)
    pub min_freq_hz: u64,

    /// Maximum tunable frequency (Hz)
    pub max_freq_hz: u64,

    /// Maximum supported sample rate (Hz)
    pub max_sample_rate_hz: u32,

    /// Supported demodulation modes
    pub supports_wfm: bool,
    pub supports_nfm: bool,
    pub supports_usb: bool,
    pub supports_lsb: bool,
    pub supports_am: bool,
    pub supports_cw: bool,
}

/// Description of a radio available on the server.
///
/// This is sent to clients during discovery and used to:
/// - populate radio selection UI
/// - provide metadata about hardware and capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RadioDescriptor {
    /// Unique identifier for this radio
    pub id: RadioId,

    /// Human-readable name (for UI display)
    pub display_name: String,

    /// Underlying hardware/source type
    pub hardware_kind: HardwareKind,

    /// Index within a given hardware type (e.g., RTL device index)
    pub index: u32,

    /// Optional hardware serial number (if available)
    pub serial: Option<String>,

    /// Capabilities of this radio
    pub capabilities: RadioCapabilities,
}
