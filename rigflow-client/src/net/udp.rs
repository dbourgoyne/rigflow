use std::sync::{Arc, Mutex};

use rigflow_core::{
    audio::jitter_buffer::JitterBuffer,
    net::udp_framing::{
        parse_media_header, is_valid_header,
        STREAM_TYPE_AUDIO, STREAM_TYPE_WATERFALL,
    },
};

use crate::{
    render::spectrum::update_spectrum_db,
    render::waterfall::draw_row,
    UiState,
};

#[allow(clippy::too_many_arguments)]
pub fn handle_media_packet(
    packet: &[u8],
    jitter: &Arc<Mutex<JitterBuffer>>,
    waterfall_buffer: &Arc<Mutex<Vec<u32>>>,
    spectrum_db: &Arc<Mutex<Vec<f32>>>,
    ui_state: &Arc<Mutex<UiState>>,
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

    match header.stream_type {
        STREAM_TYPE_AUDIO => {
            handle_audio_packet(payload, header.sequence, jitter);
        }

        STREAM_TYPE_WATERFALL => {
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
    if payload.is_empty() {
        return;
    }

    let row = payload;

    {
        if let Ok(mut spectrum) = spectrum_db.lock() {
            update_spectrum_db(&mut spectrum, row);
        }
    }

    let bins = {
        if let Ok(state) = ui_state.lock() {
            state.waterfall_bins
        } else {
            0
        }
    };

    if bins == 0 {
        return;
    }

    if let Ok(mut fb) = waterfall_buffer.lock() {
        draw_row(
            &mut fb,
            row,
            width,
            height,
            waterfall_top,
        );
    }
}
