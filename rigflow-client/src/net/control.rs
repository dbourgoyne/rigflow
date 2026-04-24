use rigflow_protocol::ClientRadioMessage;

/// Commands sent from the UI layer → WebSocket control task.
///
/// This enum acts as the boundary between:
/// - synchronous UI (egui thread)
/// - async networking (Tokio WebSocket task)
///
/// Design notes:
/// - High-level actions (Connect, AcquireRadio, etc.) are represented directly
/// - Lower-level protocol messages are wrapped in `LegacyClientMessage`
///
/// Over time, the goal is likely to eliminate `LegacyClientMessage`
/// and move fully to structured command variants.
#[derive(Debug, Clone)]
pub enum ControlCommand {
    /// Establish a WebSocket connection to the server.
    Connect {
        server_ip: String,
    },

    /// Disconnect from the current server.
    Disconnect,

    /// Request to acquire a specific radio by ID.
    ///
    /// The WebSocket task will translate this into the appropriate
    /// protocol message (`ClientRadioMessage::AcquireRadio`).
    AcquireRadio {
        radio_id: String,
    },

    /// Release the currently held radio.
    ReleaseRadio,

    RadioMessage(ClientRadioMessage),
}
