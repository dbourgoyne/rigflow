//! Decoder for the WSJT-X UDP telemetry protocol (the datagrams it sends to
//! UDP port 2237).
//!
//! The wire format is a Qt `QDataStream` `NetworkMessage`: big-endian, framed
//! as `quint32 magic (0xADBCCBDA)`, `quint32 schema`, `quint32 message_type`,
//! then a UTF-8 client id, then message-specific fields. Strings are QByteArrays
//! (a `quint32` byte length then the bytes; length `0xFFFFFFFF` = null).
//! Verified against WSJT-X `NetworkMessage.hpp`.
//!
//! We decode the **`LoggedADIF`** message (type 12), which carries the contact
//! as an ADIF string we feed straight into [`crate::adif`]. WSJT-X emits this
//! alongside the typed `QSOLogged` (type 5) for every logged QSO; we
//! deliberately ingest only `LoggedADIF` so a contact is never logged twice.
//! Every other message type is reported as [`WsjtxMessage::Other`] for the
//! caller to ignore.

/// WSJT-X datagram magic number.
pub const MAGIC: u32 = 0xADBC_CBDA;

// Message type discriminants (NetworkMessage.hpp).
const MSG_QSO_LOGGED: u32 = 5;
const MSG_LOGGED_ADIF: u32 = 12;

/// A decoded WSJT-X message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsjtxMessage {
    /// `LoggedADIF` (type 12): the logged contact as an ADIF document.
    LoggedAdif { client_id: String, adif: String },
    /// `QSOLogged` (type 5): recognized but intentionally not ingested (the
    /// `LoggedADIF` for the same QSO is used instead).
    QsoLoggedIgnored,
    /// Any other message type (Heartbeat, Status, Decode, …).
    Other { message_type: u32 },
}

/// Decode a single datagram. Returns `None` if it isn't a WSJT-X message (bad
/// magic) or is truncated/malformed.
pub fn decode(buf: &[u8]) -> Option<WsjtxMessage> {
    let mut r = Reader::new(buf);
    if r.u32()? != MAGIC {
        return None;
    }
    let _schema = r.u32()?;
    let msg_type = r.u32()?;
    let client_id = r.qstr()?; // present on every message

    match msg_type {
        MSG_LOGGED_ADIF => {
            let adif = r.qstr()?;
            Some(WsjtxMessage::LoggedAdif { client_id, adif })
        }
        MSG_QSO_LOGGED => Some(WsjtxMessage::QsoLoggedIgnored),
        other => Some(WsjtxMessage::Other {
            message_type: other,
        }),
    }
}

/// Minimal big-endian QDataStream reader. Every method returns `None` on
/// underflow so a truncated packet decodes to `None` rather than panicking.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let slice = self.buf.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }

    fn u32(&mut self) -> Option<u32> {
        let b = self.take(4)?;
        Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A QByteArray/utf8 string: `quint32` length then bytes. Length
    /// `0xFFFFFFFF` denotes a null string, decoded as empty. Invalid UTF-8 is
    /// decoded lossily rather than rejected.
    fn qstr(&mut self) -> Option<String> {
        let len = self.u32()?;
        if len == 0xFFFF_FFFF {
            return Some(String::new());
        }
        let bytes = self.take(len as usize)?;
        Some(String::from_utf8_lossy(bytes).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test datagram builder mirroring WSJT-X's big-endian QDataStream framing.
    struct Builder {
        buf: Vec<u8>,
    }
    impl Builder {
        fn new(msg_type: u32) -> Self {
            let mut buf = Vec::new();
            buf.extend_from_slice(&MAGIC.to_be_bytes());
            buf.extend_from_slice(&3u32.to_be_bytes()); // schema 3
            buf.extend_from_slice(&msg_type.to_be_bytes());
            Builder { buf }
        }
        fn qstr(mut self, s: &str) -> Self {
            self.buf.extend_from_slice(&(s.len() as u32).to_be_bytes());
            self.buf.extend_from_slice(s.as_bytes());
            self
        }
        fn null_qstr(mut self) -> Self {
            self.buf.extend_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
            self
        }
        fn build(self) -> Vec<u8> {
            self.buf
        }
    }

    const SAMPLE_ADIF: &str = "<ADIF_VER:5>3.1.6<EOH><CALL:4>W1AW<QSO_DATE:8>20260711<TIME_ON:6>142300\
         <BAND:3>20m<MODE:3>FT8<FREQ:9>14.074000<EOR>";

    #[test]
    fn decode_logged_adif() {
        let dg = Builder::new(MSG_LOGGED_ADIF)
            .qstr("WSJT-X")
            .qstr(SAMPLE_ADIF)
            .build();
        match decode(&dg) {
            Some(WsjtxMessage::LoggedAdif { client_id, adif }) => {
                assert_eq!(client_id, "WSJT-X");
                assert_eq!(adif, SAMPLE_ADIF);
            }
            other => panic!("expected LoggedAdif, got {other:?}"),
        }
    }

    #[test]
    fn logged_adif_feeds_the_parser() {
        let dg = Builder::new(MSG_LOGGED_ADIF)
            .qstr("WSJT-X")
            .qstr(SAMPLE_ADIF)
            .build();
        let WsjtxMessage::LoggedAdif { adif, .. } = decode(&dg).unwrap() else {
            panic!("expected LoggedAdif");
        };
        let qsos = crate::adif::parse_adif_to_qsos(&adif).unwrap();
        assert_eq!(qsos.len(), 1);
        assert_eq!(qsos[0].call, "W1AW");
        assert_eq!(qsos[0].band, "20m");
        assert_eq!(qsos[0].mode, "FT8");
        assert_eq!(qsos[0].freq_hz, Some(14_074_000));
    }

    #[test]
    fn null_client_id_is_handled() {
        let dg = Builder::new(MSG_LOGGED_ADIF)
            .null_qstr()
            .qstr(SAMPLE_ADIF)
            .build();
        assert!(matches!(decode(&dg), Some(WsjtxMessage::LoggedAdif { .. })));
    }

    #[test]
    fn qso_logged_is_recognized_but_ignored() {
        let dg = Builder::new(MSG_QSO_LOGGED).qstr("WSJT-X").build();
        assert_eq!(decode(&dg), Some(WsjtxMessage::QsoLoggedIgnored));
    }

    #[test]
    fn status_message_is_other() {
        let dg = Builder::new(1).qstr("WSJT-X").build();
        assert_eq!(decode(&dg), Some(WsjtxMessage::Other { message_type: 1 }));
    }

    #[test]
    fn wrong_magic_is_none() {
        let mut dg = Builder::new(MSG_LOGGED_ADIF).qstr("WSJT-X").build();
        dg[0] = 0x00; // corrupt the magic
        assert_eq!(decode(&dg), None);
    }

    #[test]
    fn truncated_is_none() {
        let dg = Builder::new(MSG_LOGGED_ADIF)
            .qstr("WSJT-X")
            .qstr(SAMPLE_ADIF)
            .build();
        // Cut the ADIF string's declared length short.
        assert_eq!(decode(&dg[..dg.len() - 10]), None);
        // Just the header, no id/payload.
        assert_eq!(decode(&dg[..12]), None);
    }

    #[test]
    fn empty_buffer_is_none() {
        assert_eq!(decode(&[]), None);
    }
}
