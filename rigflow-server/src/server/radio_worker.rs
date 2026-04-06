use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};

use rigflow_core::radio::RadioDescriptor;

use crate::dsp::pipeline::{DspPipeline, DspPipelineConfig};
use crate::server::config::{
    choose_block_size, choose_decimation, ServerConfig, SourceKind, WATERFALL_BINS,
};
use crate::server::radio_types::{
    AcquireRequest, StopReason, WorkerCommand, WorkerExit, WorkerReadyInfo,
    WorkerStartResult, WorkerStatus,
};
use crate::source::fake_iq::FakeIqSource;
use crate::source::IqSource;
use crate::streaming::udp_audio::UdpAudioSender;
use crate::streaming::udp_waterfall::UdpWaterfallSender;
use crate::waterfall::simple::WaterfallGenerator;

pub async fn run_radio_worker(
    descriptor: RadioDescriptor,
    request: AcquireRequest,
    server_cfg: ServerConfig,
    worker_rx: mpsc::Receiver<WorkerCommand>,
    status_tx: watch::Sender<WorkerStatus>,
    stop_rx: oneshot::Receiver<()>,
    startup_tx: oneshot::Sender<WorkerStartResult>,
) -> WorkerExit {
    println!(
        "[radio-worker {}] starting worker center={} target={}",
        descriptor.id.0, request.center_freq_hz, request.target_freq_hz
    );

    if descriptor.id.0.starts_with("fake:") {
        run_fake_worker(
            descriptor,
            request,
            server_cfg,
            worker_rx,
            status_tx,
            stop_rx,
            startup_tx,
        )
        .await
    } else {
        let reason = format!(
            "source for {} not implemented yet; fake is the first migrated path",
            descriptor.id.0
        );
        let _ = startup_tx.send(WorkerStartResult::Failed(reason.clone()));
        WorkerExit::Failed { reason }
    }
}

fn pipeline_cfg_for_fake(
    server_cfg: &ServerConfig,
    center_freq_hz: u64,
    target_freq_hz: u64,
    input_sample_rate_hz: f32,
) -> DspPipelineConfig {
    let (channel_cutoff_hz, audio_cutoff_hz) = match server_cfg.demod {
        rigflow_core::dsp::demod::DemodMode::Wfm => (100_000.0, 15_000.0),
        rigflow_core::dsp::demod::DemodMode::Nfm => (12_500.0, 5_000.0),
        rigflow_core::dsp::demod::DemodMode::Usb => (4_000.0, 3_000.0),
        rigflow_core::dsp::demod::DemodMode::Lsb => (4_000.0, 3_000.0),
    };

    DspPipelineConfig {
        center_freq_hz: center_freq_hz as f32,
        target_freq_hz: target_freq_hz as f32,
        input_sample_rate_hz,

        channel_cutoff_hz,
        fir_taps: 129,
        decimation_factor: choose_decimation(&SourceKind::Fake),

        audio_cutoff_hz,
        audio_fir_taps: 129,

        client_output_sample_rate_hz: 48_000.0,
        mode: server_cfg.demod,
    }
}

async fn run_fake_worker(
    descriptor: RadioDescriptor,
    request: AcquireRequest,
    server_cfg: ServerConfig,
    mut worker_rx: mpsc::Receiver<WorkerCommand>,
    status_tx: watch::Sender<WorkerStatus>,
    mut stop_rx: oneshot::Receiver<()>,
    startup_tx: oneshot::Sender<WorkerStartResult>,
) -> WorkerExit {
    if !matches!(server_cfg.source, SourceKind::Fake) {
        println!(
            "[radio-worker {}] warning: acquired fake radio while global source config is {:?}; using fake source anyway",
            descriptor.id.0,
            server_cfg.source
        );
    }

    let mut center_freq_hz: u64 = request.center_freq_hz;
    let mut target_freq_hz: u64 = request.target_freq_hz;

    let block_size = choose_block_size(&SourceKind::Fake);

    let mut source = FakeIqSource::new(server_cfg.fake_sample_rate_hz, server_cfg.fake_tone_hz);

    if let Err(reason) = source.set_center_frequency(center_freq_hz as f32) {
        let _ = startup_tx.send(WorkerStartResult::Failed(reason.clone()));
        return WorkerExit::Failed { reason };
    }

    let mut pipeline = DspPipeline::new(pipeline_cfg_for_fake(
        &server_cfg,
        center_freq_hz,
        target_freq_hz,
        source.sample_rate(),
    ));

    let mut waterfall_gen = WaterfallGenerator::new(WATERFALL_BINS);

    println!(
    "[radio-worker {}] pipeline mode={:?} input_sr={} output_sr={} client_sr={}",
    descriptor.id.0,
    server_cfg.demod,
    source.sample_rate(),
    pipeline.output_sample_rate(),
    pipeline.client_output_sample_rate(),
);

    let mut audio = match UdpAudioSender::new(480) {
        Ok(s) => s,
        Err(e) => {
            let reason = format!("failed to create UDP audio sender: {e}");
            let _ = startup_tx.send(WorkerStartResult::Failed(reason.clone()));
            return WorkerExit::Failed { reason };
        }
    };

    let mut waterfall = match UdpWaterfallSender::new() {
        Ok(s) => s,
        Err(e) => {
            let reason = format!("failed to create UDP waterfall sender: {e}");
            let _ = startup_tx.send(WorkerStartResult::Failed(reason.clone()));
            return WorkerExit::Failed { reason };
        }
    };

    let audio_target = request.audio_udp_peer;
    let wf_target = request.waterfall_udp_peer;

    let ready = WorkerReadyInfo {
        center_freq_hz,
        target_freq_hz,
        audio_sample_rate_hz: 48_000,
    };

    if startup_tx.send(WorkerStartResult::Ready(ready.clone())).is_err() {
        return WorkerExit::Failed {
            reason: "manager dropped startup receiver".to_string(),
        };
    }

    let _ = status_tx.send(WorkerStatus::Running {
        center_freq_hz,
        target_freq_hz,
    });

    let block_period =
        Duration::from_secs_f32((block_size as f32 / source.sample_rate()).max(0.001));
    let mut source_tick = tokio::time::interval(block_period);
    let mut blocks_read: u64 = 0;

    println!(
        "[radio-worker {}] fake source running: sample_rate={} block_size={} audio_peer={} waterfall_peer={}",
        descriptor.id.0,
        source.sample_rate(),
        block_size,
        audio_target,
        wf_target,
    );

    loop {
        tokio::select! {
            _ = &mut stop_rx => {
                let reason = StopReason::InternalFault;
                let _ = status_tx.send(WorkerStatus::Stopping { reason: reason.clone() });
                let _ = status_tx.send(WorkerStatus::Stopped { reason: reason.clone() });
                return WorkerExit::Clean { reason };
            }

            _ = source_tick.tick() => {
                let iq = match source.read_block(block_size) {
                    Ok(v) => v,
                    Err(reason) => return WorkerExit::Failed { reason },
                };

                if iq.is_empty() {
                    continue;
                }

                blocks_read += 1;

                if blocks_read % 20 == 0 {
                    println!(
                        "[radio-worker {}] fake source alive: blocks={} iq_samples={} center={} target={}",
                        descriptor.id.0,
                        blocks_read,
                        iq.len(),
                        center_freq_hz,
                        target_freq_hz,
                    );
                }

                // Waterfall from tuned/channelized IQ.
                let wf_iq = pipeline.process_iq(&iq);
                let row = waterfall_gen.generate_row(&wf_iq);
                if !row.is_empty() {
                    waterfall.send_row_to(wf_target, &row);
                }

                // Audio from the full DSP pipeline.
                let audio_f32 = pipeline.process_audio(&iq);

                let mut audio_i16 = Vec::with_capacity(audio_f32.len());
                for s in audio_f32 {
                    let v = (s * 32767.0).clamp(-32768.0, 32767.0) as i16;
                    audio_i16.push(v);
                }

                if !audio_i16.is_empty() {
                    audio.send_audio_to(audio_target, &audio_i16);
                }
            }

            cmd = worker_rx.recv() => {
                match cmd {
                    Some(WorkerCommand::SetTargetFrequency { hz }) => {
                        target_freq_hz = hz;
                        pipeline.set_target_frequency(target_freq_hz as f32);

                        println!(
                            "[radio-worker {}] SetTargetFrequency {}",
                            descriptor.id.0,
                            target_freq_hz
                        );

                        let _ = status_tx.send(WorkerStatus::Running {
                            center_freq_hz,
                            target_freq_hz,
                        });
                    }

                    Some(WorkerCommand::SetCenterFrequency { hz }) => {
                        center_freq_hz = hz;
                        pipeline.set_center_frequency(center_freq_hz as f32);

                        println!(
                            "[radio-worker {}] SetCenterFrequency {}",
                            descriptor.id.0,
                            center_freq_hz
                        );

                        if let Err(reason) = source.set_center_frequency(center_freq_hz as f32) {
                            return WorkerExit::Failed { reason };
                        }

                        let _ = status_tx.send(WorkerStatus::Running {
                            center_freq_hz,
                            target_freq_hz,
                        });
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
