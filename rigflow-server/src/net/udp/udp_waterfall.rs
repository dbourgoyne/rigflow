use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;

use rigflow_core::net::udp_framing::{MAGIC, STREAM_TYPE_WATERFALL, VERSION};

/// Bins per waterfall chunk. Sized so a chunk packet stays well under the typical
/// 1500-byte MTU: 256 × 4 (f32) + 16 (media header) + 6 (chunk header) = 1046 bytes.
/// Keeping each datagram under the MTU avoids IP fragmentation, where losing one
/// fragment would drop the whole row (the cause of waterfall stutter on WiFi).
pub const WATERFALL_BINS_PER_CHUNK: usize = 256;

/// Sends waterfall rows over UDP, split into sub-MTU chunks reassembled by the client.
///
/// Chunk packet layout:
/// - 16-byte media header (magic, version, stream_type = waterfall, sequence, timestamp)
/// - u16 row_seq    (big-endian) — same for every chunk of one row
/// - u16 total_bins (big-endian) — bins in the full row
/// - u16 bin_offset (big-endian) — index of this chunk's first bin in the row
/// - f32 bins (little-endian) — this chunk's bins (count inferred from datagram length)
pub struct UdpWaterfallSender {
    socket: Arc<UdpSocket>,
    /// Per-packet (per-chunk) sequence — drives the client's chunk-level loss stats.
    sequence: u32,
    timestamp: u64,
    /// Per-row identifier — groups a row's chunks for reassembly.
    row_seq: u16,
}

impl UdpWaterfallSender {
    /// `socket` is the shared server socket bound to the registration port, so
    /// waterfall rows egress from the same 5-tuple the client registered against.
    pub fn new(socket: Arc<UdpSocket>) -> Self {
        Self {
            socket,
            sequence: 0,
            timestamp: 0,
            row_seq: 0,
        }
    }

    /// Send one waterfall row as one or more sub-MTU chunks.
    pub fn send_row_db_to(&mut self, target: SocketAddr, row_db: &[f32]) {
        let total_bins = row_db.len();
        if total_bins == 0 || total_bins > u16::MAX as usize {
            return;
        }

        let row_seq = self.row_seq;
        self.row_seq = self.row_seq.wrapping_add(1);

        for (chunk_index, chunk) in row_db.chunks(WATERFALL_BINS_PER_CHUNK).enumerate() {
            let bin_offset = chunk_index * WATERFALL_BINS_PER_CHUNK;

            let mut buf = Vec::with_capacity(16 + 6 + chunk.len() * 4);

            // Media header.
            buf.extend_from_slice(&MAGIC.to_be_bytes());
            buf.push(VERSION);
            buf.push(STREAM_TYPE_WATERFALL);
            buf.extend_from_slice(&self.sequence.to_be_bytes());
            buf.extend_from_slice(&self.timestamp.to_be_bytes());

            // Chunk header.
            buf.extend_from_slice(&row_seq.to_be_bytes());
            buf.extend_from_slice(&(total_bins as u16).to_be_bytes());
            buf.extend_from_slice(&(bin_offset as u16).to_be_bytes());

            // Bin payload.
            for value in chunk {
                buf.extend_from_slice(&value.to_le_bytes());
            }

            let _ = self.socket.send_to(&buf, target);

            self.sequence = self.sequence.wrapping_add(1);
            self.timestamp = self.timestamp.wrapping_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rigflow_core::net::udp_framing::{is_valid_header, parse_media_header};

    /// Build the chunk packets the sender would emit (framing only, no socket).
    fn split_into_chunks(row_db: &[f32]) -> Vec<Vec<u8>> {
        let total_bins = row_db.len();
        let mut packets = Vec::new();
        for (i, chunk) in row_db.chunks(WATERFALL_BINS_PER_CHUNK).enumerate() {
            let bin_offset = i * WATERFALL_BINS_PER_CHUNK;
            let mut buf = Vec::new();
            buf.extend_from_slice(&MAGIC.to_be_bytes());
            buf.push(VERSION);
            buf.push(STREAM_TYPE_WATERFALL);
            buf.extend_from_slice(&0u32.to_be_bytes());
            buf.extend_from_slice(&0u64.to_be_bytes());
            buf.extend_from_slice(&0u16.to_be_bytes()); // row_seq
            buf.extend_from_slice(&(total_bins as u16).to_be_bytes());
            buf.extend_from_slice(&(bin_offset as u16).to_be_bytes());
            for v in chunk {
                buf.extend_from_slice(&v.to_le_bytes());
            }
            packets.push(buf);
        }
        packets
    }

    #[test]
    fn row_splits_into_submtu_chunks_and_round_trips() {
        let row: Vec<f32> = (0..1024).map(|i| i as f32 * 0.5 - 100.0).collect();
        let packets = split_into_chunks(&row);

        // 1024 bins / 256 per chunk = 4 chunks, each comfortably under the MTU.
        assert_eq!(packets.len(), 4);
        for p in &packets {
            assert!(p.len() <= 1200, "chunk packet too large: {}", p.len());
            let h = parse_media_header(p).unwrap();
            assert!(is_valid_header(&h));
            assert_eq!(h.stream_type, STREAM_TYPE_WATERFALL);
        }

        // Reassemble in bin_offset order → original row.
        let mut reassembled = vec![0f32; row.len()];
        for p in &packets {
            let bin_offset = u16::from_be_bytes([p[16 + 4], p[16 + 5]]) as usize;
            let data = &p[16 + 6..];
            for (j, c) in data.chunks_exact(4).enumerate() {
                reassembled[bin_offset + j] = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            }
        }
        assert_eq!(reassembled, row);
    }
}
