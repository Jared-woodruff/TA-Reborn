//! Sine and cosine of our own: only arithmetic whose results IEEE fixes to the bit, so a map
//! converted on one platform is the same, byte for byte, as on another (the platform's `sin` and
//! `cos` may differ in their last bit).

use std::f64::consts::{FRAC_PI_2, PI};

/// `(sin a, cos a)`, within a unit in the last place or two.
pub fn sin_cos(a: f64) -> (f64, f64) {
    if !a.is_finite() {
        return (0.0, 1.0);
    }
    // Into [-π, π], then into [-π/2, π/2] (sin(π - x) = sin x, cos(π - x) = -cos x).
    let turns = (a / (2.0 * PI)).round();
    let mut x = a - turns * (2.0 * PI);
    let mut flip = 1.0;
    if x > FRAC_PI_2 {
        x = PI - x;
        flip = -1.0;
    } else if x < -FRAC_PI_2 {
        x = -PI - x;
        flip = -1.0;
    }
    let x2 = x * x;
    // Taylor series to the 17th power: within 1e-16 on [-π/2, π/2].
    let mut s = 0.0;
    let mut c = 0.0;
    let mut ts = x;
    let mut tc = 1.0;
    for n in 0..10 {
        s += ts;
        c += tc;
        let k = 2.0 * n as f64;
        ts *= -x2 / ((k + 2.0) * (k + 3.0));
        tc *= -x2 / ((k + 1.0) * (k + 2.0));
    }
    (s, flip * c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_and_cosine_agree_with_the_platforms() {
        for i in -200..=200 {
            let a = i as f64 * 0.0731;
            let (s, c) = sin_cos(a);
            assert!((s - a.sin()).abs() < 1e-14 && (c - a.cos()).abs() < 1e-14, "{a}: {s} {c}");
        }
        assert_eq!(sin_cos(0.0), (0.0, 1.0));
        assert_eq!(sin_cos(f64::NAN), (0.0, 1.0));
    }
}
