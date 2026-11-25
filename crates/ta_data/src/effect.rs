//! Particle effects of a mod's own (`effects/*.tdf`; TA Reborn's, not TA's): each top-level
//! section an effect, named by its section, each section within it a burst of particles the
//! battle's renderer makes, moves and draws itself (ta_render's `ParticleBurst`). A weapon plays
//! them where its shots leave, along their flight and where they hit (`fireeffect`,
//! `traileffect`, `hiteffect`, `waterhiteffect`: [`crate::weapon::WeaponLook`]), a unit by its
//! script (`emit-sfx 1024 + n`, its `SfxEffects`' nth, counted from 0).
//!
//! ```text
//! [GAUSS_HIT]
//!     {
//!     [FLASH]  { kind=glow; size=40 12; life=0.1 0.15; color=1 0.85 0.6; endcolor=1 0.4 0.1; intensity=3; }
//!     [SPARKS] { kind=spark; count=12; direction=up; cone=60; speed=60 200; life=0.2 0.5; }
//!     }
//! ```
//!
//! Every key of a burst but `kind` may be left out: it then takes its kind's own (a spark's
//! fall, a flame's rise). Presentation only: the simulation never reads any of it, and players
//! with other effects play together.

use ta_formats::tdf::TdfSection;

/// The most bursts an effect plays (the rest are left out) and the most particles a burst makes
/// (`count` is held to it), so that an effect costs at most so much however it is written.
pub const MAX_BURSTS: usize = 16;
pub const MAX_COUNT: u32 = 256;
/// The longest a particle lives, the widest it is and the longest a burst's births are spread
/// over or wait (seconds, pixels): past them a mod's numbers are held to them.
pub const MAX_LIFE: f32 = 20.0;
pub const MAX_SIZE: f32 = 2048.0;
pub const MAX_SPEED: f32 = 4096.0;
pub const MAX_WAIT: f32 = 10.0;

/// What a burst's particles are: those of ta_render's `ParticleKind`, and a ring running out over
/// the ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BurstKind {
    /// White-hot sparks streaking along their flight, cooling as they fall (`cold`: keeping their
    /// colour).
    Spark,
    /// Glowing motes rising on the hot air.
    Ember,
    /// A soft glow fading out: flashes, flares, plasma.
    Glow,
    /// Flames billowing up.
    Fire,
    /// Dust spreading low over the ground.
    Dust,
    /// Smoke rising and spreading.
    Smoke,
    /// Grit and clods thrown up, falling back.
    Dirt,
    /// Droplets of spray.
    Spray,
    /// A ball of flame cooling into soot.
    Fireball,
    /// A ring on the ground (or the water) running out from where the effect plays: from nothing
    /// to `size`'s second number across, its band as wide as the first; `count` is not read.
    Ring,
}

impl BurstKind {
    pub const ALL: [(&'static str, BurstKind); 10] = [
        ("spark", BurstKind::Spark),
        ("ember", BurstKind::Ember),
        ("glow", BurstKind::Glow),
        ("fire", BurstKind::Fire),
        ("dust", BurstKind::Dust),
        ("smoke", BurstKind::Smoke),
        ("dirt", BurstKind::Dirt),
        ("spray", BurstKind::Spray),
        ("fireball", BurstKind::Fireball),
        ("ring", BurstKind::Ring),
    ];

    /// The kind a burst's `kind` names (any case).
    pub fn named(name: &str) -> Option<BurstKind> {
        let name = name.trim();
        BurstKind::ALL.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|&(_, k)| k)
    }

    pub fn name(self) -> &'static str {
        BurstKind::ALL.iter().find(|&&(_, k)| k == self).map_or("", |&(n, _)| n)
    }
}

/// Which way a burst's particles fly, in the frame the effect plays in: `Forward` the way a shot
/// leaves its muzzle, the way it flies along its trail, back the way it came where it hits (up
/// where that is not known) and the way a unit's piece points for its script's effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BurstDirection {
    Forward,
    Back,
    Up,
    Down,
    /// Out from the burst's middle every way, as a blast's do (its `cone` round the way out).
    Out,
}

impl BurstDirection {
    pub const ALL: [(&'static str, BurstDirection); 5] = [("forward", BurstDirection::Forward), ("back", BurstDirection::Back), ("up", BurstDirection::Up), ("down", BurstDirection::Down), ("out", BurstDirection::Out)];

    pub fn named(name: &str) -> Option<BurstDirection> {
        let name = name.trim();
        BurstDirection::ALL.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|&(_, d)| d)
    }
}

/// One burst of an effect. What is `None` is the kind's own.
#[derive(Clone, Debug, PartialEq)]
pub struct BurstDef {
    pub name: String,
    pub kind: BurstKind,
    /// Particles (at most [`MAX_COUNT`]).
    pub count: u32,
    /// They start at random in a ball this big (pixels, its radius) round the burst's middle...
    pub radius: Option<f32>,
    /// ...squashed up and down by this (0: a flat disc, 1: a ball).
    pub vertical: Option<f32>,
    /// Which way they fly, and how far from it (`cone`, degrees either side).
    pub direction: Option<BurstDirection>,
    pub cone: Option<f32>,
    /// Their speeds (pixels a second, the least and the most).
    pub speed: Option<[f32; 2]>,
    /// The share of the effect's own speed they take with them (a trail's shot's, 0 to 1).
    pub inherit: f32,
    /// How long each lives (seconds, the least and the most).
    pub life: Option<[f32; 2]>,
    /// How big each is across (pixels): as it is born, and as it dies.
    pub size: Option<[f32; 2]>,
    /// Its colour (linear red, green and blue, 0 to 1 or more), and the colour it turns to as it
    /// dies (`endcolor`; none: its colour throughout).
    pub color: Option<[f32; 3]>,
    pub end_color: Option<[f32; 3]>,
    /// How bright the glowing kinds are (sparks, embers, glows, flames, fireballs), how opaque
    /// the others (0 to 1); a ring's opacity.
    pub intensity: Option<f32>,
    /// How fast they fall (pixels a second, a second; less than none rises) and how the air
    /// slows them (their speed falling by e^-drag a second).
    pub gravity: Option<f32>,
    pub drag: Option<f32>,
    /// Their births spread over this long (seconds), after this long (`delay`).
    pub over: f32,
    pub delay: f32,
    /// Where its middle is from the effect's place (pixels: to the right, up, forward).
    pub offset: [f32; 3],
    /// Whether they bounce off and rest on the ground, and die in water.
    pub collide: Option<bool>,
    /// Sparks keeping their colour rather than cooling as hot metal does (energy, electricity).
    pub cold: bool,
    /// Taking the colour of the ground where they start (dust, grit), times their `color`.
    pub ground: bool,
}

/// An effect: its bursts, played together.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EffectDef {
    /// Its section's name, in capitals.
    pub name: String,
    pub bursts: Vec<BurstDef>,
}

/// What reading an effect found wrong (each a sentence's words: the burst, what, why).
#[derive(Clone, Debug, PartialEq)]
pub enum EffectProblem {
    /// A burst whose `kind` is not one of [`BurstKind::ALL`] (or is not given): left out.
    UnknownKind { burst: String, kind: String },
    /// A key whose value does not read (`count=many;`): the kind's own taken.
    BadValue { burst: String, key: String, value: String },
    /// More bursts than an effect plays: those after [`MAX_BURSTS`] left out.
    TooManyBursts { bursts: usize },
    /// A burst asking for more particles than [`MAX_COUNT`]: held to it.
    TooMany { burst: String, count: i64 },
    /// An effect of no bursts: it plays nothing.
    Empty,
}

/// Numbers by spaces or commas (`size=40 12;`, `color=1,0.5,0;`), every one of them finite.
fn numbers(value: &str) -> Option<Vec<f32>> {
    let parts: Vec<f32> = value.split([' ', ',', '\t']).filter(|p| !p.is_empty()).map(str::parse::<f32>).collect::<Result<_, _>>().ok()?;
    (!parts.is_empty() && parts.iter().all(|p| p.is_finite())).then_some(parts)
}

impl EffectDef {
    /// An effect from its section (`effects/*.tdf`'s top level), and what was wrong with it.
    pub fn from_tdf(s: &TdfSection) -> (EffectDef, Vec<EffectProblem>) {
        let mut problems = Vec::new();
        let mut bursts = Vec::new();
        if s.sections.len() > MAX_BURSTS {
            problems.push(EffectProblem::TooManyBursts { bursts: s.sections.len() });
        }
        for b in s.sections.iter().take(MAX_BURSTS) {
            if let Some(burst) = BurstDef::from_tdf(b, &mut problems) {
                bursts.push(burst);
            }
        }
        if bursts.is_empty() && !problems.iter().any(|p| matches!(p, EffectProblem::UnknownKind { .. })) {
            problems.push(EffectProblem::Empty);
        }
        (EffectDef { name: s.name.trim().to_ascii_uppercase(), bursts }, problems)
    }

    /// The most particles it makes when played (its bursts' counts; a ring, one).
    pub fn particles(&self) -> u32 {
        self.bursts.iter().map(|b| if b.kind == BurstKind::Ring { 1 } else { b.count }).sum()
    }
}

impl BurstDef {
    fn from_tdf(s: &TdfSection, problems: &mut Vec<EffectProblem>) -> Option<BurstDef> {
        let burst = s.name.trim().to_string();
        let raw = s.get("kind").unwrap_or("").trim();
        let Some(kind) = BurstKind::named(raw) else {
            problems.push(EffectProblem::UnknownKind { burst, kind: raw.to_string() });
            return None;
        };
        let one =|v: Option<Vec<f32>>, lo: f32, hi: f32| v.map(|v| v[0].clamp(lo, hi));
        let pair = |v: Option<Vec<f32>>, lo: f32, hi: f32| v.map(|v| [v[0].clamp(lo, hi), v[1].clamp(lo, hi)]);
        let colour = |v: Option<Vec<f32>>| v.map(|v| [v[0], v[1], v[2]].map(|c| c.clamp(0.0, 64.0)));
        let count = match s.get("count") {
            None => 1,
            Some(value) => match value.trim().parse::<f64>() {
                Ok(n) if n.is_finite() => {
                    if n > MAX_COUNT as f64 {
                        problems.push(EffectProblem::TooMany { burst: burst.clone(), count: n as i64 });
                    }
                    (n.round().max(0.0) as u32).min(MAX_COUNT)
                }
                _ => {
                    problems.push(EffectProblem::BadValue { burst: burst.clone(), key: "count".into(), value: value.to_string() });
                    1
                }
            },
        };
        let direction = s.get("direction").and_then(|d| {
            let found = BurstDirection::named(d);
            if found.is_none() {
                problems.push(EffectProblem::BadValue { burst: burst.clone(), key: "direction".into(), value: d.to_string() });
            }
            found
        });
        // (A key of `n` numbers: one number given where two are wanted serves for both.)
        let mut get = |key: &str, n: usize| -> Option<Vec<f32>> {
            let value = s.get(key)?;
            match numbers(value) {
                Some(v) if v.len() == n => Some(v),
                Some(v) if v.len() == 1 && n == 2 => Some(vec![v[0], v[0]]),
                _ => {
                    problems.push(EffectProblem::BadValue { burst: burst.clone(), key: key.to_string(), value: value.to_string() });
                    None
                }
            }
        };
        let flag = |key: &str| s.get(key).map(|v| v.trim() != "0" && !v.trim().eq_ignore_ascii_case("false"));
        let def = BurstDef {
            name: burst.clone(),
            kind,
            count,
            radius: one(get("radius", 1), 0.0, MAX_SIZE),
            vertical: one(get("vertical", 1), 0.0, 1.0),
            direction,
            cone: one(get("cone", 1), 0.0, 180.0),
            speed: pair(get("speed", 2), 0.0, MAX_SPEED),
            inherit: one(get("inherit", 1), 0.0, 1.0).unwrap_or(0.0),
            life: pair(get("life", 2), 0.0, MAX_LIFE),
            size: pair(get("size", 2), 0.0, MAX_SIZE),
            color: colour(get("color", 3)),
            end_color: colour(get("endcolor", 3)),
            intensity: one(get("intensity", 1), 0.0, 64.0),
            gravity: one(get("gravity", 1), -MAX_SPEED, MAX_SPEED),
            drag: one(get("drag", 1), 0.0, 100.0),
            over: one(get("over", 1), 0.0, MAX_WAIT).unwrap_or(0.0),
            delay: one(get("delay", 1), 0.0, MAX_WAIT).unwrap_or(0.0),
            offset: get("offset", 3).map_or([0.0; 3], |v| [v[0], v[1], v[2]].map(|c| c.clamp(-MAX_SIZE, MAX_SIZE))),
            collide: flag("collide"),
            cold: flag("cold").unwrap_or(false),
            ground: flag("ground").unwrap_or(false),
        };
        Some(def)
    }
}

/// A unit's own effects (`SfxEffects`, in its `[UNITINFO]`): names by spaces or commas, in order;
/// its script's `emit-sfx 1024 + n` plays the nth, counted from 0 (as Spring counts its explosion
/// generators).
pub fn unit_effects(s: &TdfSection) -> Vec<String> {
    s.get("SfxEffects").map_or_else(Vec::new, |v| v.split([' ', ',', '\t']).filter(|n| !n.is_empty()).map(|n| n.to_ascii_uppercase()).collect())
}

/// The `emit-sfx` kinds that play a unit's own effects: 1024 + the effect's place in its list.
pub const SFX_EFFECT: i32 = 1024;
/// How many of a unit's effects its script can play (`emit-sfx` 1024 to 2047).
pub const MAX_UNIT_EFFECTS: usize = 1024;

/// The place in its unit's list of the effect an `emit-sfx` of `kind` plays, if it plays one.
pub fn sfx_effect(kind: i32) -> Option<usize> {
    (SFX_EFFECT..SFX_EFFECT + MAX_UNIT_EFFECTS as i32).contains(&kind).then(|| (kind - SFX_EFFECT) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_formats::tdf;

    fn effect(text: &str) -> (EffectDef, Vec<EffectProblem>) {
        EffectDef::from_tdf(&tdf::parse(text).sections[0])
    }

    /// An effect's bursts as written: what each gives is read, what it leaves out is the kind's
    /// own (none here), its colour and the one it turns to, its numbers held to their bounds.
    #[test]
    fn an_effect_is_read_with_its_bursts() {
        let (e, problems) = effect(
            "[Gauss_Hit] { [FLASH] { kind=Glow; size=40 12; life=0.1 0.15; color=1 0.85 0.6; endcolor=1,0.4,0.1; intensity=3; } \
             [SPARKS] { kind=spark; count=12; direction=up; cone=60; speed=60 200; life=0.2; gravity=-30; cold=1; offset=0 4 2; inherit=2; } \
             [RING] { kind=ring; size=6 80; life=0.5; } }",
        );
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(e.name, "GAUSS_HIT");
        assert_eq!(e.bursts.len(), 3);
        let flash = &e.bursts[0];
        assert_eq!((flash.kind, flash.count, flash.size, flash.life), (BurstKind::Glow, 1, Some([40.0, 12.0]), Some([0.1, 0.15])));
        assert_eq!((flash.color, flash.end_color, flash.intensity), (Some([1.0, 0.85, 0.6]), Some([1.0, 0.4, 0.1]), Some(3.0)));
        assert_eq!((flash.speed, flash.gravity, flash.direction, flash.collide), (None, None, None, None), "the kind's own");
        let sparks = &e.bursts[1];
        assert_eq!((sparks.count, sparks.direction, sparks.cone, sparks.speed), (12, Some(BurstDirection::Up), Some(60.0), Some([60.0, 200.0])));
        assert_eq!((sparks.life, sparks.gravity, sparks.cold, sparks.offset, sparks.inherit), (Some([0.2, 0.2]), Some(-30.0), true, [0.0, 4.0, 2.0], 1.0), "one number for both; the share held to 1");
        assert_eq!(e.particles(), 1 + 12 + 1);
    }

    /// What does not read is said, and the effect plays what it can: a burst of no kind is left
    /// out, a number that is none takes the kind's own, a count past the most is held to it, the
    /// bursts past the most are left out.
    #[test]
    fn what_does_not_read_is_said_and_the_rest_plays() {
        let (e, problems) = effect("[X] { [A] { kind=laser; } [B] { kind=smoke; count=5000; speed=fast; direction=sideways; } }");
        assert_eq!(e.bursts.len(), 1);
        assert_eq!((e.bursts[0].count, e.bursts[0].speed, e.bursts[0].direction), (MAX_COUNT, None, None));
        assert!(problems.contains(&EffectProblem::UnknownKind { burst: "A".into(), kind: "laser".into() }));
        assert!(problems.contains(&EffectProblem::TooMany { burst: "B".into(), count: 5000 }));
        assert!(problems.iter().any(|p| matches!(p, EffectProblem::BadValue { key, .. } if key == "speed")));
        assert!(problems.iter().any(|p| matches!(p, EffectProblem::BadValue { key, .. } if key == "direction")));
        let many: String = (0..20).map(|i| format!("[B{i}] {{ kind=glow; }}")).collect();
        let (e, problems) = effect(&format!("[Y] {{ {many} }}"));
        assert_eq!(e.bursts.len(), MAX_BURSTS);
        assert!(problems.contains(&EffectProblem::TooManyBursts { bursts: 20 }));
        assert_eq!(effect("[Z] { }").1, vec![EffectProblem::Empty]);
    }

    /// A unit's own effects, and which `emit-sfx` plays which: 1024 the first.
    #[test]
    fn a_units_effects_are_played_from_1024() {
        let doc = tdf::parse("[UNITINFO] { UnitName=X; SfxEffects=sparks, steam  Glow; }");
        assert_eq!(unit_effects(&doc.sections[0]), ["SPARKS", "STEAM", "GLOW"]);
        assert_eq!((sfx_effect(1024), sfx_effect(1026), sfx_effect(2047)), (Some(0), Some(2), Some(1023)));
        assert_eq!((sfx_effect(256 | 2), sfx_effect(1023), sfx_effect(2048)), (None, None, None), "smoke and Spring's weapon numbers are none");
    }
}
