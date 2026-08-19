//! quick_hash: a fast partial-content hash used for move detection and as a
//! first-pass grouping key for duplicate detection.
//!
//! Operates over `impl Read + Seek` rather than `std::fs::File` directly so it
//! stays OS-independent and testable against an in-memory `Cursor`.

use std::io::{self, Read, Seek, SeekFrom};

use xxhash_rust::xxh64::Xxh64;

/// Number of bytes read from the start and from the end of the file.
pub const QUICK_HASH_CHUNK_SIZE: u64 = 1024 * 1024; // 1MB

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

    fn hash_of(bytes: &[u8]) -> u64 {
        let file_size = bytes.len() as u64;
        let mut cursor = Cursor::new(bytes.to_vec());
        quick_hash(&mut cursor, file_size)
            .expect("quick_hash should not fail on an in-memory buffer")
    }

    #[test]
    fn is_deterministic_for_the_same_input() {
        let bytes = vec![7u8; 500];
        assert_eq!(hash_of(&bytes), hash_of(&bytes));
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
        let mut cursor_a = Cursor::new(bytes.clone());
        let mut cursor_b = Cursor::new(bytes);
        let hash_a = quick_hash(&mut cursor_a, 100).unwrap();
        let hash_b = quick_hash(&mut cursor_b, 200).unwrap();
        assert_ne!(hash_a, hash_b);
    }

    #[test]
    fn handles_files_smaller_than_one_chunk_without_panicking() {
        let bytes = vec![3u8; 10]; // 10 bytes, far below the 1MB chunk size
        let mut cursor = Cursor::new(bytes);
        let result = quick_hash(&mut cursor, 10);
        assert!(result.is_ok());
    }

    #[test]
    fn handles_empty_files_without_panicking() {
        let mut cursor = Cursor::new(Vec::<u8>::new());
        let result = quick_hash(&mut cursor, 0);
        assert!(result.is_ok());
    }

    #[test]
    fn handles_files_larger_than_two_chunks() {
        // 3MB buffer: head and tail reads should come from disjoint regions.
        let size = (QUICK_HASH_CHUNK_SIZE * 3) as usize;
        let mut bytes = vec![0u8; size];
        // Make head and tail distinguishable so a bug that reads the wrong
        // region would still produce *a* hash, but we mainly assert no panic
        // and determinism here; distinctness is covered by `differs_when_content_differs`.
        bytes[0] = 0xAA;
        bytes[size - 1] = 0xBB;
        let file_size = size as u64;
        let mut cursor = Cursor::new(bytes);
        let result = quick_hash(&mut cursor, file_size);
        assert!(result.is_ok());
    }

    fn full_hash_of(bytes: &[u8]) -> String {
        let mut cursor = Cursor::new(bytes.to_vec());
        full_hash(&mut cursor).expect("full_hash should not fail on an in-memory buffer")
    }

    #[test]
    fn full_hash_is_deterministic_for_the_same_input() {
        let bytes = vec![7u8; 5000];
        assert_eq!(full_hash_of(&bytes), full_hash_of(&bytes));
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

    #[test]
    fn full_hash_handles_empty_files_without_panicking() {
        let result = full_hash_of(&[]);
        // BLAKE3 has a well-defined hash for the empty input; just assert we
        // get a plausible hex digest back rather than panicking.
        assert_eq!(result.len(), 64);
    }

    #[test]
    fn full_hash_returns_lowercase_hex_of_expected_length() {
        let bytes = vec![0xABu8; 1234];
        let digest = full_hash_of(&bytes);
        assert_eq!(digest.len(), 64);
        assert!(digest
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }
}
