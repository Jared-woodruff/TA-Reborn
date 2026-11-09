//! Lazily decoded, shared sound cache.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use ta_vfs::Vfs;

use crate::decode::{self, SoundData};
use crate::{AudioError, Result};

/// Where sound files come from. Implemented for the game's [`Vfs`]; tests and tools can
/// provide in-memory sources.
pub trait SoundSource: Send + Sync {
    /// Reads a whole file by (case-insensitive) VFS path, `None` if it does not exist.
    fn read(&self, path: &str) -> Option<Vec<u8>>;
    /// Files directly inside `dir` (normalised paths, sorted).
    fn list(&self, dir: &str) -> Vec<String>;
}

impl SoundSource for Vfs {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.read_opt(path)
    }

    fn list(&self, dir: &str) -> Vec<String> {
        Vfs::list(self, dir, None, false)
    }
}

/// Sound names TA's own data misspells, and the files they mean: the factories' sound
/// category asks for `untdone`, the file is `unitdone.wav`.
const MISSPELT: [(&str, &str); 1] = [("untdone", "unitdone")];

/// Maps a sound name as written in TA data to its VFS path: `"xplolrg1"` →
/// `"sounds/xplolrg1.wav"`, `"explode.wav"` → `"sounds/explode.wav"`, and names with a
/// directory (`"camps/briefs/arm01"`) are kept, gaining `.wav` when they have no extension.
/// Names TA's data misspells are read as meant (see `MISSPELT`).
pub fn sound_path(name: &str) -> String {
    let name = name.trim();
    let name = MISSPELT.iter().find(|(wrong, _)| name.eq_ignore_ascii_case(wrong)).map_or(name, |&(_, right)| right);
    let mut path = ta_vfs::normalize(name);
    if !path.contains('/') {
        path.insert_str(0, "sounds/");
    }
    let file = path.rsplit('/').next().unwrap_or_default();
    if !file.contains('.') {
        path.push_str(".wav");
    }
    path
}

/// Thread-safe cache of decoded sounds keyed by VFS path. Sounds are decoded on first use and
/// kept until [`SoundBank::clear`]; a missing or undecodable file is logged once and
/// remembered, so a broken reference in a unit definition costs nothing per play.
pub struct SoundBank {
    source: Arc<dyn SoundSource>,
    cache: RwLock<HashMap<String, Option<Arc<SoundData>>>>,
}

impl std::fmt::Debug for SoundBank {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SoundBank").field("cached", &self.cached_count()).finish_non_exhaustive()
    }
}

impl SoundBank {
    pub fn new(source: Arc<dyn SoundSource>) -> SoundBank {
        SoundBank { source, cache: RwLock::new(HashMap::new()) }
    }

    /// A bank reading from the game's virtual file system.
    pub fn from_vfs(vfs: Arc<Vfs>) -> SoundBank {
        SoundBank::new(vfs)
    }

    pub fn source(&self) -> &Arc<dyn SoundSource> {
        &self.source
    }

    /// Returns a decoded sound by name (see [`sound_path`]), loading it on first use.
    pub fn get(&self, name: &str) -> Option<Arc<SoundData>> {
        let path = sound_path(name);
        if let Some(entry) = self.read_cache().get(&path) {
            return entry.clone();
        }
        let loaded = match self.load_path(&path) {
            Ok(sound) => Some(Arc::new(sound)),
            Err(e) => {
                log::warn!("sound '{name}' unavailable: {e}");
                None
            }
        };
        // Another thread may have loaded it meanwhile; keep whichever landed first.
        self.write_cache().entry(path).or_insert(loaded).clone()
    }

    /// Loads and decodes a sound without caching, reporting why it failed.
    pub fn load(&self, name: &str) -> Result<SoundData> {
        self.load_path(&sound_path(name))
    }

    fn load_path(&self, path: &str) -> Result<SoundData> {
        let bytes = self.source.read(path).ok_or_else(|| AudioError::NotFound(path.to_string()))?;
        let mut decoded =
            decode::decode(&bytes).map_err(|source| AudioError::Decode { path: path.to_string(), source })?;
        if decoded.format != decode::SourceFormat::Wav(decode::WavCodec::Pcm { bits: 8 }) {
            log::debug!("{path}: {}", decoded.format);
        }
        decoded.sound.declick();
        Ok(decoded.sound)
    }

    /// Decodes the given sounds now (e.g. during a loading screen). Returns how many are
    /// available.
    pub fn preload<'a>(&self, names: impl IntoIterator<Item = &'a str>) -> usize {
        names.into_iter().filter(|n| self.get(n).is_some()).count()
    }

    /// Adds or replaces a sound under `name` (procedural sounds, tests).
    pub fn insert(&self, name: &str, sound: SoundData) -> Arc<SoundData> {
        let sound = Arc::new(sound);
        self.write_cache().insert(sound_path(name), Some(sound.clone()));
        sound
    }

    pub fn is_cached(&self, name: &str) -> bool {
        self.read_cache().contains_key(&sound_path(name))
    }

    /// Number of cached entries (including remembered failures).
    pub fn cached_count(&self) -> usize {
        self.read_cache().len()
    }

    /// Bytes of decoded sample data held by the cache.
    pub fn memory_bytes(&self) -> usize {
        self.read_cache().values().flatten().map(|s| s.memory_bytes()).sum()
    }

    /// Drops every cached sound. Voices still playing keep their data alive until they end.
    pub fn clear(&self) {
        self.write_cache().clear();
    }

    fn read_cache(&self) -> std::sync::RwLockReadGuard<'_, HashMap<String, Option<Arc<SoundData>>>> {
        self.cache.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write_cache(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<String, Option<Arc<SoundData>>>> {
        self.cache.write().unwrap_or_else(|e| e.into_inner())
    }
}

/// An in-memory [`SoundSource`] for tests and tools.
#[derive(Default)]
pub struct MemorySource {
    files: std::collections::BTreeMap<String, Vec<u8>>,
}

impl MemorySource {
    pub fn new() -> MemorySource {
        MemorySource::default()
    }

    pub fn insert(&mut self, path: &str, bytes: Vec<u8>) {
        self.files.insert(ta_vfs::normalize(path), bytes);
    }
}

impl SoundSource for MemorySource {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.files.get(&ta_vfs::normalize(path)).cloned()
    }

    fn list(&self, dir: &str) -> Vec<String> {
        let mut prefix = ta_vfs::normalize(dir);
        if !prefix.is_empty() && !prefix.ends_with('/') {
            prefix.push('/');
        }
        self.files.keys().filter(|k| k.starts_with(&prefix) && !k[prefix.len()..].contains('/')).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm8_wav(samples: &[u8]) -> Vec<u8> {
        let mut b = b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x11\x2b\0\0\x11\x2b\0\0\x01\0\x08\0data".to_vec();
        b.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        b.extend_from_slice(samples);
        b
    }

    #[test]
    fn resolves_sound_names() {
        assert_eq!(sound_path("xplolrg1"), "sounds/xplolrg1.wav");
        assert_eq!(sound_path("Explode.WAV"), "sounds/explode.wav");
        assert_eq!(sound_path("camps\\briefs\\ARM01"), "camps/briefs/arm01.wav");
        assert_eq!(sound_path(" beep6 "), "sounds/beep6.wav");
        // TA's factories ask for "untdone"; the file is unitdone.wav.
        assert_eq!(sound_path("UNTDONE"), "sounds/unitdone.wav");
    }

    #[test]
    fn caches_hits_and_misses() {
        let mut src = MemorySource::new();
        src.insert("sounds/beep.wav", pcm8_wav(&[128, 200, 60, 128]));
        src.insert("sounds/broken.wav", b"RIFF\0\0\0\0WAVEjunk".to_vec());
        let bank = SoundBank::new(Arc::new(src));
        let a = bank.get("BEEP").unwrap();
        assert_eq!(a.frames(), 4);
        assert!(Arc::ptr_eq(&a, &bank.get("beep.wav").unwrap()));
        assert!(bank.get("missing").is_none());
        assert!(bank.get("broken").is_none());
        assert_eq!(bank.cached_count(), 3);
        assert!(matches!(bank.load("missing"), Err(AudioError::NotFound(_))));
        assert!(matches!(bank.load("broken"), Err(AudioError::Decode { .. })));
        assert_eq!(bank.memory_bytes(), 16);
        bank.clear();
        assert_eq!(bank.cached_count(), 0);
    }

    #[test]
    fn memory_source_lists_one_level() {
        let mut src = MemorySource::new();
        src.insert("music/1.mp3", vec![]);
        src.insert("music/sub/2.mp3", vec![]);
        src.insert("musicx/3.mp3", vec![]);
        assert_eq!(src.list("MUSIC"), vec!["music/1.mp3".to_string()]);
    }
}
