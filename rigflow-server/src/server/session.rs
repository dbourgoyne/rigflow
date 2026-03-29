use rigflow_core::radio::{LeaseId, RadioId};

use crate::server::radio_types::ClientId;

#[derive(Debug, Clone)]
pub struct AcquiredRadioSession {
    pub radio_id: RadioId,
    pub lease_id: LeaseId,
}

#[derive(Debug, Clone)]
pub struct SessionState {
    pub client_id: ClientId,
    pub acquired_radio: Option<AcquiredRadioSession>,
}

impl SessionState {
    pub fn new(client_id: ClientId) -> Self {
        Self {
            client_id,
            acquired_radio: None,
        }
    }

    pub fn has_radio(&self) -> bool {
        self.acquired_radio.is_some()
    }

    pub fn acquired_radio(&self) -> Option<&AcquiredRadioSession> {
        self.acquired_radio.as_ref()
    }

    pub fn set_acquired_radio(&mut self, radio_id: RadioId, lease_id: LeaseId) {
        self.acquired_radio = Some(AcquiredRadioSession { radio_id, lease_id });
    }

    pub fn clear_acquired_radio(&mut self) {
        self.acquired_radio = None;
    }
}
