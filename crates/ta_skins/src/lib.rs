//! Cosmetic unit skins. Author folders contain `skin.toml` (a livery, see [`Livery`], and
//! texture mappings) and the opaque PNGs it references; `.taskin` is a bounded, deterministic
//! container for those files, not a general archive. Nothing here mounts game data, runs
//! scripts, or modifies simulation definitions.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use image::{ImageFormat, ImageReader, RgbaImage};
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod livery;

pub use livery::{Colour, Livery, Pattern, MAX_COLOURS, SCALE};

/// Format 2 added `[livery]`; format 1's `base` (Reborn, Heritage or Christmas) is read as the
/// livery of that built-in.
pub const FORMAT: u32 = 2;
pub const EXTENSION: &str = "taskin";
pub const MAX_IMAGES: usize = 512;
pub const MAX_IMAGE_SIDE: u32 = 1024;
/// Includes the renderer's fallback layer. Mips cost a further one third on the GPU.
pub const MAX_BANK_TEXELS: u64 = 64 * 1024 * 1024;
pub const MAX_DECODED_BYTES: u64 = 256 * 1024 * 1024;
const MAGIC: &[u8; 8] = b"TASKIN01";
const MAX_MANIFEST: u64 = 1024 * 1024;
const MAX_PNG: u64 = 8 * 1024 * 1024;
const MAX_PACKAGE: u64 = 272 * 1024 * 1024;
const MAX_MAPPINGS: usize = 16_384;
const MANIFEST_FILE: &str = "skin.toml";

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Manifest {
    pub format: u32,
    pub id: String,
    pub name: String,
    pub author: String,
    pub description: String,
    /// Paint over every unit's eligible surfaces, under the texture mappings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub livery: Option<Livery>,
    pub textures: BTreeMap<String, String>,
    pub units: BTreeMap<String, UnitSkin>,
}

/// `skin.toml` as written, in either format.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    format: u32,
    id: String,
    name: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    base: Option<Base>,
    #[serde(default)]
    livery: Option<Livery>,
    #[serde(default)]
    textures: BTreeMap<String, String>,
    #[serde(default)]
    units: BTreeMap<String, UnitSkin>,
}

/// Format 1's base.
#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Base {
    Reborn,
    Christmas,
    Heritage,
}

/// Format 1's control image for its Heritage base, a texture key of no use since.
const HERITAGE_DETAIL: &str = "_heritage_detail";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UnitSkin {
    #[serde(default)]
    pub textures: BTreeMap<String, String>,
}

impl Manifest {
    /// Unit-specific paint wins over global paint. Wildcards are considered only by the
    /// renderer for eligible armour; functional/team/animated textures stay protected.
    pub fn texture_for(&self, unit: &str, texture: &str) -> Option<&str> {
        let unit = unit.to_ascii_lowercase();
        let texture = texture.to_ascii_lowercase();
        self.units.get(&unit)
            .and_then(|u| u.textures.get(&texture).or_else(|| u.textures.get("*")))
            .or_else(|| self.textures.get(&texture).or_else(|| self.textures.get("*")))
            .map(String::as_str)
    }

    pub fn image_names(&self) -> BTreeSet<&str> {
        self.textures.values().chain(self.units.values().flat_map(|u| u.textures.values()))
            .map(String::as_str).collect()
    }
}

#[derive(Clone, Debug)]
pub struct Pack {
    pub manifest: Manifest,
    pub images: BTreeMap<String, RgbaImage>,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub author: String,
    pub description: String,
    pub builtin: bool,
    pub path: Option<PathBuf>,
    pub livery: Option<Livery>,
    pub texture_count: usize,
    pub unit_count: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub entries: Vec<Entry>,
    pub errors: Vec<String>,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid skin pack: {0}")]
    Invalid(String),
    #[error("a skin with id '{0}' is already installed; choose another id in skin.toml")]
    AlreadyInstalled(String),
    #[error("destination already exists: {0}")]
    Exists(String),
    #[error("{path}: {source}")]
    Io { path: String, source: std::io::Error },
}

pub type Result<T> = std::result::Result<T, Error>;

fn invalid(message: impl Into<String>) -> Error { Error::Invalid(message.into()) }
fn io(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io { path: path.display().to_string(), source }
}

struct Builtin {
    id: &'static str,
    name: &'static str,
    description: &'static str,
}

/// Every built-in is a livery (Reborn none), so none carries an image. The ids of those before
/// liveries are kept, so a player's choice of one still holds.
const BUILTINS: [Builtin; 19] = [
    Builtin { id: "reborn", name: "Reborn", description: "TA Reborn's original unit materials and detail." },
    Builtin { id: "builtin:heritage", name: "Heritage", description: "TA's own colours, battle-worn: chipped edges and grime in every crease." },
    Builtin { id: "christmas", name: "Christmas", description: "Evergreen and festive red with snowflakes and gold trim." },
    Builtin { id: "builtin:halloween", name: "Halloween", description: "Witching purple and black splinters, trimmed in pumpkin orange." },
    Builtin { id: "builtin:desert", name: "Desert", description: "Sand, ochre and earth blotches for desert operations." },
    Builtin { id: "builtin:winter", name: "Winter", description: "Worn whitewash over grey for winter operations." },
    Builtin { id: "builtin:woodlands", name: "Woodlands", description: "Hard-edged green, brown and black for woodland operations." },
    Builtin { id: "builtin:navy", name: "Navy", description: "Splinters of haze and ocean grey, as a warship's dazzle paint." },
    Builtin { id: "builtin:urban", name: "Urban", description: "Pixelated greys for fighting among the buildings." },
    Builtin { id: "builtin:midnight", name: "Midnight", description: "Dark gunmetal hex plating that disappears at night." },
    Builtin { id: "builtin:tiger", name: "Tiger", description: "Burnt orange slashed with black stripes." },
    Builtin { id: "builtin:ceremonial", name: "Ceremonial", description: "Parade white with polished gold edges." },
    Builtin { id: "builtin:uk", name: "United Kingdom", description: "Navy blue crossed with white and red, as the Union flag." },
    Builtin { id: "builtin:usa", name: "United States", description: "Stars and stripes in red, white and blue." },
    Builtin { id: "builtin:australia", name: "Australia", description: "Green and gold, with stars of the Southern Cross." },
    Builtin { id: "builtin:new-zealand", name: "New Zealand", description: "All black, trimmed in silver." },
    Builtin { id: "builtin:italy", name: "Italy", description: "Il Tricolore: bands of green, white and red." },
    Builtin { id: "builtin:france", name: "France", description: "Le Tricolore: bands of blue, white and red." },
    Builtin { id: "builtin:germany", name: "Germany", description: "Bands of black, red and gold." },
];

impl Builtin {
    fn manifest(&self) -> Manifest {
        Manifest { format: FORMAT, id: self.id.into(), name: self.name.into(), author: "TA Reborn".into(), description: self.description.into(),
            livery: livery::builtin(self.id), textures: BTreeMap::new(), units: BTreeMap::new() }
    }

    fn pack(&self) -> Pack {
        Pack { manifest: self.manifest(), images: BTreeMap::new() }
    }
}

fn builtin(id: &str) -> Option<&'static Builtin> { BUILTINS.iter().find(|b| b.id == id) }

/// The ids of the built-in skins, in the order the library lists them.
pub fn builtin_ids() -> impl Iterator<Item = &'static str> { BUILTINS.iter().map(|b| b.id) }

/// A built-in's livery (none for Reborn, or for an id that is not a built-in).
pub fn builtin_livery(id: &str) -> Option<Livery> { builtin(id).and_then(|b| livery::builtin(b.id)) }

fn entry(manifest: &Manifest, path: Option<PathBuf>) -> Entry {
    Entry { id: manifest.id.clone(), name: manifest.name.clone(), author: manifest.author.clone(), description: manifest.description.clone(), builtin: path.is_none(), path,
        livery: manifest.livery.clone(), texture_count: manifest.image_names().len(), unit_count: manifest.units.len() }
}

/// A lightweight catalog: validates manifests, but decodes images only when a pack is
/// loaded/imported/exported. Invalid folders are reported without hiding valid entries.
pub fn list(root: &Path) -> Catalog {
    // The catalog reads metadata only: opening the library never renders/decodes paint.
    let mut out = Catalog { entries: BUILTINS.iter().map(|b| entry(&b.manifest(), None)).collect(), errors: Vec::new() };
    if !root.exists() { return out; }
    if let Err(e) = ordinary_dir(root) { out.errors.push(e.to_string()); return out; }
    let read = match std::fs::read_dir(root) { Ok(r) => r, Err(e) => { out.errors.push(io(root)(e).to_string()); return out; } };
    let mut paths = Vec::new();
    for child in read {
        match child {
            Ok(c) => {
                if c.file_name().to_string_lossy().starts_with('.') { continue; }
                if paths.len() >= 256 { out.errors.push("Only the first 256 skin folders are listed.".into()); break; }
                paths.push(c.path());
            }
            Err(e) => out.errors.push(io(root)(e).to_string()),
        }
    }
    paths.sort();
    for path in paths {
        let result = (|| {
            ordinary_dir(&path)?;
            let manifest = read_manifest(&path)?;
            if path.file_name().and_then(|n| n.to_str()) != Some(&manifest.id) {
                return Err(invalid("folder name must match the skin id"));
            }
            Ok(entry(&manifest, Some(path.clone())))
        })();
        match result { Ok(e) => out.entries.push(e), Err(e) => out.errors.push(format!("{}: {e}", path.display())) }
    }
    out
}

/// Loads a built-in or installed skin. No gameplay VFS files are involved.
pub fn load(id: &str, root: &Path) -> Result<Pack> {
    if let Some(builtin) = builtin(id) { return Ok(builtin.pack()); }
    validate_id(id)?;
    ordinary_dir(root)?;
    let pack = load_folder(&root.join(id))?;
    if pack.manifest.id != id { return Err(invalid("folder name must match the skin id")); }
    Ok(pack)
}

/// Loads an authoring folder, including bounded image decoding. Unreferenced files are
/// deliberately ignored and can never become part of an exported share file.
pub fn load_folder(folder: &Path) -> Result<Pack> {
    ordinary_dir(folder)?;
    let manifest = read_manifest(folder)?;
    let mut encoded = BTreeMap::new();
    let mut total = 0u64;
    for name in manifest.image_names() {
        let path = child_file(folder, name)?;
        let data = read_limited(&path, MAX_PNG)?;
        total = total.saturating_add(data.len() as u64);
        if total > MAX_PACKAGE - MAX_MANIFEST { return Err(invalid("PNG files exceed the package byte budget")); }
        encoded.insert(name.to_string(), data);
    }
    decode_images(manifest, encoded)
}

fn read_manifest(folder: &Path) -> Result<Manifest> {
    let path = child_file(folder, MANIFEST_FILE)?;
    Ok(parse_manifest(&read_limited(&path, MAX_MANIFEST)?)?.0)
}

/// The manifest, and the PNGs only format 1's Heritage control image referenced (a share file
/// of that format still carries them; they are left out).
fn parse_manifest(bytes: &[u8]) -> Result<(Manifest, BTreeSet<String>)> {
    if bytes.len() as u64 > MAX_MANIFEST { return Err(invalid("skin.toml exceeds 1 MiB")); }
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("skin.toml must be UTF-8"))?;
    let stored: Stored = toml::from_str(text).map_err(|e| invalid(format!("skin.toml: {e}")))?;
    let (mut manifest, dropped) = upgrade(stored)?;
    validate_manifest(&mut manifest)?;
    let kept = manifest.image_names();
    let dropped = dropped.into_iter().filter(|path| !kept.contains(path.as_str())).collect();
    Ok((manifest, dropped))
}

/// A manifest of either format in this one's terms.
fn upgrade(stored: Stored) -> Result<(Manifest, Vec<String>)> {
    let Stored { format, id, name, author, description, base, livery, mut textures, units } = stored;
    let mut dropped = Vec::new();
    let livery = match format {
        1 => {
            if livery.is_some() { return Err(invalid("[livery] needs format = 2")); }
            if let Some(key) = textures.keys().find(|key| key.eq_ignore_ascii_case(HERITAGE_DETAIL)).cloned() {
                dropped.extend(textures.remove(&key));
            }
            match base.unwrap_or(Base::Reborn) {
                Base::Reborn => None,
                Base::Christmas => livery::builtin("christmas"),
                Base::Heritage => livery::builtin("builtin:heritage"),
            }
        }
        FORMAT => {
            if base.is_some() { return Err(invalid("format 2 replaces base with a [livery] table")); }
            livery
        }
        other => return Err(invalid(format!("unsupported format {other}; expected {FORMAT}"))),
    };
    Ok((Manifest { format: FORMAT, id, name, author, description, livery, textures, units }, dropped))
}

fn validate_manifest(manifest: &mut Manifest) -> Result<()> {
    if manifest.format != FORMAT { return Err(invalid(format!("unsupported format {}; expected {FORMAT}", manifest.format))); }
    validate_id(&manifest.id)?;
    if let Some(livery) = &manifest.livery { livery.check().map_err(invalid)?; }
    if builtin(&manifest.id).is_some() { return Err(invalid(format!("{} is a reserved built-in skin id", manifest.id))); }
    text_field(&manifest.name, "name", 80, false, false)?;
    text_field(&manifest.author, "author", 120, true, false)?;
    text_field(&manifest.description, "description", 2000, true, true)?;
    if manifest.units.len() > 4096 { return Err(invalid("more than 4096 unit overrides")); }
    normalize_textures(&mut manifest.textures)?;
    let mut units = BTreeMap::new();
    let mut mappings = manifest.textures.len();
    for (unit, mut skin) in std::mem::take(&mut manifest.units) {
        if !valid_key(&unit, false) { return Err(invalid(format!("invalid unit name '{unit}'"))); }
        normalize_textures(&mut skin.textures)?;
        mappings = mappings.saturating_add(skin.textures.len());
        if mappings > MAX_MAPPINGS { return Err(invalid("more than 16384 texture mappings")); }
        if units.insert(unit.to_ascii_lowercase(), skin).is_some() { return Err(invalid(format!("duplicate unit name ignoring case: {unit}"))); }
    }
    manifest.units = units;
    let names = manifest.image_names();
    if names.len() > MAX_IMAGES { return Err(invalid("more than 512 PNG images")); }
    let mut insensitive = BTreeSet::new();
    for name in names {
        if !safe_image_path(name) { return Err(invalid(format!("unsafe PNG path '{name}'"))); }
        if !insensitive.insert(name.to_ascii_lowercase()) { return Err(invalid("PNG paths must not differ only by case")); }
    }
    for name in &insensitive {
        for (index, _) in name.match_indices('/') {
            if insensitive.contains(&name[..index]) { return Err(invalid("a PNG path cannot also be another image's parent directory")); }
        }
    }
    Ok(())
}

fn normalize_textures(textures: &mut BTreeMap<String, String>) -> Result<()> {
    if textures.len() > MAX_MAPPINGS { return Err(invalid("more than 16384 texture mappings")); }
    let mut normalized = BTreeMap::new();
    for (name, path) in std::mem::take(textures) {
        if !valid_key(&name, true) { return Err(invalid(format!("invalid texture name '{name}'"))); }
        if normalized.insert(name.to_ascii_lowercase(), path).is_some() { return Err(invalid(format!("duplicate texture name ignoring case: {name}"))); }
    }
    *textures = normalized;
    Ok(())
}

fn valid_key(key: &str, wildcard: bool) -> bool {
    wildcard && key == "*" || !key.is_empty() && key.len() <= 96
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-. ".contains(&b))
        && key.trim() == key && key != "." && key != ".."
}

fn text_field(text: &str, label: &str, max: usize, empty: bool, multiline: bool) -> Result<()> {
    if (!empty && text.trim().is_empty()) || text.chars().count() > max || text.chars().any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\t'))) {
        return Err(invalid(format!("{label} is empty, too long or contains control characters")));
    }
    Ok(())
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.as_bytes()[0].is_ascii_alphanumeric()
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && safe_component(id)
}

fn validate_id(id: &str) -> Result<()> {
    if valid_id(id) { Ok(()) } else { Err(invalid("skin id must be 1–64 lowercase letters, digits or hyphens, beginning with a letter or digit")) }
}

fn safe_component(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." || name.starts_with('.') || name.ends_with(['.', ' '])
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b)) { return false; }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$")
        && !stem.strip_prefix("COM").or_else(|| stem.strip_prefix("LPT")).is_some_and(|n| n.len() == 1 && n.as_bytes()[0].is_ascii_digit())
}

fn safe_image_path(name: &str) -> bool {
    let first = name.split('/').next().unwrap_or("");
    name.len() <= 180 && name.ends_with(".png") && name.split('/').count() <= 8 && name.split('/').all(safe_component)
        && ![MANIFEST_FILE, "CREATOR-GUIDE.md", "UNIT-TEXTURES.toml"].iter().any(|reserved| first.eq_ignore_ascii_case(reserved))
}

fn decode_images(manifest: Manifest, encoded: BTreeMap<String, Vec<u8>>) -> Result<Pack> {
    let expected = manifest.image_names();
    if encoded.len() != expected.len() || encoded.keys().any(|k| !expected.contains(k.as_str())) {
        return Err(invalid("package PNGs do not exactly match skin.toml references"));
    }
    let mut images = BTreeMap::new();
    let mut decoded = 0u64;
    let mut largest = 0u32;
    for (name, bytes) in encoded {
        if bytes.len() as u64 > MAX_PNG { return Err(invalid(format!("{name} exceeds 8 MiB"))); }
        // Check dimensions before allocating an image, and cap metadata/decompression too.
        let dimensions = png_reader(Cursor::new(&bytes)).into_dimensions()
            .map_err(|e| invalid(format!("{name}: {e}")))?;
        let (width, height) = dimensions;
        if width != height || !(64..=MAX_IMAGE_SIDE).contains(&width) || !width.is_power_of_two() {
            return Err(invalid(format!("{name} must be a square power-of-two PNG from 64 to 1024 pixels")));
        }
        largest = largest.max(width);
        decoded += u64::from(width) * u64::from(height) * 4;
        if decoded > MAX_DECODED_BYTES || (expected.len() as u64 + 1) * u64::from(largest).pow(2) > MAX_BANK_TEXELS {
            return Err(invalid("skin exceeds the decoded image or GPU texture budget; reduce image count or resolution"));
        }
        let image = png_reader(Cursor::new(bytes)).decode().map_err(|e| invalid(format!("{name}: {e}")))?.into_rgba8();
        if image.pixels().any(|p| p[3] != 255) { return Err(invalid(format!("{name} contains transparency; unit skins must be fully opaque"))); }
        images.insert(name, image);
    }
    Ok(Pack { manifest, images })
}

fn png_reader<R: std::io::BufRead + std::io::Seek>(reader: R) -> ImageReader<R> {
    let mut reader = ImageReader::with_format(reader, ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    reader
}

/// Reads and validates a share file in memory. Every count/length is checked before an
/// allocation, no file paths are extracted, and no trailing/unreferenced payload is accepted.
pub fn read_file(file: &Path) -> Result<Pack> {
    let bytes = read_limited(file, MAX_PACKAGE)?;
    read_bytes(&bytes)
}

fn read_bytes(bytes: &[u8]) -> Result<Pack> {
    if bytes.len() as u64 > MAX_PACKAGE { return Err(invalid("package exceeds 272 MiB")); }
    let mut data = bytes;
    if take(&mut data, 8)? != MAGIC { return Err(invalid("not a version 1 .taskin file")); }
    let length = number(&mut data)? as usize;
    if length as u64 > MAX_MANIFEST { return Err(invalid("skin.toml exceeds 1 MiB")); }
    let (manifest, dropped) = parse_manifest(take(&mut data, length)?)?;
    let count = number(&mut data)? as usize;
    if count > MAX_IMAGES + 1 || count != manifest.image_names().len() + dropped.len() { return Err(invalid("invalid PNG count")); }
    let mut encoded = BTreeMap::new();
    let mut insensitive = BTreeSet::new();
    for _ in 0..count {
        let name_len = number(&mut data)? as usize;
        if name_len > 180 { return Err(invalid("PNG path is too long")); }
        let name = std::str::from_utf8(take(&mut data, name_len)?).map_err(|_| invalid("PNG path must be UTF-8"))?;
        if !safe_image_path(name) || !insensitive.insert(name.to_ascii_lowercase()) { return Err(invalid("unsafe or duplicate PNG path")); }
        let length = number(&mut data)? as usize;
        if length as u64 > MAX_PNG { return Err(invalid("PNG exceeds 8 MiB")); }
        let bytes = take(&mut data, length)?;
        if !dropped.contains(name) { encoded.insert(name.to_owned(), bytes.to_vec()); }
    }
    if !data.is_empty() { return Err(invalid("unexpected trailing package content")); }
    decode_images(manifest, encoded)
}

fn take<'a>(data: &mut &'a [u8], count: usize) -> Result<&'a [u8]> {
    if count > data.len() { return Err(invalid("truncated package")); }
    let (head, tail) = data.split_at(count);
    *data = tail;
    Ok(head)
}
fn number(data: &mut &[u8]) -> Result<u32> {
    Ok(u32::from_le_bytes(take(data, 4)?.try_into().unwrap()))
}

/// Installs a validated share file without replacing any installed pack.
pub fn import_file(file: &Path, root: &Path) -> Result<Entry> { install(&read_file(file)?, root) }

/// Exports only manifest + referenced paint. Built-ins become editable copies with a
/// non-reserved id, and never include any art from the player's original game install.
pub fn export(id: &str, root: &Path, to: &Path) -> Result<()> {
    write_new(to, &share_bytes(id, root)?)
}

/// [`export`] over a file already at `to`, one a save dialog asked the player to replace:
/// written beside it and moved over it (a link there is replaced, never written through).
pub fn export_replacing(id: &str, root: &Path, to: &Path) -> Result<()> {
    replace_file(to, &share_bytes(id, root)?)
}

/// The share file of the skin `id`.
fn share_bytes(id: &str, root: &Path) -> Result<Vec<u8>> {
    let mut pack = load(id, root)?;
    if builtin(id).is_some() {
        // The built-in-only namespace cannot collide with any author folder. Exports
        // leave that namespace before manifest validation or filesystem operations.
        let name = id.strip_prefix("builtin:").unwrap_or(id);
        pack.manifest.id = format!("{name}-copy");
        pack.manifest.name.push_str(" copy");
    }
    encode(&pack)
}

/// Creates an editable folder with a livery and an original neutral panel texture. The example
/// maps the Commander's armour; add global mappings to paint all eligible unit surfaces.
pub fn create(root: &Path, name: &str, author: &str, livery: Option<Livery>) -> Result<Entry> {
    let id = id_for(name);
    let mut manifest = Manifest { format: FORMAT, id, name: name.trim().into(), author: author.trim().into(), description: "An editable unit skin. Paint textures/paint.png, then map unit textures in skin.toml.".into(), livery, textures: BTreeMap::new(), units: BTreeMap::new() };
    manifest.units.insert("armcom".into(), UnitSkin { textures: BTreeMap::from([("*".into(), "textures/paint.png".into())]) });
    validate_manifest(&mut manifest)?;
    let image = RgbaImage::from_fn(256, 256, |x, y| {
        let seam = x < 4 || y < 4 || x >= 252 || y >= 252;
        let groove = x % 64 < 2 || y % 64 < 2;
        let grain = ((x.wrapping_mul(37) ^ y.wrapping_mul(91)) % 7) as u8;
        let v = if seam { 47 } else if groove { 86 } else { 132 + grain };
        image::Rgba([v, v, v, 255])
    });
    install(&Pack { manifest, images: BTreeMap::from([("textures/paint.png".into(), image)]) }, root)
}

/// The id a skin named `name` is installed under: its letters and digits in lower case, other
/// runs as hyphens (`-custom` after a built-in's).
pub fn id_for(name: &str) -> String {
    let mut id = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() { id.push(c.to_ascii_lowercase()); }
        else if !id.ends_with('-') { id.push('-'); }
    }
    id = id.trim_matches('-').chars().take(64).collect::<String>().trim_end_matches('-').into();
    if id.is_empty() {
        // Names in scripts without ASCII letters are still distinct. Use an explicitly
        // stable UTF-8 hash, rather than a platform/process-random DefaultHasher; the same
        // visible name keeps the same id and ordinary duplicate protection still applies.
        let hash = name.trim().as_bytes().iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3));
        id = format!("custom-{hash:016x}");
    }
    if builtin(&id).is_some() { id.push_str("-custom"); }
    id
}

/// An id for a new skin named `name` that no skin in `root` has yet (`-2`, `-3`... after a
/// taken one).
pub fn free_id(root: &Path, name: &str) -> String {
    let id = id_for(name);
    let taken = |id: &str| std::fs::symlink_metadata(root.join(id)).is_ok();
    if !taken(&id) { return id; }
    (2..).map(|n| {
        let suffix = format!("-{n}");
        format!("{}{suffix}", id[..id.len().min(64 - suffix.len())].trim_end_matches('-'))
    }).find(|id| !taken(id)).expect("some suffix is free")
}

/// Installs `pack` as a new skin in `root` (its id must be free), checked as an import is.
pub fn add(pack: &Pack, root: &Path) -> Result<Entry> { install(pack, root) }

/// Saves an installed skin as edited: `manifest` over its `skin.toml`, and `added` (the images
/// it maps that are not in its folder yet) beside the others. Nothing else in the folder
/// changes, an artist's working files included. The manifest goes in last, and the skin is read
/// back before it stands: should it not load, the manifest before is put back and the added
/// images taken away again.
pub fn update(root: &Path, manifest: &Manifest, added: &BTreeMap<String, RgbaImage>) -> Result<Entry> {
    let mut manifest = manifest.clone();
    validate_manifest(&mut manifest)?;
    ordinary_dir(root)?;
    let folder = root.join(&manifest.id);
    ordinary_dir(&folder)?;
    let before = read_limited(&child_file(&folder, MANIFEST_FILE)?, MAX_MANIFEST)?;
    let names = manifest.image_names();
    let mut written: Vec<PathBuf> = Vec::new();
    let result = (|| {
        for (name, image) in added.iter().filter(|(name, _)| names.contains(name.as_str())) {
            if !safe_image_path(name) { return Err(invalid(format!("unsafe PNG path '{name}'"))); }
            let path = folder.join(name);
            if let Some(parent) = path.parent() {
                if !parent.exists() { std::fs::create_dir_all(parent).map_err(io(parent))?; }
                ordinary_dir(parent)?;
            }
            let (width, height) = image.dimensions();
            if width != height || !(64..=MAX_IMAGE_SIDE).contains(&width) || !width.is_power_of_two() {
                return Err(invalid(format!("{name} must be a square power-of-two PNG from 64 to 1024 pixels")));
            }
            if image.pixels().any(|p| p[3] != 255) { return Err(invalid(format!("{name} contains transparency; unit skins must be fully opaque"))); }
            write_new(&path, &png_bytes(image)?)?;
            written.push(path);
        }
        replace_file(&folder.join(MANIFEST_FILE), &with_header(manifest_bytes(&manifest)?))?;
        let pack = load_folder(&folder)?;
        if pack.manifest.id != manifest.id { return Err(invalid("folder name must match the skin id")); }
        Ok(entry(&pack.manifest, Some(folder.clone())))
    })();
    if result.is_err() {
        let _ = replace_file(&folder.join(MANIFEST_FILE), &before);
        for path in written { let _ = std::fs::remove_file(path); }
    }
    result
}

/// `image` made fit to be a skin's: square, a power of two from 64 to 1024 pixels a side (the
/// nearest to its longer side, stretched to it), and opaque (over its own average colour).
/// Also whether it had to change.
pub fn fit_image(image: &image::DynamicImage) -> (RgbaImage, bool) {
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let longer = width.max(height).clamp(64, MAX_IMAGE_SIDE);
    // (The power of two nearest the longer side, by ratio: 600 becomes 512, 800 becomes 1024.)
    let lower = 1u32 << longer.ilog2();
    let side = if lower < MAX_IMAGE_SIDE && longer as f32 / lower as f32 > (2.0f32).sqrt() { lower * 2 } else { lower };
    let resized = width != side || height != side;
    let mut fitted = if resized { image::imageops::resize(&rgba, side, side, image::imageops::FilterType::Lanczos3) } else { rgba };
    let transparent = fitted.pixels().any(|p| p[3] != 255);
    if transparent {
        let (mut sum, mut weight) = ([0.0f64; 3], 0.0f64);
        for p in fitted.pixels() {
            let a = f64::from(p[3]) / 255.0;
            for c in 0..3 { sum[c] += f64::from(p[c]) * a; }
            weight += a;
        }
        let under = sum.map(|v| if weight > 0.0 { v / weight } else { 128.0 });
        for p in fitted.pixels_mut() {
            let a = f64::from(p[3]) / 255.0;
            for c in 0..3 { p[c] = (f64::from(p[c]) * a + under[c] * (1.0 - a)).round() as u8; }
            p[3] = 255;
        }
    }
    (fitted, resized || transparent)
}

/// The note at the top of an installed `skin.toml`, when it fits.
fn with_header(text: Vec<u8>) -> Vec<u8> {
    let header = b"# TA Reborn unit skin. [livery] paints every unit; texture mappings paint over it.\n# PNGs must be opaque, square, power-of-two 64..1024.\n# Texture/unit names are case-insensitive; '*' paints eligible armour only.\n# Add [textures] with '*' = 'textures/paint.png' for global armour paint.\n# Team colours, animated textures and lights are protected by the renderer.\n\n";
    // The installed file obeys the same limit as the incoming one. Optional author
    // comments must not turn a valid near-limit manifest into an unreadable skin.
    let mut data = if header.len() as u64 + text.len() as u64 <= MAX_MANIFEST { header.to_vec() } else { Vec::new() };
    data.extend(text);
    data
}

/// Writes `bytes` beside `path` and moves them over it, so a reader sees the old file or the
/// new one, never half of one.
fn replace_file(path: &Path, bytes: &[u8]) -> Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let partial = path.with_extension(format!("part-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    write_new(&partial, bytes)?;
    std::fs::rename(&partial, path).map_err(|e| {
        let _ = std::fs::remove_file(&partial);
        io(path)(e)
    })
}

fn manifest_bytes(manifest: &Manifest) -> Result<Vec<u8>> {
    let mut manifest = manifest.clone();
    validate_manifest(&mut manifest)?;
    let text = toml::to_string_pretty(&manifest).map_err(|e| invalid(e.to_string()))?;
    if text.len() as u64 > MAX_MANIFEST { return Err(invalid("skin.toml exceeds 1 MiB")); }
    Ok(text.into_bytes())
}

fn png_bytes(image: &RgbaImage) -> Result<Vec<u8>> {
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, ImageFormat::Png).map_err(|e| invalid(e.to_string()))?;
    let bytes = out.into_inner();
    if bytes.len() as u64 > MAX_PNG { return Err(invalid("encoded PNG exceeds 8 MiB")); }
    Ok(bytes)
}

fn encode(pack: &Pack) -> Result<Vec<u8>> {
    let manifest = manifest_bytes(&pack.manifest)?;
    let names = pack.manifest.image_names();
    if pack.images.len() != names.len() || pack.images.keys().any(|n| !names.contains(n.as_str())) {
        return Err(invalid("package PNGs do not exactly match skin.toml references"));
    }
    let mut decoded = 0u64;
    let mut largest = 0u32;
    for image in pack.images.values() {
        let (width, height) = image.dimensions();
        if width != height || !(64..=MAX_IMAGE_SIDE).contains(&width) || !width.is_power_of_two() {
            return Err(invalid("images must be square power-of-two PNGs from 64 to 1024 pixels"));
        }
        if image.pixels().any(|p| p[3] != 255) { return Err(invalid("unit skins must be fully opaque")); }
        largest = largest.max(width);
        decoded += u64::from(width).pow(2) * 4;
    }
    if decoded > MAX_DECODED_BYTES || (pack.images.len() as u64 + 1) * u64::from(largest).pow(2) > MAX_BANK_TEXELS {
        return Err(invalid("skin exceeds the decoded image or GPU texture budget"));
    }
    let mut out = Vec::new();
    out.extend(MAGIC);
    out.extend((manifest.len() as u32).to_le_bytes());
    out.extend(manifest);
    out.extend((pack.images.len() as u32).to_le_bytes());
    for (name, image) in &pack.images {
        let bytes = png_bytes(image)?;
        out.extend((name.len() as u32).to_le_bytes());
        out.extend(name.as_bytes());
        out.extend((bytes.len() as u32).to_le_bytes());
        out.extend(bytes);
        if out.len() as u64 > MAX_PACKAGE { return Err(invalid("package exceeds 272 MiB")); }
    }
    Ok(out)
}

fn install(pack: &Pack, root: &Path) -> Result<Entry> {
    let bytes = encode(pack)?;
    let verified = read_bytes(&bytes)?;
    let manifest = &verified.manifest;
    ensure_root(root)?;
    let target = root.join(&manifest.id);
    if std::fs::symlink_metadata(&target).is_ok() { return Err(Error::AlreadyInstalled(manifest.id.clone())); }
    let staging = Temp::directory(root)?;
    let result = (|| {
        write_new(&staging.path.join(MANIFEST_FILE), &with_header(manifest_bytes(manifest)?))?;
        write_new(&staging.path.join("CREATOR-GUIDE.md"), include_bytes!("../../../docs/UNIT_SKINS.md"))?;
        for (name, image) in &verified.images {
            let path = staging.path.join(name);
            if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(io(parent))?; }
            write_new(&path, &png_bytes(image)?)?;
        }
        // rename never merges folders. On Unix an empty directory could be replaced, so
        // reserve the destination first; then move only into our own reserved directory.
        std::fs::create_dir(&target).map_err(|e| if e.kind() == std::io::ErrorKind::AlreadyExists { Error::AlreadyInstalled(manifest.id.clone()) } else { io(&target)(e) })?;
        let mut reserved = Temp { path: target.clone(), directory: true, keep: false };
        for child in std::fs::read_dir(&staging.path).map_err(io(&staging.path))? {
            let child = child.map_err(io(&staging.path))?;
            if child.file_name() == MANIFEST_FILE { continue; }
            std::fs::rename(child.path(), target.join(child.file_name())).map_err(io(&target))?;
        }
        // Publishing the manifest last keeps readers from seeing an incomplete pack.
        std::fs::rename(staging.path.join(MANIFEST_FILE), target.join(MANIFEST_FILE)).map_err(io(&target))?;
        reserved.keep = true;
        Ok(entry(manifest, Some(target)))
    })();
    result
}

fn ensure_root(root: &Path) -> Result<()> {
    if !root.exists() { std::fs::create_dir_all(root).map_err(io(root))?; }
    ordinary_dir(root)
}

fn is_link(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)] { use std::os::windows::fs::MetadataExt; meta.file_attributes() & 0x400 != 0 }
    #[cfg(not(windows))] { meta.file_type().is_symlink() }
}

fn ordinary_dir(path: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(path).map_err(io(path))?;
    if is_link(&meta) || !meta.is_dir() { return Err(invalid(format!("{} is not an ordinary directory", path.display()))); }
    Ok(())
}

fn child_file(root: &Path, name: &str) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    let pieces: Vec<_> = name.split('/').collect();
    for (index, part) in pieces.iter().enumerate() {
        if !safe_component(part) { return Err(invalid("unsafe file path")); }
        path.push(part);
        let meta = std::fs::symlink_metadata(&path).map_err(io(&path))?;
        if is_link(&meta) || if index + 1 == pieces.len() { !meta.is_file() } else { !meta.is_dir() } {
            return Err(invalid(format!("{} is linked or not an ordinary file/directory", path.display())));
        }
    }
    Ok(path)
}

fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let meta = std::fs::symlink_metadata(path).map_err(io(path))?;
    if is_link(&meta) || !meta.is_file() || meta.len() > limit { return Err(invalid(format!("{} is linked, not a file or exceeds its byte limit", path.display()))); }
    let mut bytes = Vec::new();
    std::fs::File::open(path).map_err(io(path))?.take(limit + 1).read_to_end(&mut bytes).map_err(io(path))?;
    if bytes.len() as u64 > limit { return Err(invalid("file grew beyond its byte limit while reading")); }
    Ok(bytes)
}

/// Reserve the final name with create_new, never overwrite/truncate another file (or a
/// symlink/hardlink). On write failure the incomplete file is removed by its owner.
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path)
        .map_err(|e| if e.kind() == std::io::ErrorKind::AlreadyExists { Error::Exists(path.display().to_string()) } else { io(path)(e) })?;
    let mut owned = Temp { path: path.to_path_buf(), directory: false, keep: false };
    file.write_all(bytes).map_err(io(path))?;
    file.sync_all().map_err(io(path))?;
    drop(file);
    owned.keep = true;
    Ok(())
}

struct Temp { path: PathBuf, directory: bool, keep: bool }
impl Temp {
    fn directory(root: &Path) -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..100 {
            let path = root.join(format!(".skin-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path, directory: true, keep: false }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(io(&path)(e)),
            }
        }
        Err(invalid("unable to reserve a skin staging directory"))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        if self.keep { return; }
        if self.directory { let _ = std::fs::remove_dir_all(&self.path); }
        else { let _ = std::fs::remove_file(&self.path); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox() -> Temp { Temp::directory(&std::env::temp_dir()).unwrap() }
    fn manifest() -> Manifest {
        Manifest { format: FORMAT, id: "test-paint".into(), name: "Test paint".into(), author: "Artist".into(), description: "Original paint".into(), livery: None, textures: BTreeMap::new(), units: BTreeMap::new() }
    }
    fn solid(side: u32) -> RgbaImage { RgbaImage::from_pixel(side, side, image::Rgba([74, 100, 132, 255])) }
    fn one_image() -> Pack {
        let mut manifest = manifest();
        manifest.textures.insert("*".into(), "paint.png".into());
        Pack { manifest, images: BTreeMap::from([("paint.png".into(), solid(64))]) }
    }
    fn raw(manifest: &Manifest, payloads: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
        let text = toml::to_string(manifest).unwrap();
        let mut out = MAGIC.to_vec();
        out.extend((text.len() as u32).to_le_bytes());
        out.extend(text.bytes());
        out.extend((payloads.len() as u32).to_le_bytes());
        for (name, data) in payloads {
            out.extend((name.len() as u32).to_le_bytes());
            out.extend(name.bytes());
            out.extend((data.len() as u32).to_le_bytes());
            out.extend(data);
        }
        out
    }

    #[test]
    fn authoring_export_import_is_deterministic_and_never_overwrites() {
        let tmp = sandbox();
        let root = tmp.path.join("skins");
        let created = create(&root, "My Paint", "An artist", builtin_livery("christmas")).unwrap();
        assert_eq!(created.id, "my-paint");
        assert_eq!(created.texture_count, 1);
        let folder = created.path.unwrap();
        assert!(folder.join("CREATOR-GUIDE.md").is_file());
        // Authoring notes/tools are permitted in a folder, but cannot leak into a share.
        std::fs::write(folder.join("private.txt"), "not part of a share").unwrap();
        std::fs::write(folder.join("script.exe"), "not executable content").unwrap();
        let first = tmp.path.join("first.taskin");
        let second = tmp.path.join("second.taskin");
        export("my-paint", &root, &first).unwrap();
        export("my-paint", &root, &second).unwrap();
        let bytes = std::fs::read(&first).unwrap();
        assert_eq!(bytes, std::fs::read(&second).unwrap());
        let dest = tmp.path.join("imported");
        let imported = import_file(&first, &dest).unwrap();
        let pack = load(&imported.id, &dest).unwrap();
        assert_eq!(pack.manifest.livery, builtin_livery("christmas"));
        assert_eq!(pack.images.len(), 1);
        assert!(dest.join("my-paint/CREATOR-GUIDE.md").is_file());
        assert!(!bytes.windows(b"CREATOR-GUIDE.md".len()).any(|part| part == b"CREATOR-GUIDE.md"));
        assert!(!dest.join("my-paint/private.txt").exists());
        assert!(!dest.join("my-paint/script.exe").exists());
        assert_eq!(pack.manifest.texture_for("ARMCOM", "ArmMetal"), Some("textures/paint.png"));
        assert!(matches!(import_file(&first, &dest), Err(Error::AlreadyInstalled(_))));
        assert!(matches!(export("my-paint", &root, &first), Err(Error::Exists(_))));
        assert_eq!(bytes, std::fs::read(first).unwrap());
        assert_eq!(list(&dest).entries.len(), BUILTINS.len() + 1);
    }

    #[test]
    fn builtins_export_as_editable_copies_without_original_game_art() {
        let tmp = sandbox();
        let root = tmp.path.join("skins");
        for builtin in &BUILTINS {
            let name = builtin.id.strip_prefix("builtin:").unwrap_or(builtin.id);
            let file = tmp.path.join(format!("{name}.taskin"));
            export(builtin.id, &root, &file).unwrap();
            let copy = import_file(&file, &root).unwrap();
            assert_eq!(copy.id, format!("{name}-copy"));
            let pack = load(&copy.id, &root).unwrap();
            assert_eq!(pack.manifest.livery, builtin.manifest().livery);
            assert_eq!(pack.manifest.livery.is_none(), builtin.id == "reborn");
            assert!(pack.images.is_empty() && pack.manifest.textures.is_empty());
            assert!(validate_manifest(&mut builtin.manifest()).is_err());
        }
        assert_eq!(list(&root).entries.iter().filter(|e| e.builtin).count(), BUILTINS.len());
        assert_eq!(list(&root).entries.len(), BUILTINS.len() * 2);
        assert!(list(&root).errors.is_empty());
        let created = create(&root, "Christmas", "Artist", None).unwrap();
        assert_eq!(created.id, "christmas-custom");
    }

    #[test]
    fn builtin_namespace_preserves_existing_custom_theme_ids() {
        let tmp = sandbox();
        let root = tmp.path.join("skins");
        for name in ["Desert", "Winter", "Woodlands", "Navy", "Heritage", "Urban", "Midnight", "Tiger", "Ceremonial"] {
            // These names were valid before the built-ins shipped and must keep loading
            // the user's own art, not silently select a new built-in pack after updating.
            let created = create(&root, name, "Custom artist", builtin_livery("christmas")).unwrap();
            assert_eq!(created.id, name.to_ascii_lowercase());
            let custom = load(&created.id, &root).unwrap();
            assert_eq!(custom.manifest.author, "Custom artist");
            assert_eq!(custom.manifest.livery, builtin_livery("christmas"));
            let builtin_id = format!("builtin:{}", created.id);
            let builtin = load(&builtin_id, &root).unwrap();
            assert_eq!(builtin.manifest.author, "TA Reborn");
            assert_eq!(builtin.manifest.livery, builtin_livery(&builtin_id));
            assert!(builtin.manifest.livery.is_some());
            assert!(!valid_id(&builtin_id), "built-in namespaces cannot become author folders");
            let catalog = list(&root);
            assert!(catalog.errors.is_empty());
            assert!(catalog.entries.iter().any(|e| e.id == created.id && !e.builtin));
            assert!(catalog.entries.iter().any(|e| e.id == builtin_id && e.builtin));
        }
    }

    #[test]
    fn distinct_unicode_creator_names_keep_distinct_safe_ids() {
        let tmp = sandbox();
        let root = tmp.path.join("skins");
        let winter = create(&root, "冬の装甲", "作者", None).unwrap();
        let summer = create(&root, "夏の装甲", "作者", None).unwrap();
        assert!(valid_id(&winter.id) && valid_id(&summer.id));
        assert_ne!(winter.id, summer.id);
        assert_eq!(load(&winter.id, &root).unwrap().manifest.name, "冬の装甲");
        assert!(matches!(create(&root, "冬の装甲", "作者", None), Err(Error::AlreadyInstalled(id)) if id == winter.id));
    }

    #[test]
    fn near_limit_manifest_is_still_readable_after_installing() {
        let tmp = sandbox();
        let mut m = manifest();
        m.description.clear();
        let empty_len = manifest_bytes(&m).unwrap().len();
        let key = |n| format!("surface-{n:05}{}", "x".repeat(80));
        m.textures.insert(key(0), "paint.png".into());
        let one_len = manifest_bytes(&m).unwrap().len();
        m.textures.insert(key(1), "paint.png".into());
        let each_len = manifest_bytes(&m).unwrap().len() - one_len;
        let section_len = one_len - empty_len - each_len;
        let count = (MAX_MANIFEST as usize - empty_len - section_len - 1000) / each_len;
        for index in 2..count { m.textures.insert(key(index), "paint.png".into()); }
        let pad = MAX_MANIFEST as usize - manifest_bytes(&m).unwrap().len() - 16;
        assert!(pad <= 2000);
        m.description = "x".repeat(pad);
        assert_eq!(manifest_bytes(&m).unwrap().len(), MAX_MANIFEST as usize - 16);
        let entry = install(&Pack { manifest: m, images: BTreeMap::from([("paint.png".into(), solid(64))]) }, &tmp.path).unwrap();
        let loaded = load(&entry.id, &tmp.path).unwrap();
        assert_eq!(loaded.manifest.textures.len(), count);
        assert_eq!(list(&tmp.path).errors.len(), 0);
    }

    #[test]
    fn unsafe_ids_paths_unknown_fields_and_case_aliases_are_rejected() {
        for id in ["", "../outside", "C:\\target", "CON", "con", "lpt1", "nul", "a.b", "-name", "a "] { assert!(!valid_id(id), "{id}"); }
        for path in ["../out.png", "/out.png", "C:/out.png", "a\\out.png", "a//out.png", "nul.png", "foo/COM1.png", "foo/x.png:stream", "foo./x.png", ".hidden.png", "x.exe", "skin.toml/x.png", "creator-guide.md/x.png"] {
            let mut m = manifest();
            m.textures.insert("*".into(), path.into());
            assert!(validate_manifest(&mut m).is_err(), "{path}");
        }
        let mut m = manifest();
        m.textures = BTreeMap::from([("ARMpaint".into(), "paint.png".into()), ("armpaint".into(), "paint.png".into())]);
        assert!(validate_manifest(&mut m).is_err());
        let mut m = manifest();
        m.units = BTreeMap::from([("ARMCOM".into(), UnitSkin::default()), ("armcom".into(), UnitSkin::default())]);
        assert!(validate_manifest(&mut m).is_err());
        let text = "format=1\nid='custom'\nname='Custom'\nscript='run.exe'";
        assert!(parse_manifest(text.as_bytes()).is_err());
        let mut m = manifest();
        m.textures = BTreeMap::from([("a".into(), "Paint.png".into()), ("b".into(), "paint.png".into())]);
        assert!(validate_manifest(&mut m).is_err());
        let mut m = manifest();
        m.textures = BTreeMap::from([("a".into(), "paint.png".into()), ("b".into(), "paint.png/detail.png".into())]);
        assert!(validate_manifest(&mut m).is_err());
    }

    #[test]
    fn every_truncated_header_and_payload_fails_without_installing() {
        let pack = one_image();
        let bytes = encode(&pack).unwrap();
        for length in 0..bytes.len() {
            assert!(read_bytes(&bytes[..length]).is_err(), "accepted truncation at {length}");
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(read_bytes(&trailing).is_err());
        let mut huge_manifest = MAGIC.to_vec();
        huge_manifest.extend(u32::MAX.to_le_bytes());
        assert!(read_bytes(&huge_manifest).is_err());
        let tmp = sandbox();
        let file = tmp.path.join("broken.taskin");
        std::fs::write(&file, trailing).unwrap();
        let root = tmp.path.join("not-created");
        assert!(import_file(&file, &root).is_err());
        assert!(!root.exists());
    }

    #[test]
    fn opacity_dimensions_and_scaled_gpu_budget_are_enforced() {
        let pack = one_image();
        for side in [1, 63, 65, 2048] {
            let payload = BTreeMap::from([("paint.png".into(), png_bytes(&solid(side)).unwrap())]);
            assert!(read_bytes(&raw(&pack.manifest, &payload)).is_err(), "{side}");
        }
        let mut transparent = solid(64);
        transparent.get_pixel_mut(5, 7)[3] = 254;
        let payload = BTreeMap::from([("paint.png".into(), png_bytes(&transparent).unwrap())]);
        assert!(read_bytes(&raw(&pack.manifest, &payload)).is_err());
        let mut m = manifest();
        let mut payloads = BTreeMap::new();
        // Small decoded files can still require a huge array when one layer is large.
        // 64 images plus the fallback at 1024² exceed the 64M-texel budget.
        for index in 0..64 {
            let path = format!("paint{index}.png");
            m.textures.insert(format!("surface{index}"), path.clone());
            payloads.insert(path, png_bytes(&solid(if index == 0 { 1024 } else { 64 })).unwrap());
        }
        assert!(read_bytes(&raw(&m, &payloads)).unwrap_err().to_string().contains("GPU texture budget"));
        m.textures.remove("surface63");
        payloads.remove("paint63.png");
        assert_eq!(read_bytes(&raw(&m, &payloads)).unwrap().images.len(), 63);
    }

    #[test]
    fn extra_images_and_mismatched_folder_ids_are_rejected() {
        let pack = one_image();
        let mut payloads = BTreeMap::from([("paint.png".into(), png_bytes(&solid(64)).unwrap())]);
        payloads.insert("secret.png".into(), png_bytes(&solid(64)).unwrap());
        assert!(read_bytes(&raw(&pack.manifest, &payloads)).is_err());
        let tmp = sandbox();
        let root = tmp.path.join("skins");
        let e = create(&root, "Paint", "Artist", None).unwrap();
        std::fs::rename(e.path.unwrap(), root.join("different-id")).unwrap();
        assert!(load("different-id", &root).is_err());
        let catalog = list(&root);
        assert_eq!(catalog.entries.len(), BUILTINS.len());
        assert_eq!(catalog.errors.len(), 1);
    }

    #[test]
    fn links_in_authoring_folders_and_existing_destinations_cannot_redirect_writes() {
        let tmp = sandbox();
        let root = tmp.path.join("skins");
        let e = create(&root, "Paint", "Artist", None).unwrap();
        let outside = tmp.path.join("outside.png");
        std::fs::write(&outside, png_bytes(&solid(64)).unwrap()).unwrap();
        let original = std::fs::read(&outside).unwrap();
        let paint = e.path.unwrap().join("textures/paint.png");
        std::fs::remove_file(&paint).unwrap();
        #[cfg(unix)] let result = std::os::unix::fs::symlink(&outside, &paint);
        #[cfg(windows)] let result = std::os::windows::fs::symlink_file(&outside, &paint);
        if result.is_ok() {
            assert!(load("paint", &root).is_err());
            assert!(export("reborn", &root, &paint).is_err());
            assert_eq!(std::fs::read(&outside).unwrap(), original);
        } else {
            eprintln!("symlink creation unavailable without platform permission");
        }
        let hardlink = tmp.path.join("share.taskin");
        std::fs::hard_link(&outside, &hardlink).unwrap();
        assert!(export("reborn", &root, &hardlink).is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), original);
        // Replacing (the save dialog asked): the link is replaced, the file it led to is not
        // written through.
        export_replacing("reborn", &root, &hardlink).unwrap();
        assert_eq!(std::fs::read(&outside).unwrap(), original);
        assert!(import_file(&hardlink, &tmp.path.join("imported")).is_ok(), "a whole share file in its place");
    }

    /// Format 1 packs keep loading: their base becomes that built-in's livery, and Heritage's
    /// control image, of no use now, is left out of a folder and a share file alike.
    #[test]
    fn format_one_packs_load_as_liveries() {
        let read = |text: &str| parse_manifest(text.as_bytes());
        let (reborn, dropped) = read("format = 1\nid = 'old'\nname = 'Old'\n").unwrap();
        assert_eq!((reborn.format, reborn.livery, dropped.len()), (FORMAT, None, 0));
        let (christmas, _) = read("format = 1\nid = 'old'\nname = 'Old'\nbase = 'christmas'\n[textures]\n'*' = 'paint.png'\n").unwrap();
        assert_eq!(christmas.livery, builtin_livery("christmas"));
        assert_eq!(christmas.texture_for("armcom", "armmetal"), Some("paint.png"));
        let heritage = "format = 1\nid = 'old'\nname = 'Old'\nbase = 'heritage'\n[textures]\n'_Heritage_Detail' = 'detail.png'\n";
        let (manifest, dropped) = read(heritage).unwrap();
        assert_eq!(manifest.livery, builtin_livery("builtin:heritage"));
        assert!(manifest.textures.is_empty());
        assert_eq!(dropped, BTreeSet::from(["detail.png".to_string()]));
        // (A control image also mapped as paint stays.)
        let (both, dropped) = read(&format!("{heritage}armmetal = 'detail.png'\n")).unwrap();
        assert!(dropped.is_empty() && both.texture_for("armcom", "armmetal") == Some("detail.png"));
        let payloads = BTreeMap::from([("detail.png".to_string(), png_bytes(&solid(64)).unwrap())]);
        let mut share = MAGIC.to_vec();
        share.extend((heritage.len() as u32).to_le_bytes());
        share.extend(heritage.bytes());
        share.extend((payloads.len() as u32).to_le_bytes());
        for (name, data) in &payloads {
            share.extend((name.len() as u32).to_le_bytes());
            share.extend(name.bytes());
            share.extend((data.len() as u32).to_le_bytes());
            share.extend(data);
        }
        let pack = read_bytes(&share).unwrap();
        assert!(pack.images.is_empty());
        assert_eq!(pack.manifest.livery, builtin_livery("builtin:heritage"));
        // Each format keeps to its own words.
        assert!(read("format = 1\nid = 'old'\nname = 'Old'\n[livery]\ncolours = ['#ffffff']\n").is_err());
        assert!(read("format = 2\nid = 'new'\nname = 'New'\nbase = 'christmas'\n").is_err());
        assert!(read("format = 3\nid = 'new'\nname = 'New'\n").is_err());
    }

    #[test]
    fn a_livery_is_checked_and_written_back_as_format_two() {
        let tmp = sandbox();
        let root = tmp.path.join("skins");
        let text = "format = 2\nid = 'camo'\nname = 'Camo'\n[livery]\ncolours = ['#4f5d34', '#5f4a32']\npattern = 'blotch'\nscale = 1.5\nwear = 0.8\n";
        let (manifest, _) = parse_manifest(text.as_bytes()).unwrap();
        let livery = manifest.livery.clone().unwrap();
        assert_eq!((livery.pattern, livery.scale, livery.wear, livery.coverage), (Pattern::Blotch, 1.5, 0.8, 1.0));
        let entry = install(&Pack { manifest, images: BTreeMap::new() }, &root).unwrap();
        assert_eq!(entry.livery.as_ref(), Some(&livery));
        let written = std::fs::read_to_string(root.join("camo/skin.toml")).unwrap();
        assert!(written.contains("format = 2") && written.contains("[livery]") && written.contains("pattern = \"blotch\""), "{written}");
        assert_eq!(load("camo", &root).unwrap().manifest.livery, Some(livery));
        assert_eq!(list(&root).entries.iter().find(|e| e.id == "camo").unwrap().livery.as_ref().unwrap().scale, 1.5);
        for bad in ["colours = []", "colours = ['#ffffff']\nscale = 9.0", "colours = ['#ffffff']\ngloss = 2.0", "colours = ['#ffffff']\nshine = 1.0"] {
            let text = format!("format = 2\nid = 'bad'\nname = 'Bad'\n[livery]\n{bad}\n");
            assert!(parse_manifest(text.as_bytes()).is_err(), "{bad}");
        }
    }

    #[test]
    fn every_builtin_is_a_livery_and_reborn_none() {
        assert_eq!(builtin_ids().count(), 19);
        for id in builtin_ids() {
            let pack = load(id, Path::new("unused")).unwrap();
            assert!(pack.images.is_empty(), "{id} needs no images");
            assert_eq!(pack.manifest.livery.is_none(), id == "reborn", "{id}");
            assert_eq!(builtin_livery(id), pack.manifest.livery);
        }
        assert_eq!(builtin_livery("my-skin"), None);
    }

    /// Saving an edited skin changes its manifest and adds its new images, leaving every other
    /// file in its folder; a save that would not load leaves the skin as it was.
    #[test]
    fn an_edited_skin_saves_in_place_and_a_bad_save_changes_nothing() {
        let tmp = sandbox();
        let root = tmp.path.join("skins");
        let created = create(&root, "Paint", "Artist", None).unwrap();
        let folder = created.path.unwrap();
        std::fs::write(folder.join("working.psd"), "the artist's own file").unwrap();
        let mut manifest = load("paint", &root).unwrap().manifest;
        manifest.livery = builtin_livery("builtin:tiger");
        manifest.textures.insert("armmetal".into(), "textures/armmetal.png".into());
        let added = BTreeMap::from([("textures/armmetal.png".to_string(), solid(128))]);
        let entry = update(&root, &manifest, &added).unwrap();
        assert_eq!(entry.livery, builtin_livery("builtin:tiger"));
        let pack = load("paint", &root).unwrap();
        assert_eq!(pack.manifest.texture_for("corak", "ArmMetal"), Some("textures/armmetal.png"));
        assert_eq!(pack.images["textures/armmetal.png"], solid(128));
        assert!(folder.join("working.psd").is_file() && folder.join("CREATOR-GUIDE.md").is_file());
        assert!(std::fs::read_to_string(folder.join(MANIFEST_FILE)).unwrap().starts_with("# TA Reborn unit skin."));
        // (A mapping to an image that is nowhere: refused, and nothing changes.)
        let before = std::fs::read(folder.join(MANIFEST_FILE)).unwrap();
        let mut broken = manifest.clone();
        broken.textures.insert("cormetal".into(), "textures/missing.png".into());
        let added = BTreeMap::from([("textures/extra.png".to_string(), solid(64))]);
        broken.textures.insert("corplate".into(), "textures/extra.png".into());
        assert!(update(&root, &broken, &added).is_err());
        assert_eq!(std::fs::read(folder.join(MANIFEST_FILE)).unwrap(), before);
        assert!(!folder.join("textures/extra.png").exists(), "the images a failed save added go again");
        assert!(load("paint", &root).is_ok());
        // An image of the folder is never written over, nor a skin that is not installed saved.
        let added = BTreeMap::from([("textures/armmetal.png".to_string(), solid(64))]);
        assert!(matches!(update(&root, &manifest, &added), Err(Error::Exists(_))));
        let mut other = manifest.clone();
        other.id = "not-installed".into();
        assert!(update(&root, &other, &BTreeMap::new()).is_err());
        assert_eq!(list(&root).entries.iter().filter(|e| !e.builtin).count(), 1);
    }

    #[test]
    fn new_skins_take_a_free_id_from_their_name() {
        let tmp = sandbox();
        let root = tmp.path.join("skins");
        assert_eq!(free_id(&root, "Night Camo"), "night-camo");
        let mut manifest = manifest();
        manifest.id = free_id(&root, "Night Camo");
        manifest.livery = builtin_livery("builtin:midnight");
        add(&Pack { manifest: manifest.clone(), images: BTreeMap::new() }, &root).unwrap();
        assert_eq!(free_id(&root, "Night Camo"), "night-camo-2");
        assert_eq!(free_id(&root, "Desert"), "desert", "a built-in's name is free: built-ins are builtin:");
        assert_eq!(id_for("Reborn"), "reborn-custom");
        assert!(matches!(add(&Pack { manifest, images: BTreeMap::new() }, &root), Err(Error::AlreadyInstalled(_))));
        let long = "x".repeat(80);
        assert!(valid_id(&free_id(&root, &long)));
    }

    #[test]
    fn any_picture_is_fitted_to_a_skin_image() {
        let picture = image::DynamicImage::ImageRgba8(RgbaImage::from_fn(300, 200, |x, _| image::Rgba([200, 100, 50, if x < 150 { 255 } else { 0 }])));
        let (fitted, changed) = fit_image(&picture);
        assert!(changed);
        assert_eq!(fitted.dimensions(), (256, 256));
        assert!(fitted.pixels().all(|p| p[3] == 255), "made opaque");
        assert!(fitted.pixels().all(|p| p[0] == 200 && p[1] == 100 && p[2] == 50), "the transparent part takes the picture's own colour");
        for (side, expect) in [(40, 64), (600, 512), (800, 1024), (3000, 1024)] {
            let (fitted, _) = fit_image(&image::DynamicImage::ImageRgba8(solid(side)));
            assert_eq!(fitted.width(), expect, "{side}");
        }
        let (same, changed) = fit_image(&image::DynamicImage::ImageRgba8(solid(512)));
        assert!(!changed && same == solid(512));
    }

    #[test]
    fn mapping_precedence_is_case_insensitive_and_unit_specific() {
        let mut m = manifest();
        m.textures = BTreeMap::from([("*".into(), "global.png".into()), ("ARMmetal".into(), "global-specific.png".into())]);
        m.units.insert("ARMCOM".into(), UnitSkin { textures: BTreeMap::from([("*".into(), "commander.png".into()), ("ARMmetal".into(), "commander-specific.png".into())]) });
        validate_manifest(&mut m).unwrap();
        assert_eq!(m.texture_for("ArmCom", "ArmMetal"), Some("commander-specific.png"));
        assert_eq!(m.texture_for("ArmCom", "other"), Some("commander.png"));
        assert_eq!(m.texture_for("ArmPeep", "ArmMetal"), Some("global-specific.png"));
        assert_eq!(m.texture_for("ArmPeep", "other"), Some("global.png"));
    }
}
