/// Magic value used to identify rigflow UDP packets.
///
/// "RS" → Rigflow Stream
pub const MAGIC: u16 = 0x5253;

/// Protocol version for UDP framing.
///
/// Allows future evolution while maintaining backward compatibility.
///
/// **v2** adds an 8-byte server send wall-clock (epoch nanoseconds) between the
/// header and the samples of **audio** packets, and the `TIME_SYNC` stream types,
/// enabling clock-offset / one-way latency measurement. v1 packets (no wall-clock)
/// are still accepted.
pub const VERSION: u8 = 2;

/// Stream type identifiers.
///
/// These distinguish payload formats carried over UDP.
pub const STREAM_TYPE_AUDIO: u8 = 1;
pub const STREAM_TYPE_WATERFALL: u8 = 2;
pub const STREAM_TYPE_REGISTER_AUDIO: u8 = 10;
/// Client → server microphone audio (mono f32 LE samples after a 4-byte
/// `MAGIC/VERSION/stream_type/_` header).  Loss-tolerant; no sequence/codec.
pub const STREAM_TYPE_MIC_AUDIO: u8 = 11;
/// Client → server clock-offset probe. 16-byte header (sequence = probe id) +
/// `t1` (client send wall-clock, epoch ns, big-endian).
pub const STREAM_TYPE_TIME_SYNC_REQUEST: u8 = 12;
/// Server → client clock-offset reply. 16-byte header (echoes the request
/// sequence) + `t1`, `t2` (server recv), `t3` (server send), all epoch-ns BE.
pub const STREAM_TYPE_TIME_SYNC_RESPONSE: u8 = 13;

/// Size of the fixed media header, in bytes.
pub const HEADER_LEN: usize = 16;
/// Size of the v2 audio send-wall-clock field (epoch nanoseconds), in bytes.
pub const AUDIO_SEND_WALL_NS_LEN: usize = 8;

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
        sequence: u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]),
        timestamp: u64::from_be_bytes([
            packet[8], packet[9], packet[10], packet[11], packet[12], packet[13], packet[14],
            packet[15],
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
///
/// Accepts any protocol version from 1 up to the current `VERSION`, so a v2 peer
/// still understands v1 packets (which simply lack the audio wall-clock).
pub fn is_valid_header(header: &MediaHeader) -> bool {
    header.magic == MAGIC && header.version >= 1 && header.version <= VERSION
}

/// Byte offset where the i16 audio samples begin in an audio packet of the given
/// protocol `version`. v2 audio packets carry an 8-byte send wall-clock between the
/// header and the samples; v1 packets do not.
pub fn audio_samples_offset(version: u8) -> usize {
    if version >= 2 {
        HEADER_LEN + AUDIO_SEND_WALL_NS_LEN
    } else {
        HEADER_LEN
    }
}

/// Read the server send wall-clock (epoch nanoseconds) from a v2 audio packet.
/// Returns `None` for v1 packets or if the packet is too short.
pub fn audio_send_wall_ns(header: &MediaHeader, packet: &[u8]) -> Option<u64> {
    if header.version < 2 || packet.len() < HEADER_LEN + AUDIO_SEND_WALL_NS_LEN {
        return None;
    }
    let b: [u8; 8] = packet[HEADER_LEN..HEADER_LEN + 8].try_into().ok()?;
    Some(u64::from_be_bytes(b))
}

/// Current wall-clock time as nanoseconds since the Unix epoch (saturating to 0
/// before the epoch). Used for the audio send-stamp and the `TIME_SYNC` exchange;
/// must be the same clock domain on both ends (system clock, not monotonic).
pub fn epoch_nanos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Write the fixed 16-byte media header into `buf`.
fn push_header(buf: &mut Vec<u8>, stream_type: u8, sequence: u32, timestamp: u64) {
    buf.extend_from_slice(&MAGIC.to_be_bytes());
    buf.push(VERSION);
    buf.push(stream_type);
    buf.extend_from_slice(&sequence.to_be_bytes());
    buf.extend_from_slice(&timestamp.to_be_bytes());
}

/// Build a `TIME_SYNC` request packet carrying the client's send wall-clock `t1`.
pub fn build_time_sync_request(probe_id: u32, t1_ns: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(HEADER_LEN + 8);
    push_header(&mut buf, STREAM_TYPE_TIME_SYNC_REQUEST, probe_id, 0);
    buf.extend_from_slice(&t1_ns.to_be_bytes());
    buf
}

/// Parse a `TIME_SYNC` request → `(probe_id, t1_ns)`.
pub fn parse_time_sync_request(packet: &[u8]) -> Option<(u32, u64)> {
    let header = parse_media_header(packet)?;
    if header.stream_type != STREAM_TYPE_TIME_SYNC_REQUEST || packet.len() < HEADER_LEN + 8 {
        return None;
    }
    let t1: [u8; 8] = packet[HEADER_LEN..HEADER_LEN + 8].try_into().ok()?;
    Some((header.sequence, u64::from_be_bytes(t1)))
}

/// Build a `TIME_SYNC` response echoing `t1` and adding the server receive (`t2`)
/// and send (`t3`) wall-clocks.
pub fn build_time_sync_response(probe_id: u32, t1_ns: u64, t2_ns: u64, t3_ns: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(HEADER_LEN + 24);
    push_header(&mut buf, STREAM_TYPE_TIME_SYNC_RESPONSE, probe_id, 0);
    buf.extend_from_slice(&t1_ns.to_be_bytes());
    buf.extend_from_slice(&t2_ns.to_be_bytes());
    buf.extend_from_slice(&t3_ns.to_be_bytes());
    buf
}

/// Parse a `TIME_SYNC` response → `(probe_id, t1_ns, t2_ns, t3_ns)`.
pub fn parse_time_sync_response(packet: &[u8]) -> Option<(u32, u64, u64, u64)> {
    let header = parse_media_header(packet)?;
    if header.stream_type != STREAM_TYPE_TIME_SYNC_RESPONSE || packet.len() < HEADER_LEN + 24 {
        return None;
    }
    let t1: [u8; 8] = packet[16..24].try_into().ok()?;
    let t2: [u8; 8] = packet[24..32].try_into().ok()?;
    let t3: [u8; 8] = packet[32..40].try_into().ok()?;
    Some((
        header.sequence,
        u64::from_be_bytes(t1),
        u64::from_be_bytes(t2),
        u64::from_be_bytes(t3),
    ))
}

/// NTP / Cristian's-algorithm 4-timestamp clock offset and round-trip time, from
/// the four epoch-nanosecond timestamps of one probe exchange:
/// `T1` client send, `T2` server recv, `T3` server send, `T4` client recv.
///
/// - `offset = ((T2−T1) + (T3−T4)) / 2`  — the **server clock minus the client
///   clock** (positive ⇒ server ahead). A server timestamp `S` corresponds to
///   client time `S − offset`, so the one-way delay of an audio packet is
///   `one_way = T4_client_recv − server_send_wall + offset`.
/// - `rtt    = (T4−T1) − (T3−T2)`         — network round-trip, server processing removed.
///
/// Computed in `i128` to avoid overflow; results fit comfortably in `i64`.
pub fn clock_offset_rtt(t1: u64, t2: u64, t3: u64, t4: u64) -> (i64, i64) {
    let (t1, t2, t3, t4) = (t1 as i128, t2 as i128, t3 as i128, t4 as i128);
    let offset = ((t2 - t1) + (t3 - t4)) / 2;
    let rtt = (t4 - t1) - (t3 - t2);
    (offset as i64, rtt as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_validation_accepts_v1_and_v2() {
        let mut h = parse_media_header(&build_time_sync_request(1, 42)).unwrap();
        assert!(is_valid_header(&h));
        h.version = 1;
        assert!(is_valid_header(&h));
        h.version = 3;
        assert!(!is_valid_header(&h));
        h.version = 2;
        h.magic = 0;
        assert!(!is_valid_header(&h));
    }

    #[test]
    fn audio_offset_depends_on_version() {
        assert_eq!(audio_samples_offset(1), 16);
        assert_eq!(audio_samples_offset(2), 24);
    }

    #[test]
    fn time_sync_request_round_trip() {
        let pkt = build_time_sync_request(7, 123_456_789);
        assert_eq!(parse_time_sync_request(&pkt), Some((7, 123_456_789)));
        // A response packet must not parse as a request.
        let resp = build_time_sync_response(7, 1, 2, 3);
        assert_eq!(parse_time_sync_request(&resp), None);
    }

    #[test]
    fn time_sync_response_round_trip() {
        let pkt = build_time_sync_response(9, 10, 20, 30);
        assert_eq!(parse_time_sync_response(&pkt), Some((9, 10, 20, 30)));
        let req = build_time_sync_request(9, 10);
        assert_eq!(parse_time_sync_response(&req), None);
    }

    #[test]
    fn offset_and_rtt_math() {
        // Server clock is 1000 ns ahead; symmetric 100 ns each way; 0 processing.
        // T1=0 (client), T2=1100 (server recv), T3=1100 (server send), T4=200 (client).
        let (offset, rtt) = clock_offset_rtt(0, 1100, 1100, 200);
        assert_eq!(offset, 1000); // offset = server − client (server is 1000 ns ahead)
        assert_eq!(rtt, 200);
    }

    #[test]
    fn offset_handles_server_processing_delay() {
        // No clock skew, 50 ns each way, 400 ns server processing.
        // T1=0, T2=50, T3=450, T4=500.
        let (offset, rtt) = clock_offset_rtt(0, 50, 450, 500);
        assert_eq!(offset, 0);
        assert_eq!(rtt, 100); // processing (400) removed
    }
}
