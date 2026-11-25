//! Forged Alliance's weapon effects as its scripts and blueprints name them: what a weapon shows
//! at its muzzle (its script class's `FxMuzzleFlash`), what a beam weapon draws and shows at its
//! two ends (its `BeamType`'s `FxBeam`, `FxBeamStartPoint`, `FxBeamEndPoint`), and how its
//! projectile looks in flight and where it lands (its script class's `FxTrails`, `PolyTrail(s)`,
//! `BeamName` and `FxImpactLand`, `FxImpactUnit`, `FxImpactWater`...). The classes are those of the
//! game's own Lua (`lua/terranweapons.lua`, `lua/cybranprojectiles.lua`... and their bases), the
//! lists often named by `lua/EffectTemplates.lua`; every name leads to emitter blueprints
//! (`effects/emitters/*.bp`, [`Emitter`]): a particle emitter, a trail of a ribbon, a beam.
//!
//! The scripts are programs; nothing is run. They are read as text for their classes' plain
//! fields (a list of emitters, a template's name, a number), each class through its bases as Lua
//! looks a field up. Nothing of them is kept: what the factions' mod makes of them is TA Reborn's
//! own effects (`factions::effects`).

use std::collections::BTreeMap;

use crate::behaviour::{block_end, is_name, tokens, Token};
use crate::files::Files;
use crate::lua::{self, Table, Value};
use crate::Result;

/// The files of the classes the units' weapons and their projectiles are made of, in the order a
/// name is looked for in them (the first file defining a class keeps it).
const CLASS_FILES: [&str; 14] = [
    "lua/terranweapons.lua",
    "lua/cybranweapons.lua",
    "lua/aeonweapons.lua",
    "lua/seraphimweapons.lua",
    "lua/sim/defaultweapons.lua",
    "lua/defaultcollisionbeams.lua",
    "lua/sim/collisionbeam.lua",
    "lua/terranprojectiles.lua",
    "lua/cybranprojectiles.lua",
    "lua/aeonprojectiles.lua",
    "lua/seraphimprojectiles.lua",
    "lua/defaultantiprojectile.lua",
    "lua/sim/defaultprojectiles.lua",
    "lua/sim/projectile.lua",
];

/// The templates' file, and the names its paths are written with (`EmtBpPath .. 'x_emit.bp'`).
const TEMPLATES: &str = "lua/effecttemplates.lua";
const PATH_PREFIXES: [(&str, &str); 2] = [("EmtBpPath", "/effects/emitters/"), ("EmitterTempEmtBpPath", "/effects/emitters/temp/")];

/// How deep templates and classes are followed (a loop in the game's files stops there).
const MAX_DEPTH: usize = 24;

/// A class of the game's Lua: what it is made of (`Class(A, B)`: their last names) and its plain
/// fields (`Name = value`, functions left out), each as its tokens.
#[derive(Clone, Debug, Default)]
struct Class {
    bases: Vec<String>,
    fields: BTreeMap<String, Vec<Token>>,
}

/// The game's effect templates and the classes of its weapons and projectiles.
#[derive(Clone, Debug, Default)]
pub struct Library {
    templates: BTreeMap<String, Vec<String>>,
    classes: BTreeMap<String, Class>,
    /// Local names standing for another file's class (`local TMissileCruiseProjectile =
    /// import('/lua/terranprojectiles.lua').TMissileCruiseProjectile02`).
    aliases: BTreeMap<String, String>,
}

/// What a unit's weapon shows of its own: at its muzzle, and, a beam weapon, its beam.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WeaponFx {
    /// Its script class's name (its base's, the class its unit's script makes it of).
    pub class: String,
    pub muzzle: Vec<String>,
    pub muzzle_scale: f64,
    pub beam: Option<BeamFx>,
}

/// A beam weapon's beam: the beam drawn (`FxBeam`: beam blueprints), and what shows where it
/// starts and where it ends.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BeamFx {
    pub class: String,
    pub beam: Vec<String>,
    pub start: Vec<String>,
    pub end: Vec<String>,
}

/// How a projectile looks in flight and where it lands.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShotFx {
    pub class: String,
    /// Emitters it carries along (`FxTrails`), at `trail_scale`.
    pub trails: Vec<String>,
    pub trail_scale: f64,
    /// Ribbons trailing behind it (`PolyTrail`, `PolyTrails`).
    pub ribbons: Vec<String>,
    /// Beams drawn on it (`BeamName`, `Beams`): a bolt's look, a missile's exhaust.
    pub beams: Vec<String>,
    /// Where it hits the ground (`FxImpactLand`), a unit (`FxImpactUnit`), the water
    /// (`FxImpactWater`) and an aircraft (`FxImpactAirUnit`), each at its scale.
    pub land: Vec<String>,
    pub unit: Vec<String>,
    pub water: Vec<String>,
    pub air: Vec<String>,
    pub land_scale: f64,
    pub unit_scale: f64,
    pub water_scale: f64,
}

/// An emitter blueprint: particles, a ribbon trailing behind what carries it, or a beam.
#[derive(Clone, Debug, PartialEq)]
pub enum Emitter {
    Particles(Particles),
    Ribbon(Ribbon),
    Beam(Beam),
}

/// One of a curve's keys: when (along its `XRange`), its value, and how far a particle's takes
/// it at random (its range, the value's either side by half).
pub type Key = [f64; 3];

/// A curve of an emitter (`EmitRateCurve`, `SizeCurve`...): its keys over `0..range`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Curve {
    pub range: f64,
    pub keys: Vec<Key>,
}

impl Curve {
    fn read(t: Option<&Table>) -> Curve {
        let Some(t) = t else { return Curve::default() };
        let range = t.get_ci("XRange").and_then(Value::num).unwrap_or(1.0);
        let keys = t
            .get_ci("Keys")
            .and_then(Value::table)
            .map(|k| {
                k.array
                    .iter()
                    .filter_map(|key| {
                        let key = key.table()?;
                        let n = |name: &str| key.get_ci(name).and_then(Value::num);
                        Some([n("x")?, n("y")?, n("z").unwrap_or(0.0)])
                    })
                    .collect()
            })
            .unwrap_or_default();
        Curve { range, keys }
    }

    /// Its value at `x` (between keys, along the straight line from one to the next; before the
    /// first and after the last, theirs) and its range there.
    pub fn at(&self, x: f64) -> (f64, f64) {
        let Some(first) = self.keys.first() else { return (0.0, 0.0) };
        if x <= first[0] {
            return (first[1], first[2]);
        }
        for w in self.keys.windows(2) {
            let (a, b) = (w[0], w[1]);
            if x <= b[0] {
                let span = b[0] - a[0];
                let k = if span > 0.0 { (x - a[0]) / span } else { 1.0 };
                return (a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k);
            }
        }
        let last = self.keys[self.keys.len() - 1];
        (last[1], last[2])
    }

    /// Its value and range on average over its whole range (sampled at 64 even steps: the same
    /// arithmetic on every computer), `floor` the least a value counts as (a rate's 0).
    pub fn mean(&self, floor: f64) -> (f64, f64) {
        if self.keys.is_empty() {
            return (0.0, 0.0);
        }
        const STEPS: u32 = 64;
        let range = if self.range > 0.0 { self.range } else { 1.0 };
        let (mut value, mut spread) = (0.0, 0.0);
        for i in 0..STEPS {
            let (v, s) = self.at(range * (i as f64 + 0.5) / STEPS as f64);
            value += v.max(floor);
            spread += s.max(0.0);
        }
        (value / STEPS as f64, spread / STEPS as f64)
    }

    /// Whether it has any keys.
    pub fn given(&self) -> bool {
        !self.keys.is_empty()
    }
}

/// A particle emitter (`EmitterBlueprint`): how long it emits (`lifetime`, ticks; less than none:
/// as long as what carries it lives), its curves over its life, its picture (`texture`, and the
/// `ramp` of colours its particles take as they age), how it blends (`blend`: 0 painted over, 3
/// added as light...), whether its particles lie flat (`flat`), and how it moves them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Particles {
    pub path: String,
    pub lifetime: f64,
    pub blend: i64,
    pub texture: String,
    pub ramp: String,
    pub flat: bool,
    pub gravity: bool,
    pub resistance: bool,
    pub high_fidelity: bool,
    pub curves: BTreeMap<String, Curve>,
}

impl Particles {
    /// Its curve `name` (`EmitRateCurve`...), or none.
    pub fn curve(&self, name: &str) -> &Curve {
        static NONE: Curve = Curve { range: 1.0, keys: Vec::new() };
        self.curves.get(name).unwrap_or(&NONE)
    }
}

/// A ribbon trailing behind what carries it (`TrailEmitterBlueprint`): how long a stretch of it
/// lasts (`length`, ticks), how wide it is (`size`, ogrids), its picture and ramp.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ribbon {
    pub path: String,
    pub length: f64,
    pub size: f64,
    pub texture: String,
    pub ramp: String,
}

/// A beam (`BeamBlueprint`): how thick (ogrids) and long (ogrids; behind what carries it where
/// less than none), its colours at its start and its end (RGBA, 0 to 1), its picture.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Beam {
    pub path: String,
    pub thickness: f64,
    pub length: f64,
    pub start: [f64; 4],
    pub end: [f64; 4],
    pub texture: String,
}

/// The index of the bracket closing the one at `open`, or the end.
fn matching(t: &[Token], open: usize) -> usize {
    let (o, c) = match t.get(open) {
        Some(Token::Mark('{')) => ('{', '}'),
        Some(Token::Mark('[')) => ('[', ']'),
        _ => ('(', ')'),
    };
    let mut depth = 0i32;
    for (k, tok) in t.iter().enumerate().skip(open) {
        if *tok == Token::Mark(o) {
            depth += 1;
        } else if *tok == Token::Mark(c) {
            depth -= 1;
            if depth == 0 {
                return k;
            }
        }
    }
    t.len()
}

/// Where the value starting at `from` ends (exclusive): at the first `,`, `;` or closing bracket
/// at its own depth, a function's `end`.
fn value_end(t: &[Token], from: usize, to: usize) -> usize {
    if t.get(from).is_some_and(|tok| is_name(tok, "function")) {
        return block_end(t, from).min(to);
    }
    let mut depth = 0i32;
    let mut k = from;
    while k < to {
        match &t[k] {
            Token::Mark('(') | Token::Mark('{') | Token::Mark('[') => depth += 1,
            Token::Mark(')') | Token::Mark('}') | Token::Mark(']') => {
                if depth == 0 {
                    return k;
                }
                depth -= 1;
            }
            Token::Mark(',') | Token::Mark(';') if depth == 0 => return k,
            Token::Name(n) if depth == 0 && n == "function" => return block_end(t, k).min(to),
            _ => {}
        }
        k += 1;
    }
    to
}

/// The plain fields of a table's body `t[from..to]` (`Name = value`), functions left out.
fn fields(t: &[Token], from: usize, to: usize) -> BTreeMap<String, Vec<Token>> {
    let mut out = BTreeMap::new();
    let mut k = from;
    while k < to {
        if let Token::Name(name) = &t[k]
            && t.get(k + 1) == Some(&Token::Mark('='))
            && t.get(k + 2) != Some(&Token::Mark('='))
        {
            let end = value_end(t, k + 2, to);
            if !t.get(k + 2).is_some_and(|tok| is_name(tok, "function")) {
                out.entry(name.clone()).or_insert_with(|| t[k + 2..end].to_vec());
            }
            k = end.max(k + 1);
        } else {
            k += 1;
        }
    }
    out
}

/// The last name of a `Class(...)`'s argument `t[from..to]` (`TDFGaussCannonWeapon`,
/// `import('/lua/aeonweapons.lua').ADFLaserLightWeapon`'s `ADFLaserLightWeapon`).
fn last_name(t: &[Token], from: usize, to: usize) -> Option<String> {
    t[from..to].iter().rev().find_map(|tok| match tok {
        Token::Name(n) if n != "import" => Some(n.clone()),
        _ => None,
    })
}

/// A `Class(bases) { body }` at `k` (its `Class` token): its bases, its fields and where it ends.
fn class_at(t: &[Token], k: usize) -> Option<(Class, usize)> {
    if !(t.get(k).is_some_and(|tok| is_name(tok, "Class")) && t.get(k + 1) == Some(&Token::Mark('('))) {
        return None;
    }
    let close = matching(t, k + 1);
    let mut bases = Vec::new();
    let (mut from, mut depth) = (k + 2, 0i32);
    for j in k + 2..=close.min(t.len().saturating_sub(1)) {
        match &t[j] {
            Token::Mark('(') | Token::Mark('{') | Token::Mark('[') => depth += 1,
            Token::Mark(')') | Token::Mark('}') | Token::Mark(']') if depth > 0 => depth -= 1,
            Token::Mark(',') if depth == 0 => {
                bases.extend(last_name(t, from, j));
                from = j + 1;
            }
            _ => {}
        }
        if j == close {
            bases.extend(last_name(t, from, j));
        }
    }
    let mut class = Class { bases, fields: BTreeMap::new() };
    let mut end = close + 1;
    if t.get(close + 1) == Some(&Token::Mark('{')) {
        let body = matching(t, close + 1);
        class.fields = fields(t, close + 2, body);
        end = body + 1;
    }
    Some((class, end))
}

/// Classes by name, in the order a file defines them.
type Classes = Vec<(String, Class)>;
/// Local names for other files' classes: the name, the class's.
type Aliases = Vec<(String, String)>;

/// The classes a file defines at its top (`Name = Class(...) { ... }`), and the local names it
/// gives other files' classes (`local A = import('...').B`, `local A = File.B`).
fn read_classes(text: &str) -> (Classes, Aliases) {
    let t = tokens(text);
    let (mut classes, mut aliases) = (Vec::new(), Vec::new());
    let mut k = 0;
    while k < t.len() {
        if let Token::Name(name) = &t[k]
            && t.get(k + 1) == Some(&Token::Mark('='))
        {
            if let Some((class, end)) = class_at(&t, k + 2) {
                classes.push((name.clone(), class));
                k = end;
                continue;
            }
            // (`import('...').B`, `File.B`: the last name of the chain of dots.)
            let mut j = k + 2;
            if t.get(j).is_some_and(|tok| is_name(tok, "import")) && t.get(j + 1) == Some(&Token::Mark('(')) {
                j = matching(&t, j + 1) + 1;
            } else if matches!(t.get(j), Some(Token::Name(_))) {
                j += 1;
            } else {
                k += 1;
                continue;
            }
            let mut member = None;
            while t.get(j) == Some(&Token::Mark('.')) {
                match t.get(j + 1) {
                    Some(Token::Name(n)) => {
                        member = Some(n.clone());
                        j += 2;
                    }
                    _ => break,
                }
            }
            if let Some(m) = member {
                aliases.push((name.clone(), m));
            }
        }
        k += 1;
    }
    (classes, aliases)
}

/// The templates of `lua/EffectTemplates.lua`: each name's emitters, in order, the templates a
/// template names (`TableCat(A, B)`, `C = D`, a list naming others) followed.
fn read_templates(text: &str) -> BTreeMap<String, Vec<String>> {
    let t = tokens(text);
    let mut raw: BTreeMap<String, Vec<Token>> = BTreeMap::new();
    let mut k = 0;
    let mut depth = 0i32;
    while k < t.len() {
        match &t[k] {
            Token::Mark('(') | Token::Mark('{') | Token::Mark('[') => depth += 1,
            Token::Mark(')') | Token::Mark('}') | Token::Mark(']') => depth -= 1,
            Token::Name(name) if depth == 0 && t.get(k + 1) == Some(&Token::Mark('=')) && t.get(k + 2) != Some(&Token::Mark('=')) => {
                let from = k + 2;
                let end = match t.get(from) {
                    Some(Token::Mark('{')) => matching(&t, from) + 1,
                    Some(Token::Name(n)) if t.get(from + 1) == Some(&Token::Mark('(')) && n != "import" => matching(&t, from + 1) + 1,
                    Some(Token::Name(_)) | Some(Token::Str(_)) => from + 1,
                    _ => {
                        k += 1;
                        continue;
                    }
                };
                raw.entry(name.clone()).or_insert_with(|| t[from..end.min(t.len())].to_vec());
                k = end;
                continue;
            }
            _ => {}
        }
        k += 1;
    }
    let mut done: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let names: Vec<String> = raw.keys().cloned().collect();
    for name in names {
        let list = expand(&raw, &name, 0, &mut done);
        done.insert(name, list);
    }
    done
}

/// A path written after a prefix's name at `k` (`EmtBpPath .. 'x_emit.bp'`): the prefix and the
/// path.
fn prefixed(t: &[Token], k: usize) -> Option<(&'static str, &str)> {
    let Token::Name(n) = t.get(k)? else { return None };
    let &(_, prefix) = PATH_PREFIXES.iter().find(|(p, _)| p == n)?;
    match (t.get(k + 1), t.get(k + 2), t.get(k + 3)) {
        (Some(Token::Mark('.')), Some(Token::Mark('.')), Some(Token::Str(s))) => Some((prefix, s.as_str())),
        _ => None,
    }
}

/// A template's emitters, the templates it names followed (`done`: those found already).
fn expand(raw: &BTreeMap<String, Vec<Token>>, name: &str, depth: usize, done: &mut BTreeMap<String, Vec<String>>) -> Vec<String> {
    if let Some(list) = done.get(name) {
        return list.clone();
    }
    let Some(toks) = raw.get(name) else { return Vec::new() };
    if depth > MAX_DEPTH {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut k = 0;
    while k < toks.len() {
        if let Some((prefix, path)) = prefixed(toks, k) {
            out.push(format!("{prefix}{path}"));
            k += 4;
            continue;
        }
        match &toks[k] {
            Token::Str(s) if s.to_ascii_lowercase().ends_with(".bp") => out.push(s.clone()),
            Token::Name(n) if raw.contains_key(n) && n != name => {
                let more = expand(raw, n, depth + 1, done);
                out.extend(more);
            }
            _ => {}
        }
        k += 1;
    }
    out
}

impl Library {
    /// The game's templates and classes, from its files (those missing: none of theirs).
    pub fn read(files: &Files) -> Result<Library> {
        let mut lib = Library::default();
        if let Some(bytes) = files.read(TEMPLATES)? {
            lib.templates = read_templates(&String::from_utf8_lossy(&bytes));
        }
        for path in CLASS_FILES {
            let Some(bytes) = files.read(path)? else { continue };
            let (classes, aliases) = read_classes(&String::from_utf8_lossy(&bytes));
            lib.add(classes, aliases);
        }
        Ok(lib)
    }

    fn add(&mut self, classes: Vec<(String, Class)>, aliases: Vec<(String, String)>) {
        for (name, class) in classes {
            self.classes.entry(name).or_insert(class);
        }
        for (local, member) in aliases {
            if local != member {
                self.aliases.entry(local).or_insert(member);
            }
        }
    }

    /// The class a name stands for (its alias followed).
    fn real<'a>(&'a self, name: &'a str, local: &'a BTreeMap<String, String>) -> &'a str {
        let name = local.get(name).map_or(name, String::as_str);
        self.aliases.get(name).map_or(name, String::as_str)
    }

    /// A class's field `name`, its own or its bases' (in their order, depth first, as Lua's
    /// classes look it up); `own` a class body of a script's own over it, `local` the script's own
    /// names for other files' classes.
    fn field(&self, class: &str, name: &str, own: Option<&BTreeMap<String, Vec<Token>>>, local: &BTreeMap<String, String>, depth: usize) -> Option<Vec<Token>> {
        if let Some(v) = own.and_then(|o| o.get(name)) {
            return Some(v.clone());
        }
        if depth > MAX_DEPTH {
            return None;
        }
        let class = self.real(class, local);
        let c = self.classes.get(class)?;
        if let Some(v) = c.fields.get(name) {
            return Some(v.clone());
        }
        c.bases.iter().find_map(|b| self.field(b, name, None, local, depth + 1))
    }

    /// The emitters a field's value names: a template (`EffectTemplate.X`), a list of paths and
    /// templates, a path; each path as the files are read (lower case, no leading `/`).
    fn emitters(&self, value: &[Token]) -> Vec<String> {
        let mut out = Vec::new();
        let mut k = 0;
        while k < value.len() {
            match &value[k] {
                Token::Name(n) if n == "EffectTemplate" && value.get(k + 1) == Some(&Token::Mark('.')) => {
                    if let Some(Token::Name(t)) = value.get(k + 2)
                        && let Some(list) = self.templates.get(t)
                    {
                        out.extend(list.iter().cloned());
                    }
                    k += 3;
                    continue;
                }
                Token::Str(s) if s.to_ascii_lowercase().ends_with(".bp") => out.push(s.clone()),
                _ => {
                    if let Some((prefix, path)) = prefixed(value, k) {
                        out.push(format!("{prefix}{path}"));
                        k += 4;
                        continue;
                    }
                }
            }
            k += 1;
        }
        out.into_iter().map(|p| crate::projectile::normalise(&p)).collect()
    }

    /// A number a field's value is (`FxMuzzleFlashScale = 0.5`).
    fn number(value: &[Token]) -> Option<f64> {
        match value {
            [Token::Num(n)] => n.parse().ok(),
            [Token::Mark('-'), Token::Num(n)] => n.parse::<f64>().ok().map(|v| -v),
            _ => None,
        }
    }

    /// A field's number, its own or its bases' (else `default`).
    fn scale(&self, class: &str, name: &str, own: Option<&BTreeMap<String, Vec<Token>>>, local: &BTreeMap<String, String>, default: f64) -> f64 {
        self.field(class, name, own, local, 0).and_then(|v| Self::number(&v)).filter(|v| v.is_finite() && *v > 0.0).unwrap_or(default)
    }

    /// What the weapons of a unit's script (`units/<id>/<id>_script.lua`) show of their own, by
    /// their labels (`Weapons = { MainGun = Class(TDFGaussCannonWeapon) { ... }, ... }`).
    pub fn weapons(&self, script: &str) -> BTreeMap<String, WeaponFx> {
        let (classes, aliases) = read_classes(script);
        let local: BTreeMap<String, String> = aliases.into_iter().collect();
        let mut out = BTreeMap::new();
        // (The unit's class: its `TypeClass`, else the last the file defines.)
        let t = tokens(script);
        let type_class = (0..t.len().saturating_sub(2)).find_map(|k| match (&t[k], &t[k + 1], &t[k + 2]) {
            (Token::Name(n), Token::Mark('='), Token::Name(c)) if n == "TypeClass" => Some(c.clone()),
            _ => None,
        });
        let Some((_, unit)) = classes.iter().find(|(n, _)| Some(n) == type_class.as_ref()).or(classes.last()) else { return out };
        let Some(table) = unit.fields.get("Weapons") else { return out };
        if table.first() != Some(&Token::Mark('{')) {
            return out;
        }
        let close = matching(table, 0);
        let mut k = 1;
        while k < close {
            if let Token::Name(label) = &table[k]
                && table.get(k + 1) == Some(&Token::Mark('='))
                && let Some((class, end)) = class_at(table, k + 2)
            {
                let base = class.bases.last().cloned().unwrap_or_default();
                out.insert(label.clone(), self.weapon(&base, &class.fields, &local));
                k = end;
                continue;
            }
            k += 1;
        }
        out
    }

    /// What a weapon of class `base` shows, `own` its unit's script's own fields over it.
    fn weapon(&self, base: &str, own: &BTreeMap<String, Vec<Token>>, local: &BTreeMap<String, String>) -> WeaponFx {
        let muzzle = self.field(base, "FxMuzzleFlash", Some(own), local, 0).map(|v| self.emitters(&v)).unwrap_or_default();
        let muzzle_scale = self.scale(base, "FxMuzzleFlashScale", Some(own), local, 1.0);
        let beam = self.field(base, "BeamType", Some(own), local, 0).and_then(|v| last_name(&v, 0, v.len())).map(|b| {
            let class = self.real(&b, local).to_string();
            let list = |name: &str| self.field(&class, name, None, local, 0).map(|v| self.emitters(&v)).unwrap_or_default();
            BeamFx { beam: list("FxBeam"), start: list("FxBeamStartPoint"), end: list("FxBeamEndPoint"), class: class.clone() }
        });
        WeaponFx { class: self.real(base, local).to_string(), muzzle, muzzle_scale, beam }
    }

    /// How the projectile of a script (`projectiles/<x>/<x>_script.lua`) looks: its class's
    /// trails, ribbons, beams and impacts, its own fields over its bases'.
    pub fn shot(&self, script: &str) -> ShotFx {
        let (classes, aliases) = read_classes(script);
        let local: BTreeMap<String, String> = aliases.into_iter().collect();
        let t = tokens(script);
        let type_class = (0..t.len().saturating_sub(2)).find_map(|k| match (&t[k], &t[k + 1], &t[k + 2]) {
            (Token::Name(n), Token::Mark('='), Token::Name(c)) if n == "TypeClass" => Some(c.clone()),
            _ => None,
        });
        let Some((name, class)) = classes.iter().find(|(n, _)| Some(n) == type_class.as_ref()).or(classes.last()) else { return ShotFx::default() };
        let base = class.bases.last().cloned().unwrap_or_default();
        let own = Some(&class.fields);
        let list = |field: &str| self.field(&base, field, own, &local, 0).map(|v| self.emitters(&v)).unwrap_or_default();
        let mut ribbons = list("PolyTrail");
        ribbons.extend(list("PolyTrails"));
        let mut beams = list("BeamName");
        beams.extend(list("Beams"));
        let scale = |field: &str| self.scale(&base, field, own, &local, 1.0);
        let _ = name;
        ShotFx {
            class: self.real(&base, &local).to_string(),
            trails: list("FxTrails"),
            trail_scale: scale("FxTrailScale"),
            ribbons,
            beams,
            land: list("FxImpactLand"),
            unit: list("FxImpactUnit"),
            water: list("FxImpactWater"),
            air: list("FxImpactAirUnit"),
            land_scale: scale("FxLandHitScale"),
            unit_scale: scale("FxUnitHitScale"),
            water_scale: scale("FxWaterHitScale"),
        }
    }
}

/// The emitter blueprint at `path` (as the files are read), if the game has it and it reads.
pub fn emitter(files: &Files, path: &str) -> Result<Option<Emitter>> {
    let Some(bytes) = files.read(path)? else { return Ok(None) };
    let text = String::from_utf8_lossy(&bytes);
    let Ok(statements) = lua::parse(&text) else {
        log::warn!("{path}: does not read");
        return Ok(None);
    };
    let Some((kind, value)) = statements.first() else { return Ok(None) };
    let Some(t) = value.table() else { return Ok(None) };
    let num = |key: &str, default: f64| t.get_ci(key).and_then(Value::num).unwrap_or(default);
    let text_of = |key: &str| t.get_ci(key).and_then(Value::str).map(crate::projectile::normalise).unwrap_or_default();
    let flag = |key: &str, default: bool| t.get_ci(key).and_then(Value::bool).unwrap_or(default);
    let colour = |key: &str| -> [f64; 4] {
        let Some(c) = t.get_ci(key).and_then(Value::table) else { return [1.0; 4] };
        let n = |k: &str, d: f64| c.get_ci(k).and_then(Value::num).unwrap_or(d);
        [n("x", 1.0), n("y", 1.0), n("z", 1.0), n("w", 1.0)]
    };
    let path = crate::projectile::normalise(path);
    Ok(match kind.as_str() {
        k if k.eq_ignore_ascii_case("EmitterBlueprint") => {
            let curves = t.fields.iter().filter(|(k, _)| k.ends_with("Curve")).map(|(k, v)| (k.clone(), Curve::read(v.table()))).collect();
            Some(Emitter::Particles(Particles {
                path,
                lifetime: num("Lifetime", 1.0),
                blend: num("Blendmode", 0.0) as i64,
                texture: text_of("Texture"),
                ramp: text_of("RampTexture"),
                flat: flag("Flat", false),
                gravity: flag("Gravity", false),
                resistance: flag("ParticleResistance", false),
                high_fidelity: flag("HighFidelity", true),
                curves,
            }))
        }
        k if k.eq_ignore_ascii_case("TrailEmitterBlueprint") => Some(Emitter::Ribbon(Ribbon { path, length: num("TrailLength", 1.0), size: num("Size", 0.05), texture: text_of("RepeatTexture"), ramp: text_of("RampTexture") })),
        k if k.eq_ignore_ascii_case("BeamBlueprint") => Some(Emitter::Beam(Beam { path, thickness: num("Thickness", 0.1), length: num("Length", 0.0), start: colour("StartColor"), end: colour("EndColor"), texture: text_of("TextureName") })),
        _ => None,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Templates in the game's own way of writing them (made up): paths after a prefix's name,
    /// lists joined, one named after another.
    pub(crate) const TEMPLATES_LUA: &str = r#"
TableCat = import('utilities.lua').TableCat
EmtBpPath = '/effects/emitters/'
TestFlash = { EmtBpPath .. 'test_flash_01_emit.bp', EmtBpPath .. 'test_sparks_01_emit.bp', }
TestSmoke = { '/effects/emitters/test_smoke_01_emit.bp' }
TestHit = TableCat( TestFlash, TestSmoke )
TestHitLand = TestHit
"#;

    /// Classes as the game's files make them (made up): a weapon with its muzzle flash, a beam
    /// weapon and its beam, projectiles with trails, ribbons and impacts, through their bases.
    pub(crate) const CLASSES_LUA: &str = r#"
local WeaponFile = import('/lua/sim/DefaultWeapons.lua')
local DefaultProjectileWeapon = WeaponFile.DefaultProjectileWeapon
local EffectTemplate = import('/lua/EffectTemplates.lua')
TestCannonWeapon = Class(DefaultProjectileWeapon) {
    FxMuzzleFlash = EffectTemplate.TestFlash,
    FxMuzzleFlashScale = 0.5,
    OnCreate = function(self)
        local a, b = 1, 2
        DefaultProjectileWeapon.OnCreate(self)
    end,
}
TestBeamWeapon = Class(DefaultBeamWeapon) {
    BeamType = CollisionBeamFile.TestCollisionBeam,
    FxMuzzleFlash = {},
}
TestCollisionBeam = Class(SCCollisionBeam) {
    FxBeam = { '/effects/emitters/test_beam_01_emit.bp' },
    FxBeamStartPoint = EffectTemplate.TestFlash,
    FxBeamEndPoint = EffectTemplate.TestHit,
}
TestEmitterProjectile = Class(Projectile) {
    FxTrails = { '/effects/emitters/test_trail_01_emit.bp', },
    FxTrailScale = 1,
}
TestShellProjectile = Class(TestEmitterProjectile) {
    PolyTrail = '/effects/emitters/test_polytrail_01_emit.bp',
    FxImpactLand = EffectTemplate.TestHitLand,
    FxImpactUnit = EffectTemplate.TestHit,
    FxImpactWater = EffectTemplate.TestSmoke,
}
"#;

    pub(crate) fn library() -> Library {
        let mut lib = Library { templates: read_templates(TEMPLATES_LUA), ..Default::default() };
        let (classes, aliases) = read_classes(CLASSES_LUA);
        lib.add(classes, aliases);
        lib
    }

    #[test]
    fn templates_follow_the_templates_they_name() {
        let t = read_templates(TEMPLATES_LUA);
        assert_eq!(t["TestFlash"], ["/effects/emitters/test_flash_01_emit.bp", "/effects/emitters/test_sparks_01_emit.bp"]);
        assert_eq!(t["TestHit"].len(), 3);
        assert_eq!(t["TestHitLand"], t["TestHit"]);
        assert!(!t.contains_key("TableCat") || t["TableCat"].is_empty());
    }

    /// A unit's weapons by their labels: the muzzle of its class (through its local name), its
    /// scale; a beam weapon's beam and its ends; a script's own field over its class's.
    #[test]
    fn a_units_weapons_show_their_classes_effects() {
        let lib = library();
        let script = r#"
local TestCannonWeapon = import('/lua/terranweapons.lua').TestCannonWeapon
local Laser = import('/lua/terranweapons.lua').TestBeamWeapon
TEST0201 = Class(TLandUnit) {
    Weapons = {
        MainGun = Class(TestCannonWeapon) {},
        Beam = Class(Laser) { FxMuzzleFlash = { '/effects/emitters/own_flash_emit.bp' }, },
    },
    OnCreate = function(self) local x, y = 1, 2 end,
}
TypeClass = TEST0201
"#;
        let w = lib.weapons(script);
        assert_eq!(w.keys().collect::<Vec<_>>(), ["Beam", "MainGun"]);
        let gun = &w["MainGun"];
        assert_eq!((gun.class.as_str(), gun.muzzle.len(), gun.muzzle_scale, gun.beam.is_none()), ("TestCannonWeapon", 2, 0.5, true));
        assert_eq!(gun.muzzle[0], "effects/emitters/test_flash_01_emit.bp", "as the files are read");
        let beam = &w["Beam"];
        assert_eq!(beam.muzzle, ["effects/emitters/own_flash_emit.bp"], "its script's own");
        let b = beam.beam.as_ref().expect("its beam");
        assert_eq!((b.class.as_str(), b.beam.len(), b.start.len(), b.end.len()), ("TestCollisionBeam", 1, 2, 3));
    }

    #[test]
    fn a_projectile_shows_its_classes_trails_and_impacts() {
        let lib = library();
        let shot = lib.shot("local Shell = import('/lua/terranprojectiles.lua').TestShellProjectile\nTEST01 = Class(Shell) { FxLandHitScale = 2 }\nTypeClass = TEST01\n");
        assert_eq!(shot.class, "TestShellProjectile");
        assert_eq!(shot.trails, ["effects/emitters/test_trail_01_emit.bp"], "its base's");
        assert_eq!(shot.ribbons, ["effects/emitters/test_polytrail_01_emit.bp"]);
        assert_eq!((shot.land.len(), shot.unit.len(), shot.water.len(), shot.air.len()), (3, 3, 1, 0));
        assert_eq!((shot.land_scale, shot.unit_scale), (2.0, 1.0), "its own scale");
    }

    /// A curve's value at a point and on average (its rate's below none counting as none).
    #[test]
    fn curves_are_read_between_their_keys() {
        let c = Curve { range: 10.0, keys: vec![[0.0, 10.0, 2.0], [4.0, 0.0, 0.0]] };
        assert_eq!(c.at(2.0), (5.0, 1.0));
        assert_eq!(c.at(8.0), (0.0, 0.0));
        let (mean, spread) = c.mean(0.0);
        assert!((mean - 2.0).abs() < 0.05 && (spread - 0.4).abs() < 0.05, "{mean} {spread}");
        let dip = Curve { range: 2.0, keys: vec![[0.0, -1.0, 0.0], [2.0, -1.0, 0.0]] };
        assert_eq!(dip.mean(0.0).0, 0.0);
    }
}
