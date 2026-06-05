/// Live TX-audio diagnostics for SSB microphone transmit.
///
/// **Diagnostics only** — nothing here alters transmitted audio.  Levels are
/// measured immediately before SSB modulation (after the client-side Mic Gain
/// and the server-side DC-block / band-limit), so the operator sees the audio
/// actually feeding the modulator.
///
/// `rms` / `peak` are linear amplitudes (0..~1+, where 1.0 = digital full
/// scale).  `peak` is held briefly by the producer before it decays.
/// `clipping` is a held flag (≈1 s) that trips when a pre-modulator sample
/// reaches full scale.  `underruns` / `overruns` are monotonic transport-health
/// counters that only reset on operator request.
///
/// All fields are zero when not transmitting; the client shows the pane only
/// in USB/LSB.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct TxAudioDiag {
    /// RMS level of the audio feeding the modulator (linear, 0..~1+).
    pub rms: f32,
    /// Recent peak level with short hold (linear, 0..~1+).
    pub peak: f32,
    /// True while clipping is being held (a sample reached full scale).
    pub clipping: bool,
    /// Current TX-limiter gain reduction in dB (≥0; 0 = limiter inactive).
    pub gain_reduction_db: f32,
    /// Current speech-compressor gain reduction in dB (≥0; 0 = inactive).
    pub compressor_reduction_db: f32,
    /// Count of TX-audio underruns (modulator wanted audio, buffer empty).
    pub underruns: u64,
    /// Count of TX-audio overruns (producer outran the consumer; samples dropped).
    pub overruns: u64,
}
