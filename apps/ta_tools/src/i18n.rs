//! `ta-tools i18n <lang dir>`: collects the interface's translatable strings from the
//! sources into `<lang dir>/template.toml`, and adds new strings (empty) to the languages
//! already there. A string counts when the innermost call it is passed to translates it:
//! `t`, `tf` (its first argument), the `n` marker, and the shared widgets that call `t`
//! on their labels (`ActionButton::primary`, `setting`, `Panel::new`...). Tuples, arrays,
//! blocks and `Some(..)` in between are looked through, so `segmented(.., &[(x, "LOW")])`
//! and `setting(.., Some("hint"), ..)` count too.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Calls whose string arguments are translated (a `::` entry must match the path's end).
const TRANSLATING: &[&str] = &[
    "t", "tf", "n", "caption", "card_title", "primary", "secondary", "danger", "pill", "tag", "nav_item", "segmented", "segmented_fill", "Panel::new",
    "subtitle", "menu_button", "menu_entry", "menu_entry_beacon", "menu_item", "menu_item_beacon", "menu_back_entry", "group_heading", "back_button", "hero_button", "section", "setting", "switch_row",
    "setting_row", "toggle_row", "glow_title", "shadowed_title", "page_header", "paragraph", "small", "chip", "notify",
    "message", "t_str", "text_link",
];
/// Only the first string argument is the text (the rest are placeholder names).
const FIRST_ONLY: &[&str] = &["tf"];
/// Where the interface's sources are, from the repository root.
const SOURCES: &[&str] = &["apps/ta_game/src", "apps/ta_launcher/src", "crates/ta_ui/src"];
/// The mod check's words and the maps' origins (`ta_data`, which has no `t`: the launcher and
/// the game translate them): in these files only the texts marked `en`, and `enf`'s first
/// argument, count (their other calls, `doc.section("UNITINFO")`, are no text).
const DATA_SOURCES: &[&str] = &["crates/ta_data/src/validation.rs", "crates/ta_data/src/words.rs", "crates/ta_data/src/map.rs"];
const DATA_TRANSLATING: &[&str] = &["en", "enf"];
const DATA_FIRST_ONLY: &[&str] = &["enf"];

pub fn run(lang_dir: &Path) -> Result<()> {
    let root = std::env::current_dir()?;
    let mut files = Vec::new();
    for dir in SOURCES {
        collect_rs(&root.join(dir), &mut files);
    }
    files.sort();
    if files.is_empty() {
        anyhow::bail!("no sources found: run from the repository root");
    }
    // (file, strings in order of appearance), each string once.
    let mut seen = std::collections::HashSet::new();
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).with_context(|| f.display().to_string())?;
        let strings: Vec<String> = scan(&text).into_iter().filter(|s| seen.insert(s.clone())).collect();
        if !strings.is_empty() {
            let rel = f.strip_prefix(&root).unwrap_or(f).to_string_lossy().replace('\\', "/");
            groups.push((rel, strings));
        }
    }
    for rel in DATA_SOURCES {
        let f = root.join(rel);
        let text = std::fs::read_to_string(&f).with_context(|| f.display().to_string())?;
        // (Not their tests' words.)
        let code = text.split("#[cfg(test)]").next().unwrap_or(&text);
        let strings: Vec<String> = scan_with(code, DATA_TRANSLATING, DATA_FIRST_ONLY).into_iter().filter(|s| seen.insert(s.clone())).collect();
        if !strings.is_empty() {
            groups.push((rel.to_string(), strings));
        }
    }
    // The keys' descriptions, which the settings and the controls reference show (they live in
    // ta_config, which has no `t`).
    let actions: Vec<String> = ta_config::Keys::ACTIONS.iter().map(|a| a.1.to_string()).filter(|s| seen.insert(s.clone())).collect();
    if !actions.is_empty() {
        groups.push(("crates/ta_config/src/lib.rs (Keys::ACTIONS)".to_string(), actions));
    }
    // Achievement metadata lives below ta_ui to keep profile bookkeeping independent
    // of rendering. Include its public catalogue exactly once in the language template.
    let achievements: Vec<String> = ta_config::achievements::ALL.iter()
        .flat_map(|a| [a.name, a.description]).map(str::to_string)
        .filter(|s| seen.insert(s.clone())).collect();
    if !achievements.is_empty() {
        groups.push(("crates/ta_config/src/achievements.rs (ALL)".to_string(), achievements));
    }
    // The built-in scenarios' words (their files are compiled into the game, which shows them
    // through `t`; players' own scenarios stay as written).
    let mut scenarios: Vec<PathBuf> = std::fs::read_dir(root.join("apps/ta_game/scenarios")).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "toml")).collect()).unwrap_or_default();
    scenarios.sort();
    for f in scenarios {
        let table: toml::Table = std::fs::read_to_string(&f)?.parse().with_context(|| f.display().to_string())?;
        // (And its transmissions: who speaks, and what.)
        let messages = table.get("message").and_then(|m| m.as_array()).into_iter().flatten().filter_map(|m| m.as_table()).flat_map(|m| ["from", "text"].into_iter().filter_map(|k| m.get(k)?.as_str()));
        let strings: Vec<String> = ["title", "tagline", "briefing"].iter().filter_map(|k| table.get(*k)?.as_str()).chain(messages).map(|s| s.trim().to_string()).filter(|s| !s.is_empty() && seen.insert(s.clone())).collect();
        if !strings.is_empty() {
            let name = f.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            groups.push((format!("apps/ta_game/scenarios/{name}"), strings));
        }
    }
    let total: usize = groups.iter().map(|g| g.1.len()).sum();
    std::fs::create_dir_all(lang_dir)?;
    std::fs::write(lang_dir.join("template.toml"), render("", &groups, &BTreeMap::new()))?;
    println!("{total} strings from {} files -> {}", groups.len(), lang_dir.join("template.toml").display());

    // Languages: new strings added (empty), translations kept, unused ones listed last.
    for entry in std::fs::read_dir(lang_dir)?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|x| x.to_str()) != Some("toml") || path.file_stem().and_then(|s| s.to_str()) == Some("template") {
            continue;
        }
        let table: toml::Table = std::fs::read_to_string(&path)?.parse().with_context(|| path.display().to_string())?;
        let name = table.get("language").and_then(|l| l.get("name")).and_then(|n| n.as_str()).unwrap_or_default().to_string();
        let old: BTreeMap<String, String> = table
            .get("strings")
            .and_then(|s| s.as_table())
            .map(|t| t.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect())
            .unwrap_or_default();
        let done = groups.iter().flat_map(|g| &g.1).filter(|s| old.get(*s).is_some_and(|v| !v.is_empty())).count();
        std::fs::write(&path, render(&name, &groups, &old))?;
        println!("{}: {done} of {total} translated", path.display());
    }
    Ok(())
}

/// `s` as a TOML basic string on one line: a key may not span lines (a multi-line string as
/// a key makes the whole file unreadable), so newlines are written `\n`.
fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_rs(&p, out);
        } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
            out.push(p);
        }
    }
}

/// A language file: its name, the strings grouped by source file, then (commented as
/// such) translations no longer used.
fn render(name: &str, groups: &[(String, Vec<String>)], old: &BTreeMap<String, String>) -> String {
    let q = quoted;
    let mut out = String::new();
    out.push_str("# TA Reborn interface text. Copy template.toml to <code>.toml (e.g. de.toml), set the\n");
    out.push_str("# language's name and fill in the translations; empty ones keep the English. {name}\n");
    out.push_str("# placeholders stay as they are. `ta-tools i18n lang` adds new strings to every language.\n\n");
    out.push_str(&format!("[language]\nname = {}\n\n[strings]\n", q(name)));
    let mut used = std::collections::HashSet::new();
    for (file, strings) in groups {
        out.push_str(&format!("\n# {file}\n"));
        for s in strings {
            used.insert(s.as_str());
            out.push_str(&format!("{} = {}\n", q(s), q(old.get(s).map_or("", |v| v.as_str()))));
        }
    }
    let unused: Vec<(&String, &String)> = old.iter().filter(|(k, v)| !used.contains(k.as_str()) && !v.is_empty()).collect();
    if !unused.is_empty() {
        out.push_str("\n# No longer used by the interface:\n");
        for (k, v) in unused {
            out.push_str(&format!("{} = {}\n", q(k), q(v)));
        }
    }
    out
}

/// The translatable string literals of a Rust source file, in order.
fn scan(src: &str) -> Vec<String> {
    scan_with(src, TRANSLATING, FIRST_ONLY)
}

/// The string literals passed to the calls `translating` names (only the first argument of those
/// `first_only` names), in order.
fn scan_with(src: &str, translating: &[&str], first_only: &[&str]) -> Vec<String> {
    let b = src.as_bytes();
    let mut found = Vec::new();
    // Open calls and groups: (name, strings seen directly in it).
    let mut stack: Vec<(String, usize)> = Vec::new();
    // The path just read (`ActionButton::primary`), if the next `(` calls it.
    let mut path = String::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
                continue;
            }
            b'"' => {
                let (s, end) = string_literal(src, i);
                record(&mut stack, s, &mut found, translating, first_only);
                i = end;
                path.clear();
                continue;
            }
            b'r' if (b.get(i + 1) == Some(&b'"') || b.get(i + 1) == Some(&b'#')) && !prev_is_ident(b, i) => {
                if let Some((s, end)) = raw_literal(src, i) {
                    record(&mut stack, s, &mut found, translating, first_only);
                    i = end;
                    path.clear();
                    continue;
                }
            }
            b'b' if b.get(i + 1) == Some(&b'"') && !prev_is_ident(b, i) => {
                // Byte strings are never interface text.
                let (_, end) = string_literal(src, i + 1);
                i = end;
                continue;
            }
            b'\'' => {
                // A char literal ('a', '\n') or a lifetime ('a).
                if b.get(i + 1) == Some(&b'\\') {
                    i += 2;
                    while i < b.len() && b[i] != b'\'' {
                        i += 1;
                    }
                    i += 1;
                } else if b.get(i + 2) == Some(&b'\'') {
                    i += 3;
                } else {
                    let ch_len = src[i + 1..].chars().next().map_or(1, |c| c.len_utf8());
                    if b.get(i + 1 + ch_len) == Some(&b'\'') {
                        i += 2 + ch_len;
                    } else {
                        i += 1;
                    }
                }
                path.clear();
                continue;
            }
            b'(' => {
                stack.push((std::mem::take(&mut path), 0));
            }
            b'[' | b'{' => {
                stack.push((String::new(), 0));
                path.clear();
            }
            b')' | b']' | b'}' => {
                stack.pop();
                path.clear();
            }
            b':' if b.get(i + 1) == Some(&b':') => {
                if !path.is_empty() {
                    path.push_str("::");
                }
                i += 2;
                continue;
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                let ident = &src[start..i];
                if !path.ends_with("::") {
                    path.clear();
                }
                path.push_str(ident);
                // Macros (`format!(..)`) are calls of their own, never translating.
                if b.get(i) == Some(&b'!') {
                    path.push('!');
                    i += 1;
                }
                continue;
            }
            c if c.is_ascii_whitespace() => {}
            _ => path.clear(),
        }
        i += 1;
    }
    found
}

fn prev_is_ident(b: &[u8], i: usize) -> bool {
    i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_')
}

/// Records `s` when the innermost call around it (looking through groups, blocks and
/// `Some`) translates it.
fn record(stack: &mut [(String, usize)], s: String, found: &mut Vec<String>, translating: &[&str], first_only: &[&str]) {
    let direct = stack.len();
    for k in (0..stack.len()).rev() {
        let name = stack[k].0.as_str();
        if name.is_empty() || name == "Some" {
            continue;
        }
        let last = name.rsplit("::").next().unwrap_or(name);
        let translates = translating.iter().any(|&f| if f.contains("::") { name.ends_with(f) } else { last == f });
        if translates {
            let first_only = first_only.contains(&last);
            let is_first = stack[k].1 == 0 && k + 1 == direct;
            stack[k].1 += 1;
            if (!first_only || is_first) && !s.trim().is_empty() && s.chars().any(|c| c.is_alphabetic()) {
                found.push(s);
            }
        }
        return;
    }
}

/// A `"…"` literal starting at `start`: its value and the index after it.
fn string_literal(src: &str, start: usize) -> (String, usize) {
    let mut out = String::new();
    let mut chars = src[start + 1..].char_indices();
    while let Some((k, c)) = chars.next() {
        match c {
            '"' => return (out, start + 1 + k + 1),
            '\\' => match chars.next().map(|x| x.1) {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('0') => out.push('\0'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some('\'') => out.push('\''),
                Some('u') => {
                    let mut hex = String::new();
                    for (_, h) in chars.by_ref() {
                        match h {
                            '{' => {}
                            '}' => break,
                            h => hex.push(h),
                        }
                    }
                    if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        out.push(ch);
                    }
                }
                // A line continuation skips the newline and the next line's indent.
                Some('\n') | Some('\r') => {
                    let rest = chars.as_str();
                    let skip = rest.len() - rest.trim_start().len();
                    for _ in 0..rest[..skip].chars().count() {
                        chars.next();
                    }
                }
                Some(other) => out.push(other),
                None => {}
            },
            c => out.push(c),
        }
    }
    (out, src.len())
}

/// A raw literal (`r"…"`, `r#"…"#`) starting at `start`.
fn raw_literal(src: &str, start: usize) -> Option<(String, usize)> {
    let rest = &src[start + 1..];
    let hashes = rest.bytes().take_while(|&c| c == b'#').count();
    let body = rest[hashes..].strip_prefix('"')?;
    let close = format!("\"{}", "#".repeat(hashes));
    let end = body.find(&close)?;
    let consumed = 1 + hashes + 1 + end + close.len();
    Some((body[..end].to_string(), start + consumed))
}

#[cfg(test)]
mod tests {
    use super::{render, scan};
    use std::collections::BTreeMap;

    #[test]
    fn languages_read_back_whatever_the_strings() {
        let tricky = ["Commands: {c}\nCheat codes: {d}", "Saved \"{name}\" here", "back\\slash", "tab\there", "plain"];
        let groups = vec![("a.rs".to_string(), tricky.iter().map(|s| s.to_string()).collect())];
        let old: BTreeMap<String, String> = tricky.iter().map(|s| (s.to_string(), format!("<{s}>"))).collect();
        let text = render("Test", &groups, &old);
        let table: toml::Table = text.parse().expect("a language file parses");
        let strings = table["strings"].as_table().unwrap();
        for s in tricky {
            assert_eq!(strings[s].as_str(), Some(format!("<{s}>").as_str()));
        }
    }

    #[test]
    fn finds_what_the_interface_translates() {
        let src = r##"
            ui.add(ActionButton::primary("START"));
            setting(ui, "LEVEL", Some("A hint"), |ui| { ui.label("not me"); segmented(ui, &mut v, &[(1, "LOW"), (2, "HIGH")]); });
            let p = Panel::new("OPTIONS").subtitle("SOUND · GRAPHICS");
            let x = RichText::new("plain"); // "comment"
            tf("{n} units", &[("n", &3)]);
            t(if on { "ON" } else { "OFF" });
            caption(&format!("{} left", n));
            const TIPS: &[&str] = &[n("Build extractors."), n("Line two \
                continues")];
            let c = 'x'; let l: &'a str = t(r#"raw "quoted""#);
        "##;
        let got = scan(src);
        assert_eq!(
            got,
            ["START", "LEVEL", "A hint", "LOW", "HIGH", "OPTIONS", "SOUND · GRAPHICS", "{n} units", "ON", "OFF", "Build extractors.", "Line two continues", "raw \"quoted\""]
        );
    }

    /// The mod check's words: `en` and `enf`'s text only, what fills them and the file's other
    /// calls (a section's name) not.
    #[test]
    fn finds_the_mod_checks_words() {
        let src = r##"
            let Some(s) = doc.section("UNITINFO") else { return };
            self.issue(Severity::Error, "unit.header", path, Words::text(en("Missing [UNITINFO]; this unit is skipped.")), en("Add a UNITINFO section."));
            let words = enf("{whats}: {what} is missing.", [("whats", Fill::List(whats)), ("what", describe(key).into())]);
            const HOW: &str = en("Include the asset.");
        "##;
        assert_eq!(super::scan_with(src, super::DATA_TRANSLATING, super::DATA_FIRST_ONLY), ["Missing [UNITINFO]; this unit is skipped.", "Add a UNITINFO section.", "{whats}: {what} is missing.", "Include the asset."]);
    }
}
