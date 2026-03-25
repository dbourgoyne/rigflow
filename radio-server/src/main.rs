use std::env;
use std::net::SocketAddr;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::thread;
use std::time::{Duration, Instant};

use axum::{routing::get, Router};
use num_complex::Complex32;

use rigflow_core::dsp::demod::{DemodMode, Sideband};
use rigflow_server::{
    api::{protocol::ServerMessage, websocket::ws_handler},
    dsp::pipeline::DspPipeline,
    server::app_state::{AppState, RadioState, StreamState},
    source::factory::{create_source, SourceConfig},
    streaming::{
        udp_audio::UdpAudioSender,
        udp_registration::run_udp_registration_listener,
        udp_waterfall::UdpWaterfallSender,
    },
    waterfall::simple::WaterfallGenerator,
};

#[derive(Debug, Clone)]
enum SourceKind {
    Fake,
    Wav,
    RtlSdr,
}

#[derive(Debug, Clone)]
struct ServerConfig {
    demod: DemodMode,
    
    source: SourceKind,

    wav_file: String,

    fake_sample_rate_hz: f32,
    fake_tone_hz: f32,

    rtlsdr_device_index: usize,
    rtlsdr_sample_rate_hz: u32,
    rtlsdr_gain_tenths_db: Option<i32>,
    rtlsdr_ppm_correction: i32,
    rtlsdr_direct_sampling: bool,

    center_freq_hz: f32,
    target_freq_hz: f32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
	    demod: DemodMode::Lsb,

            source: SourceKind::Fake,

            wav_file: "input_iq.wav".to_string(),

            fake_sample_rate_hz: 48_000.0,
            fake_tone_hz: 1_500.0,

            rtlsdr_device_index: 0,
            rtlsdr_sample_rate_hz: 2_048_000,
            rtlsdr_gain_tenths_db: None,
            rtlsdr_ppm_correction: 0,
            rtlsdr_direct_sampling: false,

            center_freq_hz: 101_100_000.0,
            target_freq_hz: 101_100_000.0,
        }
    }
}

impl ServerConfig {
    fn from_args() -> Result<Self, String> {
        let mut cfg = Self::default();
        let mut args = env::args().skip(1);

        while let Some(arg) = args.next() {
            match arg.as_str() {

		"--demod" => {
                    let value = args.next().ok_or("--demod requires a value")?;
                    cfg.demod = match value.as_str() {
                        "usb" => DemodMode::Usb,
                        "lsb" => DemodMode::Lsb,
                        "wfm" => DemodMode::Wfm,
                        _ => return Err(format!("unknown source '{value}'\n\n{}", Self::usage())),
                    };
		}
		
                "--source" => {
                    let value = args.next().ok_or("--source requires a value")?;
                    cfg.source = match value.as_str() {
                        "fake" => SourceKind::Fake,
                        "wav" => SourceKind::Wav,
                        "rtlsdr" => SourceKind::RtlSdr,
                        _ => return Err(format!("unknown source '{value}'\n\n{}", Self::usage())),
                    };
                }

                "--wav-file" => {
                    cfg.wav_file = args.next().ok_or("--wav-file requires a value")?;
                }

                "--fake-sample-rate" => {
                    cfg.fake_sample_rate_hz = args
                        .next()
                        .ok_or("--fake-sample-rate requires a value")?
                        .parse()
                        .map_err(|_| "invalid --fake-sample-rate".to_string())?;
                }

                "--fake-tone" => {
                    cfg.fake_tone_hz = args
                        .next()
                        .ok_or("--fake-tone requires a value")?
                        .parse()
                        .map_err(|_| "invalid --fake-tone".to_string())?;
                }

                "--rtl-device" => {
                    cfg.rtlsdr_device_index = args
                        .next()
                        .ok_or("--rtl-device requires a value")?
                        .parse()
                        .map_err(|_| "invalid --rtl-device".to_string())?;
                }

                "--rtl-sample-rate" => {
                    cfg.rtlsdr_sample_rate_hz = args
                        .next()
                        .ok_or("--rtl-sample-rate requires a value")?
                        .parse()
                        .map_err(|_| "invalid --rtl-sample-rate".to_string())?;
                }

                "--rtl-gain" => {
                    cfg.rtlsdr_gain_tenths_db = Some(
                        args.next()
                            .ok_or("--rtl-gain requires a value")?
                            .parse()
                            .map_err(|_| "invalid --rtl-gain".to_string())?,
                    );
                }

                "--rtl-auto-gain" => {
                    cfg.rtlsdr_gain_tenths_db = None;
                }

                "--rtl-ppm" => {
                    cfg.rtlsdr_ppm_correction = args
                        .next()
                        .ok_or("--rtl-ppm requires a value")?
                        .parse()
                        .map_err(|_| "invalid --rtl-ppm".to_string())?;
                }

                "--rtl-direct-sampling" => {
                    cfg.rtlsdr_direct_sampling = true;
                }

                "--center" => {
                    cfg.center_freq_hz = args
                        .next()
                        .ok_or("--center requires a value")?
                        .parse()
                        .map_err(|_| "invalid --center".to_string())?;
                }

                "--target" => {
                    cfg.target_freq_hz = args
                        .next()
                        .ok_or("--target requires a value")?
                        .parse()
                        .map_err(|_| "invalid --target".to_string())?;
                }

                "--help" | "-h" => {
                    return Err(Self::usage());
                }

                other => {
                    return Err(format!("unknown argument '{other}'\n\n{}", Self::usage()));
                }
            }
        }

        Ok(cfg)
    }

    fn usage() -> String {
        r#"Usage:
  rigflow_server --source fake [options]
  rigflow_server --source wav --wav-file input_iq.wav [options]
  rigflow_server --source rtlsdr [options]

Common options:
  --center HZ
  --target HZ
  --demod  "wfm|lsb|usb"

Fake source:
  --fake-sample-rate HZ
  --fake-tone HZ

WAV source:
  --wav-file PATH

RTL-SDR source:
  --rtl-device INDEX
  --rtl-sample-rate HZ
  --rtl-gain TENTHS_DB
  --rtl-auto-gain
  --rtl-ppm PPM
  --rtl-direct-sampling
"#
        .to_string()
    }
}

fn make_source_config(cfg: &ServerConfig, block_size: usize) -> SourceConfig {
    match cfg.source {
        SourceKind::Fake => SourceConfig::Fake {
            sample_rate_hz: cfg.fake_sample_rate_hz,
            tone_hz: cfg.fake_tone_hz,
        },
        SourceKind::Wav => SourceConfig::WavFile {
            path: cfg.wav_file.clone(),
        },
        SourceKind::RtlSdr => SourceConfig::RtlSdr {
            device_index: cfg.rtlsdr_device_index,
            sample_rate_hz: cfg.rtlsdr_sample_rate_hz,
            center_freq_hz: cfg.center_freq_hz as u32,
            gain_tenths_db: cfg.rtlsdr_gain_tenths_db,
            ppm_correction: cfg.rtlsdr_ppm_correction,
            direct_sampling: cfg.rtlsdr_direct_sampling,
            block_complex_samples: block_size,
        },
    }
}

fn choose_block_size(source: &SourceKind) -> usize {
    match source {
        SourceKind::Fake => 8192,
        SourceKind::Wav => 8192,
        SourceKind::RtlSdr => 65536,
    }
}

fn choose_decimation(source: &SourceKind) -> usize {
    match source {
        SourceKind::Fake => 4,
        SourceKind::Wav => 16,
        SourceKind::RtlSdr => 8,
    }
}

fn mode_to_string(mode: DemodMode) -> String {
    match mode {
        DemodMode::Usb => "usb".to_string(),
        DemodMode::Lsb => "lsb".to_string(),
        DemodMode::Wfm => "wfm".to_string(),
    }
}

#[derive(Debug, Clone, Copy)]
struct PipelineSettings {
    channel_cutoff_hz: f32,
    fir_taps: usize,
    audio_cutoff_hz: f32,
    audio_fir_taps: usize,
}

fn pipeline_settings_for_mode(mode: DemodMode) -> PipelineSettings {
    match mode {
        DemodMode::Wfm => PipelineSettings {
            channel_cutoff_hz: 100_000.0,
            fir_taps: 129,
            audio_cutoff_hz: 15_000.0,
            audio_fir_taps: 101,
        },
        DemodMode::Usb | DemodMode::Lsb => PipelineSettings {
            channel_cutoff_hz: 2_800.0,
            fir_taps: 129,
            audio_cutoff_hz: 2_700.0,
            audio_fir_taps: 101,
        },
    }
}

fn build_pipeline(
    center_freq_hz: f32,
    target_freq_hz: f32,
    input_sample_rate_hz: f32,
    decimation_factor: usize,
    mode: DemodMode,
) -> DspPipeline {
    let settings = pipeline_settings_for_mode(mode);

    let mut pipeline = DspPipeline::new(
        center_freq_hz,
        target_freq_hz,
        input_sample_rate_hz,
        settings.channel_cutoff_hz,
        settings.fir_taps,
        decimation_factor,
        settings.audio_cutoff_hz,
        settings.audio_fir_taps,
        48_000.0,
        mode,
    );

    match mode {
        DemodMode::Usb => pipeline.set_sideband(Sideband::Usb),
        DemodMode::Lsb => pipeline.set_sideband(Sideband::Lsb),
        DemodMode::Wfm => {}
    }

    pipeline
}

fn spawn_waterfall_worker(
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

fn spawn_realtime_capture_worker(
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
fn spawn_dsp_worker(
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
) {
    thread::spawn(move || {
        let decimation_factor = choose_decimation(&cfg.source);
        let input_sample_rate_hz = match cfg.source {
            SourceKind::RtlSdr => cfg.rtlsdr_sample_rate_hz as f32,
            SourceKind::Fake => cfg.fake_sample_rate_hz,
            SourceKind::Wav => 0.0,
        };

	// DWB: should this go away?
	let initial_mode = if let Ok(radio) = radio_state.try_read() {
	    radio.demod_mode
	} else {
	    DemodMode::Wfm
	};

	let mut pipeline = build_pipeline(
	    cfg.center_freq_hz,
	    cfg.target_freq_hz,
	    input_sample_rate_hz,
	    decimation_factor,
	    cfg.demod, //initial_mode,
	);
	
        if let Ok(mut s) = stream_state.try_write() {
            s.audio_sample_rate_hz = pipeline.client_output_sample_rate();
            s.audio_format = "i16".to_string();
            s.waterfall_bins = 1024;
            s.waterfall_frame_rate_hz = 10.0;
            s.center_freq_hz = cfg.center_freq_hz;
            s.input_sample_rate_hz = input_sample_rate_hz;
            s.udp_audio_port = 9001;
        }

        let _ = tx.send(ServerMessage::StreamConfig {
            audio_sample_rate_hz: pipeline.client_output_sample_rate(),
            audio_format: "i16".to_string(),
            waterfall_bins: 1024,
            waterfall_frame_rate_hz: 10.0,
            center_freq_hz: cfg.center_freq_hz,
	    target_freq_hz: cfg.target_freq_hz,
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

        let mut last_center_freq_hz = cfg.center_freq_hz;
        let mut last_target_freq_hz = cfg.target_freq_hz;
        let mut last_sideband = Sideband::Lsb;
        let mut last_demod_mode = initial_mode;
        let mut wf_counter = 0usize;

        let mut stats_start = Instant::now();
        let mut iq_samples_per_sec = 0usize;
        let mut audio_samples_per_sec = 0usize;
        let mut audio_packets_per_sec = 0usize;
        let mut blocks_per_sec = 0usize;

        while let Ok(iq_block) = iq_rx.recv() {
            if let Ok(radio) = radio_state.try_read() {
                if (radio.center_freq_hz - last_center_freq_hz).abs() > 0.5 {
                    pipeline.set_center_frequency(radio.center_freq_hz);
                    last_center_freq_hz = radio.center_freq_hz;
                }

                if (radio.target_freq_hz - last_target_freq_hz).abs() > 0.5 {
                    pipeline.set_target_frequency(radio.target_freq_hz);
                    last_target_freq_hz = radio.target_freq_hz;

                    let _ = tx.send(ServerMessage::FrequencyChanged {
                        target_freq_hz: radio.target_freq_hz,
                    });
                }

                if radio.sideband != last_sideband {
                    pipeline.set_sideband(radio.sideband);
                    last_sideband = radio.sideband;

                    let _ = tx.send(ServerMessage::SidebandChanged {
                        sideband: match radio.sideband {
                            Sideband::Usb => "usb".to_string(),
                            Sideband::Lsb => "lsb".to_string(),
                        },
                    });
                }

		if radio.demod_mode != last_demod_mode {
		    pipeline = build_pipeline(
			last_center_freq_hz,
			last_target_freq_hz,
			input_sample_rate_hz,
			decimation_factor,
			radio.demod_mode,
		    );

		    if matches!(radio.demod_mode, DemodMode::Usb | DemodMode::Lsb) {
			pipeline.set_sideband(radio.sideband);
		    }

		    last_demod_mode = radio.demod_mode;

		    let _ = tx.send(ServerMessage::DemodModeChanged {
			mode: mode_to_string(radio.demod_mode),
		    });

		    let settings = pipeline_settings_for_mode(radio.demod_mode);
		    let _ = tx.send(ServerMessage::Info {
			message: format!(
			    "rebuilt pipeline for mode={} channel_cutoff={}Hz audio_cutoff={}Hz",
			    mode_to_string(radio.demod_mode),
			    settings.channel_cutoff_hz,
			    settings.audio_cutoff_hz
			),
		    });
		}
            }

            iq_samples_per_sec += iq_block.len();
            blocks_per_sec += 1;

            let audio = pipeline.process_audio(&iq_block);

            // Browser path intentionally disabled while tuning desktop live path.
            // let audio_bytes = f32_samples_to_i16_bytes(&audio);
            // let _ = audio_tx.send(audio_bytes);

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
fn spawn_nonrealtime_worker(
    cfg: ServerConfig,
    block_size: usize,
    waterfall_every_n_blocks: usize,
    radio_state: std::sync::Arc<tokio::sync::RwLock<RadioState>>,
    stream_state: std::sync::Arc<tokio::sync::RwLock<StreamState>>,
    udp_audio_target: std::sync::Arc<tokio::sync::RwLock<Option<SocketAddr>>>,
    tx: tokio::sync::broadcast::Sender<ServerMessage>,
    _audio_tx: tokio::sync::broadcast::Sender<Vec<u8>>,
    waterfall_iq_tx: SyncSender<Vec<Complex32>>,
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

	let initial_mode = if let Ok(radio) = radio_state.try_read() {
	    radio.demod_mode
	} else {
	    DemodMode::Wfm
	};

	let mut pipeline = build_pipeline(
	    cfg.center_freq_hz,
	    cfg.target_freq_hz,
	    input_sample_rate_hz,
	    decimation_factor,
	    initial_mode,
	);
	
        if let Ok(mut s) = stream_state.try_write() {
            s.audio_sample_rate_hz = pipeline.client_output_sample_rate();
            s.audio_format = "i16".to_string();
            s.waterfall_bins = 1024;
            s.waterfall_frame_rate_hz = 10.0;
            s.center_freq_hz = cfg.center_freq_hz;
	    s.target_freq_hz = cfg.target_freq_hz;
            s.input_sample_rate_hz = input_sample_rate_hz;
            s.udp_audio_port = 9001;
        }

        let _ = tx.send(ServerMessage::StreamConfig {
            audio_sample_rate_hz: pipeline.client_output_sample_rate(),
            audio_format: "i16".to_string(),
            waterfall_bins: 1024,
            waterfall_frame_rate_hz: 10.0,
            center_freq_hz: cfg.center_freq_hz,
	    target_freq_hz: cfg.target_freq_hz,
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
        let mut last_center_freq_hz = cfg.center_freq_hz;
        let mut last_target_freq_hz = cfg.target_freq_hz;
        let mut last_sideband = Sideband::Lsb;
        let mut last_demod_mode = initial_mode;
        let mut wf_counter = 0usize;

        loop {
            if let Ok(radio) = radio_state.try_read() {
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

                if (radio.target_freq_hz - last_target_freq_hz).abs() > 0.5 {
                    pipeline.set_target_frequency(radio.target_freq_hz);
                    last_target_freq_hz = radio.target_freq_hz;

                    let _ = tx.send(ServerMessage::FrequencyChanged {
                        target_freq_hz: radio.target_freq_hz,
                    });
                }

                if radio.sideband != last_sideband {
                    pipeline.set_sideband(radio.sideband);
                    last_sideband = radio.sideband;

                    let _ = tx.send(ServerMessage::SidebandChanged {
                        sideband: match radio.sideband {
                            Sideband::Usb => "usb".to_string(),
                            Sideband::Lsb => "lsb".to_string(),
                        },
                    });
                }

		if radio.demod_mode != last_demod_mode {
		    pipeline = build_pipeline(
			last_center_freq_hz,
			last_target_freq_hz,
			input_sample_rate_hz,
			decimation_factor,
			radio.demod_mode,
		    );

		    if matches!(radio.demod_mode, DemodMode::Usb | DemodMode::Lsb) {
			pipeline.set_sideband(radio.sideband);
		    }

		    last_demod_mode = radio.demod_mode;
		    
		    let _ = tx.send(ServerMessage::DemodModeChanged {
			mode: mode_to_string(radio.demod_mode),
		    });

		    let settings = pipeline_settings_for_mode(radio.demod_mode);
		    let _ = tx.send(ServerMessage::Info {
			message: format!(
			    "rebuilt pipeline for mode={} channel_cutoff={}Hz audio_cutoff={}Hz",
			    mode_to_string(radio.demod_mode),
			    settings.channel_cutoff_hz,
			    settings.audio_cutoff_hz
			),
		    });
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = match ServerConfig::from_args() {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(2);
        }
    };

    println!("rigflow_server config: {:?}", cfg);

    let center_freq_hz = cfg.center_freq_hz;
    let target_freq_hz = cfg.target_freq_hz;
    let sideband = Sideband::Lsb;
    let demod_mode = cfg.demod;

    let block_size = choose_block_size(&cfg.source);
    let waterfall_bins = 1024;
    let ws_addr: SocketAddr = "0.0.0.0:9000".parse()?;
    let udp_registration_addr = "0.0.0.0:9001";

    let state = AppState::new(center_freq_hz, target_freq_hz, sideband, demod_mode);

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state.clone());

    println!("rigflow_server listening on ws://{ws_addr}/ws");
    println!("UDP registration listener on {}", udp_registration_addr);

    {
        let udp_audio_target = state.udp_audio_target.clone();
        tokio::spawn(async move {
            if let Err(e) =
                run_udp_registration_listener(udp_registration_addr, udp_audio_target).await
            {
                eprintln!("UDP registration listener failed: {e}");
            }
        });
    }

    let (wf_tx, wf_rx) = sync_channel::<Vec<Complex32>>(2);

    spawn_waterfall_worker(
        wf_rx,
        state.udp_audio_target.clone(),
        state.tx.clone(),
        state.waterfall_tx.clone(),
        waterfall_bins,
    );

    match cfg.source {
        SourceKind::RtlSdr => {
            let (iq_tx, iq_rx) = sync_channel::<Vec<Complex32>>(4);

            spawn_realtime_capture_worker(
                cfg.clone(),
                block_size,
                iq_tx,
                state.radio.clone(),
                state.stream.clone(),
                state.tx.clone(),
            );

            spawn_dsp_worker(
                cfg.clone(),
                block_size,
                10,
                state.radio.clone(),
                state.stream.clone(),
                state.udp_audio_target.clone(),
                state.tx.clone(),
                state.audio_tx.clone(),
                iq_rx,
                wf_tx,
            );
        }

        SourceKind::Fake | SourceKind::Wav => {
            spawn_nonrealtime_worker(
                cfg.clone(),
                block_size,
                10,
                state.radio.clone(),
                state.stream.clone(),
                state.udp_audio_target.clone(),
                state.tx.clone(),
                state.audio_tx.clone(),

                wf_tx,
            );
        }
    }

    let listener = tokio::net::TcpListener::bind(ws_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
