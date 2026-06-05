use log::{debug, error, info};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{mpsc, oneshot, watch, RwLock};
use tokio::task::JoinHandle;

use rigflow_core::radio::{LeaseId, RadioDescriptor, RadioId};

use crate::config::ServerConfig;
use crate::radio::types::{
    AcquireRadioResult, AcquireRequest, ClientId, LeaseRecord, RadioManagerConfig,
    RadioManagerError, RadioState, RadioSummary, StopReason, WorkerCommand, WorkerExit,
    WorkerStartResult, WorkerStatus,
};
use crate::radio::worker::run_radio_worker;

/// Runtime state for a radio that currently has an active worker task.
///
/// This is kept separate from the static radio descriptor so the manager can
/// cleanly represent radios that exist but are not currently running.
pub struct RadioRuntime {
    pub worker_tx: mpsc::Sender<WorkerCommand>,
    pub status_rx: watch::Receiver<WorkerStatus>,
    pub stop_tx: Option<oneshot::Sender<()>>,
    pub join_handle: JoinHandle<WorkerExit>,
    pub started_at: Instant,
}

/// Full manager-owned state for a radio.
///
/// A radio always has a descriptor, and may also have:
/// - a lease, if a client currently owns it
/// - a runtime, if its worker task is active
pub struct ManagedRadio {
    pub descriptor: RadioDescriptor,
    pub state: RadioState,
    pub lease: Option<LeaseRecord>,
    pub runtime: Option<RadioRuntime>,
}

/// Central authority for radio discovery, leasing, and worker lifecycle.
///
/// This owns the global map of radios and enforces:
/// - one active lease per radio
/// - only the lease owner may send commands
/// - worker startup/shutdown tied to acquire/release
pub struct RadioManager {
    radios: RwLock<HashMap<RadioId, ManagedRadio>>,
    config: RadioManagerConfig,
    server_cfg: ServerConfig,
}

impl RadioManager {
    pub fn new(
        descriptors: Vec<RadioDescriptor>,
        config: RadioManagerConfig,
        server_cfg: ServerConfig,
    ) -> Self {
        let radios = descriptors
            .into_iter()
            .map(|descriptor| {
                let id = descriptor.id.clone();
                let managed = ManagedRadio {
                    descriptor,
                    state: RadioState::Available,
                    lease: None,
                    runtime: None,
                };

                (id, managed)
            })
            .collect();

        Self {
            radios: RwLock::new(radios),
            config,
            server_cfg,
        }
    }

    /// Returns a cloned runtime status receiver for the current lease owner.
    ///
    /// This is used by connection/session code that wants to observe worker
    /// status transitions after a radio has been acquired.
    pub async fn subscribe_runtime_status(
        &self,
        client_id: &ClientId,
        radio_id: &RadioId,
        lease_id: &LeaseId,
    ) -> Result<watch::Receiver<WorkerStatus>, RadioManagerError> {
        let radios = self.radios.read().await;
        let radio = radios
            .get(radio_id)
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

        match radio.state {
            RadioState::Running | RadioState::Starting => {}
            _ => return Err(RadioManagerError::RadioNotRunning),
        }

        let status_rx = radio
            .runtime
            .as_ref()
            .ok_or(RadioManagerError::RadioNotRunning)?
            .status_rx
            .clone();

        Ok(status_rx)
    }

    /// Periodically scans for expired leases and releases them.
    ///
    /// The scan itself is done under a read lock; actual release work happens
    /// afterward so we do not hold the lock across async shutdown paths.
    pub async fn lease_expiry_loop(manager: Arc<RadioManager>) {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));

        loop {
            interval.tick().await;

            let expired: Vec<(RadioId, ClientId, LeaseId)> = {
                let radios = manager.radios.read().await;
                let now = Instant::now();

                radios
                    .iter()
                    .filter_map(|(radio_id, radio)| {
                        let lease = radio.lease.as_ref()?;

                        if lease.expires_at <= now {
                            Some((
                                radio_id.clone(),
                                lease.client_id.clone(),
                                lease.lease_id.clone(),
                            ))
                        } else {
                            None
                        }
                    })
                    .collect()
            };

            for (radio_id, client_id, lease_id) in expired {
                let _ = manager
                    .release_radio(&client_id, &radio_id, &lease_id, StopReason::LeaseExpired)
                    .await;
            }
        }
    }

    pub async fn list_radios(&self) -> Vec<RadioSummary> {
        let radios = self.radios.read().await;

        radios
            .values()
            .map(|radio| RadioSummary {
                descriptor: radio.descriptor.clone(),
                state: radio.state.clone(),
                is_leased: radio.lease.is_some(),
            })
            .collect()
    }

    /// Acquires a lease for a radio and starts its worker.
    ///
    /// Behavior is intentionally:
    /// 1. reserve the lease first
    /// 2. spawn the worker
    /// 3. wait for startup readiness
    /// 4. mark the radio running or clean up on failure
    pub async fn acquire_radio(
        &self,
        client_id: ClientId,
        radio_id: &RadioId,
        request: AcquireRequest,
    ) -> Result<AcquireRadioResult, RadioManagerError> {
        let lease_id = LeaseId(uuid::Uuid::new_v4().to_string());
        let now = Instant::now();
        let expires_at = now + self.config.lease_ttl;

        let descriptor = {
            let mut radios = self.radios.write().await;
            let radio = radios
                .get_mut(radio_id)
                .ok_or(RadioManagerError::RadioNotFound)?;

            if radio.lease.is_some() {
                return Err(RadioManagerError::RadioBusy);
            }

            radio.lease = Some(LeaseRecord {
                lease_id: lease_id.clone(),
                client_id: client_id.clone(),
                acquired_at: now,
                last_renewed_at: now,
                expires_at,
            });
            radio.state = RadioState::Starting;

            radio.descriptor.clone()
        };

        info!("[radio-manager] acquire requested for {}", radio_id.0);

        let (worker_tx, worker_rx) = mpsc::channel(64);
        let (status_tx, status_rx) = watch::channel(WorkerStatus::Starting);
        let (stop_tx, stop_rx) = oneshot::channel();
        let (startup_tx, startup_rx) = oneshot::channel();

        let join_handle = tokio::spawn(run_radio_worker(
            descriptor,
            request,
            self.server_cfg.clone(),
            worker_rx,
            status_tx,
            stop_rx,
            startup_tx,
        ));

        debug!("[radio-manager] worker spawned for {}", radio_id.0);

        {
            let mut radios = self.radios.write().await;
            let radio = radios
                .get_mut(radio_id)
                .ok_or_else(|| RadioManagerError::Internal("radio disappeared".to_string()))?;

            radio.runtime = Some(RadioRuntime {
                worker_tx,
                status_rx,
                stop_tx: Some(stop_tx),
                join_handle,
                started_at: Instant::now(),
            });
        }

        let startup = tokio::time::timeout(self.config.startup_timeout, startup_rx).await;

        match startup {
            Ok(Ok(WorkerStartResult::Ready(_ready))) => {
                debug!("[radio-manager] worker reported READY for {}", radio_id.0);

                let mut radios = self.radios.write().await;
                let radio = radios
                    .get_mut(radio_id)
                    .ok_or_else(|| RadioManagerError::Internal("radio disappeared".to_string()))?;

                radio.state = RadioState::Running;

                Ok(AcquireRadioResult {
                    radio_id: radio_id.clone(),
                    lease_id,
                    lease_expires_at: expires_at,
                })
            }
            Ok(Ok(WorkerStartResult::Failed(reason))) => {
                error!(
                    "[radio-manager] worker reported FAILED for {}: {}",
                    radio_id.0, reason
                );
                self.cleanup_failed_start(radio_id, reason.clone()).await;
                Err(RadioManagerError::StartupFailed(reason))
            }
            Ok(Err(_)) => {
                let reason = "worker exited before startup completed".to_string();

                info!(
                    "[radio-manager] worker exited before READY for {}",
                    radio_id.0
                );

                self.cleanup_failed_start(radio_id, reason.clone()).await;
                Err(RadioManagerError::StartupFailed(reason))
            }
            Err(_) => {
                let reason = "startup timed out".to_string();

                info!(
                    "[radio-manager] worker startup TIMED OUT for {}",
                    radio_id.0
                );

                self.cleanup_failed_start(radio_id, reason.clone()).await;
                Err(RadioManagerError::StartupTimedOut)
            }
        }
    }

    pub async fn renew_lease(
        &self,
        client_id: &ClientId,
        radio_id: &RadioId,
        lease_id: &LeaseId,
    ) -> Result<LeaseRecord, RadioManagerError> {
        let now = Instant::now();

        let mut radios = self.radios.write().await;
        let radio = radios
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

        lease.last_renewed_at = now;
        lease.expires_at = now + self.config.lease_ttl;

        info!(
            "LEASE RENEWED: client_id={:?} radio_id={:?} lease_id={:?}",
            client_id, radio_id, lease_id
        );

        Ok(lease.clone())
    }

    /// Sends a worker command after verifying lease ownership and runtime state.
    pub async fn send_command(
        &self,
        client_id: &ClientId,
        radio_id: &RadioId,
        lease_id: &LeaseId,
        cmd: WorkerCommand,
    ) -> Result<(), RadioManagerError> {
        let worker_tx = {
            let radios = self.radios.read().await;
            let radio = radios
                .get(radio_id)
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

            match radio.state {
                RadioState::Running => {}
                _ => return Err(RadioManagerError::RadioNotRunning),
            }

            radio
                .runtime
                .as_ref()
                .ok_or(RadioManagerError::RadioNotRunning)?
                .worker_tx
                .clone()
        };

        worker_tx
            .send(cmd)
            .await
            .map_err(|_| RadioManagerError::WorkerChannelClosed)
    }

    /// Releases a radio lease and shuts down the associated worker.
    pub async fn release_radio(
        &self,
        client_id: &ClientId,
        radio_id: &RadioId,
        lease_id: &LeaseId,
        reason: StopReason,
    ) -> Result<(), RadioManagerError> {
        let runtime = {
            let mut radios = self.radios.write().await;
            let radio = radios
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

            radio.state = RadioState::Stopping;

            radio
                .runtime
                .take()
                .ok_or(RadioManagerError::RadioNotRunning)?
        };

        Self::stop_runtime(runtime, reason, self.config.shutdown_timeout).await?;

        let mut radios = self.radios.write().await;
        let radio = radios
            .get_mut(radio_id)
            .ok_or(RadioManagerError::RadioNotFound)?;

        radio.lease = None;
        radio.state = RadioState::Available;

        Ok(())
    }

    /// Handles startup failure by stopping any partially started runtime and
    /// returning the radio to the available state.
    async fn cleanup_failed_start(&self, radio_id: &RadioId, reason: String) {
        let runtime = {
            let mut radios = self.radios.write().await;
            let Some(radio) = radios.get_mut(radio_id) else {
                return;
            };

            radio.state = RadioState::Faulted {
                reason: reason.clone(),
            };

            radio.runtime.take()
        };

        if let Some(runtime) = runtime {
            let _ = Self::stop_runtime(
                runtime,
                StopReason::StartupFailed,
                self.config.shutdown_timeout,
            )
            .await;
        }

        let mut radios = self.radios.write().await;
        if let Some(radio) = radios.get_mut(radio_id) {
            radio.lease = None;
            radio.runtime = None;
            radio.state = RadioState::Available;
        }
    }

    /// Stops a running worker and waits for its task to exit.
    ///
    /// The stop path uses both:
    /// - a structured worker command
    /// - a one-shot stop signal
    ///
    /// That may look redundant, but it preserves the current shutdown behavior.
    pub async fn stop_runtime(
        mut runtime: RadioRuntime,
        reason: StopReason,
        shutdown_timeout: std::time::Duration,
    ) -> Result<(), RadioManagerError> {
        let _ = runtime
            .worker_tx
            .send(WorkerCommand::Stop {
                reason: reason.clone(),
            })
            .await;

        info!(
            "[radio-manager] stopping runtime for radio, reason={:?}",
            reason
        );

        if let Some(stop_tx) = runtime.stop_tx.take() {
            let _ = stop_tx.send(());
        }

        debug!("[radio-manager] worker join completed cleanly");

        match tokio::time::timeout(shutdown_timeout, runtime.join_handle).await {
            Ok(Ok(WorkerExit::Clean { .. })) => Ok(()),
            Ok(Ok(WorkerExit::Failed { reason })) => Err(RadioManagerError::Internal(format!(
                "worker failed during stop: {reason}"
            ))),
            Ok(Err(join_err)) => Err(RadioManagerError::Internal(format!(
                "worker join error: {join_err}"
            ))),
            Err(_) => Err(RadioManagerError::ShutdownTimedOut),
        }
    }
}
