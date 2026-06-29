use std::sync::{Arc, Mutex};

use rigflow_core::{
    audio::jitter_buffer::JitterBuffer,
    net::udp_framing::{
        STREAM_TYPE_AUDIO, STREAM_TYPE_AUDIO_VFO_B, STREAM_TYPE_WATERFALL,
        STREAM_TYPE_WATERFALL_VFO_B, audio_samples_offset, is_valid_header, parse_media_header,
    },
    radio::vfo::VfoSelect,
};

use crate::ui::{
    layout::{WATERFALL_IMAGE_HEIGHT, WATERFALL_IMAGE_WIDTH},
    spectrum_utils::{estimate_row_floor_and_top_db, update_spectrum_db},
    state::UiState,
    waterfall::draw_row_db,
};

/// Smoothing factor for adaptive waterfall normalization.
///
/// Lower values react more slowly and look more stable.
const ADAPTIVE_NORMALIZATION_ALPHA: f32 = 0.05;

/// Extra headroom above the estimated top so strong peaks do not pin
/// the display ceiling too aggressively.
const ADAPTIVE_TOP_HEADROOM_DB: f32 = 3.0;

/// Clamp the automatically chosen visible range to something sane.
const ADAPTIVE_MIN_RANGE_DB: f32 = 30.0;
const ADAPTIVE_MAX_RANGE_DB: f32 = 100.0;

/// Runtime statistics for incoming media packets.
#[derive(Debug, Default)]
pub struct MediaPacketStats {
    pub incoming_packets: u64,
    pub audio_packets: u64,
    pub waterfall_packets: u64,
    pub dropped_audio_packets: u64,
    pub dropped_waterfall_packets: u64,
    pub late_audio_packets: u64,
    pub late_waterfall_packets: u64,
    last_audio_sequence: Option<u32>,
    last_waterfall_sequence: Option<u32>,
}

impl MediaPacketStats {
    pub fn new() -> Self {
        Self::default()
    }
}

enum StreamKind {
    Audio,
    Waterfall,
}

/// Reassembles a waterfall row from its sub-MTU chunks.
///
/// The server splits each row into chunks (see `udp_waterfall.rs`) so no datagram
/// exceeds the MTU. Each chunk carries `row_seq` / `total_bins` / `bin_offset`. We
/// accumulate chunks for the current row and emit the full row once every bin has
/// arrived. Rows are loss-tolerant: an incomplete row is dropped when the next
/// `row_seq` starts (imperceptible at ~20 rows/s, and far rarer than the whole-row
/// losses IP fragmentation used to cause).
#[derive(Default)]
pub struct WaterfallReassembler {
    row_seq: Option<u16>,
    total_bins: usize,
    buf: Vec<f32>,
    filled: usize,
    done: bool,
}

impl WaterfallReassembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one chunk (the bytes after the 16-byte media header). Returns the full
    /// row once this chunk completes it, else `None`.
    fn push_chunk(&mut self, payload: &[u8]) -> Option<&[f32]> {
        // Chunk header: row_seq(2) total_bins(2) bin_offset(2), all big-endian.
        if payload.len() < 6 {
            return None;
        }
        let row_seq = u16::from_be_bytes([payload[0], payload[1]]);
        let total_bins = u16::from_be_bytes([payload[2], payload[3]]) as usize;
        let bin_offset = u16::from_be_bytes([payload[4], payload[5]]) as usize;
        let data = &payload[6..];

        if total_bins == 0 || !data.len().is_multiple_of(4) {
            return None;
        }
        let n = data.len() / 4;
        if bin_offset + n > total_bins {
            return None;
        }

        // Start a fresh row when the row id changes (dropping any incomplete prior row).
        if self.row_seq != Some(row_seq) || self.total_bins != total_bins {
            self.row_seq = Some(row_seq);
            self.total_bins = total_bins;
            self.buf = vec![0.0; total_bins];
            self.filled = 0;
            self.done = false;
        } else if self.done {
            // Row already emitted; ignore stray/duplicate chunks.
            return None;
        }

        for (j, c) in data.chunks_exact(4).enumerate() {
            let v = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            if !v.is_finite() {
                return None;
            }
            self.buf[bin_offset + j] = v;
        }
        self.filled += n;

        if self.filled >= self.total_bins {
            self.done = true;
            return Some(&self.buf);
        }
        None
    }
}

#[allow(clippy::too_many_arguments)]
pub fn handle_media_packet(
    packet: &[u8],
    jitter: &Arc<Mutex<JitterBuffer>>,
    waterfall_buffer: &Arc<Mutex<Vec<u32>>>,
    spectrum_db: &Arc<Mutex<Vec<f32>>>,
    ui_state: &Arc<Mutex<UiState>>,
    stats: &Arc<Mutex<MediaPacketStats>>,
    cw_decoder: &mut crate::cw_decode::CwDecoder,
    digital_rx: &crate::digital_rx::DigitalRxOutput,
    tci_rx_audio: &crate::tci_server::TciRxAudio,
    waterfall_reasm: &mut WaterfallReassembler,
    // VFO B (dual-watch) sinks — second receiver's audio + spectrum/waterfall.
    jitter_b: &Arc<Mutex<JitterBuffer>>,
    waterfall_buffer_b: &Arc<Mutex<Vec<u32>>>,
    spectrum_db_b: &Arc<Mutex<Vec<f32>>>,
    waterfall_reasm_b: &mut WaterfallReassembler,
) {
    let Some(header) = parse_media_header(packet) else {
        return;
    };

    if !is_valid_header(&header) {
        return;
    }

    // Waterfall payload begins right after the 16-byte header; audio payload
    // begins after the optional v2 send-wall-clock (see `audio_samples_offset`).
    let payload = &packet[16..];

    if let Ok(mut s) = stats.lock() {
        s.incoming_packets += 1;
    }

    match header.stream_type {
        STREAM_TYPE_AUDIO => {
            if let Ok(mut s) = stats.lock() {
                s.audio_packets += 1;
                update_sequence_stats(&mut s, StreamKind::Audio, header.sequence);
            }

            let audio_payload = packet
                .get(audio_samples_offset(header.version)..)
                .unwrap_or(&[]);
            handle_audio_packet(
                audio_payload,
                header.sequence,
                jitter,
                cw_decoder,
                digital_rx,
                tci_rx_audio,
            );
        }

        STREAM_TYPE_WATERFALL => {
            if let Ok(mut s) = stats.lock() {
                s.waterfall_packets += 1;
                update_sequence_stats(&mut s, StreamKind::Waterfall, header.sequence);
            }

            handle_waterfall_packet(
                waterfall_reasm,
                payload,
                waterfall_buffer,
                spectrum_db,
                ui_state,
                VfoSelect::A,
            );
        }

        // VFO B (dual-watch): route the second receiver's streams to the B
        // buffers.  B audio does NOT feed the CW decoder / digital-RX / TCI taps
        // (those follow VFO A only).
        STREAM_TYPE_AUDIO_VFO_B => {
            let audio_payload = packet
                .get(audio_samples_offset(header.version)..)
                .unwrap_or(&[]);
            handle_audio_packet_b(audio_payload, header.sequence, jitter_b);
        }

        STREAM_TYPE_WATERFALL_VFO_B => {
            handle_waterfall_packet(
                waterfall_reasm_b,
                payload,
                waterfall_buffer_b,
                spectrum_db_b,
                ui_state,
                VfoSelect::B,
            );
        }

        _ => {}
    }
}

/// VFO B audio: decode i16 LE → f32 and push to VFO B's jitter buffer only.
/// (No CW-decode / digital-RX / TCI taps — those are VFO A.)
fn handle_audio_packet_b(payload: &[u8], sequence: u32, jitter_b: &Arc<Mutex<JitterBuffer>>) {
    if payload.len() < 2 || !payload.len().is_multiple_of(2) {
        return;
    }
    let mut samples = Vec::with_capacity(payload.len() / 2);
    for chunk in payload.chunks_exact(2) {
        let s = i16::from_le_bytes([chunk[0], chunk[1]]);
        samples.push(s as f32 / i16::MAX as f32);
    }
    if let Ok(mut jb) = jitter_b.lock() {
        jb.push_packet(sequence, samples);
    }
}

fn update_sequence_stats(stats: &mut MediaPacketStats, kind: StreamKind, sequence: u32) {
    match kind {
        StreamKind::Audio => {
            if let Some(prev) = stats.last_audio_sequence {
                let expected = prev.wrapping_add(1);

                if sequence > expected {
                    stats.dropped_audio_packets += (sequence - expected) as u64;
                } else if sequence < expected {
                    stats.late_audio_packets += 1;
                }
            }

            stats.last_audio_sequence = Some(sequence);
        }

        StreamKind::Waterfall => {
            if let Some(prev) = stats.last_waterfall_sequence {
                let expected = prev.wrapping_add(1);

                if sequence > expected {
                    stats.dropped_waterfall_packets += (sequence - expected) as u64;
                } else if sequence < expected {
                    stats.late_waterfall_packets += 1;
                }
            }

            stats.last_waterfall_sequence = Some(sequence);
        }
    }
}

fn handle_audio_packet(
    payload: &[u8],
    sequence: u32,
    jitter: &Arc<Mutex<JitterBuffer>>,
    cw_decoder: &mut crate::cw_decode::CwDecoder,
    digital_rx: &crate::digital_rx::DigitalRxOutput,
    tci_rx_audio: &crate::tci_server::TciRxAudio,
) {
    if payload.len() < 2 || !payload.len().is_multiple_of(2) {
        return;
    }

    let mut samples = Vec::with_capacity(payload.len() / 2);

    for chunk in payload.chunks_exact(2) {
        let s = i16::from_le_bytes([chunk[0], chunk[1]]);
        samples.push(s as f32 / i16::MAX as f32);
    }

    // Feed the received audio to the CW decoder (no-op unless enabled).  This
    // only reads the samples — the receive audio path is untouched.
    cw_decoder.process(&samples);

    // Mirror a copy to the digital RX output sink (no-op unless enabled).  This
    // tap is post-server-volume; the speaker path below is unaffected.
    digital_rx.push(&samples);

    // Same tap for the TCI server (no-op unless a TCI client is streaming).
    tci_rx_audio.push(&samples);

    if let Ok(mut jb) = jitter.lock() {
        jb.push_packet(sequence, samples);
    }
}

fn update_adaptive_waterfall_display(
    row_db: &[f32],
    ui_state: &Arc<Mutex<UiState>>,
    vfo: VfoSelect,
) {
    let Some((row_floor_db, row_top_db)) = estimate_row_floor_and_top_db(row_db) else {
        return;
    };

    if let Ok(mut state) = ui_state.lock() {
        // Reborrow the inner UiState so the disjoint field borrows below are seen
        // as distinct (a MutexGuard's Deref would otherwise borrow the whole guard).
        let state = &mut *state;
        // Each VFO keeps its own adaptive estimates, computed from its own rows.
        let adaptive = match vfo {
            VfoSelect::A => state.adaptive_waterfall_normalization,
            VfoSelect::B => state.vfo_b_adaptive_waterfall_normalization,
        };
        if !adaptive {
            return;
        }

        let alpha = ADAPTIVE_NORMALIZATION_ALPHA;
        let (top_est, floor_est, range_est) = match vfo {
            VfoSelect::A => (
                &mut state.adaptive_top_db_estimate,
                &mut state.adaptive_floor_db_estimate,
                &mut state.adaptive_range_db_estimate,
            ),
            VfoSelect::B => (
                &mut state.vfo_b_adaptive_top_db_estimate,
                &mut state.vfo_b_adaptive_floor_db_estimate,
                &mut state.vfo_b_adaptive_range_db_estimate,
            ),
        };

        *top_est = (1.0 - alpha) * *top_est + alpha * row_top_db;
        *floor_est = (1.0 - alpha) * *floor_est + alpha * row_floor_db;
        let adaptive_display_top_db = *top_est + ADAPTIVE_TOP_HEADROOM_DB;
        *range_est = (adaptive_display_top_db - *floor_est)
            .clamp(ADAPTIVE_MIN_RANGE_DB, ADAPTIVE_MAX_RANGE_DB);
    }
}

fn handle_waterfall_packet(
    reasm: &mut WaterfallReassembler,
    payload: &[u8],
    waterfall_buffer: &Arc<Mutex<Vec<u32>>>,
    spectrum_db: &Arc<Mutex<Vec<f32>>>,
    ui_state: &Arc<Mutex<UiState>>,
    vfo: VfoSelect,
) {
    // Accumulate this chunk; only proceed once the full row has been reassembled.
    let Some(row_db) = reasm.push_chunk(payload) else {
        return;
    };

    // Update smoothed spectrum trace.
    if let Ok(mut spectrum) = spectrum_db.lock() {
        update_spectrum_db(&mut spectrum, row_db);
    }

    // If adaptive mode is enabled, update this VFO's display controls from
    // the incoming spectral row using slow smoothing.
    update_adaptive_waterfall_display(row_db, ui_state, vfo);

    // Read this VFO's current display mapping controls.
    let (top_db, range_db, zoom) = if let Ok(state) = ui_state.lock() {
        let (adaptive, top_est, range_est, manual_top, manual_range, zoom) = match vfo {
            VfoSelect::A => (
                state.adaptive_waterfall_normalization,
                state.adaptive_top_db_estimate,
                state.adaptive_range_db_estimate,
                state.manual_waterfall_top_db,
                state.manual_waterfall_range_db,
                state.display_zoom,
            ),
            VfoSelect::B => (
                state.vfo_b_adaptive_waterfall_normalization,
                state.vfo_b_adaptive_top_db_estimate,
                state.vfo_b_adaptive_range_db_estimate,
                state.vfo_b_manual_waterfall_top_db,
                state.vfo_b_manual_waterfall_range_db,
                state.vfo_b_display_zoom,
            ),
        };
        if adaptive {
            (top_est + ADAPTIVE_TOP_HEADROOM_DB, range_est, zoom)
        } else {
            (manual_top, manual_range, zoom)
        }
    } else {
        (-35.0, 70.0, 1.0)
    };

    // Update waterfall image buffer using client-side dB mapping.
    if let Ok(mut fb) = waterfall_buffer.lock() {
        draw_row_db(
            &mut fb,
            WATERFALL_IMAGE_WIDTH,
            WATERFALL_IMAGE_HEIGHT,
            row_db,
            top_db,
            range_db,
            zoom,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a chunk payload (the bytes that follow the 16-byte media header).
    fn chunk(row_seq: u16, total_bins: u16, bin_offset: u16, bins: &[f32]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&row_seq.to_be_bytes());
        p.extend_from_slice(&total_bins.to_be_bytes());
        p.extend_from_slice(&bin_offset.to_be_bytes());
        for b in bins {
            p.extend_from_slice(&b.to_le_bytes());
        }
        p
    }

    #[test]
    fn reassembles_full_row_from_chunks() {
        let mut r = WaterfallReassembler::new();
        let row: Vec<f32> = (0..1024).map(|i| i as f32).collect();

        assert!(r.push_chunk(&chunk(0, 1024, 0, &row[0..256])).is_none());
        assert!(r.push_chunk(&chunk(0, 1024, 256, &row[256..512])).is_none());
        assert!(r.push_chunk(&chunk(0, 1024, 512, &row[512..768])).is_none());
        assert_eq!(
            r.push_chunk(&chunk(0, 1024, 768, &row[768..1024])),
            Some(row.as_slice())
        );
    }

    #[test]
    fn incomplete_row_is_dropped_when_next_row_starts() {
        let mut r = WaterfallReassembler::new();
        let row: Vec<f32> = vec![1.0; 1024];

        // Only 3 of 4 chunks for row 0 → never completes.
        assert!(r.push_chunk(&chunk(0, 1024, 0, &row[0..256])).is_none());
        assert!(r.push_chunk(&chunk(0, 1024, 256, &row[256..512])).is_none());
        assert!(r.push_chunk(&chunk(0, 1024, 512, &row[512..768])).is_none());

        // A new row id starts: the incomplete row 0 is silently dropped.
        assert!(r.push_chunk(&chunk(1, 1024, 0, &row[0..256])).is_none());
    }

    #[test]
    fn rejects_out_of_bounds_chunk() {
        let mut r = WaterfallReassembler::new();
        // bin_offset + n exceeds total_bins → dropped.
        assert!(r.push_chunk(&chunk(0, 4, 2, &[0.0; 4])).is_none());
    }
}
