//! VBAN wire format for the fixed send profile: PCM Int16 LE, 48 kHz, stereo.
//!
//! Header layout (28 bytes, little-endian fields), verified against the
//! VB-Audio VBAN protocol and interoperable implementations:
//!
//! | offset | size | field |
//! |--------|------|-------|
//! | 0..4   | 4    | magic `"VBAN"` |
//! | 4      | 1    | bits 0-4: sample rate index (48 kHz = 3), bits 5-7: protocol (audio = 0) |
//! | 5      | 1    | samples per packet - 1 (1..=256) |
//! | 6      | 1    | channels - 1 (stereo = 1) |
//! | 7      | 1    | bit format (1 = signed 16-bit integer PCM) |
//! | 8..24  | 16   | stream name, zero padded |
//! | 24..28 | 4    | `nuFrame`: growing per-stream packet counter (u32 LE) |

/// Fixed stream profile: everything sent through VBAN is converted to this.
pub(crate) const SAMPLE_RATE: u32 = 48_000;
pub(crate) const CHANNELS: usize = 2;
/// Maximum stereo frames carried by one UDP packet (1024-byte payload).
pub(crate) const MAX_PACKET_FRAMES: usize = 256;
pub(crate) const HEADER_SIZE: usize = 28;
pub(crate) const MAX_PACKET_SIZE: usize = HEADER_SIZE + MAX_PACKET_FRAMES * CHANNELS * 2;

const STREAM_NAME_SIZE: usize = 16;
const SAMPLE_RATE_INDEX_48K: u8 = 3;
const BIT_FORMAT_I16: u8 = 1;
const SAMPLE_RATE_INDEX_MASK: u8 = 0x1F;

/// Total wire size of a packet carrying `frames` stereo frames.
#[cfg(test)]
pub(crate) fn packet_size(frames: usize) -> usize {
    HEADER_SIZE + frames * CHANNELS * 2
}

/// Wall-clock duration of `frames` stereo frames at the stream sample rate.
pub(crate) fn frame_duration(frames: usize) -> std::time::Duration {
    std::time::Duration::from_nanos((frames as u64 * 1_000_000_000) / u64::from(SAMPLE_RATE))
}

/// Encode a VBAN header for the fixed profile into `buf`.
///
/// `samples_per_packet` is the actual frame count of this packet, which may be
/// smaller than [`MAX_PACKET_FRAMES`] for the final packet of a stream burst.
pub(crate) fn encode_header(
    buf: &mut [u8],
    stream_name: &str,
    samples_per_packet: usize,
    nu_frame: u32,
) {
    debug_assert!(buf.len() >= HEADER_SIZE);
    debug_assert!((1..=MAX_PACKET_FRAMES).contains(&samples_per_packet));
    buf[..HEADER_SIZE].fill(0);
    buf[0..4].copy_from_slice(b"VBAN");
    // Audio protocol in the upper bits, 48 kHz index in the lower bits.
    buf[4] = SAMPLE_RATE_INDEX_48K & SAMPLE_RATE_INDEX_MASK;
    buf[5] = (samples_per_packet - 1) as u8;
    buf[6] = (CHANNELS - 1) as u8;
    buf[7] = BIT_FORMAT_I16;
    let name = stream_name.as_bytes();
    let copied = name.len().min(STREAM_NAME_SIZE);
    buf[8..8 + copied].copy_from_slice(&name[..copied]);
    buf[24..28].copy_from_slice(&nu_frame.to_le_bytes());
}

/// Quantize a mixed stereo block (source gains already applied to the sum) to
/// Int16 LE payload bytes written at the start of `out`. Returns the number of
/// payload bytes written: `mix.len() * 2`.
///
/// The stream gain is applied to the sum here, right before clipping and
/// rounding, per the output contract.
pub(crate) fn quantize_payload(mix: &[f32], stream_gain: f32, out: &mut [u8]) -> usize {
    let frames = mix.len() / CHANNELS;
    for frame in 0..frames {
        for channel in 0..CHANNELS {
            let sample = mix[frame * CHANNELS + channel] * stream_gain;
            let quantized = (sample.clamp(-1.0, 1.0) * 32_767.0).round() as i16;
            let offset = (frame * CHANNELS + channel) * 2;
            out[offset..offset + 2].copy_from_slice(&quantized.to_le_bytes());
        }
    }
    frames * CHANNELS * 2
}

/// Parsed header fields, used by loopback tests and diagnostics.
#[cfg(test)]
pub(crate) struct DecodedHeader {
    pub samples_per_packet: usize,
    pub channels: usize,
    pub sample_rate_index: u8,
    pub bit_format: u8,
    pub stream_name: String,
    pub nu_frame: u32,
}

/// Decode a VBAN header from `buf`; `None` when the magic does not match.
#[cfg(test)]
pub(crate) fn decode_header(buf: &[u8]) -> Option<DecodedHeader> {
    if buf.len() < HEADER_SIZE || &buf[0..4] != b"VBAN" {
        return None;
    }
    let name_end = buf[8..24]
        .iter()
        .position(|&b| b == 0)
        .map_or(24, |pos| 8 + pos);
    Some(DecodedHeader {
        samples_per_packet: usize::from(buf[5]) + 1,
        channels: usize::from(buf[6]) + 1,
        sample_rate_index: buf[4] & SAMPLE_RATE_INDEX_MASK,
        bit_format: buf[7],
        stream_name: String::from_utf8_lossy(&buf[8..name_end]).into_owned(),
        nu_frame: u32::from_le_bytes(buf[24..28].try_into().expect("nuFrame is 4 bytes")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_fixed_profile_header() {
        let mut buf = [0u8; MAX_PACKET_SIZE];
        encode_header(&mut buf, "TTSBard", MAX_PACKET_FRAMES, 42);
        assert_eq!(&buf[0..4], b"VBAN");
        assert_eq!(
            buf[4], 3,
            "48 kHz is sample rate index 3, audio protocol bits 0"
        );
        assert_eq!(buf[5], 255, "256 samples per packet encoded as value-1");
        assert_eq!(buf[6], 1, "stereo encoded as channels-1");
        assert_eq!(buf[7], 1, "16-bit integer PCM");
        let mut expected_name = [0u8; 16];
        expected_name[..7].copy_from_slice(b"TTSBard");
        assert_eq!(&buf[8..24], &expected_name);
        assert_eq!(u32::from_le_bytes(buf[24..28].try_into().unwrap()), 42);
        assert_eq!(packet_size(MAX_PACKET_FRAMES), 1052);
    }

    #[test]
    fn encodes_short_tail_header() {
        let mut buf = [0u8; MAX_PACKET_SIZE];
        encode_header(&mut buf, "TTSBard", 44, 7);
        assert_eq!(buf[5], 43);
        assert_eq!(packet_size(44), 28 + 44 * 4);
    }

    #[test]
    fn quantize_applies_gain_clips_and_rounds_to_le_int16() {
        // Two frames: one clipped high, one nominal, mix values chosen so the
        // gain moves them into view of the clamp and the rounding.
        let mix = [1.5, -1.5, 0.25, -0.25];
        let mut out = [0u8; 8];
        let written = quantize_payload(&mix, 2.0, &mut out);
        assert_eq!(written, 8);
        let samples: Vec<i16> = out
            .as_chunks::<2>()
            .0
            .iter()
            .map(|s| i16::from_le_bytes(*s))
            .collect();
        assert_eq!(samples, [32_767, -32_767, 16_384, -16_384]);
    }

    #[test]
    fn decode_round_trips_header() {
        let mut buf = [0u8; MAX_PACKET_SIZE];
        encode_header(&mut buf, "TTSBard", 200, 1_234_567);
        let decoded = decode_header(&buf).expect("valid header");
        assert_eq!(decoded.samples_per_packet, 200);
        assert_eq!(decoded.channels, 2);
        assert_eq!(decoded.sample_rate_index, 3);
        assert_eq!(decoded.bit_format, 1);
        assert_eq!(decoded.stream_name, "TTSBard");
        assert_eq!(decoded.nu_frame, 1_234_567);
    }

    #[test]
    fn decode_rejects_non_vban_magic() {
        let mut buf = [0u8; HEADER_SIZE];
        buf[0..4].copy_from_slice(b"RTPC");
        assert!(decode_header(&buf).is_none());
    }
}
