//! FNV-1a 64-bit content hash over raw bytes (no normalization).
//! Staleness contract: stat short-circuit on (mtime, size); hash only on mismatch.

pub const FNV_OFFSET: u64 = 0xcbf29ce484222325;
pub const FNV_PRIME: u64 = 0x100000001b3;

pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a64_known_vectors() {
        // Standard FNV-1a 64 test vectors.
        assert_eq!(fnv1a64(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a64(b"a"), 0xaf63dc4c8601ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x85944171f73967e8);
    }

    #[test]
    fn fnv1a64_raw_bytes_no_normalization() {
        assert_ne!(fnv1a64(b"Foo"), fnv1a64(b"foo"));
        assert_ne!(fnv1a64(b"a\n"), fnv1a64(b"a"));
    }
}
