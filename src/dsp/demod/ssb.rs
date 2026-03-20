use num_complex::Complex32;

use crate::dsp::demod::Sideband;

/// Very simple first-pass SSB demodulator.
///
/// Assumes the desired SSB signal has already been:
/// - tuned close to baseband
/// - low-pass filtered
/// - optionally decimated
///
/// This version extracts audio from the complex baseband stream.
/// It is intentionally simple and good for getting the pipeline working.
///
/// Later improvements can add:
/// - AGC
/// - DC blocking
/// - audio low-pass filtering
/// - better sideband image rejection
pub struct SsbDemodulator {
    sideband: Sideband,
    audio_gain: f32,
}

impl SsbDemodulator {
    pub fn new(sideband: Sideband) -> Self {
        Self {
            sideband,
            audio_gain: 1.0,
        }
    }

    pub fn with_gain(sideband: Sideband, audio_gain: f32) -> Self {
        Self {
            sideband,
            audio_gain,
        }
    }

    pub fn set_sideband(&mut self, sideband: Sideband) {
        self.sideband = sideband;
    }

    pub fn set_gain(&mut self, gain: f32) {
        self.audio_gain = gain;
    }

    pub fn process(&mut self, input: &[Complex32]) -> Vec<f32> {
        let mut output = Vec::with_capacity(input.len());

        for &sample in input {
            let audio = match self.sideband {
                Sideband::Usb => sample.re,
                Sideband::Lsb => -sample.re,
            };

            output.push(audio * self.audio_gain);
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_complex::Complex32;

    #[test]
    fn preserves_length() {
        let mut demod = SsbDemodulator::new(Sideband::Usb);
        let input = vec![Complex32::new(1.0, 0.0); 128];
        let output = demod.process(&input);

        assert_eq!(output.len(), input.len());
    }

    #[test]
    fn usb_uses_real_component() {
        let mut demod = SsbDemodulator::new(Sideband::Usb);

        let input = vec![
            Complex32::new(1.0, 2.0),
            Complex32::new(-0.5, 7.0),
            Complex32::new(0.25, -3.0),
        ];

        let output = demod.process(&input);

        assert_eq!(output, vec![1.0, -0.5, 0.25]);
    }

    #[test]
    fn lsb_inverts_real_component() {
        let mut demod = SsbDemodulator::new(Sideband::Lsb);

        let input = vec![
            Complex32::new(1.0, 2.0),
            Complex32::new(-0.5, 7.0),
            Complex32::new(0.25, -3.0),
        ];

        let output = demod.process(&input);

        assert_eq!(output, vec![-1.0, 0.5, -0.25]);
    }
}
