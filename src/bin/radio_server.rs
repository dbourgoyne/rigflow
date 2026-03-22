use std::env;
use std::net::SocketAddr;
use std::time::Duration;

use axum::{routing::get, Router};
use tokio::task::LocalSet;
use tokio::time::Instant;

use radio_server::{
    api::{protocol::ServerMessage, websocket::ws_handler},
    dsp::{demod::Sideband, pipeline::DspPipeline},
    server::app_state::AppState,
    source::factory::{create_source, SourceConfig},
    streaming::{
        audio_binary::f32_samples_to_i16_bytes,
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
            source: SourceKind::Fake,

            wav_file: "input_iq.wav".to_string(),

            fake_sample_rate_hz: 48_000.0,
            fake_tone_hz: 1_500.0,

            rtlsdr_device_index: 0,
            rtlsdr_sample_rate_hz: 2_048_000,
            rtlsdr_gain_tenths_db: None,
            rtlsdr_ppm_correction: 0,
            rtlsdr_direct_sampling: false,

            center_freq_hz: 7_100_000.0,
            target_freq_hz: 7_101_500.0,
        }
    }
}

impl ServerConfig {
    fn from_args() -> Result<Self, String> {
        let mut cfg = Self::default();
        let mut args = env::args().skip(1);

        while let Some(arg) = args.next() {
            match arg.as_str() {
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
  radio_server --source fake [options]
  radio_server --source wav --wav-file input_iq.wav [options]
  radio_server --source rtlsdr [options]

Common options:
  --center HZ
  --target HZ

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

Examples:
  radio_server --source fake --center 1000000 --target 1001500
  radio_server --source wav --wav-file input_iq.wav --center 7100000 --target 7101500
  radio_server --source rtlsdr --center 7100000 --target 7101500 --rtl-sample-rate 2048000
"#
        .to_string()
    }
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

    println!("radio_server config: {:?}", cfg);

    let center_freq_hz = cfg.center_freq_hz;
    let target_freq_hz = cfg.target_freq_hz;
    let sideband = Sideband::Lsb;

    let block_size = 8192;
    let waterfall_bins = 512;
    let waterfall_frame_rate_hz = 10.0;

    let ws_addr: SocketAddr = "0.0.0.0:9000".parse()?;
    let udp_registration_addr = "0.0.0.0:9001";
    let udp_registration_port = 9001;

    let state = AppState::new(center_freq_hz, target_freq_hz, sideband);

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state.clone());

    println!("radio_server listening on ws://{ws_addr}/ws");

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

    let tx = state.tx.clone();
    let audio_tx = state.audio_tx.clone();
    let waterfall_tx = state.waterfall_tx.clone();
    let radio_state = state.radio.clone();
    let stream_state = state.stream.clone();
    let udp_audio_target = state.udp_audio_target.clone();

    let cfg_for_task = cfg.clone();
    let local = LocalSet::new();

    local.spawn_local(async move {
        let source_config = match cfg_for_task.source {
            SourceKind::Fake => SourceConfig::Fake {
                sample_rate_hz: cfg_for_task.fake_sample_rate_hz,
                tone_hz: cfg_for_task.fake_tone_hz,
            },

            SourceKind::Wav => SourceConfig::WavFile {
                path: cfg_for_task.wav_file.clone(),
            },

            SourceKind::RtlSdr => SourceConfig::RtlSdr {
                device_index: cfg_for_task.rtlsdr_device_index,
                sample_rate_hz: cfg_for_task.rtlsdr_sample_rate_hz,
                center_freq_hz: cfg_for_task.center_freq_hz as u32,
                gain_tenths_db: cfg_for_task.rtlsdr_gain_tenths_db,
                ppm_correction: cfg_for_task.rtlsdr_ppm_correction,
                direct_sampling: cfg_for_task.rtlsdr_direct_sampling,
                block_complex_samples: block_size,
            },
        };

        let mut source = match create_source(source_config) {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(ServerMessage::Error {
                    message: format!("failed to create source: {e}"),
                });
                return;
            }
        };

        let is_realtime_source = source.is_realtime();
        let input_sample_rate_hz = source.sample_rate();

	let (channel_cutoff_hz, decimation_factor, audio_cutoff_hz) = match cfg_for_task.source {
	    SourceKind::Fake => (2_800.0, 4, 2_700.0),
	    SourceKind::Wav => (2_800.0, 16, 2_700.0),
	    SourceKind::RtlSdr => (2_800.0, 16, 2_700.0),
	};

	let mut pipeline = DspPipeline::new(
	    center_freq_hz,
	    target_freq_hz,
	    input_sample_rate_hz,
	    channel_cutoff_hz,
	    129,
	    decimation_factor,
	    audio_cutoff_hz,
	    101,
	    48_000.0,
	);

        pipeline.set_sideband(sideband);

        {
            let mut s = stream_state.write().await;
            s.audio_sample_rate_hz = pipeline.client_output_sample_rate();
            s.audio_format = "i16".to_string();
            s.waterfall_bins = waterfall_bins;
            s.waterfall_frame_rate_hz = waterfall_frame_rate_hz;
            s.center_freq_hz = center_freq_hz;
            s.input_sample_rate_hz = input_sample_rate_hz;
            s.udp_audio_port = udp_registration_port;
        }

        let _ = tx.send(ServerMessage::StreamConfig {
            audio_sample_rate_hz: pipeline.client_output_sample_rate(),
            audio_format: "i16".to_string(),
            waterfall_bins,
            waterfall_frame_rate_hz,
            center_freq_hz,
            input_sample_rate_hz,
        });

        let _ = tx.send(ServerMessage::UdpAudioOffer {
            server_udp_port: udp_registration_port,
        });

        let _ = tx.send(ServerMessage::Info {
            message: format!("source initialized at {} Hz", input_sample_rate_hz),
        });

        let mut waterfall = WaterfallGenerator::new(waterfall_bins);

        let mut udp_audio = match UdpAudioSender::new(480) {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(ServerMessage::Error {
                    message: format!("UDP audio init failed: {e}"),
                });
                return;
            }
        };

        let mut udp_waterfall = match UdpWaterfallSender::new() {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(ServerMessage::Error {
                    message: format!("UDP waterfall init failed: {e}"),
                });
                return;
            }
        };

        let mut wf_counter = 0usize;
        let wf_every_n_blocks = 5usize;
        let mut next_tick = Instant::now();

        let mut last_center_freq_hz = center_freq_hz;
        let mut last_target_freq_hz = target_freq_hz;
        let mut last_sideband = sideband;

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
                    }
                }

                if (radio.target_freq_hz - last_target_freq_hz).abs() > 0.5 {
                    pipeline.set_target_frequency(radio.target_freq_hz);
                    last_target_freq_hz = radio.target_freq_hz;
                }

                if radio.sideband != last_sideband {
                    pipeline.set_sideband(radio.sideband);
                    last_sideband = radio.sideband;
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

            // Browser path
            let audio_bytes = f32_samples_to_i16_bytes(&audio);
            let _ = audio_tx.send(audio_bytes);

            // UDP desktop path
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
            if wf_counter >= wf_every_n_blocks {
                wf_counter = 0;
                let row = waterfall.generate_row(&iq_block);

                let _ = waterfall_tx.send(row.clone());

                if let Ok(target_guard) = udp_audio_target.try_read() {
                    if let Some(target) = *target_guard {
                        udp_waterfall.send_row_to(target, &row);
                    }
                }
            }

            if !is_realtime_source {
                next_tick += block_duration;
                tokio::time::sleep_until(next_tick).await;
            }
        }
    });

    local
        .run_until(async move {
            let listener = tokio::net::TcpListener::bind(ws_addr).await?;
            axum::serve(listener, app).await?;
            Ok::<(), Box<dyn std::error::Error>>(())
        })
        .await?;

    Ok(())
}
