//! Interface translations. English text is the key: `t("SKIRMISH")` gives the current
//! language's text for it, or the English when it has none, so untranslated text still
//! reads. A language is a TOML file, `lang/<code>.toml`, beside the programs or in the
//! settings folder:
//!
//! ```toml
//! [language]
//! name = "Deutsch"
//!
//! [strings]
//! "SKIRMISH" = "GEFECHT"
//! "{done} of {total} done" = "{done} von {total} erledigt"
//! ```
//!
//! The shared widgets (`panel`, `widgets`) translate the labels they are given; other text
//! goes through [`t`], or [`tf`] when it carries values. `ta-tools i18n` writes a template
//! with every string it finds in the sources. The language `pseudo` brackets every string
//! that passes through here and accents its vowels, which shows the text that is not
//! translatable yet (it stays plain).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, RwLock};

struct Catalog {
    code: String,
    strings: HashMap<String, &'static str>,
    pseudo: bool,
}

/// A language other than English is loaded (the fast path skips the lookup otherwise).
static ACTIVE: AtomicBool = AtomicBool::new(false);
static CATALOG: RwLock<Option<Catalog>> = RwLock::new(None);
/// Counts the changes of language (the fonts follow: [`crate::theme`]).
static GENERATION: AtomicU32 = AtomicU32::new(0);
/// The language is Chinese, Japanese or Korean (set solid: [`tracking`]).
static CJK: AtomicBool = AtomicBool::new(false);
/// Pseudo-language strings made so far (bounded: text that changes every frame would
/// otherwise pile up).
static PSEUDO: Mutex<Option<HashMap<String, &'static str>>> = Mutex::new(None);
const PSEUDO_MAX: usize = 20_000;

/// The pseudo-language's code.
pub const PSEUDO_CODE: &str = "pseudo";

/// `text` in the current language (the English itself when there is no translation).
pub fn t(text: &str) -> &str {
    if !ACTIVE.load(Ordering::Relaxed) || text.trim().is_empty() {
        return text;
    }
    let pseudo = {
        let Ok(guard) = CATALOG.read() else { return text };
        let Some(c) = guard.as_ref() else { return text };
        if let Some(&s) = c.strings.get(text) {
            return s;
        }
        if !c.pseudo {
            note_missing(text);
        }
        c.pseudo
    };
    if pseudo {
        let mut made = PSEUDO.lock().unwrap_or_else(|e| e.into_inner());
        let made = made.get_or_insert_with(HashMap::new);
        if let Some(&s) = made.get(text) {
            return s;
        }
        if made.len() < PSEUDO_MAX {
            let s: &'static str = Box::leak(pseudo_text(text).into_boxed_str());
            made.insert(text.to_string(), s);
            return s;
        }
    }
    text
}

/// `TA_I18N_MISSING=1`: each text shown without a translation in the language in use is
/// logged once (finding what the template lacks; names from the game's data are among them).
fn note_missing(text: &str) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    static SEEN: Mutex<Option<std::collections::HashSet<String>>> = Mutex::new(None);
    if !*ON.get_or_init(|| std::env::var_os("TA_I18N_MISSING").is_some()) {
        return;
    }
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if seen.get_or_insert_with(Default::default).insert(text.to_string()) {
        log::info!("i18n: no translation for {text:?}");
    }
}

/// Marks text kept in constants (tips, lesson steps) for `ta-tools i18n`; it is
/// translated where it is shown, with [`t`].
pub const fn n(text: &str) -> &str {
    text
}

/// `text` in the current language with `{name}` placeholders filled in:
/// `tf("{n} units", &[("n", &count)])`.
pub fn tf(text: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut out = t(text).to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), &value.to_string());
    }
    out
}

/// A whole number with its digits in threes, as the language in use writes them: 1,250,000
/// in English, 1.250.000 in German, 1 250 000 in French (as the translations of the
/// achievements write theirs).
pub fn grouped(value: u64) -> String {
    grouped_in(value, &language())
}

fn grouped_in(value: u64, code: &str) -> String {
    let separator = match code.split('-').next().unwrap_or("") {
        "de" | "es" | "it" | "pt" | "tr" => '.',
        "fr" | "pl" | "ru" | "uk" => '\u{a0}',
        _ => ',',
    };
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 * 2);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) { out.push(separator); }
        out.push(c);
    }
    out
}

/// The code of the language in use ("" for English).
pub fn language() -> String {
    CATALOG.read().ok().and_then(|g| g.as_ref().map(|c| c.code.clone())).unwrap_or_default()
}

/// Goes up each time the language is set.
pub fn generation() -> u32 {
    GENERATION.load(Ordering::Relaxed)
}

/// The letter spacing of the interface's stencil-like capitals: `px` in Latin, Greek and
/// Cyrillic letters; Chinese, Japanese and Korean are set solid.
pub fn tracking(px: f32) -> f32 {
    if CJK.load(Ordering::Relaxed) { 0.0 } else { px }
}

/// Translated text in capitals, as the language writes them: in Turkish and Azerbaijani
/// i is İ (Rust's `to_uppercase` gives I).
pub fn upper(text: &str) -> String {
    let dotted = matches!(language().split('-').next(), Some("tr" | "az"));
    if dotted { text.replace('i', "İ").to_uppercase() } else { text.to_uppercase() }
}

/// Whether a language's code is Chinese, Japanese or Korean ("zh-Hans", "ja", "ko").
pub fn is_cjk(code: &str) -> bool {
    matches!(code.split('-').next().map(str::to_ascii_lowercase).as_deref(), Some("zh" | "ja" | "ko"))
}

/// Of `available` (from [`languages`]), the one for the computer's language `system` (a
/// tag such as "de-DE", "pt_PT.UTF-8" or "zh-TW"): the same tag, else the same language
/// (Portugal's Portuguese reads Brazil's); Chinese by its script, Taiwan's, Hong Kong's and
/// Macao's traditional. None for English, or a language without a translation.
pub fn closest(system: &str, available: &[(String, String)]) -> Option<String> {
    let tag = system.split(['.', '@']).next().unwrap_or("").replace('_', "-").to_ascii_lowercase();
    let mut parts = tag.split('-');
    let primary = parts.next().unwrap_or("");
    if primary.is_empty() || primary == "en" {
        return None;
    }
    let want = if primary == "zh" {
        if parts.any(|p| matches!(p, "hant" | "tw" | "hk" | "mo")) { "zh-hant".to_string() } else { "zh-hans".to_string() }
    } else {
        tag.clone()
    };
    let codes = || available.iter().map(|(c, _)| c).filter(|c| *c != "en" && *c != PSEUDO_CODE);
    codes().find(|c| c.to_ascii_lowercase() == want).or_else(|| codes().find(|c| c.split('-').next().is_some_and(|p| p.eq_ignore_ascii_case(primary)))).cloned()
}

/// Where languages are looked for: in macOS bundle Resources or beside the program,
/// in the settings folder (players' translations), and in a debug build the source tree.
pub fn language_dirs(settings_dir: &Path) -> Vec<PathBuf> {
    language_dirs_from_exe(settings_dir, std::env::current_exe().ok().as_deref())
}

fn language_dirs_from_exe(settings_dir: &Path, exe: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = exe.and_then(Path::parent) {
        // Match ta_vfs's bundle layout without introducing a UI -> VFS dependency.
        if let Some(contents) = dir.parent()
            && dir.file_name().is_some_and(|n| n == "MacOS")
            && contents.file_name().is_some_and(|n| n == "Contents")
            && contents.parent().and_then(Path::extension).is_some_and(|e| e.eq_ignore_ascii_case("app"))
        {
            dirs.push(contents.join("Resources/lang"));
        }
        dirs.push(dir.join("lang"));
    }
    dirs.push(settings_dir.join("lang"));
    if cfg!(debug_assertions) {
        dirs.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lang"));
    }
    dirs
}

/// The languages available: (code, name), English first, then by name. The template the
/// tools write (`template.toml`) is not one.
pub fn languages(dirs: &[PathBuf]) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            let Some(code) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else { continue };
            if path.extension().and_then(|x| x.to_str()) != Some("toml") || code == "template" || found.iter().any(|(c, _)| *c == code) {
                continue;
            }
            // (Only the head is read: thousands of strings follow it.)
            let name = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| text.split("\n[strings]").next().and_then(|head| head.parse::<toml::Table>().ok()))
                .and_then(|t| t.get("language")?.get("name")?.as_str().map(str::to_string))
                .unwrap_or_else(|| code.clone());
            found.push((code, name));
        }
    }
    found.sort_by_key(|a| a.1.to_lowercase());
    let mut all = vec![("en".to_string(), "English".to_string())];
    all.extend(found);
    if cfg!(debug_assertions) {
        all.push((PSEUDO_CODE.to_string(), "Pseudo (marks translatable text)".to_string()));
    }
    all
}

/// Switches the interface to `code` ("" or "en": English), read from the first of `dirs`
/// that has it. Unknown or unreadable languages leave English in place.
pub fn set_language(code: &str, dirs: &[PathBuf]) -> Result<(), String> {
    let code = code.trim();
    if code.is_empty() || code.eq_ignore_ascii_case("en") {
        install(None);
        return Ok(());
    }
    if code == PSEUDO_CODE {
        install(Some(Catalog { code: code.to_string(), strings: HashMap::new(), pseudo: true }));
        return Ok(());
    }
    let Some(path) = dirs.iter().map(|d| d.join(format!("{code}.toml"))).find(|p| p.is_file()) else {
        install(None);
        return Err(format!("language '{code}' not found"));
    };
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    match parse(&text) {
        Ok(strings) => {
            log::info!("language {code}: {} strings from {}", strings.len(), path.display());
            install(Some(Catalog { code: code.to_string(), strings, pseudo: false }));
            Ok(())
        }
        Err(e) => {
            install(None);
            Err(format!("{}: {e}", path.display()))
        }
    }
}

/// The `[strings]` of a language file (translations left empty are skipped).
fn parse(text: &str) -> Result<HashMap<String, &'static str>, String> {
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.message().to_string())?;
    let Some(strings) = table.get("strings").and_then(|s| s.as_table()) else { return Ok(HashMap::new()) };
    Ok(strings
        .iter()
        .filter_map(|(k, v)| v.as_str().filter(|v| !v.trim().is_empty()).map(|v| (k.clone(), &*Box::leak(v.to_string().into_boxed_str()))))
        .collect())
}

/// A drop-down of `languages` (from [`languages`]) editing a language code ("" = English).
/// Returns whether the choice changed.
pub fn picker(ui: &mut egui::Ui, id: &str, code: &mut String, languages: &[(String, String)]) -> bool {
    let current = if code.is_empty() { "en".to_string() } else { code.clone() };
    let shown = languages.iter().find(|(c, _)| *c == current).map_or(current.clone(), |(_, n)| n.clone());
    let mut changed = false;
    let list = egui::ComboBox::from_id_salt(id).selected_text(shown).width(ui.available_width()).show_ui(ui, |ui| {
        for (c, name) in languages {
            if ui.selectable_label(*c == current, name).clicked() && *c != current {
                *code = if c == "en" { String::new() } else { c.clone() };
                changed = true;
            }
        }
    });
    // (A list's row, as the interface's others sound.)
    if changed || list.response.clicked() {
        crate::sound::play(ui.ctx(), crate::sound::UiSound::Toggle);
    }
    changed
}

/// "[Skírmísh]": the pseudo-language's version of a string.
fn pseudo_text(text: &str) -> String {
    // (`{name}` placeholders stay as they are, or `tf` would not find them.)
    let mut placeholder = false;
    let accented: String = text
        .chars()
        .map(|c| match c {
            '{' => {
                placeholder = true;
                c
            }
            '}' => {
                placeholder = false;
                c
            }
            c if placeholder => c,
            'a' => 'á',
            'e' => 'é',
            'i' => 'í',
            'o' => 'ó',
            'u' => 'ú',
            'A' => 'Á',
            'E' => 'É',
            'I' => 'Í',
            'O' => 'Ó',
            'U' => 'Ú',
            c => c,
        })
        .collect();
    format!("[{accented}]")
}

fn install(catalog: Option<Catalog>) {
    let active = catalog.is_some();
    CJK.store(catalog.as_ref().is_some_and(|c| is_cjk(&c.code)), Ordering::Relaxed);
    if let Ok(mut guard) = CATALOG.write() {
        *guard = catalog;
    }
    ACTIVE.store(active, Ordering::Relaxed);
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_bundle_languages_resolve_without_the_working_directory() {
        let settings = Path::new("players/settings");
        let app = Path::new("Applications/TA Reborn.app/Contents");
        let dirs = language_dirs_from_exe(settings, Some(&app.join("MacOS/ta-launcher")));
        assert_eq!(&dirs[..3], &[app.join("Resources/lang"), app.join("MacOS/lang"), settings.join("lang")]);
        let portable = language_dirs_from_exe(settings, Some(Path::new("opt/ta-reborn/ta-launcher")));
        assert_eq!(&portable[..2], &[PathBuf::from("opt/ta-reborn/lang"), settings.join("lang")]);
    }

    #[test]
    fn numbers_are_grouped_as_each_language_writes_them() {
        assert_eq!(grouped_in(0, ""), "0");
        assert_eq!(grouped_in(999, ""), "999");
        assert_eq!(grouped_in(1000, ""), "1,000");
        assert_eq!(grouped_in(1_250_000, "en"), "1,250,000");
        assert_eq!(grouped_in(1_250_000, "de"), "1.250.000");
        assert_eq!(grouped_in(275_800, "pt-BR"), "275.800");
        assert_eq!(grouped_in(10_000, "fr"), "10\u{a0}000");
        assert_eq!(grouped_in(10_000, "ja"), "10,000");
        assert_eq!(grouped_in(u64::MAX, ""), "18,446,744,073,709,551,615");
    }

    // (Switching the language is tests/i18n.rs's, in a process of its own: the language is the
    // process's, and the tests here, side by side on its threads, read their text in English.)

    #[test]
    fn the_computers_language_finds_its_translation() {
        let available: Vec<(String, String)> = ["en", "de", "pt-BR", "zh-Hans", "zh-Hant", "ja", PSEUDO_CODE].iter().map(|c| (c.to_string(), c.to_string())).collect();
        assert_eq!(closest("de-DE", &available).as_deref(), Some("de"));
        assert_eq!(closest("de_AT.UTF-8", &available).as_deref(), Some("de"));
        assert_eq!(closest("pt-BR", &available).as_deref(), Some("pt-BR"));
        assert_eq!(closest("pt-PT", &available).as_deref(), Some("pt-BR"));
        assert_eq!(closest("zh-CN", &available).as_deref(), Some("zh-Hans"));
        assert_eq!(closest("zh-TW", &available).as_deref(), Some("zh-Hant"));
        assert_eq!(closest("zh-Hant-HK", &available).as_deref(), Some("zh-Hant"));
        assert_eq!(closest("ja-JP", &available).as_deref(), Some("ja"));
        assert_eq!(closest("en-AU", &available), None);
        assert_eq!(closest("nb-NO", &available), None);
        assert_eq!(closest("", &available), None);
    }
}
