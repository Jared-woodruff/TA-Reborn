//! Reproducible CPU-only benchmark: `cargo test --release -p ta_sim --test
//! fixed_performance -- --ignored --nocapture`. No original game assets are needed.

use std::hint::black_box;
use std::time::Instant;

use ta_sim::fixed::isqrt_u128;

/// The former Newton implementation, retained here as the benchmark baseline.
fn newton_sqrt(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    let bits = 128 - n.leading_zeros();
    let mut x = 1u128 << bits.div_ceil(2);
    loop {
        let y = (x + n / x) >> 1;
        if y >= x {
            return x;
        }
        x = y;
    }
}

fn measure(values: &[u128], root: fn(u128) -> u128) -> f64 {
    let started = Instant::now();
    let mut sum = 0u128;
    for _ in 0..128 {
        for &n in values {
            sum = sum.wrapping_add(root(black_box(n)));
        }
    }
    black_box(sum);
    started.elapsed().as_secs_f64() * 1e9 / (values.len() * 128) as f64
}

#[test]
#[ignore = "manual timing; run release on a quiet machine"]
fn fixed_point_square_roots() {
    let mut state = 0x4d59_5df4_d0f3_3173u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for (name, bits) in [
        ("nearby (128 px)", 23),
        ("map (8192 px)", 29),
        ("full u128 range", 64),
    ] {
        let values: Vec<u128> = (0..8192)
            .map(|_| {
                let (x, y, z) = (next() as u128, next() as u128, next() as u128);
                if bits == 64 {
                    (x << 64) | y
                } else {
                    let mask = (1 << bits) - 1;
                    (x & mask).pow(2) + (y & mask).pow(2) + (z & mask).pow(2)
                }
            })
            .collect();
        for &n in &values {
            assert_eq!(isqrt_u128(n), newton_sqrt(n));
            assert_eq!(n.isqrt(), newton_sqrt(n));
        }
        let (mut baseline, mut production, mut standard) = (Vec::new(), Vec::new(), Vec::new());
        for round in 0..8 {
            // Reverse the order on alternate samples to reduce warm-up/clock bias.
            let (a, b, c) = if round % 2 == 0 {
                (
                    measure(&values, newton_sqrt),
                    measure(&values, isqrt_u128),
                    measure(&values, u128::isqrt),
                )
            } else {
                let c = measure(&values, u128::isqrt);
                let b = measure(&values, isqrt_u128);
                (measure(&values, newton_sqrt), b, c)
            };
            if round > 0 {
                baseline.push(a);
                production.push(b);
                standard.push(c);
            }
        }
        for samples in [&mut baseline, &mut production, &mut standard] {
            samples.sort_by(f64::total_cmp);
        }
        println!(
            "{name}: median ns/root over 7 samples: Newton {:.2}, production {:.2}, standard {:.2}",
            baseline[3], production[3], standard[3]
        );
    }
}
