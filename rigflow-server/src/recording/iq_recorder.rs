//! Receive IQ recorder (IQ Recording Phase 1).
//!
//! Records the raw IQ stream **immediately after the hardware source**, before
//! any DSP / demod / AGC / filtering, so recordings hold the original received
//! samples at full bandwidth.  Writing is fully asynchronous: the capture
//! thread only does a non-blocking `try_send` of each IQ block onto a bounded
//! channel; a dedicated background thread drains the channel and writes a
//! standard IEEE-float IQ WAV file.  If the disk can't keep up the channel
//! fills, blocks are dropped (counted, logged) and the radio keeps running.
//!
//! File format: canonical RIFF/WAVE, `WAVE_FORMAT_IEEE_FLOAT`, 2 channels
//! (ch0 = I, ch1 = Q), 32-bit float, at the source sample rate — this is
//! exactly what Rigflow's WAV source (`source::wav::IqWavReader`) already plays
//! back, so a recording dropped into the wav directory shows up in the radio
//! list and replays directly.  The center frequency is embedded in the
//! filename (`..._<hz>Hz.wav`) because that is how playback recovers tuning
//! (`wav_metadata::parse_center_freq_hz_from_filename`).  An `auxi` chunk (the
//! SDR convention used by HDSDR / SDR Console) additionally carries center
//! frequency, sample rate, and start time for interop with other SDR tools.

use std::fs::{self, File};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use num_complex::Complex32;

/// Bounded channel depth (IQ blocks).  Generous enough to ride out short disk
/// stalls without unbounded memory growth.
const CHANNEL_CAPACITY: usize = 256;

/// Parameters captured at the start of a recording (for the WAV `auxi` chunk).
#[derive(Debug, Clone)]
pub struct RecordParams {
    pub sample_rate_hz: u32,
    pub center_freq_hz: u32,
    pub gain_db: f32,
    pub ppm: f32,
    pub source: String,
}

/// A live recording handle owned by the capture thread.
pub struct IqRecorder {
    tx: Option<SyncSender<Vec<Complex32>>>,
    writer: Option<JoinHandle<()>>,
    dropped: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
    filename: String,
    started: Instant,
}

impl IqRecorder {
    /// Begin a recording, spawning the background writer thread.  Writes into
    /// `dir` (the server's wav directory, so recordings auto-appear in the radio
    /// list) and creates it if needed.  Returns an error (without side effects
    /// on the caller) if the file can't be created.
    pub fn start(dir: &Path, params: RecordParams) -> Result<Self, String> {
        fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;

        // Embed the center frequency in the name so Rigflow's existing WAV
        // playback (which recovers tuning from the filename via
        // `wav_metadata::parse_center_freq_hz_from_filename`) plays the file
        // back at the correct center frequency when it is dropped into the
        // wav directory.
        let filename = format!("iq_{}_{}Hz.wav", timestamp_compact(), params.center_freq_hz);
        let path = dir.join(&filename);

        let file = File::create(&path).map_err(|e| format!("create {}: {e}", path.display()))?;

        let dropped = Arc::new(AtomicU64::new(0));
        let bytes = Arc::new(AtomicU64::new(0));
        let (tx, rx) = sync_channel::<Vec<Complex32>>(CHANNEL_CAPACITY);

        let bytes_w = Arc::clone(&bytes);
        let params_w = params.clone();
        let writer = thread::Builder::new()
            .name("iq-recorder".into())
            .spawn(move || writer_loop(file, rx, params_w, bytes_w))
            .map_err(|e| format!("spawn writer: {e}"))?;

        log::info!(
            "[iq-rec] recording started: {} ({} Hz, center {} Hz)",
            filename,
            params.sample_rate_hz,
            params.center_freq_hz
        );

        Ok(Self {
            tx: Some(tx),
            writer: Some(writer),
            dropped,
            bytes,
            filename,
            started: Instant::now(),
        })
    }

    /// Queue one IQ block for writing (non-blocking).  On a full channel the
    /// block is dropped and counted; the radio is never stalled.
    pub fn record_block(&self, iq: &[Complex32]) {
        let Some(tx) = &self.tx else { return };
        match tx.try_send(iq.to_vec()) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                let n = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
                log::debug!("[iq-rec] recording overrun — dropped buffers: {n}");
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }

    pub fn elapsed_secs(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    pub fn file_size_bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    pub fn dropped_buffers(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Stop recording: close the channel so the writer finalizes the WAV header
    /// and flushes, then join it.
    pub fn stop(mut self) {
        // Dropping the sender signals end-of-stream to the writer.
        self.tx.take();
        if let Some(h) = self.writer.take() {
            let _ = h.join();
        }
        log::info!(
            "[iq-rec] recording stopped: {} ({} bytes, {} dropped)",
            self.filename,
            self.file_size_bytes(),
            self.dropped_buffers()
        );
    }
}

/// Background writer: emit the WAV header, append interleaved float IQ for each
/// received block, and finalize the RIFF/data sizes when the channel closes.
fn writer_loop(
    file: File,
    rx: Receiver<Vec<Complex32>>,
    params: RecordParams,
    bytes: Arc<AtomicU64>,
) {
    let mut w = BufWriter::new(file);
    if let Err(e) = write_header(&mut w, &params) {
        log::error!("[iq-rec] WAV header write failed: {e}");
        return;
    }

    let mut data_bytes: u64 = 0;
    let mut scratch: Vec<u8> = Vec::new();
    while let Ok(block) = rx.recv() {
        scratch.clear();
        scratch.reserve(block.len() * 8);
        for s in &block {
            scratch.extend_from_slice(&s.re.to_le_bytes());
            scratch.extend_from_slice(&s.im.to_le_bytes());
        }
        if let Err(e) = w.write_all(&scratch) {
            log::error!("[iq-rec] IQ write failed: {e}");
            break;
        }
        data_bytes += scratch.len() as u64;
        bytes.store(data_bytes, Ordering::Relaxed);
    }

    if let Err(e) = finalize(&mut w, data_bytes) {
        log::error!("[iq-rec] WAV finalize failed: {e}");
    }
}

// ---- WAV (RIFF/WAVE) writing --------------------------------------------

const WAVE_FORMAT_IEEE_FLOAT: u16 = 0x0003;
const CHANNELS: u16 = 2; // I, Q
const BITS_PER_SAMPLE: u16 = 32;

/// Byte offsets patched on finalize (sizes unknown until the recording ends).
const RIFF_SIZE_OFFSET: u64 = 4;

/// Write the RIFF header, `fmt `, `auxi` (metadata), and the `data` chunk
/// header with placeholder sizes (RIFF size and data size are patched in
/// `finalize` once the total byte count is known).
fn write_header<W: Write>(w: &mut W, params: &RecordParams) -> std::io::Result<()> {
    let sr = params.sample_rate_hz;
    let block_align = CHANNELS * BITS_PER_SAMPLE / 8; // 8 bytes
    let byte_rate = sr * block_align as u32;

    w.write_all(b"RIFF")?;
    w.write_all(&0u32.to_le_bytes())?; // RIFF size (patched)
    w.write_all(b"WAVE")?;

    // fmt chunk (16 bytes).
    w.write_all(b"fmt ")?;
    w.write_all(&16u32.to_le_bytes())?;
    w.write_all(&WAVE_FORMAT_IEEE_FLOAT.to_le_bytes())?;
    w.write_all(&CHANNELS.to_le_bytes())?;
    w.write_all(&sr.to_le_bytes())?;
    w.write_all(&byte_rate.to_le_bytes())?;
    w.write_all(&block_align.to_le_bytes())?;
    w.write_all(&BITS_PER_SAMPLE.to_le_bytes())?;

    // auxi chunk (SDR metadata: start time, center freq, sample rate).
    write_auxi(w, params)?;

    // data chunk header (size patched on finalize).
    w.write_all(b"data")?;
    w.write_all(&0u32.to_le_bytes())?;
    Ok(())
}

/// SDR `auxi` chunk (HDSDR / SDR Console convention): two SYSTEMTIMEs followed
/// by center frequency, sample rate, and reserved fields.  68-byte body.
fn write_auxi<W: Write>(w: &mut W, params: &RecordParams) -> std::io::Result<()> {
    let (year, month, day, hour, min, sec) = now_civil();
    let dow = day_of_week(year, month, day);

    let mut body = Vec::with_capacity(68);
    let systemtime = |b: &mut Vec<u8>| {
        b.extend_from_slice(&(year as u16).to_le_bytes());
        b.extend_from_slice(&(month as u16).to_le_bytes());
        b.extend_from_slice(&(dow as u16).to_le_bytes());
        b.extend_from_slice(&(day as u16).to_le_bytes());
        b.extend_from_slice(&(hour as u16).to_le_bytes());
        b.extend_from_slice(&(min as u16).to_le_bytes());
        b.extend_from_slice(&(sec as u16).to_le_bytes());
        b.extend_from_slice(&0u16.to_le_bytes()); // milliseconds
    };
    systemtime(&mut body); // StartTime
    systemtime(&mut body); // StopTime (start time; not patched in Phase 1)
    body.extend_from_slice(&params.center_freq_hz.to_le_bytes()); // CenterFreq
    body.extend_from_slice(&params.sample_rate_hz.to_le_bytes()); // ADfrequency
    body.extend_from_slice(&0u32.to_le_bytes()); // IFFrequency
    body.extend_from_slice(&0u32.to_le_bytes()); // Bandwidth
    body.extend_from_slice(&0u32.to_le_bytes()); // IQOffset
    body.extend_from_slice(&0u32.to_le_bytes()); // Unused
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes());

    w.write_all(b"auxi")?;
    w.write_all(&(body.len() as u32).to_le_bytes())?;
    w.write_all(&body)?;
    Ok(())
}

/// Patch the RIFF and `data` chunk sizes once the total is known.
fn finalize(w: &mut BufWriter<File>, data_bytes: u64) -> std::io::Result<()> {
    w.flush()?;
    let inner = w.get_mut();
    let file_len = inner.seek(SeekFrom::End(0))?;

    // RIFF size = file length - 8 (RIFF id + size field).
    let riff_size = (file_len - 8) as u32;
    inner.seek(SeekFrom::Start(RIFF_SIZE_OFFSET))?;
    inner.write_all(&riff_size.to_le_bytes())?;

    // data chunk size is the last 4 bytes before the audio: at file_len -
    // data_bytes - 4.
    let data_size_offset = file_len - data_bytes - 4;
    inner.seek(SeekFrom::Start(data_size_offset))?;
    inner.write_all(&(data_bytes as u32).to_le_bytes())?;
    inner.flush()?;
    Ok(())
}

// ---- Time helpers (no external date dependency) --------------------------

/// `YYYY-MM-DD_HH-MM-SS` (UTC) for the filename.
fn timestamp_compact() -> String {
    let (y, mo, d, h, mi, s) = now_civil();
    format!("{y:04}-{mo:02}-{d:02}_{h:02}-{mi:02}-{s:02}")
}

/// Current UTC time as (year, month, day, hour, minute, second).
fn now_civil() -> (i64, u32, u32, u64, u64, u64) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let sod = secs % 86_400;
    let (y, mo, d) = civil_from_days(days);
    (y, mo, d, sod / 3_600, (sod % 3_600) / 60, sod % 60)
}

/// Howard Hinnant's `civil_from_days`: days since the Unix epoch → (Y, M, D).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Day of week (0 = Sunday) for the `auxi` SYSTEMTIME, via Sakamoto's method.
fn day_of_week(y: i64, m: u32, d: u32) -> u32 {
    const T: [i64; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y - 1 } else { y };
    (((y + y / 4 - y / 100 + y / 400 + T[(m - 1) as usize] + d as i64) % 7) + 7) as u32 % 7
}
