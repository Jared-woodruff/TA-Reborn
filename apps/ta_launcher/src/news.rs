//! What is new: the changelog that ships with the game (`docs/CHANGELOG.md` in the package's
//! resources, or the repository's own in a build folder), its newest release's heading and the
//! paragraph that sums it up (the launcher shows it once after an update), and a release's
//! whole section, shown in a dialog ([`show`]: the changelog's headings, bold lines, lists
//! and paragraphs).

use std::path::{Path, PathBuf};

/// A release, as the changelog has it.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// "2.1.0".
    pub version: String,
    /// "2026-10-20".
    pub date: String,
    /// The paragraph under its heading (its lines joined).
    pub summary: String,
}

/// Where the changelog is: macOS bundle Resources, `docs` beside the programs, beside
/// them in older portable packages, or at the repository root above a Cargo build folder.
pub fn changelog(app_dir: &Path) -> Option<PathBuf> {
    ta_vfs::bundle_resource_dir(app_dir).map(|dir| dir.join("docs/CHANGELOG.md")).into_iter()
        .chain([app_dir.join("docs/CHANGELOG.md"), app_dir.join("CHANGELOG.md")])
        .chain(app_dir.ancestors().skip(1).take(3).map(|dir| dir.join("CHANGELOG.md")))
        .find(|path| path.is_file())
}

/// Separate the release date without splitting a prerelease version such as 2.1.0-rc.1.
fn release_heading(heading: &str) -> (&str, &str) {
    let (version, date) = heading.split_once(" — ").or_else(|| heading.split_once(" - ")).unwrap_or((heading, ""));
    (version.trim(), date.trim())
}

/// The newest release in a changelog (the "Unreleased" section skipped).
pub fn newest(text: &str) -> Option<Release> {
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Some(heading) = line.strip_prefix("## ") else { continue };
        let (version, date) = release_heading(heading);
        if !version.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        // The first paragraph after the heading.
        let mut summary = String::new();
        for line in lines.by_ref() {
            let line = line.trim();
            if line.is_empty() {
                if summary.is_empty() {
                    continue;
                }
                break;
            }
            if line.starts_with('#') || line.starts_with("- ") {
                break;
            }
            if !summary.is_empty() {
                summary.push(' ');
            }
            summary.push_str(line);
        }
        return Some(Release { version: version.to_string(), date: date.to_string(), summary });
    }
    None
}

/// A release's whole section (the lines under its heading, to the next release's).
pub fn section(text: &str, version: &str) -> Option<String> {
    let mut lines = text.lines();
    lines.by_ref().find(|l| l.strip_prefix("## ").is_some_and(|h| release_heading(h).0 == version))?;
    let body: Vec<&str> = lines.take_while(|l| !l.starts_with("## ")).collect();
    Some(body.join("\n").trim().to_string())
}

/// A summary in a few sentences at most (a long one cut at a sentence's end).
pub fn brief(summary: &str, sentences: usize) -> String {
    let mut out = String::new();
    for (k, sentence) in summary.split_inclusive(". ").enumerate() {
        if k == sentences {
            break;
        }
        out.push_str(sentence);
    }
    out.trim().to_string()
}

/// Shows changelog text (or a release's notes): `##` and `###` headings, lines in bold (and list
/// items all in bold) as small headings, `- ` lists (nested ones indented, their lines run on),
/// paragraphs; `**` and backquotes dropped, links read as their words, pictures left out.
pub fn show(ui: &mut egui::Ui, text: &str) {
    use crate::theme::*;
    use egui::RichText;
    // (Where a list item stands: not one, or one so deep in.)
    let mut block: Vec<String> = Vec::new();
    let mut bullet: Option<usize> = None;
    let flush = |ui: &mut egui::Ui, block: &mut Vec<String>, bullet: Option<usize>| {
        if block.is_empty() {
            return;
        }
        let text = plain(&block.join(" "));
        block.clear();
        match bullet {
            Some(depth) => {
                ui.horizontal_top(|ui| {
                    ui.add_space(18.0 * depth as f32);
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(RichText::new(if depth == 0 { "•" } else { "◦" }).font(strong(14.0)).color(AMBER));
                    ui.add(egui::Label::new(RichText::new(text).color(TEXT)).wrap());
                });
            }
            None => {
                ui.add(egui::Label::new(RichText::new(text).color(TEXT)).wrap());
            }
        }
        ui.add_space(4.0);
    };
    let small_heading = |ui: &mut egui::Ui, text: &str, depth: usize| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(18.0 * depth as f32);
            ui.label(RichText::new(plain(text).trim_end_matches(':').to_uppercase()).font(strong(12.5)).color(AMBER).extra_letter_spacing(ta_ui::i18n::tracking(1.4)));
        });
    };
    let bold = |s: &str| s.len() > 4 && s.starts_with("**") && s.ends_with("**") && !s[2..s.len() - 2].contains("**");
    for line in text.lines() {
        let trimmed = line.trim();
        let depth = (line.len() - line.trim_start().len()) / 2;
        if let Some(heading) = trimmed.strip_prefix("### ").or_else(|| trimmed.strip_prefix("## ")) {
            flush(ui, &mut block, bullet);
            bullet = None;
            ui.add_space(10.0);
            ta_ui::metal::embossed_label(ui, RichText::new(plain(heading).to_uppercase()).font(display(20.0)).extra_letter_spacing(ta_ui::i18n::tracking(1.2)), TEXT_BRIGHT);
            ui.add_space(4.0);
        } else if bold(trimmed) {
            flush(ui, &mut block, bullet);
            bullet = None;
            small_heading(ui, trimmed, 0);
        } else if let Some(item) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            flush(ui, &mut block, bullet);
            if bold(item.trim()) {
                // (An item all in bold heads the items under it.)
                small_heading(ui, item.trim(), depth);
                bullet = None;
            } else {
                bullet = Some(depth);
                block.push(item.to_string());
            }
        } else if trimmed.is_empty() {
            flush(ui, &mut block, bullet);
            bullet = None;
        } else {
            // (A list item's lines run on, indented; a paragraph's run on as they are.)
            if bullet.is_some() && !line.starts_with(' ') {
                flush(ui, &mut block, bullet);
                bullet = None;
            }
            block.push(trimmed.to_string());
        }
    }
    flush(ui, &mut block, bullet);
}

/// Markdown's marks taken out of a line: bold and code marks, a link as its words, a picture
/// as nothing.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let picture = open > 0 && rest[..open].ends_with('!');
        let link = rest[open..].find("](").and_then(|mid| rest[open + mid..].find(')').map(|end| (open + mid, open + mid + end)));
        match link {
            Some((mid, end)) if !rest[open + 1..mid].contains('[') => {
                out.push_str(&rest[..if picture { open - 1 } else { open }]);
                if !picture {
                    out.push_str(&rest[open + 1..mid]);
                }
                rest = &rest[end + 1..];
            }
            _ => {
                out.push_str(&rest[..=open]);
                rest = &rest[open + 1..];
            }
        }
    }
    out.push_str(rest);
    out.replace("**", "").replace('`', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_changelog_resolves_in_bundles_portables_and_target_builds() {
        let root = std::env::temp_dir().join(format!("ta_news_resources_{}", std::process::id()));
        let app = root.join("TA Reborn.app/Contents");
        let bundled = app.join("Resources/docs/CHANGELOG.md");
        std::fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        std::fs::write(&bundled, "bundled release notes").unwrap();
        std::fs::create_dir_all(app.join("MacOS/docs")).unwrap();
        std::fs::write(app.join("MacOS/docs/CHANGELOG.md"), "stale adjacent notes").unwrap();
        assert_eq!(changelog(&app.join("MacOS")), Some(bundled));

        let portable = root.join("portable");
        std::fs::create_dir_all(portable.join("docs")).unwrap();
        std::fs::write(portable.join("docs/CHANGELOG.md"), "portable notes").unwrap();
        assert_eq!(changelog(&portable), Some(portable.join("docs/CHANGELOG.md")));
        std::fs::remove_file(portable.join("docs/CHANGELOG.md")).unwrap();
        std::fs::write(portable.join("CHANGELOG.md"), "legacy portable notes").unwrap();
        assert_eq!(changelog(&portable), Some(portable.join("CHANGELOG.md")));

        let source = root.join("source");
        std::fs::create_dir_all(source.join("target/aarch64-apple-darwin/release")).unwrap();
        std::fs::write(source.join("CHANGELOG.md"), "development notes").unwrap();
        assert_eq!(changelog(&source.join("target/release")), Some(source.join("CHANGELOG.md")));
        assert_eq!(changelog(&source.join("target/aarch64-apple-darwin/release")), Some(source.join("CHANGELOG.md")));
        assert_eq!(changelog(&root.join("missing/one/two/three")), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_newest_release_is_read() {
        let text = "# Changelog\n\n## Unreleased\n\n- Something coming.\n\n## 2.1.0 — 2026-10-20\n\nA closer look at\neverything.\n\n### Part\n\n- A fix.\n\n## 2.0.0 — 2026-10-05\n\nOlder.\n";
        assert_eq!(newest(text), Some(Release { version: "2.1.0".into(), date: "2026-10-20".into(), summary: "A closer look at everything.".into() }));
        assert_eq!(newest("## 1.0.0\n- Only a list.\n").map(|r| r.summary), Some(String::new()));
        assert_eq!(newest("# Nothing released\n"), None);
        assert_eq!(section(text, "2.1.0").as_deref(), Some("A closer look at\neverything.\n\n### Part\n\n- A fix."));
        assert_eq!(section(text, "9.9.9"), None);
        assert_eq!(brief("One. Two. Three. Four.", 2), "One. Two.");
    }

    #[test]
    fn release_notes_match_the_complete_version() {
        let text = "## 2.1.01 — 2026-10-21\nOther patch.\n## 2.1.0-rc.1 - 2026-10-20\nPreview.\n## 2.1.0 — 2026-10-19\nStable.";
        assert_eq!(section(text, "2.1.0").as_deref(), Some("Stable."));
        let preview_version = "2.1.0-rc.1";
        assert_eq!(section(text, preview_version).as_deref(), Some("Preview."));
        assert_eq!(section(text, "2.1"), None);
        let preview = newest("## 2.1.0-rc.1 — 2026-10-20\nPreview.").unwrap();
        assert_eq!((preview.version.as_str(), preview.date.as_str()), ("2.1.0-rc.1", "2026-10-20"));
    }

    #[test]
    fn markdown_marks_are_taken_out() {
        assert_eq!(plain("**Bold** and `code`"), "Bold and code");
        assert_eq!(plain("Under the [GNU GPL](https://www.gnu.org/licenses/gpl-3.0.html), v3"), "Under the GNU GPL, v3");
        assert_eq!(plain("A picture ![emblem](docs/e.png) gone"), "A picture  gone");
        assert_eq!(plain("[not a link] and a ] alone ("), "[not a link] and a ] alone (");
    }

    /// The repository's changelog reads, and its newest release is this version or the one
    /// before it (the version is bumped as a release is cut).
    #[test]
    fn the_changelog_that_ships_reads() {
        let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("CHANGELOG.md")).unwrap();
        let release = newest(&text).expect("a release");
        assert!(!release.summary.is_empty(), "{release:?}");
        assert!(!crate::update::newer(&release.version, env!("CARGO_PKG_VERSION")), "{} is newer than this build", release.version);
    }
}
