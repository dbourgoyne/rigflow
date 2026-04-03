use rigflow_protocol::ClientMessage;

use crate::{
    input::keyboard::UiAction,
    net::control::ControlCommand,
};

pub fn ui_action_to_control_command(
    action: UiAction,
    server_ip: &str,
) -> Option<ControlCommand> {
    match action {
        UiAction::SetTargetFrequency(target_freq_hz) => {
            Some(ControlCommand::LegacyClientMessage(
                ClientMessage::SetFrequency { target_freq_hz },
            ))
        }

        UiAction::SetCenterFrequency(center_freq_hz) => {
            Some(ControlCommand::LegacyClientMessage(
                ClientMessage::SetCenterFrequency { center_freq_hz },
            ))
        }

        UiAction::SetDemodMode(mode) => {
            Some(ControlCommand::LegacyClientMessage(
                ClientMessage::SetDemodMode {
                    mode: mode.to_string(),
                },
            ))
        }

        UiAction::SetSideband(sideband) => {
            Some(ControlCommand::LegacyClientMessage(
                ClientMessage::SetSideband {
                    sideband: sideband.to_string(),
                },
            ))
        }

        UiAction::SetSsbPitch(pitch_hz) => {
            Some(ControlCommand::LegacyClientMessage(
                ClientMessage::SetSsbPitch { pitch_hz },
            ))
        }

        UiAction::Ping => {
            Some(ControlCommand::LegacyClientMessage(ClientMessage::Ping))
        }

        UiAction::ConnectToRigflowServer => {
            Some(ControlCommand::Connect {
                server_ip: server_ip.to_string(),
            })
        }

        UiAction::DisconnectFromRigflowServer => {
            Some(ControlCommand::Disconnect)
        }

        UiAction::ToggleRigflowServerMenu => None,
        UiAction::FocusRigflowServerIpField => None,
        UiAction::CycleLicenseForward => None,
        UiAction::CycleLicenseBackward => None,
    }
}
