//! Supreme Commander: Forged Alliance's maps in TA Reborn. The maps are read from the player's
//! own copy of the game (GOG's or Steam's, or a folder they pick), never shipped: their land,
//! painted ground, forests, rocks, resources and start positions are made into TA maps in the
//! player's mods folder, to play in skirmish.
//!
//! - [`install`]: finding the game, and the player's own maps;
//! - [`scd`], [`dds`], [`lua`], [`scmap`]: reading its archives, textures, data and maps;
//! - [`map`]: a map as the converter needs it (the scenario, the save file's markers, the map);
//! - [`convert`]: the map made into a TA map.

pub mod convert;
pub mod dds;
pub mod files;
pub mod install;
pub mod lua;
pub mod map;
pub mod scd;
pub mod scmap;

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A file that is not what it should be.
    #[error("{0}")]
    Format(String),
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    /// A file or folder the map needs that is not there.
    #[error("{0}")]
    Missing(String),
}

impl Error {
    pub(crate) fn io(path: &Path, source: std::io::Error) -> Error {
        Error::Io { path: path.to_path_buf(), source }
    }
}

impl From<ta_formats::FormatError> for Error {
    fn from(e: ta_formats::FormatError) -> Error {
        Error::Format(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
pub(crate) mod tests {
    use std::path::PathBuf;

    /// A folder of the test's own under the system's temporary folder, removed afterwards.
    pub(crate) struct Temp(pub PathBuf);

    impl Temp {
        pub(crate) fn new(name: &str) -> Temp {
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let dir = std::env::temp_dir().join(format!("ta-supcom-{name}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Temp(dir)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
