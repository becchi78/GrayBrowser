//! Image size from the file header, without decoding or spawning a process.

const MARKER_PREFIX: u8 = 0xFF;
const SOI: u8 = 0xD8;
const EOI: u8 = 0xD9;
const SOS: u8 = 0xDA;
/// Markers in the SOF range that are not frame headers: DHT, JPG (reserved)
/// and DAC.
const NOT_SOF: [u8; 3] = [0xC4, 0xC8, 0xCC];
/// SOF segment up to the width: length (2) + precision (1) + height (2) + width (2).
const SOF_MIN_LEN: usize = 7;

/// Width and height of a JPEG, read from its frame header (SOF0-SOF15, so
/// baseline and progressive alike). Only the leading bytes are needed: the
/// frame header comes before the entropy-coded data, after any APPn/EXIF
/// segments.
///
/// `None` when `bytes` is not a JPEG, ends before a frame header, is
/// corrupt, or reports a zero dimension. A caller must treat that as "size
/// unknown", never as a size of zero.
pub fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != MARKER_PREFIX || bytes[1] != SOI {
        return None;
    }
    let mut pos = 2;
    loop {
        if *bytes.get(pos)? != MARKER_PREFIX {
            return None;
        }
        // Any number of 0xFF fill bytes may precede the marker code.
        while *bytes.get(pos)? == MARKER_PREFIX {
            pos += 1;
        }
        let marker = *bytes.get(pos)?;
        pos += 1;
        match marker {
            // Stuffed zero, TEM, SOI and RSTn carry no length.
            0x00 | 0x01 | SOI | 0xD0..=0xD7 => continue,
            // No frame header before the end of the image or the scan data.
            EOI | SOS => return None,
            _ => {}
        }
        let len = u16::from_be_bytes([*bytes.get(pos)?, *bytes.get(pos + 1)?]) as usize;
        if len < 2 {
            return None;
        }
        if (0xC0..=0xCF).contains(&marker) && !NOT_SOF.contains(&marker) {
            if len < SOF_MIN_LEN {
                return None;
            }
            let height = u16::from_be_bytes([*bytes.get(pos + 3)?, *bytes.get(pos + 4)?]);
            let width = u16::from_be_bytes([*bytes.get(pos + 5)?, *bytes.get(pos + 6)?]);
            if width == 0 || height == 0 {
                return None;
            }
            return Some((u32::from(width), u32::from(height)));
        }
        pos = pos.checked_add(len)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(marker: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![0xFF, marker];
        v.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        v.extend_from_slice(payload);
        v
    }

    fn sof_payload(w: u16, h: u16) -> Vec<u8> {
        let mut p = vec![8];
        p.extend_from_slice(&h.to_be_bytes());
        p.extend_from_slice(&w.to_be_bytes());
        p.extend_from_slice(&[1, 1, 0x11, 0]);
        p
    }

    fn jpeg(sof_marker: u8, w: u16, h: u16, before_sof: &[Vec<u8>]) -> Vec<u8> {
        let mut v = vec![0xFF, SOI];
        for s in before_sof {
            v.extend_from_slice(s);
        }
        v.extend(segment(sof_marker, &sof_payload(w, h)));
        v.extend(segment(SOS, &[1, 1, 0, 0, 63, 0]));
        v.extend_from_slice(&[0x12, 0x34, 0xFF, 0xD9]);
        v
    }

    #[test]
    fn reads_baseline_dimensions() {
        assert_eq!(
            jpeg_dimensions(&jpeg(0xC0, 480, 240, &[])),
            Some((480, 240))
        );
        assert_eq!(jpeg_dimensions(&jpeg(0xC0, 1, 1, &[])), Some((1, 1)));
        assert_eq!(
            jpeg_dimensions(&jpeg(0xC0, 65535, 65535, &[])),
            Some((65535, 65535))
        );
    }

    #[test]
    fn width_and_height_are_not_swapped() {
        assert_eq!(jpeg_dimensions(&jpeg(0xC0, 640, 90, &[])), Some((640, 90)));
    }

    #[test]
    fn reads_progressive_and_other_sof_kinds() {
        for marker in [0xC1, 0xC2, 0xC3, 0xC5, 0xCA, 0xCF] {
            assert_eq!(
                jpeg_dimensions(&jpeg(marker, 800, 600, &[])),
                Some((800, 600)),
                "marker {marker:#x}"
            );
        }
    }

    #[test]
    fn skips_long_app_and_table_segments_before_the_frame_header() {
        let exif = segment(0xE1, &vec![0xAB; 40_000]);
        let jfif = segment(0xE0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        let dqt = segment(0xDB, &[0; 65]);
        let dht = segment(0xC4, &[0; 20]);
        assert_eq!(
            jpeg_dimensions(&jpeg(0xC0, 320, 200, &[jfif, exif, dqt, dht])),
            Some((320, 200))
        );
    }

    #[test]
    fn a_dht_marker_is_not_taken_for_a_frame_header() {
        let mut v = vec![0xFF, SOI];
        v.extend(segment(0xC4, &sof_payload(10, 20)));
        assert_eq!(jpeg_dimensions(&v), None);
    }

    #[test]
    fn tolerates_fill_bytes_before_a_marker() {
        let mut v = vec![0xFF, SOI, 0xFF, 0xFF, 0xFF];
        v.extend(segment(0xC0, &sof_payload(100, 50))[1..].iter());
        assert_eq!(jpeg_dimensions(&v), Some((100, 50)));
    }

    #[test]
    fn rejects_non_jpeg_input() {
        let png = [
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R',
        ];
        assert_eq!(jpeg_dimensions(&png), None);
        assert_eq!(jpeg_dimensions(b"GIF89a......"), None);
    }

    #[test]
    fn rejects_empty_and_tiny_input() {
        assert_eq!(jpeg_dimensions(&[]), None);
        assert_eq!(jpeg_dimensions(&[0xFF]), None);
        assert_eq!(jpeg_dimensions(&[0xFF, 0xD8]), None);
        assert_eq!(jpeg_dimensions(&[0xFF, 0xD8, 0xFF]), None);
    }

    #[test]
    fn rejects_input_that_ends_before_the_frame_header() {
        let full = jpeg(0xC0, 480, 240, &[]);
        // SOI (2) + marker (2) + the first 7 bytes of the SOF segment hold the size; any
        // shorter prefix must be None, not a panic.
        let complete = 2 + 2 + SOF_MIN_LEN;
        for cut in 0..complete {
            assert_eq!(jpeg_dimensions(&full[..cut]), None, "cut at {cut}");
        }
        assert_eq!(jpeg_dimensions(&full[..complete]), Some((480, 240)));
    }

    #[test]
    fn rejects_scan_or_end_before_any_frame_header() {
        let mut v = vec![0xFF, SOI];
        v.extend(segment(SOS, &[0; 4]));
        assert_eq!(jpeg_dimensions(&v), None);
        assert_eq!(jpeg_dimensions(&[0xFF, SOI, 0xFF, EOI, 0, 0]), None);
    }

    #[test]
    fn rejects_bad_segment_lengths() {
        // Length 0 and 1 are below the 2-byte length field itself.
        assert_eq!(jpeg_dimensions(&[0xFF, SOI, 0xFF, 0xE0, 0, 0, 0, 0]), None);
        assert_eq!(jpeg_dimensions(&[0xFF, SOI, 0xFF, 0xE0, 0, 1, 0, 0]), None);
        // Length reaching past the end.
        assert_eq!(
            jpeg_dimensions(&[0xFF, SOI, 0xFF, 0xE0, 0xFF, 0xFF, 1, 2, 3]),
            None
        );
        // SOF too short to hold the dimensions.
        assert_eq!(
            jpeg_dimensions(&[0xFF, SOI, 0xFF, 0xC0, 0, 5, 8, 0, 1, 0, 1]),
            None
        );
    }

    #[test]
    fn rejects_a_zero_dimension() {
        assert_eq!(jpeg_dimensions(&jpeg(0xC0, 0, 240, &[])), None);
        assert_eq!(jpeg_dimensions(&jpeg(0xC0, 480, 0, &[])), None);
    }

    #[test]
    fn rejects_garbage_where_a_marker_should_be() {
        assert_eq!(jpeg_dimensions(&[0xFF, SOI, 0x12, 0x34, 0x56, 0x78]), None);
    }
}
