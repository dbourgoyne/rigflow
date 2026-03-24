pub const MAGIC: u16 = 0x5253;
pub const VERSION: u8 = 1;

pub const STREAM_TYPE_AUDIO: u8 = 1;
pub const STREAM_TYPE_WATERFALL: u8 = 2;
pub const STREAM_TYPE_REGISTER_AUDIO: u8 = 10;

#[derive(Debug, Clone, Copy)]
pub struct MediaHeader {
    pub magic: u16,
    pub version: u8,
    pub stream_type: u8,
    pub sequence: u32,
    pub timestamp: u64,
}

pub fn parse_media_header(packet: &[u8]) -> Option<MediaHeader> {
    if packet.len() < 16 {
        return None;
    }

    Some(MediaHeader {
        magic: u16::from_be_bytes([packet[0], packet[1]]),
        version: packet[2],
        stream_type: packet[3],
        sequence: u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]),
        timestamp: u64::from_be_bytes([
            packet[8], packet[9], packet[10], packet[11],
            packet[12], packet[13], packet[14], packet[15],
        ]),
    })
}

pub fn is_valid_header(h: &MediaHeader) -> bool {
    h.magic == MAGIC && h.version == VERSION
}
