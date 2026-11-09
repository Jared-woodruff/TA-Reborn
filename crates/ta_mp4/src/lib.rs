//! Writes MP4 films: H.264 pictures and AAC sound in an MPEG-4 file, through Windows' own Media
//! Foundation (its sink writer, which takes the graphics card's encoder where there is one), for
//! the replays' export (`apps/ta_game/src/export.rs`).
//!
//! The writing runs on a thread of its own. [`Mp4Writer::picture`] hands it a picture (RGBA, the
//! top row first), which it turns into the encoder's NV12 there (BT.709, studio range: what
//! players take a high-definition film to be), and [`Mp4Writer::sound`] the sound that goes with
//! the pictures (interleaved stereo); either waits only while the thread is a few pictures
//! behind. [`Mp4Writer::finish`] completes the file; dropped unfinished, or stopped by an error,
//! the writer leaves no file behind. Elsewhere than Windows, [`Mp4Writer::create`] says it
//! cannot. [`Mp4Writer::create_fast_start`] puts the file's index before its pictures once it
//! is whole (the `moov` box before `mdat`, [`faststart`]: social sites and browsers play such a
//! file while it is still coming).

use std::path::Path;
use std::sync::mpsc;

pub mod faststart;
#[cfg(windows)]
mod mf;

/// How many pictures may wait for the writer's thread (a 4K picture is 33 MB).
const QUEUE: usize = 3;

/// The film's pictures: their size (even numbers of pixels), how many a second, and the bits a
/// second the encoder aims at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pictures {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate: u32,
}

/// The film's sound: samples a second (44,100 or 48,000, stereo) and bits a second (AAC takes 96,
/// 128, 160 or 192 thousand).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sound {
    pub rate: u32,
    pub bitrate: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum Mp4Error {
    #[error("films are written with Windows' Media Foundation, which this system does not have")]
    Unsupported,
    #[error("{0}")]
    Invalid(String),
    #[error("{step}: {message}")]
    Encoder { step: &'static str, message: String },
    #[error("the film's writer stopped: {0}")]
    Stopped(String),
}

/// What the writer's thread is handed.
enum Packet {
    Picture(Vec<u8>),
    Sound(Vec<f32>),
    Finish,
}

/// An MP4 film being written (see the module documentation).
pub struct Mp4Writer {
    /// The film's name, and its private file until it is whole.
    path: std::path::PathBuf,
    partial: PartialFilm,
    pictures: Pictures,
    /// Its index moved before its pictures when it is finished.
    fast_start: bool,
    to: Option<mpsc::SyncSender<Packet>>,
    thread: Option<std::thread::JoinHandle<Result<(), Mp4Error>>>,
}

/// Each encoder owns its temporary file, even when destinations match. Cleanup also
/// runs when startup fails or the encoder thread panics.
struct PartialFilm { path: std::path::PathBuf }

impl PartialFilm {
    #[cfg(windows)]
    fn create(destination: &Path) -> Result<Self, Mp4Error> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = destination.parent().unwrap_or_else(|| Path::new("."));
        for _ in 0..100 {
            let path = parent.join(format!(".ta-film-{}-{}.mp4.part", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(Self { path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {},
                Err(e) => return Err(Mp4Error::Stopped(format!("could not create the film: {e}"))),
            }
        }
        Err(Mp4Error::Stopped("could not reserve a private film file".into()))
    }
}

impl Drop for PartialFilm {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.path); }
}

impl Mp4Writer {
    /// Starts a film at `path` (an existing file is replaced once the film is finished),
    /// with these pictures and, if any, this sound. Fails
    /// at once when the encoder will not take them (a size too large for it, say).
    pub fn create(path: &Path, pictures: Pictures, sound: Option<Sound>) -> Result<Mp4Writer, Mp4Error> {
        Self::open(path, pictures, sound, false)
    }

    /// [`Mp4Writer::create`], the file's index moved before its pictures once it is finished (the
    /// whole file is read and written again then: for short films, a clip's).
    pub fn create_fast_start(path: &Path, pictures: Pictures, sound: Option<Sound>) -> Result<Mp4Writer, Mp4Error> {
        Self::open(path, pictures, sound, true)
    }

    fn open(path: &Path, pictures: Pictures, sound: Option<Sound>, fast_start: bool) -> Result<Mp4Writer, Mp4Error> {
        if pictures.width < 16 || pictures.height < 16 || !pictures.width.is_multiple_of(2) || !pictures.height.is_multiple_of(2) {
            return Err(Mp4Error::Invalid(format!("a film's pictures are an even number of pixels each way, 16 at least (not {} x {})", pictures.width, pictures.height)));
        }
        if !(1..=240).contains(&pictures.fps) || pictures.bitrate == 0 {
            return Err(Mp4Error::Invalid(format!("{} pictures a second at {} bits a second", pictures.fps, pictures.bitrate)));
        }
        if let Some(s) = sound
            && (!matches!(s.rate, 44_100 | 48_000) || !matches!(s.bitrate, 96_000 | 128_000 | 160_000 | 192_000))
        {
            return Err(Mp4Error::Invalid(format!("AAC sound at {} Hz and {} bits a second", s.rate, s.bitrate)));
        }
        #[cfg(windows)]
        {
            let (to, from) = mpsc::sync_channel(QUEUE);
            let (ready_to, ready) = mpsc::channel();
            // (Written beside its name, and moved there once whole: a film cut short by a
            // crash or a closed window is never left under the film's own name.)
            let partial = PartialFilm::create(path)?;
            let writing = partial.path.clone();
            let thread = std::thread::Builder::new()
                .name("mp4 writer".into())
                .spawn(move || mf::run(&writing, pictures, sound, from, ready_to))
                .map_err(|e| Mp4Error::Stopped(e.to_string()))?;
            match ready.recv() {
                Ok(Ok(())) => Ok(Mp4Writer { path: path.to_path_buf(), partial, pictures, fast_start, to: Some(to), thread: Some(thread) }),
                Ok(Err(e)) => {
                    let _ = thread.join();
                    Err(e)
                }
                Err(_) => Err(match thread.join() {
                    Ok(Err(e)) => e,
                    _ => Mp4Error::Stopped("it ended before it began".into()),
                }),
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (path, fast_start);
            Err(Mp4Error::Unsupported)
        }
    }

    pub fn pictures(&self) -> Pictures {
        self.pictures
    }

    /// The next picture: `width` × `height` RGBA pixels, the top row first.
    pub fn picture(&mut self, rgba: Vec<u8>) -> Result<(), Mp4Error> {
        let want = self.pictures.width as usize * self.pictures.height as usize * 4;
        if rgba.len() != want {
            return Err(Mp4Error::Invalid(format!("a picture of {} bytes where {want} were wanted", rgba.len())));
        }
        self.send(Packet::Picture(rgba))
    }

    /// The sound that goes on from the last: interleaved stereo samples, -1 to 1.
    pub fn sound(&mut self, samples: Vec<f32>) -> Result<(), Mp4Error> {
        if samples.is_empty() {
            return Ok(());
        }
        self.send(Packet::Sound(samples))
    }

    fn send(&mut self, packet: Packet) -> Result<(), Mp4Error> {
        let Some(to) = &self.to else { return Err(Mp4Error::Stopped("it was finished".into())) };
        if to.send(packet).is_err() {
            return Err(self.stopped());
        }
        Ok(())
    }

    /// Why the thread ended (it stops at the first error).
    fn stopped(&mut self) -> Mp4Error {
        self.to = None;
        match self.thread.take().map(|t| t.join()) {
            Some(Ok(Err(e))) => e,
            Some(Err(_)) => Mp4Error::Stopped("it failed".into()),
            _ => Mp4Error::Stopped("it ended".into()),
        }
    }

    /// Completes the film: what is waiting is written, then the file's index; returns once the
    /// file is whole.
    pub fn finish(mut self) -> Result<(), Mp4Error> {
        if let Some(to) = self.to.take() {
            let _ = to.send(Packet::Finish);
        }
        let result = match self.thread.take().map(|t| t.join()) {
            Some(Ok(result)) => result,
            Some(Err(_)) => Err(Mp4Error::Stopped("it failed".into())),
            None => Ok(()),
        };
        result?;
        if self.fast_start {
            let bytes = std::fs::read(&self.partial.path).map_err(|e| Mp4Error::Stopped(format!("could not read the film back: {e}")))?;
            if let Some(moved) = faststart::fast_start(&bytes) {
                std::fs::write(&self.partial.path, moved).map_err(|e| Mp4Error::Stopped(format!("could not write the film: {e}")))?;
            }
        }
        std::fs::rename(&self.partial.path, &self.path).map_err(|e| Mp4Error::Stopped(format!("could not name the film: {e}")))
    }
}

impl Drop for Mp4Writer {
    /// Unfinished: the thread finds no more coming, and leaves no file.
    fn drop(&mut self) {
        self.to = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// An RGBA picture (`width` × `height`, even, the top row first) as NV12 into `out` (`width` ×
/// `height` × 3 / 2 bytes): its brightness a byte a pixel, then its colour a pair of bytes (blue
/// and red difference) for each square of four, BT.709 in the studio range (16 to 235, 16 to
/// 240). A band of rows a core.
pub fn rgba_to_nv12(rgba: &[u8], width: usize, height: usize, out: &mut [u8]) {
    use rayon::prelude::*;
    debug_assert!(width.is_multiple_of(2) && height.is_multiple_of(2) && rgba.len() >= width * height * 4 && out.len() >= width * height * 3 / 2);
    let (luma, chroma) = out[..width * height * 3 / 2].split_at_mut(width * height);
    luma.par_chunks_mut(width * 2).zip(chroma.par_chunks_mut(width)).enumerate().for_each(|(pair, (y, uv))| {
        let top = &rgba[pair * 2 * width * 4..][..width * 4];
        let bottom = &rgba[(pair * 2 + 1) * width * 4..][..width * 4];
        let (y_top, y_bottom) = y.split_at_mut(width);
        for (row, line) in [(&mut *y_top, top), (&mut *y_bottom, bottom)] {
            for (y, p) in row.iter_mut().zip(line.as_chunks::<4>().0) {
                let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
                *y = (((47 * r + 157 * g + 16 * b + 128) >> 8) + 16) as u8;
            }
        }
        for (x, uv) in uv.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let at = x * 8;
            let sum = |c: usize| top[at + c] as i32 + top[at + 4 + c] as i32 + bottom[at + c] as i32 + bottom[at + 4 + c] as i32;
            let (r, g, b) = (sum(0), sum(1), sum(2));
            uv[0] = (((-26 * r - 86 * g + 112 * b + 512) >> 10) + 128).clamp(16, 240) as u8;
            uv[1] = (((112 * r - 102 * g - 10 * b + 512) >> 10) + 128).clamp(16, 240) as u8;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nv12_of(colour: [u8; 3]) -> (u8, u8, u8) {
        let rgba: Vec<u8> = (0..4 * 4).flat_map(|_| [colour[0], colour[1], colour[2], 255]).collect();
        let mut out = vec![0u8; 4 * 4 * 3 / 2];
        rgba_to_nv12(&rgba, 4, 4, &mut out);
        assert!(out[..16].iter().all(|&y| y == out[0]), "one brightness for one colour");
        (out[0], out[16], out[17])
    }

    #[test]
    fn colours_come_out_as_bt709_in_the_studio_range() {
        assert_eq!(nv12_of([255, 255, 255]), (235, 128, 128));
        assert_eq!(nv12_of([0, 0, 0]), (16, 128, 128));
        assert_eq!(nv12_of([128, 128, 128]), (126, 128, 128));
        // (BT.709's red, green and blue: 63/102/240, 173/42/26, 32/240/118.)
        let near = |a: (u8, u8, u8), b: (u8, u8, u8)| a.0.abs_diff(b.0) <= 1 && a.1.abs_diff(b.1) <= 1 && a.2.abs_diff(b.2) <= 1;
        assert!(near(nv12_of([255, 0, 0]), (63, 102, 240)), "{:?}", nv12_of([255, 0, 0]));
        assert!(near(nv12_of([0, 255, 0]), (173, 42, 26)), "{:?}", nv12_of([0, 255, 0]));
        assert!(near(nv12_of([0, 0, 255]), (32, 240, 118)), "{:?}", nv12_of([0, 0, 255]));
    }

    #[test]
    fn colour_is_the_mean_of_each_square_of_four() {
        // Columns of white and black: grey colour (no difference), brightness pixel by pixel.
        let (w, h) = (4, 2);
        let rgba: Vec<u8> = (0..w * h).flat_map(|i| if i % 2 == 0 { [255, 255, 255, 255] } else { [0, 0, 0, 255] }).collect();
        let mut out = vec![0u8; w * h * 3 / 2];
        rgba_to_nv12(&rgba, w, h, &mut out);
        assert_eq!(&out[..8], &[235, 16, 235, 16, 235, 16, 235, 16]);
        assert_eq!(&out[8..], &[128, 128, 128, 128]);
    }

    #[test]
    fn odd_sizes_and_unknown_sound_are_refused() {
        let path = std::env::temp_dir().join("ta-mp4-refused.mp4");
        assert!(matches!(Mp4Writer::create(&path, Pictures { width: 641, height: 360, fps: 30, bitrate: 1_000_000 }, None), Err(Mp4Error::Invalid(_))));
        assert!(matches!(Mp4Writer::create(&path, Pictures { width: 640, height: 360, fps: 0, bitrate: 1_000_000 }, None), Err(Mp4Error::Invalid(_))));
        assert!(matches!(Mp4Writer::create(&path, Pictures { width: 640, height: 360, fps: 30, bitrate: 1_000_000 }, Some(Sound { rate: 22_050, bitrate: 128_000 })), Err(Mp4Error::Invalid(_))));
        assert!(!path.exists());
    }

    /// The top-level boxes of an MPEG-4 file, and its length in seconds (from `moov`'s `mvhd`).
    #[cfg(windows)]
    fn boxes_and_length(bytes: &[u8]) -> (Vec<String>, f64) {
        let read = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let (mut boxes, mut at, mut length) = (Vec::new(), 0usize, 0.0);
        while at + 8 <= bytes.len() {
            let (size, kind) = (read(at), String::from_utf8_lossy(&bytes[at + 4..at + 8]).to_string());
            if kind == "moov" {
                let mut inner = at + 8;
                while inner + 8 <= at + size {
                    if &bytes[inner + 4..inner + 8] == b"mvhd" {
                        let version = bytes[inner + 8];
                        let (scale, duration) = if version == 1 {
                            (read(inner + 28), u64::from_be_bytes(bytes[inner + 32..inner + 40].try_into().unwrap()) as f64)
                        } else {
                            (read(inner + 20), read(inner + 24) as f64)
                        };
                        length = duration / scale as f64;
                    }
                    inner += read(inner).max(8);
                }
            }
            boxes.push(kind);
            at += size.max(8);
        }
        (boxes, length)
    }

    #[cfg(windows)]
    #[test]
    fn a_short_film_is_written_whole() {
        let dir = std::env::temp_dir().join(format!("ta-mp4-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("film.mp4");
        let pictures = Pictures { width: 320, height: 180, fps: 30, bitrate: 2_000_000 };
        let sound = Sound { rate: 48_000, bitrate: 192_000 };
        let mut film = Mp4Writer::create(&path, pictures, Some(sound)).expect("Media Foundation writes MP4 on Windows");
        for frame in 0..60u32 {
            let rgba: Vec<u8> = (0..320 * 180u32).flat_map(|i| [((i % 320 + frame * 4) % 256) as u8, (i / 320) as u8, 128, 255]).collect();
            film.picture(rgba).unwrap();
            let samples: Vec<f32> = (0..1600u32).flat_map(|s| {
                let t = (frame * 1600 + s) as f32 / 48_000.0;
                let v = (t * 440.0 * std::f32::consts::TAU).sin() * 0.25;
                [v, v]
            }).collect();
            film.sound(samples).unwrap();
        }
        film.finish().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let (boxes, length) = boxes_and_length(&bytes);
        assert_eq!(boxes.first().map(String::as_str), Some("ftyp"), "{boxes:?}");
        assert!(boxes.iter().any(|b| b == "moov") && boxes.iter().any(|b| b == "mdat"), "{boxes:?}");
        assert!((length - 2.0).abs() < 0.1, "two seconds of film: {length}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn a_fast_start_film_has_its_index_first() {
        let dir = std::env::temp_dir().join(format!("ta-mp4-fast-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clip.mp4");
        let pictures = Pictures { width: 180, height: 320, fps: 30, bitrate: 1_000_000 };
        let mut film = Mp4Writer::create_fast_start(&path, pictures, Some(Sound { rate: 48_000, bitrate: 192_000 })).unwrap();
        for frame in 0..30u32 {
            film.picture((0..180 * 320u32).flat_map(|i| [(i % 180 + frame) as u8, 60, 90, 255]).collect()).unwrap();
            film.sound(vec![0.1; 1600 * 2]).unwrap();
        }
        film.finish().unwrap();
        let (boxes, length) = boxes_and_length(&std::fs::read(&path).unwrap());
        let at = |name: &str| boxes.iter().position(|b| b == name);
        assert!(matches!((at("moov"), at("mdat")), (Some(index), Some(pictures)) if index < pictures), "{boxes:?}");
        assert!((length - 1.0).abs() < 0.1, "a second of film: {length}");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "nothing left beside it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn an_export_does_not_touch_an_existing_partial_file() {
        let dir = std::env::temp_dir().join(format!("ta-mp4-private-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("film.mp4");
        let partial = path.with_extension("mp4.part");
        std::fs::write(&partial, b"another export's unfinished film").unwrap();
        std::fs::write(&path, b"previous finished film").unwrap();
        let film = Mp4Writer::create(&path, Pictures { width: 64, height: 64, fps: 30, bitrate: 500_000 }, None).unwrap();
        drop(film);
        assert_eq!(std::fs::read(&partial).unwrap(), b"another export's unfinished film");
        assert_eq!(std::fs::read(&path).unwrap(), b"previous finished film");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn cancelling_one_export_does_not_interrupt_another_to_the_same_destination() {
        let dir = std::env::temp_dir().join(format!("ta-mp4-concurrent-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("film.mp4");
        let pictures = Pictures { width: 64, height: 64, fps: 30, bitrate: 500_000 };
        let first = Mp4Writer::create(&path, pictures, None).unwrap();
        let mut second = Mp4Writer::create(&path, pictures, None).unwrap();
        drop(first);
        for _ in 0..3 { second.picture(vec![90; 64 * 64 * 4]).unwrap(); }
        second.finish().unwrap();
        let (boxes, _) = boxes_and_length(&std::fs::read(&path).unwrap());
        assert!(boxes.iter().any(|b| b == "moov") && boxes.iter().any(|b| b == "mdat"));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn an_unfinished_film_leaves_no_file() {
        let dir = std::env::temp_dir().join(format!("ta-mp4-left-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("left.mp4");
        let mut film = Mp4Writer::create(&path, Pictures { width: 64, height: 64, fps: 30, bitrate: 500_000 }, None).unwrap();
        film.picture(vec![90; 64 * 64 * 4]).unwrap();
        drop(film);
        assert!(!path.exists());
        assert!(!path.with_extension("mp4.part").exists(), "cancelling must also remove the unfinished export");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
