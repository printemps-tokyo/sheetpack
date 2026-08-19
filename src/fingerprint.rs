//! The input fingerprint behind `--check`.
//!
//! A pack run hashes every source file's name and bytes plus the settings
//! line into one FNV-1a 64-bit value and writes it into the manifest's meta
//! block. `--check` recomputes the same value later; a mismatch means some
//! source changed, appeared or disappeared after the committed atlas was
//! built. FNV is not cryptographic and does not need to be here -- the
//! adversary is forgetfulness, not forgery.

/// Incremental FNV-1a, 64-bit.
#[derive(Debug, Clone, Copy)]
pub struct Fnv1a64(u64);

impl Fnv1a64 {
    /// The standard FNV-1a offset basis.
    pub fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    /// Fold bytes into the state.
    pub fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    /// Current value.
    pub fn finish(&self) -> u64 {
        self.0
    }
}

impl Default for Fnv1a64 {
    fn default() -> Self {
        Self::new()
    }
}

/// Hash one sprite source into an ongoing fingerprint.
///
/// Name and content are length-prefixed so `("ab", "c")` and `("a", "bc")`
/// cannot collide by concatenation.
pub fn add_source(f: &mut Fnv1a64, name: &str, bytes: &[u8]) {
    f.write(&(name.len() as u64).to_le_bytes());
    f.write(name.as_bytes());
    f.write(&(bytes.len() as u64).to_le_bytes());
    f.write(bytes);
}

/// Render a fingerprint the way manifests store it.
pub fn hex(v: u64) -> String {
    format!("{v:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_fnv1a_reference_vectors() {
        // From the FNV reference code by Fowler, Noll and Vo.
        let mut f = Fnv1a64::new();
        f.write(b"");
        assert_eq!(f.finish(), 0xcbf2_9ce4_8422_2325);
        let mut f = Fnv1a64::new();
        f.write(b"a");
        assert_eq!(f.finish(), 0xaf63_dc4c_8601_ec8c);
        let mut f = Fnv1a64::new();
        f.write(b"foobar");
        assert_eq!(f.finish(), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn length_prefix_prevents_concatenation_collisions() {
        let mut a = Fnv1a64::new();
        add_source(&mut a, "ab", b"c");
        let mut b = Fnv1a64::new();
        add_source(&mut b, "a", b"bc");
        assert_ne!(a.finish(), b.finish());
    }

    #[test]
    fn hex_is_zero_padded() {
        assert_eq!(hex(0x1a), "000000000000001a");
    }
}
