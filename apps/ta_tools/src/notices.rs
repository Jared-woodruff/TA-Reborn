//! Third-party notices: every library compiled into the game and the launcher, its licence
//! and the licence texts, from `cargo metadata` (run from the repository root). Only what
//! ends up in the executables counts: normal dependencies, not build tools, and no
//! procedural macros (their code runs while compiling only).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;

/// The executables whose libraries are listed.
const ROOTS: [&str; 2] = ["ta_game", "ta_launcher"];

/// Standard licence texts (copies in `installer/licenses`) for libraries that ship none of
/// their own, in order of preference when a library offers a choice.
const STANDARD: [(&str, &str); 4] = [("MIT", "MIT.txt"), ("Apache-2.0", "Apache-2.0.txt"), ("MPL-2.0", "MPL-2.0.txt"), ("BSL-1.0", "BSL-1.0.txt")];

struct Library {
    name: String,
    version: String,
    licence: String,
    /// Where its source code is: the repository, else its crates.io page.
    source: String,
    authors: Vec<String>,
    dir: PathBuf,
}

/// Writes the notices for the libraries built for `target` (a target triple) to `out`.
pub fn run(out: &Path, target: &str) -> Result<()> {
    let standard_dir = Path::new("installer").join("licenses");
    if !standard_dir.is_dir() {
        bail!("{} not found: run from the repository root", standard_dir.display());
    }
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--filter-platform", target])
        .output()
        .context("running cargo metadata")?;
    if !output.status.success() {
        bail!("cargo metadata failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    let meta: Value = serde_json::from_slice(&output.stdout).context("reading cargo metadata")?;
    let version = meta["packages"]
        .as_array()
        .and_then(|ps| ps.iter().find(|p| p["name"] == ROOTS[0]))
        .and_then(|p| p["version"].as_str())
        .unwrap_or("?")
        .to_string();
    let libraries = libraries(&meta)?;

    // Every licence text once, with the libraries it applies to.
    let mut texts: Vec<(String, Vec<String>)> = Vec::new();
    let mut add = |text: String, whose: String| {
        let key = normalise(&text);
        match texts.iter_mut().find(|(t, _)| normalise(t) == key) {
            Some(entry) => entry.1.push(whose),
            None => texts.push((text, vec![whose])),
        }
    };
    let mut unlicensed = Vec::new();
    for lib in &libraries {
        let label = format!("{} {}", lib.name, lib.version);
        let files = licence_files(&lib.dir);
        let at_root = files.iter().any(|f| f.parent() == Some(lib.dir.as_path()));
        for f in &files {
            let rel = f.strip_prefix(&lib.dir).unwrap_or(f).to_string_lossy().replace('\\', "/");
            // A GPL text where the library also offers a permissive licence (the one used).
            if lib.licence.contains(" OR ") && rel.to_ascii_lowercase().contains("gpl") {
                continue;
            }
            let text = String::from_utf8_lossy(&std::fs::read(f)?).replace("\r\n", "\n");
            add(text.trim_end().to_string(), format!("{label} ({rel})"));
        }
        if !at_root {
            // No licence file of its own: the standard text of the licence it is used under.
            match chosen_licence(&lib.licence) {
                Some((id, file)) => {
                    let body = std::fs::read_to_string(standard_dir.join(file))?.replace("\r\n", "\n");
                    let text = if id == "MIT" { format!("Copyright (c) {}\n\n{body}", lib.authors.join(", ")) } else { body };
                    add(text.trim_end().to_string(), format!("{label} ({id}, standard text)"));
                }
                None => unlicensed.push(label),
            }
        }
    }
    if !unlicensed.is_empty() {
        bail!("no licence text found for: {}", unlicensed.join(", "));
    }
    // The font compiled in (`crates/ta_ui/fonts`): its licence goes with it.
    let ofl = std::fs::read_to_string(Path::new("crates").join("ta_ui").join("fonts").join("OFL.txt")).context("reading the bundled font's licence")?;
    add(ofl.replace("\r\n", "\n").trim_end().to_string(), format!("{SOFIA_SANS} (crates/ta_ui/fonts/OFL.txt)"));

    let mut s = String::new();
    let title = format!("TA Reborn {version} - third-party notices");
    let _ = writeln!(s, "{title}\n{}\n", "=".repeat(title.len()));
    s.push_str(
        "TA Reborn is licensed under the GNU General Public License, version 3 or any later version\n\
         (LICENSE.txt in the program's folder).\n\n\
         Its programs include the open-source libraries listed below. Their licences ask that\n\
         their notices go with the program: they follow in full, each text once with the\n\
         libraries it applies to.\n\n",
    );
    let mpl: Vec<&Library> = libraries.iter().filter(|l| l.licence.contains("MPL")).collect();
    if !mpl.is_empty() {
        s.push_str("The libraries under the Mozilla Public License 2.0 are used unmodified. Their source code\nis available at:\n");
        for l in &mpl {
            let _ = writeln!(s, "    {} {}: https://crates.io/crates/{}/{} ({})", l.name, l.version, l.name, l.version, l.source);
        }
        s.push('\n');
    }
    s.push_str("Total Annihilation's own data is not part of TA Reborn and is not covered here.\n\n");
    s.push_str(
        "The music of the credits, \"The War Reborn\" by Hadal Static\n\
         (assets/reborn/music/the_war_reborn.ogg), is (c) Hadal Static, all rights reserved, and is\n\
         used with permission. It is not covered by TA Reborn's licence\n\
         (assets/reborn/music/NOTICE.txt).\n\n",
    );
    let _ = writeln!(s, "{}\n", font_notice());

    let heading = format!("LIBRARIES ({})", libraries.len());
    let _ = writeln!(s, "{heading}\n{}\n", "-".repeat(heading.len()));
    let width = libraries.iter().map(|l| l.name.len() + l.version.len() + 1).max().unwrap_or(0);
    for l in &libraries {
        let _ = writeln!(s, "{:width$}  {}", format!("{} {}", l.name, l.version), l.licence);
        let _ = writeln!(s, "{:width$}  {}", "", l.source);
    }
    let heading = format!("LICENCE TEXTS ({})", texts.len());
    let _ = writeln!(s, "\n{heading}\n{}", "-".repeat(heading.len()));
    for (i, (text, whose)) in texts.iter().enumerate() {
        let _ = writeln!(s, "\n[{}] {}\n", i + 1, whose.join(", "));
        let _ = writeln!(s, "{text}");
    }
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(out, s)?;
    eprintln!("{}: {} libraries, {} licence texts", out.display(), libraries.len(), texts.len());
    Ok(())
}

/// The bundled font, as the licence texts name it.
const SOFIA_SANS: &str = "Sofia Sans 4.101";

/// What the notices say of the bundled font: whose it is, its licence, and what TA Reborn changed.
fn font_notice() -> String {
    format!(
        "The interface's typeface where Windows' Bahnschrift is missing (Linux, macOS) is {SOFIA_SANS}\n\
         by Lettersoup (Botio Nikoltchev and Ani Petrova), Copyright 2019 The Sofia Sans Project\n\
         Authors (https://github.com/lettersoup/Sofia-Sans), its Semi Condensed and Extra Condensed\n\
         variable fonts, compiled into the programs. It is licensed under the SIL Open Font License,\n\
         version 1.1, whose text follows with the libraries' licences below. TA Reborn's copy is\n\
         modified: its Cyrillic letters are mapped to the forms used in Russian and Ukrainian, which\n\
         the original keeps for its Russian language setting (tools/fonts/sofia_sans.py)."
    )
}

/// The third-party libraries in the executables, sorted by name and version.
fn libraries(meta: &Value) -> Result<Vec<Library>> {
    let empty = Vec::new();
    let packages: BTreeMap<&str, &Value> = meta["packages"].as_array().unwrap_or(&empty).iter().filter_map(|p| Some((p["id"].as_str()?, p))).collect();
    let nodes: BTreeMap<&str, &Value> = meta["resolve"]["nodes"].as_array().unwrap_or(&empty).iter().filter_map(|n| Some((n["id"].as_str()?, n))).collect();
    let proc_macro = |p: &Value| p["targets"].as_array().is_some_and(|ts| ts.iter().any(|t| t["kind"].as_array().is_some_and(|k| k.iter().any(|k| k == "proc-macro"))));
    let mut stack: Vec<&str> = packages.iter().filter(|(_, p)| ROOTS.iter().any(|r| p["name"] == *r)).map(|(id, _)| *id).collect();
    if stack.len() != ROOTS.len() {
        bail!("{ROOTS:?} not found in cargo metadata: run from the repository root");
    }
    let mut seen = BTreeSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) || packages.get(id).is_none_or(|p| proc_macro(p)) {
            continue;
        }
        for d in nodes.get(id).and_then(|n| n["deps"].as_array()).unwrap_or(&empty) {
            let normal = d["dep_kinds"].as_array().is_some_and(|ks| ks.iter().any(|k| k["kind"].is_null()));
            if let (true, Some(pkg)) = (normal, d["pkg"].as_str()) {
                stack.push(pkg);
            }
        }
    }
    // Ours are the workspace's members; procedural macros stay in the compiler. (A path
    // dependency outside the workspace is a third-party library patched in the repository,
    // under third_party/: listed, and said to be modified.)
    let ours: BTreeSet<&str> = meta["workspace_members"].as_array().unwrap_or(&empty).iter().filter_map(Value::as_str).collect();
    let mut out: Vec<Library> = seen
        .iter()
        .filter_map(|id| packages.get(id))
        .filter(|p| p["id"].as_str().is_some_and(|id| !ours.contains(id)) && !proc_macro(p))
        .map(|p| {
            let (name, version) = (p["name"].as_str().unwrap_or("?").to_string(), p["version"].as_str().unwrap_or("?").to_string());
            let licence = p["license"].as_str().map(str::to_string).or_else(|| p["license_file"].as_str().map(|f| format!("see {f}"))).unwrap_or_else(|| "unspecified".into());
            let mut source = p["repository"].as_str().map(str::to_string).unwrap_or_else(|| format!("https://crates.io/crates/{name}"));
            if p["source"].is_null() {
                source.push_str(&format!(" (modified for TA Reborn: third_party/{name}/TA_REBORN_PATCH.md in its source says how)"));
            }
            let authors = p["authors"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
            let dir = PathBuf::from(p["manifest_path"].as_str().unwrap_or("")).parent().map(Path::to_path_buf).unwrap_or_default();
            Library { name, version, licence, source, authors, dir }
        })
        .collect();
    out.sort_by(|a, b| (a.name.to_ascii_lowercase(), &a.version).cmp(&(b.name.to_ascii_lowercase(), &b.version)));
    Ok(out)
}

/// The licence a library without a licence file is used under: MIT when offered, else the
/// first standard one its expression names.
fn chosen_licence(expr: &str) -> Option<(&'static str, &'static str)> {
    let ids: Vec<&str> = expr.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-')).filter(|t| !t.is_empty()).collect();
    STANDARD.iter().copied().find(|(id, _)| ids.contains(id))
}

/// Licence files of a library: at its root (LICENSE*, COPYING*, NOTICE*...), and further
/// down those named like a licence, everything in a `licenses` folder, and a text beside a
/// font of the same name (fonts carry their licences that way).
fn licence_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(dir, 0, false, &mut found);
    found
}

fn walk(dir: &Path, depth: usize, in_licence_dir: bool, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    let lower = |p: &Path| p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let ext = |p: &Path| p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let stem = |p: &Path| p.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let fonts: Vec<String> = entries.iter().filter(|p| matches!(ext(p).as_str(), "ttf" | "otf")).map(|p| stem(p)).collect();
    for p in &entries {
        let name = lower(p);
        if p.is_dir() {
            let skip = ["target", ".git", "tests", "test", "benches", "examples", "fuzz"].contains(&name.as_str());
            if depth < 3 && !skip {
                walk(p, depth + 1, in_licence_dir || name.starts_with("license") || name.starts_with("licence"), found);
            }
            continue;
        }
        let text = matches!(ext(p).as_str(), "" | "txt" | "md" | "markdown" | "apache" | "apache2" | "mit" | "bsd" | "zlib");
        if !text {
            continue;
        }
        let named = ["license", "licence", "copying", "copyright", "notice", "unlicense"].iter().any(|k| name.starts_with(k));
        let below = depth > 0 && (name.contains("license") || name.contains("licence") || name == "ofl.txt" || name == "ufl.txt");
        let font_text = ext(p) == "txt" && fonts.contains(&stem(p));
        if named || below || font_text || in_licence_dir {
            found.push(p.clone());
        }
    }
}

/// A licence text with its layout ignored (the same licence wrapped differently is one).
fn normalise(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The workspace's own crates are left out; a library from crates.io is listed, and so is
    /// one patched in the repository (a path dependency outside the workspace), said to be
    /// modified.
    #[test]
    fn third_party_libraries_patched_in_the_repository_are_listed() {
        let package = |id: &str, name: &str, source: Option<&str>| {
            serde_json::json!({ "id": id, "name": name, "version": "1.0.0", "license": "MIT OR Apache-2.0", "source": source, "repository": format!("https://example.invalid/{name}"), "authors": [], "manifest_path": format!("/x/{name}/Cargo.toml"), "targets": [{ "kind": ["lib"] }] })
        };
        let meta = serde_json::json!({
            "packages": [
                package("game", "ta_game", None),
                package("launcher", "ta_launcher", None),
                package("lib", "ta_render", None),
                package("naga", "naga", None),
                package("wgpu", "wgpu", Some("registry+https://github.com/rust-lang/crates.io-index")),
            ],
            "workspace_members": ["game", "launcher", "lib"],
            "resolve": { "nodes": [
                { "id": "game", "deps": [{ "pkg": "lib", "dep_kinds": [{ "kind": null }] }] },
                { "id": "launcher", "deps": [] },
                { "id": "lib", "deps": [{ "pkg": "wgpu", "dep_kinds": [{ "kind": null }] }, { "pkg": "naga", "dep_kinds": [{ "kind": null }] }] },
                { "id": "wgpu", "deps": [] },
                { "id": "naga", "deps": [] },
            ] },
        });
        let listed = libraries(&meta).unwrap();
        let names: Vec<&str> = listed.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["naga", "wgpu"]);
        assert!(listed[0].source.contains("modified for TA Reborn"), "{}", listed[0].source);
        assert!(!listed[1].source.contains("modified"), "{}", listed[1].source);
    }

    /// The font compiled into the programs is named with its licence, the licence's text is in
    /// the repository to go with it, and the change made to it is said.
    #[test]
    fn the_bundled_font_and_its_licence_are_in_the_notices() {
        let notice = font_notice();
        for part in ["Sofia Sans", "SIL Open Font License", "Copyright 2019 The Sofia Sans Project", "modified"] {
            assert!(notice.contains(part), "{part:?} in {notice}");
        }
        let ofl = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/ta_ui/fonts/OFL.txt")).unwrap();
        assert!(ofl.starts_with("Copyright 2019 The Sofia Sans Project Authors") && ofl.contains("SIL OPEN FONT LICENSE Version 1.1"));
    }
}
