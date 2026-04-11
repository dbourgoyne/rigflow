use rigflow_core::radio::{LeaseId, RadioId};

use crate::server::radio_types::ClientId;

/// Represents a radio currently acquired by a session.
#[derive(Debug, Clone)]
pub struct AcquiredRadioSession {
    pub radio_id: RadioId,
    pub lease_id: LeaseId,
}

/// Per-WebSocket session state.
///
/// Tracks:
/// - client identity
/// - currently acquired radio (if any)
#[derive(Debug, Clone)]
pub struct SessionState {
    pub client_id: ClientId,
    acquired_radio: Option<AcquiredRadioSession>,
}

impl SessionState {
    pub fn new(client_id: ClientId) -> Self {
        Self {
            client_id,
            acquired_radio: None,
        }
    }

    /// Returns true if the session currently owns a radio.
    pub fn has_radio(&self) -> bool {
        self.acquired_radio.is_some()
    }

    /// Returns the currently acquired radio, if any.
    pub fn acquired_radio(&self) -> Option<&AcquiredRadioSession> {
        self.acquired_radio.as_ref()
    }

    /// Sets the currently acquired radio for this session.
    pub fn set_acquired_radio(&mut self, radio_id: RadioId, lease_id: LeaseId) {
        self.acquired_radio = Some(AcquiredRadioSession { radio_id, lease_id });
    }

    /// Clears the currently acquired radio.
    pub fn clear_acquired_radio(&mut self) {
        self.acquired_radio = None;
    }
}
