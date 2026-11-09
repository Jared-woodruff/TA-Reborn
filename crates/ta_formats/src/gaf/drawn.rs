//! How big an animation's frames are drawn, and with how many texels.
//!
//! TA draws an explosion's frames a pixel to a world unit, and Reborn does too: a frame is as
//! big in the world as it was painted. TA's own explosions are small (the largest, `fx/alfboom2`,
//! 126 x 53; its nuke 76 x 59), but a mod's may be painted as big as their blasts: TA:
//! Escalation's are their weapons' areas of effect across, up to 3240 x 3240 for an antimatter
//! cannon's, in animations of 45 to 75 frames; its 86 named by weapons come to 1.33 billion
//! pixels (5 GiB as textures). Every explosion's frames sit in one sprite atlas, so a set like
//! that is drawn with fewer texels than it was painted with, each still as big in the world:
//! [`plan`] gives each animation its scale (pixels to a texel), and [`super::GafReader::reduced`]
//! reads its frames so.
//!
//! The plan keeps the set within a budget of texels, and reduces no more than that needs: when
//! the whole set fits, every frame is drawn as painted. Otherwise every animation larger than TA
//! ever painted one ([`NATIVE_SIDE`]) is drawn at one density (texels to a world unit), the
//! highest the budget allows, but never with fewer than [`NATIVE_SIDE`] texels across: big
//! blasts and small ones alike as sharp as each other on screen. Nothing at TA's own sizes is
//! reduced. And no frame is drawn with more than [`MAX_DRAWN_SIDE`] texels across, however it
//! was painted: an explosion that fills the screen (about 2000 lines on a 4K one) needs no more.

/// The longest side of a frame of an animation that is never reduced: TA's largest explosion
/// (`fx/alfboom2`) is 126 x 53.
pub const NATIVE_SIDE: u32 = 128;
/// The most texels a frame is drawn with across or down, however big it was painted: about the
/// height of a 4K screen, as much as an explosion filling it shows.
pub const MAX_DRAWN_SIDE: u32 = 2048;
/// The most sprite atlas texels a battle's explosions take (512 MiB as RGBA): every mod known
/// draws its own as painted within it (TA: Enhanced's, the largest, take 122 Mi), all but TA:
/// Escalation's (1.2 Gi), which are drawn reduced to fit.
pub const EXPLOSION_TEXELS: u64 = 128 << 20;

/// The explosions' budget of texels in a sprite atlas at most `max_side` texels a side:
/// [`EXPLOSION_TEXELS`], or half of the atlas where that is less (the map's features need room).
pub fn explosion_budget(max_side: u32) -> u64 {
    EXPLOSION_TEXELS.min(u64::from(max_side).pow(2) / 2)
}

/// What an animation's frames take, enough to plan how it is drawn ([`plan`]): its frames that
/// hold pixels, their area and edges (pixels), and the longest side of any of them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnimSize {
    pub frames: u64,
    pub area: u64,
    /// The frames' widths and heights, summed.
    pub edges: u64,
    pub largest: u32,
}

impl AnimSize {
    /// The size of an animation of frames `sizes` (width, height); frames without pixels take
    /// no room.
    pub fn of(sizes: impl IntoIterator<Item = (u32, u32)>) -> AnimSize {
        sizes.into_iter().filter(|&(w, h)| w > 0 && h > 0).fold(AnimSize::default(), |a, (w, h)| AnimSize {
            frames: a.frames + 1,
            area: a.area + u64::from(w) * u64::from(h),
            edges: a.edges + u64::from(w) + u64::from(h),
            largest: a.largest.max(w).max(h),
        })
    }

    /// The texels its frames take in the sprite atlas drawn `scale` pixels to a texel, each
    /// with a texel of padding round it: exact as painted (scale 1), and at most that reduced
    /// (a frame of `w` pixels is `ceil(w / scale)` texels across).
    pub fn texels(&self, scale: f64) -> u64 {
        if scale <= 1.0 {
            return self.area + 2 * self.edges + 4 * self.frames;
        }
        // (`ceil(w / s) + 2` is less than `w / s + 3`.)
        (self.area as f64 / (scale * scale) + 3.0 * self.edges as f64 / scale + 9.0 * self.frames as f64).ceil() as u64
    }
}

/// How a set of animations is drawn ([`plan`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// Each animation's scale, pixels of its frames to a texel: 1 as painted, more reduced.
    pub scales: Vec<f64>,
    /// The density the reduced ones are drawn at, texels to a pixel (1: none reduced for the
    /// budget, though a frame larger than [`MAX_DRAWN_SIDE`] still is).
    pub density: f64,
    /// The texels the set would take as painted, and takes as planned.
    pub full: u64,
    pub drawn: u64,
}

impl Plan {
    /// How many animations are drawn reduced.
    pub fn reduced(&self) -> usize {
        self.scales.iter().filter(|&&s| s > 1.0).count()
    }
}

/// How each of `anims` is drawn so that together they take at most `budget` texels: as
/// painted, if they fit; else those larger than [`NATIVE_SIDE`] at one density, the highest
/// that fits (see the module's notes). Where even [`NATIVE_SIDE`] texels across for each of
/// them is too much, that is what they get, over the budget.
pub fn plan(anims: &[AnimSize], budget: u64) -> Plan {
    let scale = |a: &AnimSize, density: f64| -> f64 {
        if a.largest <= NATIVE_SIDE {
            return 1.0;
        }
        let largest = f64::from(a.largest);
        let side = (largest * density).max(f64::from(NATIVE_SIDE)).min(largest).min(f64::from(MAX_DRAWN_SIDE));
        (largest / side).max(1.0)
    };
    let total = |density: f64| anims.iter().map(|a| a.texels(scale(a, density))).sum::<u64>();
    let density = if total(1.0) <= budget {
        1.0
    } else {
        let (mut fits, mut over) = (0.0f64, 1.0f64);
        for _ in 0..48 {
            let mid = (fits + over) / 2.0;
            if total(mid) <= budget {
                fits = mid;
            } else {
                over = mid;
            }
        }
        fits
    };
    Plan {
        scales: anims.iter().map(|a| scale(a, density)).collect(),
        density,
        full: anims.iter().map(|a| a.texels(1.0)).sum(),
        drawn: total(density),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anim(side: u32, frames: usize) -> AnimSize {
        AnimSize::of(std::iter::repeat_n((side, side), frames))
    }

    #[test]
    fn sizes_count_padded_texels_exactly_as_painted() {
        let a = AnimSize::of([(3, 2), (0, 5), (4, 4)]);
        assert_eq!(a, AnimSize { frames: 2, area: 22, edges: 13, largest: 4 });
        // (3 + 2) x (2 + 2) + (4 + 2) x (4 + 2)
        assert_eq!(a.texels(1.0), 20 + 36);
        // Reduced, no fewer than the frames really take: 3000 x 3000 at 7.5 is 400 x 400.
        let big = anim(3000, 1);
        assert!(big.texels(7.5) >= 402 * 402 && big.texels(7.5) < 404 * 404, "{}", big.texels(7.5));
    }

    #[test]
    fn a_set_that_fits_is_drawn_as_painted() {
        let set = [anim(51, 23), anim(126, 8), anim(1500, 18)];
        let p = plan(&set, 64 << 20);
        assert_eq!(p.scales, [1.0, 1.0, 1.0]);
        assert_eq!((p.density, p.drawn, p.reduced()), (1.0, p.full, 0));
    }

    #[test]
    fn frames_larger_than_a_screen_are_drawn_no_larger_whatever_the_budget() {
        let p = plan(&[anim(3240, 2), anim(2048, 2)], u64::MAX);
        assert_eq!(p.density, 1.0);
        assert!((p.scales[0] - 3240.0 / 2048.0).abs() < 1e-9 && p.scales[1] == 1.0, "{:?}", p.scales);
    }

    #[test]
    fn over_budget_the_large_ones_share_one_density_and_tas_sizes_keep_theirs() {
        let set = [anim(3240, 48), anim(1080, 60), anim(400, 50), anim(126, 8), anim(51, 23)];
        let budget = 24 << 20;
        let p = plan(&set, budget);
        assert!(p.drawn <= budget && p.drawn > budget * 9 / 10, "{} of {budget}", p.drawn);
        assert!(p.density > 0.1 && p.density < 1.0, "{}", p.density);
        // TA's sizes as painted; the big ones at the same density (its texels to their pixels).
        assert_eq!((p.scales[3], p.scales[4]), (1.0, 1.0));
        assert!((1.0 / p.scales[0] - p.density).abs() < 1e-9 && (1.0 / p.scales[1] - p.density).abs() < 1e-9, "{p:?}");
        // One just over TA's sizes no smaller than they are.
        let side = 400.0 / p.scales[2];
        assert!(side >= f64::from(NATIVE_SIDE) - 1e-9, "{side}");
    }

    #[test]
    fn a_budget_too_small_for_any_density_leaves_each_at_tas_largest() {
        let p = plan(&[anim(3240, 48), anim(1080, 60), anim(64, 10)], 1000);
        assert_eq!(p.density, 0.0);
        assert!((3240.0 / p.scales[0] - 128.0).abs() < 1e-6 && (1080.0 / p.scales[1] - 128.0).abs() < 1e-6, "{:?}", p.scales);
        assert_eq!(p.scales[2], 1.0);
        assert!(p.drawn > 1000);
    }
}
