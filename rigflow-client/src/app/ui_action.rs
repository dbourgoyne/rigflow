#[derive(Debug, Clone)]
pub enum UiAction {
    SetTargetFrequency(f32),
    SetCenterFrequency(f32),
    SetDemodMode(&'static str),
    SetSideband(&'static str),
    SetSsbPitch(f32),

    ToggleRigflowServerMenu,
    FocusRigflowServerIpField,
    ConnectToRigflowServer,
    DisconnectFromRigflowServer,

    CycleLicenseForward,
    CycleLicenseBackward,
}
