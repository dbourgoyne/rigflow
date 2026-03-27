use rigflow_protocol::ClientMessage;

use crate::input::keyboard::UiAction;

pub fn ui_action_to_client_message(action: UiAction) -> Option<ClientMessage> {
    match action {
        UiAction::SetTargetFrequency(target_freq_hz) => {
            Some(ClientMessage::SetFrequency { target_freq_hz })
        }
        UiAction::SetCenterFrequency(center_freq_hz) => {
            Some(ClientMessage::SetCenterFrequency { center_freq_hz })
        }
        UiAction::SetDemodMode(mode) => {
            Some(ClientMessage::SetDemodMode {
                mode: mode.to_string(),
            })
        }
        UiAction::SetSideband(sideband) => {
            Some(ClientMessage::SetSideband {
                sideband: sideband.to_string(),
            })
        }
        UiAction::SetSsbPitch(pitch_hz) => {
            Some(ClientMessage::SetSsbPitch { pitch_hz })
        }
        UiAction::Ping => Some(ClientMessage::Ping),

        UiAction::CycleLicenseForward => None,
        UiAction::CycleLicenseBackward => None,
    }
}
