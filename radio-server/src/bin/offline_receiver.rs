use std::env;
use std::error::Error;

use rigflow_core::dsp::demod::{DemodMode, Sideband};
use rigflow_server::audio_output::wav_writer::AudioWavWriter;
use rigflow_server::dsp::pipeline::DspPipeline;
use rigflow_server::input::iq_wav_reader::IqWavReader;

struct Config {
    input_path: String,
    output_path: String,
    center_freq_hz: f32,
    target_freq_hz: f32,
    cutoff_hz: f32,
    fir_taps: usize,
    decimation_factor: usize,
    audio_cutoff_hz: f32,
    audio_fir_taps: usize,
    sideband: Sideband,
    block_size: usize,
    client_output_sample_rate_hz: f32,
}

impl Config {
    fn from_args() -> Result<Self, String> {
        let args: Vec<String> = env::args().collect();

        if args.len() < 5 {
            return Err(format!(
                "Usage:\n  {} <input.wav> <output.wav> <center_freq_hz> <target_freq_hz> [lsb|usb] [cutoff_hz] [fir_taps] [decimation_factor] [audio_cutoff_hz] [audio_fir_taps] [block_size]\n\nExample:\n  {} input.wav output.wav 7100000 7095000 lsb 2800 129 16 2800 101 8192 48000.0",
                args[0], args[0]
            ));
        }

        let input_path = args[1].clone();
        let output_path = args[2].clone();

        let center_freq_hz: f32 = args[3]
            .parse()
            .map_err(|_| "invalid center_freq_hz".to_string())?;

        let target_freq_hz: f32 = args[4]
            .parse()
            .map_err(|_| "invalid target_freq_hz".to_string())?;

        let sideband = match args.get(5).map(|s| s.to_ascii_lowercase()) {
            Some(s) if s == "lsb" => Sideband::Lsb,
            Some(s) if s == "usb" => Sideband::Usb,
            None => Sideband::Lsb,
            Some(other) => {
                return Err(format!("invalid sideband '{other}', expected lsb or usb"));
            }
        };

        let cutoff_hz: f32 = args
            .get(6)
            .map(|s| s.parse().map_err(|_| "invalid cutoff_hz".to_string()))
            .transpose()?
            .unwrap_or(2_800.0);

        let fir_taps: usize = args
            .get(7)
            .map(|s| s.parse().map_err(|_| "invalid fir_taps".to_string()))
            .transpose()?
            .unwrap_or(129);

        let decimation_factor: usize = args
            .get(8)
            .map(|s| {
                s.parse()
                    .map_err(|_| "invalid decimation_factor".to_string())
            })
            .transpose()?
            .unwrap_or(16);

        let audio_cutoff_hz: f32 = args
            .get(9)
            .map(|s| s.parse().map_err(|_| "invalid audio_cutoff_hz".to_string()))
            .transpose()?
            .unwrap_or(2_800.0);

        let audio_fir_taps: usize = args
            .get(10)
            .map(|s| s.parse().map_err(|_| "invalid audio_fir_taps".to_string()))
            .transpose()?
            .unwrap_or(101);

        let block_size: usize = args
            .get(11)
            .map(|s| s.parse().map_err(|_| "invalid block_size".to_string()))
            .transpose()?
            .unwrap_or(8192);

	let client_output_sample_rate_hz: f32 = args
            .get(12)
            .map(|s| s.parse().map_err(|_| "invalid client output sample rate".to_string()))
            .transpose()?
            .unwrap_or(48000.0);

        Ok(Self {
            input_path,
            output_path,
            center_freq_hz,
            target_freq_hz,
            cutoff_hz,
            fir_taps,
            decimation_factor,
            audio_cutoff_hz,
            audio_fir_taps,
            sideband,
            block_size,
	    client_output_sample_rate_hz,
        })
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let config = Config::from_args().map_err(|e| format!("{e}\n"))?;

    let mut reader = IqWavReader::open(&config.input_path)
        .map_err(|e| format!("failed to open IQ WAV '{}': {e}", config.input_path))?;

    let input_sample_rate_hz = reader.sample_rate() as f32;
    let output_sample_rate_hz =
        (input_sample_rate_hz / config.decimation_factor as f32).round() as u32;

    println!("Input file:                    {}", config.input_path);
    println!("Output file:                   {}", config.output_path);
    println!("Input sample rate:             {} Hz", reader.sample_rate());
    println!("Center freq:                   {} Hz", config.center_freq_hz);
    println!("Target freq:                   {} Hz", config.target_freq_hz);
    println!("Sideband:                      {:?}", config.sideband);
    println!("Cutoff:                        {} Hz", config.cutoff_hz);
    println!("FIR taps:                      {}", config.fir_taps);
    println!("Decimation:                    {}", config.decimation_factor);
    println!("Output sample rate:            {} Hz", output_sample_rate_hz);
    println!("Block size:                    {}", config.block_size);
    println!("Audio cutoff:                  {} Hz", config.audio_cutoff_hz);
    println!("Audio FIR taps:                {}", config.audio_fir_taps);
    println!("Client output sample rate:     {} Hz", config.client_output_sample_rate_hz);

    let mut pipeline = DspPipeline::new(
        config.center_freq_hz,
        config.target_freq_hz,
        input_sample_rate_hz,
        config.cutoff_hz,
        config.fir_taps,
        config.decimation_factor,
        config.audio_cutoff_hz,
        config.audio_fir_taps,
        output_sample_rate_hz as f32,
	DemodMode::Wfm,
    );

    pipeline.set_sideband(config.sideband);

    let mut writer =
	AudioWavWriter::create(&config.output_path, config.client_output_sample_rate_hz as u32)
        .map_err(|e| format!("failed to create output WAV '{}': {e}", config.output_path))?;

    let mut total_iq_samples = 0usize;
    let mut total_audio_samples = 0usize;
    let mut blocks = 0usize;

    loop {
        let iq_block = reader
            .read_block(config.block_size)
            .map_err(|e| format!("failed reading IQ block: {e}"))?;

        if iq_block.is_empty() {
            break;
        }

        total_iq_samples += iq_block.len();

        let audio = pipeline.process_audio(&iq_block);
        total_audio_samples += audio.len();

        writer
            .write_samples(&audio)
            .map_err(|e| format!("failed writing audio samples: {e}"))?;

        blocks += 1;
    }

    writer
        .finalize()
        .map_err(|e| format!("failed finalizing output WAV: {e}"))?;

    println!("Done.");
    println!("Blocks processed:   {}", blocks);
    println!("IQ samples read:    {}", total_iq_samples);
    println!("Audio samples wrote: {}", total_audio_samples);

    Ok(())
}
