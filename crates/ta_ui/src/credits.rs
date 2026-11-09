//! Who made TA Reborn, for its credits: the game's roll (`apps/ta_game/src/credits.rs`) and
//! the launcher's About page.

use crate::i18n::{n, tf};

/// The project lead.
pub const PROJECT_LEAD: &str = "Jared Woodruff";
/// The project lead's page.
pub const PROJECT_LEAD_URL: &str = "https://github.com/Jared-woodruff";
/// TA Reborn's website as the game's credits give it, under the team: an address that stays
/// (a repository's may move).
pub const WEBSITE: &str = "ta-reborn.com";

/// The band whose song the credits play, and the song (the game's credits film plays it):
/// names, never translated.
pub const BAND: &str = "Hadal Static";
pub const SONG: &str = "The War Reborn";

/// A credit of TA Reborn's team: its role and who has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Credit {
    /// The role alone, as a caption over the name (the About page's plaques; translated where
    /// it is shown). Credits of one role follow one another.
    pub role: &'static str,
    /// The credit as a line, `{name}` where the name goes (translated where it is shown: its
    /// role is).
    pub line: &'static str,
    /// Who: names and model designations stay exactly as credited, in any language.
    pub name: &'static str,
}

impl Credit {
    /// The credit in the interface's language: "Project Lead: Jared Woodruff".
    pub fn text(&self) -> String {
        tf(self.line, &[("name", &self.name)])
    }
}

/// TA Reborn's team, in the order the credits name them (both credits screens).
pub const TEAM: [Credit; 5] = [
    Credit { role: n("PROJECT LEAD"), line: n("Project Lead: {name}"), name: PROJECT_LEAD },
    Credit { role: n("DEV"), line: n("DEV: {name}"), name: "Claude Opus 5.5 by Anthropic" },
    Credit { role: n("QA"), line: n("QA: {name}"), name: "GPT6 Astra by OpenAI" },
    Credit { role: n("VO"), line: n("VO: {name}"), name: "Kokoro-82M by Hexgrad" },
    Credit { role: n("VO"), line: n("VO: {name}"), name: "Chatterbox by Resemble AI" },
];

/// The team as the credits list it, a line each.
pub fn contributors() -> Vec<String> {
    TEAM.iter().map(Credit::text).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_team_reads_as_credited() {
        assert_eq!(
            contributors(),
            [
                "Project Lead: Jared Woodruff",
                "DEV: Claude Opus 5.5 by Anthropic",
                "QA: GPT6 Astra by OpenAI",
                "VO: Kokoro-82M by Hexgrad",
                "VO: Chatterbox by Resemble AI",
            ]
        );
        assert_eq!(TEAM[0].name, PROJECT_LEAD);
        // (A role's credits together: the About page gives each role one plaque.)
        let mut roles: Vec<&str> = TEAM.iter().map(|c| c.role).collect();
        roles.dedup();
        let mut each = roles.clone();
        each.sort();
        each.dedup();
        assert_eq!(roles.len(), each.len(), "a role's credits follow one another: {roles:?}");
    }
}
