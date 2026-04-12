use serde::{Deserialize, Serialize};

/// General client → server control messages.
///
/// These are **stateless control commands** that modify the active radio.
/// They are separate from `ClientRadioMessage`, which manages:
/// - radio discovery
/// - leasing
///
/// These messages assume a radio is already acquired.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Set the tuned frequency (Hz)
    SetFrequency {
        target_freq_hz: f32,
    },

    /// Set the RF center frequency (Hz)
    SetCenterFrequency {
        center_freq_hz: f32,
    },

    /// Set the active sideband ("usb" or "lsb")
    SetSideband {
        sideband: String,
    },

    /// Set the demodulation mode ("wfm", "nfm", etc.)
    SetDemodMode {
        mode: String,
    },

    /// Adjust SSB pitch offset (Hz)
    SetSsbPitch {
        pitch_hz: f32,
    },
}

/// General server → client messages.
///
/// These are **global responses or errors** not tied to radio lifecycle.
/// Radio-specific messages are handled by `ServerRadioMessage`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Generic error response
    Error {
        message: String,
    },
}

/// Radio control protocol (discovery, leasing, runtime state).
pub mod radio_control;

/// Re-export commonly used radio protocol types.
///
/// This allows consumers to import from `rigflow_protocol` directly:
///
/// ```rust
/// use rigflow_protocol::{ClientRadioMessage, ServerRadioMessage};
/// ```
pub use radio_control::{
    ClientRadioMessage,
    ServerRadioMessage,
    RadioInfo,
    RadioAvailability,
};
