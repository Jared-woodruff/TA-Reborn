//! The writer's thread: Media Foundation's sink writer, in a COM apartment of its own.

use std::path::Path;
use std::sync::mpsc;

use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::core::{GUID, HSTRING};

use crate::{Mp4Error, Packet, Pictures, Sound};

/// Media Foundation's time: hundreds of nanoseconds.
const SECOND: u64 = 10_000_000;

/// A step that failed, with what Windows said.
fn at(step: &'static str) -> impl FnOnce(windows::core::Error) -> Mp4Error {
    move |e| Mp4Error::Encoder { step, message: e.message().trim().to_string() }
}

/// Two 32-bit halves of a Media Foundation size or ratio.
fn pair(high: u32, low: u32) -> u64 {
    ((high as u64) << 32) | low as u64
}

/// The thread's life: the writer opened (and said so through `ready`), what comes written, the
/// film completed or, when it stops short, removed.
pub(crate) fn run(path: &Path, pictures: Pictures, sound: Option<Sound>, from: mpsc::Receiver<Packet>, ready: mpsc::Sender<Result<(), Mp4Error>>) -> Result<(), Mp4Error> {
    // SAFETY: COM and Media Foundation are started and stopped on this thread alone, round
    // everything that uses them.
    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    let started = unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }.map_err(at("starting Media Foundation"));
    let result = match started {
        Err(e) => {
            let _ = ready.send(Err(e));
            Ok(())
        }
        Ok(()) => {
            let result = write(path, pictures, sound, &from, &ready);
            // SAFETY: every Media Foundation object made here was dropped inside `write`.
            unsafe {
                let _ = MFShutdown();
            }
            result
        }
    };
    if com {
        // SAFETY: balances the successful `CoInitializeEx` above, on the same thread.
        unsafe { CoUninitialize() };
    }
    result
}

fn write(path: &Path, pictures: Pictures, sound: Option<Sound>, from: &mpsc::Receiver<Packet>, ready: &mpsc::Sender<Result<(), Mp4Error>>) -> Result<(), Mp4Error> {
    // The graphics card's encoder where Windows has one, else its own.
    let opened = Writer::open(path, pictures, sound, true).or_else(|e| {
        log::info!("film: no hardware encoder for {} x {} ({e}); using Windows' own", pictures.width, pictures.height);
        let _ = std::fs::remove_file(path);
        Writer::open(path, pictures, sound, false)
    });
    let mut writer = match opened {
        Ok(w) => {
            let _ = ready.send(Ok(()));
            w
        }
        Err(e) => {
            let _ = std::fs::remove_file(path);
            let _ = ready.send(Err(e));
            return Ok(());
        }
    };
    let mut result = Err(Mp4Error::Stopped("the film was left unfinished".into()));
    for packet in from.iter() {
        let step = match packet {
            Packet::Picture(rgba) => writer.picture(&rgba),
            Packet::Sound(samples) => writer.sound(&samples),
            Packet::Finish => {
                result = writer.finish();
                break;
            }
        };
        if let Err(e) = step {
            result = Err(e);
            break;
        }
    }
    drop(writer);
    match result {
        Ok(()) => Ok(()),
        // (Left unfinished on purpose: no error, no file.)
        Err(Mp4Error::Stopped(_)) => {
            let _ = std::fs::remove_file(path);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(path);
            Err(e)
        }
    }
}

struct Writer {
    sink: IMFSinkWriter,
    video: u32,
    audio: Option<(u32, Sound)>,
    pictures: Pictures,
    /// Pictures and sound frames written so far.
    frames: u64,
    samples: u64,
}

impl Writer {
    fn open(path: &Path, pictures: Pictures, sound: Option<Sound>, hardware: bool) -> Result<Writer, Mp4Error> {
        // SAFETY: plain Media Foundation calls on objects made here, on the thread that started
        // it; every pointer passed lives through the call.
        unsafe {
            let mut attributes: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut attributes, 3).map_err(at("preparing the writer"))?;
            let attributes = attributes.ok_or(Mp4Error::Stopped("no attributes".into()))?;
            attributes.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, hardware as u32).map_err(at("preparing the writer"))?;
            attributes.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1).map_err(at("preparing the writer"))?;
            attributes.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4).map_err(at("preparing the writer"))?;
            let sink = MFCreateSinkWriterFromURL(&HSTRING::from(path.as_os_str()), None::<&IMFByteStream>, &attributes).map_err(at("creating the film's file"))?;

            // The pictures: H.264 out, NV12 in.
            let frame = |t: &IMFMediaType| -> windows::core::Result<()> {
                t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                t.SetUINT64(&MF_MT_FRAME_SIZE, pair(pictures.width, pictures.height))?;
                t.SetUINT64(&MF_MT_FRAME_RATE, pair(pictures.fps, 1))?;
                t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pair(1, 1))?;
                t.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)?;
                t.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32)?;
                t.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32)?;
                t.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)
            };
            let out = MFCreateMediaType().map_err(at("describing the pictures"))?;
            frame(&out).map_err(at("describing the pictures"))?;
            out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264).map_err(at("describing the pictures"))?;
            out.SetUINT32(&MF_MT_AVG_BITRATE, pictures.bitrate).map_err(at("describing the pictures"))?;
            out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32).map_err(at("describing the pictures"))?;
            let video = sink.AddStream(&out).map_err(at("adding the pictures"))?;
            let input = MFCreateMediaType().map_err(at("describing the pictures"))?;
            frame(&input).map_err(at("describing the pictures"))?;
            input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12).map_err(at("describing the pictures"))?;
            input.SetUINT32(&MF_MT_DEFAULT_STRIDE, pictures.width).map_err(at("describing the pictures"))?;
            sink.SetInputMediaType(video, &input, None::<&IMFAttributes>).map_err(at("the H.264 encoder taking these pictures"))?;

            // The sound: AAC out, 16-bit PCM in.
            let audio = match sound {
                None => None,
                Some(s) => {
                    let pcm = |t: &IMFMediaType, subtype: &GUID| -> windows::core::Result<()> {
                        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
                        t.SetGUID(&MF_MT_SUBTYPE, subtype)?;
                        t.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
                        t.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, s.rate)?;
                        t.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)
                    };
                    let out = MFCreateMediaType().map_err(at("describing the sound"))?;
                    pcm(&out, &MFAudioFormat_AAC).map_err(at("describing the sound"))?;
                    out.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, s.bitrate / 8).map_err(at("describing the sound"))?;
                    out.SetUINT32(&MF_MT_AAC_AUDIO_PROFILE_LEVEL_INDICATION, 0x29).map_err(at("describing the sound"))?;
                    let stream = sink.AddStream(&out).map_err(at("adding the sound"))?;
                    let input = MFCreateMediaType().map_err(at("describing the sound"))?;
                    pcm(&input, &MFAudioFormat_PCM).map_err(at("describing the sound"))?;
                    input.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4).map_err(at("describing the sound"))?;
                    input.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, s.rate * 4).map_err(at("describing the sound"))?;
                    sink.SetInputMediaType(stream, &input, None::<&IMFAttributes>).map_err(at("the AAC encoder taking this sound"))?;
                    Some((stream, s))
                }
            };
            sink.BeginWriting().map_err(at("starting the film"))?;
            Ok(Writer { sink, video, audio, pictures, frames: 0, samples: 0 })
        }
    }

    /// A buffer of `len` bytes filled by `fill`, as a sample from `time` for `duration` (in
    /// Media Foundation's time), written to `stream`.
    fn write(&self, stream: u32, len: usize, time: u64, duration: u64, fill: impl FnOnce(&mut [u8])) -> Result<(), Mp4Error> {
        // SAFETY: the buffer is locked while `fill` writes into its `len` bytes (it was made that
        // large), and unlocked before it is handed on.
        unsafe {
            let buffer = MFCreateMemoryBuffer(len as u32).map_err(at("making room for a sample"))?;
            let mut data: *mut u8 = std::ptr::null_mut();
            buffer.Lock(&mut data, None, None).map_err(at("making room for a sample"))?;
            fill(std::slice::from_raw_parts_mut(data, len));
            buffer.Unlock().map_err(at("making room for a sample"))?;
            buffer.SetCurrentLength(len as u32).map_err(at("making room for a sample"))?;
            let sample = MFCreateSample().map_err(at("making a sample"))?;
            sample.AddBuffer(&buffer).map_err(at("making a sample"))?;
            sample.SetSampleTime(time as i64).map_err(at("making a sample"))?;
            sample.SetSampleDuration(duration as i64).map_err(at("making a sample"))?;
            self.sink.WriteSample(stream, &sample).map_err(at("writing to the film"))
        }
    }

    fn picture(&mut self, rgba: &[u8]) -> Result<(), Mp4Error> {
        let (w, h, fps) = (self.pictures.width as usize, self.pictures.height as usize, self.pictures.fps as u64);
        let (from, to) = (self.frames * SECOND / fps, (self.frames + 1) * SECOND / fps);
        self.write(self.video, w * h * 3 / 2, from, to - from, |out| crate::rgba_to_nv12(rgba, w, h, out))?;
        self.frames += 1;
        Ok(())
    }

    fn sound(&mut self, samples: &[f32]) -> Result<(), Mp4Error> {
        let Some((stream, s)) = self.audio else { return Ok(()) };
        let frames = (samples.len() / 2) as u64;
        let rate = s.rate as u64;
        let (from, to) = (self.samples * SECOND / rate, (self.samples + frames) * SECOND / rate);
        self.write(stream, frames as usize * 4, from, to - from, |out| {
            for (o, v) in out.as_chunks_mut::<2>().0.iter_mut().zip(samples) {
                *o = ((v.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes();
            }
        })?;
        self.samples += frames;
        Ok(())
    }

    fn finish(&mut self) -> Result<(), Mp4Error> {
        // SAFETY: the writer is whole; this writes what it holds and the file's index.
        unsafe { self.sink.Finalize() }.map_err(at("completing the film"))
    }
}
