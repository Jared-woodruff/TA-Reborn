//! What may be wrong with a map before it is saved or played: a name another map has, start
//! positions missing, off the map, under the sea or on top of one another, the whole map under
//! water, the tiles running out.

use std::path::Path;

use ta_formats::tnt::MAX_TILES;
use ta_ui::i18n::{t, tf};
use ta_vfs::Vfs;

use super::doc::MapDoc;

/// Closer than this (pixels, four tiles), two start positions share a base: their Commanders
/// start on top of each other.
const CROWDED: f32 = 128.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    /// What is wrong, translated.
    pub text: String,
    /// Where on the map (a click on it takes the camera there).
    pub at: Option<[f32; 2]>,
    /// The map can't be saved until it is put right (else it is a warning).
    pub blocking: bool,
}

impl Problem {
    fn warning(text: String, at: Option<[f32; 2]>) -> Problem {
        Problem { text, at, blocking: false }
    }
}

/// Everything wrong with `doc`, the problems that stop it being saved first.
pub fn problems(doc: &MapDoc, vfs: &Vfs, mods_dir: &Path) -> Vec<Problem> {
    let mut out = Vec::new();
    if let Some(problem) = super::save::name_problem(vfs, &doc.props.name, mods_dir) {
        out.push(Problem { text: problem, at: None, blocking: true });
    }
    out.extend(map_problems(doc));
    out
}

/// The map's own problems (not its name's).
pub fn map_problems(doc: &MapDoc) -> Vec<Problem> {
    let mut out = Vec::new();
    let (w, h) = (doc.width_px(), doc.height_px());
    let sea = doc.tnt.sea_level as f32;
    if doc.starts.len() < 2 {
        out.push(Problem::warning(t("Skirmish needs at least two start positions.").to_string(), None));
    }
    for (i, s) in doc.starts.iter().enumerate() {
        let n = i + 1;
        if s[0] < 0.0 || s[1] < 0.0 || s[0] >= w || s[1] >= h {
            out.push(Problem::warning(tf("Start position {n} is off the map.", &[("n", &n)]), Some([s[0].clamp(0.0, w), s[1].clamp(0.0, h)])));
        } else if doc.ground(s[0], s[1]) < sea {
            out.push(Problem::warning(tf("Start position {n} is under the sea: only amphibious units can build there.", &[("n", &n)]), Some(*s)));
        }
    }
    for (i, a) in doc.starts.iter().enumerate() {
        for (j, b) in doc.starts.iter().enumerate().skip(i + 1) {
            if (a[0] - b[0]).hypot(a[1] - b[1]) < CROWDED {
                out.push(Problem::warning(tf("Start positions {a} and {b} are almost on top of each other.", &[("a", &(i + 1)), ("b", &(j + 1))]),
                    Some([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0])));
            }
        }
    }
    if doc.tnt.heights.iter().all(|&height| (height as f32) < sea) {
        out.push(Problem::warning(t("The whole map is under the sea.").to_string(), None));
    }
    let tiles = doc.tnt.tiles.len();
    if tiles * 10 >= MAX_TILES * 9 {
        out.push(Problem::warning(tf("The map uses {n} of its {max} tiles.", &[("n", &tiles), ("max", &MAX_TILES)]), None));
    }
    out
}
