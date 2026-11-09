//! A test film: colour bars that slide and a tone, to try the encoder at a size and rate.
//!
//!     cargo run --release -p ta_mp4 --example film -- out.mp4 [width height fps seconds]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = std::path::PathBuf::from(args.first().map_or("film.mp4", String::as_str));
    let number = |i: usize, default: u32| args.get(i).and_then(|v| v.parse().ok()).unwrap_or(default);
    let (width, height, fps, seconds) = (number(1, 1920), number(2, 1080), number(3, 60), number(4, 5));
    let pictures = ta_mp4::Pictures { width, height, fps, bitrate: (width as u64 * height as u64 * fps as u64 / 8).min(80_000_000) as u32 };
    let sound = ta_mp4::Sound { rate: 48_000, bitrate: 192_000 };
    let started = std::time::Instant::now();
    let mut film = match ta_mp4::Mp4Writer::create(&path, pictures, Some(sound)) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let bars = [[192, 192, 192], [192, 192, 0], [0, 192, 192], [0, 192, 0], [192, 0, 192], [192, 0, 0], [0, 0, 192], [16, 16, 16]];
    let per_frame = 48_000 / fps as usize;
    for frame in 0..fps * seconds {
        let shift = (frame * 4) as usize;
        let mut rgba = vec![0u8; width as usize * height as usize * 4];
        for (i, p) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let x = (i % width as usize + shift) % width as usize;
            let c = bars[x * bars.len() / width as usize];
            *p = [c[0], c[1], c[2], 255];
        }
        film.picture(rgba).expect("picture");
        let samples: Vec<f32> = (0..per_frame)
            .flat_map(|s| {
                let t = (frame as usize * per_frame + s) as f32 / 48_000.0;
                let v = (t * 440.0 * std::f32::consts::TAU).sin() * 0.2;
                [v, v]
            })
            .collect();
        film.sound(samples).expect("sound");
    }
    film.finish().expect("finish");
    println!("{} ({width} x {height}, {fps} a second, {seconds} s) in {:.1} s", path.display(), started.elapsed().as_secs_f32());
}
