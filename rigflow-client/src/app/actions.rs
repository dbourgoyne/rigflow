use rigflow_protocol::ClientMessage;
use crate::input::keyboard::UiAction;

pub fn ui_action_to_client_message(action: UiAction) -> ClientMessage {
    match action {
        UiAction::SetTargetFrequency(target_freq_hz) => {
            ClientMessage::SetFrequency { target_freq_hz }
        }
        UiAction::SetCenterFrequency(center_freq_hz) => {
            ClientMessage::SetCenterFrequency { center_freq_hz }
        }
        UiAction::SetDemodMode(mode) => {
            ClientMessage::SetDemodMode { mode: mode.to_string() }
        }
        UiAction::SetSideband(sideband) => {
            ClientMessage::SetSideband { sideband: sideband.to_string() }
        }
        UiAction::SetSsbPitch(pitch_hz) => {
            ClientMessage::SetSsbPitch { pitch_hz }
        }
        UiAction::Ping => ClientMessage::Ping,
    }
}
