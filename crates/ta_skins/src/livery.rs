//! Liveries: paint the renderer lays over a unit's own (ta_render's `shaders/livery.wgsl`), fixed
//! to each of its pieces so it moves and turns with them. Up to three colours in a pattern, a
//! trim along the convex edges, how much of the paint under it it covers and lets show through
//! (TA Reborn's paint, or TA's own as TA drew it), its finish, and how worn, dirty and faded it
//! is. A few numbers, so it is shared as words in `skin.toml` and changes from frame to frame
//! without new textures.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An sRGB colour, written `"#rrggbb"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Colour(pub [u8; 3]);

impl Colour {
    pub const fn rgb(rgb: u32) -> Self {
        Self([(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8])
    }

    pub fn hex(self) -> String {
        let [r, g, b] = self.0;
        format!("#{r:02x}{g:02x}{b:02x}")
    }

    /// `#rrggbb` or `rrggbb`, in either case.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.trim().strip_prefix('#').unwrap_or(text.trim());
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(digits, 16).ok().map(Self::rgb)
    }
}

impl Serialize for Colour {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.hex())
    }
}

impl<'de> Deserialize<'de> for Colour {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).ok_or_else(|| serde::de::Error::custom(format!("'{text}' is not a colour: write it as \"#rrggbb\"")))
    }
}

/// How a livery's colours are laid out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pattern {
    /// The first colour all over.
    #[default]
    Solid,
    /// Soft-edged patches of the second and third colours over the first (woodland or desert
    /// camouflage).
    Blotch,
    /// Angular shards of all three.
    Splinter,
    /// Patches in square pixels.
    Digital,
    /// Bands of the second colour across the first, thinner ones of the third between them.
    Stripes,
    /// Hexagonal plates of the three, with dark gaps between them.
    Hex,
    /// Waves of the first two colours with snowflakes of the third.
    Festive,
    /// Equal bands of the colours in turn, straight across the length: a tricolour's.
    Tricolour,
    /// Crosses upright and diagonal, broad ones of the second colour and narrow ones of the
    /// third over the first: a union flag's.
    Union,
    /// Rows of five-pointed stars of the second colour on the first; with a third, bands of it
    /// between the rows: stars and stripes.
    Stars,
}

impl Pattern {
    pub const ALL: [Pattern; 10] =
        [Self::Solid, Self::Blotch, Self::Splinter, Self::Digital, Self::Stripes, Self::Hex, Self::Festive, Self::Tricolour, Self::Union, Self::Stars];

    /// Its place in the renderer's list (`livery.wgsl`).
    pub fn index(self) -> u32 {
        Self::ALL.iter().position(|p| *p == self).unwrap_or(0) as u32
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Livery {
    /// One to three colours: the ground, then the pattern's.
    #[serde(alias = "colors")]
    pub colours: Vec<Colour>,
    /// A colour along the convex edges, or none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trim: Option<Colour>,
    #[serde(default)]
    pub pattern: Pattern,
    /// The pattern's size, 0.25 to 4 (1: patches about a third of a light tank).
    #[serde(default = "one", serialize_with = "tidy")]
    pub scale: f32,
    /// How crisp the pattern's edges are, 0 (soft) to 1 (hard).
    #[serde(default = "sharpness", serialize_with = "tidy")]
    pub sharpness: f32,
    /// Another layout of the same pattern.
    #[serde(default)]
    pub seed: u32,
    /// How much of the paint under it the livery covers, 0 (none: the unit's own colours, worn
    /// and dirtied as below) to 1.
    #[serde(default = "one", serialize_with = "tidy")]
    pub coverage: f32,
    /// How much of the paint's own light and dark (panel lines, seams, markings) shows through,
    /// 0 (flat colour) to 1.
    #[serde(default = "detail", serialize_with = "tidy")]
    pub detail: f32,
    /// 0 (matte) to 1 (glossy).
    #[serde(default = "gloss", serialize_with = "tidy")]
    pub gloss: f32,
    /// 0 (paint) to 1 (bare metal).
    #[serde(default, serialize_with = "tidy")]
    pub metal: f32,
    /// 0 (factory fresh) to 1 (battle-worn: chipped to the metal); 0.5 as Reborn's own units.
    #[serde(default = "wear", serialize_with = "tidy")]
    pub wear: f32,
    /// Dirt in the creases and streaks, 0 to 1.
    #[serde(default, serialize_with = "tidy")]
    pub grime: f32,
    /// Sun-bleached: the paint's colours washed out towards a dusty grey and its finish chalky,
    /// 0 (as painted) to 1.
    #[serde(default, skip_serializing_if = "is_zero", serialize_with = "tidy")]
    pub fade: f32,
    /// On TA's own shapes and paint as TA drew them (its models without Reborn's kit of plates,
    /// bolts and road wheels; its textures' crisp texels and their painted light) rather than
    /// TA Reborn's; the same in the Enhanced Classic style, which shows TA's anyway.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub original: bool,
}

fn is_zero(value: &f32) -> bool { *value == 0.0 }

fn one() -> f32 { 1.0 }
fn sharpness() -> f32 { 0.7 }
fn detail() -> f32 { 0.8 }
fn gloss() -> f32 { 0.35 }
fn wear() -> f32 { 0.5 }

/// Written to three places: a slider's value is not a measurement.
fn tidy<S: Serializer>(value: &f32, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_f64((f64::from(*value) * 1000.0).round() / 1000.0)
}

pub const MAX_COLOURS: usize = 3;
pub const SCALE: std::ops::RangeInclusive<f32> = 0.25..=4.0;

impl Livery {
    /// A livery of these colours in this pattern, the rest as by default.
    pub fn new(pattern: Pattern, colours: &[Colour]) -> Self {
        Self {
            colours: colours.to_vec(),
            trim: None,
            pattern,
            scale: one(),
            sharpness: sharpness(),
            seed: 0,
            coverage: one(),
            detail: detail(),
            gloss: gloss(),
            metal: 0.0,
            wear: wear(),
            grime: 0.0,
            fade: 0.0,
            original: false,
        }
    }

    /// Paint that changes nothing: Reborn's own colours, finish and wear.
    pub fn none() -> Self {
        Self { coverage: 0.0, ..Self::new(Pattern::Solid, &[Colour::rgb(0x808080)]) }
    }

    /// Whether every value is one a manifest may hold.
    pub fn check(&self) -> Result<(), String> {
        if self.colours.is_empty() || self.colours.len() > MAX_COLOURS {
            return Err(format!("a livery has one to {MAX_COLOURS} colours"));
        }
        if !self.scale.is_finite() || !SCALE.contains(&self.scale) {
            return Err(format!("livery scale {} is outside 0.25 to 4", self.scale));
        }
        for (name, value) in [("sharpness", self.sharpness), ("coverage", self.coverage), ("detail", self.detail), ("gloss", self.gloss),
            ("metal", self.metal), ("wear", self.wear), ("grime", self.grime), ("fade", self.fade)] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(format!("livery {name} {value} is outside 0 to 1"));
            }
        }
        Ok(())
    }
}

/// The built-in skins' liveries, by id (none for Reborn).
pub(crate) fn builtin(id: &str) -> Option<Livery> {
    let c = Colour::rgb;
    Some(match id {
        // TA's own shapes and paint as TA drew them, a veteran's: a little sun-faded, chipped,
        // grimy.
        "builtin:heritage" => Livery { coverage: 0.0, original: true, fade: 0.12, wear: 0.8, grime: 0.55,
            ..Livery::new(Pattern::Solid, &[c(0x7a7f6c)]) },
        "christmas" => Livery { trim: Some(c(0xc9a646)), sharpness: 0.9, detail: 0.85, gloss: 0.55, metal: 0.05, wear: 0.3,
            ..Livery::new(Pattern::Festive, &[c(0x0e5a27), c(0xa3171f), c(0xe9eef0)]) },
        "builtin:halloween" => Livery { trim: Some(c(0xe0761c)), scale: 0.9, sharpness: 0.9, seed: 31, gloss: 0.35, metal: 0.1, wear: 0.4, grime: 0.3,
            ..Livery::new(Pattern::Splinter, &[c(0x47295e), c(0x16111a), c(0x6b3f8a)]) },
        "builtin:desert" => Livery { scale: 1.1, sharpness: 0.7, seed: 11, gloss: 0.15, wear: 0.6, grime: 0.35,
            ..Livery::new(Pattern::Blotch, &[c(0xc4a676), c(0x9b7445), c(0x5e4630)]) },
        "builtin:winter" => Livery { scale: 1.25, sharpness: 0.45, seed: 23, coverage: 0.92, detail: 0.85, gloss: 0.2, wear: 0.75, grime: 0.25,
            ..Livery::new(Pattern::Blotch, &[c(0xe4e7e6), c(0xa6adb0), c(0x5b646a)]) },
        "builtin:woodlands" => Livery { scale: 0.95, sharpness: 0.9, seed: 5, gloss: 0.15, wear: 0.55, grime: 0.4,
            ..Livery::new(Pattern::Blotch, &[c(0x4f5d34), c(0x5f4a32), c(0x22231f)]) },
        "builtin:navy" => Livery { scale: 1.4, sharpness: 0.95, seed: 7, detail: 0.75, gloss: 0.45, metal: 0.1, wear: 0.45, grime: 0.3,
            ..Livery::new(Pattern::Splinter, &[c(0x8e99a0), c(0x5f6c75), c(0x333e48)]) },
        "builtin:urban" => Livery { sharpness: 1.0, seed: 3, gloss: 0.25, grime: 0.3,
            ..Livery::new(Pattern::Digital, &[c(0xb7b9b8), c(0x7f8386), c(0x3e4144)]) },
        "builtin:midnight" => Livery { scale: 0.8, sharpness: 0.9, gloss: 0.5, metal: 0.35, wear: 0.4, grime: 0.1,
            ..Livery::new(Pattern::Hex, &[c(0x1b1f27), c(0x232a36), c(0x12151a)]) },
        "builtin:tiger" => Livery { sharpness: 0.85, seed: 2, gloss: 0.3, wear: 0.55, grime: 0.25,
            ..Livery::new(Pattern::Stripes, &[c(0xc86a1e), c(0x1c1712)]) },
        "builtin:ceremonial" => Livery { trim: Some(c(0xc9a44c)), detail: 0.6, gloss: 0.8, metal: 0.1, wear: 0.15,
            ..Livery::new(Pattern::Solid, &[c(0xece8dc)]) },
        // The nations' colours (their flags' and their teams', the whites a touch off white).
        "builtin:uk" => Livery { scale: 1.15, sharpness: 0.95, detail: 0.7, gloss: 0.4, wear: 0.4, grime: 0.15,
            ..Livery::new(Pattern::Union, &[c(0x0d2a6b), c(0xe9e9e4), c(0xc8102e)]) },
        "builtin:usa" => Livery { trim: Some(c(0xb22234)), sharpness: 0.95, detail: 0.7, gloss: 0.4, wear: 0.4, grime: 0.15,
            ..Livery::new(Pattern::Stars, &[c(0x2e3a6e), c(0xe9e9e4), c(0xb22234)]) },
        "builtin:australia" => Livery { trim: Some(c(0xffcd00)), scale: 1.1, sharpness: 0.95, detail: 0.7, gloss: 0.45, wear: 0.4, grime: 0.15,
            ..Livery::new(Pattern::Stars, &[c(0x00733a), c(0xffcd00)]) },
        "builtin:new-zealand" => Livery { trim: Some(c(0xc3c7cb)), detail: 0.75, gloss: 0.3, metal: 0.05, wear: 0.35, grime: 0.1,
            ..Livery::new(Pattern::Solid, &[c(0x161616)]) },
        "builtin:italy" => Livery { sharpness: 1.0, detail: 0.7, gloss: 0.5, wear: 0.35, grime: 0.1,
            ..Livery::new(Pattern::Tricolour, &[c(0x009246), c(0xeeeeea), c(0xce2b37)]) },
        "builtin:france" => Livery { sharpness: 1.0, detail: 0.7, gloss: 0.5, wear: 0.35, grime: 0.1,
            ..Livery::new(Pattern::Tricolour, &[c(0x0055a4), c(0xeeeeea), c(0xef4135)]) },
        "builtin:germany" => Livery { sharpness: 1.0, detail: 0.7, gloss: 0.5, wear: 0.35, grime: 0.1,
            ..Livery::new(Pattern::Tricolour, &[c(0x1b1b1b), c(0xdd0000), c(0xffce00)]) },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_read_and_write_as_hex() {
        assert_eq!(Colour::parse("#C4a676"), Some(Colour([0xc4, 0xa6, 0x76])));
        assert_eq!(Colour::parse("c4a676"), Some(Colour([0xc4, 0xa6, 0x76])));
        for bad in ["", "#c4a67", "#c4a6767", "#g4a676", "red", "#+4a676"] {
            assert_eq!(Colour::parse(bad), None, "{bad}");
        }
        assert_eq!(Colour::rgb(0x0e5a27).hex(), "#0e5a27");
    }

    #[test]
    fn a_livery_round_trips_through_toml_with_tidy_numbers() {
        let livery = builtin("builtin:winter").unwrap();
        let text = toml::to_string(&livery).unwrap();
        assert!(text.contains("coverage = 0.92\n"), "{text}");
        assert!(text.contains("colours = [\"#e4e7e6\", \"#a6adb0\", \"#5b646a\"]"), "{text}");
        assert!(!text.contains("trim"), "an absent trim is left out: {text}");
        let back: Livery = toml::from_str(&text).unwrap();
        assert_eq!(back, livery);
        let american: Livery = toml::from_str("colors = [\"#ffffff\"]").unwrap();
        assert_eq!(american, Livery::new(Pattern::Solid, &[Colour::rgb(0xffffff)]));
        assert!(toml::from_str::<Livery>("colours = [\"#ffffff\"]\npatern = \"hex\"").is_err(), "misspelt fields are caught");
        assert!(!text.contains("fade") && !text.contains("original"), "a livery as painted over Reborn's paint writes neither: {text}");
        let heritage = builtin("builtin:heritage").unwrap();
        let text = toml::to_string(&heritage).unwrap();
        assert!(text.contains("fade = 0.12\n") && text.contains("original = true\n"), "{text}");
        assert_eq!(toml::from_str::<Livery>(&text).unwrap(), heritage);
        assert!(toml::from_str::<Livery>("colours = [\"white\"]").is_err());
    }

    #[test]
    fn every_builtin_livery_is_valid_and_out_of_range_values_are_not() {
        for id in ["builtin:heritage", "christmas", "builtin:halloween", "builtin:desert", "builtin:winter", "builtin:woodlands", "builtin:navy",
            "builtin:urban", "builtin:midnight", "builtin:tiger", "builtin:ceremonial", "builtin:uk", "builtin:usa", "builtin:australia",
            "builtin:new-zealand", "builtin:italy", "builtin:france", "builtin:germany"] {
            builtin(id).unwrap().check().unwrap_or_else(|e| panic!("{id}: {e}"));
        }
        Livery::none().check().unwrap();
        let good = Livery::new(Pattern::Hex, &[Colour::rgb(0)]);
        for bad in [Livery { colours: Vec::new(), ..good.clone() }, Livery { colours: vec![Colour::rgb(0); 4], ..good.clone() },
            Livery { scale: 0.1, ..good.clone() }, Livery { scale: f32::NAN, ..good.clone() }, Livery { gloss: 1.5, ..good.clone() },
            Livery { grime: -0.1, ..good.clone() }, Livery { wear: f32::INFINITY, ..good.clone() }, Livery { fade: 1.2, ..good.clone() }] {
            assert!(bad.check().is_err(), "{bad:?}");
        }
        assert_eq!(Pattern::Festive.index(), 6);
        // (The shader's order: those of before keep their places.)
        assert_eq!([Pattern::Tricolour, Pattern::Union, Pattern::Stars].map(Pattern::index), [7, 8, 9]);
    }
}
