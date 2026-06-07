//! Transport seam for amplifier communication.
//!
//! Decouples the HR50 protocol/polling from how bytes actually reach the amp.
//! Phase 1 ships only [`super::serial::SerialTransport`] (full-duplex serial over
//! the Raspberry Pi USB link).  A future write-only `Hl2BlindTransport` (the HL2
//! ACC "master" band command, which has no return path) can implement this trait
//! with `is_bidirectional() == false`; detection/polling are skipped for such a
//! transport, so it provides control without pretending to offer status.

use std::io;
use std::time::Duration;

/// A bidirectional (or write-only) link to an amplifier.
pub trait AmplifierTransport: Send {
    /// Write a command (already framed, e.g. `b"HRRX;"`).
    fn write_cmd(&mut self, bytes: &[u8]) -> io::Result<()>;

    /// Read one `;`-terminated response, decoded as UTF-8 (lossy).  Returns
    /// `Ok(None)` if no complete response arrives within `timeout`.
    fn read_response(&mut self, timeout: Duration) -> io::Result<Option<String>>;

    /// Whether this transport can read replies.  Detection/polling only run on a
    /// bidirectional transport.
    fn is_bidirectional(&self) -> bool;
}
