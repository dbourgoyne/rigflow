use minifb::{Key, KeyRepeat, Window};

use crate::app::state::UiState;

#[derive(Debug, Clone)]
pub enum UiAction {
    SetTargetFrequency(f32),
    SetCenterFrequency(f32),
    SetDemodMode(&'static str),
    SetSideband(&'static str),
    SetSsbPitch(f32),
    Ping,
}

const DEFAULT_TUNE_STEP_HZ: f32 = 1_000.0;
const FAST_TUNE_STEP_HZ: f32 = 10_000.0;
const PITCH_STEP_HZ: f32 = 50.0;

pub fn collect_keyboard_actions(window: &Window, state: &UiState) -> Vec<UiAction> {
    let mut actions = Vec::new();

    // Demod mode shortcuts
    if window.is_key_pressed(Key::Key1, KeyRepeat::No) {
        actions.push(UiAction::SetDemodMode("wfm"));
    }

    if window.is_key_pressed(Key::Key2, KeyRepeat::No) {
        actions.push(UiAction::SetDemodMode("usb"));
        actions.push(UiAction::SetSideband("usb"));
    }

    if window.is_key_pressed(Key::Key3, KeyRepeat::No) {
        actions.push(UiAction::SetDemodMode("lsb"));
        actions.push(UiAction::SetSideband("lsb"));
    }

    // SSB pitch controls
    if window.is_key_pressed(Key::LeftBracket, KeyRepeat::Yes) {
        actions.push(UiAction::SetSsbPitch(state.ssb_pitch_hz - PITCH_STEP_HZ));
    }

    if window.is_key_pressed(Key::RightBracket, KeyRepeat::Yes) {
        actions.push(UiAction::SetSsbPitch(state.ssb_pitch_hz + PITCH_STEP_HZ));
    }

    if window.is_key_pressed(Key::Backslash, KeyRepeat::No) {
        actions.push(UiAction::SetSsbPitch(0.0));
    }

    // Simple target tuning controls
    let step_hz = if window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift) {
        FAST_TUNE_STEP_HZ
    } else {
        DEFAULT_TUNE_STEP_HZ
    };

    if window.is_key_pressed(Key::Left, KeyRepeat::Yes) {
        actions.push(UiAction::SetTargetFrequency(state.target_freq_hz - step_hz));
    }

    if window.is_key_pressed(Key::Right, KeyRepeat::Yes) {
        actions.push(UiAction::SetTargetFrequency(state.target_freq_hz + step_hz));
    }

    // Center-frequency moves, if you want them
    if window.is_key_pressed(Key::Down, KeyRepeat::Yes) {
        actions.push(UiAction::SetCenterFrequency(state.center_freq_hz - step_hz));
    }

    if window.is_key_pressed(Key::Up, KeyRepeat::Yes) {
        actions.push(UiAction::SetCenterFrequency(state.center_freq_hz + step_hz));
    }

    // Optional ping/debug shortcut
    if window.is_key_pressed(Key::P, KeyRepeat::No) {
        actions.push(UiAction::Ping);
    }

    actions
}
