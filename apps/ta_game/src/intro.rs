//! Cutscene playback (TA's Smacker `.ZRB` movies) shown through egui.
//!
//! The audio track is decoded up front on a worker thread (a few hundred milliseconds in
//! release builds) and handed to the audio engine as one sound; video frames are then
//! decoded on demand in step with the wall clock, so picture and sound stay in sync.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use ta_video::SmackerVideo;

pub struct MoviePlayer {
    video: SmackerVideo,
    texture: Option<egui::TextureHandle>,
    audio_rx: Option<mpsc::Receiver<Option<ta_audio::SoundData>>>,
    started: Option<Instant>,
    shown_frame: usize,
    pub finished: bool,
    name: String,
    /// Frames go to the intro room's tube (see `take_frame`), not to a texture of their own.
    pub for_room: bool,
    fresh: Option<(u32, u32, Vec<u8>)>,
}

impl MoviePlayer {
    /// Opens a movie from the install's `Data` folder (loose files, not in archives).
    pub fn open(bytes: Vec<u8>, name: &str) -> Option<MoviePlayer> {
        let video = SmackerVideo::open(bytes.clone()).map_err(|e| log::warn!("movie {name}: {e}")).ok()?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(decode_audio(bytes));
        });
        Some(MoviePlayer { video, texture: None, audio_rx: Some(rx), started: None, shown_frame: usize::MAX, finished: false, name: name.to_string(), for_room: false, fresh: None })
    }

    /// Advances playback; call every frame. Starts the soundtrack once decoded.
    pub fn update(&mut self, ctx: &egui::Context, audio: &mut crate::audio::GameAudio) {
        if self.finished {
            return;
        }
        if let Some(rx) = &self.audio_rx {
            match rx.try_recv() {
                Ok(sound) => {
                    if let Some(s) = sound {
                        audio.play_movie_audio(&self.name, s);
                    }
                    self.audio_rx = None;
                    self.started = Some(Instant::now());
                }
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.audio_rx = None;
                    self.started = Some(Instant::now());
                }
            }
        }
        let Some(start) = self.started else { return };
        let frame_time = self.video.frame_duration().max(Duration::from_millis(1));
        let target = (start.elapsed().as_secs_f64() / frame_time.as_secs_f64()) as usize;
        if target >= self.video.frame_count() {
            self.finished = true;
            return;
        }
        // Decode up to the frame due now (skipping display of late frames).
        let mut latest = None;
        while self.video.position() <= target {
            match self.video.next_frame() {
                Ok(Some(f)) => latest = Some(f),
                Ok(None) => {
                    self.finished = true;
                    break;
                }
                Err(e) => log::debug!("movie frame error: {e}"),
            }
        }
        if let Some(f) = latest {
            if self.for_room {
                self.fresh = Some((f.width, f.display_height(), f.to_rgba()));
                self.shown_frame = target;
                return;
            }
            let img = egui::ColorImage::from_rgba_unmultiplied([f.width as usize, f.display_height() as usize], &f.to_rgba());
            match &mut self.texture {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => self.texture = Some(ctx.load_texture("movie", img, egui::TextureOptions::LINEAR)),
            }
            self.shown_frame = target;
        }
    }

    /// The newest frame since the last call (width, height, RGBA), when playing for the room.
    pub fn take_frame(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        self.fresh.take()
    }

    /// A picture is showing (the soundtrack has started and the movie has not ended).
    pub fn showing(&self) -> bool {
        self.started.is_some() && !self.finished && self.shown_frame != usize::MAX
    }

    /// Draws the movie letterboxed to fill the screen.
    pub fn draw(&self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, egui::Color32::BLACK);
        if let Some(t) = &self.texture {
            let size = t.size_vec2();
            let scale = (rect.width() / size.x).min(rect.height() / size.y);
            let r = egui::Rect::from_center_size(rect.center(), size * scale);
            ui.painter().image(t.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
    }
}

fn decode_audio(bytes: Vec<u8>) -> Option<ta_audio::SoundData> {
    // A movie is decoded as a whole soundtrack. Per-chunk limits in ta_video alone do
    // not bound repeated, highly compressed chunks in a damaged or custom movie.
    const MAX_SAMPLES: usize = (256 << 20) / std::mem::size_of::<f32>();
    decode_audio_with_limit(bytes, MAX_SAMPLES)
}

fn decode_audio_with_limit(bytes: Vec<u8>, max_samples: usize) -> Option<ta_audio::SoundData> {
    let mut v = SmackerVideo::open(bytes).ok()?;
    let info = v.audio_tracks().first()?.clone();
    let mut samples: Vec<f32> = Vec::new();
    while let Ok(Some(f)) = v.next_frame() {
        if let Some(track) = f.audio.first() {
            if track.len() > max_samples.saturating_sub(samples.len()) {
                log::warn!("movie soundtrack exceeds the decoded audio memory limit; playing silently");
                return None;
            }
            // Keep geometric growth within the cap and return silence if allocation fails.
            let needed = samples.len() + track.len();
            if needed > samples.capacity() {
                let capacity = needed.max(samples.capacity().saturating_mul(2)).min(max_samples);
                samples.try_reserve_exact(capacity - samples.len()).ok()?;
            }
            samples.extend(track.iter().map(|&s| s as f32 / 32768.0));
        }
    }
    ta_audio::SoundData::new(info.sample_rate, info.channels, samples).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each nine-byte chunk expands to a caller-selected number of silent 8-bit samples.
    /// Four absent video trees encode a black 4x4 picture without a video payload.
    fn silent_movie(frames: u32, samples_per_frame: u32) -> Vec<u8> {
        let mut bytes = vec![0u8; 104];
        bytes[..4].copy_from_slice(b"SMK2");
        for (offset, value) in [(4, 4), (8, 4), (12, frames), (16, 100),
            (24, samples_per_frame), (52, 1), (72, 0xC000_5622)]
        {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        for _ in 0..frames { bytes.extend_from_slice(&12u32.to_le_bytes()); }
        bytes.extend(std::iter::repeat_n(2, frames as usize)); // audio track 0
        bytes.push(0); // absent Huffman trees
        for _ in 0..frames {
            bytes.extend_from_slice(&9u32.to_le_bytes());
            bytes.extend_from_slice(&samples_per_frame.to_le_bytes());
            bytes.extend_from_slice(&[0; 4]); // silent chunk + alignment
        }
        bytes
    }

    #[test]
    fn movie_audio_limits_total_pcm_across_small_compressed_frames() {
        let bytes = silent_movie(3, 8);
        assert!(bytes.len() < 200);
        let sound = decode_audio_with_limit(bytes.clone(), 24).unwrap();
        assert_eq!((sound.frames(), sound.channels(), sound.sample_rate()), (24, 1, 22_050));
        assert!(sound.samples().iter().all(|s| *s == 0.0));
        assert!(decode_audio_with_limit(bytes.clone(), 23).is_none(),
            "the final frame must not grow the soundtrack past its aggregate limit");
        assert!(decode_audio_with_limit(bytes, 7).is_none(),
            "a single oversized chunk must not enter the accumulation buffer");
    }
}
