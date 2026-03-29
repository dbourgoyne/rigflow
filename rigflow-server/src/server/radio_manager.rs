use std::collections::HashMap;
use std::time::Instant;

use crate::server::radio_types::{
    AcquireRadioResult, AcquireRequest, ClientId, LeaseId, LeaseRecord, RadioDescriptor,
    RadioManagerConfig, RadioManagerError, RadioState, RadioSummary, StopReason, RadioId,
};

pub struct ManagedRadio {
    pub descriptor: RadioDescriptor,
    pub state: RadioState,
    pub lease: Option<LeaseRecord>,
}

pub struct RadioManager {
    radios: HashMap<RadioId, ManagedRadio>,
    config: RadioManagerConfig,
}

impl RadioManager {
    pub fn new(descriptors: Vec<RadioDescriptor>, config: RadioManagerConfig) -> Self {
        let radios = descriptors
            .into_iter()
            .map(|descriptor| {
                let id = descriptor.id.clone();
                (
                    id,
                    ManagedRadio {
                        descriptor,
                        state: RadioState::Available,
                        lease: None,
                    },
                )
            })
            .collect();

        Self { radios, config }
    }

    pub fn list_radios(&self) -> Vec<RadioSummary> {
        self.radios
            .values()
            .map(|radio| RadioSummary {
                descriptor: radio.descriptor.clone(),
                state: radio.state,
                is_leased: radio.lease.is_some(),
            })
            .collect()
    }

    pub fn acquire_radio(
        &mut self,
        client_id: ClientId,
        radio_id: &RadioId,
        _request: AcquireRequest,
    ) -> Result<AcquireRadioResult, RadioManagerError> {
        let radio = self
            .radios
            .get_mut(radio_id)
            .ok_or(RadioManagerError::RadioNotFound)?;

        if radio.lease.is_some() {
            return Err(RadioManagerError::RadioBusy);
        }

        let now = Instant::now();
        let lease_id = LeaseId(format!("lease-{}", now.elapsed().as_nanos()));
        let expires_at = now + self.config.lease_ttl;

        radio.lease = Some(LeaseRecord {
            lease_id: lease_id.clone(),
            client_id,
            acquired_at: now,
            last_renewed_at: now,
            expires_at,
        });

        radio.state = RadioState::Running;

        Ok(AcquireRadioResult {
            radio_id: radio_id.clone(),
            lease_id,
            lease_expires_at: expires_at,
        })
    }

    pub fn renew_lease(
        &mut self,
        client_id: &ClientId,
        radio_id: &RadioId,
        lease_id: &LeaseId,
    ) -> Result<LeaseRecord, RadioManagerError> {
        let radio = self
            .radios
            .get_mut(radio_id)
            .ok_or(RadioManagerError::RadioNotFound)?;

        let lease = radio
            .lease
            .as_mut()
            .ok_or(RadioManagerError::NoActiveLease)?;

        if &lease.client_id != client_id {
            return Err(RadioManagerError::NotLeaseOwner);
        }

        if &lease.lease_id != lease_id {
            return Err(RadioManagerError::InvalidLease);
        }

        let now = Instant::now();
        lease.last_renewed_at = now;
        lease.expires_at = now + self.config.lease_ttl;

        Ok(lease.clone())
    }

    pub fn release_radio(
        &mut self,
        client_id: &ClientId,
        radio_id: &RadioId,
        lease_id: &LeaseId,
        _reason: StopReason,
    ) -> Result<(), RadioManagerError> {
        let radio = self
            .radios
            .get_mut(radio_id)
            .ok_or(RadioManagerError::RadioNotFound)?;

        let lease = radio
            .lease
            .as_ref()
            .ok_or(RadioManagerError::NoActiveLease)?;

        if &lease.client_id != client_id {
            return Err(RadioManagerError::NotLeaseOwner);
        }

        if &lease.lease_id != lease_id {
            return Err(RadioManagerError::InvalidLease);
        }

        radio.lease = None;
        radio.state = RadioState::Available;

        Ok(())
    }

    pub fn expire_leases(&mut self) {
        let now = Instant::now();

        for radio in self.radios.values_mut() {
            let expired = radio
                .lease
                .as_ref()
                .map(|lease| lease.expires_at <= now)
                .unwrap_or(false);

            if expired {
                radio.lease = None;
                radio.state = RadioState::Available;
            }
        }
    }
}
