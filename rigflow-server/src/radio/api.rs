use std::net::SocketAddr;
use std::str::FromStr;

use rigflow_protocol::radio_control::{
    RadioAvailability, RadioInfo, ServerRadioMessage,
};

use crate::radio::types::{
    AcquireRequest, RadioManagerError, RadioState, RadioSummary,
};

/// Parse an acquire request from protocol fields.
///
/// Converts string socket addresses into `SocketAddr` and maps
/// parsing failures into protocol-level `RadioError` messages.
pub fn parse_acquire_request(
    center_freq_hz: u64,
    target_freq_hz: u64,
    audio_udp_peer: String,
    waterfall_udp_peer: String,
) -> Result<AcquireRequest, ServerRadioMessage> {
    let audio_udp_peer = parse_socket_addr(&audio_udp_peer, "audio_udp_peer")?;
    let waterfall_udp_peer = parse_socket_addr(&waterfall_udp_peer, "waterfall_udp_peer")?;

    Ok(AcquireRequest {
        center_freq_hz,
        target_freq_hz,
        audio_udp_peer,
        waterfall_udp_peer,
    })
}

/// Convert internal `RadioSummary` into protocol `RadioInfo`.
pub fn radio_summary_to_protocol(summary: RadioSummary) -> RadioInfo {
    let RadioSummary {
        descriptor,
        state,
        is_leased,
    } = summary;

    RadioInfo {
        id: descriptor.id,
        display_name: descriptor.display_name,
        hardware_kind: descriptor.hardware_kind,
        index: descriptor.index,
        serial: descriptor.serial,
        capabilities: descriptor.capabilities,
        state: radio_state_to_protocol(&state),
        is_leased,
    }
}

/// Convert internal radio state into protocol availability.
pub fn radio_state_to_protocol(state: &RadioState) -> RadioAvailability {
    match state {
        RadioState::Available => RadioAvailability::Available,
        RadioState::Starting => RadioAvailability::Starting,
        RadioState::Running => RadioAvailability::Running,
        RadioState::Stopping => RadioAvailability::Stopping,
        RadioState::Faulted { .. } => RadioAvailability::Faulted,
    }
}

/// Convert `RadioManagerError` into protocol-level error messages.
pub fn manager_error_to_protocol(err: RadioManagerError) -> ServerRadioMessage {
    use RadioManagerError::*;

    match err {
        RadioNotFound => error("radio_not_found", "radio not found"),
        RadioBusy => error("radio_busy", "radio is already leased"),
        NotLeaseOwner => error("not_lease_owner", "session does not own this lease"),
        NoActiveLease => error("no_active_lease", "session has no active radio lease"),
        InvalidLease => error("invalid_lease", "lease id is invalid"),
        RadioNotRunning => error("radio_not_running", "radio is not running"),

        StartupFailed(reason) => error_owned("startup_failed", reason),
        StartupTimedOut => error("startup_timed_out", "radio startup timed out"),
        ShutdownTimedOut => error("shutdown_timed_out", "radio shutdown timed out"),
        WorkerChannelClosed => error("worker_channel_closed", "radio worker channel closed"),

        Internal(reason) => error_owned("internal_error", reason),
    }
}

//
// ============================
// Helpers
// ============================
//

fn parse_socket_addr(value: &str, field: &str) -> Result<SocketAddr, ServerRadioMessage> {
    SocketAddr::from_str(value).map_err(|e| ServerRadioMessage::RadioError {
        code: format!("invalid_{field}"),
        message: format!("invalid {field}: {e}"),
    })
}

fn error(code: &str, message: &str) -> ServerRadioMessage {
    ServerRadioMessage::RadioError {
        code: code.to_string(),
        message: message.to_string(),
    }
}

fn error_owned(code: &str, message: String) -> ServerRadioMessage {
    ServerRadioMessage::RadioError {
        code: code.to_string(),
        message,
    }
}
