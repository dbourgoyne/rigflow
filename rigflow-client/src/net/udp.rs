use std::sync::{Arc, Mutex};

use rigflow_core::{
    audio::jitter_buffer::JitterBuffer,
    net::udp_framing::{
        is_valid_header, parse_media_header, STREAM_TYPE_AUDIO, STREAM_TYPE_WATERFALL,
    },
};

use crate::{
    app::state::UiState,
    render::spectrum::update_spectrum_db,
    render::waterfall::draw_row,
};

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
pub fn handle_media_packet(
    packet: &[u8],
    jitter: &Arc<Mutex<JitterBuffer>>,
    waterfall_buffer: &Arc<Mutex<Vec<u32>>>,
    spectrum_db: &Arc<Mutex<Vec<f32>>>,
    ui_state: &Arc<Mutex<UiState>>,
    stats: &Arc<Mutex<MediaPacketStats>>,
    width: usize,
    height: usize,
    waterfall_top: usize,
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
	    println!("UDP AUDIO packet seq={}", header.sequence);
            if let Ok(mut s) = stats.lock() {
                s.audio_packets += 1;
                update_sequence_stats(&mut s, StreamKind::Audio, header.sequence);
            }

            handle_audio_packet(payload, header.sequence, jitter);
        }

        STREAM_TYPE_WATERFALL => {
	    println!("UDP WATERFALL packet seq={} payload_len={}", header.sequence, payload.len());
            if let Ok(mut s) = stats.lock() {
                s.waterfall_packets += 1;
                update_sequence_stats(&mut s, StreamKind::Waterfall, header.sequence);
            }

            handle_waterfall_packet(
                payload,
                waterfall_buffer,
                spectrum_db,
                ui_state,
                width,
                height,
                waterfall_top,
            );
        }

        _ => {}
    }
}

fn update_sequence_stats(
    stats: &mut MediaPacketStats,
    kind: StreamKind,
    sequence: u32,
) {
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
) {
    if payload.len() < 2 || !payload.len().is_multiple_of(2) {
        return;
    }

    let mut samples = Vec::with_capacity(payload.len() / 2);
    for chunk in payload.chunks_exact(2) {
        let s = i16::from_le_bytes([chunk[0], chunk[1]]);
        samples.push(s as f32 / i16::MAX as f32);
    }

    if let Ok(mut jb) = jitter.lock() {
        jb.push_packet(sequence, samples);
    }
}

fn handle_waterfall_packet(
    payload: &[u8],
    waterfall_buffer: &Arc<Mutex<Vec<u32>>>,
    spectrum_db: &Arc<Mutex<Vec<f32>>>,
    ui_state: &Arc<Mutex<UiState>>,
    width: usize,
    height: usize,
    waterfall_top: usize,
) {
    println!(
	"handle_waterfall_packet: payload_len={} width={} height={} waterfall_top={}",
	payload.len(),
	width,
	height,
	waterfall_top
    );
    if payload.is_empty() {
        return;
    }

    let row = payload;

    if let Ok(mut spectrum) = spectrum_db.lock() {
        update_spectrum_db(&mut spectrum, row);
    }

    let state_snapshot = match ui_state.lock() {
        Ok(state) => state.clone(),
        Err(_) => return,
    };

    println!("handle_waterfall_packet: waterfall_bins={}", state_snapshot.waterfall_bins);
    if state_snapshot.waterfall_bins == 0 {
	println!("handle_waterfall_packet: early return because waterfall_bins == 0");
        return;
    }

    if let Ok(mut fb) = waterfall_buffer.lock() {
        draw_row(
            &mut fb,
            row,
            width,
            height,
            waterfall_top,
            &state_snapshot,
        );
    }
}
