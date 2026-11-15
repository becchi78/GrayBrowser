//! quick_hash: a fast partial-content hash used for move detection and as a
//! first-pass grouping key for duplicate detection.
//!
//! Operates over `impl Read + Seek` rather than `std::fs::File` directly so it
//! stays OS-independent and testable against an in-memory `Cursor`.

use std::io::{self, Read, Seek, SeekFrom};

use xxhash_rust::xxh64::Xxh64;

/// Number of bytes read from the start and from the end of the file.
pub(crate) const QUICK_HASH_CHUNK_SIZE: u64 = 1024 * 1024; // 1MB

const QUICK_HASH_SEED: u64 = 0;

/// Computes quick_hash over the first `QUICK_HASH_CHUNK_SIZE` bytes, the last
/// `QUICK_HASH_CHUNK_SIZE` bytes, and `file_size` itself.
///
/// For files smaller than `2 * QUICK_HASH_CHUNK_SIZE` the head and tail reads
/// overlap (or are identical) -- this is expected and harmless: the hash
/// still identifies the file's content plus its size.
pub fn quick_hash<R: Read + Seek>(reader: &mut R, file_size: u64) -> io::Result<u64> {
    let head = read_chunk(reader, SeekFrom::Start(0), QUICK_HASH_CHUNK_SIZE)?;

    let tail_start = file_size.saturating_sub(QUICK_HASH_CHUNK_SIZE);
    let tail = read_chunk(reader, SeekFrom::Start(tail_start), QUICK_HASH_CHUNK_SIZE)?;

    let mut hasher = Xxh64::new(QUICK_HASH_SEED);
    hasher.update(&head);
    hasher.update(&tail);
    hasher.update(&file_size.to_le_bytes());
    Ok(hasher.digest())
}

/// Computes full_hash (BLAKE3) over the entire contents of `reader`, returned
/// as a lowercase hex digest string.
///
/// Unlike `quick_hash`, this reads the whole stream sequentially and does not
/// require `Seek`. It is intended to be run only on candidate pairs that
/// already share a `quick_hash` + `file_size` match, not as an eager,
/// whole-library computation.
pub fn full_hash<R: Read>(reader: &mut R) -> io::Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Reads up to `max_len` bytes starting at `from`, stopping early at EOF.
fn read_chunk<R: Read + Seek>(reader: &mut R, from: SeekFrom, max_len: u64) -> io::Result<Vec<u8>> {
    reader.seek(from)?;
    let mut buf = vec![0u8; max_len as usize];
    let mut total = 0usize;
    loop {
        let n = reader.read(&mut buf[total..])?;
        if n == 0 {
            break;
        }
        total += n;
        if total == buf.len() {
            break;
        }
    }
    buf.truncate(total);
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use xxhash_rust::xxh64::xxh64;

    const CHUNK: usize = QUICK_HASH_CHUNK_SIZE as usize;

    fn hash_of(bytes: &[u8]) -> u64 {
        hash_with_size(bytes, bytes.len() as u64)
    }

    fn hash_with_size(bytes: &[u8], file_size: u64) -> u64 {
        let mut cursor = Cursor::new(bytes.to_vec());
        quick_hash(&mut cursor, file_size)
            .expect("quick_hash should not fail on an in-memory buffer")
    }

    /// The documented definition, built in one shot rather than streamed:
    /// head chunk, tail chunk, then the size as little-endian bytes.
    fn expected_quick_hash(head: &[u8], tail: &[u8], file_size: u64) -> u64 {
        let mut input = head.to_vec();
        input.extend_from_slice(tail);
        input.extend_from_slice(&file_size.to_le_bytes());
        xxh64(&input, 0)
    }

    /// 3MB of a non-repeating-looking pattern, so a read from the wrong
    /// region changes the hash.
    fn three_chunks() -> Vec<u8> {
        (0..CHUNK * 3).map(|i| (i % 251) as u8).collect()
    }

    fn hash_after_flipping(original: &[u8], index: usize) -> u64 {
        let mut changed = original.to_vec();
        changed[index] ^= 0xFF;
        hash_of(&changed)
    }

    #[test]
    fn a_small_file_hashes_its_content_twice_plus_its_size() {
        // head and tail both cover the whole file when it is under one chunk.
        let bytes: Vec<u8> = (0..10).collect();
        assert_eq!(
            hash_of(&bytes),
            expected_quick_hash(&bytes, &bytes, bytes.len() as u64)
        );
    }

    #[test]
    fn a_small_file_hash_is_pinned_to_a_known_value() {
        // Stored quick_hash values must stay comparable across versions: a
        // change here means every existing library row stops matching.
        let bytes: Vec<u8> = (0..10).collect();
        assert_eq!(hash_of(&bytes), 15_660_087_041_596_590_088);
    }

    #[test]
    fn differs_when_content_differs() {
        let a = vec![1u8; 500];
        let b = vec![2u8; 500];
        assert_ne!(hash_of(&a), hash_of(&b));
    }

    #[test]
    fn differs_when_only_file_size_argument_differs() {
        // Same bytes, but the caller reports a different file_size (e.g. a
        // truncated/extended file with identical head/tail content). The
        // hash must still change, since move-detection relies on file_size
        // being mixed in, not just head/tail bytes.
        let bytes = vec![9u8; 100];
        assert_ne!(
            hash_with_size(&bytes, 100),
            hash_with_size(&bytes, 200),
            "file_size must be mixed into the hash"
        );
    }

    #[test]
    fn an_empty_file_hashes_just_its_zero_size() {
        assert_eq!(hash_of(&[]), expected_quick_hash(&[], &[], 0));
    }

    #[test]
    fn a_file_larger_than_two_chunks_hashes_only_the_head_tail_and_size() {
        let bytes = three_chunks();
        let size = bytes.len() as u64;
        assert_eq!(
            hash_of(&bytes),
            expected_quick_hash(&bytes[..CHUNK], &bytes[bytes.len() - CHUNK..], size)
        );
    }

    #[test]
    fn bytes_in_the_middle_of_a_large_file_do_not_change_the_hash() {
        let bytes = three_chunks();
        let original = hash_of(&bytes);
        // First byte past the head chunk, the middle, and the last byte
        // before the tail chunk: none are read.
        for index in [CHUNK, CHUNK * 3 / 2, CHUNK * 2 - 1] {
            assert_eq!(hash_after_flipping(&bytes, index), original, "{index}");
        }
    }

    #[test]
    fn bytes_in_the_head_or_tail_of_a_large_file_change_the_hash() {
        let bytes = three_chunks();
        let original = hash_of(&bytes);
        // First and last byte of the head chunk; first and last byte of the
        // tail chunk (the file's final byte).
        for index in [0, CHUNK - 1, CHUNK * 2, CHUNK * 3 - 1] {
            assert_ne!(hash_after_flipping(&bytes, index), original, "{index}");
        }
    }

    fn full_hash_of(bytes: &[u8]) -> String {
        let mut cursor = Cursor::new(bytes.to_vec());
        full_hash(&mut cursor).expect("full_hash should not fail on an in-memory buffer")
    }

    #[test]
    fn full_hash_of_empty_input_is_the_known_blake3_digest() {
        assert_eq!(
            full_hash_of(&[]),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    #[test]
    fn full_hash_matches_one_shot_blake3_across_the_read_buffer_boundary() {
        // Larger than the 64KiB internal read buffer, so the streaming loop
        // runs several times and ends on a partial read.
        let bytes: Vec<u8> = (0..200_000).map(|i| (i % 251) as u8).collect();
        assert_eq!(full_hash_of(&bytes), blake3::hash(&bytes).to_hex().as_str());
    }

    #[test]
    fn full_hash_differs_when_content_differs() {
        let a = vec![1u8; 5000];
        let b = vec![2u8; 5000];
        assert_ne!(full_hash_of(&a), full_hash_of(&b));
    }

    #[test]
    fn full_hash_differs_when_only_a_middle_byte_differs() {
        // quick_hash reads head+tail chunks only; full_hash must be sensitive
        // to changes anywhere in the file, including the middle, to serve as
        // the confirmatory hash for duplicate detection.
        let a = vec![5u8; 200_000];
        let mut b = a.clone();
        b[100_000] = 6u8;
        assert_ne!(full_hash_of(&a), full_hash_of(&b));
    }
}
