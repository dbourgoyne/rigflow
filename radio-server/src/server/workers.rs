use std::net::SocketAddr;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::thread;
use std::time::{Duration, Instant};

use num_complex::Complex32;
use tokio::sync::mpsc as tokio_mpsc;

use rigflow_core::dsp::demod::{DemodMode, Sideband};
use rigflow_protocol::ServerMessage;

use crate::{
    dsp::pipeline::DspPipeline,
    server::{
        app_state::{RadioState, StreamState},
        config::{choose_decimation, make_source_config, ServerConfig, SourceKind},
        control::RadioCommand,
        pipeline_factory::{build_pipeline, mode_to_string, pipeline_settings_for_mode, sideband_to_string},
    },
    source::factory::create_source,
    streaming::{
        udp_audio::UdpAudioSender,
        udp_waterfall::UdpWaterfallSender,
    },
    waterfall::simple::WaterfallGenerator,
};

pub fn apply_radio_command_to_pipeline(
    cmd: RadioCommand,
    pipeline: &mut DspPipeline,
    radio_state: &std::sync::Arc<tokio::sync::RwLock<RadioState>>,
    stream_state: &std::sync::Arc<tokio::sync::RwLock<StreamState>>,
    tx: &tokio::sync::broadcast::Sender<ServerMessage>,
    input_sample_rate_hz: f32,
    decimation_factor: usize,
) {
    match cmd {
        RadioCommand::SetTargetFrequency(freq) => {
            pipeline.set_target_frequency(freq);

            if let Ok(mut radio) = radio_state.try_write() {
                radio.target_freq_hz = freq;
            }

            if let Ok(mut stream) = stream_state.try_write() {
                stream.target_freq_hz = freq;
            }

            let _ = tx.send(ServerMessage::FrequencyChanged {
                target_freq_hz: freq,
            });
        }

	RadioCommand::SetCenterFrequency(freq) => {
	    pipeline.set_center_frequency(freq);

	    if let Ok(mut radio) = radio_state.try_write() {
		radio.center_freq_hz = freq;
	    }
	    
	    if let Ok(mut stream) = stream_state.try_write() {
		stream.center_freq_hz = freq;
	    }

	    // Immediate UI update (control path)
	    let _ = tx.send(ServerMessage::CenterFrequencyChanged {
		center_freq_hz: freq,
	    });
	}

        RadioCommand::SetDemodMode(mode) => {
            let (center_freq_hz, target_freq_hz, sideband, ssb_pitch_hz) =
                if let Ok(mut radio) = radio_state.try_write() {
                    radio.demod_mode = mode;
                    (
                        radio.center_freq_hz,
                        radio.target_freq_hz,
                        radio.sideband,
                        radio.ssb_pitch_hz,
                    )
                } else {
                    (0.0, 0.0, Sideband::Lsb, 0.0)
                };

            *pipeline = build_pipeline(
                center_freq_hz,
                target_freq_hz,
                input_sample_rate_hz,
                decimation_factor,
                mode,
            );

            if matches!(mode, DemodMode::Usb | DemodMode::Lsb) {
                pipeline.set_sideband(sideband);
                pipeline.set_ssb_pitch_hz(ssb_pitch_hz);
            }

            let _ = tx.send(ServerMessage::DemodModeChanged {
                mode: mode_to_string(mode),
            });

            let settings = pipeline_settings_for_mode(mode);
            let _ = tx.send(ServerMessage::Info {
                message: format!(
                    "rebuilt pipeline for mode={} channel_cutoff={}Hz audio_cutoff={}Hz",
                    mode_to_string(mode),
                    settings.channel_cutoff_hz,
                    settings.audio_cutoff_hz
                ),
            });
        }

        RadioCommand::SetSideband(sideband) => {
            pipeline.set_sideband(sideband);

            if let Ok(mut radio) = radio_state.try_write() {
                radio.sideband = sideband;
            }

            let _ = tx.send(ServerMessage::SidebandChanged {
                sideband: sideband_to_string(sideband),
            });
        }

        RadioCommand::SetSsbPitch(pitch_hz) => {
            let pitch_hz = pitch_hz.clamp(-1000.0, 1000.0);

            pipeline.set_ssb_pitch_hz(pitch_hz);

            if let Ok(mut radio) = radio_state.try_write() {
                radio.ssb_pitch_hz = pitch_hz;
            }

            let _ = tx.send(ServerMessage::SsbPitchChanged { pitch_hz });
        }
    }
}

pub fn spawn_waterfall_worker(
    rx: Receiver<Vec<Complex32>>,
    udp_audio_target: std::sync::Arc<tokio::sync::RwLock<Option<SocketAddr>>>,
    tx: tokio::sync::broadcast::Sender<ServerMessage>,
    _waterfall_tx: tokio::sync::broadcast::Sender<Vec<u8>>,
    waterfall_bins: usize,
) {
    thread::spawn(move || {
        let mut waterfall = WaterfallGenerator::new(waterfall_bins);
        let mut udp_waterfall = match UdpWaterfallSender::new() {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(ServerMessage::Error {
                    message: format!("UDP waterfall init failed: {e}"),
                });
                return;
            }
        };

        while let Ok(iq_block) = rx.recv() {
            let row = waterfall.generate_row(&iq_block);

            // Browser path intentionally disabled while tuning desktop live path.
            // let _ = waterfall_tx.send(row.clone());

            if let Ok(target_guard) = udp_audio_target.try_read() {
                if let Some(target) = *target_guard {
                    udp_waterfall.send_row_to(target, &row);
                }
            }
        }
    });
}

pub fn spawn_realtime_capture_worker(
    cfg: ServerConfig,
    block_size: usize,
    iq_tx: SyncSender<Vec<Complex32>>,
    radio_state: std::sync::Arc<tokio::sync::RwLock<RadioState>>,
    stream_state: std::sync::Arc<tokio::sync::RwLock<StreamState>>,
    tx: tokio::sync::broadcast::Sender<ServerMessage>,
) {
    thread::spawn(move || {
        let source_config = make_source_config(&cfg, block_size);

        let mut source = match create_source(source_config) {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(ServerMessage::Error {
                    message: format!("failed to create source: {e}"),
                });
                return;
            }
        };

        let input_sample_rate_hz = source.sample_rate();
        let _ = tx.send(ServerMessage::Info {
            message: format!("capture source initialized at {} Hz", input_sample_rate_hz),
        });

        let mut last_center_freq_hz = cfg.center_freq_hz;
        let mut stats_start = Instant::now();
        let mut iq_samples_per_sec = 0usize;

        loop {
	    // Hardware retune synchronization path.
	    // Center frequency is updated through the command/control path first,
	    // then the source-owning capture worker observes it here and applies
	    // the actual device retune.
            if let Ok(radio) = radio_state.try_read() {
                if (radio.center_freq_hz - last_center_freq_hz).abs() > 0.5 {
                    if let Err(e) = source.set_center_frequency(radio.center_freq_hz) {
                        let _ = tx.send(ServerMessage::Error {
                            message: format!("failed to retune source: {e}"),
                        });
                    } else {
                        last_center_freq_hz = radio.center_freq_hz;

                        if let Ok(mut s) = stream_state.try_write() {
                            s.center_freq_hz = radio.center_freq_hz;
                        }

                        let _ = tx.send(ServerMessage::CenterFrequencyChanged {
                            center_freq_hz: radio.center_freq_hz,
                        });
                    }
                }
            }

            let iq_block = match source.read_block(block_size) {
                Ok(block) => block,
                Err(e) => {
                    let _ = tx.send(ServerMessage::Error {
                        message: format!("source read failed: {e}"),
                    });
                    break;
                }
            };

            if iq_block.is_empty() {
                let _ = tx.send(ServerMessage::Info {
                    message: "source reached end of stream".to_string(),
                });
                break;
            }

            iq_samples_per_sec += iq_block.len();

            if iq_tx.send(iq_block).is_err() {
                break;
            }

            if stats_start.elapsed() >= Duration::from_secs(1) {
                println!("capture stats: iq_samples/sec={}", iq_samples_per_sec);
                stats_start = Instant::now();
                iq_samples_per_sec = 0;
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_dsp_worker(
    cfg: ServerConfig,
    block_size: usize,
    waterfall_every_n_blocks: usize,
    radio_state: std::sync::Arc<tokio::sync::RwLock<RadioState>>,
    stream_state: std::sync::Arc<tokio::sync::RwLock<StreamState>>,
    udp_audio_target: std::sync::Arc<tokio::sync::RwLock<Option<SocketAddr>>>,
    tx: tokio::sync::broadcast::Sender<ServerMessage>,
    _audio_tx: tokio::sync::broadcast::Sender<Vec<u8>>,
    iq_rx: Receiver<Vec<Complex32>>,
    waterfall_iq_tx: SyncSender<Vec<Complex32>>,
    mut radio_cmd_rx: tokio_mpsc::UnboundedReceiver<RadioCommand>,
) {
    thread::spawn(move || {
        let decimation_factor = choose_decimation(&cfg.source);
        let input_sample_rate_hz = match cfg.source {
            SourceKind::RtlSdr => cfg.rtlsdr_sample_rate_hz as f32,
            SourceKind::Fake => cfg.fake_sample_rate_hz,
            SourceKind::Wav => 0.0,
        };

        let initial_radio = if let Ok(radio) = radio_state.try_read() {
            Some((
                radio.center_freq_hz,
                radio.target_freq_hz,
                radio.sideband,
                radio.demod_mode,
                radio.ssb_pitch_hz,
            ))
        } else {
            None
        };

        let (initial_center, initial_target, initial_sideband, initial_mode, initial_pitch) =
            initial_radio.unwrap_or((cfg.center_freq_hz, cfg.target_freq_hz, Sideband::Lsb, DemodMode::Wfm, 0.0));

        let mut pipeline = build_pipeline(
            initial_center,
            initial_target,
            input_sample_rate_hz,
            decimation_factor,
            initial_mode,
        );

        if matches!(initial_mode, DemodMode::Usb | DemodMode::Lsb) {
            pipeline.set_sideband(initial_sideband);
            pipeline.set_ssb_pitch_hz(initial_pitch);
        }

        if let Ok(mut s) = stream_state.try_write() {
            s.audio_sample_rate_hz = pipeline.client_output_sample_rate();
            s.audio_format = "i16".to_string();
            s.waterfall_bins = 1024;
            s.waterfall_frame_rate_hz = 10.0;
            s.center_freq_hz = initial_center;
            s.target_freq_hz = initial_target;
            s.input_sample_rate_hz = input_sample_rate_hz;
            s.udp_audio_port = 9001;
        }

        let _ = tx.send(ServerMessage::StreamConfig {
            audio_sample_rate_hz: pipeline.client_output_sample_rate(),
            audio_format: "i16".to_string(),
            waterfall_bins: 1024,
            waterfall_frame_rate_hz: 10.0,
            center_freq_hz: initial_center,
            target_freq_hz: initial_target,
            input_sample_rate_hz,
        });

        let _ = tx.send(ServerMessage::UdpAudioOffer {
            server_udp_port: 9001,
        });

        let _ = tx.send(ServerMessage::DemodModeChanged {
            mode: mode_to_string(initial_mode),
        });

        println!(
            "pipeline config: input_sample_rate_hz={} decimation_factor={} output_sample_rate_hz={} client_output_sample_rate_hz={}",
            input_sample_rate_hz,
            decimation_factor,
            pipeline.output_sample_rate(),
            pipeline.client_output_sample_rate(),
        );

        let mut udp_audio = match UdpAudioSender::new(480) {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(ServerMessage::Error {
                    message: format!("UDP audio init failed: {e}"),
                });
                return;
            }
        };

        let mut wf_counter = 0usize;

        let mut stats_start = Instant::now();
        let mut iq_samples_per_sec = 0usize;
        let mut audio_samples_per_sec = 0usize;
        let mut audio_packets_per_sec = 0usize;
        let mut blocks_per_sec = 0usize;

        while let Ok(iq_block) = iq_rx.recv() {
            loop {
                match radio_cmd_rx.try_recv() {
                    Ok(cmd) => {
                        apply_radio_command_to_pipeline(
                            cmd,
                            &mut pipeline,
                            &radio_state,
                            &stream_state,
                            &tx,
                            input_sample_rate_hz,
                            decimation_factor,
                        );
                    }
                    Err(tokio_mpsc::error::TryRecvError::Empty) => break,
                    Err(tokio_mpsc::error::TryRecvError::Disconnected) => break,
                }
            }

            iq_samples_per_sec += iq_block.len();
            blocks_per_sec += 1;

            let audio = pipeline.process_audio(&iq_block);

            let audio_i16: Vec<i16> = audio
                .iter()
                .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .collect();

            audio_samples_per_sec += audio_i16.len();
            audio_packets_per_sec += audio_i16.len() / 480;

            if let Ok(target_guard) = udp_audio_target.try_read() {
                if let Some(target) = *target_guard {
                    udp_audio.send_audio_to(target, &audio_i16);
                }
            }

            wf_counter += 1;
            if wf_counter >= waterfall_every_n_blocks {
                wf_counter = 0;
                match waterfall_iq_tx.try_send(iq_block.clone()) {
                    Ok(_) => {}
                    Err(TrySendError::Full(_)) => {}
                    Err(TrySendError::Disconnected(_)) => break,
                }
            }

            if stats_start.elapsed() >= Duration::from_secs(1) {
                let per_block = if blocks_per_sec > 0 {
                    audio_samples_per_sec as f32 / blocks_per_sec as f32
                } else {
                    0.0
                };

                println!("per-block avg audio samples = {}", per_block);
                println!(
                    "stats: iq_samples/sec={} audio_samples/sec={} audio_packets/sec={} blocks/sec={} block_size={} realtime=true",
                    iq_samples_per_sec,
                    audio_samples_per_sec,
                    audio_packets_per_sec,
                    blocks_per_sec,
                    block_size
                );

                stats_start = Instant::now();
                iq_samples_per_sec = 0;
                audio_samples_per_sec = 0;
                audio_packets_per_sec = 0;
                blocks_per_sec = 0;
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_nonrealtime_worker(
    cfg: ServerConfig,
    block_size: usize,
    waterfall_every_n_blocks: usize,
    radio_state: std::sync::Arc<tokio::sync::RwLock<RadioState>>,
    stream_state: std::sync::Arc<tokio::sync::RwLock<StreamState>>,
    udp_audio_target: std::sync::Arc<tokio::sync::RwLock<Option<SocketAddr>>>,
    tx: tokio::sync::broadcast::Sender<ServerMessage>,
    _audio_tx: tokio::sync::broadcast::Sender<Vec<u8>>,
    waterfall_iq_tx: SyncSender<Vec<Complex32>>,
    mut radio_cmd_rx: tokio_mpsc::UnboundedReceiver<RadioCommand>,
) {
    thread::spawn(move || {
        let source_config = make_source_config(&cfg, block_size);

        let mut source = match create_source(source_config) {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(ServerMessage::Error {
                    message: format!("failed to create source: {e}"),
                });
                return;
            }
        };

        let input_sample_rate_hz = source.sample_rate();
        let decimation_factor = choose_decimation(&cfg.source);

        let initial_radio = if let Ok(radio) = radio_state.try_read() {
            Some((
                radio.center_freq_hz,
                radio.target_freq_hz,
                radio.sideband,
                radio.demod_mode,
                radio.ssb_pitch_hz,
            ))
        } else {
            None
        };

        let (initial_center, initial_target, initial_sideband, initial_mode, initial_pitch) =
            initial_radio.unwrap_or((cfg.center_freq_hz, cfg.target_freq_hz, Sideband::Lsb, DemodMode::Wfm, 0.0));

        let mut pipeline = build_pipeline(
            initial_center,
            initial_target,
            input_sample_rate_hz,
            decimation_factor,
            initial_mode,
        );

        if matches!(initial_mode, DemodMode::Usb | DemodMode::Lsb) {
            pipeline.set_sideband(initial_sideband);
            pipeline.set_ssb_pitch_hz(initial_pitch);
        }

        if let Ok(mut s) = stream_state.try_write() {
            s.audio_sample_rate_hz = pipeline.client_output_sample_rate();
            s.audio_format = "i16".to_string();
            s.waterfall_bins = 1024;
            s.waterfall_frame_rate_hz = 10.0;
            s.center_freq_hz = initial_center;
            s.target_freq_hz = initial_target;
            s.input_sample_rate_hz = input_sample_rate_hz;
            s.udp_audio_port = 9001;
        }

        let _ = tx.send(ServerMessage::StreamConfig {
            audio_sample_rate_hz: pipeline.client_output_sample_rate(),
            audio_format: "i16".to_string(),
            waterfall_bins: 1024,
            waterfall_frame_rate_hz: 10.0,
            center_freq_hz: initial_center,
            target_freq_hz: initial_target,
            input_sample_rate_hz,
        });

        let _ = tx.send(ServerMessage::UdpAudioOffer {
            server_udp_port: 9001,
        });

        let _ = tx.send(ServerMessage::DemodModeChanged {
            mode: mode_to_string(initial_mode),
        });

        println!(
            "pipeline config: input_sample_rate_hz={} decimation_factor={} output_sample_rate_hz={} client_output_sample_rate_hz={}",
            input_sample_rate_hz,
            decimation_factor,
            pipeline.output_sample_rate(),
            pipeline.client_output_sample_rate(),
        );

        let mut udp_audio = match UdpAudioSender::new(480) {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(ServerMessage::Error {
                    message: format!("UDP audio init failed: {e}"),
                });
                return;
            }
        };

        let mut next_tick = Instant::now();
        let mut last_center_freq_hz = initial_center;
        let mut wf_counter = 0usize;

        loop {
            loop {
                match radio_cmd_rx.try_recv() {
                    Ok(cmd) => {
                        apply_radio_command_to_pipeline(
                            cmd,
                            &mut pipeline,
                            &radio_state,
                            &stream_state,
                            &tx,
                            input_sample_rate_hz,
                            decimation_factor,
                        );
                    }
                    Err(tokio_mpsc::error::TryRecvError::Empty) => break,
                    Err(tokio_mpsc::error::TryRecvError::Disconnected) => break,
                }
            }

            if let Ok(radio) = radio_state.try_read() {
		// Source retune synchronization path for non-realtime sources.
		// Center frequency is command-updated in RadioState first, then the
		// source-owning worker observes and applies the actual retune here.
                if (radio.center_freq_hz - last_center_freq_hz).abs() > 0.5 {
                    if let Err(e) = source.set_center_frequency(radio.center_freq_hz) {
                        let _ = tx.send(ServerMessage::Error {
                            message: format!("failed to retune source: {e}"),
                        });
                    } else {
                        pipeline.set_center_frequency(radio.center_freq_hz);
                        last_center_freq_hz = radio.center_freq_hz;

                        if let Ok(mut s) = stream_state.try_write() {
                            s.center_freq_hz = radio.center_freq_hz;
                        }

                        let _ = tx.send(ServerMessage::CenterFrequencyChanged {
                            center_freq_hz: radio.center_freq_hz,
                        });
                    }
                }
            }

            let iq_block = match source.read_block(block_size) {
                Ok(block) => block,
                Err(e) => {
                    let _ = tx.send(ServerMessage::Error {
                        message: format!("source read failed: {e}"),
                    });
                    break;
                }
            };

            if iq_block.is_empty() {
                let _ = tx.send(ServerMessage::Info {
                    message: "source reached end of stream".to_string(),
                });
                break;
            }

            let block_duration =
                Duration::from_secs_f64(iq_block.len() as f64 / input_sample_rate_hz as f64);

            let audio = pipeline.process_audio(&iq_block);

            let audio_i16: Vec<i16> = audio
                .iter()
                .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .collect();

            if let Ok(target_guard) = udp_audio_target.try_read() {
                if let Some(target) = *target_guard {
                    udp_audio.send_audio_to(target, &audio_i16);
                }
            }

            wf_counter += 1;
            if wf_counter >= waterfall_every_n_blocks {
                wf_counter = 0;
                let _ = waterfall_iq_tx.try_send(iq_block);
            }

            next_tick += block_duration;
            let now = Instant::now();
            if next_tick > now {
                thread::sleep(next_tick - now);
            }
        }
    });
}
