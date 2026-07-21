//! Deterministic PRNG.
//!
//! Reproducibility is the whole point of the replay mechanism, so the harness
//! never touches `std`'s thread RNG or the system clock. Everything derives
//! from a single `u64` seed via SplitMix64, which is small, fast, and has no
//! dependencies to pin.

/// SplitMix64. Chosen over xorshift because a bad (e.g. zero) seed still
/// produces a well-distributed stream, so seeds can be plain sequence indices.
#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, n)`. Returns 0 for `n == 0`.
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        (self.next_u64() % n as u64) as usize
    }

    /// Uniform in `[lo, hi]` inclusive.
    pub fn range_i128(&mut self, lo: i128, hi: i128) -> i128 {
        if hi <= lo {
            return lo;
        }
        let span = (hi - lo + 1) as u128;
        lo + (self.next_u64() as u128 % span) as i128
    }

    pub fn range_u32(&mut self, lo: u32, hi: u32) -> u32 {
        if hi <= lo {
            return lo;
        }
        lo + (self.next_u64() % (hi - lo + 1) as u64) as u32
    }

    pub fn range_u64(&mut self, lo: u64, hi: u64) -> u64 {
        if hi <= lo {
            return lo;
        }
        lo + self.next_u64() % (hi - lo + 1)
    }

    /// True with probability `pct/100`.
    pub fn chance(&mut self, pct: u32) -> bool {
        (self.next_u64() % 100) < pct as u64
    }

    /// Pick an element index weighted by `weights`. Returns 0 if all zero.
    pub fn weighted(&mut self, weights: &[u32]) -> usize {
        let total: u32 = weights.iter().sum();
        if total == 0 {
            return 0;
        }
        let mut pick = (self.next_u64() % total as u64) as u32;
        for (i, w) in weights.iter().enumerate() {
            if pick < *w {
                return i;
            }
            pick -= *w;
        }
        weights.len() - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        let diverged = (0..64).any(|_| a.next_u64() != b.next_u64());
        assert!(diverged);
    }

    #[test]
    fn zero_seed_is_well_distributed() {
        // SplitMix64 must not degenerate on a zero seed.
        let mut r = Rng::new(0);
        let mut buckets = [0usize; 8];
        for _ in 0..8000 {
            buckets[r.below(8)] += 1;
        }
        for b in buckets {
            assert!(b > 800, "bucket {b} too small, distribution is skewed");
        }
    }

    #[test]
    fn range_bounds_are_inclusive_and_respected() {
        let mut r = Rng::new(7);
        for _ in 0..5000 {
            let v = r.range_i128(-5, 5);
            assert!((-5..=5).contains(&v));
            let u = r.range_u32(2000, 2100);
            assert!((2000..=2100).contains(&u));
        }
    }

    #[test]
    fn degenerate_ranges_do_not_panic() {
        let mut r = Rng::new(9);
        assert_eq!(r.range_i128(3, 3), 3);
        assert_eq!(r.range_i128(5, 1), 5);
        assert_eq!(r.below(0), 0);
        assert_eq!(r.weighted(&[0, 0, 0]), 0);
    }
}
