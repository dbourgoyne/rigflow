use std::net::SocketAddr;
use std::str::FromStr;

use rigflow_protocol::radio_control::{
    RadioAvailability, RadioInfo, ServerRadioMessage,
};

use crate::server::radio_types::{
    AcquireRequest, RadioManagerError, RadioState, RadioSummary,
};

pub fn parse_acquire_request(
    center_freq_hz: u64,
    target_freq_hz: u64,
    audio_udp_peer: String,
    waterfall_udp_peer: String,
) -> Result<AcquireRequest, ServerRadioMessage> {
    let audio_udp_peer = SocketAddr::from_str(&audio_udp_peer).map_err(|e| {
        ServerRadioMessage::RadioError {
            code: "invalid_audio_udp_peer".to_string(),
            message: format!("invalid audio_udp_peer: {e}"),
        }
    })?;

    let waterfall_udp_peer = SocketAddr::from_str(&waterfall_udp_peer).map_err(|e| {
        ServerRadioMessage::RadioError {
            code: "invalid_waterfall_udp_peer".to_string(),
            message: format!("invalid waterfall_udp_peer: {e}"),
        }
    })?;

    Ok(AcquireRequest {
        center_freq_hz,
        target_freq_hz,
        audio_udp_peer,
        waterfall_udp_peer,
    })
}

pub fn radio_summary_to_protocol(summary: RadioSummary) -> RadioInfo {
    RadioInfo {
        id: summary.descriptor.id,
        display_name: summary.descriptor.display_name,
        hardware_kind: summary.descriptor.hardware_kind,
        index: summary.descriptor.index,
        serial: summary.descriptor.serial,
        capabilities: summary.descriptor.capabilities,
        state: radio_state_to_protocol(&summary.state),
        is_leased: summary.is_leased,
    }
}

pub fn radio_state_to_protocol(state: &RadioState) -> RadioAvailability {
    match state {
        RadioState::Available => RadioAvailability::Available,
        RadioState::Starting => RadioAvailability::Starting,
        RadioState::Running => RadioAvailability::Running,
        RadioState::Stopping => RadioAvailability::Stopping,
        RadioState::Faulted { .. } => RadioAvailability::Faulted,
    }
}

pub fn manager_error_to_protocol(err: RadioManagerError) -> ServerRadioMessage {
    match err {
        RadioManagerError::RadioNotFound => ServerRadioMessage::RadioError {
            code: "radio_not_found".to_string(),
            message: "radio not found".to_string(),
        },
        RadioManagerError::RadioBusy => ServerRadioMessage::RadioError {
            code: "radio_busy".to_string(),
            message: "radio is already leased".to_string(),
        },
        RadioManagerError::NotLeaseOwner => ServerRadioMessage::RadioError {
            code: "not_lease_owner".to_string(),
            message: "session does not own this lease".to_string(),
        },
        RadioManagerError::NoActiveLease => ServerRadioMessage::RadioError {
            code: "no_active_lease".to_string(),
            message: "session has no active radio lease".to_string(),
        },
        RadioManagerError::InvalidLease => ServerRadioMessage::RadioError {
            code: "invalid_lease".to_string(),
            message: "lease id is invalid".to_string(),
        },
        RadioManagerError::RadioNotRunning => ServerRadioMessage::RadioError {
            code: "radio_not_running".to_string(),
            message: "radio is not running".to_string(),
        },
        RadioManagerError::StartupFailed(reason) => ServerRadioMessage::RadioError {
            code: "startup_failed".to_string(),
            message: reason,
        },
        RadioManagerError::StartupTimedOut => ServerRadioMessage::RadioError {
            code: "startup_timed_out".to_string(),
            message: "radio startup timed out".to_string(),
        },
        RadioManagerError::ShutdownTimedOut => ServerRadioMessage::RadioError {
            code: "shutdown_timed_out".to_string(),
            message: "radio shutdown timed out".to_string(),
        },
        RadioManagerError::WorkerChannelClosed => ServerRadioMessage::RadioError {
            code: "worker_channel_closed".to_string(),
            message: "radio worker channel closed".to_string(),
        },
        RadioManagerError::Internal(reason) => ServerRadioMessage::RadioError {
            code: "internal_error".to_string(),
            message: reason,
        },
    }
}
