//! Colours that carry meaning: the players' colours, health, and where something may or
//! may not go. Each also comes in a colour-blind-safe version (after Okabe and Ito's
//! palette), chosen in the settings: `set_colour_blind` switches every user at once, and
//! the renderer tints units to match (`Renderer::set_colour_blind`, `unit.wgsl`'s
//! `team_cb`).

use std::sync::atomic::{AtomicBool, Ordering};

use egui::Color32;

/// TA's ten player colours (approximating the team-colour texture frames).
pub const TEAM_COLORS: [[u8; 3]; 10] = [
    [48, 92, 220],
    [210, 40, 30],
    [230, 230, 230],
    [40, 170, 60],
    [20, 40, 140],
    [130, 70, 180],
    [240, 220, 40],
    [40, 40, 40],
    [120, 190, 240],
    [180, 170, 120],
];

/// Player colours that stay apart with red-green (and mostly blue-yellow) colour
/// blindness, in the order of TA's colour slots: blue first, then orange where TA has red.
/// `unit.wgsl` holds the same table.
pub const TEAM_COLORS_CB: [[u8; 3]; 10] = [
    [0, 114, 178],
    [230, 159, 0],
    [240, 240, 240],
    [0, 158, 115],
    [86, 180, 233],
    [204, 121, 167],
    [240, 228, 66],
    [35, 35, 35],
    [213, 94, 0],
    [150, 150, 150],
];

static COLOUR_BLIND: AtomicBool = AtomicBool::new(false);

pub fn set_colour_blind(on: bool) {
    COLOUR_BLIND.store(on, Ordering::Relaxed);
}

pub fn colour_blind() -> bool {
    COLOUR_BLIND.load(Ordering::Relaxed)
}

fn rgb([r, g, b]: [u8; 3]) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// The next of the ten colours round from `from` that `taken` says nobody else has (every one
/// taken: simply the next).
pub fn next_free_colour(from: u8, taken: impl Fn(u8) -> bool) -> u8 {
    (1..=10u8).map(|k| (from % 10 + k) % 10).find(|&c| !taken(c)).unwrap_or((from % 10 + 1) % 10)
}

pub fn team_color(i: u8) -> Color32 {
    let table = if colour_blind() { &TEAM_COLORS_CB } else { &TEAM_COLORS };
    rgb(table[i as usize % table.len()])
}

/// A health bar's colour: full, hurt, nearly gone.
pub fn health_color(frac: f32) -> Color32 {
    let [full, hurt, low] = if colour_blind() { [[86, 180, 233], [240, 228, 66], [213, 94, 0]] } else { [[80, 220, 90], [230, 200, 60], [230, 70, 50]] };
    rgb(if frac > 0.6 {
        full
    } else if frac > 0.3 {
        hurt
    } else {
        low
    })
}

/// Map overlay cells (RGBA): where something can go, and where it cannot.
pub fn allowed_rgba() -> [u8; 4] {
    if colour_blind() { [86, 180, 233, 60] } else { [60, 220, 90, 50] }
}

pub fn blocked_rgba() -> [u8; 4] {
    if colour_blind() { [230, 120, 0, 160] } else { [230, 50, 40, 150] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colour_click_passes_over_those_taken() {
        let taken = |list: &'static [u8]| move |c: u8| list.contains(&c);
        assert_eq!(next_free_colour(0, taken(&[1, 2])), 3);
        assert_eq!(next_free_colour(8, taken(&[9, 0])), 1, "round past the last");
        assert_eq!(next_free_colour(4, taken(&[])), 5);
        assert_eq!(next_free_colour(3, taken(&[0, 1, 2, 4, 5, 6, 7, 8, 9])), 3, "the only free one is its own");
        assert_eq!(next_free_colour(3, taken(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9])), 4, "all taken: the next");
    }
}
