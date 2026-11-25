//! A faction's emblem in the classic interface, as TA's sides have theirs (`anims/logos.gaf`'s
//! `Arm32Lt` and `Core32Lt`, at the top left and in the unit bar): ten frames of 32 by 32 pixels,
//! one for each player's colour, the emblem filled with the player's colour and outlined dark on a
//! ground of polished metal, a pixel each as TA draws its own (no colour between), in TA's
//! palette. Made of Forged Alliance's emblem of the faction
//! (`textures/ui/common/faction_icon-lg/<faction>_ico.dds`: 60 by 60, its shape in its alpha) into
//! the mod's own GAF ([`GAF`], an entry `<SIDE>32Lt` a side: [`entry`]), which the side add-on names
//! (`logogaf` and `logo`: `ta_data::SideDef::emblem`).
//!
//! In whole numbers, but for the colours' distances as `ta_mapart::tiles` takes them (with
//! [`Portable`]'s arithmetic): the same frames from the same copy on every computer.

use std::collections::BTreeMap;

use ta_formats::dds;
use ta_formats::gaf::{Gaf, GafEntry, GafFrame};
use ta_formats::image::{IndexedImage, Palette};
use ta_mapart::tiles::{Colour, Portable};

use crate::Result;
use crate::banner::kernel;
use crate::files::Files;

/// The GAF of the emblems, in the mod's `anims/`, as the side add-on names it (`logogaf`).
pub const GAF: &str = "supcomlogos";

/// Where the GAF is in the mod.
pub fn path() -> String {
    format!("anims/{GAF}.gaf")
}

/// The entry of side `side`'s emblem in the GAF (`logo`): `UEF32Lt`, as TA names Arm's `Arm32Lt`.
pub fn entry(side: &str) -> String {
    format!("{side}32Lt")
}

/// A frame's side: TA's emblems' (32 pixels square).
pub const SIDE: u32 = 32;
/// How far across (or down) the emblem's shape is drawn at most: its outline round it, and a pixel
/// of the ground beyond that.
const FIT: u32 = SIDE - 4;
/// Where the shape's alpha counts when its bounds are looked for (a speck of its compression's
/// noise does not).
const SOLID: u8 = 32;
/// Whole coverage.
const ONE: u32 = 65535;
/// The coverage from which a pixel is the shape's: half.
const HALF: u32 = ONE / 2;

/// TA's ten player colours in its palette, a frame a player in TA's order (blue, red, white,
/// green, navy, purple, yellow, black, sky blue, khaki): the colours TA fills its emblems with
/// (docs/FORMATS.md, "In-game interface").
pub const PLAYERS: [u8; 10] = [227, 212, 255, 235, 108, 219, 208, 0, 130, 67];
/// The index TA's GAFs keep for see-through (no pixel of an emblem is).
const CLEAR: u8 = 9;
/// The palette's entries the other pixels are matched to: those TA keeps for its art, as the maps'
/// tiles are (not the first and last sixteen, the system's).
const ART: std::ops::Range<usize> = 16..240;

/// The emblems' GAF: an entry for each of `sides` (as the mod names them: `UEF`, `CYBRAN`, `AEON`,
/// `SERAPHIM`) whose emblem the copy has and reads, in their order, its frames in `palette`
/// (TA's); none where none does.
pub fn gaf(files: &Files, sides: &[&str], palette: &Palette) -> Result<Option<Vec<u8>>> {
    let mut near = Nearest::new(palette);
    let mut entries = Vec::new();
    for side in sides {
        let Some(path) = crate::banner::emblem_path(side) else { continue };
        // (A picture the copy has not, that does not read, or shows nothing, makes no emblem: the
        // side keeps TA's plain square of the player's colour, the rest of the mod made.)
        let bytes = match files.read(path) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => continue,
            Err(e) => {
                log::warn!("{path}: {e}: no emblem for the {side}");
                continue;
            }
        };
        if let Some(frames) = frames_in(&bytes, palette, &mut near) {
            entries.push(GafEntry { name: entry(side), frames });
        }
    }
    if entries.is_empty() {
        return Ok(None);
    }
    Ok(Some(Gaf { entries }.write()?))
}

/// The ten frames of the emblem `icon` (a DDS, its shape in its alpha) in `palette`, a frame for
/// each of [`PLAYERS`]: none for a picture that does not read or shows nothing.
pub fn frames(icon: &[u8], palette: &Palette) -> Option<Vec<GafFrame>> {
    frames_in(icon, palette, &mut Nearest::new(palette))
}

fn frames_in(icon: &[u8], palette: &Palette, near: &mut Nearest) -> Option<Vec<GafFrame>> {
    let body: Vec<bool> = coverage(&dds::decode(icon, 0).ok()?)?.into_iter().map(|c| c >= HALF).collect();
    if !body.contains(&true) {
        return None;
    }
    Some((0..PLAYERS.len()).map(|player| frame(&body, player, palette, near)).collect())
}

/// The emblem's shape at a frame's size: each pixel's coverage (0 to [`ONE`]), the bounds of its
/// alpha (where it is [`SOLID`]) made [`FIT`] pixels across or down, in proportion, in the middle
/// of the frame, each pixel the mean of what it covers. None for a picture showing nothing.
fn coverage(image: &dds::Image) -> Option<Vec<u32>> {
    let (w, h) = (image.width as usize, image.height as usize);
    if w == 0 || h == 0 || image.rgba.len() != w * h {
        return None;
    }
    let solid = |x: usize, y: usize| image.rgba[y * w + x][3] >= SOLID;
    let columns: Vec<usize> = (0..w).filter(|&x| (0..h).any(|y| solid(x, y))).collect();
    let rows: Vec<usize> = (0..h).filter(|&y| (0..w).any(|x| solid(x, y))).collect();
    let (&x0, &x1, &y0, &y1) = (columns.first()?, columns.last()?, rows.first()?, rows.last()?);
    let (cw, ch) = ((x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32);
    let long = cw.max(ch);
    let (tw, th) = (((cw * FIT + long / 2) / long).max(1), ((ch * FIT + long / 2) / long).max(1));
    // (Across, row by row; then down.)
    let mut across = vec![0u32; tw as usize * ch as usize];
    let kx = kernel(cw, tw);
    for y in 0..ch as usize {
        for (x, (first, weights)) in kx.iter().enumerate() {
            let sum: u64 = weights.iter().enumerate().map(|(n, &wt)| image.rgba[(y0 + y) * w + x0 + first + n][3] as u64 * 257 * wt as u64).sum();
            across[y * tw as usize + x] = ((sum + 32_768) >> 16) as u32;
        }
    }
    let (ox, oy) = (((SIDE - tw) / 2) as usize, ((SIDE - th) / 2) as usize);
    let mut out = vec![0u32; (SIDE * SIDE) as usize];
    for (y, (first, weights)) in kernel(ch, th).iter().enumerate() {
        for x in 0..tw as usize {
            let sum: u64 = weights.iter().enumerate().map(|(n, &wt)| across[(first + n) * tw as usize + x] as u64 * wt as u64).sum();
            out[(oy + y) * SIDE as usize + ox + x] = (((sum + 32_768) >> 16) as u32).min(ONE);
        }
    }
    Some(out)
}

/// Whether pixel `i` is on the emblem's outline: not the shape's, but beside a pixel of it (across
/// or down: a line a pixel wide, its corners cut).
fn outline(body: &[bool], i: usize) -> bool {
    let (s, x, y) = (SIDE as usize, i % SIDE as usize, i / SIDE as usize);
    !body[i] && ((x > 0 && body[i - 1]) || (x + 1 < s && body[i + 1]) || (y > 0 && body[i - s]) || (y + 1 < s && body[i + s]))
}

/// The ground's grey at row `y`: polished metal lit from above, light at its top and darkening to
/// its middle, where a brighter band (a horizon in it) darkens again to its foot.
fn ground(y: u32) -> u32 {
    let half = SIDE / 2;
    if y < half { 232 - 56 * y / (half - 1) } else { 206 - 70 * (y - half) / (half - 1) }
}

/// Frame `player` of the emblem (`body`: the pixels its shape covers half of or more): the shape in
/// the player's colour (its entry, [`PLAYERS`]), its [`outline`] dark (lighter round a colour dark
/// itself, black or navy), the rest the [`ground`]: those two in the entries of `palette` nearest
/// them.
fn frame(body: &[bool], player: usize, palette: &Palette, near: &mut Nearest) -> GafFrame {
    let index = PLAYERS[player % PLAYERS.len()];
    let fill = palette.colors[index as usize].map(u32::from);
    let light = (2126 * fill[0] + 7152 * fill[1] + 722 * fill[2]) / 10_000;
    let line = near.index([if light < 48 { 112 } else { 52 }; 3]);
    let mut image = IndexedImage::new(SIDE, SIDE, CLEAR);
    for (i, &b) in body.iter().enumerate() {
        image.pixels[i] = if b {
            index
        } else if outline(body, i) {
            line
        } else {
            near.index([ground(i as u32 / SIDE) as u8; 3])
        };
        image.opaque[i] = true;
    }
    GafFrame { image, origin_x: 0, origin_y: 0, transparency_index: CLEAR, duration: 0 }
}

/// The palette's entry nearest each colour among [`ART`]'s, by OKLab as `ta_mapart::tiles`'
/// `PaletteLut` weighs it (taken there with [`Portable`]'s arithmetic), each colour looked for
/// once; the first of entries as near.
struct Nearest {
    entries: Vec<(u8, [f32; 3])>,
    found: BTreeMap<[u8; 3], u8>,
}

impl Nearest {
    fn new(palette: &Palette) -> Nearest {
        let entries = ART.map(|i| (i as u8, Portable::oklab(palette.colors[i].map(|v| v as f32 / 255.0)))).collect();
        Nearest { entries, found: BTreeMap::new() }
    }

    fn index(&mut self, c: [u8; 3]) -> u8 {
        if let Some(&i) = self.found.get(&c) {
            return i;
        }
        let l = Portable::oklab(c.map(|v| v as f32 / 255.0));
        let mut best = (f32::MAX, self.entries[0].0);
        for &(i, e) in &self.entries {
            let (dl, da, db) = (l[0] - e[0], l[1] - e[1], l[2] - e[2]);
            let d = dl * dl + da * da * 1.4 + db * db * 1.4;
            if d < best.0 {
                best = (d, i);
            }
        }
        self.found.insert(c, best.1);
        best.1
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A made-up palette (TA's is the player's own): a ramp of greys where TA keeps its, from white
    /// down, the rest spread through the colours.
    pub(crate) fn palette() -> Palette {
        let mut colors = [[0u8; 3]; 256];
        for (i, c) in colors.iter_mut().enumerate() {
            let i = i as u32;
            *c = [(i * 37 % 256) as u8, (i * 91 % 256) as u8, (i * 53 % 256) as u8];
        }
        for k in 0..16 {
            colors[80 + k] = [(255 - 16 * k) as u8; 3];
        }
        Palette { colors }
    }

    /// An emblem as the game keeps them: an uncompressed DDS of 60 by 60, white, its shape (in its
    /// alpha) a square `side` across in its middle, with a hole `hole` across in the square's.
    pub(crate) fn icon(side: i32, hole: i32) -> Vec<u8> {
        let mut file = dds::header(60, 60, 1, None);
        let within = |v: i32, across: i32| (30 - across / 2..30 + across - across / 2).contains(&v);
        for y in 0..60 {
            for x in 0..60 {
                let inside = within(x, side) && within(y, side) && !(within(x, hole) && within(y, hole));
                file.extend_from_slice(&[255, 255, 255, if inside { 255 } else { 0 }]);
            }
        }
        file
    }

    /// The pixel (x, y) of a frame.
    fn at(f: &GafFrame, x: u32, y: u32) -> u8 {
        f.image.pixels[(y * SIDE + x) as usize]
    }

    /// Ten frames of TA's size, every pixel shown: the shape in each player's colour, exactly,
    /// [`FIT`] pixels across in the middle (a square of 56 made 28, its hole of 8 made 4); the
    /// ground round it light metal, darker at its foot; the outline between them a pixel wide,
    /// dark (lighter round the black player's colour); the ground through the hole, outlined.
    #[test]
    fn an_emblem_is_ten_frames_in_the_players_colours() {
        let palette = palette();
        let frames = frames(&icon(56, 8), &palette).expect("an emblem");
        assert_eq!(frames.len(), 10);
        let grey = |i: u8| palette.colors[i as usize].iter().map(|&v| v as u32).sum::<u32>() / 3;
        for (p, f) in frames.iter().enumerate() {
            assert_eq!((f.image.width, f.image.height), (SIDE, SIDE));
            assert!(f.image.opaque.iter().all(|&o| o), "frame {p}: no pixel see-through");
            // (The square from 2 to 29 each way, its hole from 14 to 17.)
            for (x, y) in [(2, 2), (29, 29), (8, 16), (13, 16), (18, 16), (16, 29)] {
                assert_eq!(at(f, x, y), PLAYERS[p], "frame {p}: the shape in the player's colour at {x},{y}");
            }
            // (Its corners the ground, light above, darker below.)
            let (top, foot) = (grey(at(f, 0, 0)), grey(at(f, 0, 31)));
            assert!(top > 200 && (110..160).contains(&foot), "frame {p}: the ground {top}, {foot}");
            // (The hole's middle the ground; its rim, and the square's, the outline.)
            assert_eq!(at(f, 16, 16), at(f, 0, 16), "frame {p}: the ground through the hole");
            for (x, y) in [(1, 16), (30, 16), (16, 1), (14, 16), (17, 15)] {
                let line = grey(at(f, x, y));
                if p == 7 {
                    assert!((90..140).contains(&line), "the black player's outline lighter: {line} at {x},{y}");
                } else {
                    assert!(line < 80, "frame {p}: a dark outline at {x},{y}: {line}");
                }
            }
        }
        // (The same frames each time.)
        let again = super::frames(&icon(56, 8), &palette).unwrap();
        assert!(frames.iter().zip(&again).all(|(a, b)| a.image == b.image));
        // A picture that shows nothing, or is no picture, makes none.
        assert!(super::frames(&icon(0, 0), &palette).is_none() && super::frames(b"not a picture", &palette).is_none());
    }

    /// A shape wider than it is tall is drawn [`FIT`] across, in proportion down, in the middle.
    #[test]
    fn a_shape_keeps_its_proportions() {
        let mut file = dds::header(60, 60, 1, None);
        for y in 0..60 {
            for x in 0..60 {
                file.extend_from_slice(&[255, 255, 255, if (5..55).contains(&x) && (20..45).contains(&y) { 255 } else { 0 }]);
            }
        }
        let f = &frames(&file, &palette()).unwrap()[0];
        let filled: Vec<(u32, u32)> = (0..SIDE * SIDE).filter(|&i| f.image.pixels[i as usize] == PLAYERS[0]).map(|i| (i % SIDE, i / SIDE)).collect();
        let (xs, ys): (Vec<u32>, Vec<u32>) = filled.iter().copied().unzip();
        assert_eq!((xs.iter().min(), xs.iter().max(), ys.iter().min(), ys.iter().max()), (Some(&2), Some(&29), Some(&9), Some(&22)), "50 by 25 made 28 by 14");
    }

    /// The GAF holds an entry for each side whose emblem the copy has, in their order, named as
    /// the side add-on names them, and reads back as written; none without any.
    #[test]
    fn the_gaf_holds_an_entry_a_side() {
        let dir = crate::tests::Temp::new("emblems");
        let (uef, cybran) = (icon(26, 0), icon(20, 6));
        let entries: Vec<(&str, &[u8], bool)> = vec![
            (crate::banner::emblem_path("CYBRAN").unwrap(), &cybran, true),
            (crate::banner::emblem_path("UEF").unwrap(), &uef, true),
        ];
        std::fs::write(dir.0.join("textures.scd"), crate::scd::tests::zip(&entries)).unwrap();
        let files = Files::of(Vec::new(), vec![crate::scd::Scd::open(&dir.0.join("textures.scd")).unwrap()]);
        let palette = palette();
        let bytes = gaf(&files, &["UEF", "AEON", "CYBRAN"], &palette).unwrap().expect("made");
        let back = Gaf::parse(&bytes).unwrap();
        assert_eq!(back.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["UEF32Lt", "CYBRAN32Lt"], "the Aeon's not in the copy");
        assert!(back.entries.iter().all(|e| e.frames.len() == 10));
        let made = frames(&uef, &palette).unwrap();
        assert!(back.entries[0].frames.iter().zip(&made).all(|(a, b)| a.image == b.image), "as made");
        assert_eq!(gaf(&files, &["UEF", "AEON", "CYBRAN"], &palette).unwrap(), Some(bytes), "the same bytes each time");
        assert_eq!(gaf(&files, &["AEON"], &palette).unwrap(), None);
    }
}
