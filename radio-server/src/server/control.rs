use crate::dsp::demod::{DemodMode, Sideband};

#[derive(Debug, Clone)]
pub enum RadioCommand {
    SetTargetFrequency(f32),
    SetCenterFrequency(f32),
    SetDemodMode(DemodMode),
    SetSideband(Sideband),
    SetSsbPitch(f32),
}
