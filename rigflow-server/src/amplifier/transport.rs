//! Transport seam for amplifier communication.
//!
//! Decouples the HR50 protocol/polling from how bytes reach the amp.  The amp is
//! controlled over a full-duplex USB serial link, implemented by
//! [`super::serial::SerialTransport`]; the trait keeps the protocol layer
//! testable with a mock and leaves room for other serial amp models.

use std::io;
use std::time::Duration;

/// A bidirectional serial link to an amplifier.
pub trait AmplifierTransport: Send {
    /// Write a command (already framed, e.g. `b"HRRX;"`).
    fn write_cmd(&mut self, bytes: &[u8]) -> io::Result<()>;

    /// Read one `;`-terminated response, decoded as UTF-8 (lossy).  Returns
    /// `Ok(None)` if no complete response arrives within `timeout`.
    fn read_response(&mut self, timeout: Duration) -> io::Result<Option<String>>;
}
