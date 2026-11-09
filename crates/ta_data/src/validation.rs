//! Bounded, offline compatibility checks over the same winning files the game reads.
//!
//! This is a static content check, never a gameplay or graphics-driver certification.
//! Files are decoded in parallel a chunk at a time and summarised (names and references
//! kept, never pixels or bytecode), then folded in path order, so a report is the same on
//! any number of cores. Cancelled and resource-limited reports are explicitly incomplete.
//! The caller owns mounting, content fingerprints, persistence, the comparison with the
//! same content without the mod ([`compare`]) and any human play-test evidence.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use ta_formats::{cob::{self, CobScript}, gaf::{self, AnimSize, GafReader}, model3do::Model3do, tdf::{self, TdfDocument, TdfSection}};
use ta_vfs::Vfs;

/// Bumped whenever the checks change what they report, so cached reports are made again (5:
/// WAV music not misplaced, a replaced definition's findings not the replacing mod's, a folder of
/// unreadable archives an error; 6: the model textures' room counts only those the models paint
/// with, in the game's two arrays; 7: empty files are blanks, a note a folder, and a mod's copy of
/// an original file that does not read is not the mod's; 8: GAFs read a frame at a time, a
/// weapon's explosion too large to decode whole drawn reduced, the explosions' room as drawn;
/// 9: a script's code decoded as the VM runs it, so what a compiler left between functions is
/// no unknown opcode; 10: a unit's script is its own, `scripts/<UnitName>.cob`, before its
/// model's).
pub const SCHEMA_VERSION: u32 = 10;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    #[default]
    Warning,
    Info,
}

/// Whose doing a finding is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// In the mod's own files, or in content they use.
    #[default]
    Mod,
    /// In inherited content, and only with the mod: it changes something that content relies
    /// on (see [`compare`]).
    Caused,
    /// In inherited content with or without the mod: the original game's, Reborn's or a
    /// dependency's.
    Inherited,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Issue {
    pub severity: Severity,
    pub code: String,
    pub path: String,
    pub layer: String,
    /// The archive the file came from (its file name), when it came from one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// The line of `path` it is on, where known (text files).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    pub message: String,
    pub suggestion: String,
    /// Whether it counts towards the badge: the mod's own and what it causes, not what is
    /// there without it. (Inherited findings stay in the shareable report.)
    #[serde(default = "yes")]
    pub affects_status: bool,
    #[serde(default)]
    pub origin: Origin,
}

const fn yes() -> bool { true }

impl Default for Issue {
    fn default() -> Self {
        Self { severity: Severity::Warning, code: String::new(), path: String::new(), layer: String::new(), source: String::new(), line: None, message: String::new(), suggestion: String::new(), affects_status: true, origin: Origin::Mod }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Clean,
    Issues,
    /// Errors in the mod, or caused by it.
    Errors,
    Incomplete,
    /// The mod holds nothing these checks read (music, documents, pictures).
    Empty,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Counts {
    pub files_total: u64,
    pub files_checked: u64,
    pub files_failed: u64,
    pub units: u64,
    pub weapons: u64,
    pub features: u64,
    pub maps: u64,
    pub models: u64,
    pub scripts: u64,
    pub gafs: u64,
    pub pcx: u64,
    pub textures: u64,
    pub frames: u64,
    #[serde(default)]
    pub sounds: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub schema_version: u32,
    pub target_layer: Option<String>,
    pub complete: bool,
    /// The target held nothing these checks read.
    #[serde(default)]
    pub empty: bool,
    pub counts: Counts,
    pub issues: Vec<Issue>,
}

impl Default for Report {
    fn default() -> Self {
        Self { schema_version: SCHEMA_VERSION, target_layer: None, complete: false, empty: false, counts: Counts::default(), issues: Vec::new() }
    }
}

impl Report {
    pub fn status(&self) -> Status {
        if !self.complete { return Status::Incomplete; }
        if self.issues.iter().any(|i| i.affects_status && i.severity == Severity::Error) { return Status::Errors; }
        if self.issues.iter().any(|i| i.affects_status && i.severity == Severity::Warning) { return Status::Issues; }
        if self.empty { return Status::Empty; }
        Status::Clean
    }

    /// How many findings of `severity` count towards the badge.
    pub fn counted(&self, severity: Severity) -> usize {
        self.issues.iter().filter(|i| i.affects_status && i.severity == severity).count()
    }

    /// Complete, stable diagnostic text. The launcher prepends release/scope/freshness.
    pub fn to_text(&self) -> String {
        let mut out = String::from("TA Reborn static mod validation\n");
        let _ = writeln!(out, "Validator schema: {}\nStatus: {:?}\nComplete: {}", self.schema_version, self.status(), self.complete);
        let _ = writeln!(out, "Checked {}/{} files; {} parse/read failures; {} units, {} weapons, {} features, {} maps, {} sounds.", self.counts.files_checked, self.counts.files_total, self.counts.files_failed, self.counts.units, self.counts.weapons, self.counts.features, self.counts.maps, self.counts.sounds);
        out.push_str("Static checks cannot prove gameplay, balance, performance, multiplayer compatibility or rendering on a particular GPU.\n");
        for (n, issue) in self.issues.iter().enumerate() {
            let origin = match issue.origin {
                Origin::Mod => "",
                Origin::Caused => " (in inherited content, caused by this mod)",
                Origin::Inherited => " (inherited: also there without this mod; does not affect the badge)",
            };
            let file = if issue.path.is_empty() { "(effective content set)".to_string() } else {
                let mut file = issue.path.clone();
                if let Some(line) = issue.line { let _ = write!(file, ", line {line}"); }
                if !issue.source.is_empty() { let _ = write!(file, " (in {})", issue.source); }
                file
            };
            let _ = writeln!(out, "\n{}. {:?} [{}]{origin}\n   File: {file}\n   Layer: {}\n   {}\n   Suggested action: {}", n + 1, issue.severity, issue.code, if issue.layer.is_empty() { "(combined)" } else { &issue.layer }, issue.message, issue.suggestion);
        }
        if self.issues.is_empty() { out.push_str("\nNo issues found by the completed checks.\n"); }
        out
    }

    /// Errors first, then warnings, then notes, what counts ahead of what does not, then by
    /// file and code: stable, so copied reports are diffable.
    fn sort(&mut self) {
        self.issues.sort_by(|a, b| (!a.affects_status, severity_order(a.severity), a.origin, &a.path, a.line, &a.code, &a.message).cmp(&(!b.affects_status, severity_order(b.severity), b.origin, &b.path, b.line, &b.code, &b.message)));
    }
}

/// Marks what the mod changed: a finding in inherited content that `baseline` (the same
/// content without the mod: the original game, Reborn's own and the mod's dependencies) does
/// not have is the mod's doing, [`Origin::Caused`], and counts. A mod that replaces a shared
/// definition file breaks units it never mentions; this is how they are blamed on it.
pub fn compare(report: &mut Report, baseline: &Report) {
    let known: BTreeSet<(&str, &str, Option<u32>, &str)> = baseline.issues.iter().map(|i| (i.code.as_str(), i.path.as_str(), i.line, i.message.as_str())).collect();
    for issue in &mut report.issues {
        let without = known.contains(&(issue.code.as_str(), issue.path.as_str(), issue.line, issue.message.as_str()));
        if issue.origin == Origin::Inherited && issue.severity != Severity::Info && !without {
            issue.origin = Origin::Caused;
            issue.affects_status = true;
        } else if issue.origin == Origin::Mod && issue.code == "format.invalid" && without {
            // A file of the mod's that does not read just as the one it is laid over does not (a
            // copy of the original game's own: Total Mayhem carries TA's `anims/vismasks.gaf`,
            // which is no GAF) is no fault of the mod's.
            issue.origin = Origin::Inherited;
            issue.affects_status = false;
        }
    }
    report.sort();
}

#[derive(Clone, Debug)]
pub struct Options {
    /// A mod's VFS layer. Inherited failures reached from this mod's references count;
    /// unrelated original-game findings are retained as context. None checks all layers.
    pub target_layer: Option<String>,
    pub max_files: usize,
    pub max_text_bytes: usize,
    pub max_read_bytes: u64,
    pub max_retained_bytes: usize,
    /// Reference GPU budgets, not a claim about the current device. Callers may supply
    /// actual known limits; defaults match the common 2,048-layer / 16K profile.
    pub texture_array_layers: u32,
    pub sprite_atlas_side: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self { target_layer: None, max_files: 100_000, max_text_bytes: 8 << 20, max_read_bytes: 16 << 30, max_retained_bytes: 64 << 20, texture_array_layers: 2048, sprite_atlas_side: 16384 }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Progress<'a> { pub checked: u64, pub total: u64, pub path: &'a str }

pub fn validate_all(vfs: &Vfs) -> Report { validate(vfs, &Options::default(), |_| true) }

/// Files decoded together, in parallel, before they are folded into the report in order.
const CHUNK: usize = 64;

/// How many texture arrays the game spreads the models' paint over (ta_render's
/// `TEXTURE_BANKS`): the textures fit in this many times the reference profile's layers.
const TEXTURE_BANKS: u64 = 2;

/// Returning false from `progress` cancels before the next file. Parsers themselves
/// enforce their engine limits; a file currently being decoded runs to that bound.
pub fn validate(vfs: &Vfs, options: &Options, mut progress: impl FnMut(Progress<'_>) -> bool) -> Report {
    let mut scan = Scan::new(vfs, options);
    if vfs.file_count() > options.max_files {
        scan.stop("validation.limit", "", "The effective content set exceeds the validation file budget.", "Split the content into smaller optional packs, then validate each pack.");
        return scan.report;
    }
    let everything = vfs.list("", None, true);
    let files: Vec<_> = everything.iter().filter(|p| known(p)).cloned().collect();
    scan.report.counts.files_total = files.len() as u64;
    if let Some(target) = &options.target_layer {
        let own: Vec<&str> = everything.iter().filter(|p| vfs.origin(p).is_some_and(|o| &o.layer == target)).map(String::as_str).collect();
        if own.is_empty() {
            scan.stop("validation.no_content", "", "No files of this mod were available to validate.", "Install the mod's TA data archives or loose files, then validate again.");
            return scan.report;
        }
        scan.packaging(&own);
        if !own.iter().any(|p| known(p)) {
            // Music, documents or pictures only: nothing of the game's data to check, and
            // nothing the mod changes in it.
            scan.report.empty = true;
            scan.note("validation.nothing", "", "This mod holds nothing the data checks read: no units, weapons, features, maps, models, scripts, animations or sounds (music, documents and pictures are not checked).", "Nothing to do if that is what the mod is. If it should change the game's data, check that its files are in the folders the game reads (units/, weapons/, maps/…).");
            scan.finish_scope();
            scan.report.sort();
            return scan.report;
        }
    } else if files.is_empty() {
        scan.stop("validation.no_content", "", "No supported effective files were available to validate.", "Install the original game's or the mod's TA data archives, then validate again.");
        return scan.report;
    }
    let total = files.len() as u64;
    let mut bytes_read = 0u64;
    // (Files over 32 MiB are read one at a time after the rest of their chunk: a pack of huge
    // ones must not hold gigabytes on every thread at once.)
    let huge = |path: &String| vfs.size(path).is_none_or(|n| n > 32 << 20);
    'files: for chunk in files.chunks(CHUNK) {
        let parsed: Vec<Parsed> = chunk.par_iter().map(|path| (!huge(path)).then(|| parse(vfs, path, options))).collect::<Vec<_>>()
            .into_iter().zip(chunk).map(|(parsed, path)| parsed.unwrap_or_else(|| parse(vfs, path, options))).collect();
        for (path, parsed) in chunk.iter().zip(parsed) {
            if !progress(Progress { checked: scan.report.counts.files_checked, total, path }) {
                scan.stop("validation.cancelled", path, "Validation was cancelled; remaining files and references were not checked.", "Run validation again to obtain a complete report.");
                break 'files;
            }
            bytes_read = bytes_read.saturating_add(parsed.bytes);
            if bytes_read > options.max_read_bytes {
                scan.stop("validation.limit", path, "The cumulative validation read budget was exceeded.", "Split optional content into smaller packs, then validate again.");
                break 'files;
            }
            scan.report.counts.files_checked += 1;
            scan.file(path, parsed);
            if scan.halted { break 'files; }
        }
    }
    if !scan.halted { scan.blank_notes(); }
    if !scan.halted { scan.references(); }
    scan.finish_scope();
    scan.report.sort();
    // CANBUILD commonly repeats an option to weight the original AI's choice. The
    // game deduplicates these menu entries; report the same broken reference once.
    scan.report.issues.dedup_by(|a, b| a.affects_status == b.affects_status && a.severity == b.severity && a.path == b.path && a.line == b.line && a.code == b.code && a.message == b.message);
    scan.report
}

/// Whether the checks read a file of this (lower-case) name: with the game's archives, what a
/// content fingerprint must cover.
pub fn checked_file(name: &str) -> bool { known(name) }

/// The note that the mod checked (its layer `layer`) replaces the original game's archive
/// `archive` (as the install names it, `ta_vfs::mods::replaced_archives`): with it on, the
/// install's archive of that name is not read. A note: it never counts.
pub fn replaced_archive(layer: &str, archive: &str) -> Issue {
    Issue {
        severity: Severity::Info, code: "packaging.replaces_original".into(), path: archive.into(), layer: layer.into(),
        message: format!("With this mod on, the original game's {archive} is not read: the mod takes its place (an archive of its own of that name, one its mod.toml's replaces names, or one its installation is known to remove), as installing the mod in TA's folder did. What only the original's held is missing with the mod."),
        suggestion: "Nothing to do if this is meant. To have the original's read beneath the mod instead, give the mod's archive another name, or take the name out of replaces in mod.toml.".into(),
        ..Issue::default()
    }
}

/// A mod whose content is all in archives the game cannot open (`unreadable`, the findings for
/// them, found before mounting it): those findings are its report, a complete one, as for a mod
/// that is one such archive, not a check that found nothing of it to read.
pub fn content_unreadable(report: &mut Report, unreadable: &[Issue]) {
    if unreadable.iter().any(|i| i.affects_status && i.severity == Severity::Error) && report.issues.iter().any(|i| i.code == "validation.no_content") {
        report.issues.retain(|i| i.code != "validation.no_content");
        report.complete = true;
    }
}

fn severity_order(s: Severity) -> u8 { match s { Severity::Error => 0, Severity::Warning => 1, Severity::Info => 2 } }
fn known(p: &str) -> bool { p.rsplit_once('.').is_some_and(|(_, e)| matches!(e, "3do" | "cob" | "gaf" | "tnt" | "pcx" | "tdf" | "fbi" | "gui" | "ota" | "fnt" | "pal" | "wav")) }
fn key(kind: &str, name: &str) -> String { format!("{kind}:{}", name.trim().to_ascii_lowercase()) }
fn asset(folder: &str, name: &str, ext: &str) -> String { ta_vfs::normalize(&format!("{folder}/{}.{ext}", name.trim())) }
fn direct(path: &str, folder: &str, ext: &str) -> bool { path.starts_with(&format!("{folder}/")) && path.matches('/').count() == 1 && path.ends_with(&format!(".{ext}")) }
/// Whose a layer is, as a sentence names it.
fn owner(layer: &str) -> String {
    match layer {
        "base" => "the original game's".into(),
        ta_vfs::BUILTIN_LAYER => "Reborn's own".into(),
        _ => format!("{}'s", layer.strip_prefix("mod:").unwrap_or(layer)),
    }
}

fn is_archive(path: &str) -> bool { path.rsplit_once('.').is_some_and(|(_, e)| ta_vfs::ARCHIVE_EXTENSIONS.contains(&e)) }

/// A sound's file, as the game finds it (in step with `ta_audio::bank::sound_path`):
/// `sounds/<name>.wav`, a name with a folder kept as it is, TA's misspelt `untdone` read as
/// `unitdone`.
fn sound_path(name: &str) -> String {
    let name = name.trim();
    let name = if name.eq_ignore_ascii_case("untdone") { "unitdone" } else { name };
    let mut path = ta_vfs::normalize(name);
    if !path.contains('/') { path.insert_str(0, "sounds/"); }
    if !path.rsplit('/').next().unwrap_or_default().contains('.') { path.push_str(".wav"); }
    path
}

/// A reference's target, as a sentence names it: "unit ARMCK", "the file objects3d/x.3do".
fn describe(reference: &str) -> String {
    let (kind, name) = reference.split_once(':').unwrap_or(("", reference));
    let upper = name.to_ascii_uppercase();
    match kind {
        "file" => format!("the file {name}"),
        "unit" => format!("unit {upper}"),
        "weapon" => format!("weapon {upper}"),
        "feature" => format!("feature {upper}"),
        "side" => format!("faction {upper}"),
        "move" => format!("movement class {upper}"),
        "texture" => format!("texture {upper}"),
        "sound" => format!("sound {upper} ({})", sound_path(name)),
        "sounds" => format!("sound category {upper} (gamedata/sound.tdf)"),
        "placed" => format!("unit or feature {upper}"),
        "gaf" => match name.split_once('#') { Some((file, entry)) => format!("animation {} in {file}", entry.to_ascii_uppercase()), None => name.to_string() },
        _ => reference.to_string(),
    }
}

/// The folders TA's own data lives in (a mod's files belong in the same).
const TA_FOLDERS: [&str; 20] = ["ai", "anims", "bitmaps", "camps", "download", "features", "fonts", "gamedata", "guis", "maps", "music", "objects3d", "palettes", "scripts", "sections", "sounds", "textures", "unitpics", "units", "weapons"];

/// Where the game reads a kind of file from, when `path` is not there.
fn misplaced(path: &str) -> Option<&'static str> {
    let ext = path.rsplit_once('.')?.1;
    let depth = path.matches('/').count();
    let top = path.split('/').next().unwrap_or("");
    let home = match ext {
        "fbi" => (top == "units" && depth == 1, "units/"),
        "3do" => (top == "objects3d", "objects3d/"),
        "cob" => (top == "scripts", "scripts/"),
        "gui" => (top == "guis" && depth == 1, "guis/"),
        "tnt" | "ota" => (top == "maps" && depth == 1, "maps/"),
        "gaf" => (matches!(top, "anims" | "textures" | "bitmaps") && depth == 1 || top == "camps", "anims/ or textures/"),
        "fnt" => (matches!(top, "fonts" | "guis"), "fonts/"),
        "tdf" => (matches!(top, "weapons" | "gamedata" | "download" | "maps") && depth == 1 || matches!(top, "features" | "camps"), "weapons/, features/, gamedata/, download/ or camps/"),
        // (The music folder's tracks are WAVs too; a sound named with a folder is read from it.)
        "wav" => (top == "sounds" || top == "camps" || top == "music" && depth == 1, "sounds/"),
        _ => return None,
    };
    (!home.0).then_some(home.1)
}

/// A reason a WAV file will not play, if any: not RIFF WAVE, no format or samples, or a codec
/// the game's decoder (`ta_audio::decode::wav`) does not read.
fn wav_problem(bytes: &[u8]) -> Option<String> {
    // PCM, MS ADPCM, float, A-law, µ-law, IMA ADPCM; WAVE_FORMAT_EXTENSIBLE wraps them.
    const READ: [u16; 6] = [0x0001, 0x0002, 0x0003, 0x0006, 0x0007, 0x0011];
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" { return Some("it is not a RIFF WAVE file".into()); }
    let word = |at: usize| bytes.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let (mut at, mut tag, mut data) = (12usize, None, false);
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]]) as usize;
        let body = at + 8;
        if id == b"fmt " {
            tag = word(body).map(|t| if t == 0xFFFE { word(body + 24).unwrap_or(t) } else { t });
        } else if id == b"data" {
            data = true;
        }
        at = body.saturating_add(size).saturating_add(size & 1);
    }
    match tag {
        None => Some("it has no format chunk".into()),
        Some(t) if !READ.contains(&t) => Some(format!("its codec ({t:#06x}) is not one the game plays (PCM, ADPCM, float, A-law or µ-law)")),
        _ if !data => Some("it has no sample data".into()),
        _ => None,
    }
}

/// A file read and decoded, summarised: what the report needs of it.
struct Parsed { bytes: u64, content: Result<Content, Failure> }
enum Failure { Read(String), Invalid(String), TooLarge }
enum Content {
    Model { pieces: BTreeSet<String>, textures: BTreeSet<String> },
    Script { pieces: BTreeSet<String>, functions: BTreeSet<String>, problems: Vec<Problem> },
    /// A GAF's entries; `whole`: the game could decode it all at once (as it does every GAF but
    /// the weapons' explosions, which it reads a frame at a time: `gaf_content`).
    Gaf { entries: Vec<GafEntry>, whole: bool },
    Map { features: BTreeSet<String> },
    /// A picture, font or palette: readable, nothing more to it.
    Plain,
    /// A WAV, and what keeps it from playing, if anything.
    Sound(Option<String>),
    Text { doc: TdfDocument, syntax: Vec<(u32, String)> },
    /// An empty file: TA's way of blanking out a file of the same name below it (the original
    /// game's, or a mod's it is mounted over), nothing to read, not a broken one (`blank_notes`).
    Blank,
}
struct GafEntry { name: String, frames: usize, size: AnimSize }
struct Problem { severity: Severity, code: &'static str, message: String, suggestion: &'static str }

/// Reads and decodes one file (on any thread).
fn parse(vfs: &Vfs, path: &str, options: &Options) -> Parsed {
    let bytes = match vfs.read(path) {
        Ok(b) => b,
        // (Its own words, without the folder it is in: reports are shared.)
        Err(ta_vfs::VfsError::Io { source, .. }) => return Parsed { bytes: 0, content: Err(Failure::Read(source.to_string())) },
        Err(ta_vfs::VfsError::Format { source, .. }) => return Parsed { bytes: 0, content: Err(Failure::Read(source.to_string())) },
        Err(e) => return Parsed { bytes: 0, content: Err(Failure::Read(e.to_string())) },
    };
    let size = bytes.len() as u64;
    let ext = path.rsplit_once('.').map_or("", |(_, e)| e);
    if bytes.is_empty() {
        return Parsed { bytes: 0, content: Ok(Content::Blank) };
    }
    let content = match ext {
        "3do" => Model3do::parse(&bytes).map(|model| Content::Model {
            pieces: model.pieces.iter().map(|p| p.name.to_ascii_lowercase()).collect(),
            // Selection plates and line primitives are not textured geometry.
            textures: model.pieces.iter().flat_map(|p| p.primitives.iter().enumerate().filter(|(i, r)| Some(*i) != p.selection_primitive && r.indices.len() >= 3).filter_map(|(_, r)| r.texture.clone())).collect(),
        }).map_err(|e| Failure::Invalid(e.to_string())),
        "cob" => CobScript::parse(&bytes).map(|script| Content::Script {
            pieces: script.pieces.iter().map(|s| s.to_ascii_lowercase()).collect(),
            functions: script.functions.iter().map(|f| f.name.to_ascii_lowercase()).collect(),
            problems: script_problems(&script),
        }).map_err(|e| Failure::Invalid(e.to_string())),
        "gaf" => gaf_content(&bytes),
        "tnt" => ta_formats::tnt::TntMap::parse(&bytes).map(|map| Content::Map {
            features: map.features.iter().filter_map(|&n| map.feature_names.get(n as usize)).filter(|n| !n.trim().is_empty()).cloned().collect(),
        }).map_err(|e| Failure::Invalid(e.to_string())),
        "pcx" => ta_formats::pcx::PcxImage::parse(&bytes).map(|_| Content::Plain).map_err(|e| Failure::Invalid(e.to_string())),
        "fnt" => ta_formats::fnt::Font::parse(&bytes).map(|_| Content::Plain).map_err(|e| Failure::Invalid(e.to_string())),
        "pal" => ta_formats::image::Palette::parse(&bytes).map(|_| Content::Plain).map_err(|e| Failure::Invalid(e.to_string())),
        "wav" => Ok(Content::Sound(wav_problem(&bytes))),
        _ if bytes.len() > options.max_text_bytes => Err(Failure::TooLarge),
        // (TA ships some GUI pages in a binary form of its own: no text to lint.)
        _ => Ok(Content::Text { doc: tdf::parse_bytes(&bytes), syntax: if bytes.contains(&0) { Vec::new() } else { tdf::lint(&bytes) } }),
    };
    Parsed { bytes: size, content }
}

/// A GAF read a frame at a time (`GafReader`), nothing of it kept: its entries, every frame
/// read through as the game decodes it (a damaged one makes the file invalid), and whether the
/// game could decode it all at once, as it does every GAF but the weapons' explosions (those it
/// reads a frame at a time, drawing a large one reduced: see `Scan::explosions`).
fn gaf_content(bytes: &[u8]) -> Result<Content, Failure> {
    let invalid = |e: ta_formats::FormatError| Failure::Invalid(e.to_string());
    let reader = GafReader::open(bytes).map_err(invalid)?;
    let costs = reader.entries.iter().map(|e| e.frames.iter().map(|f| reader.check(f)).collect::<Result<Vec<u64>, _>>()).collect::<Result<Vec<_>, _>>().map_err(invalid)?;
    Ok(Content::Gaf {
        entries: reader.entries.iter().map(|e| GafEntry { name: e.name.clone(), frames: e.frames.len(), size: e.size() }).collect(),
        whole: reader.whole(&costs),
    })
}

/// What is wrong with a script's code, as the VM runs it: from each function's entry, along its
/// jumps, to its returns. Known instruction boundaries only, an unknown opcode's operands never
/// read as more opcodes. What no function reaches is never run, so it is not decoded: the words
/// a compiler leaves between functions (Scriptor's "COB BLER" puts its signature after a
/// function's last `return`, in TA: Enhanced's and Total Mayhem's scripts) are no unknown opcode.
/// A function's code ends at the next function's entry (that one is checked from its own).
fn script_problems(script: &CobScript) -> Vec<Problem> {
    use cob::op::*;
    let mut problems = Vec::new();
    let entries: BTreeSet<usize> = script.functions.iter().map(|f| f.offset as usize).collect();
    let len = script.code.len();
    let mut decoded = vec![false; len];
    // (The places to decode from, lowest first: the entries, then where their jumps lead.)
    let mut pending = entries.clone();
    while let Some(mut pc) = pending.pop_first() {
        // (Past the last instruction: a terminal label, which ends the thread.)
        while pc < len && !decoded[pc] {
            decoded[pc] = true;
            let end = entries.range(pc + 1..).next().copied().unwrap_or(len);
            let opcode = script.code[pc];
            let Some(n) = cob::operand_count(opcode) else {
                problems.push(Problem { severity: Severity::Error, code: "script.opcode", message: format!("Unknown COB opcode {opcode:#010x} at word {pc}; the VM cannot execute this instruction. The remainder of this function cannot be decoded reliably."), suggestion: "Recompile the BOS script with original-TA-compatible opcodes, or report the required opcode extension to Reborn's maintainers." });
                break;
            };
            if pc + 1 + n > end {
                problems.push(Problem { severity: Severity::Error, code: "script.operands", message: format!("COB instruction {} at word {pc} is truncated or crosses the next function entry point.", cob::opcode_name(opcode)), suggestion: "Recompile this COB from its BOS source; verify function offsets and inline operand counts." });
                break;
            }
            let arg = script.code.get(pc + 1).copied().unwrap_or(0);
            let invalid = match opcode {
                MOVE | TURN | SPIN | STOP_SPIN | WAIT_TURN | WAIT_MOVE | MOVE_NOW | TURN_NOW => (arg as usize >= script.pieces.len() || script.code[pc + 2] > 2).then_some("piece index or axis"),
                SHOW | HIDE | CACHE | DONT_CACHE | SHADE | DONT_SHADE | EMIT_SFX | EXPLODE => (arg as usize >= script.pieces.len()).then_some("piece index"),
                PUSH_STATIC | POP_STATIC => (arg >= script.static_var_count).then_some("static variable index"),
                // A terminal label exactly past the last instruction ends the
                // thread (VM's code.get(pc) returns None). Some BOS compilers
                // emit this for the final else branch of Killed().
                JUMP | JUMP_IF_FALSE => (arg as usize > len).then_some("jump destination"),
                START_SCRIPT | CALL_SCRIPT => (arg as usize >= script.functions.len()).then_some("function index"),
                _ => None,
            };
            if let Some(what) = invalid {
                problems.push(Problem { severity: Severity::Error, code: "script.operand_range", message: format!("COB instruction {} at word {pc} has an invalid {what}.", cob::opcode_name(opcode)), suggestion: "Recompile the script with valid piece, variable, function and jump references." });
                // (Where a jump out of the code would lead is not followed.)
                if matches!(opcode, JUMP | JUMP_IF_FALSE) { break; }
            }
            match opcode {
                RETURN => break,
                JUMP => pc = arg as usize,
                JUMP_IF_FALSE => {
                    pending.insert(arg as usize);
                    pc += 1 + n;
                }
                _ => pc += 1 + n,
            }
            // (Run on into the next function: its code is checked from its own entry.)
            if entries.contains(&pc) { break; }
        }
    }
    problems
}

struct Reference { key: String, what: String, severity: Severity }
impl Reference {
    fn new(kind: &str, name: &str, what: impl Into<String>, severity: Severity) -> Self { Self { key: key(kind, name), what: what.into(), severity } }
}
#[derive(Default)]
struct Record { path: String, refs: Vec<Reference>, unit: Option<UnitFacts> }
/// What a reference leads to: a whole file (a model, a script, a sound, a map's terrain) or a
/// definition (a weapon in a file of many).
enum Use { File(String), Record(String) }
/// What the later checks need of a unit.
/// `script`: the unit's script as the game finds it (`UnitDef::script_path`).
struct UnitFacts { name: String, object: String, script: String, builder: bool, weapons: [bool; 3] }
#[derive(Default)]
struct Animation { frames: usize, size: AnimSize }

struct Scan<'a> {
    vfs: &'a Vfs,
    options: &'a Options,
    report: Report,
    retained: usize,
    halted: bool,
    records: BTreeMap<String, Record>,
    pieces: BTreeMap<String, BTreeSet<String>>,
    functions: BTreeMap<String, BTreeSet<String>>,
    animations: BTreeMap<String, Animation>,
    /// The GAFs too large for the game to decode whole: their frames, the longest side of any
    /// of them, and their pixels (see `explosions`).
    too_big: BTreeMap<String, (u64, u32, u64)>,
    texture_names: BTreeSet<String>,
    /// The model textures (`textures/*.gaf`), first entry of a name only, in the order the game
    /// takes them: name (lower case), frames and file; and the names the models in `objects3d`
    /// paint with, all the game loads of them (`texture_capacity`).
    texture_entries: Vec<(String, usize, String)>,
    model_textures: BTreeSet<String>,
    /// The empty files, in path order (told a folder at a time: `blank_notes`).
    blanks: Vec<String>,
    /// Production pages (`guis/<builder><page>.gui`): the units their buttons name.
    gui_pages: BTreeMap<String, Vec<Reference>>,
    /// Units some menu offers (CANBUILD, download menus, production pages).
    buildable: BTreeSet<String>,
    /// The target's definitions an inherited one of the same name comes after, by the
    /// target's file: what, the later file, whose it is.
    shadowed: BTreeMap<String, Vec<(String, String, String)>>,
    /// What each definition uses, by its key.
    uses: BTreeMap<String, Vec<Use>>,
    /// Each definition's file, and each file's definitions (from the records, in `references`).
    record_paths: BTreeMap<String, String>,
    file_records: BTreeMap<String, Vec<String>>,
    /// The definition a finding is about, where it is about one (by its place in the report):
    /// in a file of many, it counts only where that one is used.
    attributed: BTreeMap<usize, String>,
    relevant: BTreeSet<String>,
    /// Essential inherited launch inputs. Only errors here affect an unrelated mod;
    /// their optional art/menu warnings are still baseline context.
    required: BTreeSet<String>,
}

impl<'a> Scan<'a> {
    fn new(vfs: &'a Vfs, options: &'a Options) -> Self {
        Self {
            vfs, options, report: Report { complete: true, target_layer: options.target_layer.clone(), ..Default::default() }, retained: 0, halted: false,
            records: BTreeMap::new(), pieces: BTreeMap::new(), functions: BTreeMap::new(), animations: BTreeMap::new(), too_big: BTreeMap::new(), texture_names: BTreeSet::new(), texture_entries: Vec::new(), model_textures: BTreeSet::new(), blanks: Vec::new(),
            gui_pages: BTreeMap::new(), buildable: BTreeSet::new(), shadowed: BTreeMap::new(), uses: BTreeMap::new(), record_paths: BTreeMap::new(),
            file_records: BTreeMap::new(), attributed: BTreeMap::new(), relevant: BTreeSet::new(), required: BTreeSet::new(),
        }
    }

    fn stop(&mut self, code: &str, path: &str, message: &str, suggestion: &str) {
        if self.halted { return; }
        self.halted = true;
        self.report.complete = false;
        let issue = Issue { code: code.into(), path: path.into(), message: message.into(), suggestion: suggestion.into(), ..self.located(path) };
        self.report.issues.push(issue);
    }

    /// An issue's layer and archive, from where its file comes from.
    fn located(&self, path: &str) -> Issue {
        let origin = self.vfs.origin(path);
        let source = origin.filter(|o| o.path.extension().and_then(|e| e.to_str()).is_some_and(|e| ta_vfs::ARCHIVE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())))
            .and_then(|o| o.path.file_name()).map_or(String::new(), |n| n.to_string_lossy().into_owned());
        Issue { layer: origin.map_or(String::new(), |o| o.layer.clone()), source, ..Issue::default() }
    }

    fn retain(&mut self, bytes: usize, path: &str) -> bool {
        if self.halted { return false; }
        match self.retained.checked_add(bytes) {
            Some(n) if n <= self.options.max_retained_bytes => { self.retained = n; true }
            _ => { self.stop("validation.limit", path, "The retained diagnostic/reference budget was exceeded. This is a partial report, not a clean validation.", "Reduce the pack's number of definitions or split optional content; rerun validation after resolving reported issues."); false }
        }
    }

    fn issue(&mut self, severity: Severity, code: &str, path: &str, message: String, suggestion: &str) {
        self.issue_at(severity, code, path, None, message, suggestion);
    }

    fn issue_at(&mut self, severity: Severity, code: &str, path: &str, line: Option<u32>, message: String, suggestion: &str) {
        let located = self.located(path);
        if self.retain(256 + code.len() + path.len() + located.layer.len() + located.source.len() + message.len() + suggestion.len(), path) {
            self.report.issues.push(Issue { severity, code: code.into(), path: path.into(), line, message, suggestion: suggestion.into(), ..located });
        }
    }

    /// The findings told since `from` are about the definition `key`.
    fn attribute(&mut self, from: usize, key: &str) {
        for i in from..self.report.issues.len() { self.attributed.insert(i, key.to_string()); }
    }

    /// A note about the whole check (no file).
    fn note(&mut self, code: &str, path: &str, message: &str, suggestion: &str) {
        self.issue(Severity::Info, code, path, message.into(), suggestion);
    }

    fn record(&mut self, key: String, record: Record) {
        let first = self.report.issues.len();
        // The game keeps the last of two definitions of one name, in path order (as here),
        // whatever their layers. One content's two is dead content, a slip when files are
        // copied; a mod's replacing the original game's is how mods change it, unless the
        // original's comes later: then the mod's is never used.
        if let Some((kind, _)) = key.split_once(':') && matches!(kind, "unit" | "weapon" | "feature" | "side" | "move" | "sounds") && let Some(old) = self.records.get(&key) {
            let layer = |path: &str| self.vfs.origin(path).map_or(String::new(), |o| o.layer.clone());
            let (old_path, old_layer, new_layer) = (old.path.clone(), layer(&old.path), layer(&record.path));
            let what = describe(&key);
            if old_path == record.path {
                self.issue(Severity::Warning, "definition.duplicate", &record.path, format!("{what} is defined twice in this file; the game uses the later one."), "Give each definition a name of its own, or remove the one not meant to be used.");
            } else if old_layer == new_layer {
                self.issue(Severity::Warning, "definition.duplicate", &record.path, format!("{what} is also defined in {old_path}; the game uses this one ({}).", record.path), "Give each definition a name of its own, or remove the one not meant to be used.");
            } else if self.options.target_layer.as_deref() == Some(old_layer.as_str()) {
                // (Told once a file in `references`: a file of them is common.)
                if self.retain(160 + old_path.len() + what.len() + record.path.len(), &old_path) {
                    self.shadowed.entry(old_path).or_default().push((what, record.path.clone(), owner(&new_layer)));
                }
            }
        }
        self.attribute(first, &key);
        let size = 256 + key.len() + record.path.len() + record.unit.as_ref().map_or(0, |u| 64 + u.name.len() + u.object.len()) + record.refs.iter().map(|r| 96 + r.key.len() + r.what.len()).sum::<usize>();
        if self.retain(size, &record.path) { self.records.insert(key, record); }
    }

    /// The target's files where the game never reads them: archives in subfolders (only the
    /// top level's are opened), data files outside their folders, and a whole mod wrapped in
    /// a folder of its own ("My Mod/units/…").
    fn packaging(&mut self, own: &[&str]) {
        let mut misplaced_by_top: BTreeMap<&str, Vec<(&str, &'static str)>> = BTreeMap::new();
        for &path in own {
            if is_archive(path) && path.contains('/') {
                self.issue(Severity::Warning, "packaging.nested_archive", path, "The game opens archives only at the mod's top level; this one, in a subfolder, is never read.".into(), "Move the archive to the mod's top level, or unpack its contents into the mod's folders.");
            } else if let Some(home) = misplaced(path) {
                misplaced_by_top.entry(path.split('/').next().unwrap_or("")).or_default().push((path, home));
            }
            if self.halted { return; }
        }
        for (top, list) in misplaced_by_top {
            let wrapped = !top.is_empty() && !TA_FOLDERS.contains(&top) && list.iter().all(|(p, _)| p.split('/').nth(1).is_some_and(|second| TA_FOLDERS.contains(&second)));
            if wrapped {
                let message = format!("The game's folders are inside {top}/: the game reads units/, weapons/, maps/… only at the mod's top level, so nothing in it is used.");
                self.issue(Severity::Warning, "packaging.wrapped", top, message, "Move the contents of this folder up to the mod's top level.");
                // (A folder has no layer of its own: it is the files'.)
                let layer = self.vfs.origin(list[0].0).map_or(String::new(), |o| o.layer.clone());
                if let Some(issue) = self.report.issues.last_mut() { issue.layer = layer; }
            } else if list.len() > 3 {
                let (first, home) = list[0];
                self.issue(Severity::Warning, "packaging.misplaced", first, format!("{} files under {} are where the game never reads them; this one's kind is read from {home}.", list.len(), if top.is_empty() { "the mod's top level".to_string() } else { format!("{top}/") }), "Move each file to the folder the game reads its kind from.");
            } else {
                for (path, home) in list {
                    let ext = path.rsplit_once('.').map_or("", |(_, e)| e);
                    self.issue(Severity::Warning, "packaging.misplaced", path, format!("The game never reads this file: it reads .{ext} files from {home}."), "Move it to that folder, or remove it if it is not needed.");
                }
            }
        }
    }

    fn file(&mut self, path: &str, parsed: Parsed) {
        if self.options.target_layer.as_ref().is_some_and(|target| self.vfs.origin(path).is_some_and(|o| &o.layer == target)) {
            if !self.retain(96 + path.len(), path) { return; }
            self.relevant.insert(path.to_string());
        }
        let content = match parsed.content {
            Ok(content) => content,
            Err(Failure::Read(e)) => {
                self.report.counts.files_failed += 1;
                return self.issue(Severity::Error, "file.read", path, e, "Reinstall the package and check that its archives are intact and readable.");
            }
            Err(Failure::Invalid(e)) => {
                self.report.counts.files_failed += 1;
                return self.issue(Severity::Error, "format.invalid", path, e, "Re-export this asset in the original TA format, within Reborn's documented parser limits; replace truncated or corrupt files.");
            }
            Err(Failure::TooLarge) => return self.stop("validation.limit", path, "This text file exceeds the validator's per-file text budget and was not parsed.", "Split large definition files into smaller files before validating again."),
        };
        match content {
            Content::Model { pieces, textures } => {
                self.report.counts.models += 1;
                if !self.retain(128 + path.len() + pieces.iter().map(|s| 80 + s.len()).sum::<usize>(), path) { return; }
                self.pieces.insert(path.into(), pieces);
                // (The game loads the textures every model in objects3d paints with.)
                if path.starts_with("objects3d/") {
                    for name in &textures {
                        let name = name.to_ascii_lowercase();
                        if !self.model_textures.contains(&name) {
                            if !self.retain(64 + name.len(), path) { return; }
                            self.model_textures.insert(name);
                        }
                    }
                }
                let refs = textures.into_iter().map(|t| Reference::new("texture", &t, "Model texture", Severity::Warning)).collect();
                self.record(key("file", path), Record { path: path.into(), refs, ..Default::default() });
            }
            Content::Script { pieces, functions, problems } => {
                self.report.counts.scripts += 1;
                for p in problems {
                    self.issue(p.severity, p.code, path, p.message, p.suggestion);
                    if self.halted { return; }
                }
                if self.retain(128 + path.len() + pieces.iter().chain(&functions).map(|s| 80 + s.len()).sum::<usize>(), path) {
                    self.pieces.insert(path.into(), pieces);
                    self.functions.insert(path.into(), functions);
                }
            }
            Content::Gaf { entries, whole } => {
                self.report.counts.gafs += 1;
                if !whole {
                    // (Told once its uses are known: `explosions`.)
                    if !self.retain(96 + path.len(), path) { return; }
                    let frames = entries.iter().map(|e| e.size.frames).sum();
                    let largest = entries.iter().map(|e| e.size.largest).max().unwrap_or(0);
                    let pixels = entries.iter().map(|e| e.size.area).sum();
                    self.too_big.insert(path.to_string(), (frames, largest, pixels));
                }
                for entry in entries {
                    self.report.counts.frames += entry.frames as u64;
                    let entry_key = key("gaf", &format!("{path}#{}", entry.name));
                    // Gaf::entry uses the first duplicate, as does TextureLibrary.
                    if !self.animations.contains_key(&entry_key) {
                        if !self.retain(160 + entry_key.len(), path) { break; }
                        self.animations.insert(entry_key.clone(), Animation { frames: entry.frames, size: entry.size });
                        self.record(entry_key, Record { path: path.into(), ..Default::default() });
                    }
                    if direct(path, "textures", "gaf") && entry.frames > 0 && !self.texture_names.contains(&entry.name.to_ascii_lowercase()) {
                        if !self.retain(160 + 2 * entry.name.len() + path.len(), path) { break; }
                        self.texture_names.insert(entry.name.to_ascii_lowercase());
                        self.report.counts.textures += 1;
                        self.record(key("texture", &entry.name), Record { path: path.into(), ..Default::default() });
                        // (Its room is judged once every model is read: `texture_capacity`.)
                        self.texture_entries.push((entry.name.clone(), entry.frames, path.to_string()));
                    }
                    if self.halted { break; }
                }
            }
            Content::Map { features } => {
                self.report.counts.maps += 1;
                let refs = features.iter().map(|n| Reference::new("feature", n, "Map feature", Severity::Warning)).collect();
                self.record(key("file", path), Record { path: path.into(), refs, ..Default::default() });
                let ota = format!("{}.ota", path.trim_end_matches(".tnt"));
                if !self.vfs.exists(&ota) { self.issue(Severity::Warning, "map.metadata", path, "The terrain has no matching OTA metadata and cannot appear as a normal selectable map.".into(), "Include a matching maps/<name>.ota with a GlobalHeader and playable schemas."); }
            }
            Content::Plain => {
                if path.ends_with(".pcx") { self.report.counts.pcx += 1; }
            }
            Content::Blank => {
                if self.retain(64 + path.len(), path) { self.blanks.push(path.to_string()); }
            }
            Content::Sound(problem) => {
                self.report.counts.sounds += 1;
                if let Some(problem) = problem { self.issue(Severity::Warning, "sound.format", path, format!("This sound will not play: {problem}."), "Save it as a WAV file in PCM (8 or 16-bit) or IMA ADPCM, as TA's own sounds are."); }
            }
            Content::Text { doc, syntax } => {
                for (line, problem) in syntax {
                    self.issue_at(Severity::Warning, "text.syntax", path, Some(line), problem, "Correct the brackets and braces around this line: the game reads the file as it is and skips or misplaces what follows.");
                    if self.halted { return; }
                }
                self.text(path, &doc);
            }
        }
    }

    /// One finding for a footprint outside what Reborn keeps (either side).
    fn footprint(&mut self, path: &str, s: &TdfSection) {
        let range = 1..=crate::MAX_FOOTPRINT_SIDE as i64;
        let (x, z) = (s.get_i64("footprintx"), s.get_i64("footprintz"));
        if x.is_some_and(|n| !range.contains(&n)) || z.is_some_and(|n| !range.contains(&n)) {
            let side = |v: Option<i64>| v.map_or("?".to_string(), |n| n.to_string());
            self.issue(Severity::Warning, "definition.footprint", path, format!("[{}] footprint {} × {} is clamped to 1..{} cells a side by Reborn.", s.name, side(x), side(z), crate::MAX_FOOTPRINT_SIDE), "Use footprints within the supported range and regenerate the matching yard map.");
        }
    }

    fn text(&mut self, path: &str, doc: &TdfDocument) {
        if direct(path, "units", "fbi") {
            let Some(s) = doc.section("UNITINFO") else { self.issue(Severity::Error, "unit.header", path, "Missing [UNITINFO]; this unit is skipped.".into(), "Add a UNITINFO section with a non-empty UnitName."); return; };
            let Some(unit) = crate::UnitDef::from_tdf(s) else { self.issue(Severity::Error, "unit.name", path, "UNITINFO has no usable UnitName; this unit is skipped.".into(), "Give the unit a unique, non-empty UnitName and update its references."); return; };
            self.footprint(path, s);
            // (Its own script, else its model's, as the game runs it.)
            let vfs = self.vfs;
            let script = unit.script_path(|p| vfs.exists(p));
            let mut refs = vec![Reference::new("file", &asset("objects3d", &unit.object, "3do"), format!("{} model", unit.name), Severity::Error), Reference::new("file", &script, format!("{} script", unit.name), Severity::Error)];
            for (slot, w) in unit.weapons.iter().enumerate() { if let Some(w) = w { refs.push(Reference::new("weapon", w, format!("{} Weapon{}", unit.name, slot + 1), Severity::Error)); } }
            for (name, what) in [(&unit.explode_as, "death explosion"), (&unit.self_destruct_as, "self-destruct explosion")] { optional(&mut refs, "weapon", name, format!("{} {what}", unit.name), Severity::Warning); }
            optional(&mut refs, "feature", &unit.corpse, format!("{} wreckage", unit.name), Severity::Warning);
            optional(&mut refs, "side", &unit.side, format!("{} faction", unit.name), Severity::Warning);
            // (NONE: a unit meant to be silent.)
            if !unit.sound_category.trim().eq_ignore_ascii_case("none") { optional(&mut refs, "sounds", &unit.sound_category, format!("{} sounds", unit.name), Severity::Warning); }
            if unit.can_move && !unit.can_fly { optional(&mut refs, "move", &unit.movement_class, format!("{} movement class", unit.name), Severity::Warning); }
            let facts = UnitFacts { name: unit.name.clone(), object: unit.object.clone(), script, builder: unit.builder, weapons: unit.weapons.clone().map(|w| w.is_some_and(|w| !w.trim().is_empty())) };
            self.record(key("unit", &unit.name), Record { path: path.into(), refs, unit: Some(facts) });
        } else if direct(path, "weapons", "tdf") {
            for s in &doc.sections {
                let first = self.report.issues.len();
                let weapon = crate::WeaponDef::from_tdf(s);
                let mut refs = Vec::new();
                if !weapon.model.is_empty() && matches!(weapon.render_type, crate::RenderType::Model | crate::RenderType::Bomb) { refs.push(Reference::new("file", &asset("objects3d", &weapon.model, "3do"), format!("{} projectile model", weapon.name), Severity::Warning)); }
                for (gaf, entry, label) in [(&weapon.explosion_gaf, &weapon.explosion_art, "explosion"), (&weapon.water_explosion_gaf, &weapon.water_explosion_art, "water explosion"), (&weapon.lava_explosion_gaf, &weapon.lava_explosion_art, "lava explosion")] { animation_ref(&mut refs, gaf, entry, format!("{} {label}", weapon.name)); }
                for (sound, label) in [(&weapon.sound_start, "firing sound"), (&weapon.sound_hit, "impact sound"), (&weapon.sound_water, "water impact sound")] { optional(&mut refs, "sound", sound, format!("{} {label}", weapon.name), Severity::Warning); }
                if let crate::RenderType::Other(n) = weapon.render_type { self.issue(Severity::Warning, "weapon.render_type", path, format!("{} uses unsupported RenderType={n}; Reborn will use its fallback projectile appearance.", weapon.name), "Use TA RenderType 0..7, or report the required extension to Reborn's maintainers."); }
                self.attribute(first, &key("weapon", &weapon.name));
                self.record(key("weapon", &weapon.name), Record { path: path.into(), refs, ..Default::default() });
                if self.halted { break; }
            }
        } else if path.starts_with("features/") && path.ends_with(".tdf") {
            for s in &doc.sections {
                let first = self.report.issues.len();
                let f = crate::FeatureDef::from_tdf(s, "all");
                self.footprint(path, s);
                self.attribute(first, &key("feature", &f.name));
                let mut refs = Vec::new();
                if !f.object.is_empty() { refs.push(Reference::new("file", &asset("objects3d", &f.object, "3do"), format!("{} feature model", f.name), Severity::Warning)); }
                if f.is_sprite() {
                    for (entry, label) in [(&f.seq_name, "animation"), (&f.seq_name_shadow, "shadow"), (&f.seq_name_burn, "burn animation"), (&f.seq_name_burn_shadow, "burn shadow"), (&f.seq_name_die, "death animation"), (&f.seq_name_reclaim, "reclaim animation")] {
                        // TA's own shrubs use EMPTY to deliberately suppress shadows.
                        if label.contains("shadow") && entry.eq_ignore_ascii_case("empty") { continue; }
                        animation_ref(&mut refs, &f.filename, entry, format!("{} {label}", f.name));
                    }
                }
                for (name, label) in [(&f.feature_dead, "dead"), (&f.feature_burnt, "burnt"), (&f.feature_reclamate, "reclaimed")] { optional(&mut refs, "feature", name, format!("{} {label} replacement", f.name), Severity::Warning); }
                optional(&mut refs, "weapon", &f.burn_weapon, format!("{} burning weapon", f.name), Severity::Warning);
                self.record(key("feature", &f.name), Record { path: path.into(), refs, ..Default::default() });
                if self.halted { break; }
            }
        } else if path == "gamedata/moveinfo.tdf" {
            for s in &doc.sections {
                if let Some(n) = s.get("name") {
                    let first = self.report.issues.len();
                    self.footprint(path, s);
                    self.attribute(first, &key("move", n));
                    self.record(key("move", n), Record { path: path.into(), ..Default::default() });
                }
                if self.halted { break; }
            }
        } else if path == "gamedata/sound.tdf" {
            // A sound category: every entry names a sound (select1, ok1, build, count5…).
            for s in &doc.sections {
                let mut refs = Vec::new();
                for (entry, value) in &s.entries { optional(&mut refs, "sound", value, format!("{} {entry} sound", s.name.to_ascii_uppercase()), Severity::Warning); }
                self.record(key("sounds", &s.name), Record { path: path.into(), refs, ..Default::default() });
                if self.halted { break; }
            }
        } else if path == "gamedata/sidedata.tdf" {
            for s in &doc.sections {
                if s.name.to_ascii_lowercase().starts_with("side") {
                    let name = s.get("name").unwrap_or("");
                    let commander = s.get("commander").unwrap_or("");
                    if name.trim().is_empty() || commander.trim().is_empty() { self.issue(Severity::Error, "side.definition", path, format!("[{}] needs a faction name and starting commander.", s.name), "Set name and commander to a valid faction and UnitName."); }
                    let refs = vec![Reference::new("unit", commander, format!("{name} starting commander"), Severity::Error)];
                    self.record(key("side", name), Record { path: path.into(), refs, ..Default::default() });
                }
            }
            let mut refs = Vec::new();
            if let Some(can_build) = doc.section("CANBUILD") {
                for builder in &can_build.sections {
                    refs.push(Reference::new("unit", &builder.name, "Build-menu owner", Severity::Warning));
                    for (_, unit) in &builder.entries {
                        optional(&mut refs, "unit", unit, format!("{} build option", builder.name), Severity::Warning);
                        if !unit.trim().is_empty() { self.buildable.insert(key("unit", unit)); }
                    }
                }
            }
            self.record(key("file", path), Record { path: path.into(), refs, ..Default::default() });
        } else if direct(path, "download", "tdf") {
            let mut refs = Vec::new();
            for s in &doc.sections {
                if let (Some(builder), Some(unit)) = (s.get("UNITMENU"), s.get("UNITNAME")) {
                    // Community packs intentionally replace a former builder with
                    // BLANK to disable that menu entry while preserving the source.
                    if builder.trim().eq_ignore_ascii_case("blank") { continue; }
                    refs.push(Reference::new("unit", builder, "Download build-menu owner", Severity::Warning)); refs.push(Reference::new("unit", unit, format!("{builder} build option"), Severity::Warning));
                    self.buildable.insert(key("unit", unit));
                }
            }
            self.record(key("file", path), Record { path: path.into(), refs, ..Default::default() });
        } else if direct(path, "maps", "ota") {
            if doc.section("GlobalHeader").is_none() { self.issue(Severity::Warning, "map.header", path, "Missing [GlobalHeader]; this map has no usable metadata.".into(), "Export a matching OTA with a GlobalHeader and the desired game-type schemas."); }
            let map = crate::MapInfo::from_ota(doc);
            let mut refs = vec![Reference::new("file", &format!("{}.tnt", path.trim_end_matches(".ota")), "Map terrain", Severity::Error)];
            for schema in &map.schemas {
                optional(&mut refs, "weapon", &schema.meteor_weapon, "Map meteor weapon", Severity::Warning);
                for placed in &schema.placed { optional(&mut refs, "placed", &placed.what, "Map placed unit/feature", Severity::Warning); }
            }
            self.record(key("file", path), Record { path: path.into(), refs, ..Default::default() });
        } else if direct(path, "guis", "gui") {
            // Unit production buttons (which pages the game reads is settled in
            // `references`, from its builders): generic command buttons are not units.
            let mut refs = Vec::new();
            for s in &doc.sections {
                if let Some(common) = s.section("COMMON") && common.get_i32_or("id", -1) == 1 {
                    let n = common.get("name").unwrap_or("");
                    // commonattribs bit4 marks unit production. attribs bit32 also
                    // appears on empty IGPATCH slots (IGPATCH, IGPATCH2…) and stockpile
                    // buttons (commonattribs bit8), which are not missing unit names.
                    if !n.is_empty() && !n.to_ascii_uppercase().starts_with("IGPATCH") && common.get_i32_or("commonattribs", 0) & 4 != 0 { refs.push(Reference::new("unit", n, "production button", Severity::Warning)); }
                }
            }
            if self.retain(96 + path.len() + refs.iter().map(|r| 96 + r.key.len() + r.what.len()).sum::<usize>(), path) { self.gui_pages.insert(path.into(), refs); }
        } else if doc.sections.is_empty() {
            self.issue(Severity::Info, "text.no_sections", path, "This tolerant TDF parse contains no sections; section-based consumers ignore it.".into(), "Check the intended format and section headers if this file should define game content.");
        }
    }

    fn references(&mut self) {
        for (path, list) in std::mem::take(&mut self.shadowed) {
            let (what, later, whose) = &list[0];
            let message = if list.len() == 1 {
                format!("{what} defined here is never used: {later} ({whose}) defines it too and comes later, so the game uses that one.")
            } else {
                format!("{} definitions in this file are never used: other files define them too and come later, so the game uses those (for one, {what} is also in {later}, {whose}).", list.len())
            };
            self.issue(Severity::Warning, "definition.shadowed", &path, message, "Replace those files in the mod (give yours their paths), or name your file so that it sorts after them.");
        }
        let mut records = std::mem::take(&mut self.records);
        self.report.counts.units = records.keys().filter(|k| k.starts_with("unit:")).count() as u64;
        self.report.counts.weapons = records.keys().filter(|k| k.starts_with("weapon:")).count() as u64;
        self.report.counts.features = records.keys().filter(|k| k.starts_with("feature:")).count() as u64;
        // The production pages the game reads, as it reads them: a builder's
        // guis/<name>1.gui, 2, 3… up to 16, stopping at the first missing page after the first.
        let builders: Vec<(String, String)> = records.values().filter_map(|r| r.unit.as_ref().filter(|u| u.builder).map(|u| (u.name.to_ascii_lowercase(), r.path.clone()))).collect();
        for (builder, _) in &builders {
            for page in 1..=16u32 {
                let path = format!("guis/{builder}{page}.gui");
                let Some(refs) = self.gui_pages.remove(&path) else {
                    if page > 1 { break; }
                    continue;
                };
                for r in &refs { self.buildable.insert(r.key.clone()); }
                let refs = refs.into_iter().map(|r| Reference { what: format!("{} {}", builder.to_ascii_uppercase(), r.what), ..r }).collect();
                records.insert(key("gui", &path), Record { path, refs, ..Default::default() });
            }
        }
        // Required launch errors still count when a mod replaces the units folder
        // without replacing inherited sidedata. Do not seed the whole sidedata file
        // as target content: it also contains CANBUILD, which would recursively blame
        // a one-weapon mod for unrelated warnings on every original build option.
        self.required.insert("gamedata/sidedata.tdf".into());
        if records.keys().any(|k| k.starts_with("unit:")) && !records.keys().any(|k| k.starts_with("side:")) {
            self.issue(Severity::Error, "side.missing", "gamedata/sidedata.tdf", "No playable side definitions were loaded.".into(), "Include gamedata/sidedata.tdf or inherit the base game's valid sides.");
        }
        for side in records.iter().filter(|(k, _)| k.starts_with("side:")).map(|(_, r)| r) {
            for commander in side.refs.iter().filter_map(|r| records.get(&r.key)) {
                let mut paths = vec![commander.path.clone()];
                if let Some(unit) = &commander.unit { paths.extend([asset("objects3d", &unit.object, "3do"), unit.script.clone()]); }
                for path in paths {
                    if !self.retain(96 + path.len(), &path) { return; }
                    self.required.insert(path);
                }
            }
        }
        let mut effect_entries = BTreeSet::new();
        for (record_key, record) in &records {
            for reference in &record.refs {
                if let Some(used) = resolve(&reference.key, self.vfs, &records) {
                    // A unit's faction label only needs the side to exist. It does
                    // not consume every menu entry in that side's source document.
                    // A target-owned sidedata file is already seeded independently.
                    if !reference.key.starts_with("side:") {
                        let (Use::File(name) | Use::Record(name)) = &used;
                        if !self.retain(160 + record_key.len() + name.len(), &record.path) { break; }
                        self.uses.entry(record_key.clone()).or_default().push(used);
                    }
                    if record_key.starts_with("weapon:") && reference.key.starts_with("gaf:") { effect_entries.insert(reference.key.clone()); }
                } else {
                    let first = self.report.issues.len();
                    self.issue(reference.severity, "reference.missing", &record.path, format!("{}: {} is missing.", reference.what, describe(&reference.key)), "Include the referenced asset/definition, correct its name, or declare and install the required mod dependency. Names are case-insensitive.");
                    self.attribute(first, record_key);
                }
                if self.halted { break; }
            }
            if let Some(unit) = &record.unit {
                self.unit_checks(record_key, &record.path, unit);
            }
            if self.halted { break; }
        }
        for (key, record) in &records {
            self.file_records.entry(record.path.clone()).or_default().push(key.clone());
            self.record_paths.insert(key.clone(), record.path.clone());
        }
        if !self.halted {
            self.texture_capacity();
        }
        if !self.halted {
            self.explosions(&effect_entries, &records);
        }
    }

    /// The weapons' explosion animations as the game draws them (ta_game's `effects_anims.rs`):
    /// all of them in the sprite atlas from the battle's start, within the explosions' budget
    /// for the reference atlas (`gaf::explosion_budget`), each as painted, or, where together
    /// they take more, the larger ones drawn reduced (`gaf::plan`), each still as big as
    /// painted. And the GAFs too large to decode whole: the game reads only explosions a frame
    /// at a time; any other use of one (a feature's, a model's textures) finds it unreadable.
    fn explosions(&mut self, effect_entries: &BTreeSet<String>, records: &BTreeMap<String, Record>) {
        let side = self.options.sprite_atlas_side;
        let used: Vec<(String, usize, AnimSize)> = effect_entries.iter().filter_map(|k| self.animations.get(k).map(|a| (k.clone(), a.frames, a.size))).collect();
        let sizes: Vec<AnimSize> = used.iter().map(|&(_, _, size)| size).collect();
        let plan = gaf::plan(&sizes, gaf::explosion_budget(side));
        // (A rigorous lower bound: passing this check does not guarantee shelf fit.)
        let atlas = u64::from(side) * u64::from(side);
        if plan.drawn > atlas { self.issue(Severity::Warning, "render.sprite_capacity", "", format!("Referenced weapon explosions require at least {} padded sprite texels, beyond a {side} x {side} reference atlas, even drawn reduced. Frames will be omitted at this capacity; map features, generated effects and shelf packing need additional room.", plan.drawn), "Reduce explosion frame sizes/counts or split optional effects. Test on the target GPU; this static check uses a reference atlas size."); }
        for ((entry, frames, size), &scale) in used.iter().zip(&plan.scales) {
            let drawn = (f64::from(size.largest) / scale).ceil() as u32 + 2;
            if *frames == 0 || drawn > side {
                let first = self.report.issues.len();
                self.issue(Severity::Warning, "render.sprite_frames", &records[entry].path, format!("{} has no frames or contains a frame larger than the {side}-pixel reference atlas side including padding.", describe(entry)), "Include usable frames and resize oversized animation frames.");
                self.attribute(first, entry);
            }
            if self.halted { return; }
        }
        if plan.reduced() > 0 {
            let mut reduced: Vec<(u32, &str)> = used.iter().zip(&plan.scales).filter(|(_, s)| **s > 1.0).map(|((k, _, size), _)| (size.largest, k.rsplit_once('#').map_or(k.as_str(), |(_, e)| e))).collect();
            reduced.sort_by(|a, b| b.cmp(a));
            let named: Vec<String> = reduced.iter().take(3).map(|(px, name)| format!("{} ({px} px)", name.to_ascii_uppercase())).collect();
            let more = if reduced.len() > named.len() { format!(" and {} more", reduced.len() - named.len()) } else { String::new() };
            let mi = |texels: u64| texels as f64 / f64::from(1 << 20);
            self.note("render.sprite_reduced", "", &format!("The weapons' explosions take {:.0} Mi texels as painted, more than the {:.0} Mi the game gives them in a {side} x {side} atlas: {} of them, larger than TA's own, are drawn reduced to {:.0}% of their texels across, each as big as painted ({}{more}).", mi(plan.full), mi(gaf::explosion_budget(side)), reduced.len(), plan.density * 100.0, named.join(", ")), "Nothing to do: the game draws them so. Smaller paintings would keep more of their detail.");
        }
        let gaf_file = |key: &str| key.strip_prefix("gaf:").and_then(|k| k.rsplit_once('#')).map(|(path, _)| path.to_ascii_lowercase());
        let explosion_files: BTreeSet<String> = effect_entries.iter().filter_map(|k| gaf_file(k)).collect();
        let other_files: BTreeSet<String> = records.iter().filter(|(k, _)| !k.starts_with("weapon:")).flat_map(|(_, r)| r.refs.iter().filter_map(|x| gaf_file(&x.key))).collect();
        for (path, (frames, largest, pixels)) in std::mem::take(&mut self.too_big) {
            let lower = path.to_ascii_lowercase();
            if explosion_files.contains(&lower) && !other_files.contains(&lower) && !lower.starts_with("textures/") {
                self.issue(Severity::Info, "render.sprite_large", &path, format!("Large: {frames} frames up to {largest} pixels across, {:.0} million pixels in all, more than the game decodes at once. It reads them a frame at a time and draws them reduced, each as big as painted: a weapon's explosion is read so.", pixels as f64 / 1e6), "Nothing to do: the game draws it so. Smaller paintings would load faster and keep more of their detail.");
            } else {
                self.report.counts.files_failed += 1;
                self.issue(Severity::Error, "format.invalid", &path, "invalid GAF: decoded content exceeds resource limit".into(), "Re-export this asset in the original TA format, within Reborn's documented parser limits; replace truncated or corrupt files.");
            }
            if self.halted { return; }
        }
    }

    /// The empty files, a note for each folder of each layer: TA's way of taking a file out of the
    /// game is an empty one of the same name over it (Total Mayhem blanks out 57 of the original
    /// game's animations so), so an empty file is a blank, not a broken one, and nothing the game
    /// reads from it is missed: what used the file being gone is told where it is used.
    fn blank_notes(&mut self) {
        let mut folders: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
        for path in std::mem::take(&mut self.blanks) {
            let layer = self.vfs.origin(&path).map_or(String::new(), |o| o.layer.clone());
            let folder = path.rsplit_once('/').map_or(String::new(), |(folder, _)| folder.to_string());
            folders.entry((layer, folder)).or_default().push(path);
        }
        for paths in folders.into_values() {
            let message = match paths.as_slice() {
                [_] => "This file is empty: TA's way of blanking out a file of the same name below it (the original game's, or that of a mod it is mounted over). The game reads nothing from it.".to_string(),
                _ => {
                    let named: Vec<&str> = paths.iter().take(3).map(String::as_str).collect();
                    let more = if paths.len() > named.len() { format!(" and {} more", paths.len() - named.len()) } else { String::new() };
                    format!("{} empty files here ({}{more}): TA's way of blanking out files of the same names below them (the original game's, or those of a mod they are mounted over). The game reads nothing from them.", paths.len(), named.join(", "))
                }
            };
            self.issue(Severity::Info, "file.blank", &paths[0], message, "Nothing to do if they are meant to remove those files. If one should hold data, export it again.");
            if self.halted { return; }
        }
    }

    /// The model textures' room, as the game loads them (ta_render's `TextureLibrary`): only
    /// those the models in `objects3d` paint with (a mod's GAFs often hold thousands more), the
    /// first entry of a name, in file order, each whole, through [`TEXTURE_BANKS`] arrays of the
    /// reference profile's layers, one layer kept for the fallback.
    fn texture_capacity(&mut self) {
        let capacity = u64::from(self.options.texture_array_layers) * TEXTURE_BANKS;
        let mut layers = 0u64;
        for (name, frames, path) in std::mem::take(&mut self.texture_entries) {
            if !self.model_textures.contains(&name.to_ascii_lowercase()) {
                continue;
            }
            if frames > 255 {
                self.issue(Severity::Warning, "render.texture_frames", &path, format!("Texture {name} has {frames} frames; Reborn's model vertices address at most 255 and this entry uses fallback paint."), "Reduce this texture entry to at most 255 frames.");
            } else if layers + frames as u64 >= capacity {
                self.issue(Severity::Warning, "render.texture_layers", &path, format!("Texture {name} ({frames} frames) falls back: the textures the models paint with fill the reference GPU profile's {TEXTURE_BANKS} arrays of {} layers ({layers} layers before it; one is reserved for fallback). Actual device limits may differ.", self.options.texture_array_layers), "Consolidate duplicate textures or reduce animation/team frame counts. Test on the target GPU; higher limits do not fix unsupported frame counts.");
            } else {
                layers += frames as u64;
            }
            if self.halted { return; }
        }
    }

    /// A unit's script against its model and its duties, and its picture if it can be built.
    fn unit_checks(&mut self, record_key: &str, path: &str, unit: &UnitFacts) {
        let (model, script) = (asset("objects3d", &unit.object, "3do"), unit.script.clone());
        if let (Some(model_names), Some(script_names)) = (self.pieces.get(&model), self.pieces.get(&script)) {
            let missing: Vec<_> = script_names.difference(model_names).cloned().collect();
            for name in missing {
                self.issue(Severity::Warning, "script.model_piece", path, format!("{script} declares piece {name}, which is absent from {model}. Any animation or position query using it has no matching model part."), "Confirm that this is an intentional virtual/helper piece; otherwise use matching model and COB files or align the BOS piece declarations with the model's piece names.");
                if self.halted { return; }
            }
        }
        // Where the engine asks a script for its weapons' muzzles and its nano-lathe (it
        // carries on without them, but it shows). Aim functions may be missing by design:
        // TA's aircraft fire fixed weapons wherever they fly.
        if let Some(functions) = self.functions.get(&script).cloned() {
            const QUERIES: [&str; 3] = ["QueryPrimary", "QuerySecondary", "QueryTertiary"];
            for (slot, name) in QUERIES.iter().enumerate().filter(|(slot, _)| unit.weapons[*slot]) {
                if !functions.contains(&name.to_ascii_lowercase()) {
                    // (A note: mods give units dummy weapons, which need no muzzle.)
                    self.issue(Severity::Info, "script.weapon", path, format!("{} has Weapon{} but its script {script} has no {name}: if the weapon fires, its shots leave from the unit's centre, not a barrel.", unit.name, slot + 1), "Add the function to the unit's BOS script, returning the piece the shots leave from, and recompile.");
                }
            }
            if unit.builder && !functions.contains("querynanopiece") {
                self.issue(Severity::Info, "script.builder", path, format!("{} builds but its script {script} has no QueryNanoPiece: its build spray comes from the unit's centre.", unit.name), "Add QueryNanoPiece to the unit's BOS script, returning its nano-lathe piece, and recompile.");
            }
        }
        if self.buildable.contains(record_key) && !self.vfs.exists(&format!("unitpics/{}.pcx", unit.name.to_ascii_lowercase())) {
            self.issue(Severity::Info, "unit.picture", path, format!("{} can be built but has no picture (unitpics/{}.pcx): TA's classic panels and the encyclopaedia show none; Reborn's own panel draws one from its model.", unit.name, unit.name.to_ascii_lowercase()), "Add a 64 × 64 PCX of the unit to unitpics/.");
        }
    }

    /// Whose doing each finding is, and whether it counts. The target's files and what
    /// they use (along references), launch-critical errors and global limits count; the rest
    /// is inherited context until [`compare`] finds the mod caused it. What they use is told
    /// definition by definition: of a file of many (weapons, features, sound categories,
    /// movement classes), what is wrong with the ones used counts, not every other in it.
    fn finish_scope(&mut self) {
        if self.options.target_layer.is_none() {
            for issue in &mut self.report.issues { issue.origin = Origin::Mod; issue.affects_status = true; }
            return;
        }
        // Every definition in the target's files, then what they use, however indirectly: a
        // file used whole (a model, a sound, a map's terrain) brings its own definition (a
        // map's features), not the others in it (a GAF's other animations).
        let mut used: BTreeSet<String> = BTreeSet::new();
        let mut queue: VecDeque<String> = VecDeque::new();
        for path in &self.relevant {
            for key in self.file_records.get(path).into_iter().flatten() {
                if used.insert(key.clone()) { queue.push_back(key.clone()); }
            }
        }
        while let Some(record) = queue.pop_front() {
            for target in self.uses.get(&record).into_iter().flatten() {
                let next = match target {
                    Use::File(path) => { self.relevant.insert(path.clone()); key("file", path) }
                    Use::Record(key) => key.clone(),
                };
                if self.record_paths.contains_key(&next) && used.insert(next.clone()) { queue.push_back(next); }
            }
        }
        // (A finding about a file as a whole, its syntax, counts where any of it is used.)
        let touched: BTreeSet<&str> = used.iter().filter_map(|k| self.record_paths.get(k)).map(String::as_str).collect();
        for (i, issue) in self.report.issues.iter_mut().enumerate() {
            let concerned = match self.attributed.get(&i) {
                // (A finding about a definition another file's replaces is about one the game
                // never uses: the mod overriding it is not charged with it.)
                Some(record) => used.contains(record) && self.record_paths.get(record).is_none_or(|winner| *winner == issue.path),
                None => self.relevant.contains(&issue.path) || touched.contains(issue.path.as_str()),
            };
            let counts = issue.code.starts_with("validation.") || issue.code.starts_with("packaging.") || issue.path.is_empty() || issue.code.starts_with("render.texture_") || concerned || self.options.target_layer.as_ref() == Some(&issue.layer) || (issue.severity == Severity::Error && self.required.contains(&issue.path));
            issue.affects_status = counts;
            issue.origin = if counts { Origin::Mod } else { Origin::Inherited };
        }
    }
}

fn optional(refs: &mut Vec<Reference>, kind: &str, name: &str, what: impl Into<String>, severity: Severity) {
    if !name.trim().is_empty() { refs.push(Reference::new(kind, name, what, severity)); }
}

fn animation_ref(refs: &mut Vec<Reference>, gaf: &str, entry: &str, what: String) {
    if gaf.trim().is_empty() || entry.trim().is_empty() { return; }
    let path = asset("anims", gaf, "gaf");
    // The file edge survives failed parsing, making inherited parse errors relevant.
    refs.push(Reference::new("file", &path, what.clone(), Severity::Warning));
    refs.push(Reference::new("gaf", &format!("{path}#{entry}"), what, Severity::Warning));
}

/// What a reference leads to, where it is there.
fn resolve(reference: &str, vfs: &Vfs, records: &BTreeMap<String, Record>) -> Option<Use> {
    if let Some(path) = reference.strip_prefix("file:") { return vfs.exists(path).then(|| Use::File(path.to_string())); }
    if let Some(name) = reference.strip_prefix("sound:") { let path = sound_path(name); return vfs.exists(&path).then_some(Use::File(path)); }
    if let Some(name) = reference.strip_prefix("placed:") { return [key("unit", name), key("feature", name)].into_iter().find(|k| records.contains_key(k)).map(Use::Record); }
    records.contains_key(reference).then(|| Use::Record(reference.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("ta_validation_{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn put(&self, layer: &str, path: &str, bytes: impl AsRef<[u8]>) {
            let path = self.0.join(layer).join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        fn valid(&self, layer: &str) {
            self.put(layer, "units/example.fbi", b"[UNITINFO]{UnitName=EXAMPLE;ObjectName=example;Side=ARM;Builder=1;}");
            self.put(layer, "gamedata/sidedata.tdf", b"[SIDE0]{name=ARM;commander=EXAMPLE;}");
            self.put(layer, "objects3d/example.3do", model("base"));
            self.put(layer, "scripts/example.cob", script("base", &[cob::op::RETURN]));
        }
        fn vfs(&self, layers: &[&str]) -> Vfs {
            let mut vfs = Vfs::new();
            for layer in layers { vfs.mount_mod_dir(&self.0.join(layer), layer).unwrap(); }
            vfs
        }
    }
    impl Drop for Fixture { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }

    fn word(bytes: &mut [u8], at: usize, n: u32) { bytes[at..at + 4].copy_from_slice(&n.to_le_bytes()); }
    /// A one-piece model with a triangle painted with each of `textures`.
    fn textured_model(textures: &[&str]) -> Vec<u8> {
        let mut b = vec![0; 52];
        word(&mut b, 0, 1);
        word(&mut b, 4, 3);
        word(&mut b, 8, textures.len() as u32);
        word(&mut b, 12, u32::MAX);
        word(&mut b, 28, 52);
        b.extend_from_slice(b"base\0");
        let vertices = b.len();
        word(&mut b, 36, vertices as u32);
        b.resize(vertices + 36, 0);
        word(&mut b, vertices + 12, 65536);
        word(&mut b, vertices + 28, 65536);
        let indices = b.len();
        b.extend_from_slice(&[0, 0, 1, 0, 2, 0]);
        let primitives = b.len();
        word(&mut b, 40, primitives as u32);
        b.resize(primitives + 32 * textures.len(), 0);
        for (i, texture) in textures.iter().enumerate() {
            let at = primitives + 32 * i;
            word(&mut b, at + 4, 3);
            word(&mut b, at + 12, indices as u32);
            let name = b.len() as u32;
            word(&mut b, at + 16, name);
            b.extend_from_slice(texture.as_bytes());
            b.push(0);
        }
        b
    }
    fn model(piece: &str) -> Vec<u8> {
        let mut b = vec![0; 52];
        word(&mut b, 0, 1);
        word(&mut b, 12, u32::MAX);
        word(&mut b, 28, 52);
        b.extend_from_slice(piece.as_bytes()); b.push(0);
        b
    }
    /// A script with `Create` and `QueryNanoPiece` (every fixture unit is a builder).
    fn script(piece: &str, code: &[u32]) -> Vec<u8> { script_with(piece, &["Create", "QueryNanoPiece"], code) }
    /// A script of one piece and these functions, each starting at the code's first word.
    fn script_with(piece: &str, functions: &[&str], code: &[u32]) -> Vec<u8> {
        let n = functions.len();
        let (offsets, names, pieces) = (44, 44 + n * 4, 44 + n * 8);
        let code_at = pieces + 4;
        let mut b = vec![0; code_at + code.len() * 4];
        word(&mut b, 0, 4); word(&mut b, 4, n as u32); word(&mut b, 8, 1);
        word(&mut b, 12, code.len() as u32);
        word(&mut b, 24, offsets as u32); word(&mut b, 28, names as u32); word(&mut b, 32, pieces as u32); word(&mut b, 36, code_at as u32);
        for (i, &w) in code.iter().enumerate() { word(&mut b, code_at + i * 4, w); }
        for (i, name) in functions.iter().enumerate() {
            let at = b.len() as u32;
            word(&mut b, names + i * 4, at); b.extend_from_slice(name.as_bytes()); b.push(0);
        }
        let at = b.len() as u32;
        word(&mut b, pieces, at); b.extend_from_slice(piece.as_bytes()); b.push(0);
        b
    }
    fn gaf(entries: &[(&str, usize, u16, u16)]) -> Vec<u8> {
        let mut b = vec![0; 12 + entries.len() * 4];
        word(&mut b, 0, 0x0001_0100); word(&mut b, 4, entries.len() as u32);
        for (i, &(name, frames, width, height)) in entries.iter().enumerate() {
            let entry = b.len();
            word(&mut b, 12 + i * 4, entry as u32);
            b.resize(entry + 40 + frames * 8, 0);
            b[entry..entry + 2].copy_from_slice(&(frames as u16).to_le_bytes());
            b[entry + 8..entry + 8 + name.len()].copy_from_slice(name.as_bytes());
            let frame = b.len();
            b.resize(frame + 24, 0);
            b[frame..frame + 2].copy_from_slice(&width.to_le_bytes());
            b[frame + 2..frame + 4].copy_from_slice(&height.to_le_bytes());
            word(&mut b, frame + 16, (frame + 24) as u32);
            b.resize(frame + 24 + width as usize * height as usize, 1);
            for n in 0..frames { word(&mut b, entry + 40 + n * 8, frame as u32); }
        }
        b
    }

    #[test]
    fn valid_pack_is_clean_but_report_explicitly_limits_its_claim() {
        let f = Fixture::new(); f.valid("mod");
        let report = validate_all(&f.vfs(&["mod"]));
        assert_eq!(report.status(), Status::Clean, "{}", report.to_text());
        assert!(report.complete);
        assert_eq!((report.counts.files_total, report.counts.files_checked, report.counts.units, report.counts.models, report.counts.scripts), (4, 4, 1, 1, 1));
        assert!(report.to_text().contains("cannot prove gameplay"));
    }

    #[test]
    fn complete_reference_report_keeps_every_issue_and_copy_contains_the_last() {
        let f = Fixture::new();
        for n in 0..20 {
            f.put("mod", &format!("units/u{n:02}.fbi"), format!("[UNITINFO]{{UnitName=U{n:02};CanMove=1;MovementClass=MISSING;Weapon1=NO_WEAPON;Corpse=NO_CORPSE;}}"));
        }
        let report = validate_all(&f.vfs(&["mod"]));
        assert!(report.complete);
        assert_eq!(report.counts.units, 20);
        assert_eq!(report.issues.iter().filter(|i| i.code == "reference.missing").count(), 100);
        assert_eq!(report.status(), Status::Errors);
        assert!(report.to_text().contains("U19 wreckage"));
        assert!(report.issues.iter().all(|i| !i.suggestion.is_empty()));
    }

    #[test]
    fn winner_precedence_and_duplicate_definition_rules_match_game_data() {
        let f = Fixture::new(); f.valid("base"); f.valid("mod");
        f.put("base", "objects3d/example.3do", b"invalid old model");
        f.put("mod", "units/a_shadowed.fbi", b"[UNITINFO]{UnitName=EXAMPLE;ObjectName=missing;Weapon1=missing;}");
        let report = validate_all(&f.vfs(&["base", "mod"]));
        assert_eq!(report.counts.units, 1);
        assert!(!report.to_text().contains("invalid old model"));
        // The shadowed copy's broken references are not the game's, but the copy is noted.
        assert_eq!(report.issues.len(), 1, "{}", report.to_text());
        assert_eq!(report.issues[0].code, "definition.duplicate");
        assert!(report.issues[0].message.contains("unit EXAMPLE is also defined in units/a_shadowed.fbi"), "{}", report.issues[0].message);
    }

    /// The music folder's WAV tracks (the game plays them) and sounds named with a folder are
    /// where the game reads them.
    #[test]
    fn music_and_sounds_in_folders_are_where_the_game_reads_them() {
        assert_eq!(misplaced("music/track01.wav"), None);
        assert_eq!(misplaced("sounds/voices/hello.wav"), None);
        assert_eq!(misplaced("sounds/boom.wav"), None);
        assert_eq!(misplaced("units/boom.wav"), Some("sounds/"));
        assert_eq!(misplaced("music/extra/track01.wav"), Some("sounds/"), "the music folder's own tracks only");
        // (Four samples of 8-bit PCM.)
        let mut wav = b"RIFF".to_vec();
        for (tag, bytes) in [(&b""[..], 40u32.to_le_bytes().to_vec()), (b"WAVEfmt ", 16u32.to_le_bytes().to_vec())] { wav.extend_from_slice(tag); wav.extend(bytes); }
        for word in [1u16, 1] { wav.extend(word.to_le_bytes()); }
        for rate in [22050u32, 22050] { wav.extend(rate.to_le_bytes()); }
        for word in [1u16, 8] { wav.extend(word.to_le_bytes()); }
        wav.extend_from_slice(b"data"); wav.extend(4u32.to_le_bytes()); wav.extend([128u8; 4]);
        assert_eq!(wav_problem(&wav), None, "a WAV the game plays");
        let f = Fixture::new(); f.valid("base");
        f.put("music", "music/track01.wav", &wav);
        let options = Options { target_layer: Some("music".into()), ..Default::default() };
        let report = validate(&f.vfs(&["base", "music"]), &options, |_| true);
        assert!(!report.issues.iter().any(|i| i.code == "packaging.misplaced"), "{}", report.to_text());
    }

    /// A mod that replaces a definition of the game's (a patch's fixed weapon) is not charged with
    /// the replaced one's findings: the game never uses it.
    #[test]
    fn a_definition_replaced_by_the_mod_does_not_charge_it_with_the_old_ones_findings() {
        let f = Fixture::new(); f.valid("base");
        f.put("base", "weapons/a.tdf", b"[GUN]{RenderType=9;}");
        f.put("fix", "weapons/zz_fix.tdf", b"[GUN]{RenderType=0;}");
        let options = Options { target_layer: Some("fix".into()), ..Default::default() };
        let report = validate(&f.vfs(&["base", "fix"]), &options, |_| true);
        assert_eq!(report.status(), Status::Clean, "{}", report.to_text());
        assert!(report.issues.iter().any(|i| i.code == "weapon.render_type" && !i.affects_status && i.origin == Origin::Inherited), "{}", report.to_text());
        // Its own broken definition still counts.
        f.put("fix", "weapons/zz_fix.tdf", b"[GUN]{RenderType=8;}");
        let report = validate(&f.vfs(&["base", "fix"]), &options, |_| true);
        assert!(report.issues.iter().any(|i| i.code == "weapon.render_type" && i.path == "weapons/zz_fix.tdf" && i.affects_status), "{}", report.to_text());
    }

    /// TA runs `scripts/<UnitName>.cob` whatever model a unit borrows (TA: Enhanced's units
    /// with another's model and scripts under their own names only): no script is missing, and
    /// the script checked against the model is the unit's own, else its model's.
    #[test]
    fn a_unit_with_a_borrowed_model_is_checked_with_its_own_script() {
        let f = Fixture::new(); f.valid("base");
        f.put("mod", "units/gunner.fbi", b"[UNITINFO]{UnitName=GUNNER;ObjectName=example;Side=ARM;}");
        f.put("mod", "scripts/gunner.cob", script("missing_turret", &[cob::op::RETURN]));
        let options = Options { target_layer: Some("mod".into()), ..Default::default() };
        let report = validate(&f.vfs(&["base", "mod"]), &options, |_| true);
        assert!(!report.issues.iter().any(|i| i.code == "reference.missing"), "{}", report.to_text());
        assert!(report.issues.iter().any(|i| i.code == "script.model_piece" && i.message.starts_with("scripts/gunner.cob declares piece missing_turret")), "{}", report.to_text());
        std::fs::remove_file(f.0.join("mod/scripts/gunner.cob")).unwrap();
        let report = validate(&f.vfs(&["base", "mod"]), &options, |_| true);
        assert!(!report.issues.iter().any(|i| i.code == "reference.missing" || i.code == "script.model_piece"), "its model's script, which fits: {}", report.to_text());
        f.put("mod", "units/gunner.fbi", b"[UNITINFO]{UnitName=GUNNER;ObjectName=nowhere;Side=ARM;}");
        let report = validate(&f.vfs(&["base", "mod"]), &options, |_| true);
        assert!(report.issues.iter().any(|i| i.code == "reference.missing" && i.message.contains("scripts/gunner.cob")), "a missing script is reported as its own: {}", report.to_text());
    }

    #[test]
    fn inherited_failures_are_context_until_referenced_by_the_mod() {
        let f = Fixture::new(); f.valid("mod");
        f.put("base", "unitpics/unused.pcx", b"invalid image");
        f.put("base", "objects3d/borrowed.3do", b"invalid model");
        f.put("base", "scripts/borrowed.cob", script("base", &[cob::op::RETURN]));
        let options = Options { target_layer: Some("mod".into()), ..Default::default() };
        let report = validate(&f.vfs(&["base", "mod"]), &options, |_| true);
        assert_eq!(report.status(), Status::Clean, "{}", report.to_text());
        assert!(report.issues.iter().all(|i| !i.affects_status && i.layer == "base"));
        f.put("mod", "units/example.fbi", b"[UNITINFO]{UnitName=EXAMPLE;ObjectName=borrowed;Side=ARM;}");
        let report = validate(&f.vfs(&["base", "mod"]), &options, |_| true);
        assert_eq!(report.status(), Status::Errors);
        assert!(report.issues.iter().any(|i| i.path == "objects3d/borrowed.3do" && i.affects_status));
        assert!(report.issues.iter().any(|i| i.path == "unitpics/unused.pcx" && !i.affects_status));
    }

    #[test]
    fn weapon_only_and_new_unit_mods_do_not_inherit_unrelated_base_build_menu_warnings() {
        let f = Fixture::new(); f.valid("base");
        f.put("base", "units/aircraft.fbi", b"[UNITINFO]{UnitName=AIRCRAFT;ObjectName=example;Side=ARM;ExplodeAs=MEDIUM_UNITEX;}");
        f.put("base", "gamedata/sidedata.tdf", b"[SIDE0]{name=ARM;commander=EXAMPLE;} [CANBUILD]{[EXAMPLE]{canbuild1=AIRCRAFT;}}");
        f.put("mod", "weapons/qa.tdf", b"[QA_LASER]{RenderType=0;}");
        let options = Options { target_layer: Some("mod".into()), ..Default::default() };
        let report = validate(&f.vfs(&["base", "mod"]), &options, |_| true);
        assert_eq!(report.status(), Status::Clean, "{}", report.to_text());
        assert!(report.issues.iter().any(|i| i.path == "units/aircraft.fbi" && i.message.contains("MEDIUM_UNITEX") && !i.affects_status && i.origin == Origin::Inherited));

        // Merely naming the original faction must not turn its entire CANBUILD table
        // into a dependency of this new unit either.
        f.put("mod", "units/new.fbi", b"[UNITINFO]{UnitName=NEW;ObjectName=example;Side=ARM;}");
        let report = validate(&f.vfs(&["base", "mod"]), &options, |_| true);
        assert_eq!(report.status(), Status::Clean, "{}", report.to_text());
        assert!(report.issues.iter().any(|i| i.path == "units/aircraft.fbi" && !i.affects_status));
    }

    #[test]
    fn missing_inherited_commanders_and_unreadable_commander_assets_remain_launch_errors() {
        let f = Fixture::new(); f.valid("base");
        f.put("mod", "weapons/qa.tdf", b"[QA_LASER]{RenderType=0;}");
        let options = Options { target_layer: Some("mod".into()), ..Default::default() };
        f.put("base", "gamedata/sidedata.tdf", b"[SIDE0]{name=ARM;commander=MISSING;}");
        let report = validate(&f.vfs(&["base", "mod"]), &options, |_| true);
        assert_eq!(report.status(), Status::Errors);
        assert!(report.issues.iter().any(|i| i.path == "gamedata/sidedata.tdf" && i.message.contains("starting commander") && i.affects_status));

        f.put("base", "gamedata/sidedata.tdf", b"[SIDE0]{name=ARM;commander=EXAMPLE;}");
        f.put("base", "scripts/example.cob", b"unreadable commander animation");
        let report = validate(&f.vfs(&["base", "mod"]), &options, |_| true);
        assert_eq!(report.status(), Status::Errors);
        assert!(report.issues.iter().any(|i| i.path == "scripts/example.cob" && i.code == "format.invalid" && i.affects_status));
    }

    #[test]
    fn cancellation_read_failures_and_each_budget_never_award_a_clean_badge() {
        let f = Fixture::new(); f.valid("mod");
        let vfs = f.vfs(&["mod"]);
        let cancelled = validate(&vfs, &Options::default(), |p| p.checked < 1);
        assert_eq!(cancelled.status(), Status::Incomplete);
        assert_eq!(cancelled.counts.files_checked, 1);
        assert!(cancelled.issues.iter().any(|i| i.code == "validation.cancelled"));
        for options in [Options { max_files: 1, ..Default::default() }, Options { max_text_bytes: 1, ..Default::default() }, Options { max_read_bytes: 1, ..Default::default() }, Options { max_retained_bytes: 1, ..Default::default() }] {
            let report = validate(&vfs, &options, |_| true);
            assert_eq!(report.status(), Status::Incomplete, "{}", report.to_text());
            assert!(report.issues.iter().any(|i| i.code == "validation.limit"));
        }
        std::fs::remove_file(f.0.join("mod/scripts/example.cob")).unwrap();
        let report = validate_all(&vfs);
        assert_eq!(report.status(), Status::Errors);
        assert_eq!(report.counts.files_failed, 1);
        assert!(report.issues.iter().any(|i| i.code == "file.read"));
        assert_eq!(validate_all(&Vfs::new()).status(), Status::Incomplete);
    }

    #[test]
    fn cob_operand_values_are_not_mistaken_for_opcodes_and_model_pieces_are_checked() {
        let f = Fixture::new(); f.valid("mod");
        f.put("mod", "scripts/example.cob", script("base", &[cob::op::PUSH_CONSTANT, 0xdeadbeef, cob::op::RETURN]));
        assert_eq!(validate_all(&f.vfs(&["mod"])).status(), Status::Clean);
        f.put("mod", "scripts/example.cob", script("missing_turret", &[0xdeadbeef, 0xabcdef, cob::op::RETURN]));
        let report = validate_all(&f.vfs(&["mod"]));
        assert_eq!(report.issues.iter().filter(|i| i.code == "script.opcode").count(), 1, "Do not invent opcodes from an unknown instruction's operands");
        assert!(report.issues.iter().any(|i| i.code == "script.model_piece" && i.message.contains("missing_turret")));
        f.put("mod", "scripts/example.cob", script("base", &[cob::op::MOVE, 2, 0, cob::op::RETURN]));
        assert!(validate_all(&f.vfs(&["mod"])).issues.iter().any(|i| i.code == "script.operand_range"));
        f.put("mod", "scripts/example.cob", script("base", &[cob::op::PUSH_CONSTANT]));
        assert!(validate_all(&f.vfs(&["mod"])).issues.iter().any(|i| i.code == "script.operands"));
        f.put("mod", "scripts/example.cob", script("base", &[cob::op::JUMP_IF_FALSE, 3, cob::op::RETURN]));
        assert_eq!(validate_all(&f.vfs(&["mod"])).status(), Status::Clean, "A final BOS label at the end of code legitimately terminates the thread");
        f.put("mod", "scripts/example.cob", script("base", &[cob::op::JUMP_IF_FALSE, 4, cob::op::RETURN]));
        assert!(validate_all(&f.vfs(&["mod"])).issues.iter().any(|i| i.code == "script.operand_range"));
    }

    /// Scriptor's compiler ("COB BLER Ver 4.0", TA: Enhanced's and Total Mayhem's scripts) leaves
    /// its signature in the code after a function's last `return`: the VM never runs it, so it is
    /// no unknown opcode. What a function does reach still is.
    #[test]
    fn words_no_function_reaches_are_not_decoded() {
        use cob::op::*;
        // The first eight bytes of "Build by COB BLER": data, not code.
        let (b, u) = (0x6c69_7542, 0x7962_2064);
        let at = |functions: &[(&str, u32)], code: &[u32]| {
            let mut bytes = script_with("base", &functions.iter().map(|(name, _)| *name).collect::<Vec<_>>(), code);
            for (i, (_, offset)) in functions.iter().enumerate() { word(&mut bytes, 44 + i * 4, *offset); }
            bytes
        };
        let f = Fixture::new(); f.valid("mod");
        // Create returns at word 2 (after a loop back to its start, and a branch over a jump),
        // the signature lies at 3 and 4, and QueryNanoPiece starts at 5.
        f.put("mod", "scripts/example.cob", at(&[("Create", 0), ("QueryNanoPiece", 5)], &[PUSH_CONSTANT, 0, RETURN, b, u, RETURN]));
        assert_eq!(validate_all(&f.vfs(&["mod"])).status(), Status::Clean, "the signature after a return is never run");
        let looping = [PUSH_CONSTANT, 1, JUMP_IF_FALSE, 7, JUMP, 0, b, RETURN, u, RETURN];
        f.put("mod", "scripts/example.cob", at(&[("Create", 0), ("QueryNanoPiece", 9)], &looping));
        assert_eq!(validate_all(&f.vfs(&["mod"])).status(), Status::Clean, "jumped over, and after the return a branch leads to");
        // Where a branch does lead, an unknown word is an error, once.
        let reached = [PUSH_CONSTANT, 1, JUMP_IF_FALSE, 6, RETURN, RETURN, b, u, RETURN];
        f.put("mod", "scripts/example.cob", at(&[("Create", 0), ("QueryNanoPiece", 8)], &reached));
        let report = validate_all(&f.vfs(&["mod"]));
        let opcodes: Vec<_> = report.issues.iter().filter(|i| i.code == "script.opcode").collect();
        assert_eq!(opcodes.len(), 1, "{}", report.to_text());
        assert!(opcodes[0].message.contains("at word 6"), "{}", opcodes[0].message);
        // A function that runs on into the next is checked to its entry, and from it.
        f.put("mod", "scripts/example.cob", at(&[("Create", 0), ("QueryNanoPiece", 2)], &[PUSH_CONSTANT, 0, b, RETURN]));
        assert!(validate_all(&f.vfs(&["mod"])).issues.iter().any(|i| i.code == "script.opcode" && i.message.contains("at word 2")));
    }

    #[test]
    fn production_navigation_is_not_a_missing_unit_but_missing_build_options_are_reported() {
        let f = Fixture::new(); f.valid("mod");
        f.put("mod", "guis/example1.gui", b"[A]{[COMMON]{id=1;name=ARMNEXT;attribs=2;}} [B]{[COMMON]{id=1;name=ARMMISSING;attribs=32;commonattribs=4;}} [C]{[COMMON]{id=1;name=ARMATTACK;attribs=64;}} [D]{[COMMON]{id=1;name=ARMMAKENUKE;attribs=32;commonattribs=8;}} [E]{[COMMON]{id=1;name=IGPATCH;attribs=32;commonattribs=0;}} [F]{[COMMON]{id=1;name=IGPATCH;attribs=32;commonattribs=4;}} [G]{[COMMON]{id=1;name=IGPATCH2;attribs=32;commonattribs=4;}}");
        f.put("mod", "download/add.tdf", b"[MENUENTRY0]{UNITMENU=EXAMPLE;UNITNAME=MISSING_DOWNLOAD;} [MENUENTRY1]{UNITMENU=BLANK;UNITNAME=DISABLED_UNIT;} [MENUENTRY2]{UNITMENU=EXAMPLE;UNITNAME=MISSING_DOWNLOAD;}");
        let report = validate_all(&f.vfs(&["mod"]));
        let text = report.to_text();
        assert!(text.contains("ARMMISSING") && text.contains("MISSING_DOWNLOAD"), "{text}");
        assert!(!text.contains("ARMNEXT") && !text.contains("ARMATTACK"));
        assert!(!text.contains("ARMMAKENUKE") && !text.contains("IGPATCH") && !text.contains("unit BLANK") && !text.contains("DISABLED_UNIT"));
        assert_eq!(report.issues.iter().filter(|i| i.message.contains("MISSING_DOWNLOAD")).count(), 1, "Repeated AI/menu entries do not duplicate one diagnostic");
        assert_eq!(report.status(), Status::Issues);
    }

    /// One entry of `frames` frames, each the same compressed `side` x `side` square of one
    /// colour (its rows runs of 64): 32 MB of pixels a frame decoded, half a megabyte on disk.
    fn big_gaf(name: &str, frames: usize, side: u16) -> Vec<u8> {
        let mut b = gaf(&[(name, frames, 1, 1)]);
        // (The entry at 16, its first frame's pointer 40 bytes on.)
        let frame = u32::from_le_bytes(b[56..60].try_into().unwrap()) as usize;
        b[frame..frame + 2].copy_from_slice(&side.to_le_bytes());
        b[frame + 2..frame + 4].copy_from_slice(&side.to_le_bytes());
        b[frame + 9] = 1;
        let rows = b.len();
        word(&mut b, frame + 16, rows as u32);
        let mut row = Vec::new();
        let mut left = side as usize;
        while left > 0 {
            let n = left.min(64);
            row.extend_from_slice(&[(((n - 1) as u8) << 2) | 2, 5]);
            left -= n;
        }
        for _ in 0..side {
            b.extend_from_slice(&(row.len() as u16).to_le_bytes());
            b.extend_from_slice(&row);
        }
        b
    }

    /// A weapon's explosion painted too big to decode at once (TA: Escalation's are up to
    /// 3240 x 3240): the game reads it a frame at a time and draws it reduced, which the report
    /// notes, and its animation is there; a feature using such a file finds it unreadable.
    #[test]
    fn explosions_too_big_to_decode_at_once_are_drawn_reduced() {
        let f = Fixture::new(); f.valid("mod");
        f.put("mod", "anims/boom.gaf", big_gaf("boom", 9, 4000));
        f.put("mod", "weapons/test.tdf", b"[BIG]{explosiongaf=boom;explosionart=boom;}");
        let report = validate_all(&f.vfs(&["mod"]));
        let text = report.to_text();
        let note = report.issues.iter().find(|i| i.code == "render.sprite_large").unwrap_or_else(|| panic!("{text}"));
        assert!(note.severity == Severity::Info && note.path == "anims/boom.gaf", "{text}");
        assert!(note.message.starts_with("Large: 9 frames up to 4000 pixels across, 144 million pixels"), "{text}");
        assert!(!report.issues.iter().any(|i| matches!(i.code.as_str(), "format.invalid" | "reference.missing" | "render.sprite_capacity")), "{text}");
        assert_eq!(report.status(), Status::Clean, "{text}");
        f.put("mod", "features/green/rocks.tdf", b"[BOOMROCK]{filename=boom;seqname=boom;}");
        let report = validate_all(&f.vfs(&["mod"]));
        let error = report.issues.iter().find(|i| i.path == "anims/boom.gaf").unwrap();
        assert!(error.code == "format.invalid" && error.severity == Severity::Error && error.message.contains("exceeds resource limit"), "{}", report.to_text());
    }

    /// Explosions taking more of the atlas than the game gives them are drawn reduced, as the
    /// game draws them: one note, no warning (TA's own sizes and under are as painted).
    #[test]
    fn explosions_over_their_budget_are_planned_as_the_game_draws_them() {
        let f = Fixture::new(); f.valid("mod");
        f.put("mod", "anims/fx.gaf", gaf(&[("small", 4, 100, 60), ("big", 6, 300, 300), ("bigger", 3, 600, 500)]));
        f.put("mod", "weapons/test.tdf", b"[ONE]{explosiongaf=fx;explosionart=small;}[TWO]{explosiongaf=fx;explosionart=big;}[THREE]{explosiongaf=fx;explosionart=bigger;}");
        // A 1024 atlas: half a million texels for explosions, 1.5 million painted.
        let report = validate(&f.vfs(&["mod"]), &Options { sprite_atlas_side: 1024, ..Default::default() }, |_| true);
        let text = report.to_text();
        let note = report.issues.iter().find(|i| i.code == "render.sprite_reduced").unwrap_or_else(|| panic!("{text}"));
        assert!(note.severity == Severity::Info && note.message.contains("2 of them") && note.message.contains("BIGGER (600 px), BIG (300 px)"), "{text}");
        assert!(!report.issues.iter().any(|i| matches!(i.code.as_str(), "render.sprite_capacity" | "render.sprite_frames")), "{text}");
        // Within the reference atlas's budget, nothing is reduced.
        let report = validate_all(&f.vfs(&["mod"]));
        assert!(!report.issues.iter().any(|i| i.code.starts_with("render.")), "{}", report.to_text());
    }

    #[test]
    fn explicit_empty_shadow_marker_is_not_a_missing_animation() {
        let f = Fixture::new();
        f.put("mod", "features/green/shrubs.tdf", b"[SHRUB]{filename=plants;seqname=shrub;seqnameshad=EMPTY;}");
        f.put("mod", "anims/plants.gaf", gaf(&[("shrub", 1, 1, 1)]));
        assert_eq!(validate_all(&f.vfs(&["mod"])).status(), Status::Clean);
    }

    /// As the game loads them: the textures the models paint with (an entry no model names
    /// takes no room), the first entry of a name, through two arrays, a layer kept for the
    /// fallback.
    #[test]
    fn texture_capacity_matches_first_entry_wins_and_reserves_fallback() {
        let f = Fixture::new(); f.valid("mod");
        f.put("mod", "objects3d/painted.3do", textured_model(&["first", "Third", "too_many", "fourth", "fifth"]));
        f.put("mod", "textures/a.gaf", gaf(&[("first", 2, 1, 1), ("unused", 3, 1, 1)]));
        f.put("mod", "textures/b.gaf", gaf(&[("FIRST", 250, 1, 1), ("third", 1, 1, 1), ("too_many", 256, 1, 1), ("fourth", 1, 1, 1), ("fifth", 1, 1, 1)]));
        // (Two arrays of two layers: first's two frames, third's one and the fallback.)
        let report = validate(&f.vfs(&["mod"]), &Options { texture_array_layers: 2, ..Default::default() }, |_| true);
        assert_eq!(report.counts.textures, 6);
        let capacity: Vec<_> = report.issues.iter().filter(|i| i.code == "render.texture_layers").collect();
        assert_eq!(capacity.len(), 2, "{}", report.to_text());
        assert!(["fourth", "fifth"].iter().all(|name| capacity.iter().any(|i| i.message.contains(&format!("Texture {name} ")))), "{}", report.to_text());
        assert!(!report.issues.iter().any(|i| i.message.contains("unused")), "an entry no model names takes no room: {}", report.to_text());
        assert!(report.issues.iter().any(|i| i.code == "render.texture_frames" && i.message.contains("too_many")));
    }

    #[test]
    fn sprite_capacity_counts_referenced_effects_once_and_ignores_unused_entries() {
        let f = Fixture::new(); f.valid("mod");
        f.put("mod", "anims/fx.gaf", gaf(&[("small", 1, 1, 1), ("unused", 1, 20, 20), ("big", 2, 7, 7)]));
        f.put("mod", "weapons/test.tdf", b"[ONE]{explosiongaf=fx;explosionart=small;}[TWO]{explosiongaf=fx;explosionart=small;}");
        let options = Options { sprite_atlas_side: 10, ..Default::default() };
        let report = validate(&f.vfs(&["mod"]), &options, |_| true);
        assert!(!report.issues.iter().any(|i| i.code.starts_with("render.sprite")), "{}", report.to_text());
        f.put("mod", "weapons/test.tdf", b"[ONE]{explosiongaf=fx;explosionart=big;}[TWO]{explosiongaf=fx;explosionart=big;}");
        let report = validate(&f.vfs(&["mod"]), &options, |_| true);
        let issue = report.issues.iter().find(|i| i.code == "render.sprite_capacity").unwrap();
        assert!(issue.message.contains("162 padded"), "{}", issue.message);
    }

    #[test]
    fn map_features_and_missing_terrain_are_reported() {
        let f = Fixture::new();
        let map = ta_formats::tnt::TntMap { width: 2, height: 2, sea_level: 0, tiles: vec![Box::new([0; 1024])], tile_map: vec![0], heights: vec![0; 4], features: vec![0, 65535, 65535, 65535], feature_names: vec!["missing_tree".into()], minimap: ta_formats::image::IndexedImage::new(1, 1, 0) };
        f.put("mod", "maps/test.tnt", map.write().unwrap());
        f.put("mod", "maps/test.ota", b"[GlobalHeader]{missionname=Test;}");
        f.put("mod", "maps/orphan.ota", b"[GlobalHeader]{missionname=Orphan;}");
        let report = validate_all(&f.vfs(&["mod"]));
        assert_eq!(report.counts.maps, 1);
        assert!(report.issues.iter().any(|i| i.message.contains("feature MISSING_TREE")));
        assert!(report.issues.iter().any(|i| i.message.contains("the file maps/orphan.tnt") && i.severity == Severity::Error));
    }

    /// TA's way of taking one of the original game's files out is an empty file over it (Total
    /// Mayhem blanks 57 of its animations so): a blank, a note a folder, never a broken file.
    #[test]
    fn empty_files_are_blanks_told_a_folder_at_a_time() {
        let f = Fixture::new(); f.valid("base");
        f.put("base", "anims/armaac1.gaf", gaf(&[("armaac1", 1, 2, 2)]));
        for name in ["armaac1", "armaac2", "armaca1", "armack1", "armacs1"] {
            f.put("mod", &format!("anims/{name}.gaf"), b"");
        }
        f.put("mod", "units/gone.fbi", b"");
        let options = Options { target_layer: Some("mod".into()), ..Default::default() };
        let mut report = validate(&f.vfs(&["base", "mod"]), &options, |_| true);
        compare(&mut report, &validate_all(&f.vfs(&["base"])));
        assert_eq!(report.status(), Status::Clean, "{}", report.to_text());
        assert_eq!(report.counts.files_failed, 0);
        assert!(!report.issues.iter().any(|i| i.code == "format.invalid" || i.severity != Severity::Info), "{}", report.to_text());
        let blanks: Vec<_> = report.issues.iter().filter(|i| i.code == "file.blank").collect();
        assert_eq!(blanks.len(), 2, "a note for anims/ and one for units/: {}", report.to_text());
        let anims = blanks.iter().find(|i| i.path.starts_with("anims/")).unwrap();
        assert!(anims.message.starts_with("5 empty files here (anims/armaac1.gaf, anims/armaac2.gaf, anims/armaca1.gaf and 2 more)"), "{}", anims.message);
        assert!(blanks.iter().any(|i| i.path == "units/gone.fbi" && i.message.starts_with("This file is empty")));
    }

    /// A mod's file that does not read just as the original game's of that name does not (a copy
    /// of TA's own `anims/vismasks.gaf`, which is no GAF) is no fault of the mod's; one broken in a
    /// way of its own still is.
    #[test]
    fn a_mods_copy_of_an_unreadable_original_file_is_inherited() {
        let f = Fixture::new(); f.valid("base");
        f.put("base", "anims/vismasks.gaf", [0u8, 0, 0, 0, 1, 0, 0, 0, 9, 9, 9, 9]);
        f.put("mod", "anims/vismasks.gaf", [0u8, 0, 0, 0, 1, 0, 0, 0, 9, 9, 9, 9]);
        f.put("mod", "anims/broken.gaf", [7u8, 7, 7, 7, 1, 0, 0, 0]);
        let options = Options { target_layer: Some("mod".into()), ..Default::default() };
        let mut report = validate(&f.vfs(&["base", "mod"]), &options, |_| true);
        compare(&mut report, &validate_all(&f.vfs(&["base"])));
        let copy = report.issues.iter().find(|i| i.path == "anims/vismasks.gaf").unwrap();
        assert!(copy.code == "format.invalid" && copy.origin == Origin::Inherited && !copy.affects_status, "{}", report.to_text());
        let own = report.issues.iter().find(|i| i.path == "anims/broken.gaf").unwrap();
        assert!(own.origin == Origin::Mod && own.affects_status && own.severity == Severity::Error, "{}", report.to_text());
        assert_eq!(report.status(), Status::Errors);
    }

    #[test]
    fn what_a_mod_breaks_in_inherited_content_counts_against_it() {
        // A base game whose unit fires a weapon; a mod that ships its own weapons file
        // without that weapon (and never mentions the unit).
        let f = Fixture::new(); f.valid("base");
        f.put("base", "units/turret.fbi", b"[UNITINFO]{UnitName=TURRET;ObjectName=turret;Side=ARM;Weapon1=GUN;}");
        f.put("base", "objects3d/turret.3do", model("base"));
        f.put("base", "scripts/turret.cob", script_with("base", &["Create", "QueryPrimary", "AimPrimary"], &[cob::op::RETURN]));
        f.put("base", "weapons/weapons.tdf", b"[GUN]{RenderType=0;}");
        f.put("base", "units/old.fbi", b"[UNITINFO]{UnitName=OLD;ObjectName=example;Side=ARM;Weapon1=GONE;}");
        f.put("mod", "weapons/weapons.tdf", b"[NEW_GUN]{RenderType=0;}");
        let options = Options { target_layer: Some("mod".into()), ..Default::default() };
        let baseline = validate_all(&f.vfs(&["base"]));
        let mut report = validate(&f.vfs(&["base", "mod"]), &options, |_| true);
        assert_eq!(report.status(), Status::Clean, "without the comparison the mod looks innocent: {}", report.to_text());
        compare(&mut report, &baseline);
        assert_eq!(report.status(), Status::Errors, "{}", report.to_text());
        let caused: Vec<_> = report.issues.iter().filter(|i| i.origin == Origin::Caused).collect();
        assert_eq!(caused.len(), 1, "{}", report.to_text());
        assert!(caused[0].path == "units/turret.fbi" && caused[0].message.contains("TURRET Weapon1: weapon GUN is missing") && caused[0].affects_status, "{}", caused[0].message);
        // What was broken before the mod stays the original game's.
        assert!(report.issues.iter().any(|i| i.path == "units/old.fbi" && i.origin == Origin::Inherited && !i.affects_status));
    }

    #[test]
    fn a_mod_overriding_definitions_is_fine_unless_the_original_comes_later() {
        let f = Fixture::new(); f.valid("base");
        f.put("base", "features/all/z_trees.tdf", b"[TREE]{world=all;}");
        f.put("base", "features/all/a_rocks.tdf", b"[ROCK]{world=all;}");
        // The mod's TREE sorts before the original's (never used); its ROCK after (used).
        f.put("mod", "features/all/a_mine.tdf", b"[TREE]{world=all;}");
        f.put("mod", "features/all/zz_mine.tdf", b"[ROCK]{world=all;}");
        let options = Options { target_layer: Some("mod".into()), ..Default::default() };
        let report = validate(&f.vfs(&["base", "mod"]), &options, |_| true);
        let shadowed: Vec<_> = report.issues.iter().filter(|i| i.code.starts_with("definition.")).collect();
        assert_eq!(shadowed.len(), 1, "{}", report.to_text());
        assert_eq!((shadowed[0].code.as_str(), shadowed[0].path.as_str()), ("definition.shadowed", "features/all/a_mine.tdf"));
        assert!(shadowed[0].message.contains("the original game's") && shadowed[0].affects_status);
    }

    #[test]
    fn files_where_the_game_never_looks_are_reported() {
        let f = Fixture::new(); f.valid("base");
        f.put("mod", "TA Content/units/ghost.fbi", b"[UNITINFO]{UnitName=GHOST;ObjectName=example;}");
        f.put("mod", "TA Content/weapons/ghost.tdf", b"[GHOST_GUN]{RenderType=0;}");
        f.put("mod", "extras/pack.ufo", b"not opened");
        f.put("mod", "units/sub/deep.fbi", b"[UNITINFO]{UnitName=DEEP;ObjectName=example;}");
        f.put("mod", "readme.txt", b"hello");
        let options = Options { target_layer: Some("mod".into()), ..Default::default() };
        let report = validate(&f.vfs(&["base", "mod"]), &options, |_| true);
        let codes: Vec<(&str, &str)> = report.issues.iter().filter(|i| i.code.starts_with("packaging.")).map(|i| (i.code.as_str(), i.path.as_str())).collect();
        assert!(codes.contains(&("packaging.wrapped", "ta content")), "{codes:?}");
        assert!(codes.contains(&("packaging.nested_archive", "extras/pack.ufo")), "{codes:?}");
        assert!(codes.contains(&("packaging.misplaced", "units/sub/deep.fbi")), "{codes:?}");
        assert_eq!(codes.len(), 3, "one finding for the wrapped folder, none for the readme: {codes:?}");
        assert_eq!(report.status(), Status::Issues);
    }

    #[test]
    fn a_mod_of_music_only_has_nothing_to_check() {
        let f = Fixture::new(); f.valid("base");
        f.put("mod", "music/theme.ogg", b"OggS");
        f.put("mod", "readme.txt", b"hello");
        let options = Options { target_layer: Some("mod".into()), ..Default::default() };
        let report = validate(&f.vfs(&["base", "mod"]), &options, |_| true);
        assert!(report.complete && report.empty);
        assert_eq!(report.status(), Status::Empty, "{}", report.to_text());
    }

    #[test]
    fn duplicates_syntax_sounds_and_scripts_are_checked() {
        let f = Fixture::new(); f.valid("mod");
        // Two files define one unit; an unclosed section swallows the weapon after it.
        f.put("mod", "units/copy.fbi", b"[UNITINFO]{UnitName=Example;ObjectName=example;Side=ARM;Builder=1;}");
        f.put("mod", "weapons/w.tdf", b"[SWALLOWER]\n{\n  RenderType=0;\n\n[LOST]\n{\n  RenderType=0;\n}\n");
        f.put("mod", "units/gunner.fbi", b"[UNITINFO]{UnitName=GUNNER;ObjectName=gunner;Side=ARM;Weapon1=LOST;SoundCategory=GUNNER_SOUNDS;}");
        f.put("mod", "objects3d/gunner.3do", model("base"));
        f.put("mod", "scripts/gunner.cob", script_with("base", &["Create", "AimPrimary"], &[cob::op::RETURN]));
        f.put("mod", "weapons/loud.tdf", b"[LOUD]{RenderType=0;soundstart=bang;soundhit=missing_hit;}");
        f.put("mod", "sounds/bang.wav", b"RIFF\x24\0\0\0WAVEfmt \x10\0\0\0\x55\0\x01\0\x44\xac\0\0\x44\xac\0\0\x01\0\x08\0data\0\0\0\0");
        f.put("mod", "gamedata/sound.tdf", b"[OTHER]{select1=bang;}");
        let report = validate_all(&f.vfs(&["mod"]));
        let find = |code: &str| report.issues.iter().filter(|i| i.code == code).collect::<Vec<_>>();
        let duplicate = find("definition.duplicate");
        assert_eq!(duplicate.len(), 1, "{}", report.to_text());
        assert_eq!(duplicate[0].path, "units/example.fbi", "the later file is the one used");
        let syntax = find("text.syntax");
        assert_eq!(syntax.len(), 1, "{}", report.to_text());
        assert_eq!((syntax[0].path.as_str(), syntax[0].line), ("weapons/w.tdf", Some(1)));
        assert!(report.issues.iter().any(|i| i.code == "reference.missing" && i.path == "units/gunner.fbi" && i.message.contains("weapon LOST")), "the swallowed weapon is missing: {}", report.to_text());
        assert!(report.issues.iter().any(|i| i.code == "reference.missing" && i.message.contains("sound MISSING_HIT (sounds/missing_hit.wav)")));
        assert!(report.issues.iter().any(|i| i.code == "reference.missing" && i.message.contains("sound category GUNNER_SOUNDS")));
        assert!(report.issues.iter().any(|i| i.code == "sound.format" && i.path == "sounds/bang.wav" && i.message.contains("0x0055")), "{}", report.to_text());
        let weapon = find("script.weapon");
        assert_eq!(weapon.len(), 1, "QueryPrimary is missing: {}", report.to_text());
        assert!(weapon[0].message.contains("QueryPrimary"));
        assert_eq!(report.counts.sounds, 1);
    }

    #[test]
    fn a_file_of_many_definitions_counts_for_those_used() {
        // A unit mod using one of the original game's sound categories and one of its weapons:
        // what is wrong with the others in those files stays the original game's.
        let f = Fixture::new(); f.valid("base");
        f.put("base", "gamedata/sound.tdf", b"[USED]{select1=used_missing;}[OTHER]{select1=other_missing;}");
        f.put("base", "weapons/weapons.tdf", b"[USED_GUN]{RenderType=0;soundstart=gun_missing;}[OTHER_GUN]{RenderType=9;}");
        f.put("base", "features/all/rocks.tdf", b"[ROCK]{world=all;footprintx=999;}[PEBBLE]{world=all;object=gone;}");
        f.put("mod", "units/tank.fbi", b"[UNITINFO]{UnitName=TANK;ObjectName=example;Side=ARM;SoundCategory=USED;Weapon1=USED_GUN;Corpse=PEBBLE;}");
        let options = Options { target_layer: Some("mod".into()), ..Default::default() };
        let report = validate(&f.vfs(&["base", "mod"]), &options, |_| true);
        let counts = |text: &str| report.issues.iter().find(|i| i.message.contains(text)).map(|i| i.affects_status);
        assert_eq!(counts("USED_MISSING"), Some(true), "{}", report.to_text());
        assert_eq!(counts("GUN_MISSING"), Some(true), "{}", report.to_text());
        assert_eq!(counts("objects3d/gone.3do"), Some(true), "the wreck it leaves: {}", report.to_text());
        assert_eq!(counts("OTHER_MISSING"), Some(false), "{}", report.to_text());
        assert_eq!(counts("RenderType=9"), Some(false), "{}", report.to_text());
        assert_eq!(counts("[ROCK] footprint"), Some(false), "{}", report.to_text());
    }

    #[test]
    fn production_pages_are_read_as_the_game_reads_them() {
        let f = Fixture::new(); f.valid("mod");
        // A builder whose name ends in a digit; its pages 1 and 2, then a gap, then page 4.
        f.put("mod", "units/corca2.fbi", b"[UNITINFO]{UnitName=CORCA2;ObjectName=example;Side=ARM;Builder=1;}");
        let page = |unit: &str| format!("[A]{{[COMMON]{{id=1;name={unit};attribs=32;commonattribs=4;}}}}");
        f.put("mod", "guis/corca21.gui", page("ON_PAGE_ONE"));
        f.put("mod", "guis/corca22.gui", page("EXAMPLE"));
        f.put("mod", "guis/corca24.gui", page("AFTER_THE_GAP"));
        // A unit that builds nothing: its page is never read.
        f.put("mod", "units/tank.fbi", b"[UNITINFO]{UnitName=TANK;ObjectName=example;Side=ARM;}");
        f.put("mod", "guis/tank1.gui", page("NOT_A_BUILDER"));
        let report = validate_all(&f.vfs(&["mod"]));
        let text = report.to_text();
        assert!(text.contains("ON_PAGE_ONE"), "{text}");
        assert!(!text.contains("AFTER_THE_GAP") && !text.contains("NOT_A_BUILDER"), "{text}");
        // EXAMPLE is on a menu now, and has no picture.
        assert!(report.issues.iter().any(|i| i.code == "unit.picture" && i.path == "units/example.fbi" && i.severity == Severity::Info), "{text}");
    }

    /// Opt-in local QA, no proprietary assets bundled with tests. Also writes the full
    /// report when requested so reviewers can audit baseline versus target findings.
    #[test]
    #[ignore = "requires an installed original game; TA_VALIDATE_MOD optionally adds a local mod"]
    fn installed_content_validation_report() {
        let game = ta_vfs::install::discover(None).expect("Original game install not found");
        let mut options = Options::default();
        let mut item = None;
        if let Some(path) = std::env::var_os("TA_VALIDATE_MOD") {
            let path = Path::new(&path);
            let (mods, errors) = ta_vfs::mods::scan(path.parent().unwrap());
            assert!(errors.is_empty(), "{errors:?}");
            let found = mods.into_iter().find(|m| m.path == path).expect("installed mod manifest");
            options.target_layer = Some(found.manifest.id.clone());
            item = Some(found);
        }
        // (As the game mounts it: what the mod replaces of the install left out.)
        let mounted = ta_vfs::mods::mount(Some(&game.root), ta_vfs::builtin_dir().as_deref(), &item.iter().collect::<Vec<_>>(), &|m| m.manifest.id.clone());
        mounted.install.unwrap();
        mounted.mods.unwrap();
        let vfs = mounted.vfs;
        let report = validate(&vfs, &options, |_| true);
        if let Some(path) = std::env::var_os("TA_VALIDATE_REPORT") { std::fs::write(path, report.to_text()).unwrap(); }
        println!("{:?}: {} checked; {} issues, {} affecting status", report.status(), report.counts.files_checked, report.issues.len(), report.issues.iter().filter(|i| i.affects_status).count());
        assert!(report.complete, "{}", report.to_text());
    }
}
