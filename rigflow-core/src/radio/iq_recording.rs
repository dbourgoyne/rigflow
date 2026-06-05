/// Status of server-side receive IQ recording (IQ Recording Phase 1).
///
/// Read-only telemetry pushed to the client so the Source Control panel can
/// show recording state, the output filename, elapsed time, file size, and the
/// dropped-buffer counter.  All fields are inert when not recording.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct IqRecordingStatus {
    /// True while a recording is in progress.
    pub recording: bool,
    /// Output file name (no path), `None` when idle / never recorded.
    pub filename: Option<String>,
    /// Elapsed recording time in seconds.
    pub elapsed_secs: u64,
    /// Current output file size in bytes.
    pub file_size_bytes: u64,
    /// Count of IQ buffers dropped because the disk writer fell behind.
    pub dropped_buffers: u64,
}
