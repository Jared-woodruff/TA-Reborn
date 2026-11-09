//! A picture of the music: how loud it is in each band of pitch, from its latest samples (see
//! [`crate::AudioEngine::music_scope`]), for the jukebox's spectrum.

use std::f32::consts::PI;

/// How many of the music's latest samples a spectrum is made from (all the scope keeps).
pub const SAMPLES: usize = crate::mixer::SCOPE;

/// The lowest and highest pitch shown (Hz): below, a hum; above, hiss (and nothing at all in
/// most compressed music).
const LOWEST: f32 = 40.0;
const HIGHEST: f32 = 12_500.0;
/// Music has less in each higher pitch, much as pink noise has (3 dB an octave): bands are lifted
/// by as much over 1 kHz (and lowered under it), so they show it level.
const TILT_DB_PER_OCTAVE: f32 = 3.0;
/// The levels shown, in decibels below a full-scale sine: from silence to the loudest.
const FLOOR_DB: f32 = -60.0;
const CEILING_DB: f32 = -10.0;

/// How loud `samples` (mono, a power of two long, at `rate` Hz) are in each of `out.len()`
/// bands, spaced evenly in pitch from [`LOWEST`] to [`HIGHEST`] (or half the rate): 0 silent, 1
/// the loudest a band shows.
pub fn bands(samples: &[f32], rate: u32, out: &mut [f32]) {
    out.fill(0.0);
    let n = samples.len();
    if n < 16 || !n.is_power_of_two() || out.is_empty() || rate == 0 {
        return;
    }
    // Hann's window, so a pitch stays in its band.
    let mut re: Vec<f32> = samples.iter().enumerate().map(|(i, &s)| s * 0.5 * (1.0 - (2.0 * PI * i as f32 / n as f32).cos())).collect();
    let mut im = vec![0.0f32; n];
    fft(&mut re, &mut im);
    // (A full-scale sine through the window peaks at n/4.)
    let full = n as f32 / 4.0;
    let hz_per_bin = rate as f32 / n as f32;
    let top = HIGHEST.min(rate as f32 / 2.0);
    let ratio = (top / LOWEST).max(1.0);
    let count = out.len();
    for (b, level) in out.iter_mut().enumerate() {
        let lo = LOWEST * ratio.powf(b as f32 / count as f32);
        let hi = LOWEST * ratio.powf((b + 1) as f32 / count as f32);
        let first = ((lo / hz_per_bin).floor() as usize).clamp(1, n / 2 - 1);
        let last = ((hi / hz_per_bin).ceil() as usize).clamp(first + 1, n / 2);
        let peak = (first..last).map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt()).fold(0.0f32, f32::max);
        let tilt = TILT_DB_PER_OCTAVE * ((lo * hi).sqrt() / 1000.0).log2();
        let db = 20.0 * (peak / full).max(1e-9).log10() + tilt;
        *level = ((db - FLOOR_DB) / (CEILING_DB - FLOOR_DB)).clamp(0.0, 1.0);
    }
}

/// A fast Fourier transform in place (radix 2; the length a power of two).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -2.0 * PI / len as f32;
        let (wr, wi) = (angle.cos(), angle.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (xr, xi) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - xr;
                im[b] = im[a] - xi;
                re[a] += xr;
                im[a] += xi;
                (cr, ci) = (cr * wr - ci * wi, cr * wi + ci * wr);
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, amplitude: f32) -> Vec<f32> {
        (0..2048).map(|i| amplitude * (2.0 * PI * hz * i as f32 / 48_000.0).sin()).collect()
    }

    /// A tone lights its own band, and hardly any other; silence lights none; a quieter tone
    /// shows lower.
    #[test]
    fn a_tone_lights_its_band() {
        let mut out = [0.0f32; 32];
        bands(&sine(1000.0, 0.5), 48_000, &mut out);
        let loudest = out.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        let band_of = |hz: f32| ((hz / LOWEST).ln() / (HIGHEST / LOWEST).ln() * 32.0) as usize;
        // (1 kHz is near the edge of two bands.)
        assert!(loudest.abs_diff(band_of(1000.0)) <= 1, "{out:?}");
        assert!(out[loudest] > 0.8, "{out:?}");
        assert!(out[band_of(100.0)] < 0.2 && out[band_of(10_000.0)] < 0.2, "{out:?}");
        let mut quiet = [0.0f32; 32];
        bands(&sine(1000.0, 0.05), 48_000, &mut quiet);
        assert!(quiet[loudest] < out[loudest] - 0.2 && quiet[loudest] > 0.2, "{quiet:?}");
        let mut silent = [1.0f32; 32];
        bands(&[0.0; 2048], 48_000, &mut silent);
        assert!(silent.iter().all(|&l| l == 0.0));
        // Not a power of two: nothing.
        bands(&[0.5; 1000], 48_000, &mut silent);
        assert!(silent.iter().all(|&l| l == 0.0));
    }
}
