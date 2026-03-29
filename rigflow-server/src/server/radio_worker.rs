use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};

use crate::server::radio_types::{
    AcquireRequest, RadioDescriptor, StopReason, WorkerCommand, WorkerExit, WorkerReadyInfo,
    WorkerStartResult, WorkerStatus,
};

pub async fn run_radio_worker(
    descriptor: RadioDescriptor,
    request: AcquireRequest,
    mut worker_rx: mpsc::Receiver<WorkerCommand>,
    status_tx: watch::Sender<WorkerStatus>,
    mut stop_rx: oneshot::Receiver<()>,
    startup_tx: oneshot::Sender<WorkerStartResult>,
) -> WorkerExit {
    println!(
        "[radio-worker {}] starting mock worker center={} target={}",
        descriptor.id.0, request.center_freq_hz, request.target_freq_hz
    );

    tokio::time::sleep(Duration::from_millis(100)).await;

    let ready = WorkerReadyInfo {
        center_freq_hz: request.center_freq_hz,
        target_freq_hz: request.target_freq_hz,
        audio_sample_rate_hz: 48_000,
    };

    if startup_tx.send(WorkerStartResult::Ready(ready.clone())).is_err() {
        return WorkerExit::Failed {
            reason: "manager dropped startup receiver".to_string(),
        };
    }

    let _ = status_tx.send(WorkerStatus::Running {
        center_freq_hz: ready.center_freq_hz,
        target_freq_hz: ready.target_freq_hz,
    });

    loop {
        tokio::select! {
            _ = &mut stop_rx => {
                let reason = StopReason::InternalFault;
                let _ = status_tx.send(WorkerStatus::Stopping { reason: reason.clone() });
                let _ = status_tx.send(WorkerStatus::Stopped { reason: reason.clone() });
                return WorkerExit::Clean { reason };
            }

            cmd = worker_rx.recv() => {
                match cmd {
                    Some(WorkerCommand::SetTargetFrequency { hz }) => {
                        println!("[radio-worker {}] SetTargetFrequency {}", descriptor.id.0, hz);
                    }
                    Some(WorkerCommand::SetCenterFrequency { hz }) => {
                        println!("[radio-worker {}] SetCenterFrequency {}", descriptor.id.0, hz);
                    }
                    Some(WorkerCommand::Stop { reason }) => {
                        let _ = status_tx.send(WorkerStatus::Stopping { reason: reason.clone() });
                        let _ = status_tx.send(WorkerStatus::Stopped { reason: reason.clone() });
                        return WorkerExit::Clean { reason };
                    }
                    None => {
                        return WorkerExit::Failed {
                            reason: "worker command channel closed".to_string(),
                        };
                    }
                }
            }
        }
    }
}
