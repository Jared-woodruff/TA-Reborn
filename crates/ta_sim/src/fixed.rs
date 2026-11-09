//! Deterministic fixed-point arithmetic.
//!
//! The simulation must produce bit-identical results on every machine (lockstep multiplayer
//! and replays), so it never uses floating point. [`Fx`] has TA's 16 fractional bits (its
//! representation for positions and COB script values) in an `i64`: 48.16, so sums of map
//! coordinates never overflow; products and quotients go through `i128`. Arithmetic
//! saturates at [`Fx::MAX`] and [`Fx::MIN`] instead of wrapping (sums at the `i64` range),
//! so a value past them (a mod's absurd numbers) stays the largest there is.
//!
//! Angles use TA's convention: an `i32` where 65536 is a full turn. Trigonometry is table
//! based; the tables are generated with pure IEEE arithmetic (no platform `libm`), so they
//! are identical everywhere.

use std::fmt;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};
use std::sync::OnceLock;

pub const FRAC_BITS: u32 = 16;
const ONE_RAW: i64 = 1 << FRAC_BITS;

#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub struct Fx(pub i64);

impl Fx {
    pub const ZERO: Fx = Fx(0);
    pub const ONE: Fx = Fx(ONE_RAW);
    pub const HALF: Fx = Fx(ONE_RAW / 2);
    /// The largest and smallest results of conversions, products and quotients (a quarter of
    /// the `i64` range: a few of them still add up without overflowing).
    pub const MAX: Fx = Fx(i64::MAX / 4);
    pub const MIN: Fx = Fx(i64::MIN / 4);
    pub const EPSILON: Fx = Fx(1);

    /// A raw value clamped to [`Fx::MIN`]..=[`Fx::MAX`].
    #[inline]
    const fn saturate(raw: i128) -> Fx {
        if raw > Fx::MAX.0 as i128 {
            Fx::MAX
        } else if raw < Fx::MIN.0 as i128 {
            Fx::MIN
        } else {
            Fx(raw as i64)
        }
    }

    /// `v` (saturating beyond [`Fx::MAX`], about 3.5 × 10^13).
    #[inline]
    pub const fn from_int(v: i64) -> Fx {
        Fx::saturate((v as i128) << FRAC_BITS)
    }

    /// `num / den` exactly rounded towards zero (saturating; zero for a zero `den`).
    #[inline]
    pub const fn from_ratio(num: i64, den: i64) -> Fx {
        if den == 0 {
            return Fx::ZERO;
        }
        Fx::saturate(((num as i128) << FRAC_BITS) / den as i128)
    }

    /// `part / whole` (0..=1 while `part` ≤ `whole`), rounded down; ONE when `whole` is zero.
    /// For progress kept as whole amounts of work.
    #[inline]
    pub const fn fraction(part: u64, whole: u64) -> Fx {
        if whole == 0 {
            return Fx::ONE;
        }
        Fx::saturate(((part as i128) << FRAC_BITS) / whole as i128)
    }

    /// `self × part / whole`, rounded towards zero, exactly (the product cannot overflow an
    /// `i128`); `self` when `whole` is zero. What `part` of `whole` work is worth, so the
    /// shares of all the work add up to exactly `self` (see `Sim::apply_build_power`).
    #[inline]
    pub const fn share(self, part: u64, whole: u64) -> Fx {
        if whole == 0 {
            return self;
        }
        Fx::saturate(self.0 as i128 * part as i128 / whole as i128)
    }

    /// Converts a definition value (saturating; NaN is zero). The conversion is deterministic
    /// IEEE rounding.
    pub fn from_f64(v: f64) -> Fx {
        Fx(((v * ONE_RAW as f64).round() as i64).clamp(Fx::MIN.0, Fx::MAX.0))
    }

    pub fn from_f32(v: f32) -> Fx {
        Self::from_f64(v as f64)
    }

    /// For rendering/UI only.
    #[inline]
    pub fn to_f32(self) -> f32 {
        self.0 as f32 / ONE_RAW as f32
    }

    #[inline]
    pub fn to_f64(self) -> f64 {
        self.0 as f64 / ONE_RAW as f64
    }

    /// Raw 16.16 value as an `i32` (COB script representation), saturating.
    #[inline]
    pub fn to_cob(self) -> i32 {
        self.0.clamp(i32::MIN as i64, i32::MAX as i64) as i32
    }

    #[inline]
    pub fn from_cob(v: i32) -> Fx {
        Fx(v as i64)
    }

    #[inline]
    pub fn to_int(self) -> i64 {
        self.0 >> FRAC_BITS
    }

    #[inline]
    pub fn round_int(self) -> i64 {
        self.0.saturating_add(ONE_RAW / 2) >> FRAC_BITS
    }

    #[inline]
    pub fn floor(self) -> Fx {
        Fx(self.0 & !(ONE_RAW - 1))
    }

    #[inline]
    pub fn frac(self) -> Fx {
        Fx(self.0 & (ONE_RAW - 1))
    }

    #[inline]
    pub fn abs(self) -> Fx {
        Fx(self.0.saturating_abs())
    }

    #[inline]
    pub fn signum(self) -> i64 {
        self.0.signum()
    }

    #[inline]
    pub fn min(self, o: Fx) -> Fx {
        if self.0 < o.0 { self } else { o }
    }

    #[inline]
    pub fn max(self, o: Fx) -> Fx {
        if self.0 > o.0 { self } else { o }
    }

    #[inline]
    pub fn clamp(self, lo: Fx, hi: Fx) -> Fx {
        self.max(lo).min(hi)
    }

    #[inline]
    pub fn is_zero(self) -> bool {
        self.0 == 0
    }

    pub fn sqrt(self) -> Fx {
        if self.0 <= 0 {
            return Fx::ZERO;
        }
        Fx(isqrt_u128((self.0 as u128) << FRAC_BITS) as i64)
    }

    /// Moves `self` towards `target` by at most `step` (step must be non-negative).
    pub fn approach(self, target: Fx, step: Fx) -> Fx {
        if self < target { (self + step).min(target) } else { (self - step).max(target) }
    }

    pub fn lerp(a: Fx, b: Fx, t: Fx) -> Fx {
        a + (b - a) * t
    }
}

impl fmt::Debug for Fx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.4}", self.to_f64())
    }
}

impl fmt::Display for Fx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.3}", self.to_f64())
    }
}

impl Add for Fx {
    type Output = Fx;
    #[inline]
    fn add(self, o: Fx) -> Fx {
        Fx(self.0.saturating_add(o.0))
    }
}

impl Sub for Fx {
    type Output = Fx;
    #[inline]
    fn sub(self, o: Fx) -> Fx {
        Fx(self.0.saturating_sub(o.0))
    }
}

impl Neg for Fx {
    type Output = Fx;
    #[inline]
    fn neg(self) -> Fx {
        Fx(self.0.saturating_neg())
    }
}

impl AddAssign for Fx {
    #[inline]
    fn add_assign(&mut self, o: Fx) {
        self.0 = self.0.saturating_add(o.0);
    }
}

impl SubAssign for Fx {
    #[inline]
    fn sub_assign(&mut self, o: Fx) {
        self.0 = self.0.saturating_sub(o.0);
    }
}

impl Mul for Fx {
    type Output = Fx;
    #[inline]
    fn mul(self, o: Fx) -> Fx {
        Fx::saturate((self.0 as i128 * o.0 as i128) >> FRAC_BITS)
    }
}

impl Div for Fx {
    type Output = Fx;
    /// Division by zero yields 0 rather than panicking (script data can divide by zero).
    #[inline]
    fn div(self, o: Fx) -> Fx {
        if o.0 == 0 {
            return Fx::ZERO;
        }
        Fx::saturate(((self.0 as i128) << FRAC_BITS) / o.0 as i128)
    }
}

impl Mul<i64> for Fx {
    type Output = Fx;
    #[inline]
    fn mul(self, o: i64) -> Fx {
        Fx::saturate(self.0 as i128 * o as i128)
    }
}

impl Div<i64> for Fx {
    type Output = Fx;
    #[inline]
    fn div(self, o: i64) -> Fx {
        if o == 0 { Fx::ZERO } else { Fx(self.0.saturating_div(o)) }
    }
}

/// Integer square root (floor) of a u128.
#[inline]
pub fn isqrt_u128(n: u128) -> u128 {
    n.isqrt()
}

/// A 3-component fixed-point vector (x east, y up, z south).
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct FxVec3 {
    pub x: Fx,
    pub y: Fx,
    pub z: Fx,
}

impl fmt::Debug for FxVec3 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({:?}, {:?}, {:?})", self.x, self.y, self.z)
    }
}

impl FxVec3 {
    pub const ZERO: FxVec3 = FxVec3 { x: Fx::ZERO, y: Fx::ZERO, z: Fx::ZERO };

    #[inline]
    pub const fn new(x: Fx, y: Fx, z: Fx) -> FxVec3 {
        FxVec3 { x, y, z }
    }

    pub fn from_ints(x: i64, y: i64, z: i64) -> FxVec3 {
        FxVec3::new(Fx::from_int(x), Fx::from_int(y), Fx::from_int(z))
    }

    /// Squared length in 32.32 fixed point (exact unless the i128 result saturates).
    #[inline]
    pub fn length_sq_raw(self) -> i128 {
        let (x, y, z) = (self.x.0 as i128, self.y.0 as i128, self.z.0 as i128);
        (x * x).saturating_add(y * y).saturating_add(z * z)
    }

    #[inline]
    pub fn length(self) -> Fx {
        Fx::saturate(isqrt_u128(self.length_sq_raw() as u128) as i128)
    }

    /// Length of the horizontal (x, z) component.
    #[inline]
    pub fn length_xz(self) -> Fx {
        let (x, z) = (self.x.0 as i128, self.z.0 as i128);
        Fx::saturate(isqrt_u128((x * x).saturating_add(z * z) as u128) as i128)
    }

    #[inline]
    pub fn dot(self, o: FxVec3) -> Fx {
        let (x, y, z) = (self.x.0 as i128 * o.x.0 as i128, self.y.0 as i128 * o.y.0 as i128, self.z.0 as i128 * o.z.0 as i128);
        // Add opposite signs first, so cancellation remains exact even at raw i64 limits.
        let sum = if x.signum() != y.signum() { (x + y).saturating_add(z) } else if x.signum() != z.signum() { (x + z).saturating_add(y) } else { x.saturating_add(y).saturating_add(z) };
        Fx::saturate(sum >> FRAC_BITS)
    }

    pub fn cross(self, o: FxVec3) -> FxVec3 {
        FxVec3::new(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }

    pub fn normalize(self) -> FxVec3 {
        let l = self.length();
        if l.is_zero() { FxVec3::ZERO } else { FxVec3::new(self.x / l, self.y / l, self.z / l) }
    }

    #[inline]
    pub fn scale(self, s: Fx) -> FxVec3 {
        FxVec3::new(self.x * s, self.y * s, self.z * s)
    }

    #[inline]
    pub fn xz(self) -> FxVec3 {
        FxVec3::new(self.x, Fx::ZERO, self.z)
    }

    pub fn distance(self, o: FxVec3) -> Fx {
        (self - o).length()
    }

    pub fn distance_xz(self, o: FxVec3) -> Fx {
        (self - o).length_xz()
    }

    /// Compares |self - o| with `r` without taking a square root.
    pub fn within(self, o: FxVec3, r: Fx) -> bool {
        (self - o).length_sq_raw() <= (r.0 as i128) * (r.0 as i128)
    }

    pub fn within_xz(self, o: FxVec3, r: Fx) -> bool {
        let d = self - o;
        let (x, z) = (d.x.0 as i128, d.z.0 as i128);
        (x * x).saturating_add(z * z) <= (r.0 as i128) * (r.0 as i128)
    }

    pub fn to_f32(self) -> [f32; 3] {
        [self.x.to_f32(), self.y.to_f32(), self.z.to_f32()]
    }
}

impl Add for FxVec3 {
    type Output = FxVec3;
    #[inline]
    fn add(self, o: FxVec3) -> FxVec3 {
        FxVec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for FxVec3 {
    type Output = FxVec3;
    #[inline]
    fn sub(self, o: FxVec3) -> FxVec3 {
        FxVec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Neg for FxVec3 {
    type Output = FxVec3;
    #[inline]
    fn neg(self) -> FxVec3 {
        FxVec3::new(-self.x, -self.y, -self.z)
    }
}

impl AddAssign for FxVec3 {
    #[inline]
    fn add_assign(&mut self, o: FxVec3) {
        *self = *self + o;
    }
}

impl SubAssign for FxVec3 {
    #[inline]
    fn sub_assign(&mut self, o: FxVec3) {
        *self = *self - o;
    }
}

// ---- Angles -----------------------------------------------------------------------------

/// TA angle units per full turn.
pub const FULL_TURN: i32 = 65536;
pub const QUARTER_TURN: i32 = 16384;

/// Wraps an angle into `[-32768, 32767]`.
#[inline]
pub fn wrap_angle(a: i32) -> i32 {
    a as i16 as i32
}

struct Tables {
    /// sin over a quarter turn, 16385 entries, 16.16.
    sin: Vec<i64>,
    /// atan(i / 4096) for i in 0..=4096, in angle units.
    atan: Vec<i32>,
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        // Pure-arithmetic series: identical results on every IEEE-754 platform.
        fn sin_series(x: f64) -> f64 {
            let x2 = x * x;
            let mut term = x;
            let mut sum = x;
            let mut k = 1.0;
            for _ in 0..12 {
                term = -term * x2 / ((2.0 * k) * (2.0 * k + 1.0));
                sum += term;
                k += 1.0;
            }
            sum
        }
        fn atan_series(t: f64) -> f64 {
            // atan(t) for t in [0, 1] via argument halving: atan(t) = 2 atan(t / (1 + sqrt(1 + t^2))).
            let s = t / (1.0 + (1.0 + t * t).sqrt());
            let s = s / (1.0 + (1.0 + s * s).sqrt());
            // |s| <= tan(pi/16) ≈ 0.199: the series converges fast.
            let s2 = s * s;
            let mut term = s;
            let mut sum = s;
            let mut n = 1.0;
            for _ in 0..20 {
                term = -term * s2;
                n += 2.0;
                sum += term / n;
            }
            4.0 * sum
        }
        const PI: f64 = std::f64::consts::PI;
        let sin = (0..=QUARTER_TURN)
            .map(|i| (sin_series(i as f64 * (PI / 2.0) / QUARTER_TURN as f64) * ONE_RAW as f64).round() as i64)
            .collect();
        let atan = (0..=4096)
            .map(|i| (atan_series(i as f64 / 4096.0) * FULL_TURN as f64 / (2.0 * PI)).round() as i32)
            .collect();
        Tables { sin, atan }
    })
}

/// Sine of a TA angle, 16.16.
pub fn sin(a: i32) -> Fx {
    let t = tables();
    let a = a.rem_euclid(FULL_TURN);
    let q = a / QUARTER_TURN;
    let r = (a % QUARTER_TURN) as usize;
    Fx(match q {
        0 => t.sin[r],
        1 => t.sin[QUARTER_TURN as usize - r],
        2 => -t.sin[r],
        _ => -t.sin[QUARTER_TURN as usize - r],
    })
}

pub fn cos(a: i32) -> Fx {
    sin(a.wrapping_add(QUARTER_TURN))
}

/// `atan2(y, x)` in TA angle units, range `[-32768, 32767]`; `atan2(0, 1) = 0`,
/// `atan2(1, 0) = 16384`.
pub fn atan2(y: Fx, x: Fx) -> i32 {
    let t = tables();
    let (ax, ay) = (x.0.unsigned_abs(), y.0.unsigned_abs());
    if ax == 0 && ay == 0 {
        return 0;
    }
    // Angle in the first octant from the ratio min/max, then unfold.
    let (small, large, swapped) = if ay <= ax { (ay, ax, false) } else { (ax, ay, true) };
    let ratio = ((small as u128) << 24) / large as u128; // 0..=2^24 represents 0..=1
    let idx = (ratio >> 12) as usize; // 0..=4096
    let frac = (ratio & 0xFFF) as i64;
    let a0 = t.atan[idx.min(4096)] as i64;
    let a1 = t.atan[(idx + 1).min(4096)] as i64;
    let mut a = (a0 + (((a1 - a0) * frac) >> 12)) as i32;
    if swapped {
        a = QUARTER_TURN - a;
    }
    if x.0 < 0 {
        a = 2 * QUARTER_TURN - a;
    }
    if y.0 < 0 {
        a = -a;
    }
    wrap_angle(a)
}

/// Unit vector in the horizontal plane for a heading (0 = +z/south, 16384 = +x/east).
pub fn heading_vector(heading: i32) -> FxVec3 {
    FxVec3::new(sin(heading), Fx::ZERO, cos(heading))
}

/// Heading of a horizontal direction vector (inverse of [`heading_vector`]).
pub fn heading_of(v: FxVec3) -> i32 {
    atan2(v.x, v.z)
}

/// Rotates a vector about the y axis (right-handed: +angle turns +z towards +x).
/// Whole turns still saturate the rotated axes, as multiplication by ONE does.
pub fn rotate_y(v: FxVec3, a: i32) -> FxVec3 {
    if a as u16 == 0 { return FxVec3::new(v.x.clamp(Fx::MIN, Fx::MAX), v.y, v.z.clamp(Fx::MIN, Fx::MAX)); }
    let (s, c) = (sin(a), cos(a));
    FxVec3::new(v.x * c + v.z * s, v.y, v.z * c - v.x * s)
}

pub fn rotate_x(v: FxVec3, a: i32) -> FxVec3 {
    if a as u16 == 0 { return FxVec3::new(v.x, v.y.clamp(Fx::MIN, Fx::MAX), v.z.clamp(Fx::MIN, Fx::MAX)); }
    let (s, c) = (sin(a), cos(a));
    FxVec3::new(v.x, v.y * c - v.z * s, v.y * s + v.z * c)
}

pub fn rotate_z(v: FxVec3, a: i32) -> FxVec3 {
    if a as u16 == 0 { return FxVec3::new(v.x.clamp(Fx::MIN, Fx::MAX), v.y.clamp(Fx::MIN, Fx::MAX), v.z); }
    let (s, c) = (sin(a), cos(a));
    FxVec3::new(v.x * c - v.y * s, v.x * s + v.y * c, v.z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotations_match_the_full_fixed_point_formula_at_every_heading() {
        let values = [FxVec3::ZERO, FxVec3::from_ints(37, -91, 53), FxVec3::new(Fx::MIN, Fx::MAX, Fx(-1)), FxVec3::new(Fx(i64::MAX), Fx(i64::MIN), Fx(1))];
        for a in (0..FULL_TURN).chain([i32::MIN, i32::MAX, -FULL_TURN, FULL_TURN, FULL_TURN * 16]) {
            let (s, c) = (sin(a), cos(a));
            for v in values {
                assert_eq!(rotate_y(v, a), FxVec3::new(v.x * c + v.z * s, v.y, v.z * c - v.x * s), "y at {a}");
                assert_eq!(rotate_x(v, a), FxVec3::new(v.x, v.y * c - v.z * s, v.y * s + v.z * c), "x at {a}");
                assert_eq!(rotate_z(v, a), FxVec3::new(v.x * c - v.y * s, v.x * s + v.y * c, v.z), "z at {a}");
            }
        }
    }

    #[test]
    fn integer_square_root_is_the_exact_floor_across_its_range() {
        let check = |n: u128| {
            let root = isqrt_u128(n);
            assert!(root.checked_mul(root).is_some_and(|square| square <= n), "root too large for {n}");
            assert!((root + 1).checked_mul(root + 1).is_none_or(|square| square > n), "root too small for {n}");
        };
        for n in 0..65_536 {
            check(n);
        }
        for bit in 0..128 {
            let n = 1u128 << bit;
            check(n - 1);
            check(n);
            check(n + 1);
        }
        // Exact squares and both adjacent values, including the largest representable root.
        for root in [1u128, 2, 3, 255, 256, 65_535, 65_536, u32::MAX as u128, 1 << 32, u64::MAX as u128] {
            let square = root * root;
            assert_eq!(isqrt_u128(square), root);
            check(square - 1);
            check(square);
            check(square + 1);
        }
        check(u128::MAX);
        let mut state = 0x4d59_5df4_d0f3_3173u128;
        for _ in 0..10_000 {
            state = state.wrapping_mul(0xda94_2042_e4dd_58b5).wrapping_add(1);
            check(state);
        }
    }

    #[test]
    fn arithmetic() {
        let a = Fx::from_f64(1.5);
        let b = Fx::from_int(2);
        assert_eq!(a * b, Fx::from_int(3));
        assert_eq!(Fx::from_int(3) / b, a);
        assert_eq!(Fx::from_int(9).sqrt(), Fx::from_int(3));
        assert!((Fx::from_int(2).sqrt().to_f64() - std::f64::consts::SQRT_2).abs() < 1e-4);
        assert_eq!(Fx::ONE / Fx::ZERO, Fx::ZERO);
    }

    #[test]
    fn overflow_saturates_instead_of_wrapping() {
        let big = Fx::from_int(1 << 40);
        assert_eq!(big * big, Fx::MAX, "a product past the range");
        assert_eq!(big * -big, Fx::MIN);
        assert_eq!(big / Fx::EPSILON, Fx::MAX, "a quotient past it");
        assert_eq!(-big / Fx::EPSILON, Fx::MIN);
        assert_eq!(Fx::MAX * 4, Fx::MAX);
        assert_eq!(Fx::MIN * 4, Fx::MIN);
        assert_eq!(Fx(i64::MAX) + Fx::ONE, Fx(i64::MAX), "sums saturate at the i64 range");
        assert_eq!(Fx(i64::MIN) - Fx::ONE, Fx(i64::MIN));
        let mut sum = Fx(i64::MAX - 1);
        sum += Fx::ONE;
        assert_eq!(sum, Fx(i64::MAX));
        assert_eq!(-Fx(i64::MIN), Fx(i64::MAX));
        assert_eq!(Fx(i64::MIN).abs(), Fx(i64::MAX));
        assert_eq!(Fx(i64::MIN) / -1, Fx(i64::MAX));
        assert_eq!(Fx::from_int(i64::MAX), Fx::MAX);
        assert_eq!(Fx::from_int(i64::MIN), Fx::MIN);
        assert_eq!(Fx::from_ratio(i64::MAX, 1), Fx::MAX);
        assert_eq!(Fx::from_ratio(5, 0), Fx::ZERO);
        assert_eq!(Fx::from_f64(1e300), Fx::MAX);
        assert_eq!(Fx::from_f64(-1e300), Fx::MIN);
        assert_eq!(Fx::from_f64(f64::NAN), Fx::ZERO);
        let far = FxVec3::new(Fx::MAX, Fx::MAX, Fx::MAX);
        assert_eq!(far.dot(far), Fx::MAX);
        // Within the range nothing changes.
        assert_eq!(Fx::from_int(3) * Fx::from_int(-7), Fx::from_int(-21));
        assert_eq!(Fx::MAX * Fx::ONE, Fx::MAX);
        assert_eq!(Fx::from_int(-21) / Fx::from_int(3), Fx::from_int(-7));
        assert_eq!(Fx::from_ratio(1, 3), Fx(21845));
        assert_eq!(Fx::from_int(1 << 44).to_int(), 1 << 44);
    }

    #[test]
    fn deserialized_raw_vectors_do_not_overflow_intermediate_sums() {
        let extreme = FxVec3::new(Fx(i64::MIN), Fx(i64::MIN), Fx(i64::MIN));
        assert_eq!(extreme.length_sq_raw(), i128::MAX);
        assert_eq!(extreme.length(), Fx::MAX);
        assert_eq!(extreme.length_xz(), Fx::MAX);
        assert_eq!(extreme.dot(extreme), Fx::MAX);
        assert!(!extreme.within(FxVec3::ZERO, Fx::ONE));
        assert!(!extreme.within_xz(FxVec3::ZERO, Fx::ONE));
        assert_eq!(extreme.dot(FxVec3::new(Fx(i64::MIN), Fx(i64::MAX), Fx(0))), Fx(1 << 47));
        assert_eq!(FxVec3::from_ints(3, 4, 0).length(), Fx::from_int(5));
    }

    #[test]
    fn trig_tables() {
        assert_eq!(sin(0), Fx::ZERO);
        assert_eq!(sin(QUARTER_TURN), Fx::ONE);
        assert_eq!(cos(0), Fx::ONE);
        assert_eq!(sin(2 * QUARTER_TURN), Fx::ZERO);
        assert_eq!(sin(-QUARTER_TURN), -Fx::ONE);
        let s45 = sin(8192).to_f64();
        assert!((s45 - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-4);
    }

    #[test]
    fn atan2_quadrants() {
        let one = Fx::ONE;
        assert_eq!(atan2(Fx::ZERO, one), 0);
        assert_eq!(atan2(one, Fx::ZERO), QUARTER_TURN);
        assert_eq!(atan2(Fx::ZERO, -one), -32768);
        assert_eq!(atan2(-one, Fx::ZERO), -QUARTER_TURN);
        assert!((atan2(one, one) - 8192).abs() <= 1);
        assert!((atan2(-one, -one) + 3 * 8192).abs() <= 1);
        for a in (-32768..32768).step_by(997) {
            let back = atan2(sin(a), cos(a));
            assert!(wrap_angle(back - a).abs() <= 2, "a={a} back={back}");
        }
    }

    #[test]
    fn heading_round_trip() {
        for h in [0, 5000, 16384, -16384, 30000, -32768] {
            let v = heading_vector(h);
            assert!(wrap_angle(heading_of(v) - h).abs() <= 2);
        }
        // Heading 0 faces south (+z); a quarter turn faces east (+x).
        assert_eq!(heading_vector(0), FxVec3::new(Fx::ZERO, Fx::ZERO, Fx::ONE));
        assert_eq!(heading_vector(QUARTER_TURN).x, Fx::ONE);
        // rotate_y agrees with heading_vector.
        let r = rotate_y(FxVec3::new(Fx::ZERO, Fx::ZERO, Fx::ONE), 7000);
        assert_eq!(r, heading_vector(7000));
    }
}
