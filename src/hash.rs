//! FNV-1a 64-bit content hash over raw bytes (no normalization).
//! Staleness contract: stat short-circuit on (mtime, size); hash only on mismatch.

pub const FNV_OFFSET: u64 = 0xcbf29ce484222325;
pub const FNV_PRIME: u64 = 0x100000001b3;

pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    // Eight independent XOR/multiply steps per iteration: the dependency
    // chain stays exact FNV-1a while the loop overhead shrinks on 1 MiB files.
    let mut chunks = bytes.chunks_exact(8);
    for chunk in &mut chunks {
        macro_rules! step {
            ($i:expr) => {{
                hash ^= u64::from(chunk[$i]);
                hash = hash.wrapping_mul(FNV_PRIME);
            }};
        }
        step!(0);
        step!(1);
        step!(2);
        step!(3);
        step!(4);
        step!(5);
        step!(6);
        step!(7);
    }
    for &b in chunks.remainder() {
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
