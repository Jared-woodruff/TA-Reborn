//! Deterministic pseudo-random numbers (PCG-XSH-RR 32).

#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
    inc: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut r = Rng { state: 0, inc: (seed << 1) | 1 };
        r.next_u32();
        r.state = r.state.wrapping_add(seed ^ 0x853c_49e6_748f_ea9b);
        r.next_u32();
        r
    }

    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// Uniform integer in `lo..=hi` (hi >= lo).
    pub fn range_i32(&mut self, lo: i32, hi: i32) -> i32 {
        let span = (hi as i64 - lo as i64 + 1) as u64;
        if span <= 1 {
            return lo;
        }
        (lo as i64 + (self.next_u32() as u64 % span) as i64) as i32
    }

    /// Uniform fixed-point value in `[0, 1)`.
    pub fn unit_fx(&mut self) -> crate::fixed::Fx {
        crate::fixed::Fx((self.next_u32() >> 16) as i64)
    }

    pub fn chance(&mut self, numerator: u32, denominator: u32) -> bool {
        denominator > 0 && self.next_u32() % denominator < numerator
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_in_range() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..1000 {
            let v = a.range_i32(-5, 5);
            assert_eq!(v, b.range_i32(-5, 5));
            assert!((-5..=5).contains(&v));
        }
        assert_ne!(Rng::new(1).next_u32(), Rng::new(2).next_u32());
    }
}
