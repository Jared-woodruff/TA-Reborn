//! Small DSP building blocks shared by the mixer and the music decoder threads.

/// A gain ramp measured in samples, independent of device callback boundaries.
pub(crate) struct GainRamp {
    value: f32,
    target: f32,
    step: f32,
    remaining: u32,
}

impl GainRamp {
    pub(crate) fn new(value: f32) -> Self {
        Self { value, target: value, step: 0.0, remaining: 0 }
    }

    pub(crate) fn set(&mut self, target: f32, frames: u32) {
        if self.target == target { return; }
        self.target = target;
        self.remaining = frames.max(1);
        self.step = (target - self.value) / self.remaining as f32;
    }

    #[inline]
    pub(crate) fn next(&mut self) -> f32 {
        if self.remaining != 0 {
            self.remaining -= 1;
            self.value = if self.remaining == 0 { self.target } else { self.value + self.step };
        }
        self.value
    }
}

/// 4-point, 3rd-order Hermite (Catmull-Rom) interpolation between `x1` (t = 0) and `x2` (t = 1).
///
/// Good enough to upsample TA's 11 kHz effects to the device rate without the zipper and
/// aliasing artefacts of nearest-sample playback, and cheap enough for dozens of voices.
#[inline]
pub(crate) fn hermite(x0: f32, x1: f32, x2: f32, x3: f32, t: f32) -> f32 {
    let c1 = 0.5 * (x2 - x0);
    let c2 = x0 - 2.5 * x1 + 2.0 * x2 - 0.5 * x3;
    let c3 = 0.5 * (x3 - x0) + 1.5 * (x1 - x2);
    ((c3 * t + c2) * t + c1) * t + x1
}

/// Streaming sample-rate converter for stereo frames (cubic Hermite).
pub(crate) struct StereoResampler {
    /// Input frames advanced per output frame.
    step: f64,
    /// Read position in `history` (always >= 1 so one frame of look-behind exists).
    pos: f64,
    history: Vec<[f32; 2]>,
    passthrough: bool,
}

impl StereoResampler {
    pub(crate) fn new(input_rate: u32, output_rate: u32) -> StereoResampler {
        // (Never a step of 0: it would never move on. Decoders refuse such rates.)
        StereoResampler {
            step: input_rate.max(1) as f64 / output_rate.max(1) as f64,
            pos: 1.0,
            // One frame of leading silence provides the look-behind for the first real frame.
            history: vec![[0.0; 2]],
            passthrough: input_rate == output_rate,
        }
    }

    /// Converts `input`, appending the produced frames to `out`.
    pub(crate) fn process(&mut self, input: &[[f32; 2]], out: &mut Vec<[f32; 2]>) {
        if self.passthrough {
            out.extend_from_slice(input);
            return;
        }
        self.history.extend_from_slice(input);
        loop {
            let i = self.pos as usize;
            if i + 2 >= self.history.len() {
                break;
            }
            let t = (self.pos - i as f64) as f32;
            let [a, b, c, d] = [self.history[i - 1], self.history[i], self.history[i + 1], self.history[i + 2]];
            out.push([hermite(a[0], b[0], c[0], d[0], t), hermite(a[1], b[1], c[1], d[1], t)]);
            self.pos += self.step;
        }
        // Keep the frame before the read position; everything older is consumed.
        let keep_from = (self.pos as usize - 1).min(self.history.len());
        self.history.drain(..keep_from);
        self.pos -= keep_from as f64;
    }

    /// Emits the frames still held back for look-ahead (pads with silence).
    pub(crate) fn finish(&mut self, out: &mut Vec<[f32; 2]>) {
        if !self.passthrough {
            self.process(&[[0.0; 2]; 2], out);
        }
    }
}

/// Peak limiter for the master bus: instant attack (the output never exceeds the threshold),
/// exponential release. Only engages when many loud sounds pile up.
pub(crate) struct Limiter {
    gain: f32,
    release: f32,
    threshold: f32,
}

impl Limiter {
    pub(crate) fn new(sample_rate: u32) -> Limiter {
        let release_seconds = 0.15;
        let release = 1.0 - (-1.0 / (release_seconds * sample_rate.max(1) as f32)).exp();
        Limiter { gain: 1.0, release, threshold: 0.97 }
    }

    /// Current gain reduction factor (1 = none).
    pub(crate) fn gain(&self) -> f32 {
        self.gain
    }

    #[inline]
    pub(crate) fn process(&mut self, frame: &mut [f32; 2]) {
        let peak = frame[0].abs().max(frame[1].abs());
        let needed = if peak > self.threshold { self.threshold / peak } else { 1.0 };
        self.gain = (self.gain + (1.0 - self.gain) * self.release).min(needed);
        frame[0] *= self.gain;
        frame[1] *= self.gain;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hermite_passes_through_samples_and_lines() {
        assert_eq!(hermite(1.0, 2.0, 3.0, 4.0, 0.0), 2.0);
        assert_eq!(hermite(1.0, 2.0, 3.0, 4.0, 1.0), 3.0);
        assert!((hermite(1.0, 2.0, 3.0, 4.0, 0.25) - 2.25).abs() < 1e-6);
    }

    fn sine(rate: u32, freq: f32, frames: usize) -> Vec<[f32; 2]> {
        (0..frames)
            .map(|i| {
                let s = (std::f32::consts::TAU * freq * i as f32 / rate as f32).sin();
                [s, -s]
            })
            .collect()
    }

    #[test]
    fn resampler_preserves_length_and_tone_across_chunks() {
        let input = sine(44_100, 1000.0, 44_100);
        let mut rs = StereoResampler::new(44_100, 48_000);
        let mut out = Vec::new();
        for chunk in input.chunks(1152) {
            rs.process(chunk, &mut out);
        }
        rs.finish(&mut out);
        assert!((out.len() as i64 - 48_000).abs() <= 2, "{} frames", out.len());
        // Compare with the ideal 1 kHz sine at 48 kHz.
        let ideal = sine(48_000, 1000.0, out.len());
        let err: f32 =
            out[10..47_000].iter().zip(&ideal[10..47_000]).map(|(a, b)| (a[0] - b[0]).abs()).fold(0.0, f32::max);
        assert!(err < 0.01, "max error {err}");
        assert!(out.iter().all(|f| (f[0] + f[1]).abs() < 1e-6), "channels stay independent");
    }

    #[test]
    fn passthrough_copies() {
        let input = sine(48_000, 440.0, 100);
        let mut rs = StereoResampler::new(48_000, 48_000);
        let mut out = Vec::new();
        rs.process(&input, &mut out);
        rs.finish(&mut out);
        assert_eq!(out, input);
    }

    #[test]
    fn limiter_bounds_output_and_recovers() {
        let mut lim = Limiter::new(48_000);
        let mut loud = [3.0, -2.0];
        lim.process(&mut loud);
        assert!(loud[0].abs() <= 0.97 + 1e-6 && loud[1].abs() <= 0.97);
        for _ in 0..48_000 {
            let mut quiet = [0.1, 0.1];
            lim.process(&mut quiet);
        }
        assert!(lim.gain() > 0.99);
    }
}
