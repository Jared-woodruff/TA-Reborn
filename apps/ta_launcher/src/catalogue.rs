//! Community mods, their preferred project pages, and tested release files. ModDB
//! releases use its resolver; manual-only verified releases retain archive metadata.
//! Projects without a tested current release are explicitly marked unverified.

use ta_ui::i18n::n;

/// How a mod fared in TA Reborn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// Installs, loads, and the computer players build and fight with it.
    Plays,
    /// Plays, with gaps (see its note).
    Partly,
    /// Current releases come from the project; local imports have not been tested here.
    Unverified,
    /// Nothing to install from the catalogue (the card links its page).
    NoFile,
}

/// Where the release is published and credited.
#[derive(Clone, Copy, Debug)]
pub enum Provider {
    ModDb(&'static str),
    Website { url: &'static str, name: &'static str },
}

#[derive(Clone, Copy, Debug)]
pub enum Download {
    /// Import a verified archive obtained through the creator's website. Never fetched here.
    ManualOnly,
    ModDb { page: &'static str, start: u32 },
    /// Primary URL followed by backups, all for the same verified file.
    #[cfg_attr(not(test), allow(dead_code))] // Supported/tested provider; no current catalogue entry uses it.
    Direct(&'static [&'static str]),
}

/// The exact release tested here, whether fetched online or imported locally.
#[derive(Clone, Copy, Debug)]
pub struct File {
    pub download: Download,
    pub name: &'static str,
    pub version: &'static str,
    pub bytes: u64,
    /// Host-published MD5, or computed alongside the tested archive's SHA-256.
    pub md5: &'static str,
    /// Required for direct downloads and manual-only imports; pinned from the verified archive.
    pub sha256: &'static str,
    /// Release/publication date in ISO order, when known. Empty means unknown.
    pub date: &'static str,
}

/// Public release details are useful even before an archive can be tested here.
/// They never authorize an install or replace a verified file's digest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Release {
    pub version: &'static str,
    pub date: &'static str,
    pub bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CatalogueSort {
    #[default]
    Spotlight,
    Newest,
    Oldest,
    Smallest,
    Largest,
}

impl CatalogueSort {
    pub fn label(self) -> &'static str {
        match self {
            Self::Spotlight => n("Spotlight first"),
            Self::Newest => n("Newest release"),
            Self::Oldest => n("Oldest release"),
            Self::Smallest => n("Smallest download"),
            Self::Largest => n("Largest download"),
        }
    }
}

impl File {
    pub fn can_download(self) -> bool {
        !matches!(self.download, Download::ManualOnly)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Link {
    pub label: &'static str,
    pub url: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct Entry {
    /// The mod's id (its folder) once installed.
    pub id: &'static str,
    pub name: &'static str,
    pub source: Provider,
    /// Preferred project page, independent of the tested file's download provider.
    pub project: Option<&'static str>,
    /// The credited creators.
    pub team: &'static str,
    /// What it is, in a sentence.
    pub pitch: &'static str,
    pub tags: &'static [&'static str],
    pub fit: Fit,
    /// What to know before playing it ("" when nothing).
    pub note: &'static str,
    /// A picture of it (ModDB's thumbnail of a download's screenshot).
    pub image: &'static str,
    /// Official project metadata for an unpinned release. Tested entries use `file`.
    pub published: Option<Release>,
    pub file: Option<File>,
    /// Optional extras and reference downloads; never installed implicitly.
    pub links: &'static [Link],
}

/// Observations from project testing are separate from the player's automatic data check.
/// Their version is the catalogue release; they must never certify a different install.
pub struct KnownIssue {
    pub code: &'static str,
    pub title: &'static str,
    pub path: &'static str,
    pub detail: &'static str,
    pub suggestion: &'static str,
    /// An inherited observation is useful context, not evidence of a mod-authored defect.
    pub inherited: bool,
}

const ESCALATION_ISSUES: &[KnownIssue] = &[
    // (Its unit textures fit since Reborn loads only those the models paint with, in two arrays:
    // 864 textures, 1,185 layers.)
    KnownIssue {
        code: "ESC-SPRITE-CAPACITY", inherited: false, title: n("Effect capacity and loading cost"), path: "anims/*.gaf",
        detail: n("The sprite atlas reached 16,384 × 16,384 on the review GPU and dropped further frames. One startup took about 59 seconds; this was an observation, not a controlled benchmark."),
        suggestion: n("Check effect frame counts and dimensions. Reborn needs bounded animation streaming or atlas paging for this full set; test actual gameplay on the target GPU."),
    },
    KnownIssue {
        code: "ESC-GAF-BUDGET", inherited: false, title: n("Large effect archives"), path: "anims/esc_nuke_a_02.gaf; anims/esc_nuke_x_01.gaf; anims/esc_weap_x_01.gaf",
        detail: n("These archives exceed the 256 MiB aggregate GAF decode budget. Reborn rejects the oversized decode instead of exhausting memory."),
        suggestion: n("Reduce or split oversized effects. Reborn needs per-entry decoding or streaming before raising the resource allowance."),
    },
    KnownIssue {
        code: "ESC-MISSING-REFERENCES", inherited: false, title: n("Missing weapons and wreck assets"), path: "GATE_RANGE2; ANTI_NUKE_ADV; AF_VENTS; armmanta_dead; armast_dead; corcapsub_dead; corast_dead; cortrog_dead",
        detail: n("The review found 27 missing data references, including explosion weapons and six feature graphics or models. A missing reference does not always mean the affected behavior is used in play."),
        suggestion: n("Use the local automated report to locate each source reference. Supply the missing assets, correct the reference, or explain why the unused definition is intentional."),
    },
    KnownIssue {
        code: "ESC-BASE-PLACEHOLDERS", inherited: true, title: n("Inherited base-game placeholders"), path: "anims/terrain.gaf; anims/vismasks.gaf; palettes/guipal.pcx",
        detail: n("These rejected placeholder files come from the base game in the reviewed installation, not Escalation. They do not establish a mod defect."),
        suggestion: n("Use the local report's layer field to verify origin. Only replace these assets if your mod intentionally overrides them."),
    },
];

const FRONTIER_ISSUES: &[KnownIssue] = &[
    KnownIssue {
        code: "TAFF-AI-SPACE-MAPS", inherited: false, title: n("Computer players can stall"), path: "Space maps",
        detail: n("AI Boost is included and computer players work on Asteroid Field, but can stall on some other space maps."),
        suggestion: n("Test each space map with computer players before publishing it. Asteroid Field is the reviewed starting point."),
    },
    KnownIssue {
        code: "TAFF-MAP-PACKS", inherited: false, title: n("Damaged optional map archives"), path: "TAFF-Map_pack_1.zip; TAFF-Map_pack_2.zip",
        detail: n("The optional map packs contain three damaged map archives. These are separate downloads and may not be present in your installation."),
        suggestion: n("Import working maps individually. Rebuild or replace damaged archives before distributing a map pack."),
    },
];

const SECRET_FORCES_ISSUES: &[KnownIssue] = &[
    KnownIssue {
        code: "TSF-PREVIEW-CONTENT", inherited: false, title: n("Unfinished preview armies"), path: "gamedata/sidedata.tdf; units/*",
        detail: n("Its new armies use ARM's name, and some units are unfinished in this early preview."),
        suggestion: n("Give each army its intended side identity and complete its unit data and assets. Check every faction in a skirmish."),
    },
];

impl Entry {
    pub fn review_issues(&self) -> &'static [KnownIssue] {
        match self.id {
            "escalation" => ESCALATION_ISSUES,
            "finalfrontier" => FRONTIER_ISSUES,
            "secretforces" => SECRET_FORCES_ISSUES,
            _ => &[],
        }
    }

    /// Stable English evidence can be pasted directly into an issue tracker.
    pub fn review_text(&self) -> String {
        let version = self.release().version;
        let mut out = format!("PROJECT REVIEW — {}\nReviewed release: {}\nThis curated review is separate from automated checks of the installed files.\n", self.name, if version.is_empty() { "Unknown" } else { version });
        if !self.note.is_empty() { out.push_str(self.note); out.push('\n'); }
        for issue in self.review_issues() {
            out.push_str(&format!("\n[{}] {}{}\nPath / reference: {}\n{}\nSuggested action: {}\n", issue.code, issue.title, if issue.inherited { " (inherited base-game context)" } else { "" }, issue.path, issue.detail, issue.suggestion));
        }
        if self.fit == Fit::Unverified || self.fit == Fit::NoFile {
            out.push_str("Gameplay compatibility has not been verified for this release.\n");
        }
        out
    }
}

impl Entry {
    pub fn release(&self) -> Release {
        self.file.map(|file| Release { version: file.version, date: file.date, bytes: Some(file.bytes) })
            .or(self.published).unwrap_or_default()
    }

    /// Tested releases and creator-hosted releases can both be installed from a local file.
    pub fn can_import(&self) -> bool {
        self.file.is_some() || self.fit == Fit::Unverified
    }

    /// Original launcher artwork, available immediately even without an image cache.
    pub fn bundled_image(&self) -> Option<&'static [u8]> {
        match self.id {
            "finalfrontier" => Some(include_bytes!("../assets/final-frontier-banner.png")),
            _ => None,
        }
    }

    pub fn moddb(&self) -> Option<&'static str> {
        match self.source { Provider::ModDb(slug) => Some(slug), Provider::Website { .. } => None }
    }

    pub fn source_name(&self) -> &'static str {
        match self.source { Provider::ModDb(_) => "ModDB", Provider::Website { name, .. } => name }
    }

    /// The creator's preferred project page, or the release provider's page.
    pub fn page(&self) -> String {
        self.project.map(str::to_string).unwrap_or_else(|| self.source_page())
    }

    /// The release provider's page, retained for download attribution and resolution.
    pub fn source_page(&self) -> String {
        match self.source {
            Provider::ModDb(slug) => crate::moddb::mod_page(slug),
            Provider::Website { url, .. } => url.into(),
        }
    }
}

pub const MODS: &[Entry] = &[
    Entry {
        id: "escalation",
        name: "TA: Escalation",
        source: Provider::Website { url: "https://taesc.tauniverse.com/?p=downloads", name: "TA: Escalation" },
        project: None,
        team: "TA: Escalation Mod Group",
        pitch: n("Expanded ARM and CORE armies with new units and weapons."),
        tags: &[n("549 UNITS"), n("177 MAPS")],
        fit: Fit::Plays,
        note: n("Gold 10.2.0 installs and plays: computer players build and fight, every unit is painted and every explosion drawn. A few wrecks have no model in the mod's own files."),
        image: "https://media.moddb.com/cache/images/downloads/1/204/203503/thumb_620x2000/2020-09-25_03.png",
        published: None,
        file: Some(File {
            download: Download::ManualOnly,
            name: "TAESC_GOLD_10_2_0_FULL.rar", version: "Gold 10.2.0", bytes: 350_859_995,
            md5: "23863bbd6c0a15bc1d6edc982358449b",
            sha256: "a9873e551d7fa72ad2f74ca37d8bbc7c873043d178979c842bbf8ed667eea2c3",
            date: "2026-08-08",
        }),
        links: &[],
    },
    Entry {
        id: "totalmayhem",
        name: "Total Mayhem",
        source: Provider::ModDb("total-mayhem"),
        project: Some("https://mayhem.tauniverse.com/totalm.htm"),
        team: "Mayhem Inc",
        pitch: n("Total Annihilation as Cavedog might have carried it on: every unit earns its place, hundreds more join, and a third tech level tops it off."),
        tags: &[n("500 UNITS"), n("105 MAPS"), n("TECH 3")],
        fit: Fit::Plays,
        note: "",
        image: "https://media.moddb.com/cache/images/downloads/1/274/273174/thumb_620x2000/mayhem1130.png",
        published: None,
        file: Some(File { download: Download::ModDb { page: "totalm1130", start: 273174 }, name: "TotalM1130.zip", version: "11.3.0", bytes: 26_035_480, md5: "9f96d80651934c048d134f007cd324cd", sha256: "", date: "2024-06-20" }),
        links: &[],
    },
    Entry {
        id: "taenhanced",
        name: "TA: Enhanced",
        source: Provider::ModDb("taenhanced"),
        project: None,
        team: "Firmament",
        pitch: n("Faster games that keep the strategy: new units, each carefully balanced, and hard-hitting armies from the start."),
        tags: &[n("FASTER GAMES"), n("297 UNITS")],
        fit: Fit::Plays,
        note: n("Still in alpha, and a big download."),
        image: "https://media.moddb.com/cache/images/downloads/1/198/197161/thumb_620x2000/0_8.png",
        published: None,
        file: Some(File { download: Download::ModDb { page: "ta-enhanced-alpha-0-8v", start: 197161 }, name: "TA_Enhanced_Alpha_0_8v.rar", version: "Alpha 0.8v", bytes: 252_300_220, md5: "4e1f0fa644430a020aad560cdaae2667", sha256: "", date: "2020-07-06" }),
        links: &[],
    },
    Entry {
        id: "tafrenzy",
        name: "TA Frenzy",
        source: Provider::ModDb("ta-frenzy"),
        project: None,
        team: "Firmament",
        pitch: n("Big and brutal: 512 units, fierce fighting, and new sounds and effects throughout."),
        tags: &[n("512 UNITS"), n("91 MAPS")],
        fit: Fit::Plays,
        note: n("The optional map weapons in its download are left out."),
        image: "https://media.moddb.com/cache/images/downloads/1/39/38978/thumb_620x2000/s4.png",
        published: None,
        file: Some(File { download: Download::ModDb { page: "ta-frenzy-1-41", start: 38978 }, name: "TA_Frenzy_1_41.rar", version: "1.41", bytes: 66_764_582, md5: "3859e48f6941dc18fd31a17f2ea77a28", sha256: "", date: "2011-12-28" }),
        links: &[],
    },
    Entry {
        id: "talon",
        name: "Talon",
        source: Provider::ModDb("talon-race"),
        project: None,
        team: "TheRegisteredOne",
        pitch: n("A third army, the Talon, from ten years of the community's ideas: teleporters, air transports that carry whole squads, and more."),
        tags: &[n("NEW RACE"), n("500 UNITS")],
        fit: Fit::Plays,
        note: n("Choose TAL as your side in a skirmish."),
        image: "https://media.moddb.com/cache/images/downloads/1/22/21880/thumb_620x2000/small.png",
        published: None,
        file: Some(File { download: Download::ModDb { page: "talon-v103", start: 21880 }, name: "Talon_v1.03.zip", version: "1.03", bytes: 47_774_482, md5: "8a7340fb7e1d7b87cda074ef32ec3470", sha256: "", date: "2010-02-23" }),
        links: &[],
    },
    Entry {
        id: "devolution",
        name: "Devolution",
        source: Provider::ModDb("total-annihilation-devolution"),
        project: None,
        team: "Boogalizer",
        pitch: n("Cavedog's style, kept simple: every unit has a clear job and looks as if it always belonged."),
        tags: &[n("CLASSIC STYLE"), n("NEW COMMANDERS")],
        fit: Fit::Plays,
        note: "",
        image: "https://media.moddb.com/cache/images/downloads/1/25/24040/thumb_620x2000/frontend.jpg",
        published: None,
        file: Some(File { download: Download::ModDb { page: "devolution-51-beta-1", start: 24040 }, name: "Devolution5_1_Beta1.zip", version: "5.1 Beta 1", bytes: 15_336_615, md5: "8eda01679844073d6b4d18f768304281", sha256: "", date: "2010-06-06" }),
        links: &[],
    },
    Entry {
        id: "battletech",
        name: "Total Battletech",
        source: Provider::ModDb("total-battletech"),
        project: None,
        team: "bobthedino",
        pitch: n("Classic BattleTech at Total Annihilation's scale: the Inner Sphere against the Clans, with walking mechs."),
        tags: &[n("BATTLETECH"), n("NEW SIDES")],
        fit: Fit::Plays,
        note: n("Computer players are weaker as the Inner Sphere."),
        image: "https://media.moddb.com/cache/images/downloads/1/15/14976/thumb_620x2000/v140splash8x6.jpg",
        published: None,
        file: Some(File { download: Download::ModDb { page: "total-battletech-1401", start: 14976 }, name: "TBT140.1.zip", version: "1.40", bytes: 13_356_105, md5: "522c34cf9b337b94793a5c7cbc9b7856", sha256: "", date: "2009-03-11" }),
        links: &[],
    },
    Entry {
        id: "twilight",
        name: "TA: Twilight",
        source: Provider::ModDb("total-annihilation-twilight"),
        project: None,
        team: "Twilight",
        pitch: n("The original's feel with well-balanced units and structures, tuned by veterans of the game."),
        tags: &[n("CLASSIC FEEL"), n("500 UNITS")],
        fit: Fit::Plays,
        note: n("Computer players struggle as ARM."),
        image: "https://media.moddb.com/cache/images/downloads/1/18/17017/thumb_620x2000/TAT_Main-Screen.jpg",
        published: None,
        file: Some(File { download: Download::ModDb { page: "total-annihilation-twilight-v17-full", start: 17017 }, name: "TAT-v1.7-Full.zip", version: "1.7", bytes: 78_293_560, md5: "c5700bc6d9581c7ad3afb5cf0f6fcd52", sha256: "", date: "2009-07-07" }),
        links: &[],
    },
    Entry {
        id: "secretforces",
        name: "The Secret Forces",
        source: Provider::ModDb("the-secret-forces"),
        project: None,
        team: "Shelby123456789",
        pitch: n("An add-on race in the making: new units and new weapons for a third and a fourth army."),
        tags: &[n("NEW RACES"), n("PREVIEW")],
        fit: Fit::Partly,
        note: n("An early preview: its new armies go by ARM's name, and some of their units are unfinished."),
        image: "https://media.moddb.com/cache/images/downloads/1/117/116758/thumb_620x2000/SHOT0014.png",
        published: None,
        file: Some(File { download: Download::ModDb { page: "the-secret-forces-v0-7-6a", start: 116758 }, name: "The_Secret_Forces_v0.7.6a.zip", version: "0.7.6a", bytes: 899_690, md5: "cc19be3e53372d6e83a76bd013dacaad", sha256: "", date: "2017-01-04" }),
        links: &[],
    },
    Entry {
        id: "finalfrontier",
        name: "The Final Frontier",
        source: Provider::Website { url: "https://taff.tauniverse.com/index.html", name: "TA Universe" },
        project: None,
        team: "Savant, Longbow and the TA:FF team",
        pitch: n("Take ARM and CORE into space with command cruisers and fleets of warships."),
        tags: &[n("SPACE WARFARE"), "TAFF", n("172 UNITS")],
        fit: Fit::Partly,
        note: n("AI Boost is already included. Computer players work on Asteroid Field, but can stall on some space maps. The optional map packs contain three damaged map archives; import working maps individually."),
        image: "",
        published: None,
        file: Some(File {
            download: Download::ManualOnly,
            name: "taffV74.zip", version: "7.4", bytes: 8_581_261,
            md5: "1ad4e4e9b02c2bf5b2240612427856c1",
            sha256: "caca1844d4bb1ce591c25030327a05d106e3f5da4169dc1143787c0149a97fc1",
            date: "",
        }),
        links: &[],
    },
    Entry {
        id: "czta",
        name: "CZTA",
        source: Provider::ModDb("czta"),
        project: None,
        team: "Guessmyname",
        pitch: n("Combat Zone TA: three new sides in a semi-realistic war."),
        tags: &[n("NEW SIDES")],
        fit: Fit::NoFile,
        note: n("Nothing to download: it was made for the Spring engine."),
        image: "",
        published: None,
        file: None,
        links: &[],
    },
    Entry {
        id: "gundam",
        name: "Gundam Annihilation",
        source: Provider::ModDb("gundam-annihilation"),
        project: None,
        team: "Smoth",
        pitch: n("The One Year War of Mobile Suit Gundam: the Federation against Zeon."),
        tags: &[n("ANIME")],
        fit: Fit::NoFile,
        note: n("Its files are no longer on ModDB."),
        image: "",
        published: None,
        file: None,
        links: &[],
    },
];

pub fn get(id: &str) -> Option<&'static Entry> {
    MODS.iter().find(|m| m.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_complete_and_distinct() {
        for (i, m) in MODS.iter().enumerate() {
            assert!(MODS[..i].iter().all(|o| o.id != m.id && o.page() != m.page()), "{} twice", m.id);
            assert!(m.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()), "{}: id", m.id);
            assert_eq!(matches!(m.fit, Fit::NoFile | Fit::Unverified), m.file.is_none(), "{}", m.id);
            assert_eq!(m.can_import(), m.fit != Fit::NoFile, "{}", m.id);
            if let Some(project) = m.project { assert!(project.starts_with("https://"), "{}: project page", m.id); }
            assert!(!(m.file.is_some() && m.published.is_some()), "{}: one authoritative release", m.id);
            let release = m.release();
            if !release.date.is_empty() {
                assert!(release.date.len() == 10 && release.date.bytes().enumerate().all(|(i, b)| if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() }), "{}: ISO release date", m.id);
            }
            for link in m.links { assert!(!link.label.is_empty() && link.url.starts_with("https://")); }
            if let Some(f) = m.file {
                assert!(f.bytes > 0 && !f.version.is_empty(), "{}", m.id);
                match f.download {
                    Download::ManualOnly => {
                        assert!(!f.can_download());
                        assert!(f.sha256.len() == 64 && f.sha256.bytes().all(|c| c.is_ascii_hexdigit()));
                        assert!(m.page().starts_with("https://"));
                        assert!(m.links.is_empty(), "manual-only entries direct players to their creator's website");
                    }
                    Download::ModDb { page, start } => {
                        assert!(m.moddb().is_some() && start > 0 && !page.is_empty());
                        assert!(f.md5.len() == 32 && f.md5.bytes().all(|c| c.is_ascii_hexdigit()));
                        assert!(m.image.starts_with("https://media.moddb.com/cache/images/"), "{}: a thumbnail", m.id);
                    }
                    Download::Direct(urls) => {
                        assert!(!urls.is_empty());
                        assert!(f.sha256.len() == 64 && f.sha256.bytes().all(|c| c.is_ascii_hexdigit()));
                        for url in urls { assert!(url.starts_with("https://")); }
                    }
                }
            }
        }
    }

    #[test]
    fn preferred_project_page_keeps_the_verified_download_provider() {
        let mayhem = get("totalmayhem").unwrap();
        assert_eq!(mayhem.page(), "https://mayhem.tauniverse.com/totalm.htm");
        assert_eq!(mayhem.source_page(), "https://www.moddb.com/mods/total-mayhem");
        assert_eq!(mayhem.source_name(), "ModDB");
        assert_eq!(mayhem.moddb(), Some("total-mayhem"));
        assert!(matches!(mayhem.file.unwrap().download, Download::ModDb { page: "totalm1130", start: 273174 }));
        let enhanced = get("taenhanced").unwrap();
        assert_eq!(enhanced.page(), enhanced.source_page(), "mods without a separate project keep their provider link");
    }

    #[test]
    fn escalation_pins_the_tested_current_release_without_stale_moddb_updates() {
        let escalation = get("escalation").unwrap();
        assert_eq!(escalation.page(), "https://taesc.tauniverse.com/?p=downloads");
        assert!(escalation.moddb().is_none());
        let file = escalation.file.unwrap();
        assert!(matches!(file.download, Download::ManualOnly));
        assert_eq!((file.version, file.date, file.bytes), ("Gold 10.2.0", "2026-08-08", 350_859_995));
        assert_eq!(escalation.fit, Fit::Plays, "painted and drawn whole: the catalogue's spotlight");
        assert_eq!(MODS[0].id, "escalation");
        assert!(escalation.can_import());
        assert!(!get("czta").unwrap().can_import(), "unavailable catalogue entries remain link-only");
    }
}
