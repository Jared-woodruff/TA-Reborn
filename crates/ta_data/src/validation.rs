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
use ta_formats::{cob::{self, CobScript}, gaf::{self, AnimSize, GafReader}, model3do::Model3do, tdf::{self, LintProblem, TdfDocument, TdfSection}};
use ta_vfs::Vfs;

use crate::words::{en, enf, Fill, Words};

/// Bumped whenever the checks change what they report, so cached reports are made again (5:
/// WAV music not misplaced, a replaced definition's findings not the replacing mod's, a folder of
/// unreadable archives an error; 6: the model textures' room counts only those the models paint
/// with, in the game's two arrays; 7: empty files are blanks, a note a folder, and a mod's copy of
/// an original file that does not read is not the mod's; 8: GAFs read a frame at a time, a
/// weapon's explosion too large to decode whole drawn reduced, the explosions' room as drawn;
/// 9: a script's code decoded as the VM runs it, so what a compiler left between functions is
/// no unknown opcode; 10: a unit's script is its own, `scripts/<UnitName>.cob`, before its
/// model's; 11: as the game reads mods, every name a finding lists named; 12: each finding's words
/// kept for the launcher to show in the player's language, `Issue::words`, a file's reading
/// error told in words of the check's own around the reader's; 13: what a mod uses of TA's later
/// engines and how the game plays it, `Report::engine`).
pub const SCHEMA_VERSION: u32 = 13;

/// The keys of `gamedata/sound.tdf`'s categories the game plays (`ta_audio`'s sound events), each
/// with or without a variant number (`select1`, `select2`…), with `count0` to `count9`; the
/// others (`load`, `unload`…) it never plays. (A test in `ta_audio` holds this to its events.)
pub const PLAYED_SOUND_KEYS: &[&str] = &["select", "ok", "arrived", "cant", "underattack", "build", "repair", "working", "activate", "deactivate", "unitcomplete", "cloak", "uncloak", "capture", "canceldestruct"];

/// Whether the game plays a sound category's key (`select1`, `count3`): [`PLAYED_SOUND_KEYS`],
/// each with or without a variant number, and `count0` to `count9`.
fn played_sound_key(key: &str) -> bool {
    let key = key.trim().to_ascii_lowercase();
    let base = key.trim_end_matches(|c: char| c.is_ascii_digit());
    let digits = &key[base.len()..];
    if base == "count" {
        return digits.parse::<u8>().is_ok_and(|n| n <= 9);
    }
    (digits.is_empty() || digits.parse::<u32>().is_ok()) && PLAYED_SOUND_KEYS.contains(&base)
}

/// The unit values Reborn's engine answers when a script asks for one (`get`) or sets one
/// (`set`), as `ta_sim`'s script host does: TA's own 1 to 20, the veteran level (32), TA 3.1's
/// questions about units by id (69 to 75) and a later patch's cloak (111). A script asking for
/// another reads 0, and one setting another sets nothing (`script.value`). (A test in `ta_sim`
/// holds this list to the host's.)
pub const ANSWERED_VALUES: &[i32] = &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 32, 69, 70, 71, 72, 73, 74, 75, 111];

/// Of [`ANSWERED_VALUES`], those of TA's later patches: TA 3.1's questions about units by id (69
/// to 75) and a later patch's cloak (111). A mod's scripts asking them are told
/// (`engine.script_values`): the engine it was made for, and how the game plays them.
const PATCH_VALUES: [i32; 8] = [69, 70, 71, 72, 73, 74, 75, 111];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    #[default]
    Warning,
    Info,
}

impl Severity {
    /// The graver of two.
    fn graver(self, other: Severity) -> Severity {
        let rank = |s: Severity| match s {
            Severity::Error => 0,
            Severity::Warning => 1,
            Severity::Info => 2,
        };
        if rank(other) < rank(self) { other } else { self }
    }
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
    /// In English: the words a finding is compared by ([`compare`]) and the copied report's.
    pub message: String,
    /// `message`'s words, for the launcher to show in the player's language (none in a report
    /// kept by an older version: `message` is shown).
    #[serde(default, skip_serializing_if = "Words::is_empty")]
    pub words: Words,
    /// What to do, in English: a key of the languages' files (marked [`en`]).
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
        Self { severity: Severity::Warning, code: String::new(), path: String::new(), layer: String::new(), source: String::new(), line: None, message: String::new(), words: Words::default(), suggestion: String::new(), affects_status: true, origin: Origin::Mod }
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
    /// The mod's own (its layer's), where the check is of a mod: the counts above take in what
    /// it plays over too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own: Option<Own>,
}

/// What a mod checked holds of its own ([`Counts::own`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Own {
    /// Its files the game reads, and those of them that do not read.
    pub files: u64,
    pub failed: u64,
    pub units: u64,
    pub weapons: u64,
    pub features: u64,
    pub maps: u64,
    pub sounds: u64,
}

/// How the game plays something a mod uses ([`EngineUse`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Support {
    /// Played as in the engine the mod was made for.
    Played,
    /// Not used: what it did in TA is not there.
    Unused,
}

/// Something a mod uses of TA's later patched engines, or of what TA's own engine left to
/// itself, and how TA Reborn plays it ([`Report::engine`]). Not a finding, and it never counts:
/// what a player, and the mod's makers, want to know of how the mod plays here. Of the mod's own
/// files only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineUse {
    /// What it is (`engine.ai_lists`, `engine.tall_panels`...), the same in every language.
    pub code: String,
    pub support: Support,
    /// What the mod has, in English, and in words the launcher shows in the player's language.
    pub message: String,
    pub words: Words,
    /// How TA Reborn plays it, in English: a key of the languages' files (marked [`en`]).
    pub how: String,
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
    /// What the mod checked uses of TA's later engines, and how the game plays it (none in a
    /// check of the whole content, nor in a report kept by an older version).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub engine: Vec<EngineUse>,
}

impl Default for Report {
    fn default() -> Self {
        Self { schema_version: SCHEMA_VERSION, target_layer: None, complete: false, empty: false, counts: Counts::default(), issues: Vec::new(), engine: Vec::new() }
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
        if let Some(own) = &self.counts.own {
            let _ = writeln!(out, "This mod: {} files ({} not read); {} units, {} weapons, {} features, {} maps, {} sounds.", own.files, own.failed, own.units, own.weapons, own.features, own.maps, own.sounds);
            let _ = writeln!(out, "Checked with what it plays over: {}/{} files ({} not read); {} units, {} weapons, {} features, {} maps, {} sounds.", self.counts.files_checked, self.counts.files_total, self.counts.files_failed, self.counts.units, self.counts.weapons, self.counts.features, self.counts.maps, self.counts.sounds);
        } else {
            let _ = writeln!(out, "Checked {}/{} files; {} parse/read failures; {} units, {} weapons, {} features, {} maps, {} sounds.", self.counts.files_checked, self.counts.files_total, self.counts.files_failed, self.counts.units, self.counts.weapons, self.counts.features, self.counts.maps, self.counts.sounds);
        }
        if !self.engine.is_empty() {
            out.push_str("\nHow TA Reborn plays what it uses:\n");
            for (n, used) in self.engine.iter().enumerate() {
                let support = match used.support { Support::Played => "Played", Support::Unused => "Not used" };
                let _ = writeln!(out, "{}. {support} [{}]\n   {}\n   {}", n + 1, used.code, used.message, used.how);
            }
            out.push('\n');
        }
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
        if without {
            // There without the mod too, the same in the same place: the original game's or a
            // dependency's, in what the mod uses (a sound category of TA's whose sound TA lacks)
            // or carries a copy of (Total Mayhem's and TA: Escalation's of TA's
            // `gamedata/sound.tdf`, Total Mayhem's of its `anims/vismasks.gaf`, which is no GAF):
            // no fault of the mod's.
            issue.origin = Origin::Inherited;
            issue.affects_status = false;
        } else if issue.origin == Origin::Inherited {
            // Only there with the mod: its doing. (A note too, though notes never decide the badge.)
            issue.origin = Origin::Caused;
            issue.affects_status = true;
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
        scan.stop("validation.limit", "", en("The effective content set exceeds the validation file budget."), en("Split the content into smaller optional packs, then validate each pack."));
        return scan.report;
    }
    let everything = vfs.list("", None, true);
    let files: Vec<_> = everything.iter().filter(|p| read_by_game(p)).cloned().collect();
    scan.report.counts.files_total = files.len() as u64;
    if let Some(target) = &options.target_layer {
        let own: Vec<&str> = everything.iter().filter(|p| vfs.origin(p).is_some_and(|o| &o.layer == target)).map(String::as_str).collect();
        if own.is_empty() {
            scan.stop("validation.no_content", "", en("No files of this mod were available to validate."), en("Install the mod's TA data archives or loose files, then validate again."));
            return scan.report;
        }
        scan.report.counts.own = Some(Own { files: own.iter().filter(|p| read_by_game(p)).count() as u64, ..Own::default() });
        scan.packaging(&own);
        scan.own_files = own.iter().filter(|p| p.starts_with("ai/") || p.starts_with("camps/") || is_program(p)).map(|p| p.to_string()).collect();
        if !own.iter().any(|p| known(p)) {
            // Music, documents or pictures only: nothing of the game's data to check, and
            // nothing the mod changes in it.
            scan.report.empty = true;
            scan.note("validation.nothing", "", Words::text(en("This mod holds nothing the data checks read: no units, weapons, features, maps, models, scripts, animations or sounds (music, documents and pictures are not checked).")), en("Nothing to do if that is what the mod is. If it should change the game's data, check that its files are in the folders the game reads (units/, weapons/, maps/…)."));
            // (What it brings that TA Reborn does not use is still told: a patch alone, an
            // engine's DLL, TA's own computer players' files.)
            scan.engine_uses();
            scan.finish_scope();
            scan.report.sort();
            return scan.report;
        }
    } else if files.is_empty() {
        scan.stop("validation.no_content", "", en("No supported effective files were available to validate."), en("Install the original game's or the mod's TA data archives, then validate again."));
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
                scan.stop("validation.cancelled", path, en("Validation was cancelled; remaining files and references were not checked."), en("Run validation again to obtain a complete report."));
                break 'files;
            }
            bytes_read = bytes_read.saturating_add(parsed.bytes);
            if bytes_read > options.max_read_bytes {
                scan.stop("validation.limit", path, en("The cumulative validation read budget was exceeded."), en("Split optional content into smaller packs, then validate again."));
                break 'files;
            }
            scan.report.counts.files_checked += 1;
            scan.file(path, parsed);
            if scan.halted { break 'files; }
        }
    }
    if !scan.halted { scan.blank_notes(); }
    if !scan.halted { scan.references(); }
    if !scan.halted && options.target_layer.is_some() { scan.engine_uses(); }
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

/// Whether what a mod uses of TA's later engines (`Report::engine`) is told from a file of this
/// (lower-case) name in a folder of this (lower-case) name by its presence alone: a program or a
/// library of one, or a file of TA's own computer players (`ai/`). With [`checked_file`], what a
/// content fingerprint must cover, its adding or removing being another report.
pub fn noted_file(folder: &str, name: &str) -> bool { is_program(name) || folder == "ai" }

/// The note that the mod checked (its layer `layer`) replaces the original game's archive
/// `archive` (as the install names it, `ta_vfs::mods::replaced_archives`): with it on, the
/// install's archive of that name is not read. A note: it never counts.
pub fn replaced_archive(layer: &str, archive: &str) -> Issue {
    let words = enf("With this mod on, the original game's {archive} is not read: the mod takes its place (an archive of its own of that name, one its mod.toml's replaces names, or one its installation is known to remove), as installing the mod in TA's folder did. What only the original's held is missing with the mod.", [("archive", Fill::name(archive))]);
    Issue {
        severity: Severity::Info, code: "packaging.replaces_original".into(), path: archive.into(), layer: layer.into(),
        message: words.english(), words,
        suggestion: en("Nothing to do if this is meant. To have the original's read beneath the mod instead, give the mod's archive another name, or take the name out of replaces in mod.toml.").into(),
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

/// Whether a page's button makes units, as the game reads the page: its `commonattribs` bit 4;
/// on a hand-made page without them (the game reads its buttons all, keeping those that name
/// units), the `attribs` bit 32 TA marks its production buttons with, its navigation's not.
fn production(common: &TdfSection) -> bool {
    match common.get("commonattribs") {
        Some(_) => common.get_i32_or("commonattribs", 0) & 4 != 0,
        None => common.get_i32_or("attribs", 0) & 32 != 0,
    }
}

/// The folders the game reads its data from. A file elsewhere (a picture in `docs/`, a GAF beside
/// a read-me) it never opens, so it is never decoded: the packaging check tells where the game
/// would read it (`misplaced`).
const GAME_FOLDERS: &[&str] = &["ai", "anims", "bitmaps", "camps", "download", "features", "fonts", "gamedata", "guis", "maps", "music", "objects3d", "palettes", "scripts", "sounds", "textures", "unitpics", "units", "weapons"];

/// Whether the game reads a file of a kind it decodes: in one of its folders.
fn read_by_game(p: &str) -> bool { known(p) && p.split_once('/').is_some_and(|(top, _)| GAME_FOLDERS.contains(&top)) }
fn key(kind: &str, name: &str) -> String { format!("{kind}:{}", name.trim().to_ascii_lowercase()) }
fn asset(folder: &str, name: &str, ext: &str) -> String { ta_vfs::normalize(&format!("{folder}/{}.{ext}", name.trim())) }
fn direct(path: &str, folder: &str, ext: &str) -> bool { path.starts_with(&format!("{folder}/")) && path.matches('/').count() == 1 && path.ends_with(&format!(".{ext}")) }
/// Whose a layer is, as a sentence names it.
fn owner(layer: &str) -> Words {
    match layer {
        "base" => Words::text(en("the original game's")),
        ta_vfs::BUILTIN_LAYER => Words::text(en("Reborn's own")),
        _ => enf("{mod}'s", [("mod", Fill::name(layer.strip_prefix("mod:").unwrap_or(layer)))]),
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
fn describe(reference: &str) -> Words {
    let (kind, name) = reference.split_once(':').unwrap_or(("", reference));
    let upper = || Fill::name(name.to_ascii_uppercase());
    match kind {
        "file" => enf("the file {path}", [("path", Fill::name(name))]),
        "unit" => enf("unit {name}", [("name", upper())]),
        "weapon" => enf("weapon {name}", [("name", upper())]),
        "feature" => enf("feature {name}", [("name", upper())]),
        "side" => enf("faction {name}", [("name", upper())]),
        "move" => enf("movement class {name}", [("name", upper())]),
        "texture" => enf("texture {name}", [("name", upper())]),
        "sound" => enf("sound {name} ({path})", [("name", upper()), ("path", Fill::name(sound_path(name)))]),
        "sounds" => enf("sound category {name} (gamedata/sound.tdf)", [("name", upper())]),
        "placed" => enf("unit or feature {name}", [("name", upper())]),
        "gaf" => match name.split_once('#') { Some((file, entry)) => enf("animation {entry} in {file}", [("entry", Fill::name(entry.to_ascii_uppercase())), ("file", Fill::name(file))]), None => Words::raw(name) },
        _ => Words::raw(reference),
    }
}

/// The folders TA's own data lives in (a mod's files belong in the same).
const TA_FOLDERS: [&str; 20] = ["ai", "anims", "bitmaps", "camps", "download", "features", "fonts", "gamedata", "guis", "maps", "music", "objects3d", "palettes", "scripts", "sections", "sounds", "textures", "unitpics", "units", "weapons"];

/// Where the game reads a kind of file from, when `path` is not there: the folders, one of which.
fn misplaced(path: &str) -> Option<&'static [&'static str]> {
    let ext = path.rsplit_once('.')?.1;
    let depth = path.matches('/').count();
    let top = path.split('/').next().unwrap_or("");
    let home: (bool, &[&str]) = match ext {
        "fbi" => (top == "units" && depth == 1, &["units/"]),
        "3do" => (top == "objects3d", &["objects3d/"]),
        "cob" => (top == "scripts", &["scripts/"]),
        "gui" => (top == "guis" && depth == 1, &["guis/"]),
        "tnt" | "ota" => (top == "maps" && depth == 1, &["maps/"]),
        "gaf" => (matches!(top, "anims" | "textures" | "bitmaps") && depth == 1 || top == "camps", &["anims/", "textures/"]),
        "fnt" => (matches!(top, "fonts" | "guis"), &["fonts/"]),
        "tdf" => (matches!(top, "weapons" | "gamedata" | "download" | "maps") && depth == 1 || matches!(top, "features" | "camps"), &["weapons/", "features/", "gamedata/", "download/", "camps/"]),
        // (The music folder's tracks are WAVs too; a sound named with a folder is read from it.)
        "wav" => (top == "sounds" || top == "camps" || top == "music" && depth == 1, &["sounds/"]),
        _ => return None,
    };
    (!home.0).then_some(home.1)
}

/// Folders, as a sentence offers them: "units/", "anims/ or textures/".
fn folders(home: &[&str]) -> Fill {
    Fill::Either(home.iter().map(|folder| Fill::name(*folder)).collect())
}

/// A reason a sound file will not play, if any, as the game's decoder (`ta_audio::decode`) reads
/// one: a RIFF WAVE file of a codec it reads (PCM, MS ADPCM, float, A-law, µ-law, IMA ADPCM at 4
/// bits; WAVE_FORMAT_EXTENSIBLE wraps them), its format of sense (1 to 32 channels, a rate up to
/// 768 kHz, a sample size it reads), with samples; else Ogg Vorbis, Humongous's DIGI or TALK
/// blocks, or headerless 8-bit samples saved as `.wav` (TA's own `honk.wav` and `sing.wav`). (A
/// test in `ta_audio` holds this to the decoder.)
pub fn sound_problem(bytes: &[u8]) -> Option<Words> {
    const PCM: u16 = 0x0001;
    const MS_ADPCM: u16 = 0x0002;
    const FLOAT: u16 = 0x0003;
    const ALAW: u16 = 0x0006;
    const MULAW: u16 = 0x0007;
    const IMA_ADPCM: u16 = 0x0011;
    const EXTENSIBLE: u16 = 0xFFFE;
    if !(bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE") {
        if bytes.starts_with(b"OggS") {
            return (!ogg_stream_played(bytes)).then(|| Words::text(en("it is an Ogg file without a Vorbis or FLAC stream, the ones the game plays")));
        }
        if bytes.len() >= 8 && (bytes.starts_with(b"DIGI") || bytes.starts_with(b"TALK")) {
            return (!humongous_samples(bytes)).then(|| Words::text(en("it is a Humongous sound block without its samples (no SDAT block within its length)")));
        }
        return (!looks_like_pcm8(bytes)).then(|| Words::text(en("it is neither a RIFF WAVE file nor another sound the game reads")));
    }
    let le16 = |b: &[u8], at: usize| u16::from_le_bytes([b[at], b[at + 1]]);
    let le32 = |b: &[u8], at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
    // (The first format and data chunks, walked as the decoder walks them.)
    let (mut fmt, mut data): (Option<&[u8]>, bool) = (None, false);
    let mut at = 12usize;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let declared = le32(bytes, at + 4) as usize;
        let start = at + 8;
        if id == b"data" && (declared == 0 || declared == u32::MAX as usize) {
            data = true;
            break;
        }
        let body = &bytes[start..start + declared.min(bytes.len() - start)];
        match id {
            b"fmt " if fmt.is_none() => fmt = Some(body),
            b"data" => data = true,
            _ => {}
        }
        match start.checked_add(declared).and_then(|end| end.checked_add(declared & 1)) {
            Some(next) => at = next,
            None => break,
        }
    }
    let Some(c) = fmt else { return Some(Words::text(en("it has no format chunk"))) };
    if c.len() < 14 {
        return Some(enf("its format chunk is {n} bytes long, too short to read", [("n", Fill::number(c.len()))]));
    }
    let (mut tag, channels, rate, block_align) = (le16(c, 0), le16(c, 2), le32(c, 4), le16(c, 12));
    let bits = if c.len() >= 16 { le16(c, 14) } else { 8 };
    if tag == EXTENSIBLE {
        let extra: &[u8] = if c.len() >= 18 { &c[18..(18 + le16(c, 16) as usize).min(c.len())] } else { &[] };
        if extra.len() < 8 {
            return Some(Words::text(en("its WAVE_FORMAT_EXTENSIBLE header is too short to read")));
        }
        tag = le16(extra, 6);
    }
    if channels == 0 || channels > 32 {
        return Some(enf("it has {n} channels (the game plays 1 to 32)", [("n", Fill::number(channels))]));
    }
    if rate == 0 || rate > 768_000 {
        return Some(enf("its sample rate is {rate} (the game plays up to 768,000)", [("rate", Fill::number(rate))]));
    }
    // (Bytes a sample, as the decoder works them out: from the block, else from the bits.)
    let width = || {
        let from_bits = (bits as usize).div_ceil(8);
        let from_align = if (block_align as usize).is_multiple_of(channels as usize) { block_align as usize / channels as usize } else { 0 };
        if (1..=8).contains(&from_align) && from_align >= from_bits { from_align } else { from_bits }
    };
    let problem = match tag {
        PCM if !(1..=4).contains(&width()) => Some(enf("its samples are {bits}-bit PCM, which the game does not play (up to 32-bit)", [("bits", Fill::number(bits))])),
        FLOAT if !matches!(width(), 4 | 8) => Some(enf("its samples are {bits}-bit float, which the game does not play (32 or 64-bit)", [("bits", Fill::number(bits))])),
        IMA_ADPCM if bits != 4 => Some(enf("its IMA ADPCM is {bits}-bit, which the game does not play (4-bit)", [("bits", Fill::number(bits))])),
        PCM | FLOAT | ALAW | MULAW | IMA_ADPCM | MS_ADPCM => None,
        tag => Some(enf("its codec ({tag}) is not one the game plays (PCM, ADPCM, float, A-law or µ-law)", [("tag", Fill::name(format!("{tag:#06x}")))])),
    };
    problem.or_else(|| (!data).then(|| Words::text(en("it has no sample data"))))
}

/// Whether an Ogg file opens a stream the game's decoder plays (symphonia's Ogg reader with its
/// Vorbis and FLAC decoders), by the first packet of each of its first pages: a stream's first
/// page holds its identification header alone, and every stream's first page comes before any
/// other. Its audio is not decoded here: a stream damaged further on passes.
fn ogg_stream_played(b: &[u8]) -> bool {
    let mut at = 0;
    for _ in 0..8 {
        if b.get(at..at + 4) != Some(&b"OggS"[..]) {
            return false;
        }
        let Some(&segments) = b.get(at + 26) else { return false };
        let packet = at + 27 + segments as usize;
        let Some(table) = b.get(at + 27..packet) else { return false };
        if b.get(packet..).is_some_and(|p| p.starts_with(b"\x01vorbis") || p.starts_with(b"\x7fFLAC")) {
            return true;
        }
        at = packet + table.iter().map(|&s| s as usize).sum::<usize>();
    }
    false
}

/// Whether a Humongous `DIGI` or `TALK` block holds its samples (`SDAT`) within its length, as
/// the game's decoder reads its blocks (`ta_audio::decode`).
fn humongous_samples(b: &[u8]) -> bool {
    let be32 = |at: usize| u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as usize;
    let end = be32(4).clamp(8, b.len());
    let mut at = 8;
    while at + 8 <= end {
        let size = be32(at + 4);
        if size < 8 {
            return false;
        }
        if &b[at..at + 4] == b"SDAT" {
            return true;
        }
        at = at.saturating_add(size);
    }
    false
}

/// Headerless unsigned 8-bit samples, as the game's decoder tells them (`ta_audio::decode`): long
/// enough, and each sample near the one before (a correlation above 0.6).
fn looks_like_pcm8(b: &[u8]) -> bool {
    if b.len() < 256 {
        return false;
    }
    let mean = b.iter().map(|&x| x as f64).sum::<f64>() / b.len() as f64;
    let (mut num, mut den) = (0.0f64, 0.0f64);
    for w in b.windows(2) {
        let (x0, x1) = (w[0] as f64 - mean, w[1] as f64 - mean);
        num += x0 * x1;
        den += x0 * x0;
    }
    den > 0.0 && num / den > 0.6
}

/// A file read and decoded, summarised: what the report needs of it.
struct Parsed { bytes: u64, content: Result<Content, Failure> }
enum Failure { Read(String), Invalid(String), TooLarge }
enum Content {
    Model { pieces: BTreeSet<String>, textures: BTreeSet<String> },
    /// `patch_values`: those of [`PATCH_VALUES`] it asks or sets.
    Script { pieces: BTreeSet<String>, functions: BTreeSet<String>, problems: Vec<Problem>, patch_values: BTreeSet<i32> },
    /// A GAF's entries; `whole`: the game could decode it all at once (as it does every GAF but
    /// the weapons' explosions, which it reads a frame at a time: `gaf_content`).
    Gaf { entries: Vec<GafEntry>, whole: bool },
    Map { features: BTreeSet<String> },
    /// A picture, font or palette: readable, nothing more to it.
    Plain,
    /// A WAV, and what keeps it from playing, if anything.
    Sound(Option<Words>),
    Text { doc: TdfDocument, syntax: Vec<(u32, LintProblem)> },
    /// An empty file: TA's way of blanking out a file of the same name below it (the original
    /// game's, or a mod's it is mounted over), nothing to read, not a broken one (`blank_notes`).
    Blank,
}
struct GafEntry { name: String, frames: usize, size: AnimSize }
struct Problem { severity: Severity, code: &'static str, message: Words, suggestion: &'static str }

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
        "cob" => CobScript::parse(&bytes).map(|script| {
            let (problems, patch_values) = script_problems(&script);
            Content::Script {
                pieces: script.pieces.iter().map(|s| s.to_ascii_lowercase()).collect(),
                functions: script.functions.iter().map(|f| f.name.to_ascii_lowercase()).collect(),
                problems, patch_values,
            }
        }).map_err(|e| Failure::Invalid(e.to_string())),
        "gaf" => gaf_content(&bytes),
        "tnt" => ta_formats::tnt::TntMap::parse(&bytes).map(|map| Content::Map {
            features: map.features.iter().filter_map(|&n| map.feature_names.get(n as usize)).filter(|n| !n.trim().is_empty()).cloned().collect(),
        }).map_err(|e| Failure::Invalid(e.to_string())),
        "pcx" => ta_formats::pcx::PcxImage::parse(&bytes).map(|_| Content::Plain).map_err(|e| Failure::Invalid(e.to_string())),
        "fnt" => ta_formats::fnt::Font::parse(&bytes).map(|_| Content::Plain).map_err(|e| Failure::Invalid(e.to_string())),
        "pal" => ta_formats::image::Palette::parse(&bytes).map(|_| Content::Plain).map_err(|e| Failure::Invalid(e.to_string())),
        "wav" => Ok(Content::Sound(sound_problem(&bytes))),
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
/// And the values of TA's later patches it asks or sets ([`PATCH_VALUES`]).
fn script_problems(script: &CobScript) -> (Vec<Problem>, BTreeSet<i32>) {
    use cob::op::*;
    let mut problems = Vec::new();
    let entries: BTreeSet<usize> = script.functions.iter().map(|f| f.offset as usize).collect();
    let len = script.code.len();
    let mut decoded = vec![false; len];
    // The unit values it asks for and sets that the engine does not answer (see `asked`).
    let (mut unanswered_get, mut unanswered_set, mut patch_values) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    // (The places to decode from, lowest first: the entries, then where their jumps lead.)
    let mut pending = entries.clone();
    while let Some(mut pc) = pending.pop_first() {
        // (The instructions just before this one, on the way here, opcode and operand.)
        let mut recent: Vec<(u32, u32)> = Vec::new();
        // (Past the last instruction: a terminal label, which ends the thread.)
        while pc < len && !decoded[pc] {
            decoded[pc] = true;
            let end = entries.range(pc + 1..).next().copied().unwrap_or(len);
            let opcode = script.code[pc];
            let Some(n) = cob::operand_count(opcode) else {
                problems.push(Problem { severity: Severity::Error, code: "script.opcode", message: enf("Unknown COB opcode {opcode} at word {word}; the VM cannot execute this instruction. The remainder of this function cannot be decoded reliably.", [("opcode", Fill::name(format!("{opcode:#010x}"))), ("word", Fill::number(pc))]), suggestion: en("Recompile the BOS script with original-TA-compatible opcodes, or report the required opcode extension to Reborn's maintainers.") });
                break;
            };
            if pc + 1 + n > end {
                problems.push(Problem { severity: Severity::Error, code: "script.operands", message: enf("COB instruction {instruction} at word {word} is truncated or crosses the next function entry point.", [("instruction", Fill::name(cob::opcode_name(opcode))), ("word", Fill::number(pc))]), suggestion: en("Recompile this COB from its BOS source; verify function offsets and inline operand counts.") });
                break;
            }
            let arg = script.code.get(pc + 1).copied().unwrap_or(0);
            let invalid = match opcode {
                MOVE | TURN | SPIN | STOP_SPIN | WAIT_TURN | WAIT_MOVE | MOVE_NOW | TURN_NOW => (arg as usize >= script.pieces.len() || script.code[pc + 2] > 2).then_some(en("COB instruction {instruction} at word {word} has an invalid piece index or axis.")),
                SHOW | HIDE | CACHE | DONT_CACHE | SHADE | DONT_SHADE | EMIT_SFX | EXPLODE => (arg as usize >= script.pieces.len()).then_some(en("COB instruction {instruction} at word {word} has an invalid piece index.")),
                PUSH_STATIC | POP_STATIC => (arg >= script.static_var_count).then_some(en("COB instruction {instruction} at word {word} has an invalid static variable index.")),
                // A terminal label exactly past the last instruction ends the
                // thread (VM's code.get(pc) returns None). Some BOS compilers
                // emit this for the final else branch of Killed().
                JUMP | JUMP_IF_FALSE => (arg as usize > len).then_some(en("COB instruction {instruction} at word {word} has an invalid jump destination.")),
                START_SCRIPT | CALL_SCRIPT => (arg as usize >= script.functions.len()).then_some(en("COB instruction {instruction} at word {word} has an invalid function index.")),
                _ => None,
            };
            if let Some(text) = invalid {
                let message = enf(text, [("instruction", Fill::name(cob::opcode_name(opcode))), ("word", Fill::number(pc))]);
                problems.push(Problem { severity: Severity::Warning, code: "script.operand_range", message, suggestion: en("Recompile the script with valid piece, variable, function and jump references.") });
                // (Where a jump out of the code would lead is not followed.)
                if matches!(opcode, JUMP | JUMP_IF_FALSE) { break; }
            }
            match asked(opcode, &recent) {
                Some((false, value)) if !ANSWERED_VALUES.contains(&value) => { unanswered_get.insert(value); }
                Some((true, value)) if !ANSWERED_VALUES.contains(&value) => { unanswered_set.insert(value); }
                Some((_, value)) if PATCH_VALUES.contains(&value) => { patch_values.insert(value); }
                _ => {}
            }
            recent.push((opcode, arg));
            if recent.len() > 5 { recent.remove(0); }
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
    let values = |set: &BTreeSet<i32>| Fill::List(set.iter().map(|&v| Fill::number(v)).collect());
    if !unanswered_get.is_empty() {
        let message = enf("This script asks the engine for unit value {values}, which Reborn does not answer: it reads 0, so what the script does with it may not happen as in the engine it was made for.", [("values", values(&unanswered_get))]);
        problems.push(Problem { severity: Severity::Warning, code: "script.value", message, suggestion: en("A later patch of TA's engine defines these values: report them, and what the mod does with them, to Reborn's maintainers.") });
    }
    if !unanswered_set.is_empty() {
        let message = enf("This script sets unit value {values}, which Reborn does not answer: setting it does nothing, so what the script means by it does not happen as in the engine it was made for.", [("values", values(&unanswered_set))]);
        problems.push(Problem { severity: Severity::Warning, code: "script.value", message, suggestion: en("A later patch of TA's engine defines these values: report them, and what the mod does with them, to Reborn's maintainers.") });
    }
    (problems, patch_values)
}

/// The unit value an instruction asks for or sets, where the code names it outright, as a BOS
/// compiler writes `get NAME`, `get NAME(a, b, c, d)` and `set NAME to x`: a constant pushed just
/// before `get-unit-value`, before the four simple arguments of `get`, or before the one simple
/// value of `set` (`recent`: the instructions just before, opcode and operand). Whether it sets
/// (true) and the value. (A value worked out as the script runs is not named so: not told.)
fn asked(opcode: u32, recent: &[(u32, u32)]) -> Option<(bool, i32)> {
    use cob::op::*;
    let simple = |op: u32| matches!(op, PUSH_CONSTANT | PUSH_LOCAL | PUSH_STATIC);
    let back = |n: usize| recent.len().checked_sub(n).map(|i| recent[i]);
    match opcode {
        GET_UNIT_VALUE => back(1).filter(|(op, _)| *op == PUSH_CONSTANT).map(|(_, v)| (false, v as i32)),
        GET => (back(5).is_some_and(|(op, _)| op == PUSH_CONSTANT) && (1..=4).all(|n| back(n).is_some_and(|(op, _)| simple(op)))).then(|| (false, back(5).unwrap().1 as i32)),
        SET_VALUE => (back(2).is_some_and(|(op, _)| op == PUSH_CONSTANT) && back(1).is_some_and(|(op, _)| simple(op))).then(|| (true, back(2).unwrap().1 as i32)),
        _ => None,
    }
}

/// A definition's reference: what it is to, what it is in the definition (its words: "ARMCK
/// model"), and how grave its going missing is.
struct Reference { key: String, what: Words, severity: Severity }
impl Reference {
    fn new(kind: &str, name: &str, what: Words, severity: Severity) -> Self { Self { key: key(kind, name), what, severity } }
}

/// A definition's part, as a reference names it: "{name} model".
fn part(text: &'static str, name: &str) -> Words {
    enf(text, [("name", Fill::name(name))])
}
#[derive(Default)]
struct Record { path: String, refs: Vec<Reference>, unit: Option<UnitFacts> }
/// What a reference leads to: a whole file (a model, a script, a sound, a map's terrain) or a
/// definition (a weapon in a file of many).
enum Use { File(String), Record(String) }
/// What the later checks need of a unit.
/// `script`: the unit's script as the game finds it (`UnitDef::script_path`); `builds`: it has
/// build power (`WorkerTime`), which makes it a builder whatever its Builder flag, as the game
/// has it; `takes_no_ground`: a mod's upgrade module (`UnitDef::takes_no_ground`).
struct UnitFacts { name: String, object: String, script: String, builds: bool, takes_no_ground: bool, weapons: [bool; 3] }
#[derive(Default)]
struct Animation { frames: usize, size: AnimSize }

/// What the mod checked uses of TA's later engines, gathered from its own files as they are read
/// (told in `Scan::engine_uses`).
#[derive(Default)]
struct EngineFacts {
    /// `[CANBUILD]`'s builders and their entries.
    ai_builders: usize,
    ai_entries: usize,
    /// Side panel pages laid out for a screen taller than TA's 480 pixels, and the tallest.
    tall_pages: usize,
    tallest: i64,
    /// Build pages with buttons off TA's grid of 64-pixel places, and those buttons.
    shared_pages: usize,
    shared_buttons: usize,
    /// Scripts asking the patched engines' values, and the values.
    patch_scripts: usize,
    patch_values: BTreeSet<i32>,
    /// Every weapon that gives an ID (the mod's and those it plays over): its name, the ID, and
    /// whether it is the mod's.
    weapon_ids: Vec<(String, i64, bool)>,
    /// Sides of its own (not TA's ARM and CORE).
    sides: Vec<String>,
}

/// A program or a library of one (a patched `TotalA.exe`, a DLL).
fn is_program(path: &str) -> bool { path.ends_with(".exe") || path.ends_with(".dll") }

/// TA's screen and its build places: a side panel taller than its 480 pixels was laid out for
/// a patched engine's taller screen, and a build button other than a 64-pixel square shares a
/// place or takes several (docs/FORMATS.md).
const TA_SCREEN_HEIGHT: i64 = 480;
const TA_PLACE: i64 = 64;

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
    /// The empty files, in path order (told a folder at a time: `blank_notes`), and all of them
    /// (a reference to one is to nothing: `references`).
    blanks: Vec<String>,
    blank_paths: BTreeSet<String>,
    /// Production pages (`guis/<builder><page>.gui`): the units their buttons name.
    gui_pages: BTreeMap<String, Vec<Reference>>,
    /// Units some builder with build power offers a player (download menus, production pages;
    /// not CANBUILD, the computer players' lists, which no menu shows).
    buildable: BTreeSet<String>,
    /// CANBUILD's and the download menus' entries, builder and unit, and whether on a player's
    /// menu (a download menu's: CANBUILD's are the computer players'). Settled in `references`,
    /// once the builders' build power is known.
    menu_entries: Vec<(String, String, bool)>,
    /// Builders whose every option is a module of their own (a mod's upgradeable building).
    module_builders: BTreeSet<String>,
    /// The target's definitions an inherited one of the same name comes after, by the
    /// target's file: what, the later file, whose it is.
    shadowed: BTreeMap<String, Vec<(Words, String, Words)>>,
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
    /// What the mod uses of TA's later engines, and its own files the game does not read that
    /// tell of it (`ai/`, `camps/`, programs).
    engine: EngineFacts,
    own_files: Vec<String>,
}

impl<'a> Scan<'a> {
    fn new(vfs: &'a Vfs, options: &'a Options) -> Self {
        Self {
            vfs, options, report: Report { complete: true, target_layer: options.target_layer.clone(), ..Default::default() }, retained: 0, halted: false,
            records: BTreeMap::new(), pieces: BTreeMap::new(), functions: BTreeMap::new(), animations: BTreeMap::new(), too_big: BTreeMap::new(), texture_names: BTreeSet::new(), texture_entries: Vec::new(), model_textures: BTreeSet::new(), blanks: Vec::new(), blank_paths: BTreeSet::new(),
            gui_pages: BTreeMap::new(), buildable: BTreeSet::new(), menu_entries: Vec::new(), module_builders: BTreeSet::new(), shadowed: BTreeMap::new(), uses: BTreeMap::new(), record_paths: BTreeMap::new(),
            file_records: BTreeMap::new(), attributed: BTreeMap::new(), relevant: BTreeSet::new(), required: BTreeSet::new(),
            engine: EngineFacts::default(), own_files: Vec::new(),
        }
    }

    fn stop(&mut self, code: &str, path: &str, message: &'static str, suggestion: &'static str) {
        if self.halted { return; }
        self.halted = true;
        self.report.complete = false;
        let issue = Issue { code: code.into(), path: path.into(), message: message.into(), words: Words::text(message), suggestion: suggestion.into(), ..self.located(path) };
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
            _ => { self.stop("validation.limit", path, en("The retained diagnostic/reference budget was exceeded. This is a partial report, not a clean validation."), en("Reduce the pack's number of definitions or split optional content; rerun validation after resolving reported issues.")); false }
        }
    }

    fn issue(&mut self, severity: Severity, code: &str, path: &str, words: Words, suggestion: &str) {
        self.issue_at(severity, code, path, None, words, suggestion);
    }

    /// A finding: its words (`message`, their English), and what to do (marked [`en`]).
    fn issue_at(&mut self, severity: Severity, code: &str, path: &str, line: Option<u32>, words: Words, suggestion: &str) {
        let located = self.located(path);
        let message = words.english();
        // (Its words take about as much again as their English.)
        if self.retain(256 + code.len() + path.len() + located.layer.len() + located.source.len() + 2 * message.len() + suggestion.len(), path) {
            self.report.issues.push(Issue { severity, code: code.into(), path: path.into(), line, message, words, suggestion: suggestion.into(), ..located });
        }
    }

    /// The findings told since `from` are about the definition `key`.
    fn attribute(&mut self, from: usize, key: &str) {
        for i in from..self.report.issues.len() { self.attributed.insert(i, key.to_string()); }
    }

    /// A note about the whole check (no file).
    fn note(&mut self, code: &str, path: &str, words: Words, suggestion: &str) {
        self.issue(Severity::Info, code, path, words, suggestion);
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
            if kind == "side" {
                // (The game keeps every side section: two of one name are two sides.)
                self.issue(Severity::Warning, "definition.duplicate", &record.path, enf("{what} is defined twice; the game keeps both, two sides of one name.", [("what", what.into())]), en("Give each side a name of its own, or remove the one not meant to be played."));
            } else if old_path == record.path {
                self.issue(Severity::Warning, "definition.duplicate", &record.path, enf("{what} is defined twice in this file; the game uses the later one.", [("what", what.into())]), en("Give each definition a name of its own, or remove the one not meant to be used."));
            } else if old_layer == new_layer {
                self.issue(Severity::Warning, "definition.duplicate", &record.path, enf("{what} is also defined in {other}; the game uses this one ({this}).", [("what", what.into()), ("other", Fill::name(&old_path)), ("this", Fill::name(&record.path))]), en("Give each definition a name of its own, or remove the one not meant to be used."));
            } else if self.options.target_layer.as_deref() == Some(old_layer.as_str()) {
                // (Told once a file in `references`: a file of them is common.)
                if self.retain(160 + old_path.len() + 2 * what.text.len() + record.path.len(), &old_path) {
                    self.shadowed.entry(old_path).or_default().push((what, record.path.clone(), owner(&new_layer)));
                }
            }
        }
        self.attribute(first, &key);
        let size = 256 + key.len() + record.path.len() + record.unit.as_ref().map_or(0, |u| 64 + u.name.len() + u.object.len()) + record.refs.iter().map(|r| 128 + r.key.len() + r.what.text.len()).sum::<usize>();
        if self.retain(size, &record.path) { self.records.insert(key, record); }
    }

    /// The target's files where the game never reads them: archives in subfolders (only the
    /// top level's are opened), data files outside their folders, and a whole mod wrapped in
    /// a folder of its own ("My Mod/units/…").
    fn packaging(&mut self, own: &[&str]) {
        let mut misplaced_by_top: BTreeMap<&str, Vec<(&str, &'static [&'static str])>> = BTreeMap::new();
        for &path in own {
            if is_archive(path) && path.contains('/') {
                self.issue(Severity::Warning, "packaging.nested_archive", path, Words::text(en("The game opens archives only at the mod's top level; this one, in a subfolder, is never read.")), en("Move the archive to the mod's top level, or unpack its contents into the mod's folders."));
            } else if let Some(home) = misplaced(path) {
                misplaced_by_top.entry(path.split('/').next().unwrap_or("")).or_default().push((path, home));
            }
            if self.halted { return; }
        }
        for (top, list) in misplaced_by_top {
            let wrapped = !top.is_empty() && !TA_FOLDERS.contains(&top) && list.iter().all(|(p, _)| p.split('/').nth(1).is_some_and(|second| TA_FOLDERS.contains(&second)));
            if wrapped {
                let message = enf("The game's folders are inside {folder}: the game reads units/, weapons/, maps/… only at the mod's top level, so nothing in it is used.", [("folder", Fill::name(format!("{top}/")))]);
                self.issue(Severity::Warning, "packaging.wrapped", top, message, en("Move the contents of this folder up to the mod's top level."));
                // (A folder has no layer of its own: it is the files'.)
                let layer = self.vfs.origin(list[0].0).map_or(String::new(), |o| o.layer.clone());
                if let Some(issue) = self.report.issues.last_mut() { issue.layer = layer; }
            } else if list.len() > 3 {
                let (first, home) = list[0];
                let message = if top.is_empty() {
                    enf("{n} files at the mod's top level are where the game never reads them; this one's kind is read from {home}.", [("n", Fill::number(list.len())), ("home", folders(home))])
                } else {
                    enf("{n} files under {folder} are where the game never reads them; this one's kind is read from {home}.", [("n", Fill::number(list.len())), ("folder", Fill::name(format!("{top}/"))), ("home", folders(home))])
                };
                self.issue(Severity::Warning, "packaging.misplaced", first, message, en("Move each file to the folder the game reads its kind from."));
            } else {
                for (path, home) in list {
                    let ext = path.rsplit_once('.').map_or("", |(_, e)| e);
                    self.issue(Severity::Warning, "packaging.misplaced", path, enf("The game never reads this file: it reads {kind} files from {home}.", [("kind", Fill::name(format!(".{ext}"))), ("home", folders(home))]), en("Move it to that folder, or remove it if it is not needed."));
                }
            }
        }
    }

    /// Whether a file is the mod's own (its layer's), where the check is of a mod.
    fn own(&self, path: &str) -> bool {
        self.options.target_layer.as_ref().is_some_and(|target| self.vfs.origin(path).is_some_and(|o| &o.layer == target))
    }

    /// A file that does not read, counted (and as the mod's, where it is).
    fn count_failure(&mut self, path: &str) {
        self.report.counts.files_failed += 1;
        if self.own(path) && let Some(own) = &mut self.report.counts.own { own.failed += 1; }
    }

    fn file(&mut self, path: &str, parsed: Parsed) {
        if self.own(path) {
            if !self.retain(96 + path.len(), path) { return; }
            self.relevant.insert(path.to_string());
        }
        let content = match parsed.content {
            Ok(content) => content,
            Err(Failure::Read(e)) => {
                self.count_failure(path);
                return self.issue(Severity::Error, "file.read", path, enf("This file cannot be read: {error}", [("error", Words::raw(e).into())]), en("Reinstall the package and check that its archives are intact and readable."));
            }
            Err(Failure::Invalid(e)) => {
                self.count_failure(path);
                return self.issue(Severity::Error, "format.invalid", path, unreadable(e), REEXPORT);
            }
            Err(Failure::TooLarge) => return self.stop("validation.limit", path, en("This text file exceeds the validator's per-file text budget and was not parsed."), en("Split large definition files into smaller files before validating again.")),
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
                let refs = textures.into_iter().map(|t| Reference::new("texture", &t, Words::text(en("Model texture")), Severity::Warning)).collect();
                self.record(key("file", path), Record { path: path.into(), refs, ..Default::default() });
            }
            Content::Script { pieces, functions, problems, patch_values } => {
                self.report.counts.scripts += 1;
                if !patch_values.is_empty() && self.own(path) {
                    self.engine.patch_scripts += 1;
                    self.engine.patch_values.extend(patch_values);
                }
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
                if self.own(path) && let Some(own) = &mut self.report.counts.own { own.maps += 1; }
                let refs = features.iter().map(|n| Reference::new("feature", n, Words::text(en("Map feature")), Severity::Warning)).collect();
                self.record(key("file", path), Record { path: path.into(), refs, ..Default::default() });
                let ota = format!("{}.ota", path.trim_end_matches(".tnt"));
                if !self.vfs.exists(&ota) { self.issue(Severity::Warning, "map.metadata", path, Words::text(en("The terrain has no matching OTA metadata and cannot appear as a normal selectable map.")), en("Include a matching maps/<name>.ota with a GlobalHeader and playable schemas.")); }
            }
            Content::Plain => {
                if path.ends_with(".pcx") { self.report.counts.pcx += 1; }
            }
            Content::Blank => {
                if self.retain(128 + 2 * path.len(), path) {
                    self.blanks.push(path.to_string());
                    self.blank_paths.insert(path.to_string());
                }
            }
            Content::Sound(problem) => {
                self.report.counts.sounds += 1;
                if self.own(path) && let Some(own) = &mut self.report.counts.own { own.sounds += 1; }
                if let Some(problem) = problem { self.issue(Severity::Warning, "sound.format", path, enf("This sound will not play: {problem}.", [("problem", problem.into())]), en("Save it as a WAV file in PCM (8 or 16-bit) or IMA ADPCM, as TA's own sounds are.")); }
            }
            Content::Text { doc, syntax } => {
                for (line, problem) in syntax {
                    self.issue_at(Severity::Warning, "text.syntax", path, Some(line), lint_words(&problem), en("Correct the brackets and braces around this line: the game reads the file as it is and skips or misplaces what follows."));
                    if self.halted { return; }
                }
                self.text(path, &doc);
            }
        }
    }

    /// One finding for a footprint outside what Reborn keeps (either side). A footprint of none is
    /// a mod's upgrade module's, which takes no ground (`UnitDef::takes_no_ground`).
    fn footprint(&mut self, path: &str, s: &TdfSection) {
        let range = 0..=crate::MAX_FOOTPRINT_SIDE as i64;
        let (x, z) = (s.get_i64("footprintx"), s.get_i64("footprintz"));
        if x.is_some_and(|n| !range.contains(&n)) || z.is_some_and(|n| !range.contains(&n)) {
            let side = |v: Option<i64>| v.map_or("?".to_string(), |n| n.to_string());
            let words = enf("[{section}] footprint {x} × {z} is clamped to 1..{max} cells a side by Reborn.", [("section", Fill::name(&s.name)), ("x", Fill::name(side(x))), ("z", Fill::name(side(z))), ("max", Fill::number(crate::MAX_FOOTPRINT_SIDE))]);
            self.issue(Severity::Warning, "definition.footprint", path, words, en("Use footprints within the supported range and regenerate the matching yard map."));
        }
    }

    fn text(&mut self, path: &str, doc: &TdfDocument) {
        if direct(path, "units", "fbi") {
            let Some(s) = doc.section("UNITINFO") else { self.issue(Severity::Error, "unit.header", path, Words::text(en("Missing [UNITINFO]; this unit is skipped.")), en("Add a UNITINFO section with a non-empty UnitName.")); return; };
            let Some(unit) = crate::UnitDef::from_tdf(s) else { self.issue(Severity::Error, "unit.name", path, Words::text(en("UNITINFO has no usable UnitName; this unit is skipped.")), en("Give the unit a unique, non-empty UnitName and update its references.")); return; };
            self.footprint(path, s);
            // (Its own script, else its model's, as the game runs it.)
            let vfs = self.vfs;
            let script = unit.script_path(|p| vfs.exists(p));
            let mut refs = vec![Reference::new("file", &asset("objects3d", &unit.object, "3do"), part(en("{name} model"), &unit.name), Severity::Error), Reference::new("file", &script, part(en("{name} script"), &unit.name), Severity::Error)];
            for (slot, w) in unit.weapons.iter().enumerate() { if let Some(w) = w { refs.push(Reference::new("weapon", w, enf("{name} {key}", [("name", Fill::name(&unit.name)), ("key", Fill::name(format!("Weapon{}", slot + 1)))]), Severity::Error)); } }
            for (name, what) in [(&unit.explode_as, en("{name} death explosion")), (&unit.self_destruct_as, en("{name} self-destruct explosion"))] { optional(&mut refs, "weapon", name, part(what, &unit.name), Severity::Warning); }
            optional(&mut refs, "feature", &unit.corpse, part(en("{name} wreckage"), &unit.name), Severity::Warning);
            // (A note: the game never acts on a unit's side, which only files it in the
            // encyclopaedia; TA: Escalation's units both sides build are `Side=ALL`.)
            optional(&mut refs, "side", &unit.side, part(en("{name} faction"), &unit.name), Severity::Info);
            // (NONE: a unit meant to be silent.)
            if !unit.sound_category.trim().eq_ignore_ascii_case("none") { optional(&mut refs, "sounds", &unit.sound_category, part(en("{name} sounds"), &unit.name), Severity::Warning); }
            if unit.can_move && !unit.can_fly { optional(&mut refs, "move", &unit.movement_class, part(en("{name} movement class"), &unit.name), Severity::Warning); }
            let facts = UnitFacts { name: unit.name.clone(), object: unit.object.clone(), script, builds: unit.worker_time > 0.0, takes_no_ground: unit.takes_no_ground, weapons: unit.weapons.clone().map(|w| w.is_some_and(|w| !w.trim().is_empty())) };
            self.record(key("unit", &unit.name), Record { path: path.into(), refs, unit: Some(facts) });
        } else if direct(path, "weapons", "tdf") {
            let own = self.own(path);
            for s in &doc.sections {
                let first = self.report.issues.len();
                let weapon = crate::WeaponDef::from_tdf(s);
                // (An ID of 0, or none, is no number of its own: TA's own leave it so.)
                if let Some(id) = s.get_i64("id").filter(|&id| id > 0) && self.retain(64 + weapon.name.len(), path) {
                    self.engine.weapon_ids.push((weapon.name.to_ascii_uppercase(), id, own));
                }
                let mut refs = Vec::new();
                if !weapon.model.is_empty() && matches!(weapon.render_type, crate::RenderType::Model | crate::RenderType::Bomb) { refs.push(Reference::new("file", &asset("objects3d", &weapon.model, "3do"), part(en("{name} projectile model"), &weapon.name), Severity::Warning)); }
                for (gaf, entry, label) in [(&weapon.explosion_gaf, &weapon.explosion_art, en("{name} explosion")), (&weapon.water_explosion_gaf, &weapon.water_explosion_art, en("{name} water explosion")), (&weapon.lava_explosion_gaf, &weapon.lava_explosion_art, en("{name} lava explosion"))] { animation_ref(&mut refs, gaf, entry, part(label, &weapon.name)); }
                for (sound, label) in [(&weapon.sound_start, en("{name} firing sound")), (&weapon.sound_hit, en("{name} impact sound")), (&weapon.sound_water, en("{name} water impact sound"))] { optional(&mut refs, "sound", sound, part(label, &weapon.name), Severity::Warning); }
                if let crate::RenderType::Other(n) = weapon.render_type { self.issue(Severity::Warning, "weapon.render_type", path, enf("{name} uses unsupported RenderType={n}; Reborn will use its fallback projectile appearance.", [("name", Fill::name(&weapon.name)), ("n", Fill::number(n))]), en("Use TA RenderType 0..7, or report the required extension to Reborn's maintainers.")); }
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
                if !f.object.is_empty() { refs.push(Reference::new("file", &asset("objects3d", &f.object, "3do"), part(en("{name} feature model"), &f.name), Severity::Warning)); }
                if f.is_sprite() {
                    // (The game draws a feature by these three; TA's others, its burn shadow, death
                    // and reclaim animations, it never reads.)
                    for (entry, label, shadow) in [(&f.seq_name, en("{name} animation"), false), (&f.seq_name_shadow, en("{name} shadow"), true), (&f.seq_name_burn, en("{name} burn animation"), false)] {
                        // TA's own shrubs use EMPTY to deliberately suppress shadows.
                        if shadow && entry.eq_ignore_ascii_case("empty") { continue; }
                        animation_ref(&mut refs, &f.filename, entry, part(label, &f.name));
                    }
                }
                for (name, label) in [(&f.feature_dead, en("{name} dead replacement")), (&f.feature_burnt, en("{name} burnt replacement")), (&f.feature_reclamate, en("{name} reclaimed replacement"))] { optional(&mut refs, "feature", name, part(label, &f.name), Severity::Warning); }
                optional(&mut refs, "weapon", &f.burn_weapon, part(en("{name} burning weapon"), &f.name), Severity::Warning);
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
                // (What the game plays: the keys it knows, a repeated one's last value.)
                let mut played: Vec<(&str, &str)> = Vec::new();
                for (entry, value) in &s.entries {
                    // (An empty value is none: the engine keeps the one before.)
                    if !played_sound_key(entry) || value.trim().is_empty() { continue; }
                    played.retain(|(key, _)| !key.eq_ignore_ascii_case(entry));
                    played.push((entry, value));
                }
                for (entry, value) in played { optional(&mut refs, "sound", value, enf("{category} {key} sound", [("category", Fill::name(s.name.to_ascii_uppercase())), ("key", Fill::name(entry))]), Severity::Warning); }
                self.record(key("sounds", &s.name), Record { path: path.into(), refs, ..Default::default() });
                if self.halted { break; }
            }
        } else if path == "gamedata/sidedata.tdf" {
            let own = self.own(path);
            if own && let Some(can_build) = doc.section("CANBUILD") {
                self.engine.ai_builders += can_build.sections.len();
                self.engine.ai_entries += can_build.sections.iter().map(|b| b.entries.iter().filter(|(_, unit)| !unit.trim().is_empty()).count()).sum::<usize>();
            }
            for s in &doc.sections {
                if s.name.to_ascii_lowercase().starts_with("side") {
                    let name = s.get("name").unwrap_or("");
                    let commander = s.get("commander").unwrap_or("");
                    if own && !name.trim().is_empty() && !["arm", "core"].contains(&name.trim().to_ascii_lowercase().as_str()) && !self.engine.sides.iter().any(|n| n.eq_ignore_ascii_case(name.trim())) {
                        self.engine.sides.push(name.trim().to_string());
                    }
                    if name.trim().is_empty() || commander.trim().is_empty() { self.issue(Severity::Error, "side.definition", path, enf("[{section}] needs a faction name and starting commander.", [("section", Fill::name(&s.name))]), en("Set name and commander to a valid faction and UnitName.")); }
                    let refs = vec![Reference::new("unit", commander, part(en("{name} starting commander"), name), Severity::Error)];
                    self.record(key("side", name), Record { path: path.into(), refs, ..Default::default() });
                }
            }
            let mut refs = Vec::new();
            if let Some(can_build) = doc.section("CANBUILD") {
                for builder in &can_build.sections {
                    // (A note: the game skips a list whose builder is missing.)
                    refs.push(Reference::new("unit", &builder.name, Words::text(en("Owner of a build list for the computer players")), Severity::Info));
                    for (_, unit) in &builder.entries {
                        optional(&mut refs, "unit", unit, part(en("{name} build option for the computer players"), &builder.name), Severity::Warning);
                        // (The computer players' list, TA 3.1's: no player's menu.)
                        if !unit.trim().is_empty() { self.menu_entries.push((key("unit", &builder.name), key("unit", unit), false)); }
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
                    refs.push(Reference::new("unit", builder, Words::text(en("Download build-menu owner")), Severity::Warning)); refs.push(Reference::new("unit", unit, part(en("{name} build option"), builder), Severity::Warning));
                    self.menu_entries.push((key("unit", builder), key("unit", unit), true));
                }
            }
            self.record(key("file", path), Record { path: path.into(), refs, ..Default::default() });
        } else if direct(path, "maps", "ota") {
            if doc.section("GlobalHeader").is_none() { self.issue(Severity::Warning, "map.header", path, Words::text(en("Missing [GlobalHeader]; this map has no usable metadata.")), en("Export a matching OTA with a GlobalHeader and the desired game-type schemas.")); }
            let map = crate::MapInfo::from_ota(doc);
            // (Without its terrain the game and the launcher leave the map out of their lists: its
            // description alone does no harm, a mod's for a map of a pack of its own, say.)
            let mut refs = vec![Reference::new("file", &format!("{}.tnt", path.trim_end_matches(".ota")), Words::text(en("Map terrain, without which the map is left out of the lists")), Severity::Warning)];
            for schema in &map.schemas {
                optional(&mut refs, "weapon", &schema.meteor_weapon, Words::text(en("Map meteor weapon")), Severity::Warning);
                for placed in &schema.placed { optional(&mut refs, "placed", &placed.what, Words::text(en("Map placed unit/feature")), Severity::Warning); }
            }
            self.record(key("file", path), Record { path: path.into(), refs, ..Default::default() });
        } else if direct(path, "guis", "gui") {
            // Unit production buttons (which pages the game reads is settled in
            // `references`, from its builders): generic command buttons are not units.
            let mut refs = Vec::new();
            // (The page's header, its first gadget 0: where it is on the screen and how tall; the
            // buttons' lowest edge, from the header's top; its unit buttons off TA's grid.)
            let (mut header, mut lowest, mut off_grid) = (None, 0, 0);
            for s in &doc.sections {
                let Some(common) = s.section("COMMON") else { continue };
                let (y, height) = (common.get_i64_or("ypos", 0), common.get_i64_or("height", 0));
                match common.get_i32_or("id", -1) {
                    0 if header.is_none() => header = Some((y, height)),
                    1 => {
                        lowest = lowest.max(y.saturating_add(height));
                        let n = common.get("name").unwrap_or("");
                        // commonattribs bit4 marks unit production. attribs bit32 also
                        // appears on empty IGPATCH slots (IGPATCH, IGPATCH2…) and stockpile
                        // buttons (commonattribs bit8), which are not missing unit names.
                        // (Its builder named once the pages are settled: `references`.)
                        if !n.is_empty() && !n.to_ascii_uppercase().starts_with("IGPATCH") && production(common) {
                            refs.push(Reference::new("unit", n, Words::default(), Severity::Warning));
                            if (common.get_i64_or("width", 0), height) != (TA_PLACE, TA_PLACE) { off_grid += 1; }
                        }
                    }
                    _ => {}
                }
            }
            if self.own(path) {
                // (Past a 4096-pixel screen: no layout of a screen, but numbers gone wrong.)
                let bottom = header.map(|(y, height): (i64, i64)| y.saturating_add(height.max(lowest)));
                if let Some(bottom) = bottom.filter(|b| (TA_SCREEN_HEIGHT + 1..=4096).contains(b)) {
                    self.engine.tall_pages += 1;
                    self.engine.tallest = self.engine.tallest.max(bottom);
                }
                if off_grid > 0 {
                    self.engine.shared_pages += 1;
                    self.engine.shared_buttons += off_grid;
                }
            }
            if self.retain(96 + path.len() + refs.iter().map(|r| 128 + r.key.len()).sum::<usize>(), path) { self.gui_pages.insert(path.into(), refs); }
        } else if doc.sections.is_empty() {
            self.issue(Severity::Info, "text.no_sections", path, Words::text(en("This tolerant TDF parse contains no sections; section-based consumers ignore it.")), en("Check the intended format and section headers if this file should define game content."));
        }
    }

    fn references(&mut self) {
        for (path, list) in std::mem::take(&mut self.shadowed) {
            let (what, later, whose) = list[0].clone();
            let message = if list.len() == 1 {
                enf("{what} defined here is never used: {later} ({whose}) defines it too and comes later, so the game uses that one.", [("what", what.into()), ("later", Fill::name(later)), ("whose", whose.into())])
            } else {
                enf("{n} definitions in this file are never used: other files define them too and come later, so the game uses those (for one, {what} is also in {later}, {whose}).", [("n", Fill::number(list.len())), ("what", what.into()), ("later", Fill::name(later)), ("whose", whose.into())])
            };
            self.issue(Severity::Warning, "definition.shadowed", &path, message, en("Replace those files in the mod (give yours their paths), or name your file so that it sorts after them."));
        }
        let mut records = std::mem::take(&mut self.records);
        self.report.counts.units = records.keys().filter(|k| k.starts_with("unit:")).count() as u64;
        self.report.counts.weapons = records.keys().filter(|k| k.starts_with("weapon:")).count() as u64;
        self.report.counts.features = records.keys().filter(|k| k.starts_with("feature:")).count() as u64;
        if self.report.counts.own.is_some() {
            let own = |kind: &str| records.iter().filter(|(k, r)| k.starts_with(kind) && self.own(&r.path)).count() as u64;
            let (units, weapons, features) = (own("unit:"), own("weapon:"), own("feature:"));
            if let Some(own) = &mut self.report.counts.own { (own.units, own.weapons, own.features) = (units, weapons, features); }
        }
        // The production pages the game reads, as it reads them: a builder's
        // guis/<name>1.gui, 2, 3… up to 16, stopping at the first missing page after the first.
        // A builder is a unit with build power (`WorkerTime`), whatever its Builder flag, as the
        // game has it: another's pages and menu entries offer nothing (Total Mayhem's and TA:
        // Escalation's silos list missile pseudo-units for their computer players).
        let builders: Vec<String> = records.values().filter_map(|r| r.unit.as_ref().filter(|u| u.builds).map(|u| u.name.to_ascii_lowercase())).collect();
        let mut options: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for builder in &builders {
            for page in 1..=16u32 {
                let path = format!("guis/{builder}{page}.gui");
                let Some(refs) = self.gui_pages.remove(&path) else {
                    if page > 1 { break; }
                    continue;
                };
                options.entry(key("unit", builder)).or_default().extend(refs.iter().map(|r| r.key.clone()));
                self.buildable.extend(refs.iter().map(|r| r.key.clone()));
                let refs = refs.into_iter().map(|r| Reference { what: part(en("{name} production button"), &builder.to_ascii_uppercase()), ..r }).collect();
                records.insert(key("gui", &path), Record { path, refs, ..Default::default() });
            }
        }
        for (builder, unit, menu) in std::mem::take(&mut self.menu_entries) {
            if records.get(&builder).and_then(|r| r.unit.as_ref()).is_some_and(|u| u.builds) {
                if menu { self.buildable.insert(unit.clone()); }
                options.entry(builder).or_default().push(unit);
            }
        }
        for (builder, offered) in &options {
            // (A building whose every option is a module it makes for itself, out of its build
            // stance: no nano spray, so no nano-lathe to ask of its script.)
            let mut known = offered.iter().filter_map(|o| records.get(o).and_then(|r| r.unit.as_ref())).peekable();
            if known.peek().is_some() && known.all(|u| u.takes_no_ground) {
                self.module_builders.insert(builder.clone());
            }
        }
        // Required launch errors still count when a mod replaces the units folder
        // without replacing inherited sidedata. Do not seed the whole sidedata file
        // as target content: it also contains CANBUILD, which would recursively blame
        // a one-weapon mod for unrelated warnings on every original build option.
        self.required.insert("gamedata/sidedata.tdf".into());
        if records.keys().any(|k| k.starts_with("unit:")) && !records.keys().any(|k| k.starts_with("side:")) {
            self.issue(Severity::Error, "side.missing", "gamedata/sidedata.tdf", Words::text(en("No playable side definitions were loaded.")), en("Include gamedata/sidedata.tdf or inherit the base game's valid sides."));
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
            // (What it names that is missing: told once a thing, with every entry that names it.)
            let mut missing: Vec<(&str, Severity, Vec<&Words>)> = Vec::new();
            for reference in &record.refs {
                if let Some(used) = resolve(&reference.key, self.vfs, &records, &self.blank_paths) {
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
                    match missing.iter_mut().find(|(key, _, _)| *key == reference.key) {
                        Some((_, severity, whats)) => {
                            *severity = severity.graver(reference.severity);
                            whats.push(&reference.what);
                        }
                        None => missing.push((&reference.key, reference.severity, vec![&reference.what])),
                    }
                }
                if self.halted { break; }
            }
            // (A GAF's entries are missing where the GAF is: told once, for the file.)
            let gone: BTreeSet<&str> = missing.iter().filter_map(|(key, _, _)| key.strip_prefix("file:")).collect();
            missing.retain(|(key, _, _)| key.strip_prefix("gaf:").and_then(|entry| entry.split_once('#')).is_none_or(|(gaf, _)| !gone.contains(gaf)));
            const HOW: &str = en("Include the referenced asset/definition, correct its name, or declare and install the required mod dependency. Names are case-insensitive.");
            // A model's textures: told once a model, every one named.
            let textures: Vec<(&str, Severity)> = missing.iter().filter_map(|(key, severity, _)| key.strip_prefix("texture:").map(|t| (t, *severity))).collect();
            if textures.len() > 1 && !self.halted {
                missing.retain(|(key, _, _)| !key.starts_with("texture:"));
                let severity = textures.iter().map(|(_, s)| *s).fold(Severity::Info, Severity::graver);
                let names = Fill::List(textures.iter().map(|(t, _)| Fill::name(t.to_ascii_uppercase())).collect());
                let first = self.report.issues.len();
                self.issue(severity, "reference.missing", &record.path, enf("Model texture: textures {names} are missing.", [("names", names)]), HOW);
                self.attribute(first, record_key);
            }
            for (missing_key, severity, whats) in missing {
                if self.halted { break; }
                let first = self.report.issues.len();
                let whats = Fill::List(whats.into_iter().map(|what| what.clone().into()).collect());
                let message = if file_of(missing_key).is_some_and(|path| self.blank_paths.contains(&path)) {
                    enf("{whats}: {what} is an empty file here, which blanks it out: nothing of it is read.", [("whats", whats), ("what", describe(missing_key).into())])
                } else {
                    enf("{whats}: {what} is missing.", [("whats", whats), ("what", describe(missing_key).into())])
                };
                self.issue(severity, "reference.missing", &record.path, message, HOW);
                self.attribute(first, record_key);
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
        if plan.drawn > atlas { self.issue(Severity::Warning, "render.sprite_capacity", "", enf("Referenced weapon explosions require at least {texels} padded sprite texels, beyond a {side} x {side} reference atlas, even drawn reduced. Frames will be omitted at this capacity; map features, generated effects and shelf packing need additional room.", [("texels", Fill::number(plan.drawn)), ("side", Fill::number(side))]), en("Reduce explosion frame sizes/counts or split optional effects. Test on the target GPU; this static check uses a reference atlas size.")); }
        for ((entry, frames, size), &scale) in used.iter().zip(&plan.scales) {
            let drawn = (f64::from(size.largest) / scale).ceil() as u32 + 2;
            if *frames == 0 || drawn > side {
                let first = self.report.issues.len();
                self.issue(Severity::Warning, "render.sprite_frames", &records[entry].path, enf("{what} has no frames or contains a frame larger than the {side}-pixel reference atlas side including padding.", [("what", describe(entry).into()), ("side", Fill::number(side))]), en("Include usable frames and resize oversized animation frames."));
                self.attribute(first, entry);
            }
            if self.halted { return; }
        }
        if plan.reduced() > 0 {
            let mut reduced: Vec<(u32, &str)> = used.iter().zip(&plan.scales).filter(|(_, s)| **s > 1.0).map(|((k, _, size), _)| (size.largest, k.rsplit_once('#').map_or(k.as_str(), |(_, e)| e))).collect();
            reduced.sort_by(|a, b| b.cmp(a));
            // (The three largest, and how many more: a note, which never decides the badge.)
            let mut named: Vec<Fill> = reduced.iter().take(3).map(|(px, name)| enf("{name} ({px} px)", [("name", Fill::name(name.to_ascii_uppercase())), ("px", Fill::number(px))]).into()).collect();
            if reduced.len() > 3 { named.push(enf("{n} more", [("n", Fill::number(reduced.len() - 3))]).into()); }
            let mi = |texels: u64| Fill::name(format!("{:.0}", texels as f64 / f64::from(1 << 20)));
            let words = enf("The weapons' explosions take {full} Mi texels as painted, more than the {budget} Mi the game gives them in a {side} x {side} atlas: {n} of them, larger than TA's own, are drawn reduced to {share}% of their texels across, each as big as painted ({named}).", [
                ("full", mi(plan.full)), ("budget", mi(gaf::explosion_budget(side))), ("side", Fill::number(side)), ("n", Fill::number(reduced.len())),
                ("share", Fill::name(format!("{:.0}", plan.density * 100.0))), ("named", Fill::List(named)),
            ]);
            self.note("render.sprite_reduced", "", words, en("Nothing to do: the game draws them so. Smaller paintings would keep more of their detail."));
        }
        let gaf_file = |key: &str| key.strip_prefix("gaf:").and_then(|k| k.rsplit_once('#')).map(|(path, _)| path.to_ascii_lowercase());
        let explosion_files: BTreeSet<String> = effect_entries.iter().filter_map(|k| gaf_file(k)).collect();
        let other_files: BTreeSet<String> = records.iter().filter(|(k, _)| !k.starts_with("weapon:")).flat_map(|(_, r)| r.refs.iter().filter_map(|x| gaf_file(&x.key))).collect();
        for (path, (frames, largest, pixels)) in std::mem::take(&mut self.too_big) {
            let lower = path.to_ascii_lowercase();
            if explosion_files.contains(&lower) && !other_files.contains(&lower) && !lower.starts_with("textures/") {
                let words = enf("Large: {frames} frames up to {largest} pixels across, {millions} million pixels in all, more than the game decodes at once. It reads them a frame at a time and draws them reduced, each as big as painted: a weapon's explosion is read so.", [("frames", Fill::number(frames)), ("largest", Fill::number(largest)), ("millions", Fill::name(format!("{:.0}", pixels as f64 / 1e6)))]);
                self.issue(Severity::Info, "render.sprite_large", &path, words, en("Nothing to do: the game draws it so. Smaller paintings would load faster and keep more of their detail."));
            } else {
                self.count_failure(&path);
                self.issue(Severity::Error, "format.invalid", &path, unreadable("invalid GAF: decoded content exceeds resource limit".into()), REEXPORT);
            }
            if self.halted { return; }
        }
    }

    /// What the mod uses of TA's later engines, and of what TA left to its own engine, and how
    /// the game plays it (`Report::engine`), in a fixed order: from its own files, as they were
    /// read, and those of its files the game does not read that tell of it.
    fn engine_uses(&mut self) {
        let facts = std::mem::take(&mut self.engine);
        let files = std::mem::take(&mut self.own_files);
        let mut uses: Vec<EngineUse> = Vec::new();
        let mut add = |code: &str, support: Support, words: Words, how: &str| uses.push(EngineUse { code: code.into(), support, message: words.english(), words, how: how.into() });
        // (The first three named, and how many more.)
        let named = |names: &[String]| {
            let mut named: Vec<Fill> = names.iter().take(3).map(|n| Fill::name(n.as_str())).collect();
            if names.len() > 3 { named.push(enf("{n} more", [("n", Fill::number(names.len() - 3))]).into()); }
            Fill::List(named)
        };
        if facts.ai_builders > 0 {
            add("engine.ai_lists", Support::Played,
                enf("Build lists for the computer players (TA 3.1's [CANBUILD]). Builders: {builders}; entries: {entries}.", [("builders", Fill::number(facts.ai_builders)), ("entries", Fill::number(facts.ai_entries))]),
                en("Computer players build from them. Build menus offer what the mod's menu pages and download lists put on them, as TA's did: what only these lists name is the computer players' alone."));
        }
        if facts.tall_pages > 0 {
            add("engine.tall_panels", Support::Played,
                enf("Side panels laid out for a screen {height} pixels tall, as the patched engines of later years drew them (TA's are 480). Pages: {pages}.", [("height", Fill::number(facts.tallest)), ("pages", Fill::number(facts.tall_pages))]),
                en("The classic interface lays them out as the mod does: its build places, page arrows and orders where the mod puts them."));
        }
        if facts.shared_buttons > 0 {
            add("engine.shared_places", Support::Played,
                enf("Build buttons off TA's grid of 64-pixel places: several sharing a place, or one taking several. Buttons: {buttons}; pages: {pages}.", [("buttons", Fill::number(facts.shared_buttons)), ("pages", Fill::number(facts.shared_pages))]),
                en("The classic interface lays them out as the mod does, and a click builds the unit of the button under it. The modern interface shows a place several buttons share as one picture, its parts marked."));
        }
        if facts.patch_scripts > 0 {
            let values = Fill::List(facts.patch_values.iter().map(|&v| Fill::number(v)).collect());
            add("engine.script_values", Support::Played,
                enf("Scripts asking the engine for unit values of TA's later patches ({values}). Scripts: {scripts}.", [("values", values), ("scripts", Fill::number(facts.patch_scripts))]),
                en("TA Reborn answers them as those engines did: TA 3.1's questions about units by their IDs (69 to 75), a later patch's cloak (111)."));
        }
        // (The mod's weapons numbered past what TA's own engine numbered, and those numbered as
        // another weapon is, the mod's or one it plays over.)
        let mut by_id: BTreeMap<i64, BTreeSet<&str>> = BTreeMap::new();
        for (name, id, _) in &facts.weapon_ids { by_id.entry(*id).or_default().insert(name.as_str()); }
        let mine: BTreeSet<(&str, i64)> = facts.weapon_ids.iter().filter(|(_, _, own)| *own).map(|(name, id, _)| (name.as_str(), *id)).collect();
        let high: Vec<i64> = mine.iter().map(|&(_, id)| id).filter(|&id| id > 255).collect();
        const BY_NAME: &str = en("TA Reborn finds a weapon by its name and never reads its ID: IDs make no difference here.");
        if let Some(&max) = high.iter().max() {
            add("engine.weapon_ids", Support::Played,
                enf("Weapon IDs past 255, the highest TA's own engine numbered (the weapon patches of later years went further). Weapons: {n}; the highest ID: {max}.", [("n", Fill::number(high.len())), ("max", Fill::number(max))]),
                BY_NAME);
        }
        let shared: Vec<String> = mine.iter().filter(|&&(_, id)| by_id.get(&id).is_some_and(|names| names.len() > 1)).map(|&(name, _)| name.to_string()).collect();
        if !shared.is_empty() {
            add("engine.weapon_ids_shared", Support::Played,
                enf("Weapons sharing an ID with another weapon: {names}.", [("names", named(&shared))]),
                BY_NAME);
        }
        if !facts.sides.is_empty() {
            add("engine.sides", Support::Played,
                enf("Sides of its own: {names}.", [("names", named(&facts.sides))]),
                en("Offered with the others when a skirmish or a multiplayer battle is set up."));
        }
        let campaigns = files.iter().filter(|p| direct(p, "camps", "tdf") && !p.eq_ignore_ascii_case("camps/example.tdf")).count();
        if campaigns > 0 {
            add("engine.campaigns", Support::Played,
                enf("Campaigns of its own: {n}.", [("n", Fill::number(campaigns))]),
                en("Played from the campaign screen, with TA's own."));
        }
        let profiles = files.iter().filter(|p| p.starts_with("ai/")).count();
        if profiles > 0 {
            add("engine.ai_profiles", Support::Unused,
                enf("Files for TA's own computer players (ai/): {n}.", [("n", Fill::number(profiles))]),
                en("Not used: TA Reborn's computer players are its own. They build from the mod's build lists and menus."));
        }
        let programs: Vec<String> = files.iter().filter(|p| is_program(p)).map(|p| p.rsplit('/').next().unwrap_or(p).to_string()).collect();
        if !programs.is_empty() {
            add("engine.programs", Support::Unused,
                enf("Programs and engine patches: {names}.", [("names", named(&programs))]),
                en("Not used: TA Reborn runs no program a mod brings. What a patch changed in TA's engine is there only where TA Reborn plays it itself, as told here."));
        }
        self.report.engine = uses;
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
                [_] => Words::text(en("This file is empty: TA's way of blanking out a file of the same name below it (the original game's, or that of a mod it is mounted over). The game reads nothing from it.")),
                _ => {
                    // (The first three, and how many more: a note, which never decides the badge.)
                    let mut named: Vec<Fill> = paths.iter().take(3).map(|path| Fill::name(path.as_str())).collect();
                    if paths.len() > 3 { named.push(enf("{n} more", [("n", Fill::number(paths.len() - 3))]).into()); }
                    enf("{n} empty files here ({files}): TA's way of blanking out files of the same names below them (the original game's, or those of a mod they are mounted over). The game reads nothing from them.", [("n", Fill::number(paths.len())), ("files", Fill::List(named))])
                }
            };
            self.issue(Severity::Info, "file.blank", &paths[0], message, en("Nothing to do if they are meant to remove those files. If one should hold data, export it again."));
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
                self.issue(Severity::Warning, "render.texture_frames", &path, enf("Texture {name} has {frames} frames; Reborn's model vertices address at most 255 and this entry uses fallback paint.", [("name", Fill::name(&name)), ("frames", Fill::number(frames))]), en("Reduce this texture entry to at most 255 frames."));
            } else if layers + frames as u64 >= capacity {
                let words = enf("Texture {name} ({frames} frames) falls back: the textures the models paint with fill the reference GPU profile's {banks} arrays of {layers} layers ({before} layers before it; one is reserved for fallback). Actual device limits may differ.", [
                    ("name", Fill::name(&name)), ("frames", Fill::number(frames)), ("banks", Fill::number(TEXTURE_BANKS)), ("layers", Fill::number(self.options.texture_array_layers)), ("before", Fill::number(layers)),
                ]);
                self.issue(Severity::Warning, "render.texture_layers", &path, words, en("Consolidate duplicate textures or reduce animation/team frame counts. Test on the target GPU; higher limits do not fix unsupported frame counts."));
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
            let missing: Vec<String> = script_names.difference(model_names).cloned().collect();
            let message = match missing.as_slice() {
                [] => None,
                [name] => Some(enf("{script} declares piece {piece}, which is absent from {model}. Any animation or position query using it has no matching model part.", [("script", Fill::name(&script)), ("piece", Fill::name(name)), ("model", Fill::name(&model))])),
                names => Some(enf("{script} declares pieces {pieces}, which are absent from {model}. Any animation or position query using them has no matching model part.", [("script", Fill::name(&script)), ("pieces", Fill::List(names.iter().map(|n| Fill::name(n.as_str())).collect())), ("model", Fill::name(&model))])),
            };
            if let Some(message) = message {
                self.issue(Severity::Warning, "script.model_piece", path, message, en("Confirm that these are intentional virtual/helper pieces; otherwise use matching model and COB files or align the BOS piece declarations with the model's piece names."));
                if self.halted { return; }
            }
        }
        // Where the engine asks a script for its weapons' muzzles and its nano-lathe (it
        // carries on without them, but it shows). Aim functions may be missing by design:
        // TA's aircraft fire fixed weapons wherever they fly.
        if let Some(functions) = self.functions.get(&script).cloned() {
            const QUERIES: [&str; 3] = ["QueryPrimary", "QuerySecondary", "QueryTertiary"];
            // (A note: mods give units dummy weapons, which need no muzzle. One a unit.)
            let lacking: Vec<(usize, &str)> = QUERIES.iter().enumerate().filter(|(slot, name)| unit.weapons[*slot] && !functions.contains(&name.to_ascii_lowercase())).map(|(slot, name)| (slot, *name)).collect();
            let message = match lacking.as_slice() {
                [] => None,
                [(slot, name)] => Some(enf("{unit} has {weapon} but its script {script} has no {function}: if the weapon fires, its shots leave from the unit's centre, not a barrel.", [
                    ("unit", Fill::name(&unit.name)), ("weapon", Fill::name(format!("Weapon{}", slot + 1))), ("script", Fill::name(&script)), ("function", Fill::name(*name)),
                ])),
                many => {
                    let slots = Fill::List(many.iter().map(|(slot, _)| Fill::name(format!("Weapon{}", slot + 1))).collect());
                    let names = Fill::Either(many.iter().map(|(_, name)| Fill::name(*name)).collect());
                    Some(enf("{unit} has {weapons} but its script {script} has no {functions}: if they fire, their shots leave from the unit's centre, not a barrel.", [("unit", Fill::name(&unit.name)), ("weapons", slots), ("script", Fill::name(&script)), ("functions", names)]))
                }
            };
            if let Some(message) = message {
                self.issue(Severity::Info, "script.weapon", path, message, en("Add the functions to the unit's BOS script, returning the pieces the shots leave from, and recompile."));
            }
            if unit.builds && !functions.contains("querynanopiece") && !self.module_builders.contains(record_key) {
                self.issue(Severity::Info, "script.builder", path, enf("{unit} builds but its script {script} has no QueryNanoPiece: its build spray comes from the unit's centre.", [("unit", Fill::name(&unit.name)), ("script", Fill::name(&script))]), en("Add QueryNanoPiece to the unit's BOS script, returning its nano-lathe piece, and recompile."));
            }
        }
        if self.buildable.contains(record_key) && !self.vfs.exists(&format!("unitpics/{}.pcx", unit.name.to_ascii_lowercase())) {
            let words = enf("{unit} can be built but has no picture ({picture}): TA's classic panels and the encyclopaedia show none; Reborn's own panel draws one from its model.", [("unit", Fill::name(&unit.name)), ("picture", Fill::name(format!("unitpics/{}.pcx", unit.name.to_ascii_lowercase())))]);
            self.issue(Severity::Info, "unit.picture", path, words, en("Add a 64 × 64 PCX of the unit to unitpics/."));
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

/// What to do about a file that does not read, whatever its kind.
const REEXPORT: &str = en("Re-export this asset in the original TA format, within Reborn's documented parser limits; replace truncated or corrupt files.");

/// A file the game cannot read: the reader's own words, shown as they are in every language.
fn unreadable(error: String) -> Words {
    enf("The game cannot read this file: {error}", [("error", Words::raw(error).into())])
}

/// A TDF file's structural problem in words (their English is the problem's own: a test holds
/// them to each other).
fn lint_words(problem: &LintProblem) -> Words {
    match problem {
        LintProblem::NameUnclosed(name) => enf("The section name [{name} has no closing ].", [("name", Fill::name(name))]),
        LintProblem::NoBody(name) => enf("[{name}] has no { } body after it: what follows is read into the section around it.", [("name", Fill::name(name))]),
        LintProblem::StrayClose => Words::text(en("This } closes no section.")),
        LintProblem::BlockUnclosed => Words::text(en("A { opened on this line is never closed.")),
        LintProblem::SectionUnclosed(name) => enf("[{name}] opened on this line is never closed: any section after it is read as part of it.", [("name", Fill::name(name))]),
        LintProblem::CommentUnended => Words::text(en("A /* comment opened on this line never ends: the rest of the file is read as a comment.")),
    }
}

/// The file a reference is to, where it is to one: a file's, a sound's.
fn file_of(reference: &str) -> Option<String> {
    reference.strip_prefix("file:").map(str::to_string).or_else(|| reference.strip_prefix("sound:").map(sound_path))
}

fn optional(refs: &mut Vec<Reference>, kind: &str, name: &str, what: Words, severity: Severity) {
    if !name.trim().is_empty() { refs.push(Reference::new(kind, name, what, severity)); }
}

fn animation_ref(refs: &mut Vec<Reference>, gaf: &str, entry: &str, what: Words) {
    if gaf.trim().is_empty() || entry.trim().is_empty() { return; }
    let path = asset("anims", gaf, "gaf");
    // The file edge survives failed parsing, making inherited parse errors relevant.
    refs.push(Reference::new("file", &path, what.clone(), Severity::Warning));
    refs.push(Reference::new("gaf", &format!("{path}#{entry}"), what, Severity::Warning));
}

/// What a reference leads to, where it is there.
/// (An empty file is there, but nothing: TA's way of blanking one out, `blanks`.)
fn resolve(reference: &str, vfs: &Vfs, records: &BTreeMap<String, Record>, blanks: &BTreeSet<String>) -> Option<Use> {
    if let Some(path) = file_of(reference) { return (vfs.exists(&path) && !blanks.contains(&path)).then_some(Use::File(path)); }
    if let Some(name) = reference.strip_prefix("placed:") { return [key("unit", name), key("feature", name)].into_iter().find(|k| records.contains_key(k)).map(Use::Record); }
    records.contains_key(reference).then(|| Use::Record(reference.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    /// A list's English, as a finding gives it.
    fn listed(items: &[&str]) -> String {
        enf("{items}", [("items", Fill::List(items.iter().map(|item| Fill::name(*item)).collect()))]).english()
    }

    // Every report the tests make has its findings' words checked (`assert_words`).
    fn validate_all(vfs: &Vfs) -> Report {
        checked(super::validate_all(vfs))
    }
    fn validate(vfs: &Vfs, options: &Options, progress: impl FnMut(Progress<'_>) -> bool) -> Report {
        checked(super::validate(vfs, options, progress))
    }
    fn checked(report: Report) -> Report {
        assert_words(&report);
        report
    }

    /// A finding's words: their English is its message, and every text in them, and what to do,
    /// is one the languages' files collect (marked `en`, or `enf`'s first argument, here).
    fn assert_words(report: &Report) {
        let sources = [include_str!("validation.rs"), include_str!("words.rs")].concat();
        let marked = |text: &str| sources.contains(&format!("en(\"{text}\")")) || sources.contains(&format!("enf(\"{text}\""));
        for issue in &report.issues {
            assert_eq!(issue.words.english(), issue.message, "{}", issue.code);
            for text in issue.words.texts() {
                assert!(marked(text), "{}: {text:?} is not marked for translation", issue.code);
            }
            assert!(issue.suggestion.is_empty() || marked(&issue.suggestion), "{}: what to do, {:?}, is not marked for translation", issue.code, issue.suggestion);
        }
        for used in &report.engine {
            assert_eq!(used.words.english(), used.message, "{}", used.code);
            for text in used.words.texts() {
                assert!(marked(text), "{}: {text:?} is not marked for translation", used.code);
            }
            assert!(marked(&used.how) || sources.contains(&format!("const BY_NAME: &str = en(\"{}\")", used.how)), "{}: how it plays, {:?}, is not marked for translation", used.code, used.how);
        }
    }

    /// The words of a TDF file's structural problems are the problems' own.
    #[test]
    fn a_files_structure_is_told_in_the_words_of_its_problems() {
        for problem in [LintProblem::NameUnclosed("A{x".into()), LintProblem::NoBody("B".into()), LintProblem::StrayClose, LintProblem::BlockUnclosed, LintProblem::SectionUnclosed("C".into()), LintProblem::CommentUnended] {
            assert_eq!(lint_words(&problem).english(), problem.to_string());
        }
    }

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
            self.put(layer, "units/example.fbi", b"[UNITINFO]{UnitName=EXAMPLE;ObjectName=example;Side=ARM;Builder=1;WorkerTime=100;}");
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
        assert_eq!(misplaced("units/boom.wav"), Some(&["sounds/"][..]));
        assert_eq!(misplaced("music/extra/track01.wav"), Some(&["sounds/"][..]), "the music folder's own tracks only");
        // (Four samples of 8-bit PCM.)
        let mut wav = b"RIFF".to_vec();
        for (tag, bytes) in [(&b""[..], 40u32.to_le_bytes().to_vec()), (b"WAVEfmt ", 16u32.to_le_bytes().to_vec())] { wav.extend_from_slice(tag); wav.extend(bytes); }
        for word in [1u16, 1] { wav.extend(word.to_le_bytes()); }
        for rate in [22050u32, 22050] { wav.extend(rate.to_le_bytes()); }
        for word in [1u16, 8] { wav.extend(word.to_le_bytes()); }
        wav.extend_from_slice(b"data"); wav.extend(4u32.to_le_bytes()); wav.extend([128u8; 4]);
        assert_eq!(sound_problem(&wav), None, "a WAV the game plays");
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
        f.put("mod", "guis/example1.gui", b"[A]{[COMMON]{id=1;name=ARMNEXT;attribs=2;}} [B]{[COMMON]{id=1;name=ARMMISSING;attribs=32;commonattribs=4;}} [C]{[COMMON]{id=1;name=ARMATTACK;attribs=64;}} [D]{[COMMON]{id=1;name=ARMMAKENUKE;attribs=32;commonattribs=8;}} [E]{[COMMON]{id=1;name=IGPATCH;attribs=32;commonattribs=0;}} [F]{[COMMON]{id=1;name=IGPATCH;attribs=32;commonattribs=4;}} [G]{[COMMON]{id=1;name=IGPATCH2;attribs=32;commonattribs=4;}} [H]{[COMMON]{id=1;name=HANDMADE_MISSING;attribs=32;}}");
        f.put("mod", "download/add.tdf", b"[MENUENTRY0]{UNITMENU=EXAMPLE;UNITNAME=MISSING_DOWNLOAD;} [MENUENTRY1]{UNITMENU=BLANK;UNITNAME=DISABLED_UNIT;} [MENUENTRY2]{UNITMENU=EXAMPLE;UNITNAME=MISSING_DOWNLOAD;}");
        let report = validate_all(&f.vfs(&["mod"]));
        let text = report.to_text();
        assert!(text.contains("ARMMISSING") && text.contains("MISSING_DOWNLOAD"), "{text}");
        // (A hand-made page without commonattribs: its production button, by its attribs.)
        assert!(text.contains("HANDMADE_MISSING"), "{text}");
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
        assert!(note.severity == Severity::Info && note.message.contains("2 of them") && note.message.contains("(BIGGER (600 px) and BIG (300 px))"), "{text}");
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
        // (A description without its terrain is left out of the lists: a warning, not an error.)
        assert!(report.issues.iter().any(|i| i.message.contains("the file maps/orphan.tnt") && i.message.contains("left out of the lists") && i.severity == Severity::Warning));
    }

    /// The original game's slips are no mod's: a mod's copy of one of its files carries the same
    /// finding in the same place (Total Mayhem's and TA: Escalation's copies of TA's
    /// `gamedata/sound.tdf`), and a mod's unit using one of its sound categories reaches a finding
    /// the original has without the mod. Both are there without the mod: inherited. A note only
    /// there with the mod is its doing.
    #[test]
    fn what_the_original_game_has_without_the_mod_is_not_the_mods() {
        let f = Fixture::new(); f.valid("base");
        let sounds = "[CAT]\n{\nselect1=nowhere;\n}\n";
        f.put("base", "gamedata/sound.tdf", sounds);
        f.put("mod", "gamedata/sound.tdf", sounds);
        f.put("mod", "units/scout.fbi", b"[UNITINFO]{UnitName=SCOUT;ObjectName=example;Side=ARM;SoundCategory=CAT;Weapon1=GUN;}");
        f.put("mod", "weapons/gun.tdf", b"[GUN]{range=100;}");
        let options = Options { target_layer: Some("mod".into()), ..Default::default() };
        let mut report = validate(&f.vfs(&["base", "mod"]), &options, |_| true);
        compare(&mut report, &validate_all(&f.vfs(&["base"])));
        let sound = report.issues.iter().find(|i| i.message.contains("NOWHERE")).expect("the missing sound is told");
        assert!(sound.origin == Origin::Inherited && !sound.affects_status, "{}", report.to_text());
        // (The unit's script, the example's, has no QueryPrimary: a note only there with the mod.)
        let note = report.issues.iter().find(|i| i.code == "script.weapon").expect("the note is told");
        assert!(note.origin != Origin::Inherited && note.affects_status, "{}", report.to_text());
        assert_eq!(report.status(), Status::Clean, "{}", report.to_text());
    }

    /// An empty file blanks the original game's out: what still uses it uses nothing, and is told
    /// so (an emptied model or script would leave a unit without one).
    #[test]
    fn an_emptied_file_still_used_is_told() {
        let f = Fixture::new(); f.valid("base");
        f.put("mod", "objects3d/example.3do", b"");
        let options = Options { target_layer: Some("mod".into()), ..Default::default() };
        let mut report = validate(&f.vfs(&["base", "mod"]), &options, |_| true);
        compare(&mut report, &validate_all(&f.vfs(&["base"])));
        let emptied = report.issues.iter().find(|i| i.code == "reference.missing").unwrap_or_else(|| panic!("{}", report.to_text()));
        assert!(emptied.message.contains("the file objects3d/example.3do is an empty file here") && emptied.affects_status, "{}", emptied.message);
        assert_eq!(report.status(), Status::Errors, "a unit without its model");
    }

    /// What the game never reads it never decodes: a picture among a mod's documents that is no
    /// PCX is told where the game would read one, not broken.
    #[test]
    fn files_the_game_never_reads_are_not_decoded() {
        let f = Fixture::new(); f.valid("base");
        f.put("mod", "docs/screenshot.pcx", b"not a picture");
        f.put("mod", "weapons/gun.tdf", b"[GUN]{range=100;}");
        let options = Options { target_layer: Some("mod".into()), ..Default::default() };
        let mut report = validate(&f.vfs(&["base", "mod"]), &options, |_| true);
        compare(&mut report, &validate_all(&f.vfs(&["base"])));
        assert!(!report.issues.iter().any(|i| i.code == "format.invalid"), "{}", report.to_text());
        assert_ne!(report.status(), Status::Errors, "{}", report.to_text());
    }

    /// Sound categories: only the keys the game plays are checked, a repeated key's last value.
    #[test]
    fn only_the_sounds_the_game_plays_are_checked() {
        let f = Fixture::new(); f.valid("mod");
        f.put("mod", "gamedata/sound.tdf", "[CAT]\n{\nload=never_played;\nselect1=first;\nselect1=example;\nok1=gone;\narrived1=kept;\narrived1=;\n}\n");
        f.put("mod", "sounds/example.wav", b"");
        let report = validate_all(&f.vfs(&["mod"]));
        let text = report.to_text();
        assert!(!text.contains("NEVER_PLAYED") && !text.contains("FIRST"), "{text}");
        assert!(text.contains("GONE"), "{text}");
        assert!(text.contains("KEPT"), "an empty value leaves the one before: {text}");
        assert!(played_sound_key("Select2") && played_sound_key("count9") && !played_sound_key("count10") && !played_sound_key("unload"));
    }

    /// Sounds as the game's decoder reads them: Humongous's blocks and headerless 8-bit samples
    /// (TA's own `honk.wav`) play; IMA ADPCM of other than 4 bits, and no channels, do not.
    #[test]
    fn sounds_are_judged_as_the_decoder_reads_them() {
        let wav = |tag: u16, channels: u16, bits: u16| {
            let mut b = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
            b.extend(16u32.to_le_bytes());
            b.extend(tag.to_le_bytes());
            b.extend(channels.to_le_bytes());
            b.extend(11025u32.to_le_bytes());
            b.extend((11025u32 * channels as u32).to_le_bytes());
            b.extend((channels * bits.div_ceil(8)).max(1).to_le_bytes());
            b.extend(bits.to_le_bytes());
            b.extend(b"data");
            b.extend(4u32.to_le_bytes());
            b.extend([0u8; 4]);
            b
        };
        assert_eq!(sound_problem(&wav(1, 1, 16)), None);
        assert!(sound_problem(&wav(0x11, 1, 3)).is_some_and(|p| p.english().contains("IMA ADPCM")));
        assert!(sound_problem(&wav(1, 0, 16)).is_some_and(|p| p.english().contains("0 channels")));
        assert!(sound_problem(b"DIGI\0\0\0\x10HSHD").is_some(), "no samples");
        let mut digi = b"DIGI\0\0\0\x1cHSHD\0\0\0\x08SDAT\0\0\0\x0c".to_vec();
        digi.extend([128u8; 4]);
        assert_eq!(sound_problem(&digi), None, "its samples");
        assert!(sound_problem(&digi[..20]).is_some(), "its samples cut off");
        assert!(sound_problem(b"OggS").is_some(), "an Ogg file without its stream");
        // (A page of one segment, its packet a stream's identification header.)
        let page = |packet: &[u8]| [b"OggS\0\x02".as_slice(), &[0; 20], &[1, packet.len() as u8], packet].concat();
        assert_eq!(sound_problem(&page(b"\x01vorbis\0\0\0\0")), None, "Vorbis");
        assert_eq!(sound_problem(&page(b"\x7fFLAC\x01\0")), None, "FLAC");
        assert!(sound_problem(&page(b"OpusHead\x01")).is_some(), "Opus, which the game does not decode");
        assert_eq!(sound_problem(&[page(b"fishead\0").as_slice(), &page(b"\x01vorbis\0")].concat()), None, "a stream after another");
        let tone: Vec<u8> = (0..2000).map(|i| (128.0 + 90.0 * (i as f64 / 20.0).sin()) as u8).collect();
        assert_eq!(sound_problem(&tone), None, "headerless 8-bit samples");
        assert!(sound_problem(b"just some words, not a sound at all").is_some());
    }

    /// A model's missing textures are told once, and a list repeats no label.
    #[test]
    fn a_models_missing_textures_are_told_once() {
        let f = Fixture::new(); f.valid("mod");
        f.put("mod", "objects3d/shield.3do", textured_model(&["S01", "S02", "S03", "S04"]));
        let report = validate_all(&f.vfs(&["mod"]));
        let textures: Vec<_> = report.issues.iter().filter(|i| i.message.starts_with("Model texture")).collect();
        assert_eq!(textures.len(), 1, "{}", report.to_text());
        assert!(textures[0].message.contains("textures S01, S02, S03 and S04 are missing"), "{}", textures[0].message);
        assert_eq!(listed(&["A", "B", "A", "A"]), "A (3 times) and B");
        assert_eq!(listed(&["A", "B", "C", "D", "E"]), "A, B, C, D and E", "every one named: compared by its message");
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
        f.put("mod", "units/copy.fbi", b"[UNITINFO]{UnitName=Example;ObjectName=example;Side=ARM;Builder=1;WorkerTime=100;}");
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
        f.put("mod", "units/corca2.fbi", b"[UNITINFO]{UnitName=CORCA2;ObjectName=example;Side=ARM;Builder=1;WorkerTime=100;}");
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

    /// A script asking the engine for unit values Reborn does not answer is told of them (a mod
    /// made for a later patch of the engine): those asked for, and those set, apart. Those it
    /// answers are not told, nor a value worked out as the script runs.
    #[test]
    fn scripts_asking_for_values_the_engine_does_not_answer_are_told() {
        use cob::op::*;
        let f = Fixture::new();
        f.valid("mod");
        let code = [
            PUSH_CONSTANT, 87, GET_UNIT_VALUE,
            PUSH_CONSTANT, 99, PUSH_LOCAL, 0, PUSH_CONSTANT, 0, PUSH_CONSTANT, 0, PUSH_CONSTANT, 0, GET,
            PUSH_CONSTANT, 5, PUSH_CONSTANT, 1, SET_VALUE,
            PUSH_CONSTANT, 90, PUSH_CONSTANT, 1, SET_VALUE,
            PUSH_CONSTANT, 11, GET_UNIT_VALUE,
            PUSH_LOCAL, 0, GET_UNIT_VALUE,
            RETURN,
        ];
        f.put("mod", "scripts/example.cob", script("base", &code));
        let report = validate_all(&f.vfs(&["mod"]));
        let told: Vec<&str> = report.issues.iter().filter(|i| i.code == "script.value").map(|i| i.message.as_str()).collect();
        assert_eq!(told.len(), 2, "{}", report.to_text());
        assert!(told[0].contains("asks the engine for unit value 87 and 99,"), "{}", told[0]);
        assert!(told[1].contains("sets unit value 90,"), "{}", told[1]);
    }

    /// What a mod uses of TA's later engines is told with how the game plays it, from the mod's
    /// own files only: the computer players' lists, side panels for a taller screen, buttons
    /// sharing places, the patched engines' script values, weapon IDs past TA's 255 or shared with
    /// another weapon (the one it plays over too), sides and campaigns of its own (not the engine's
    /// example), TA's AI profiles and programs, which are not used. A check of everything, or of a
    /// mod with none of it, tells none.
    #[test]
    fn what_a_mod_uses_of_the_later_engines_is_told_with_how_it_plays() {
        use cob::op::*;
        let f = Fixture::new();
        f.valid("base");
        f.put("base", "weapons/base.tdf", b"[BASEGUN]{id=5;} [UNNUMBERED]{range=10;}");
        f.valid("mod");
        f.put("mod", "gamedata/sidedata.tdf", b"[SIDE0]{name=ARM;commander=EXAMPLE;} [SIDE1]{name=Talon;commander=EXAMPLE;} [CANBUILD]{[EXAMPLE]{canbuild1=EXAMPLE;canbuild2=EXAMPLE;canbuild3=;} [OTHER]{canbuild1=EXAMPLE;}}");
        let button = |name: &str, x: u32, y: u32, w: u32, h: u32| format!("[GADGET]{{[COMMON]{{id=1;name={name};xpos={x};ypos={y};width={w};height={h};attribs=32;commonattribs=4;}}}}");
        let page = format!("[GADGET0]{{[COMMON]{{id=0;name=HEADER;xpos=0;ypos=128;width=128;height=640;}}}} {} {} {}", button("EXAMPLE", 0, 0, 64, 64), button("EXAMPLE", 0, 64, 18, 64), button("EXAMPLE", 46, 64, 18, 64));
        f.put("mod", "guis/example1.gui", page);
        f.put("mod", "guis/menu.gui", b"[GADGET0]{[COMMON]{id=0;name=HEADER;xpos=0;ypos=0;width=640;height=480;}}");
        f.put("mod", "scripts/example.cob", script("base", &[PUSH_CONSTANT, 71, GET_UNIT_VALUE, PUSH_CONSTANT, 111, PUSH_CONSTANT, 1, SET_VALUE, PUSH_CONSTANT, 5, GET_UNIT_VALUE, RETURN]));
        f.put("mod", "weapons/guns.tdf", b"[GUN1]{id=300;} [GUN2]{id=5;} [GUN3]{id=0;} [GUN4]{id=301;}");
        f.put("mod", "ai/profile.txt", b"AIBOOST");
        f.put("mod", "camps/talon.tdf", b"[HEADER]{campaignside=Talon;}");
        f.put("mod", "camps/example.tdf", b"[HEADER]{}");
        f.put("mod", "TotalA.exe", b"MZ");
        f.put("mod", "TAESC.dll", b"MZ");
        let vfs = f.vfs(&["base", "mod"]);
        let options = Options { target_layer: Some("mod".into()), ..Default::default() };
        let report = validate(&vfs, &options, |_| true);
        let codes: Vec<&str> = report.engine.iter().map(|e| e.code.as_str()).collect();
        assert_eq!(codes, ["engine.ai_lists", "engine.tall_panels", "engine.shared_places", "engine.script_values", "engine.weapon_ids", "engine.weapon_ids_shared", "engine.sides", "engine.campaigns", "engine.ai_profiles", "engine.programs"], "{}", report.to_text());
        let told = |code: &str| report.engine.iter().find(|e| e.code == code).unwrap();
        assert!(told("engine.ai_lists").message.contains("Builders: 2; entries: 3."), "{}", told("engine.ai_lists").message);
        assert!(told("engine.tall_panels").message.contains("768 pixels tall") && told("engine.tall_panels").message.ends_with("Pages: 1."), "{}", told("engine.tall_panels").message);
        assert!(told("engine.shared_places").message.contains("Buttons: 2; pages: 1."), "{}", told("engine.shared_places").message);
        assert!(told("engine.script_values").message.contains("(71 and 111). Scripts: 1."), "{}", told("engine.script_values").message);
        assert!(told("engine.weapon_ids").message.contains("Weapons: 2; the highest ID: 301."), "{}", told("engine.weapon_ids").message);
        assert!(told("engine.weapon_ids_shared").message.ends_with("another weapon: GUN2."), "BASEGUN's 5, not the unnumbered: {}", told("engine.weapon_ids_shared").message);
        assert!(told("engine.sides").message.contains("Talon") && !told("engine.sides").message.contains("ARM"), "{}", told("engine.sides").message);
        assert!(told("engine.campaigns").message.ends_with(": 1."), "the engine's example is no campaign: {}", told("engine.campaigns").message);
        assert!(told("engine.programs").message.contains("TotalA.exe") || told("engine.programs").message.to_lowercase().contains("totala.exe"), "{}", told("engine.programs").message);
        assert_eq!(told("engine.ai_profiles").support, Support::Unused);
        assert_eq!(told("engine.programs").support, Support::Unused);
        assert_eq!(told("engine.sides").support, Support::Played);
        assert!(report.engine.iter().all(|e| !report.issues.iter().any(|i| i.code == e.code)), "never findings");
        assert!(report.to_text().contains("How TA Reborn plays what it uses") && report.to_text().contains("[engine.ai_lists]"), "{}", report.to_text());
        // (Kept and read back as it was.)
        let kept: Report = serde_json::from_str(&serde_json::to_string(&report).unwrap()).unwrap();
        assert_eq!(kept.engine, report.engine);
        // The whole content checked: no mod's.
        assert!(validate_all(&vfs).engine.is_empty());
        // A mod with none of it, over the same: nothing told.
        let g = Fixture::new();
        g.valid("base");
        g.valid("plain");
        let plain = validate(&g.vfs(&["base", "plain"]), &Options { target_layer: Some("plain".into()), ..Default::default() }, |_| true);
        assert!(plain.engine.is_empty(), "{:?}", plain.engine);
        // A mod of a patch and TA's own computer players' files alone: nothing for the data checks,
        // and what it brings is told all the same.
        let h = Fixture::new();
        h.valid("base");
        h.put("patch", "TotalA.exe", b"MZ");
        h.put("patch", "ai/boost.txt", b"AIBOOST");
        let patch = validate(&h.vfs(&["base", "patch"]), &Options { target_layer: Some("patch".into()), ..Default::default() }, |_| true);
        assert!(patch.empty && patch.status() == Status::Empty, "{}", patch.to_text());
        let codes: Vec<&str> = patch.engine.iter().map(|e| e.code.as_str()).collect();
        assert_eq!(codes, ["engine.ai_profiles", "engine.programs"], "{}", patch.to_text());
        // A report kept before there were any reads as one with none.
        let old: Report = serde_json::from_str(r#"{"schema_version":12,"target_layer":"mod","complete":true,"counts":{"files_total":0,"files_checked":0,"files_failed":0,"units":0,"weapons":0,"features":0,"maps":0,"models":0,"scripts":0,"gafs":0,"pcx":0,"textures":0,"frames":0},"issues":[]}"#).unwrap();
        assert!(old.engine.is_empty());
    }

    /// TA 3.1's [CANBUILD] is the computer players' build list: a unit only it names is on no
    /// player's menu, so no panel misses its picture, and a missing one is told as the computer
    /// players' option. A download menu's unit is a player's, and its picture is missed.
    #[test]
    fn the_computer_players_lists_are_told_as_theirs() {
        let f = Fixture::new();
        f.valid("mod");
        f.put("mod", "units/aionly.fbi", b"[UNITINFO]{UnitName=AIONLY;ObjectName=example;Side=ARM;}");
        f.put("mod", "units/menuunit.fbi", b"[UNITINFO]{UnitName=MENUUNIT;ObjectName=example;Side=ARM;}");
        f.put("mod", "scripts/aionly.cob", script("base", &[cob::op::RETURN]));
        f.put("mod", "scripts/menuunit.cob", script("base", &[cob::op::RETURN]));
        f.put("mod", "gamedata/sidedata.tdf", b"[SIDE0]{name=ARM;commander=EXAMPLE;} [CANBUILD]{[EXAMPLE]{canbuild1=AIONLY;canbuild2=GONE;}}");
        f.put("mod", "download/menu.tdf", b"[MENUENTRY1]{UNITMENU=EXAMPLE;MENU=2;BUTTON=1;UNITNAME=MENUUNIT;}");
        let report = validate_all(&f.vfs(&["mod"]));
        let pictures: Vec<&str> = report.issues.iter().filter(|i| i.code == "unit.picture").map(|i| i.path.as_str()).collect();
        assert_eq!(pictures, ["units/menuunit.fbi"], "{}", report.to_text());
        assert!(report.issues.iter().any(|i| i.code == "reference.missing" && i.message.contains("EXAMPLE build option for the computer players")), "{}", report.to_text());
    }

    /// The mods' upgrades as the game reads them (Total Mayhem's geothermal plant, TA:
    /// Escalation's silos): a building with build power and no Builder flag has its page read,
    /// its module of no footprint is no clamped footprint, and the building, building nothing but
    /// its own modules out of its stance, is not asked for a nano-lathe. A silo with a Builder
    /// flag and no build power offers nothing: its page's pseudo-unit is neither missing nor one
    /// that can be built.
    #[test]
    fn the_mods_upgrades_are_read_as_the_game_reads_them() {
        let f = Fixture::new();
        f.valid("mod");
        let page = |unit: &str| format!("[A]{{[COMMON]{{id=1;name={unit};attribs=32;commonattribs=4;}}}}");
        f.put("mod", "units/geo.fbi", b"[UNITINFO]{UnitName=GEO;ObjectName=example;Side=ARM;Builder=0;WorkerTime=300;}");
        f.put("mod", "scripts/geo.cob", script_with("base", &["Create"], &[cob::op::RETURN]));
        f.put("mod", "guis/geo1.gui", page("GEOUP"));
        f.put("mod", "units/geoup.fbi", b"[UNITINFO]{UnitName=GEOUP;ObjectName=example;Side=ARM;FootprintX=0;FootprintZ=0;BMcode=1;MaxVelocity=1;Acceleration=0;}");
        f.put("mod", "units/silo.fbi", b"[UNITINFO]{UnitName=SILO;ObjectName=example;Side=ARM;Builder=1;}");
        f.put("mod", "guis/silo1.gui", page("MAKENUKE"));
        let report = validate_all(&f.vfs(&["mod"]));
        let text = report.to_text();
        assert!(report.issues.iter().any(|i| i.code == "unit.picture" && i.path == "units/geoup.fbi"), "the geothermal plant's page is read: its module can be built: {text}");
        assert!(!report.issues.iter().any(|i| i.code == "definition.footprint"), "a module's footprint of none is kept: {text}");
        assert!(!report.issues.iter().any(|i| i.code == "script.builder" && i.path == "units/geo.fbi"), "a building making its own modules has no spray: {text}");
        assert!(!text.contains("MAKENUKE"), "a silo without build power offers nothing: {text}");
    }

    /// A file's entries naming the same missing thing are one finding, the gravest of them, that
    /// lists them all (TA: Escalation's menus name units it does not ship, a builder at a time).
    #[test]
    fn a_missing_thing_is_told_once_a_file() {
        let f = Fixture::new();
        f.valid("mod");
        for builder in ["ARMCA", "ARMCK", "ARMCV", "ARMCH"] {
            f.put("mod", &format!("units/{}.fbi", builder.to_ascii_lowercase()), format!("[UNITINFO]{{UnitName={builder};ObjectName=example;Side=ARM;Builder=1;WorkerTime=100;}}"));
        }
        let entry = |n: u32, builder: &str| format!("[MENUENTRY{n}]{{UNITMENU={builder};MENU=1;BUTTON={n};UNITNAME=ARMCHEAT;}}");
        f.put("mod", "download/armcheat.tdf", [entry(1, "ARMCA"), entry(2, "ARMCK"), entry(3, "ARMCV"), entry(4, "ARMCH")].concat());
        let report = validate_all(&f.vfs(&["mod"]));
        let cheat: Vec<_> = report.issues.iter().filter(|i| i.code == "reference.missing" && i.message.contains("ARMCHEAT")).collect();
        assert_eq!(cheat.len(), 1, "{}", report.to_text());
        assert!(cheat[0].message.starts_with("ARMCA build option, ARMCK build option, ARMCV build option and ARMCH build option: unit ARMCHEAT is missing."), "{}", cheat[0].message);
        assert_eq!(listed(&["A", "B"]), "A and B");
        assert_eq!(listed(&["A", "B", "C"]), "A, B and C");
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
