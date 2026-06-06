use std::sync::{Arc, Mutex};

use rigflow_core::{
    audio::jitter_buffer::JitterBuffer,
    net::udp_framing::{
        is_valid_header, parse_media_header, STREAM_TYPE_AUDIO, STREAM_TYPE_WATERFALL,
    },
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

#[allow(clippy::too_many_arguments)]
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
) {
    let Some(header) = parse_media_header(packet) else {
        return;
    };

    if !is_valid_header(&header) {
        return;
    }

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

            handle_audio_packet(payload, header.sequence, jitter, cw_decoder, digital_rx);
        }

        STREAM_TYPE_WATERFALL => {
            if let Ok(mut s) = stats.lock() {
                s.waterfall_packets += 1;
                update_sequence_stats(&mut s, StreamKind::Waterfall, header.sequence);
            }

            handle_waterfall_packet(payload, waterfall_buffer, spectrum_db, ui_state);
        }

        _ => {}
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

    if let Ok(mut jb) = jitter.lock() {
        jb.push_packet(sequence, samples);
    }
}

fn update_adaptive_waterfall_display(row_db: &[f32], ui_state: &Arc<Mutex<UiState>>) {
    let Some((row_floor_db, row_top_db)) = estimate_row_floor_and_top_db(row_db) else {
        return;
    };

    if let Ok(mut state) = ui_state.lock() {
        if !state.adaptive_waterfall_normalization {
            return;
        }

        let alpha = ADAPTIVE_NORMALIZATION_ALPHA;

        state.adaptive_top_db_estimate =
            (1.0 - alpha) * state.adaptive_top_db_estimate + alpha * row_top_db;

        state.adaptive_floor_db_estimate =
            (1.0 - alpha) * state.adaptive_floor_db_estimate + alpha * row_floor_db;

        let adaptive_display_top_db = state.adaptive_top_db_estimate + ADAPTIVE_TOP_HEADROOM_DB;

        state.adaptive_range_db_estimate = (adaptive_display_top_db
            - state.adaptive_floor_db_estimate)
            .clamp(ADAPTIVE_MIN_RANGE_DB, ADAPTIVE_MAX_RANGE_DB);
    }
}

fn handle_waterfall_packet(
    payload_with_len: &[u8],
    waterfall_buffer: &Arc<Mutex<Vec<u32>>>,
    spectrum_db: &Arc<Mutex<Vec<f32>>>,
    ui_state: &Arc<Mutex<UiState>>,
) {
    if payload_with_len.len() < 2 {
        return;
    }

    // First 2 bytes are the waterfall payload length (big-endian u16).
    let payload_len = u16::from_be_bytes([payload_with_len[0], payload_with_len[1]]) as usize;

    let payload = &payload_with_len[2..];

    if payload.len() != payload_len {
        return;
    }

    // Each spectral bin is one little-endian f32.
    if !payload.len().is_multiple_of(4) {
        return;
    }

    let mut row_db = Vec::with_capacity(payload.len() / 4);

    for chunk in payload.chunks_exact(4) {
        row_db.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }

    if !row_db.iter().all(|v| v.is_finite()) {
        return;
    }

    // Update smoothed spectrum trace.
    if let Ok(mut spectrum) = spectrum_db.lock() {
        update_spectrum_db(&mut spectrum, &row_db);
    }

    // If adaptive mode is enabled, update the display controls from
    // the incoming spectral row using slow smoothing.
    update_adaptive_waterfall_display(&row_db, ui_state);

    // Read current display mapping controls.
    let (top_db, range_db, zoom) = if let Ok(state) = ui_state.lock() {
        if state.adaptive_waterfall_normalization {
            (
                state.adaptive_top_db_estimate + ADAPTIVE_TOP_HEADROOM_DB,
                state.adaptive_range_db_estimate,
                state.display_zoom,
            )
        } else {
            (
                state.manual_waterfall_top_db,
                state.manual_waterfall_range_db,
                state.display_zoom,
            )
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
            &row_db,
            top_db,
            range_db,
            zoom,
        );
    }
}
