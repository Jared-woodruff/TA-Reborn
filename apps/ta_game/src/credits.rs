//! What the credits say. It opens with Total Annihilation's own credits screen, in full and in
//! its own words (`guis/credits.gui`: the name, the copyright line and "Have a nice day"), goes
//! on with the teams of The Core Contingency, Battle Tactics and the TA editor as their readmes
//! in the player's install list them, and ends with TA Reborn's. Everything of the original is
//! read from the install when the credits open; none of it is in TA Reborn. (Text is translated
//! as it is put together.) The credits are played as a film, `credits_show.rs`.

use std::path::Path;

use ta_ui::i18n::{n, t, tf};
use ta_vfs::Vfs;

/// A part of the roll: a title, where it comes from, and its roles with their names.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub title: String,
    pub source: String,
    pub blocks: Vec<Block>,
}

/// A role and the people in it (a role may be empty: a line on its own).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Block {
    pub role: String,
    pub names: Vec<String>,
    /// Its names shown as they are written, never in capitals (TA Reborn's team: names and
    /// model designations as credited).
    pub as_written: bool,
}

impl Block {
    pub fn new(role: impl Into<String>, names: Vec<String>) -> Block {
        Block { role: role.into(), names, as_written: false }
    }
}

/// The original game's: TA's own credits screen, then the expansions' and the editor's
/// teams from their readmes (those the install has).
pub fn original(vfs: &Vfs, root: Option<&Path>) -> Vec<Section> {
    let mut sections = Vec::new();
    // TA's credits screen: every text on it but its caption and its button.
    if let Some(gui) = vfs.read_opt("guis/credits.gui") {
        let text = latin1(&gui);
        let lines: Vec<String> = text
            .lines()
            .filter_map(|l| l.trim().strip_prefix("text=").map(|v| v.trim_end_matches(';').trim().to_string()))
            .filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("credits") && !v.eq_ignore_ascii_case("ok"))
            .collect();
        if !lines.is_empty() {
            let blocks = lines[1..].iter().map(|l| Block::new("", vec![l.clone()])).collect();
            sections.push(Section { title: lines[0].to_uppercase(), source: t("The original game's credits screen, in full").into(), blocks });
        }
    }
    // (Each with what its names are, in the words of the screen's own above: not its file's.)
    let readmes = [
        ("taccread.txt", "TOTAL ANNIHILATION: THE CORE CONTINGENCY", n("The expansion's team, from its readme")),
        ("btreadme.txt", "TOTAL ANNIHILATION: BATTLE TACTICS", n("The mission pack's team, from its readme")),
        ("taeread.txt", "THE TOTAL ANNIHILATION EDITOR", n("The editor's team, from its readme")),
    ];
    for (file, title, source) in readmes {
        let Some(text) = root.and_then(|r| read_insensitive(r, file)) else { continue };
        let blocks = readme_credits(&text);
        if !blocks.is_empty() {
            sections.push(Section { title: title.into(), source: t(source).into(), blocks });
        }
    }
    sections
}

/// The band whose song the credits play, and the song (`credits_song.rs`): names, not translated.
pub const HADAL_STATIC: &str = ta_ui::credits::BAND;
pub const SONG: &str = ta_ui::credits::SONG;

/// TA Reborn's own.
pub fn reborn() -> Section {
    let block = |role: &str, names: &[&str]| Block::new(role, names.iter().map(|s| s.to_string()).collect());
    Section {
        title: "TA REBORN".into(),
        source: t("An unofficial reimplementation of Total Annihilation").into(),
        blocks: vec![
            // The team, each line a role and who has it, as credited (`ta_ui::credits`).
            Block { role: String::new(), names: ta_ui::credits::contributors(), as_written: true },
            // Where to find it, under the team (an address as written, not in capitals).
            Block { role: t("WEBSITE").into(), names: vec![ta_ui::credits::WEBSITE.into()], as_written: true },
            block(t("ORIGINAL ART"), &[t("The 3D trees, rocks and plants, Perspective's cockpits and the intro's room, made for TA Reborn in Blender")]),
            // (The credits' own song: `credits_song.rs`.)
            Block::new(t("MUSIC"), vec![HADAL_STATIC.into(), format!("\u{201c}{SONG}\u{201d}"), tf("© {band} · with permission", &[("band", &HADAL_STATIC)])]),
            block(t("TRANSLATIONS"), &[t("Thirteen languages beside English, machine-made; corrections from players who speak them are welcome")]),
            block(t("TESTING"), &[t("Everyone who played the test builds and said what needed fixing")]),
            // (The libraries by what they do: the language, graphics and the interface,
            // sound, speed, data, and the rest.)
            block(
                t("BUILT WITH"),
                &[
                    "Rust",
                    "wgpu · winit · egui · eframe",
                    "symphonia · cpal · rtrb",
                    "glam · rayon · rustc-hash · bytemuck",
                    "serde · toml · bincode · image · flate2 · memmap2",
                    "gilrs · rfd · clap · windows-rs · winreg",
                    t("and every library listed in THIRD-PARTY-NOTICES.txt"),
                ],
            ),
            block(t("DEVELOPMENT TOOLS"), &["Claude Code · Codex", "Blender · Python · NumPy · Pillow", "gltfpack · RenderDoc · ImageMagick · NSIS"]),
            block(
                t("TYPEFACE"),
                &[
                    t("Bahnschrift, from Windows"),
                    t("Sofia Sans by Lettersoup, where Windows' fonts are missing, under the SIL Open Font License"),
                    t("Chinese, Japanese and Korean: Microsoft YaHei, Microsoft JhengHei, Yu Gothic and Malgun Gothic, from Windows"),
                ],
            ),
            block(t("MAP DATA"), &[t("Made with Natural Earth: the coasts and lakes of Australia, the United States and the world"), "naturalearthdata.com"]),
            block(t("THANKS"), &[t("The Total Annihilation community, whose decades of file-format research made this possible"), t("Everyone who has kept Total Annihilation alive since 1997: map makers, unit makers, modders and players")]),
            block("", &[t("Total Annihilation is © its respective owners. TA Reborn is an unofficial fan project, not affiliated with or endorsed by them.")]),
        ],
    }
}

/// The credits in a readme: what follows its "Credits" heading (the last one: the first may
/// be the contents) up to the legal small print. Roles sit on a line of their own with the
/// names under them, or before a colon with the names after it ("Programming: A, B and C").
pub fn readme_credits(text: &str) -> Vec<Block> {
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).collect();
    let heading = |l: &str| {
        let l = l.trim().trim_matches('*').trim().to_ascii_lowercase();
        l == "credits" || l.ends_with(" credits") && l.split_whitespace().count() <= 3
    };
    let Some(start) = lines.iter().rposition(|l| heading(l)) else { return Vec::new() };
    let mut blocks = Vec::new();
    let mut group: Vec<String> = Vec::new();
    let flush = |group: &mut Vec<String>, blocks: &mut Vec<Block>| {
        if group.is_empty() {
            return;
        }
        if group.iter().all(|l| l.contains(':')) {
            for line in group.iter() {
                let (role, names) = line.split_once(':').unwrap_or_default();
                let names: Vec<String> = names.split(',').flat_map(|n| n.split(" and ")).map(|n| n.trim().trim_end_matches('.').to_string()).filter(|n| !n.is_empty()).collect();
                blocks.push(Block::new(role.trim(), names));
            }
        } else {
            let mut names: Vec<String> = Vec::new();
            for name in &group[1..] {
                // ("and" on a line of its own goes with the name after it.)
                match names.last_mut() {
                    Some(last) if last == "and" => *last = format!("and {name}"),
                    _ => names.push(name.clone()),
                }
            }
            blocks.push(Block::new(group[0].clone(), names));
        }
        group.clear();
    };
    for line in &lines[start + 1..] {
        let line = line.trim();
        if line.to_ascii_lowercase().starts_with("this program is protected") {
            break;
        }
        if line.is_empty() {
            flush(&mut group, &mut blocks);
        } else {
            group.push(line.to_string());
        }
    }
    flush(&mut group, &mut blocks);
    blocks
}

/// The readmes are Windows Latin-1 text.
fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// A file in `dir` by its name in any case.
fn read_insensitive(dir: &Path, name: &str) -> Option<String> {
    let entry = std::fs::read_dir(dir).ok()?.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name))?;
    std::fs::read(entry.path()).ok().map(|b| latin1(&b))
}

/// Every section of the credits, in the order they are told: TA's own, the expansions' and the
/// editor's, and TA Reborn's last. (`credits_show.rs` makes the film of them.)
pub fn sections(vfs: &Vfs) -> Vec<Section> {
    let mut sections = original(vfs, crate::install_root());
    sections.push(reborn());
    sections
}

/// TA Reborn's closing words, which the film shows over its emblem: what its last section ends
/// with (a block with no role) is taken from it and given here.
pub fn disclaimer(sections: &mut [Section]) -> Option<String> {
    let last = sections.last_mut()?;
    let at = last.blocks.iter().rposition(|b| b.role.is_empty() && b.names.len() == 1)?;
    Some(last.blocks.remove(at).names.remove(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readme_credits_read_both_ways() {
        let cc = "Contents\r\n\u{95}\tCredits\r\n\r\nblah\r\n**Credits**\r\n\r\nProject Leader\r\nChris\r\n\r\nSpecial Thanks\r\nShelley\r\nand\r\nKrogoth\r\n\r\n\r\nThis program is protected by law.\r\n";
        let blocks = readme_credits(cc);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0], Block::new("Project Leader", vec!["Chris".into()]));
        assert_eq!(blocks[1].names, vec!["Shelley".to_string(), "and Krogoth".to_string()]);
        let tae = "TAE CREDITS\r\n\r\nProgramming: Jason, Rick, Jon\r\n\r\nAlso thanks to: Chris, Rick Smith and Steve.\r\n";
        let blocks = readme_credits(tae);
        assert_eq!(blocks[0], Block::new("Programming", vec!["Jason".into(), "Rick".into(), "Jon".into()]));
        assert_eq!(blocks[1].names, vec!["Chris".to_string(), "Rick Smith".to_string(), "Steve".to_string()]);
    }

    #[test]
    fn the_install_has_its_credits() {
        let Ok(install) = ta_vfs::install::discover(None) else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let mut vfs = Vfs::new();
        vfs.mount_game_dir(&install.root, "base").unwrap();
        let sections = original(&vfs, Some(&install.root));
        for s in &sections {
            println!("{} ({}): {} roles", s.title, s.source, s.blocks.len());
        }
        assert!(sections.iter().any(|s| s.title == "TOTAL ANNIHILATION"));
        assert!(sections.iter().any(|s| s.blocks.iter().any(|b| b.role == "Musical Composition")), "the Core Contingency's credits");
    }
}
