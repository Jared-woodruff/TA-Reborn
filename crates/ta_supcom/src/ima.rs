//! IMA ADPCM WAV files (`WAVE_FORMAT_IMA_ADPCM`, 0x0011: four bits a sample) from 16-bit samples,
//! as TA Reborn's sound decoder reads them (`ta_audio::decode::wav`): a quarter of PCM's bytes,
//! the same bytes on every computer (whole numbers alone).
//!
//! Mono, blocks of [`BLOCK`] bytes: each its first sample whole and the step index (a header of
//! four bytes), then two samples a byte, the low nibble first. Each nibble is the one of the eight
//! of its sign whose sample, as the decoder rebuilds it, lies nearest the true one (ties: the
//! smaller), the encoder following the decoder's own steps, so it never drifts from what is heard.

/// Bytes a block, and the samples one holds (its header's and two a byte after it).
pub const BLOCK: usize = 512;
pub const SAMPLES_PER_BLOCK: usize = 1 + (BLOCK - 4) * 2;

const STEPS: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371,
    408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132,
    7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];
const INDEX_ADJUST: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

/// The decoder's state: the sample it predicts and its step's index.
#[derive(Clone, Copy, Debug, Default)]
struct State {
    predictor: i32,
    index: i32,
}

impl State {
    /// The sample `nibble` rebuilds, and the state after it (as the decoder steps).
    fn after(self, nibble: u8) -> (i32, State) {
        let step = STEPS[self.index as usize];
        let mut diff = step >> 3;
        if nibble & 1 != 0 {
            diff += step >> 2;
        }
        if nibble & 2 != 0 {
            diff += step >> 1;
        }
        if nibble & 4 != 0 {
            diff += step;
        }
        if nibble & 8 != 0 {
            diff = -diff;
        }
        let predictor = (self.predictor + diff).clamp(-32768, 32767);
        let index = (self.index + INDEX_ADJUST[(nibble & 7) as usize]).clamp(0, 88);
        (predictor, State { predictor, index })
    }

    /// The nibble for `sample`: of its sign's eight, the one rebuilding the nearest sample.
    fn encode(&mut self, sample: i16) -> u8 {
        let (_, nibble, next) = self.nearest(sample);
        *self = next;
        nibble
    }

    /// Of the eight nibbles of `sample`'s sign, the one whose sample lies nearest it: its error
    /// squared, the nibble and the state after it.
    fn nearest(&self, sample: i16) -> (u64, u8, State) {
        let sign = if (sample as i32) < self.predictor { 8 } else { 0 };
        let mut best = (u64::MAX, 0u8, *self);
        for magnitude in 0..8u8 {
            let nibble = sign | magnitude;
            let (rebuilt, next) = self.after(nibble);
            let error = (rebuilt - sample as i32).unsigned_abs() as u64;
            if error * error < best.0 {
                best = (error * error, nibble, next);
            }
        }
        best
    }

    /// The nibble for `sample` with the sample after it in view (`then`): of the eight of its
    /// sign, the one whose sample and the nearest the next can come to after it lie nearest the two
    /// (squared errors summed; ties, the smaller nibble). A step adapting late to a sudden change
    /// costs less so.
    fn encode_ahead(&mut self, sample: i16, then: Option<i16>) -> u8 {
        let Some(then) = then else { return self.encode(sample) };
        let mut best = (u64::MAX, 0u8, *self);
        let sign = if (sample as i32) < self.predictor { 8 } else { 0 };
        for nibble in (0..8u8).map(|m| sign | m) {
            let (rebuilt, next) = self.after(nibble);
            let error = (rebuilt - sample as i32).unsigned_abs() as u64;
            let total = error * error + next.nearest(then).0;
            if total < best.0 {
                best = (total, nibble, next);
            }
        }
        *self = best.2;
        best.1
    }
}

/// The step index a block starts with: the one whose step is nearest the sample's first change,
/// so a loud sound is not slow to start.
fn first_index(samples: &[i16]) -> i32 {
    let change = samples.windows(2).take(8).map(|w| (w[1] as i32 - w[0] as i32).abs()).max().unwrap_or(0);
    STEPS.iter().position(|&s| s >= change / 2).unwrap_or(88) as i32
}

/// `samples` (mono, 16 bits) at `rate` as an IMA ADPCM WAV file.
pub fn wav(samples: &[i16], rate: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity(samples.len() / 2 + BLOCK);
    let mut state = State { predictor: 0, index: first_index(samples) };
    for block in samples.chunks(SAMPLES_PER_BLOCK) {
        state.predictor = block[0] as i32;
        data.extend_from_slice(&block[0].to_le_bytes());
        data.push(state.index as u8);
        data.push(0);
        // Two samples a byte, in groups of eight (four bytes): the last group's missing samples
        // held at the block's last, so its tail rebuilds no more than a few samples' noise.
        let rest = &block[1..];
        let last = *block.last().expect("a block holds a sample");
        let padded = rest.len().div_ceil(8) * 8;
        let at = |i: usize| rest.get(i).copied().unwrap_or(last);
        let mut nibbles = (0..padded).map(|i| state.encode_ahead(at(i), (i + 1 < padded).then(|| at(i + 1))));
        while let (Some(low), Some(high)) = (nibbles.next(), nibbles.next()) {
            data.push(low | (high << 4));
        }
    }
    let mut b = Vec::with_capacity(60 + data.len());
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(52 + data.len() as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&20u32.to_le_bytes());
    b.extend_from_slice(&0x0011u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    let bytes_per_second = (rate as u64 * BLOCK as u64 / SAMPLES_PER_BLOCK as u64) as u32;
    b.extend_from_slice(&bytes_per_second.to_le_bytes());
    b.extend_from_slice(&(BLOCK as u16).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&(SAMPLES_PER_BLOCK as u16).to_le_bytes());
    b.extend_from_slice(b"fact");
    b.extend_from_slice(&4u32.to_le_bytes());
    b.extend_from_slice(&(samples.len() as u32).to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data.len() as u32).to_le_bytes());
    b.extend_from_slice(&data);
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chirp with silence round it and a loud burst: what a weapon's sound is like.
    fn test_sound(rate: u32) -> Vec<i16> {
        let n = rate as usize;
        (0..n)
            .map(|i| {
                let t = i as f64 / rate as f64;
                let envelope = if i < n / 10 { 0.0 } else if i < n / 5 { 0.9 } else { 0.9 * (1.0 - t) };
                (envelope * (std::f64::consts::TAU * (200.0 + 3000.0 * t) * t).sin() * 30_000.0) as i16
            })
            .collect()
    }

    /// Decoded by the game's own decoder, a sound comes back as it was: as long, at its rate, mono,
    /// and close (the noise 24 dB under a chirp's, 30 under a tone's), and every computer writes the same
    /// bytes. A short sound, a block of one sample, and silence too.
    #[test]
    fn the_game_decodes_what_it_encodes() {
        let samples = test_sound(32_000);
        let bytes = wav(&samples, 32_000);
        assert!(bytes.len() * 3 < samples.len() * 2, "a quarter of PCM's bytes, and the headers");
        let (sound, codec) = ta_audio::decode::wav::decode(&bytes).unwrap();
        assert_eq!(codec, ta_audio::decode::WavCodec::ImaAdpcm);
        assert_eq!((sound.channels(), sound.sample_rate()), (1, 32_000));
        assert!(sound.frames() >= samples.len() && sound.frames() < samples.len() + 8, "{} for {}", sound.frames(), samples.len());
        let (mut signal, mut noise) = (0f64, 0f64);
        for (a, b) in samples.iter().zip(sound.samples()) {
            let (a, b) = (*a as f64, *b as f64 * 32768.0);
            signal += a * a;
            noise += (a - b) * (a - b);
        }
        let snr = 10.0 * (signal / noise.max(1.0)).log10();
        assert!(snr > 24.0, "{snr:.1} dB");
        // (A steady tone closer still.)
        let tone: Vec<i16> = (0..16_000).map(|i| ((i as f64 * std::f64::consts::TAU * 440.0 / 32_000.0).sin() * 12_000.0) as i16).collect();
        let (back, _) = ta_audio::decode::wav::decode(&wav(&tone, 32_000)).unwrap();
        let (mut signal, mut noise) = (0f64, 0f64);
        for (a, b) in tone.iter().zip(back.samples()) {
            let (a, b) = (*a as f64, *b as f64 * 32768.0);
            signal += a * a;
            noise += (a - b) * (a - b);
        }
        let snr = 10.0 * (signal / noise.max(1.0)).log10();
        assert!(snr > 30.0, "a tone: {snr:.1} dB");
        assert_eq!(wav(&samples, 32_000), bytes, "the same bytes again");
        for short in [vec![1234i16], vec![0; 1017], vec![-32768, 32767, -32768, 32767, 0]] {
            let (decoded, _) = ta_audio::decode::wav::decode(&wav(&short, 22_050)).unwrap();
            assert!(decoded.frames() >= short.len(), "{} for {}", decoded.frames(), short.len());
            assert_eq!(decoded.samples()[0], short[0] as f32 / 32768.0, "a block's first sample whole");
        }
    }
}
