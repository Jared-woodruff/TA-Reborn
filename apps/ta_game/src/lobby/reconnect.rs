//! Persist reconnect secrets across application restarts. Never log serialized credentials.
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::path::Path;

use serde::{Deserialize, Serialize};
use ta_net::{Client, ReconnectToken};

const MAX_ENTRIES: usize = 64;
const MAX_FILE_BYTES: u64 = 64 << 10;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Key {
    endpoint: SocketAddr,
    data_hash: u64,
    name: String,
}

impl Key {
    fn new(endpoint: SocketAddr, data_hash: u64, name: &str) -> Self {
        Self { endpoint, data_hash, name: name.trim().chars().filter(|c| !c.is_control()).take(24).collect::<String>().to_ascii_lowercase() }
    }
}

#[derive(Serialize, Deserialize)]
struct Entry { key: Key, token: ReconnectToken }

fn load(path: &Path) -> Vec<Entry> {
    let Ok(file) = std::fs::File::open(path) else { return Vec::new() };
    let mut bytes = Vec::new();
    if file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes).is_err() || bytes.len() as u64 > MAX_FILE_BYTES { return Vec::new(); }
    let Ok(entries) = serde_json::from_slice::<Vec<Entry>>(&bytes) else { return Vec::new() };
    if entries.len() > MAX_ENTRIES { Vec::new() } else { entries }
}

pub(super) fn connect(endpoint: SocketAddr, name: &str, data_hash: u64, password: &str) -> io::Result<(Client, Key)> {
    let key = Key::new(endpoint, data_hash, name);
    let token = load(&ta_config::player_data_dir().join("reconnect.json")).iter().find(|e| e.key == key).map(|e| e.token);
    Client::connect_with_token(endpoint, name, data_hash, token, password).map(|client| (client, key))
}

pub(super) fn remember(key: Key, token: ReconnectToken) -> io::Result<()> {
    save(&ta_config::player_data_dir().join("reconnect.json"), key, token)
}

fn save(path: &Path, key: Key, token: ReconnectToken) -> io::Result<()> {
    let mut entries = load(path);
    entries.retain(|e| e.key != key);
    entries.push(Entry { key, token });
    if entries.len() > MAX_ENTRIES { entries.drain(..entries.len() - MAX_ENTRIES); }
    let bytes = serde_json::to_vec(&entries).map_err(io::Error::other)?;
    if bytes.len() as u64 > MAX_FILE_BYTES { return Err(io::Error::new(io::ErrorKind::InvalidData, "reconnect store exceeds its size limit")); }
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
    // The general config writer creates ordinary readable files. Secrets instead start
    // private, before any bytes are written; create_new also refuses symlink replacement.
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(io::Error::other)?.as_nanos();
    let temp = path.with_extension(format!("tmp-{}-{nonce}", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    let result = (|| {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)
    })();
    if result.is_err() { let _ = std::fs::remove_file(&temp); }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_survive_reload_and_are_scoped_and_bounded() {
        let dir = std::env::temp_dir().join(format!("ta-reconnect-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let path = dir.join("reconnect.json");
        let token: ReconnectToken = serde_json::from_value(serde_json::to_value([7u8; 32]).unwrap()).unwrap();
        let key = Key::new("127.0.0.1:7890".parse().unwrap(), 7, " Guest ");
        save(&path, key.clone(), token).unwrap();
        let entries = load(&path);
        assert!(entries.iter().any(|e| e.key == Key::new(key.endpoint, 7, "guest") && e.token == token));
        assert!(!entries.iter().any(|e| e.key == Key::new(key.endpoint, 8, "Guest")));
        assert!(!entries.iter().any(|e| e.key == Key::new("127.0.0.1:7891".parse().unwrap(), 7, "Guest")));
        for n in 0..MAX_ENTRIES { save(&path, Key::new(key.endpoint, n as u64, "Other"), token).unwrap(); }
        assert_eq!(load(&path).len(), MAX_ENTRIES);
        assert!(!load(&path).iter().any(|e| e.key == key));
        std::fs::write(&path, vec![b' '; MAX_FILE_BYTES as usize + 1]).unwrap();
        assert!(load(&path).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
