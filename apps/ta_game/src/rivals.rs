//! The enemy Commanders the player has fought, by name: how many battles, who won them, who won
//! the last. The computer players' personas come back and remember (a rematch's greeting); the
//! player's own record, kept in `rivals.toml` in their folder. Nothing here touches the battle.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const FILE: &str = "rivals.toml";
/// Rivals kept: the most recently met.
const KEPT: usize = 64;

/// The player's record against one enemy Commander.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rival {
    pub battles: u32,
    /// Battles the player won against them, and lost.
    pub won: u32,
    pub lost: u32,
    /// Whether the player won the last (none before the first is decided).
    pub last_won: Option<bool>,
    /// When they last met (seconds since 1970), for keeping the most recent.
    pub met_at: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rivals {
    pub rivals: BTreeMap<String, Rival>,
}

impl Rivals {
    fn path() -> PathBuf {
        ta_config::player_data_dir().join(FILE)
    }

    /// The player's rivals (none when the file is missing or unreadable).
    pub fn load() -> Rivals {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    /// The record against `name`, if they have met.
    pub fn get(&self, name: &str) -> Option<Rival> {
        self.rivals.get(name).copied()
    }

    /// A battle against `names` decided: the player won it or lost it.
    pub fn record(&mut self, names: &[String], won: bool, now: u64) {
        for name in names {
            let r = self.rivals.entry(name.clone()).or_default();
            r.battles = r.battles.saturating_add(1);
            if won {
                r.won = r.won.saturating_add(1);
            } else {
                r.lost = r.lost.saturating_add(1);
            }
            r.last_won = Some(won);
            r.met_at = now;
        }
        // (The most recently met, a bounded few.)
        while self.rivals.len() > KEPT {
            let Some(oldest) = self.rivals.iter().min_by_key(|(name, r)| (r.met_at, (*name).clone())).map(|(name, _)| name.clone()) else { break };
            self.rivals.remove(&oldest);
        }
    }

    /// Writes the record (a copy beside it first, then into place).
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string(self).map_err(std::io::Error::other)?;
        let staged = path.with_extension("toml.new");
        std::fs::write(&staged, text)?;
        std::fs::rename(&staged, &path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_keeps_score_and_the_most_recent_rivals() {
        let mut rivals = Rivals::default();
        let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        rivals.record(&names(&["Cortex Theta", "Rook"]), true, 10);
        rivals.record(&names(&["Cortex Theta"]), false, 20);
        let theta = rivals.get("Cortex Theta").unwrap();
        assert_eq!((theta.battles, theta.won, theta.lost, theta.last_won), (2, 1, 1, Some(false)));
        assert_eq!(rivals.get("Rook").unwrap().last_won, Some(true));
        for k in 0..KEPT {
            rivals.record(&[format!("Rival {k}")], true, 100 + k as u64);
        }
        assert_eq!(rivals.rivals.len(), KEPT);
        assert!(rivals.get("Rook").is_none() && rivals.get("Cortex Theta").is_none(), "the longest unmet go first");
        let text = toml::to_string(&rivals).unwrap();
        assert_eq!(toml::from_str::<Rivals>(&text).unwrap(), rivals);
    }
}
