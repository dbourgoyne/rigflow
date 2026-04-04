use rigflow_protocol::ClientMessage;

#[derive(Debug, Clone)]
pub enum ControlCommand {
    Connect { server_ip: String },
    Disconnect,
    LegacyClientMessage(rigflow_protocol::ClientMessage),
    AcquireRadio { radio_id: String },
    ReleaseRadio,
}

