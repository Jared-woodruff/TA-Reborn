//! Per-profile visibility of installed maps. This only controls new battle choices;
//! campaign missions, saved games and the editor retain access to the original files.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Maps {
    /// Source layer and normalized map stem. A mod's replacement and the original map
    /// have separate preferences; changing the mod load order never hides both by accident.
    pub disabled: Vec<String>,
    /// Maps marked as favourites, by normalized stem: a map is a favourite whichever source
    /// supplies it.
    pub favourites: Vec<String>,
}

impl Maps {
    pub fn key(source: &str, stem: &str) -> String {
        format!("{}|{}", source.to_ascii_lowercase(), stem.replace('\\', "/").to_ascii_lowercase())
    }

    pub fn is_enabled(&self, source: &str, stem: &str) -> bool {
        let key = Self::key(source, stem);
        !self.disabled.iter().any(|item| item == &key)
    }

    pub fn set_enabled(&mut self, source: &str, stem: &str, enabled: bool) {
        let key = Self::key(source, stem);
        self.disabled.retain(|item| item != &key);
        if !enabled { self.disabled.push(key); }
    }

    fn stem(stem: &str) -> String { stem.replace('\\', "/").to_ascii_lowercase() }

    pub fn is_favourite(&self, stem: &str) -> bool {
        let stem = Self::stem(stem);
        self.favourites.iter().any(|item| item == &stem)
    }

    pub fn set_favourite(&mut self, stem: &str, favourite: bool) {
        let stem = Self::stem(stem);
        self.favourites.retain(|item| item != &stem);
        if favourite { self.favourites.push(stem); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn map_preferences_are_source_specific_and_idempotent() {
        let mut maps = Maps::default();
        maps.set_enabled("space-pack", "Asteroid Field", false);
        maps.set_enabled("SPACE-PACK", "ASTEROID FIELD", false);
        assert_eq!(maps.disabled.len(), 1);
        assert!(!maps.is_enabled("space-pack", "asteroid field"));
        assert!(maps.is_enabled("base", "asteroid field"));
        maps.set_enabled("space-pack", "asteroid field", true);
        assert!(maps.disabled.is_empty());
    }
    #[test]
    fn favourites_follow_the_map_whatever_its_source_and_roundtrip() {
        let mut maps = Maps::default();
        maps.set_favourite("Coast To Coast", true);
        maps.set_favourite("COAST TO COAST", true);
        assert_eq!(maps.favourites, ["coast to coast"]);
        assert!(maps.is_favourite("coast to coast"));
        let mut config: crate::Config = toml::from_str("").unwrap();
        config.maps = maps.clone();
        let loaded: crate::Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
        assert_eq!(loaded.maps, maps);
        maps.set_favourite("coast to coast", false);
        assert!(maps.favourites.is_empty());
    }

    #[test]
    fn old_configs_enable_all_maps_and_preferences_roundtrip() {
        let mut config: crate::Config = toml::from_str("").unwrap();
        assert_eq!(config.maps, Maps::default());
        config.maps.set_enabled("base", "Sherwood", false);
        let text = toml::to_string(&config).unwrap();
        let loaded: crate::Config = toml::from_str(&text).unwrap();
        assert_eq!(loaded.maps, config.maps);
    }
}
