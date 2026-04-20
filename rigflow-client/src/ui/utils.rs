use std::time::{Duration, Instant};
use crate::ui::state::DebounceState;
use crate::UiState;
use rigflow_core::dsp::modes::DemodMode;

pub fn should_send_debounced(
    now: Instant,
    current_value: f32,
    debounce: &mut DebounceState,
    min_delta: f32,
    min_interval: Duration,
) -> Option<f32> {
    let rounded = current_value.round();

    let changed_enough = (rounded - debounce.last_sent_value).abs() >= min_delta;
    let interval_elapsed = now.duration_since(debounce.last_send_time) >= min_interval;

    if changed_enough && interval_elapsed {
        debounce.last_sent_value = rounded;
        debounce.last_send_time = now;
        Some(rounded)
    } else {
        None
    }
}

fn current_pitch_value(state: &UiState, mode: DemodMode) -> Option<f32> {
    match mode {
        DemodMode::Usb | DemodMode::Lsb => Some(state.ssb_pitch_hz),
        DemodMode::Cw => Some(state.cw_pitch_hz),
        _ => None,
    }
}

pub fn current_pitch_debounce_mut(
    state: &mut UiState,
    mode: DemodMode,
) -> Option<(&mut f32, &mut DebounceState)> {
    match mode {
        DemodMode::Usb | DemodMode::Lsb => {
            Some((&mut state.ssb_pitch_hz, &mut state.ssb_pitch_debounce))
        }
        DemodMode::Cw => {
            Some((&mut state.cw_pitch_hz, &mut state.cw_pitch_debounce))
        }
        _ => None,
    }
}
