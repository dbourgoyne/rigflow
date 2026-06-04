/// Magic value used to identify rigflow UDP packets.
///
/// "RS" → Rigflow Stream
pub const MAGIC: u16 = 0x5253;

/// Protocol version for UDP framing.
///
/// Allows future evolution while maintaining backward compatibility.
pub const VERSION: u8 = 1;

/// Stream type identifiers.
///
/// These distinguish payload formats carried over UDP.
pub const STREAM_TYPE_AUDIO: u8 = 1;
pub const STREAM_TYPE_WATERFALL: u8 = 2;
pub const STREAM_TYPE_REGISTER_AUDIO: u8 = 10;
/// Client → server microphone audio (mono f32 LE samples after a 4-byte
/// `MAGIC/VERSION/stream_type/_` header).  Loss-tolerant; no sequence/codec.
pub const STREAM_TYPE_MIC_AUDIO: u8 = 11;

/// Fixed-size header present at the start of every UDP media packet.
///
/// Layout (big-endian):
///
/// ```text
/// 0–1   : magic (u16)
/// 2     : version (u8)
/// 3     : stream_type (u8)
/// 4–7   : sequence (u32)
/// 8–15  : timestamp (u64)
/// ```
///
/// Total size: 16 bytes
#[derive(Debug, Clone, Copy)]
pub struct MediaHeader {
    /// Magic identifier (must equal `MAGIC`)
    pub magic: u16,

    /// Protocol version (must equal `VERSION`)
    pub version: u8,

    /// Stream type (audio, waterfall, etc.)
    pub stream_type: u8,

    /// Packet sequence number (monotonic, wrapping)
    pub sequence: u32,

    /// Timestamp in stream timebase (sender-defined units)
    pub timestamp: u64,
}

/// Parse a `MediaHeader` from the beginning of a UDP packet.
///
/// Returns:
/// - `Some(header)` if the packet is large enough
/// - `None` if the packet is too short to contain a header
///
/// Note:
/// - This function does **not** validate the header contents.
/// - Use `is_valid_header()` after parsing to check protocol compatibility.
pub fn parse_media_header(packet: &[u8]) -> Option<MediaHeader> {
    // Header is fixed at 16 bytes
    if packet.len() < 16 {
        return None;
    }

    Some(MediaHeader {
        magic: u16::from_be_bytes([packet[0], packet[1]]),
        version: packet[2],
        stream_type: packet[3],
        sequence: u32::from_be_bytes([
            packet[4], packet[5], packet[6], packet[7],
        ]),
        timestamp: u64::from_be_bytes([
            packet[8], packet[9], packet[10], packet[11],
            packet[12], packet[13], packet[14], packet[15],
        ]),
    })
}

/// Validate a parsed media header.
///
/// Checks:
/// - magic matches expected value
/// - version matches supported protocol version
///
/// This does **not** validate stream type or sequence continuity.
pub fn is_valid_header(header: &MediaHeader) -> bool {
    header.magic == MAGIC && header.version == VERSION
}
