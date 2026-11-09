//! A tiny SplitMix64 generator for variant and playlist choices. Audio randomness never feeds
//! the simulation, so it needs no cross-machine determinism, only decent spread.

#[derive(Clone, Debug)]
pub(crate) struct Rng(u64);

impl Rng {
    #[cfg(test)]
    pub(crate) fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    /// Seeded from the clock.
    pub(crate) fn from_time() -> Rng {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x1234_5678);
        Rng(nanos ^ 0x9E37_79B9_7F4A_7C15)
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform integer in `0..n` (`n > 0`).
    pub(crate) fn below(&mut self, n: usize) -> usize {
        (((self.next_u64() >> 32) * n as u64) >> 32) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn below_covers_range() {
        let mut r = Rng::new(1);
        let mut seen = [0u32; 5];
        for _ in 0..5000 {
            seen[r.below(5)] += 1;
        }
        assert!(seen.iter().all(|&c| c > 800), "{seen:?}");
    }
}
