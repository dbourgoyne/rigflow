use radio_server::audio_output::wav_writer::AudioWavWriter;
use radio_server::dsp::demod::Sideband;
use radio_server::dsp::pipeline::DspPipeline;
use radio_server::input::iq_wav_reader::IqWavReader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input_path = "input_iq.wav";
    let output_path = "output.wav";

    let center_freq_hz = 3_750_000.0;
    let target_freq_hz = 3_690_000.0;
    let channel_cutoff_hz = 2_800.0;
    let fir_taps = 129;
    let decimation_factor = 16;
    let audio_cutoff_hz = 0.0;
    let audio_fir_taps = 101;
    let block_size = 8192;

    let mut reader =
        IqWavReader::open(input_path).map_err(|e| format!("failed to open IQ WAV: {e}"))?;

    let input_sample_rate_hz = reader.sample_rate() as f32;
    let output_sample_rate_hz = (input_sample_rate_hz / decimation_factor as f32) as u32;

    println!(
        "Opened IQ WAV: sample_rate={} Hz, channels={}, bits={}",
        reader.sample_rate(),
        reader.channels(),
        reader.bits_per_sample()
    );

    let mut pipeline = DspPipeline::new(
        center_freq_hz,
        target_freq_hz,
        input_sample_rate_hz,
        channel_cutoff_hz,
        fir_taps,
        decimation_factor,
        audio_cutoff_hz,
        audio_fir_taps,
    );

    pipeline.set_sideband(Sideband::Lsb);

    let mut wav = AudioWavWriter::create(output_path, output_sample_rate_hz)?;

    loop {
        let iq_block = reader
            .read_block(block_size)
            .map_err(|e| format!("failed reading IQ block: {e}"))?;

        if iq_block.is_empty() {
            break;
        }

        let audio = pipeline.process_audio(&iq_block);
        wav.write_samples(&audio)?;
    }

    wav.finalize()?;
    println!("Wrote demodulated audio to {output_path}");

    Ok(())
}
