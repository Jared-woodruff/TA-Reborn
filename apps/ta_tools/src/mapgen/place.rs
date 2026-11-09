//! What stands on the land: forests and lone trees, shrubs at their edges, rocks on the slopes,
//! metal by every start and in fields across the map, geothermal vents, great landmarks and
//! ruins, groves round the oases, and on a winter world villages: a square by a road, its great
//! fir or clock tower in the middle, cottages round it, lanterns, snowmen and presents. Each is a feature of the world's own from the game's data
//! (named, never copied), its index in the top-left cell of its footprint. On a symmetric map
//! everything is placed in the first half and its twin turned round into the second.

use super::field::Field;
use super::noise::{fbm, smoothstep, unit};
use super::terrain::Land;

/// A world's features by role (names as in the game's `features/` TDFs), each with its footprint.
#[derive(Clone, Default)]
pub struct Kit {
    pub trees: Vec<&'static str>,
    pub shrubs: Vec<&'static str>,
    pub rocks_small: Vec<&'static str>,
    pub rocks_big: Vec<(&'static str, usize, usize)>,
    pub metal: Vec<&'static str>,
    pub vents: Vec<&'static str>,
    /// Great standing features (lava spires, nodes) with their footprints, on high, broken
    /// ground, this many per million cells.
    pub landmarks: Vec<(&'static str, usize, usize)>,
    pub landmark_density: f32,
    /// How dense the forests grow (0 none).
    pub forest: f32,
    /// How much more of the land the forests cover (added to their field; 0 as first made).
    pub cover: f32,
    /// Ruins on level open ground (desert worlds' broken walls), this many per million cells.
    pub ruins: Vec<(&'static str, usize, usize)>,
    pub ruin_density: f32,
    /// Trees round the pools (palms by an oasis).
    pub grove: Vec<&'static str>,
    /// Glades: how much of the forest opens into clearings (0 none, 1 about half), and how far
    /// round each base the ground is clear (cells; 0 as first made).
    pub glades: f32,
    pub clearing: f32,
    /// Metal deposits round each base (0: five, as first made), and how many more fields of
    /// metal over the map than the first maps had for their area (0: as many).
    pub base_metal: usize,
    pub metal_fields: f32,
    /// Reefs in the shallows (corals, anemones, kelp, with their footprints), this many clumps
    /// per million cells of shallow water, and metal under the sea (for the fleets'
    /// extractors), a field of it per this many thousand cells of shallow water.
    pub reef: Vec<(&'static str, usize, usize)>,
    pub reef_density: f32,
    pub sea_metal: Vec<&'static str>,
    pub sea_metal_every: f32,
    /// The vents' footprint (cells a side; 0 as TA's natural vents, one).
    pub vent_size: usize,
    /// Metal on the islands (land no base stands on): deposits of these kinds (the richest), one
    /// for every `isle_metal_every` cells of island and at least one an island, spread over its
    /// level ground; and a vent on every island of `isle_vent` cells or more. (None as first made.)
    pub isle_metal: Vec<&'static str>,
    pub isle_metal_every: f32,
    pub isle_vent: usize,
    /// Features right up to the line between the halves (every cell of a footprint in the first
    /// half), not a footprint and a cell off it as first made: an island across the middle wooded
    /// through, without a clearing down its middle.
    pub tight: bool,
    /// Keep a way a tank drives from every base to every other clear of features (see
    /// [`super::terrain::corridor`]).
    pub corridor: bool,
    /// Villages (a winter world's), if any.
    pub village: Option<Village>,
    /// A real place's features by biome (`dress`), if it is one.
    pub earth: Option<EarthKit>,
}

/// A real place's features by biome: what the placer put down from the kit's own (temperate)
/// lists is given, cell by cell, its biome's own of the same footprint (`dress`): the tropics'
/// trees, the north's, palms and the desert's rocks, metal and vents, the cold's.
#[derive(Clone, Default)]
pub struct EarthKit {
    pub tropical: Vec<&'static str>,
    pub boreal: Vec<&'static str>,
    pub palms: Vec<&'static str>,
    pub desert_rocks: Vec<(&'static str, usize, usize)>,
    pub cold_rocks: Vec<(&'static str, usize, usize)>,
    pub desert_metal: Vec<&'static str>,
    pub cold_metal: Vec<&'static str>,
    pub desert_vents: Vec<&'static str>,
    pub cold_vents: Vec<&'static str>,
}

impl EarthKit {
    pub fn metal(&self) -> Vec<&'static str> {
        self.desert_metal.iter().chain(&self.cold_metal).copied().collect()
    }

    pub fn vents(&self) -> Vec<&'static str> {
        self.desert_vents.iter().chain(&self.cold_vents).copied().collect()
    }
}

/// Gives each feature of a real place (`placed`, from `kit`'s lists) its biome's own name
/// (`earth`): a tree in the tropics a tropical tree, in the north a northern one, in the desert
/// a palm; a rock in the desert or the cold one of theirs of the same footprint; metal and vents
/// likewise. The biome is the strongest at the feature's cell.
pub fn dress(placed: &mut Placed, land: &Land, kit: &Kit, earth: &EarthKit) {
    let Some(geo) = land.geo.as_ref() else { return };
    let w = land.w;
    let has = |list: &[&'static str], name: &str| list.iter().any(|n| n.eq_ignore_ascii_case(name));
    let mut names = placed.names.clone();
    let slot_of = |name: &str, names: &mut Vec<String>| -> u16 {
        match names.iter().position(|n| n.eq_ignore_ascii_case(name)) {
            Some(i) => i as u16,
            None => {
                names.push(name.to_string());
                (names.len() - 1) as u16
            }
        }
    };
    for i in 0..placed.cells.len() {
        let slot = placed.cells[i];
        if slot as usize >= placed.names.len() {
            continue;
        }
        let name = placed.names[slot as usize].clone();
        let b = geo.biome[i];
        let biome = (0..4).max_by(|&a, &c| b[a].total_cmp(&b[c])).unwrap_or(0);
        let warm = geo.warmth.data[i];
        let pick = |list: &[&'static str]| list[(unit((i % w) as i32, (i / w) as i32, 0xE1) * list.len() as f32) as usize % list.len().max(1)];
        let pick_fit = |list: &[(&'static str, usize, usize)], fw: usize, fz: usize| -> Option<&'static str> {
            let fits: Vec<&'static str> = list.iter().filter(|r| (r.1, r.2) == (fw, fz)).map(|r| r.0).collect();
            (!fits.is_empty()).then(|| fits[(unit((i % w) as i32, (i / w) as i32, 0xE2) * fits.len() as f32) as usize % fits.len()])
        };
        let new = if has(&kit.trees, &name) {
            match biome {
                2 if !earth.palms.is_empty() => Some(pick(&earth.palms)),
                3 if !earth.boreal.is_empty() => Some(pick(&earth.boreal)),
                _ if warm > 0.72 && !earth.tropical.is_empty() => Some(pick(&earth.tropical)),
                _ if warm < 0.34 && !earth.boreal.is_empty() => Some(pick(&earth.boreal)),
                _ => None,
            }
        } else if let Some(&(_, fw, fz)) = kit.rocks_big.iter().find(|r| r.0.eq_ignore_ascii_case(&name)) {
            match biome {
                2 => pick_fit(&earth.desert_rocks, fw, fz),
                3 => pick_fit(&earth.cold_rocks, fw, fz),
                _ => None,
            }
        } else if let Some(k) = kit.metal.iter().position(|n| n.eq_ignore_ascii_case(&name)) {
            // (Kind for kind: the biome's lists are in the order of the kit's, deposits holding
            // about as much, so every base has as much metal whatever its biome.)
            let like = |list: &[&'static str]| list[k % list.len()];
            match biome {
                2 if !earth.desert_metal.is_empty() => Some(like(&earth.desert_metal)),
                3 if !earth.cold_metal.is_empty() => Some(like(&earth.cold_metal)),
                _ => None,
            }
        } else if has(&kit.vents, &name) {
            match biome {
                2 if !earth.desert_vents.is_empty() => Some(pick(&earth.desert_vents)),
                3 if !earth.cold_vents.is_empty() => Some(pick(&earth.cold_vents)),
                _ => None,
            }
        } else {
            None
        };
        if let Some(n) = new {
            placed.cells[i] = slot_of(n, &mut names);
        }
    }
    placed.names = names;
}

/// A winter world's villages: each a square on level ground beside a road (its centrepiece in
/// the middle, a great fir or a clock tower), cottages in a ring round it with gaps for the lanes,
/// lanterns round the square and along the roads near it, decorations about the square
/// (snowmen, presents, ice sculptures, a sleigh), fireworks for the New Year and strings of
/// lanterns, decorated firs where the forest meets it. Features by name with their footprints.
#[derive(Clone, Default)]
pub struct Village {
    pub centre: (&'static str, usize, usize),
    pub houses: Vec<(&'static str, usize, usize)>,
    pub lamps: Vec<&'static str>,
    pub decorations: Vec<(&'static str, usize, usize)>,
    pub fireworks: Vec<(&'static str, usize, usize)>,
    pub strings: Vec<(&'static str, usize, usize)>,
    pub firs: Vec<&'static str>,
    /// Villages per million cells (half each side on a symmetric map), and whether one stands
    /// in the middle of the map (its centrepiece, an even footprint, its own twin).
    pub density: f32,
    pub middle: bool,
}

/// A village's square: its radius (cells).
const SQUARE: f32 = 9.0;

pub struct Placed {
    /// Feature slot per cell (`0xFFFF` none) and the slots' names.
    pub cells: Vec<u16>,
    pub names: Vec<String>,
    /// How thick the forest is, per cell (for the painter).
    pub forest: Field,
    pub count: usize,
    /// The cells a feature stands on that units cannot pass (all but metal and vents).
    pub blocked: Vec<bool>,
    /// The villages' squares (cells: their middles and radii), both halves', for the painter.
    pub squares: Vec<(f32, f32, f32)>,
}

/// How thick the forest grows at each cell (0 to 1): where it is wet enough and not too steep,
/// thinning at the edges; none on the starts' bases or the shore.
pub fn forest_field(land: &Land, kit: &Kit, seed: u32) -> Field {
    let (w, h) = (land.w, land.h);
    let slope = land.height.ta_slope();
    Field::from_fn(w, h, |x, z| {
        if kit.forest <= 0.0 {
            return 0.0;
        }
        let i = z * w + x;
        let ground = land.height.data[i] > land.sea + 3.0 && land.water_dist.data[i] > 3.0 && slope.data[i] < 11.0;
        if !ground {
            return 0.0;
        }
        let start = land.starts.iter().map(|s| (x as f32 - s.0).hypot(z as f32 - s.1)).fold(f32::MAX, f32::min);
        let clear = if kit.clearing > 0.0 { smoothstep(kit.clearing, kit.clearing * 1.5, start) } else { smoothstep(38.0, 60.0, start) };
        let f = land.sym.eval(x as f32, z as f32, |x, z| fbm(x / 55.0, z / 55.0, seed ^ 0x71, 4)) * 0.5 + 0.5 + (land.moisture.data[i] - 0.5) * 0.45 + kit.cover;
        let glade = if kit.glades > 0.0 {
            let cut = 0.65 - 0.15 * kit.glades;
            smoothstep(cut + 0.04, cut - 0.04, land.sym.eval(x as f32, z as f32, |x, z| fbm(x / 120.0, z / 120.0, seed ^ 0x72, 3)) * 0.5 + 0.5)
        } else {
            1.0
        };
        smoothstep(0.53, 0.68, f) * clear * smoothstep(11.0, 6.0, slope.data[i]) * glade
    })
}

/// Places the kit's features. `forest` is [`forest_field`]'s; `avoid` the distance (cells) to
/// what must stay clear (the tracks), nothing standing within 1.5 cells of it.
pub fn place(land: &Land, kit: &Kit, forest: Field, avoid: Option<&Field>, seed: u32) -> Placed {
    let mut forest = forest;
    let (w, h) = (land.w, land.h);
    let sym = land.sym;
    // (On a symmetric map half of each count goes in the first half; the twins make the rest.)
    let share = if sym.on { 0.5 } else { 1.0 };
    let mut cells = vec![0xFFFFu16; w * h];
    let mut taken = vec![false; w * h];
    let mut names: Vec<String> = Vec::new();
    let mut count = 0usize;
    let mut firsts: Vec<(u16, usize, usize, usize, usize)> = Vec::new();
    let slope = land.height.ta_slope();
    let near_start = |x: usize, z: usize, r: f32| land.starts.iter().any(|s| (x as f32 - s.0).hypot(z as f32 - s.1) < r);
    // Whether a footprint `fw` x `fz` at (x, z) is level enough for something big to stand on:
    // its corners and those a cell round it within 10 units, none of its cells steep (a spire
    // straddling a cliff had its foot cut off by the step).
    let level = |x: usize, z: usize, fw: usize, fz: usize| {
        let (x0, z0, x1, z1) = (x.saturating_sub(1), z.saturating_sub(1), (x + fw + 1).min(w - 1), (z + fz + 1).min(h - 1));
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for zz in z0..=z1 {
            for xx in x0..=x1 {
                let v = land.height.at(xx, zz);
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        hi - lo <= 10.0 && (z..(z + fz).min(h)).all(|zz| (x..(x + fw).min(w)).all(|xx| slope.data[zz * w + xx] <= 14.0))
    };
    // The middle village's centrepiece, on the line between the halves (its twin itself).
    let mut squares: Vec<(f32, f32, f32)> = Vec::new();
    if let Some(v) = kit.village.as_ref().filter(|v| v.middle) {
        let (name, fw, fz) = v.centre;
        let (x, z) = ((w - fw) / 2, (h - fz) / 2);
        if level(x, z, fw, fz) {
            names.push(name.to_string());
            let slot = (names.len() - 1) as u16;
            cells[z * w + x] = slot;
            for dz in 0..fz {
                for dx in 0..fw {
                    taken[(z + dz) * w + x + dx] = true;
                }
            }
            count += 1;
            firsts.push((slot, x, z, fw, fz));
            squares.push((w as f32 * 0.5, h as f32 * 0.5, SQUARE));
        }
    }
    let mut put = |name: &str, x: usize, z: usize, fw: usize, fz: usize, cells: &mut Vec<u16>, taken: &mut Vec<bool>| -> bool {
        if x + fw > w || z + fz > h {
            return false;
        }
        // Nothing on the void, on a gantry or on a road to one.
        if !land.void.is_empty() && (z..z + fz).any(|zz| (x..x + fw).any(|xx| land.void[zz * w + xx] || land.gantry[zz * w + xx] || land.road[zz * w + xx])) {
            return false;
        }
        // In the first half, far enough from the line between the halves that its twin
        // cannot overlap it (with `tight`, every cell of it in the first half, its twin then all
        // in the second).
        if sym.on && kit.tight && !(z..z + fz).all(|zz| (x..x + fw).all(|xx| sym.side(xx as f32 + 0.5, zz as f32 + 0.5) > 0.0)) {
            return false;
        }
        if sym.on && !kit.tight && sym.side(x as f32 + fw as f32 * 0.5, z as f32 + fz as f32 * 0.5) <= (fw.max(fz) as f32 + 1.0) {
            return false;
        }
        if let Some(a) = avoid
            && (z..z + fz).any(|zz| (x..x + fw).any(|xx| a.data[zz * w + xx] < 2.6))
        {
            return false;
        }
        // (Nor its twin, on a symmetric map, where a way is kept clear: the first maps did
        // without, their tracks as good as symmetric.)
        if sym.on && kit.corridor {
            let (tx, tz) = (w - x - fw, h - z - fz);
            let clash = |xx: usize, zz: usize| avoid.is_some_and(|a| a.data[zz * w + xx] < 2.6) || (!land.void.is_empty() && (land.void[zz * w + xx] || land.gantry[zz * w + xx] || land.road[zz * w + xx]));
            if (tz..tz + fz).any(|zz| (tx..tx + fw).any(|xx| clash(xx, zz))) {
                return false;
            }
        }
        for dz in 0..fz {
            for dx in 0..fw {
                if taken[(z + dz) * w + x + dx] {
                    return false;
                }
            }
        }
        let slot = match names.iter().position(|m| m.eq_ignore_ascii_case(name)) {
            Some(i) => i,
            None => {
                names.push(name.to_string());
                names.len() - 1
            }
        } as u16;
        cells[z * w + x] = slot;
        for dz in 0..fz {
            for dx in 0..fw {
                taken[(z + dz) * w + x + dx] = true;
            }
        }
        count += 1;
        firsts.push((slot, x, z, fw, fz));
        true
    };
    // Where an extractor can be built over a deposit at (x, z) (its 3 x 3 footprint's top left):
    // as the game has it, all of it dry and no steeper than 10 (on a symmetric map its twin's too:
    // a cell's slope is read off its corners, which turn round a cell apart); `roomy`, with a
    // little to spare, for the deposits by the bases on the new maps, as they were made.
    let new = kit.base_metal > 0;
    let steep: Vec<f32> = if sym.on { (0..w * h).map(|i| slope.data[i].max(slope.data[sym.twin_index(i)])).collect() } else { slope.data.clone() };
    let fits = |x: usize, z: usize, above: f32, most: f32| x + 3 <= w && z + 3 <= h && (z..z + 3).all(|zz| (x..x + 3).all(|xx| land.height.data[zz * w + xx] > land.sea + above && steep[zz * w + xx] <= most));
    // (The game takes an extractor anywhere over a cell of the deposit: right over it first, then
    // a cell or two off.)
    let mine = |x: usize, z: usize| -> Option<(usize, usize)> {
        const OFF: [(i64, i64); 25] = [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, 1), (1, -1), (-1, -1), (2, 0), (-2, 0), (0, 2), (0, -2), (2, 1), (2, -1), (-2, 1), (-2, -1), (1, 2), (-1, 2), (1, -2), (-1, -2), (2, 2), (-2, 2), (2, -2), (-2, -2)];
        OFF.iter().map(|&(dx, dz)| (x as i64 + dx, z as i64 + dz)).filter(|&(ox, oz)| ox >= 0 && oz >= 0).map(|(ox, oz)| (ox as usize, oz as usize)).find(|&(ox, oz)| fits(ox, oz, 0.0, 10.0))
    };
    // (By a base, right over the deposit, so it yields all it holds: the first maps' with the
    // game's own limits, the new maps' with a little to spare, as they were made.)
    let roomy = |x: usize, z: usize| if new { fits(x, z, 2.0, 8.0) } else { fits(x, z, 0.0, 10.0) };
    // (The ground of the extractor found for a deposit kept clear of what is placed after.)
    let reserve = |at: (usize, usize), taken: &mut Vec<bool>| {
        for zz in at.1..(at.1 + 3).min(h) {
            for xx in at.0..(at.0 + 3).min(w) {
                taken[zz * w + xx] = true;
            }
        }
    };
    // Metal first (it decides where bases go): deposits round each start, then fields of two or
    // three over the rest of the level land.
    if !kit.metal.is_empty() {
        let around = if new { kit.base_metal } else { 5 };
        for (k, &(sx, sz)) in land.starts.iter().enumerate() {
            // (On a symmetric map a base's deposits are turned round into its twin's: those of
            // the bases in the second half are their twins'.)
            if new && !sym.first(sx, sz) {
                continue;
            }
            for j in 0..around as i32 {
                let step = if new { std::f32::consts::TAU / around as f32 } else { 1.26 };
                let a = j as f32 * step + unit(k as i32, j, seed) * 0.6;
                let r = 20.0 + 6.0 * unit(j, k as i32, seed ^ 3);
                // (Turned round the base, or further in or out, out of a track's way, onto dry
                // level ground where an extractor fits, and on a symmetric map onto the base's own
                // side of the line between the halves; the first maps took the first spot that
                // would take one.)
                const TRIES: [(f32, f32); 31] = [
                    (0.0, 0.0),
                    (0.3, 0.0),
                    (-0.3, 0.0),
                    (0.6, 0.0),
                    (-0.6, 0.0),
                    (0.0, -6.0),
                    (0.3, -6.0),
                    (-0.3, -6.0),
                    (0.0, 6.0),
                    (0.9, 0.0),
                    (-0.9, 0.0),
                    (0.6, -6.0),
                    (-0.6, -6.0),
                    (1.2, 0.0),
                    (-1.2, 0.0),
                    (1.5, 0.0),
                    (-1.5, 0.0),
                    (0.9, -6.0),
                    (-0.9, -6.0),
                    (1.2, 6.0),
                    (-1.2, 6.0),
                    (1.8, 0.0),
                    (-1.8, 0.0),
                    (2.1, 0.0),
                    (-2.1, 0.0),
                    (2.4, 0.0),
                    (-2.4, 0.0),
                    (2.7, 0.0),
                    (-2.7, 0.0),
                    (3.1, 0.0),
                    (1.5, 6.0),
                ];
                // (Every base has the same deposits, kind for kind: they hold from 56 to 223 a
                // cell, and the kinds once went round with the bases.)
                let name = kit.metal[j as usize % kit.metal.len()];
                for &(turn, out) in &TRIES {
                    let (x, z) = ((sx + (a + turn).cos() * (r + out)) as usize, (sz + (a + turn).sin() * (r + out)) as usize);
                    let (x0, z0) = (x.saturating_sub(1), z.saturating_sub(1));
                    if roomy(x0, z0) && put(name, x0, z0, 3, 3, &mut cells, &mut taken) {
                        break;
                    }
                }
            }
        }
        let fields = ((w * h) as f32 * share * if kit.metal_fields > 0.0 { kit.metal_fields } else { 1.0 }) as usize / 14_000;
        let mut placed = 0;
        let mut tries = 0;
        while placed < fields && tries < fields * 40 {
            tries += 1;
            let x = (unit(tries as i32, 1, seed ^ 0x51) * (w - 20) as f32) as usize + 10;
            let z = (unit(tries as i32, 2, seed ^ 0x52) * (h - 20) as f32) as usize + 10;
            let i = z * w + x;
            if slope.data[i] > 4.0 || land.height.data[i] < land.sea + 3.0 || near_start(x, z, 60.0) || !sym.first(x as f32, z as f32) {
                continue;
            }
            for j in 0..2 + (unit(tries as i32, 3, seed) * 2.0) as usize {
                let (dx, dz) = ((unit(tries as i32, 10 + j as i32, seed) * 12.0) as usize, (unit(tries as i32, 20 + j as i32, seed) * 12.0) as usize);
                // (Only where an extractor can be built over it: a field's deposits spread into a
                // river or onto a slope once, where none could.)
                if let Some(at) = mine(x + dx, z + dz)
                    && put(kit.metal[(tries + j) % kit.metal.len()], x + dx, z + dz, 3, 3, &mut cells, &mut taken)
                {
                    reserve(at, &mut taken);
                }
            }
            placed += 1;
        }
    }
    // Vents: one a little way from each start, and a few more. Round each, the ground a
    // geothermal plant stands on (4 x 4, the vent under it) is kept clear of everything else, and
    // on the new maps it is level and dry.
    let vs = kit.vent_size.max(1);
    let plant = |x: usize, z: usize| vs > 1 || (x >= 1 && z >= 1 && x + 3 <= w && z + 3 <= h && (z - 1..z + 3).all(|zz| (x - 1..x + 3).all(|xx| land.height.data[zz * w + xx] > land.sea + 3.0 && steep[zz * w + xx] <= 8.0)));
    let room = |x: usize, z: usize, taken: &mut Vec<bool>| {
        if vs == 1 {
            for zz in z.saturating_sub(1)..(z + 3).min(h) {
                for xx in x.saturating_sub(1)..(x + 3).min(w) {
                    taken[zz * w + xx] = true;
                }
            }
        }
    };
    if !kit.vents.is_empty() {
        for (k, &(sx, sz)) in land.starts.iter().enumerate() {
            if !sym.first(sx, sz) {
                continue;
            }
            // (Nearer in, and more tries, on the new maps: a base on a small island had none.)
            let (near, tries) = if new { (30.0, 120) } else { (45.0, 40) };
            for t in 0..tries {
                let a = unit(k as i32, t, seed ^ 0x61) * std::f32::consts::TAU;
                let r = near + unit(t, k as i32, seed ^ 0x62) * (70.0 - near);
                let (x, z) = ((sx + a.cos() * r).max(0.0) as usize, (sz + a.sin() * r).max(0.0) as usize);
                if x + vs <= w && z + vs <= h && slope.data[z * w + x] < 4.0 && land.height.data[z * w + x] > land.sea + 3.0 && (vs == 1 || level(x, z, vs, vs)) && (!new || plant(x, z)) && put(kit.vents[k % kit.vents.len()], x, z, vs, vs, &mut cells, &mut taken) {
                    room(x, z, &mut taken);
                    break;
                }
            }
        }
        for t in 0..((w * h) as f32 * share / 90_000.0) as i32 {
            let x = (unit(t, 7, seed ^ 0x63) * w as f32) as usize;
            let z = (unit(t, 8, seed ^ 0x64) * h as f32) as usize;
            if x + vs <= w && z + vs <= h && slope.data[z * w + x] < 4.0 && land.height.data[z * w + x] > land.sea + 3.0 && (vs == 1 || level(x, z, vs, vs)) && (!new || plant(x, z)) && put(kit.vents[t as usize % kit.vents.len()], x, z, vs, vs, &mut cells, &mut taken) {
                room(x, z, &mut taken);
            }
        }
    }
    // Metal on the islands, the richest kinds, and a vent on the great ones: what the fleets and
    // the hovercraft fight over. (Each island's part in the first half; its twin takes the twins.)
    if !kit.isle_metal.is_empty() && kit.isle_metal_every > 0.0 {
        let dry: Vec<bool> = land.height.data.iter().map(|&v| v > land.sea + 1.0).collect();
        let (label, sizes) = super::terrain::label_regions(w, h, &dry);
        let home: Vec<u32> = land.starts.iter().map(|s| label[(s.1 as usize).min(h - 1) * w + (s.0 as usize).min(w - 1)]).collect();
        let mut isles: std::collections::BTreeMap<u32, Vec<usize>> = Default::default();
        for (i, &l) in label.iter().enumerate() {
            if l != u32::MAX && !home.contains(&l) && sizes[l as usize] >= 40 && sym.first((i % w) as f32, (i / w) as f32) {
                isles.entry(l).or_default().push(i);
            }
        }
        let (mut deposits, mut vents) = (0, 0);
        for (&l, here) in &isles {
            let wanted = ((here.len() as f32 / kit.isle_metal_every).round() as usize).max(1);
            // (Spots in a hashed order, kept apart, each where an extractor stands right over it.)
            let mut spots: Vec<(f32, usize, usize)> = here.iter().map(|&i| (unit((i % w) as i32, (i / w) as i32, seed ^ 0xE5), i % w, i / w)).collect();
            spots.sort_by(|a, b| a.0.total_cmp(&b.0).then((a.2, a.1).cmp(&(b.2, b.1))));
            let mut laid: Vec<(usize, usize)> = Vec::new();
            for &(_, x, z) in &spots {
                if laid.len() >= wanted {
                    break;
                }
                let (x0, z0) = (x.saturating_sub(1), z.saturating_sub(1));
                if laid.iter().any(|&(px, pz)| (px as f32 - x as f32).hypot(pz as f32 - z as f32) < 7.0) || !roomy(x0, z0) {
                    continue;
                }
                if put(kit.isle_metal[laid.len() % kit.isle_metal.len()], x0, z0, 3, 3, &mut cells, &mut taken) {
                    laid.push((x, z));
                }
            }
            deposits += laid.len();
            if kit.isle_vent > 0 && sizes[l as usize] >= kit.isle_vent && !kit.vents.is_empty() {
                for &(_, x, z) in spots.iter().rev() {
                    if x + vs <= w && z + vs <= h && slope.data[z * w + x] < 4.0 && (vs == 1 || level(x, z, vs, vs)) && plant(x, z) && put(kit.vents[l as usize % kit.vents.len()], x, z, vs, vs, &mut cells, &mut taken) {
                        room(x, z, &mut taken);
                        vents += 1;
                        break;
                    }
                }
            }
        }
        eprintln!("  islands: {} (in the first half; their twins as many), {deposits} deposits and {vents} vents on them", isles.len());
    }
    // Reefs: clumps of coral in the shallows, never in the channels' deep water nor by a base.
    let shallow = |i: usize| {
        let d = land.sea - land.height.data[i];
        (3.0..14.0).contains(&d)
    };
    if !kit.reef.is_empty() {
        let cells_shallow = (0..w * h).filter(|&i| shallow(i)).count();
        let wanted = (kit.reef_density * share * cells_shallow as f32 / 1e6) as usize;
        let (mut placed, mut tries) = (0, 0);
        while placed < wanted && tries < wanted * 60 + 100 {
            tries += 1;
            let x = (unit(tries as i32, 71, seed ^ 0xC1) * (w - 12) as f32) as usize + 4;
            let z = (unit(tries as i32, 72, seed ^ 0xC2) * (h - 12) as f32) as usize + 4;
            if !shallow(z * w + x) || near_start(x, z, 45.0) {
                continue;
            }
            for j in 0..3 + (unit(tries as i32, 73, seed) * 4.0) as usize {
                let (name, fw, fz) = kit.reef[(tries * 5 + j * 7) % kit.reef.len()];
                let (dx, dz) = ((unit(tries as i32, 80 + j as i32, seed) * 10.0) as usize, (unit(tries as i32, 90 + j as i32, seed) * 10.0) as usize);
                let (xx, zz) = ((x + dx).min(w - fw), (z + dz).min(h - fz));
                if (zz..zz + fz).all(|a| (xx..xx + fw).all(|b| shallow(a * w + b))) && put(name, xx, zz, fw, fz, &mut cells, &mut taken) {
                    placed += 1;
                }
            }
        }
    }
    // Metal under the sea, for the fleets' extractors: a deposit or two together, in water deep
    // enough for both sides' underwater extractors (19 deep and more; once in the shallows, where
    // neither the land's nor ARM's could stand, and only CORE's in part). As many as the shallows
    // off the shores once had.
    if !kit.sea_metal.is_empty() && kit.sea_metal_every > 0.0 {
        let deep = |i: usize| land.sea - land.height.data[i] >= 21.0;
        let cells_shallow = (0..w * h).filter(|&i| shallow(i)).count();
        let wanted = (share * cells_shallow as f32 / (kit.sea_metal_every * 1000.0)) as usize;
        let (mut placed, mut tries) = (0, 0);
        while placed < wanted && tries < wanted * 80 + 100 {
            tries += 1;
            let x = (unit(tries as i32, 74, seed ^ 0xC3) * (w - 12) as f32) as usize + 4;
            let z = (unit(tries as i32, 75, seed ^ 0xC4) * (h - 12) as f32) as usize + 4;
            let i = z * w + x;
            if !deep(i) || near_start(x, z, 25.0) {
                continue;
            }
            for j in 0..1 + (unit(tries as i32, 76, seed) * 2.0) as usize {
                let (dx, dz) = ((unit(tries as i32, 77 + j as i32, seed) * 6.0) as usize, (unit(tries as i32, 78 + j as i32, seed) * 6.0) as usize);
                let (xx, zz) = ((x + dx).min(w - 3), (z + dz).min(h - 3));
                if (zz..zz + 3).all(|a| (xx..xx + 3).all(|b| deep(a * w + b))) && put(kit.sea_metal[(tries + j) % kit.sea_metal.len()], xx, zz, 3, 3, &mut cells, &mut taken) {
                    placed += 1;
                }
            }
        }
    }
    // Villages: a square beside a road on level open ground, far from the bases and from one
    // another; then what stands round it.
    if let Some(v) = &kit.village {
        let mut sites: Vec<(f32, f32, bool)> = squares.iter().map(|&(x, z, _)| (x, z, true)).collect();
        let wanted = (v.density * share * (w * h) as f32 / 1e6).round() as usize;
        let margin = (SQUARE + 24.0) as usize;
        let road = |x: usize, z: usize| avoid.map_or(10.0, |a| a.data[z * w + x]);
        // Every place a village might stand, best first: the most level ground over its square
        // and the ring of houses round it, a road a dozen cells off (the first half's only, well
        // away from the line between the halves, the bases and the water; its square clear of
        // metal and vents).
        let mut candidates: Vec<(f32, usize, usize)> = Vec::new();
        if w > 2 * margin && h > 2 * margin {
            for z in (margin..h - margin).step_by(5) {
                for x in (margin..w - margin).step_by(5) {
                    let (xf, zf) = (x as f32, z as f32);
                    let i = z * w + x;
                    if near_start(x, z, 75.0) || (sym.on && sym.side(xf, zf) <= SQUARE + 20.0) || land.ice.get(i).copied().unwrap_or(false) || land.water_dist.data[i] < 7.0 || road(x, z) < 5.0 {
                        continue;
                    }
                    let r = (SQUARE + 4.0) as i64;
                    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
                    for dz in (-r..=r).step_by(2) {
                        for dx in (-r..=r).step_by(2) {
                            let v = land.height.at((x as i64 + dx) as usize, (z as i64 + dz) as usize);
                            lo = lo.min(v);
                            hi = hi.max(v);
                        }
                    }
                    let r = SQUARE as i64 + 2;
                    if hi - lo > 26.0 || (-r..=r).any(|dz| (-r..=r).any(|dx| dx * dx + dz * dz <= r * r && taken[(z as i64 + dz) as usize * w + (x as i64 + dx) as usize])) {
                        continue;
                    }
                    candidates.push((hi - lo + (road(x, z) - 12.0).abs() * 0.5, x, z));
                }
            }
        }
        candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then((a.2, a.1).cmp(&(b.2, b.1))));
        for &(_, x, z) in &candidates {
            if sites.len() >= wanted + v.middle as usize {
                break;
            }
            let (xf, zf) = (x as f32, z as f32);
            let far = |a: f32, b: f32| sites.iter().all(|&(sx, sz, _)| (a - sx).hypot(b - sz) > SQUARE * 3.0 + 50.0);
            let (tx, tz) = sym.twin(xf, zf);
            if far(xf, zf) && (!sym.on || far(tx, tz)) {
                sites.push((xf, zf, false));
            }
        }
        eprintln!("  villages: {} of {} wanted (of {} places), at {:?} (pixels)", sites.len(), wanted + v.middle as usize, candidates.len(), sites.iter().map(|s| ((s.0 * 16.0) as i32, (s.1 * 16.0) as i32)).collect::<Vec<_>>());
        for (k, &(cx, cz, middle)) in sites.iter().enumerate() {
            // The way to the road (down the distance to it), and the way opposite: lanes through
            // the ring of houses (the middle village's, to the bases).
            let lanes: Vec<f32> = if middle {
                land.starts.iter().map(|s| (s.1 - cz).atan2(s.0 - cx)).collect()
            } else {
                let (xi, zi) = (cx as usize, cz as usize);
                let (gx, gz) = (road(xi + 2, zi) - road(xi - 2, zi), road(xi, zi + 2) - road(xi, zi - 2));
                let a = (-gz).atan2(-gx);
                vec![a, a + std::f32::consts::PI]
            };
            let clear = |a: f32, spread: f32| lanes.iter().all(|&l| ((a - l + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI).abs() > spread);
            let at = |a: f32, r: f32| ((cx + a.cos() * r).max(0.0), (cz + a.sin() * r).max(0.0));
            let turn = unit(k as i32, 93, seed ^ 0xD3) * std::f32::consts::TAU;
            if !middle {
                let (name, fw, fz) = v.centre;
                if !put(name, (cx - fw as f32 / 2.0) as usize, (cz - fz as f32 / 2.0) as usize, fw, fz, &mut cells, &mut taken) {
                    continue;
                }
                squares.push((cx, cz, SQUARE));
                if sym.on {
                    let (tx, tz) = sym.twin(cx, cz);
                    squares.push((tx, tz, SQUARE));
                }
            }
            // Cottages in a ring round the square, and a second ring round that, staggered.
            for j in 0..24 {
                let (ring, n) = if j < 10 { (0.0, 10.0) } else { (1.0, 14.0) };
                let a = turn + (j as f32 - ring * 10.0 + ring * 0.5) * std::f32::consts::TAU / n;
                if !clear(a, 0.42 - ring * 0.12) || v.houses.is_empty() {
                    continue;
                }
                let (name, fw, fz) = v.houses[(j + k) % v.houses.len()];
                let (hx, hz) = at(a, SQUARE + 6.5 + ring * 6.5 + 1.5 * unit(j as i32, k as i32, seed ^ 0xD4));
                let (x0, z0) = ((hx - fw as f32 / 2.0) as usize, (hz - fz as f32 / 2.0) as usize);
                if level(x0, z0, fw, fz) {
                    put(name, x0, z0, fw, fz, &mut cells, &mut taken);
                }
            }
            // Lanterns round the square, and its decorations about the middle.
            if !v.lamps.is_empty() {
                for j in 0..8 {
                    let a = turn + (j as f32 + 0.5) * std::f32::consts::TAU / 8.0;
                    let (lx, lz) = at(a, SQUARE - 0.5);
                    put(v.lamps[j % v.lamps.len()], lx as usize, lz as usize, 1, 1, &mut cells, &mut taken);
                }
            }
            let mut placed_here = 0;
            for j in 0..24 {
                if placed_here >= 7 || v.decorations.is_empty() {
                    break;
                }
                let (name, fw, fz) = v.decorations[(j * 5 + k * 3) % v.decorations.len()];
                let a = unit(j as i32, k as i32, seed ^ 0xD5) * std::f32::consts::TAU;
                let (dx, dz) = at(a, 4.5 + unit(k as i32, j as i32, seed ^ 0xD6) * (SQUARE - 6.5));
                if put(name, (dx - fw as f32 / 2.0) as usize, (dz - fz as f32 / 2.0) as usize, fw, fz, &mut cells, &mut taken) {
                    placed_here += 1;
                }
            }
            for (j, &(name, fw, fz)) in v.fireworks.iter().cycle().take(if v.fireworks.is_empty() { 0 } else { 5 }).enumerate() {
                let a = turn + 0.3 + j as f32 * std::f32::consts::TAU / 5.0;
                if clear(a, 0.3) {
                    let (fx, fz2) = at(a, SQUARE - 2.6);
                    put(name, (fx - fw as f32 / 2.0) as usize, (fz2 - fz as f32 / 2.0) as usize, fw, fz, &mut cells, &mut taken);
                }
            }
            for (j, &(name, fw, fz)) in v.strings.iter().cycle().take(if v.strings.is_empty() { 0 } else { 2 }).enumerate() {
                let (sx, sz) = (cx, cz + if j == 0 { -(SQUARE + 2.5) } else { SQUARE + 2.5 });
                put(name, (sx - fw as f32 / 2.0).max(0.0) as usize, sz.max(0.0) as usize, fw, fz, &mut cells, &mut taken);
            }
            // Lanterns along the roads near it, just off them, a few cells apart.
            if let (Some(a), false) = (avoid, v.lamps.is_empty()) {
                let mut lit: Vec<(f32, f32)> = Vec::new();
                let reach = 36i64;
                for dz in -reach..=reach {
                    for dx in -reach..=reach {
                        let (x, z) = (cx as i64 + dx, cz as i64 + dz);
                        if x < 1 || z < 1 || x >= w as i64 - 1 || z >= h as i64 - 1 || (dx * dx + dz * dz) as f32 > (reach * reach) as f32 {
                            continue;
                        }
                        let (x, z) = (x as usize, z as usize);
                        let d = a.data[z * w + x];
                        if (2.8..3.4).contains(&d) && lit.iter().all(|&(px, pz)| (px - x as f32).hypot(pz - z as f32) > 9.0) && put(v.lamps[lit.len() % v.lamps.len()], x, z, 1, 1, &mut cells, &mut taken) {
                            lit.push((x as f32, z as f32));
                        }
                    }
                }
            }
        }
        // No forest in the villages: it opens round them.
        if !squares.is_empty() {
            for (i, f) in forest.data.iter_mut().enumerate() {
                let (x, z) = ((i % w) as f32, (i / w) as f32);
                let d = squares.iter().map(|&(sx, sz, r)| (x - sx).hypot(z - sz) - r).fold(f32::MAX, f32::min);
                *f *= smoothstep(8.0, 16.0, d);
            }
        }
    }
    // How far a cell is from the edge of the nearest village's square (cells).
    let village = |x: usize, z: usize| squares.iter().map(|&(sx, sz, r)| (x as f32 - sx).hypot(z as f32 - sz) - r).fold(f32::MAX, f32::min);
    // Groves round the pools: a ring of trees just above the water, thick near it.
    if !kit.grove.is_empty() {
        for z in 0..h {
            for x in 0..w {
                let i = z * w + x;
                let d = land.water_dist.data[i];
                if land.height.data[i] <= land.sea + 1.0 || d > 9.0 || d < 1.0 || near_start(x, z, 40.0) || slope.data[i] > 8.0 {
                    continue;
                }
                let r = unit(x as i32, z as i32, seed ^ 0x8A);
                if r < 0.22 * smoothstep(9.0, 2.0, d) {
                    let k = (unit(x as i32, z as i32, seed ^ 0x8B) * kit.grove.len() as f32) as usize;
                    put(kit.grove[k.min(kit.grove.len() - 1)], x, z, 1, 1, &mut cells, &mut taken);
                }
            }
        }
    }
    if !kit.trees.is_empty() {
        for z in 0..h {
            for x in 0..w {
                let i = z * w + x;
                let f = forest.data[i];
                let r = unit(x as i32, z as i32, seed ^ 0x81);
                let lone = land.height.data[i] > land.sea + 3.0 && slope.data[i] < 8.0 && !near_start(x, z, 40.0) && land.water_dist.data[i] > 2.0;
                // (Not in a village; at its edge, some decorated.)
                let near = if squares.is_empty() { f32::MAX } else { village(x, z) };
                if near < 8.0 {
                    continue;
                }
                if r < f * kit.forest || (lone && r > 0.9975) {
                    let k = (unit(x as i32, z as i32, seed ^ 0x82) * kit.trees.len() as f32) as usize;
                    let firs = kit.village.as_ref().map_or(&[][..], |v| &v.firs[..]);
                    let name = if near < 22.0 && !firs.is_empty() && unit(z as i32, x as i32, seed ^ 0xD7) < 0.18 { firs[k % firs.len()] } else { kit.trees[k.min(kit.trees.len() - 1)] };
                    put(name, x, z, 1, 1, &mut cells, &mut taken);
                } else if !kit.shrubs.is_empty() && f > 0.04 && f < 0.35 && r > 0.985 {
                    let k = (unit(x as i32, z as i32, seed ^ 0x83) * kit.shrubs.len() as f32) as usize;
                    put(kit.shrubs[k.min(kit.shrubs.len() - 1)], x, z, 1, 1, &mut cells, &mut taken);
                }
            }
        }
    }
    // Landmarks on the heights and the broken ground, well away from the starts.
    if !kit.landmarks.is_empty() {
        let wanted = (kit.landmark_density * share * (w * h) as f32 / 1e6) as usize;
        let (mut placed, mut tries) = (0, 0);
        while placed < wanted && tries < wanted * 60 + 100 {
            tries += 1;
            let x = (unit(tries as i32, 31, seed ^ 0xA1) * (w - 16) as f32) as usize + 4;
            let z = (unit(tries as i32, 32, seed ^ 0xA2) * (h - 16) as f32) as usize + 4;
            let i = z * w + x;
            let (name, fw, fz) = kit.landmarks[tries % kit.landmarks.len()];
            if land.height.data[i] < land.sea + 12.0 || near_start(x, z, 70.0) || !level(x, z, fw, fz) {
                continue;
            }
            if put(name, x, z, fw, fz, &mut cells, &mut taken) {
                placed += 1;
            }
        }
    }
    // Ruins on level open ground, in clusters, away from the starts.
    if !kit.ruins.is_empty() {
        let wanted = (kit.ruin_density * share * (w * h) as f32 / 1e6) as usize;
        let (mut placed, mut tries) = (0, 0);
        while placed < wanted && tries < wanted * 80 + 100 {
            tries += 1;
            let x = (unit(tries as i32, 41, seed ^ 0xB1) * (w - 24) as f32) as usize + 8;
            let z = (unit(tries as i32, 42, seed ^ 0xB2) * (h - 24) as f32) as usize + 8;
            if land.height.data[z * w + x] < land.sea + 4.0 || near_start(x, z, 55.0) || land.water_dist.data[z * w + x] < 4.0 {
                continue;
            }
            // A cluster of two to four round the spot.
            for j in 0..2 + (unit(tries as i32, 43, seed) * 3.0) as usize {
                let (name, fw, fz) = kit.ruins[(tries * 7 + j * 3) % kit.ruins.len()];
                let (dx, dz) = ((unit(tries as i32, 50 + j as i32, seed) * 14.0) as usize, (unit(tries as i32, 60 + j as i32, seed) * 14.0) as usize);
                if level(x + dx, z + dz, fw, fz) && put(name, x + dx, z + dz, fw, fz, &mut cells, &mut taken) {
                    placed += 1;
                }
            }
        }
    }
    // Rocks on the broken ground: small ones scattered, big ones fewer.
    for z in 0..h {
        for x in 0..w {
            let i = z * w + x;
            let s = slope.data[i];
            if land.height.data[i] < land.sea + 2.0 || near_start(x, z, 34.0) {
                continue;
            }
            let rough = smoothstep(4.0, 10.0, s) * smoothstep(20.0, 13.0, s);
            let r = unit(x as i32, z as i32, seed ^ 0x91);
            if !kit.rocks_small.is_empty() && r < rough * 0.012 + 0.0004 {
                let k = (unit(z as i32, x as i32, seed ^ 0x92) * kit.rocks_small.len() as f32) as usize;
                put(kit.rocks_small[k.min(kit.rocks_small.len() - 1)], x, z, 1, 1, &mut cells, &mut taken);
            } else if !kit.rocks_big.is_empty() && r > 1.0 - (rough * 0.0016 + 0.00006) {
                let (name, fw, fz) = kit.rocks_big[(unit(z as i32, x as i32, seed ^ 0x93) * kit.rocks_big.len() as f32) as usize % kit.rocks_big.len()];
                if level(x, z, fw, fz) {
                    put(name, x, z, fw, fz, &mut cells, &mut taken);
                }
            }
        }
    }
    // The second half: each feature's twin, its footprint turned round with it.
    let mut all = firsts.clone();
    if sym.on {
        for &(slot, x, z, fw, fz) in &firsts {
            let (tx, tz) = (w - x - fw, h - z - fz);
            // (The middle village's centrepiece is its own twin.)
            if (tx, tz) == (x, z) {
                continue;
            }
            cells[tz * w + tx] = slot;
            count += 1;
            all.push((slot, tx, tz, fw, fz));
        }
    }
    let passable = |name: &str| kit.metal.iter().chain(&kit.vents).chain(&kit.sea_metal).chain(kit.reef.iter().map(|r| &r.0)).any(|m| m.eq_ignore_ascii_case(name));
    let mut blocked = vec![false; w * h];
    for &(slot, x, z, fw, fz) in &all {
        if passable(&names[slot as usize]) {
            continue;
        }
        for dz in 0..fz {
            for dx in 0..fw {
                blocked[(z + dz) * w + x + dx] = true;
            }
        }
    }
    Placed { cells, names, forest, count, blocked, squares }
}
