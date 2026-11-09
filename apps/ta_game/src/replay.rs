//! Replays and saved games.
//!
//! The simulation is deterministic, so a game is fully described by its setup and the
//! player orders of each tick (AI players re-derive theirs). A replay plays those orders
//! back to a spectator; a saved game is the same record that, once fast-forwarded to its
//! last tick, continues as a normal game.

use std::path::{Path, PathBuf};
use std::io::Read;

use bincode::Options;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use serde::de::{DeserializeSeed, Deserializer, Error as _, SeqAccess, Visitor};
use ta_sim::Command;

use crate::setup::GameSetup;

/// File layout; deterministic simulation compatibility is recorded separately.
const VERSION: u32 = ta_config::RECORD_VERSION;
/// Generous enough for long matches, but never allocate directly from an untrusted file
/// length. The codec uses the same fixed-width representation as existing records.
const MAX_RECORD_BYTES: u64 = ta_config::records::MAX_RECORD_BYTES;

pub const RECORDING_STOPPED: &str = ta_ui::i18n::n("Recording memory limit reached. New saves and replays are unavailable for this match.");
const MAX_RECORDING_MEMORY: usize = 256 << 20;
// A Vec's initial allocation may hold four entries; subsequent growth is charged below.
const RECORDING_BASE_MEMORY: usize = 4 * std::mem::size_of::<(u64, Vec<Command>)>();

/// A complete command history, or an explicitly unavailable recording. Once a bundle is
/// omitted, later ticks must never make the history look complete again.
pub struct Recording {
    ticks: Vec<(u64, Vec<Command>)>,
    memory: usize,
    limit: usize,
    available: bool,
}

impl Default for Recording {
    fn default() -> Self {
        Self { ticks: Vec::new(), memory: RECORDING_BASE_MEMORY, limit: MAX_RECORDING_MEMORY, available: true }
    }
}

impl Recording {
    fn bundle_memory(commands: &[Command]) -> usize {
        // Include nested vectors and leave room for spare outer/bundle Vec capacity.
        commands.iter().fold(std::mem::size_of::<(u64, Vec<Command>)>(), |n, c| n.saturating_add(ta_net::command_memory(c))).saturating_mul(2)
    }

    pub fn available(&self) -> bool { self.available }

    /// True only on the transition to unavailable, so the match reports it once. The
    /// original commands remain untouched and still execute even when recording stops.
    pub fn append(&mut self, tick: u64, commands: &[Command]) -> bool {
        if !self.available || commands.is_empty() { return false; }
        let memory = self.memory.saturating_add(Self::bundle_memory(commands));
        if memory > self.limit {
            self.ticks = Vec::new();
            self.memory = 0;
            self.available = false;
            return true;
        }
        self.memory = memory;
        self.ticks.push((tick, commands.to_vec()));
        false
    }

    pub fn snapshot(&self) -> Option<Vec<(u64, Vec<Command>)>> {
        self.available.then(|| self.ticks.clone())
    }

    pub fn retain_through(&mut self, tick: u64) {
        self.ticks.retain(|(t, _)| *t <= tick);
        self.ticks.shrink_to_fit();
        if self.available {
            self.memory = self.ticks.iter().fold(RECORDING_BASE_MEMORY, |n, (_, c)| n.saturating_add(Self::bundle_memory(c)));
        }
    }
}

fn codec() -> impl Options {
    bincode::DefaultOptions::new().with_fixint_encoding().with_limit(MAX_RECORD_BYTES).reject_trailing_bytes()
}

/// A wire-size limit alone does not bound Vec<Command>: e.g. Resign is five encoded
/// bytes but occupies a whole enum slot in memory. Debit declared inline storage before
/// allocating, then each command's bounded nested vectors before retaining that command.
struct DecodeBudget(usize);
impl DecodeBudget {
    fn charge<E: serde::de::Error>(&mut self, bytes: usize) -> Result<(), E> {
        self.0 = self.0.checked_sub(bytes).ok_or_else(|| E::custom("decoded replay history exceeds memory limit"))?;
        Ok(())
    }
    fn sequence<'de, A: SeqAccess<'de>>(&mut self, sequence: &A, item_bytes: usize) -> Result<usize, A::Error> {
        let count = sequence.size_hint().ok_or_else(|| A::Error::custom("replay sequence requires a known length"))?;
        let bytes = count.checked_mul(item_bytes).and_then(|n| n.checked_mul(2)).ok_or_else(|| A::Error::custom("decoded replay history exceeds memory limit"))?;
        self.charge(bytes)?;
        Ok(count)
    }
}

struct RecordSeed(usize);
impl<'de> DeserializeSeed<'de> for RecordSeed {
    type Value = Record;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Record, D::Error> {
        deserializer.deserialize_tuple(9, self)
    }
}
impl<'de> Visitor<'de> for RecordSeed {
    type Value = Record;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str("a replay record") }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Record, A::Error> {
        macro_rules! field { () => { sequence.next_element()?.ok_or_else(|| A::Error::custom("truncated replay record"))? }; }
        let version: u32 = field!();
        let kind = field!();
        let data_hash = field!();
        // (Before version 42 a Conquest battle's setup had no reinforcements, before 43 the
        // rules no mutators: read as it was.)
        let setup: GameSetup = if version < 42 {
            let legacy: crate::setup::LegacySetup = field!();
            legacy.into()
        } else if version < 43 {
            let legacy: crate::setup::SetupV42 = field!();
            legacy.into()
        } else {
            field!()
        };
        setup.validate().map_err(A::Error::custom)?;
        // (From version 44 the constructors' duties, a few bytes the officer reads: bounded before they are.)
        let duties = if version >= 44 { sequence.next_element_seed(DutiesSeed)?.ok_or_else(|| A::Error::custom("missing duties"))? } else { Vec::new() };
        let mut budget = DecodeBudget(self.0);
        budget.charge(RECORDING_BASE_MEMORY)?;
        let ticks = sequence.next_element_seed(HistorySeed(&mut budget))?.ok_or_else(|| A::Error::custom("missing replay history"))?;
        let end_tick = field!();
        let saved_at = field!();
        let title = sequence.next_element_seed(TitleSeed)?.ok_or_else(|| A::Error::custom("missing replay title"))?;
        Ok(Record { version, kind, data_hash, setup, duties, ticks, end_tick, saved_at, title, profile_match: None, metadata: None, mods: None, opened_anyway: false })
    }
}

/// The duties of a saved game: a few bytes, whose count is checked before any is read.
struct DutiesSeed;
impl<'de> DeserializeSeed<'de> for DutiesSeed {
    type Value = Vec<u8>;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Vec<u8>, D::Error> {
        deserializer.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for DutiesSeed {
    type Value = Vec<u8>;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str("a few bytes of duties") }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Vec<u8>, A::Error> {
        let count = sequence.size_hint().ok_or_else(|| A::Error::custom("duties require a known length"))?;
        if count > ta_ai::MAX_SAVED_BYTES { return Err(A::Error::custom("duties exceed size limit")); }
        let mut bytes = Vec::with_capacity(count.min(4096));
        for _ in 0..count {
            bytes.push(sequence.next_element()?.ok_or_else(|| A::Error::custom("truncated duties"))?);
        }
        Ok(bytes)
    }
}

struct TitleSeed;
impl<'de> DeserializeSeed<'de> for TitleSeed {
    type Value = String;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<String, D::Error> {
        deserializer.deserialize_str(self)
    }
}
impl<'de> Visitor<'de> for TitleSeed {
    type Value = String;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str("a bounded replay title") }
    fn visit_str<E: serde::de::Error>(self, title: &str) -> Result<String, E> {
        if title.len() > 4096 { return Err(E::custom("record title exceeds size limit")); }
        Ok(title.to_owned())
    }
}

struct HistorySeed<'a>(&'a mut DecodeBudget);
impl<'de> DeserializeSeed<'de> for HistorySeed<'_> {
    type Value = Vec<(u64, Vec<Command>)>;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for HistorySeed<'_> {
    type Value = Vec<(u64, Vec<Command>)>;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str("ordered replay ticks") }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let count = self.0.sequence(&sequence, std::mem::size_of::<(u64, Vec<Command>)>())?;
        let mut ticks = Vec::with_capacity(count.min(1024));
        let mut previous = 0;
        for _ in 0..count {
            let item = sequence.next_element_seed(TickSeed { budget: self.0, previous })?.ok_or_else(|| A::Error::custom("truncated replay ticks"))?;
            previous = item.0;
            ticks.push(item);
        }
        Ok(ticks)
    }
}

struct TickSeed<'a> { budget: &'a mut DecodeBudget, previous: u64 }
impl<'de> DeserializeSeed<'de> for TickSeed<'_> {
    type Value = (u64, Vec<Command>);
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_tuple(2, self)
    }
}
impl<'de> Visitor<'de> for TickSeed<'_> {
    type Value = (u64, Vec<Command>);
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str("a tick and its commands") }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let tick = sequence.next_element()?.ok_or_else(|| A::Error::custom("missing replay tick"))?;
        if tick <= self.previous || tick >= ta_config::MAX_RECORD_TICKS { return Err(A::Error::custom("invalid replay tick sequence")); }
        let commands = sequence.next_element_seed(CommandsSeed(self.budget))?.ok_or_else(|| A::Error::custom("missing replay commands"))?;
        Ok((tick, commands))
    }
}

struct CommandsSeed<'a>(&'a mut DecodeBudget);
impl<'de> DeserializeSeed<'de> for CommandsSeed<'_> {
    type Value = Vec<Command>;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for CommandsSeed<'_> {
    type Value = Vec<Command>;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str("bounded replay commands") }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let count = self.0.sequence(&sequence, std::mem::size_of::<Command>())?;
        let mut commands = Vec::with_capacity(count.min(1024));
        for _ in 0..count {
            // Command's vectors have their own count limits, so even the one command
            // being decoded before this debit has bounded temporary memory.
            let command: Command = sequence.next_element()?.ok_or_else(|| A::Error::custom("truncated replay commands"))?;
            let nested = ta_net::command_memory(&command).saturating_sub(std::mem::size_of::<Command>());
            self.0.charge(nested.saturating_mul(2))?;
            commands.push(command);
        }
        Ok(commands)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordKind {
    Replay,
    Save,
}

/// A recorded game. Serialized in its version's own layout (see the impls below).
#[derive(Clone, Debug)]
pub struct Record {
    /// Optional presentation-only identity, carried in the validated trailer, not in the
    /// deterministic simulation body. Legacy saves derive this from their content.
    pub profile_match: Option<(String, bool)>,
    /// Compatibility extension, outside the legacy deterministic command-history body.
    pub metadata: Option<ta_config::records::RecordMetadata>,
    /// The mods it was made with, in load order (version 46 on, in a trailer of their own after
    /// the body; `None` before, when records did not say).
    pub mods: Option<Vec<ta_config::records::RecordMod>>,
    /// The player chose to open it as it is, though its game data or mods are not as they were
    /// (asked first: `record_mods`): a saved game then carries on from wherever today's data takes
    /// its orders. Never written.
    pub opened_anyway: bool,
    pub version: u32,
    pub kind: RecordKind,
    /// Legacy global game-data fingerprint. Version 41 and later also have a match-scoped hash.
    pub data_hash: u64,
    pub setup: GameSetup,
    /// The constructors on duty when a saved game was made, as `ta_ai::DutyOfficer::export` writes them
    /// (version 44 on; empty for none, for replays and for older records). Client state, in no command.
    pub duties: Vec<u8>,
    /// Orders by tick; complete modern streams include AI decisions too. Empty ticks are omitted.
    pub ticks: Vec<(u64, Vec<Command>)>,
    /// Last simulated tick.
    pub end_tick: u64,
    /// Unix time the record was written.
    pub saved_at: u64,
    /// Short description for lists ("Painted Desert — 2 players").
    pub title: String,
}

/// Fields in order, the setup as the record's version lays it out: a record opened and written
/// again keeps its bytes (before version 42 a Conquest battle had no reinforcements, before 43
/// the rules no mutators).
impl Serialize for Record {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut fields = serializer.serialize_struct("Record", if self.version >= 44 { 9 } else { 8 })?;
        fields.serialize_field("version", &self.version)?;
        fields.serialize_field("kind", &self.kind)?;
        fields.serialize_field("data_hash", &self.data_hash)?;
        if self.version < 42 {
            fields.serialize_field("setup", &crate::setup::LegacySetup::from(&self.setup))?;
        } else if self.version < 43 {
            fields.serialize_field("setup", &crate::setup::SetupV42::from(&self.setup))?;
        } else {
            fields.serialize_field("setup", &self.setup)?;
        }
        if self.version >= 44 {
            fields.serialize_field("duties", &self.duties)?;
        }
        fields.serialize_field("ticks", &self.ticks)?;
        fields.serialize_field("end_tick", &self.end_tick)?;
        fields.serialize_field("saved_at", &self.saved_at)?;
        fields.serialize_field("title", &self.title)?;
        fields.end()
    }
}

/// As [`Record::decode`] reads one, within the whole memory budget.
impl<'de> Deserialize<'de> for Record {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Record, D::Error> {
        RecordSeed(MAX_RECORDING_MEMORY).deserialize(deserializer)
    }
}

/// The menu keeps metadata only. Keeping every record's full command history made the
/// saved-games page retain the combined size of all matches until the menu was dropped.
pub struct RecordSummary {
    pub version: u32,
    pub kind: RecordKind,
    pub data_hash: u64,
    pub setup: GameSetup,
    pub metadata: Option<ta_config::records::RecordMetadata>,
    pub mods: Option<Vec<ta_config::records::RecordMod>>,
    pub end_tick: u64,
    pub saved_at: u64,
    pub title: String,
    /// What it is to the game of now ([`RecordSummary::verdict_now`]), once worked out.
    pub verdict: Option<crate::record_mods::Verdict>,
}

impl From<Record> for RecordSummary {
    fn from(record: Record) -> Self {
        Self { version: record.version, kind: record.kind, data_hash: record.data_hash, setup: record.setup, metadata: record.metadata, mods: record.mods, end_tick: record.end_tick, saved_at: record.saved_at, title: record.title, verdict: None }
    }
}

impl RecordSummary {
    /// Game length as "mm:ss".
    pub fn duration(&self) -> String {
        let s = self.end_tick / 30;
        format!("{:02}:{:02}", s / 60, s % 60)
    }

    /// What it is to the game of now (its data, its simulation, its mods against `installed`).
    pub fn verdict_now(&self, assets: &crate::setup::Assets, installed: &[ta_config::records::RecordMod]) -> crate::record_mods::Verdict {
        crate::record_mods::Verdict::of(self.version, self.metadata.as_ref(), self.data_hash, &self.setup, self.mods.as_deref(), assets, installed)
    }
}

const LATEST_SAVE_CACHE_LIMIT: usize = 64;

/// Metadata-only results for the Continue scan, including unreadable/incompatible
/// candidates. Never retain decoded histories or large embedded scenario sources.
#[derive(Clone, Default)]
pub struct LatestSaveCache {
    asset_hash: Option<u64>,
    entries: Vec<LatestSaveEntry>,
}

#[derive(Clone)]
struct LatestSaveEntry {
    path: PathBuf,
    modified: std::time::SystemTime,
    size: u64,
    title: Option<String>,
}

impl Record {
    /// Validate at the execution boundary, including records opened from the command
    /// line or replay controls, which do not pass through the menu's disabled buttons.
    pub fn validate(&self, data_hash: u64) -> anyhow::Result<()> {
        anyhow::ensure!(ta_config::records::simulation_compatible(self.version, self.metadata.as_ref()), "made by a different version of the game");
        anyhow::ensure!(self.recorded_data_hash() == data_hash, ta_ui::i18n::t("Made with different game data or mods"));
        self.validate_structure()?;
        Ok(())
    }

    /// Historical replays can be watched on a newer simulation or changed data, but
    /// their outcome may differ. Never use this relaxed boundary to resume a saved game.
    /// The caller supplies the scoped hash for new records, or global hash for legacy ones.
    pub fn validate_playback(&self, data_hash: u64) -> anyhow::Result<bool> {
        anyhow::ensure!(self.kind == RecordKind::Replay, "saved games require strict compatibility validation");
        anyhow::ensure!(ta_config::records::record_version_readable(self.version), "made by an unsupported version of the game");
        self.validate_structure()?;
        Ok(!ta_config::records::simulation_compatible(self.version, self.metadata.as_ref())
            || self.recorded_data_hash() != data_hash
            || self.metadata.as_ref().is_none_or(|m| m.engine_build != crate::version_label()))
    }

    /// A saved game the player chose to load though its game data or mods are not as they were
    /// ([`Record::opened_anyway`]): its simulation must still be this one (its orders mean the
    /// same), but its data may differ, and so it plays its orders over into another battle.
    pub fn validate_anyway(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.opened_anyway, "saved games require strict compatibility validation");
        anyhow::ensure!(ta_config::records::simulation_compatible(self.version, self.metadata.as_ref()), "made by a different version of the game");
        self.validate_structure()
    }

    pub fn recorded_data_hash(&self) -> u64 {
        self.metadata.as_ref().map_or(self.data_hash, |m| m.match_data_hash)
    }

    fn validate_structure(&self) -> anyhow::Result<()> {
        anyhow::ensure!(ta_config::records::record_version_readable(self.version), "made by an unsupported version of the game");
        anyhow::ensure!((self.version >= 41) == self.metadata.is_some(), "record compatibility metadata does not match its format");
        anyhow::ensure!(ta_config::records::record_has_mods(self.version) == self.mods.is_some(), "record mods do not match its format");
        anyhow::ensure!(self.mods.as_ref().is_none_or(|m| m.len() <= ta_config::records::MAX_RECORD_MODS), "record mods exceed size limit");
        self.setup.validate()?;
        anyhow::ensure!(self.title.len() <= 4096, "record title exceeds size limit");
        anyhow::ensure!(self.duties.len() <= ta_ai::MAX_SAVED_BYTES && (self.version >= 44 || self.duties.is_empty()), "record duties do not match its format");
        anyhow::ensure!(self.end_tick < ta_config::MAX_RECORD_TICKS, "invalid final replay tick");
        let mut previous = 0;
        let mut memory = RECORDING_BASE_MEMORY;
        for (tick, _) in &self.ticks {
            anyhow::ensure!(*tick > previous && *tick <= self.end_tick, "invalid replay tick sequence");
            previous = *tick;
        }
        for (_, commands) in &self.ticks {
            memory = memory.saturating_add(Recording::bundle_memory(commands));
            anyhow::ensure!(memory <= MAX_RECORDING_MEMORY, "decoded replay history exceeds memory limit");
        }
        Ok(())
    }

    /// A record made now, with the mods in play (`mods`, in load order: `record_mods::InPlay::record`).
    pub fn new(kind: RecordKind, data_hash: u64, mods: Vec<ta_config::records::RecordMod>, setup: GameSetup, ticks: Vec<(u64, Vec<Command>)>, end_tick: u64, title: String) -> Record {
        let saved_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let metadata = Some(ta_config::records::RecordMetadata {
            simulation_version: ta_config::SIMULATION_VERSION, match_data_hash: data_hash,
            engine_build: crate::version_label(), full_commands: false,
        });
        Record { version: VERSION, kind, data_hash, setup, duties: Vec::new(), ticks, end_tick, saved_at, title, profile_match: None, metadata, mods: Some(mods), opened_anyway: false }
    }

    /// A saved battle retains the same career identity through autosaves, named saves,
    /// copied files, quick saves and subsequent reloads. Replays never earn career XP.
    pub fn with_profile(mut self, game: &crate::game::Game) -> Self {
        self.profile_match = Some((game.profile_progress.id.clone(), game.profile_progress.disqualified || game.cheated() || game.staged));
        if let Some(metadata) = &mut self.metadata {
            metadata.full_commands = true;
            metadata.match_data_hash = game.record_data_hash;
        }
        self
    }

    /// A saved battle keeps the constructors on duty with it: they are the player's machine's own
    /// (the orders they led to are in the commands, the duties are in none), so a loaded battle would
    /// otherwise start with every constructor under command. Replays have none.
    pub fn with_duties(mut self, game: &crate::game::Game) -> Self {
        if self.kind == RecordKind::Save {
            self.duties = game.duties_saved();
        }
        self
    }

    fn encode(&self) -> anyhow::Result<Vec<u8>> {
        let mut bytes = codec().serialize(self)?;
        // (From version 46 the mods it was made with, between the body and the metadata.)
        if let Some(mods) = &self.mods {
            ta_config::records::append_mods(&mut bytes, mods)?;
        }
        if let Some(metadata) = &self.metadata {
            ta_config::records::append_metadata(&mut bytes, metadata)?;
        }
        if let Some((id, excluded)) = &self.profile_match {
            ta_config::profiles::append_record_metadata(&mut bytes, id, *excluded)?;
        }
        Ok(bytes)
    }

    fn dir(kind: RecordKind) -> PathBuf {
        ta_config::player_data_dir().join(match kind {
            RecordKind::Replay => "replays",
            RecordKind::Save => "saves",
        })
    }

    /// Writes the record into the user's replays/saves folder; returns the file path.
    pub fn write(&self) -> anyhow::Result<PathBuf> {
        self.write_as(&self.file_stem())
    }

    fn file_stem(&self) -> String {
        let stamp = self.saved_at;
        // Reserve room for the timestamp, identity, extension and atomic temp suffix,
        // including on filesystems whose 255-name limit counts UTF-8 bytes.
        let mut map = String::new();
        for c in self.setup.map.chars().filter(|c| c.is_alphanumeric() || *c == ' ') {
            if map.len() + c.len_utf8() > 128 { break; }
            map.push(c);
        }
        // Named saves and automatic replay archives must never replace another record
        // just because two writes use the same map and wall-clock second.
        format!("{stamp}-{}-{}", map.trim().replace(' ', "_"), ta_config::profiles::new_id())
    }

    /// Writes under a fixed file stem (the autosave overwrites its own file).
    pub fn write_as(&self, stem: &str) -> anyhow::Result<PathBuf> {
        self.write_in(&Self::dir(self.kind), stem)
    }

    fn write_in(&self, dir: &Path, stem: &str) -> anyhow::Result<PathBuf> {
        self.validate_structure()?;
        let ext = match self.kind {
            RecordKind::Replay => "tarep",
            RecordKind::Save => "tasav",
        };
        let path = dir.join(format!("{stem}.{ext}"));
        let bytes = self.encode()?;
        ta_config::write_atomic_bytes(&path, &bytes)?;
        Ok(path)
    }

    pub fn read(path: &Path) -> anyhow::Result<Record> {
        let file = std::fs::File::open(path)?;
        anyhow::ensure!(file.metadata()?.len() <= MAX_RECORD_BYTES, "record exceeds size limit");
        let mut bytes = Vec::new();
        file.take(MAX_RECORD_BYTES + 1).read_to_end(&mut bytes)?;
        anyhow::ensure!(bytes.len() as u64 <= MAX_RECORD_BYTES, "record exceeds size limit");
        Self::decode(&bytes)
    }

    fn decode(bytes: &[u8]) -> anyhow::Result<Record> {
        Self::decode_with_budget(bytes, MAX_RECORDING_MEMORY)
    }

    fn decode_with_budget(bytes: &[u8], budget: usize) -> anyhow::Result<Record> {
        // The version comes first: check it before the rest, whose layout may have changed.
        let version = u32::from_le_bytes(bytes.get(..4).ok_or_else(|| anyhow::anyhow!("truncated record"))?.try_into()?);
        anyhow::ensure!(ta_config::records::record_version_readable(version), "made by an unsupported version of the game");
        let metadata = ta_config::profiles::record_metadata(bytes).map(|(id, excluded)| (id.to_string(), excluded));
        let bytes = if metadata.is_some() { &bytes[..bytes.len() - ta_config::profiles::RECORD_PROFILE_TRAILER_LEN] } else { bytes };
        let (body, compatibility) = ta_config::records::split_metadata(bytes)?;
        anyhow::ensure!((version >= 41) == compatibility.is_some(), "record compatibility metadata does not match its format");
        let (body, mods) = if ta_config::records::record_has_mods(version) {
            let (body, mods) = ta_config::records::split_mods(body)?;
            (body, Some(mods))
        } else {
            (body, None)
        };
        let mut record = codec().deserialize_seed(RecordSeed(budget), body)?;
        record.metadata = compatibility;
        record.mods = mods;
        record.validate_structure()?;
        record.profile_match = Some(metadata.unwrap_or_else(|| {
            let digest = Sha256::digest(body);
            (digest[..16].iter().map(|b| format!("{b:02x}")).collect(), false)
        }));
        Ok(record)
    }

    /// Candidate records, newest file first; callers can fall back past unreadable files.
    pub fn files(kind: RecordKind) -> Vec<(PathBuf, std::time::SystemTime)> {
        Self::files_in(&Self::dir(kind), kind)
    }

    fn files_in(dir: &Path, kind: RecordKind) -> Vec<(PathBuf, std::time::SystemTime)> {
        let ext = match kind { RecordKind::Save => "tasav", RecordKind::Replay => "tarep" };
        let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
        let mut files: Vec<_> = entries.flatten().filter_map(|e| {
            let path = e.path();
            if !path.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case(ext)) { return None; }
            let metadata = e.metadata().ok()?;
            if !metadata.is_file() { return None; }
            Some((path, metadata.modified().ok()?))
        }).collect();
        files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        files
    }

    pub fn latest_save(assets: &crate::setup::Assets, cache: &mut LatestSaveCache) -> Option<(PathBuf, std::time::SystemTime, String)> {
        Self::latest_save_in(Self::files(RecordKind::Save), cache, assets.data_hash,
            |record| record.validate(assets.expected_record_hash(record.version, &record.setup)).is_ok())
    }

    fn latest_save_in(files: Vec<(PathBuf, std::time::SystemTime)>, cache: &mut LatestSaveCache, asset_hash: u64, compatible: impl Fn(&Record) -> bool) -> Option<(PathBuf, std::time::SystemTime, String)> {
        // The global identity includes every scoped map/rules hash. A newly enabled mod
        // or changed map must invalidate both accepted and rejected cached candidates.
        if cache.asset_hash != Some(asset_hash) {
            cache.entries.clear();
            cache.asset_hash = Some(asset_hash);
        }
        let mut refreshed = Vec::new();
        let mut latest = None;
        for (path, modified) in files {
            let Ok(metadata) = std::fs::metadata(&path) else { continue };
            if !metadata.is_file() { continue; }
            let size = metadata.len();
            let title = if let Some(entry) = cache.entries.iter().find(|e| e.path == path && e.modified == modified && e.size == size) {
                entry.title.clone()
            } else {
                Self::read(&path).ok().filter(|record| record.kind == RecordKind::Save && compatible(record)).map(|record| record.title)
            };
            if let Some(title) = &title { latest = Some((path.clone(), modified, title.clone())); }
            let entry = LatestSaveEntry { path, modified, size, title };
            if refreshed.len() < LATEST_SAVE_CACHE_LIMIT {
                refreshed.push(entry);
            } else if latest.is_some() {
                // Keep the selected save cached even behind a long list of damaged or
                // incompatible files, while placing a fixed bound on retained metadata.
                refreshed[LATEST_SAVE_CACHE_LIMIT - 1] = entry;
            }
            if latest.is_some() { break; }
        }
        cache.entries = refreshed;
        latest
    }

    /// Records of one kind, newest first: (path, metadata). Full histories are read again
    /// only when selected, and are revalidated then in case the file changed.
    pub fn list(kind: RecordKind) -> Vec<(PathBuf, RecordSummary)> {
        let mut out: Vec<(PathBuf, RecordSummary)> = Self::files(kind).into_iter()
            .filter_map(|(p, _)| Record::read(&p).ok().filter(|r| r.kind == kind).map(|r| (p, r.into())))
            .collect();
        out.sort_by_key(|(_, r)| std::cmp::Reverse(r.saved_at));
        out
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Record {
        Record::new(RecordKind::Save, 123, Vec::new(), GameSetup::duel("QA map"), vec![], 30, "Save".into())
    }

    #[test]
    fn named_records_in_the_same_second_have_distinct_bounded_names() {
        let mut record = fixture();
        record.setup.map = "𐐀界".repeat(1000);
        let a = record.file_stem();
        let b = record.file_stem();
        assert_ne!(a, b, "a second save/archive must not replace the first");
        assert!(format!("{a}.tasav.4294967295-18446744073709551615.tmp").len() <= 255,
            "both the final filename and its atomic staging name must fit byte-limited filesystems");
        assert!(!a.contains(['/', '\\']));
    }

    #[test]
    fn simultaneous_rolling_saves_use_private_temporary_files() {
        let dir = std::env::temp_dir().join(format!("ta-record-writers-{}", ta_config::profiles::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let abandoned = dir.join("quicksave.tmp");
        std::fs::write(&abandoned, b"belongs to another writer").unwrap();
        std::thread::scope(|scope| {
            for n in 0..4 {
                let dir = &dir;
                scope.spawn(move || {
                    let mut record = fixture();
                    record.title = format!("Writer {n}");
                    for _ in 0..4 { record.write_in(dir, "quicksave").unwrap(); }
                });
            }
        });
        let saved = Record::read(&dir.join("quicksave.tasav")).unwrap();
        assert!(saved.title.starts_with("Writer "));
        assert_eq!(std::fs::read(&abandoned).unwrap(), b"belongs to another writer");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn continue_falls_back_past_corrupt_or_wrong_kind_records() {
        use std::time::{Duration, SystemTime};
        let dir = std::env::temp_dir().join(format!("ta-record-fallback-{}", ta_config::profiles::new_id()));
        let valid = fixture().write_in(&dir, "valid").unwrap();
        let corrupt = dir.join("corrupt.tasav");
        std::fs::write(&corrupt, b"truncated").unwrap();
        let mut replay = fixture();
        replay.kind = RecordKind::Replay;
        let wrong = dir.join("wrong.tasav");
        std::fs::write(&wrong, replay.encode().unwrap()).unwrap();
        let time = SystemTime::UNIX_EPOCH;
        let files = vec![(corrupt, time + Duration::from_secs(2)), (wrong, time + Duration::from_secs(1)), (valid.clone(), time)];
        let mut cache = LatestSaveCache::default();
        let selected = Record::latest_save_in(files.clone(), &mut cache, 123, |record| record.validate(123).is_ok()).unwrap();
        assert_eq!(selected.0, valid);
        assert_eq!(Record::latest_save_in(files, &mut cache, 123, |_| panic!("unchanged candidates use cached validation")).unwrap(), selected);
        assert!(Record::latest_save_in(Vec::new(), &mut cache, 123, |_| true).is_none());
        assert!(cache.entries.is_empty(), "removed files do not remain in the cache");

        replay.write_in(&dir, "replay").unwrap();
        std::fs::create_dir(dir.join("directory.tasav")).unwrap();
        assert_eq!(Record::files_in(&dir, RecordKind::Replay).len(), 1);
        assert_eq!(Record::files_in(&dir, RecordKind::Save).len(), 3);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn recording_budget_drops_incomplete_history_and_notifies_once() {
        let commands = vec![Command::Resign { player: 0 }];
        let mut recording = Recording { limit: RECORDING_BASE_MEMORY + Recording::bundle_memory(&commands), ..Default::default() };
        assert!(!recording.append(1, &commands));
        assert_eq!(recording.snapshot().unwrap().len(), 1);
        assert!(recording.append(2, &commands));
        assert!(!recording.available());
        assert!(recording.snapshot().is_none());
        assert_eq!(recording.ticks.capacity(), 0, "overflow releases retained allocations");
        assert_eq!(commands.len(), 1, "the simulation's commands are unchanged");
        assert!(!recording.append(3, &commands), "the notice occurs only once");
        recording.retain_through(0);
        assert!(!recording.available(), "rewinding cannot repair a missing history");
    }

    #[test]
    fn recording_budget_counts_nested_commands_and_reclaims_rewound_history() {
        let commands = vec![Command::Order { player: 0, units: vec![ta_sim::UnitId { slot: 0, generation: 0 }; 16], order: ta_sim::Order::Stop, queue: false }];
        let cost = Recording::bundle_memory(&commands);
        assert!(cost > 2 * (std::mem::size_of::<Command>() + std::mem::size_of::<(u64, Vec<Command>)>()));
        let mut recording = Recording { limit: RECORDING_BASE_MEMORY + cost, ..Default::default() };
        assert!(!recording.append(1, &commands));
        recording.retain_through(0);
        assert!(!recording.append(1, &commands));
        assert!(recording.available());
        assert!(!recording.append(2, &[]));
        assert_eq!(recording.snapshot().unwrap().len(), 1);
    }

    #[test]
    fn replay_decode_rejects_amplified_counts_before_reading_their_payload() {
        // (Version 40's setup, as it laid it out.)
        let setup = crate::setup::LegacySetup::from(&GameSetup::duel("test"));
        let prefix = bincode::serialize(&(40u32, RecordKind::Replay, 1u64, &setup)).unwrap();
        let mut bytes = prefix.clone();
        bytes.extend_from_slice(&u64::MAX.to_le_bytes()); // enormous tick count, no body
        assert!(Record::decode_with_budget(&bytes, 4096).unwrap_err().to_string().contains("memory limit"));
        bytes = prefix;
        bytes.extend_from_slice(&1u64.to_le_bytes()); // one tick
        bytes.extend_from_slice(&1u64.to_le_bytes());
        bytes.extend_from_slice(&100_000_000u64.to_le_bytes()); // tiny-command amplification
        assert!(Record::decode_with_budget(&bytes, 4096).unwrap_err().to_string().contains("memory limit"), "the count fails before any missing command payload is read");
        let tick_at = bytes.len() - 16;
        bytes[tick_at..tick_at + 8].copy_from_slice(&0u64.to_le_bytes());
        assert!(Record::decode_with_budget(&bytes, 4096).unwrap_err().to_string().contains("tick sequence"), "validate the tick before its commands");
    }

    #[test]
    fn replay_decode_budget_is_cumulative_and_preserves_fixed_width_layout() {
        let id = ta_sim::UnitId { slot: 7, generation: 3 };
        let commands = vec![
            Command::Resign { player: 0 },
            Command::Formation { player: 0, units: vec![id; 300], order: ta_sim::Order::Move(ta_sim::FxVec3::ZERO), queue: true, shape: ta_sim::formation::Shape::Spots(vec![ta_sim::FxVec3::ZERO; 300]) },
            Command::Cheat { player: 0, cheat: ta_sim::Cheat::Heal(vec![id; 300]) },
        ];
        let record = Record::new(RecordKind::Save, 1, Vec::new(), GameSetup::duel("test"), vec![(1, commands.clone()), (2, commands)], 2, "roundtrip".into());
        let bytes = record.encode().unwrap();
        let budget = RECORDING_BASE_MEMORY + record.ticks.iter().map(|(_, c)| Recording::bundle_memory(c)).sum::<usize>();
        let decoded = Record::decode_with_budget(&bytes, budget).unwrap();
        assert_eq!(decoded.ticks, record.ticks);
        assert_eq!(bincode::serialize(&decoded).unwrap(), bincode::serialize(&record).unwrap(), "the fixed-width body representation is unchanged");
        assert_eq!(decoded.metadata, record.metadata);
        assert!(Record::decode_with_budget(&bytes, budget - 1).unwrap_err().to_string().contains("memory limit"));
    }

    #[test]
    fn command_and_setup_vector_limits_reject_forged_lengths_before_payload() {
        let too_many = (ta_sim::serde_limits::MAX_COMMAND_ITEMS as u64 + 1).to_le_bytes();
        let commands = [
            Command::Order { player: 0, units: vec![], order: ta_sim::Order::Stop, queue: false },
            Command::SetActive { player: 0, units: vec![], on: true },
            Command::SetCloak { player: 0, units: vec![], on: true },
            Command::SetStates { player: 0, units: vec![], fire: None, movement: None },
            Command::SelfDestruct { player: 0, units: vec![] },
            Command::SetRepeat { player: 0, factories: vec![], on: true },
            Command::GiveUnits { player: 0, units: vec![], to: 1 },
            Command::Formation { player: 0, units: vec![], order: ta_sim::Order::Stop, queue: false, shape: ta_sim::formation::Shape::Keep },
        ];
        for command in commands {
            let mut bytes = bincode::serialize(&command).unwrap();
            bytes[5..13].copy_from_slice(&too_many);
            assert!(bincode::deserialize::<Command>(&bytes).unwrap_err().to_string().contains("item limit"));
        }
        for cheat in [ta_sim::Cheat::Destroy(vec![]), ta_sim::Cheat::Heal(vec![]), ta_sim::Cheat::Promote(vec![])] {
            let mut bytes = bincode::serialize(&cheat).unwrap();
            bytes[4..12].copy_from_slice(&too_many);
            assert!(bincode::deserialize::<ta_sim::Cheat>(&bytes).unwrap_err().to_string().contains("item limit"));
        }
        let mut bytes = bincode::serialize(&ta_sim::formation::Shape::Spots(vec![])).unwrap();
        bytes[4..12].copy_from_slice(&too_many);
        assert!(bincode::deserialize::<ta_sim::formation::Shape>(&bytes).unwrap_err().to_string().contains("item limit"));
        let mut bytes = bincode::serialize(&GameSetup::duel("")).unwrap();
        bytes[8..16].copy_from_slice(&65u64.to_le_bytes());
        assert!(bincode::deserialize::<GameSetup>(&bytes).unwrap_err().to_string().contains("item limit"));
        let conquest = crate::setup::ConquestStart { seed: 1, battle: 0, system: 0, metal: 0, energy: 0, commander_xp: ta_sim::Fx::ZERO, veterans: vec![], reinforcements: vec![], defence: false, perks: vec![] };
        // (Its veterans' and reinforcements' lengths, round the defence flag, and its perks'.)
        for from_end in [25, 17, 8] {
            let mut bytes = bincode::serialize(&conquest).unwrap();
            let at = bytes.len() - from_end;
            bytes[at..at + 8].copy_from_slice(&too_many);
            assert!(bincode::deserialize::<crate::setup::ConquestStart>(&bytes).unwrap_err().to_string().contains("item limit"));
        }
    }

    #[test]
    fn records_before_version_42_read_their_conquest_battles_as_laid_out_then() {
        let legacy = crate::setup::LegacySetup {
            map: "test".into(),
            players: GameSetup::duel("test").players,
            local_player: Some(0),
            seed: 7,
            mission: None,
            rules: crate::setup::LegacyRules::from(&ta_sim::GameRules::default()),
            conquest: Some(crate::setup::LegacyConquestStart { seed: 3, battle: 2, system: 4, metal: 150, energy: 300, commander_xp: ta_sim::Fx::from_int(5), veterans: vec![("ARMZEUS".into(), ta_sim::Fx::from_int(4))] }),
            scenario: None,
        };
        let ticks: Vec<(u64, Vec<Command>)> = Vec::new();
        let mut bytes = codec().serialize(&(41u32, RecordKind::Save, 123u64, &legacy, &ticks, 0u64, 1_761_000_000u64, "an older conquest")).unwrap();
        let metadata = ta_config::records::RecordMetadata { simulation_version: ta_config::SIMULATION_VERSION, match_data_hash: 123, engine_build: "fixture".into(), full_commands: false };
        ta_config::records::append_metadata(&mut bytes, &metadata).unwrap();
        let record = Record::decode(&bytes).unwrap();
        assert_eq!(record.version, 41);
        let old = record.setup.conquest.clone().unwrap();
        assert_eq!((old.seed, old.battle, old.system, old.metal, old.energy), (3, 2, 4, 150, 300));
        assert_eq!(old.veterans, vec![("ARMZEUS".to_string(), ta_sim::Fx::from_int(4))]);
        assert!(old.reinforcements.is_empty() && !old.defence);
        // Today's records keep a battle's reinforcements and whether it is a defence.
        let mut setup = record.setup;
        setup.conquest = Some(crate::setup::ConquestStart { reinforcements: vec!["ARMSTUMP".into()], defence: true, ..old });
        let current = Record::new(RecordKind::Save, 123, Vec::new(), setup, Vec::new(), 0, "a defence".into());
        let read = Record::decode(&current.encode().unwrap()).unwrap();
        assert_eq!(read.version, VERSION);
        let conquest = read.setup.conquest.unwrap();
        assert_eq!(conquest.reinforcements, vec!["ARMSTUMP".to_string()]);
        assert!(conquest.defence);
    }

    #[test]
    fn records_of_version_42_read_their_rules_as_laid_out_then() {
        // (Version 42's rules had no mutators.)
        let old = crate::setup::SetupV42::from(&GameSetup::duel("before the mutators"));
        let ticks: Vec<(u64, Vec<Command>)> = Vec::new();
        let body = codec().serialize(&(42u32, RecordKind::Save, 123u64, &old, &ticks, 0u64, 1_761_000_000u64, "an older battle")).unwrap();
        let mut bytes = body.clone();
        let metadata = ta_config::records::RecordMetadata { simulation_version: ta_config::SIMULATION_VERSION, match_data_hash: 123, engine_build: "fixture".into(), full_commands: false };
        ta_config::records::append_metadata(&mut bytes, &metadata).unwrap();
        let record = Record::decode(&bytes).unwrap();
        assert_eq!(record.version, 42);
        assert_eq!(record.setup.map, "before the mutators");
        assert_eq!(record.setup.rules.mutators, ta_sim::Mutators::default());
        assert_eq!(bincode::serialize(&record).unwrap(), body, "written again as it was laid out");
        // Today's records keep the mutators.
        let mut setup = record.setup;
        setup.rules.mutators = ta_sim::Mutators::parse("no_rush=5,resources=8,no_air");
        let current = Record::new(RecordKind::Save, 123, Vec::new(), setup, Vec::new(), 0, "with mutators".into());
        let read = Record::decode(&current.encode().unwrap()).unwrap();
        assert_eq!(read.version, VERSION);
        assert_eq!(read.setup.rules.mutators, ta_sim::Mutators::parse("no_rush=5,resources=8,no_air"));
    }

    #[test]
    fn records_of_version_43_have_no_duties_and_are_written_as_they_were() {
        // (Version 43 had no constructors on duty: a few bytes were added after the setup in 44.)
        let ticks: Vec<(u64, Vec<Command>)> = Vec::new();
        let body = codec().serialize(&(43u32, RecordKind::Save, 123u64, &GameSetup::duel("before the duties"), &ticks, 0u64, 1_761_000_000u64, "an older save")).unwrap();
        let mut bytes = body.clone();
        let metadata = ta_config::records::RecordMetadata { simulation_version: ta_config::SIMULATION_VERSION, match_data_hash: 123, engine_build: "fixture".into(), full_commands: false };
        ta_config::records::append_metadata(&mut bytes, &metadata).unwrap();
        let record = Record::decode(&bytes).unwrap();
        assert_eq!(record.version, 43);
        assert_eq!(record.setup.map, "before the duties");
        assert!(record.duties.is_empty());
        assert_eq!(bincode::serialize(&record).unwrap(), body, "written again as it was laid out");
        assert!(record.validate(123).is_ok(), "an older save still resumes");
        // A record of that layout has nowhere to keep duties.
        let mut forged = record;
        forged.duties = vec![1];
        assert!(forged.validate(123).is_err());
    }

    fn mayhem() -> ta_config::records::RecordMod {
        ta_config::records::RecordMod { id: "totalmayhem".into(), name: "Total Mayhem".into(), version: "11.3.0".into(), digest: [3; 16] }
    }

    /// From version 46 a record keeps the mods it was made with, in load order, after its body:
    /// the body is as it was, and a record made without mods says so (an empty list).
    #[test]
    fn records_keep_their_mods_in_load_order() {
        let escalation = ta_config::records::RecordMod { id: "escalation".into(), name: "TA: Escalation".into(), version: "Gold 10.2.0".into(), digest: [9; 16] };
        for mods in [vec![mayhem(), escalation], Vec::new()] {
            let mut record = Record::new(RecordKind::Save, 123, mods.clone(), GameSetup::duel("test"), vec![(1, vec![Command::Resign { player: 1 }])], 30, "with mods".into());
            record.profile_match = Some((ta_config::profiles::new_id(), false));
            let bytes = record.encode().unwrap();
            let body = codec().serialize(&record).unwrap();
            assert_eq!(&bytes[..body.len()], &body[..], "the body comes first, as it was");
            let read = Record::decode(&bytes).unwrap();
            assert_eq!((read.version, read.mods.clone()), (VERSION, Some(mods)));
            assert_eq!((read.ticks.clone(), read.title.as_str(), read.profile_match.clone()), (record.ticks.clone(), "with mods", record.profile_match.clone()));
            assert_eq!(read.encode().unwrap(), bytes, "written again as it was");
            assert!(read.validate(123).is_ok());
        }
        // Without its mods' trailer a record of today's version does not read.
        let mut record = fixture();
        record.mods = None;
        assert!(record.encode().is_ok_and(|bytes| Record::decode(&bytes).is_err()));
        assert!(record.validate(123).is_err(), "nor is it written so");
    }

    /// A record of version 45 or before does not say which mods it was made with: it reads with
    /// none known, is checked by its data hash alone as before, and is written again without any.
    #[test]
    fn records_before_version_46_name_no_mods_and_are_written_as_they_were() {
        let ticks: Vec<(u64, Vec<Command>)> = Vec::new();
        let body = codec().serialize(&(45u32, RecordKind::Save, 123u64, &GameSetup::duel("before the mods"), Vec::<u8>::new(), &ticks, 0u64, 1_761_000_000u64, "an older save")).unwrap();
        let mut bytes = body.clone();
        let metadata = ta_config::records::RecordMetadata { simulation_version: ta_config::SIMULATION_VERSION, match_data_hash: 123, engine_build: "fixture".into(), full_commands: true };
        ta_config::records::append_metadata(&mut bytes, &metadata).unwrap();
        let record = Record::decode(&bytes).unwrap();
        assert_eq!((record.version, record.mods.clone()), (45, None));
        assert!(record.validate(123).is_ok(), "an older save still resumes");
        assert!(record.validate(456).is_err(), "and is still refused with other data");
        assert_eq!(bincode::serialize(&record).unwrap(), body, "written again as it was laid out");
        assert_eq!(Record::decode(&record.encode().unwrap()).unwrap().mods, None, "with no mods' trailer");
        // A record of that layout has nowhere to keep mods.
        let mut forged = record;
        forged.mods = Some(vec![mayhem()]);
        assert!(forged.validate(123).is_err());
        // Nor can a trailer of mods be slipped into one: its body would not read.
        let mut slipped = body;
        ta_config::records::append_mods(&mut slipped, &[mayhem()]).unwrap();
        ta_config::records::append_metadata(&mut slipped, &metadata).unwrap();
        assert!(Record::decode(&slipped).is_err());
    }

    /// A saved game made with other data is loaded only when the player chose to (asked first):
    /// then its simulation must still be this one; its data may differ.
    #[test]
    fn a_save_opened_anyway_is_checked_for_its_simulation_not_its_data() {
        let mut record = fixture();
        assert!(record.validate_anyway().is_err(), "not without the player's say");
        record.opened_anyway = true;
        record.metadata.as_mut().unwrap().match_data_hash = 456;
        assert!(record.validate(123).is_err() && record.validate_anyway().is_ok());
        record.metadata.as_mut().unwrap().simulation_version -= 1;
        assert!(record.validate_anyway().is_err(), "made by another simulation: its orders mean other things");
        let read = Record::decode(&fixture().encode().unwrap()).unwrap();
        assert!(!read.opened_anyway, "never kept in the file");
    }

    #[test]
    fn a_saved_game_keeps_its_duties_and_a_forged_length_is_refused_before_they_are_read() {
        let mut record = Record::new(RecordKind::Save, 123, Vec::new(), GameSetup::duel("test"), Vec::new(), 90, "on duty".into());
        record.duties = vec![1, 2, 0, 0, 0, 7, 0, 0, 0, 3, 0, 0, 0, 1];
        let bytes = record.encode().unwrap();
        let read = Record::decode(&bytes).unwrap();
        assert_eq!((read.version, read.duties.clone()), (VERSION, record.duties.clone()));
        assert_eq!((read.end_tick, read.title.as_str()), (90, "on duty"), "the fields after them are where they were");
        assert_eq!(bincode::serialize(&read).unwrap(), bincode::serialize(&record).unwrap(), "written again as it was");
        // (The length of the list is checked before any of it is read, like every other length in a record.)
        let mut prefix = codec().serialize(&(VERSION, RecordKind::Save, 123u64, &GameSetup::duel("test"))).unwrap();
        prefix.extend_from_slice(&(ta_ai::MAX_SAVED_BYTES as u64 + 1).to_le_bytes());
        ta_config::records::append_mods(&mut prefix, &[]).unwrap();
        let metadata = ta_config::records::RecordMetadata { simulation_version: ta_config::SIMULATION_VERSION, match_data_hash: 123, engine_build: "fixture".into(), full_commands: false };
        ta_config::records::append_metadata(&mut prefix, &metadata).unwrap();
        assert!(Record::decode(&prefix).unwrap_err().to_string().contains("duties exceed size limit"));
        // A record that is not a save keeps none.
        let replay = Record::new(RecordKind::Replay, 123, Vec::new(), GameSetup::duel("test"), Vec::new(), 0, "no duties".into());
        assert!(replay.duties.is_empty());
    }

    #[test]
    fn record_execution_rejects_changed_data_and_versions() {
        let mut record = Record::new(RecordKind::Save, 123, Vec::new(), GameSetup::duel("test"), Vec::new(), 0, "test".into());
        assert!(record.validate(123).is_ok());
        assert!(record.validate(456).is_err());
        // (An unknown version, and one before the compatibility metadata this record has.)
        for version in [VERSION + 1, 40] {
            record.version = version;
            assert!(record.validate(123).is_err());
        }
    }

    #[test]
    fn historical_wire_fixture_opens_without_rewriting_its_version_or_orders() {
        // Frozen bytes assembled from the original v36 declarations, not serialized
        // from today's Record/GameSetup/Command. See fixtures/README.md for provenance.
        let original = include_bytes!("replay/fixtures/format36.tarep");
        for version in 36u32..=40 {
            let mut bytes = original.to_vec();
            bytes[..4].copy_from_slice(&version.to_le_bytes());
            let record = Record::decode(&bytes).unwrap();
            assert_eq!(record.version, version);
            assert_eq!(record.metadata, None);
            assert_eq!(record.setup.map, "Legacy arena");
            assert_eq!(record.setup.players[1].controller, crate::setup::Controller::Ai(ta_ai::Difficulty::Hard));
            assert_eq!(record.setup.conquest.as_ref().unwrap().veterans, [("ARMFAV".into(), ta_sim::Fx::ONE)]);
            let id = ta_sim::UnitId { slot: 7, generation: 3 };
            let position = ta_sim::FxVec3::new(ta_sim::Fx::ONE, ta_sim::Fx::ZERO, ta_sim::Fx::from_int(2));
            assert_eq!(record.ticks, [
                (1, vec![
                    Command::Formation { player: 0, units: vec![id], order: ta_sim::Order::Patrol(position), queue: true, shape: ta_sim::formation::Shape::Spots(vec![position]) },
                    Command::Cheat { player: 0, cheat: ta_sim::Cheat::Heal(vec![id]) },
                ]),
                (2, vec![Command::Resign { player: 1 }]),
            ]);
            assert_eq!(record.end_tick, 30);
            assert_eq!(record.title, "Legacy fixture");
            assert_eq!(bincode::serialize(&record).unwrap(), bytes, "opening a legacy record never relabels its history");
            assert!(record.validate_playback(123).unwrap(), "legacy engine identity is unknown");
            assert!(record.validate_playback(456).unwrap(), "changed data is approximate playback");
        }
    }

    #[test]
    fn metadata_separates_layout_simulation_data_and_build_compatibility() {
        let mut record = fixture();
        let metadata = record.metadata.as_mut().unwrap();
        metadata.match_data_hash = 456;
        metadata.engine_build = "older source build".into();
        assert!(record.validate(456).is_ok(), "same simulation revision and match data can resume across builds");
        assert!(record.validate(123).is_err(), "new saves compare scoped data rather than the old global hash");
        assert!(record.validate_playback(456).is_err(), "saved games cannot bypass resume checks");
        record.kind = RecordKind::Replay;
        assert!(record.validate_playback(456).unwrap(), "a changed source build is disclosed");
        record.metadata.as_mut().unwrap().engine_build = crate::version_label();
        assert!(!record.validate_playback(456).unwrap());
        record.metadata.as_mut().unwrap().simulation_version += 1;
        assert!(record.validate(456).is_err());
        assert!(record.validate_playback(456).unwrap());
        record.metadata.as_mut().unwrap().full_commands = true;
        let restored = Record::decode(&record.encode().unwrap()).unwrap();
        assert_eq!(restored.metadata, record.metadata);
        assert!(restored.metadata.unwrap().full_commands);
    }

    #[test]
    fn new_format_requires_metadata_and_rejects_truncated_or_corrupt_extensions() {
        let record = fixture();
        let body = bincode::serialize(&record).unwrap();
        assert!(Record::decode(&body).is_err(), "format 41 cannot silently fall back to a legacy hash");
        let bytes = record.encode().unwrap();
        for length in body.len()..bytes.len() {
            assert!(Record::decode(&bytes[..length]).is_err());
        }
        let mut corrupt = bytes.clone();
        *corrupt.last_mut().unwrap() = 1;
        assert!(Record::decode(&corrupt).is_err());
        let mut forged = bytes;
        forged[..4].copy_from_slice(&40u32.to_le_bytes());
        assert!(Record::decode(&forged).is_err(), "a legacy version cannot smuggle new compatibility metadata");
    }

    #[test]
    fn continue_skips_saves_of_older_simulations() {
        use std::time::{Duration, SystemTime};
        let dir = std::env::temp_dir().join(format!("ta-record-versions-{}", ta_config::profiles::new_id()));
        // Newer on disk, made by older simulations: the layouts before metadata (39 and 40,
        // their revision their own number) and one whose metadata names the revision before.
        let mut legacy = fixture();
        legacy.metadata = None;
        legacy.mods = None;
        legacy.version = 39;
        let legacy39 = legacy.write_in(&dir, "legacy39").unwrap();
        legacy.version = 40;
        let legacy40 = legacy.write_in(&dir, "legacy40").unwrap();
        let mut older = fixture();
        older.metadata.as_mut().unwrap().simulation_version -= 1;
        let older = older.write_in(&dir, "older").unwrap();
        let current = fixture().write_in(&dir, "current").unwrap();
        let time = SystemTime::UNIX_EPOCH;
        let files = vec![(legacy39, time + Duration::from_secs(3)), (legacy40.clone(), time + Duration::from_secs(2)), (older.clone(), time + Duration::from_secs(1)), (current.clone(), time)];
        let selected = Record::latest_save_in(files, &mut LatestSaveCache::default(), 123, |record| record.validate(123).is_ok()).unwrap();
        assert_eq!(selected.0, current);
        assert!(Record::read(&current).unwrap().validate(123).is_ok());
        assert_eq!(Record::read(&legacy40).unwrap().validate(123).is_ok(), ta_config::SIMULATION_VERSION == 40);
        assert!(Record::read(&older).unwrap().validate(123).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn continue_falls_back_past_changed_data_and_rechecks_when_assets_change() {
        use std::time::{Duration, SystemTime};
        let dir = std::env::temp_dir().join(format!("ta-record-data-{}", ta_config::profiles::new_id()));
        let compatible = fixture().write_in(&dir, "compatible").unwrap();
        let mut changed = fixture();
        changed.metadata.as_mut().unwrap().match_data_hash = 456;
        let incompatible = changed.write_in(&dir, "changed-data").unwrap();
        let time = SystemTime::UNIX_EPOCH;
        let files = vec![(incompatible.clone(), time + Duration::from_secs(1)), (compatible.clone(), time)];
        let mut cache = LatestSaveCache::default();
        let selected = Record::latest_save_in(files.clone(), &mut cache, 1, |record| record.validate(123).is_ok()).unwrap();
        assert_eq!(selected.0, compatible, "a newer save for different map/mod data must not hide a resumable save");
        assert_eq!(cache.entries.len(), 2, "cache both the rejected and accepted candidates");
        assert_eq!(Record::latest_save_in(files.clone(), &mut cache, 1, |_| panic!("no repeated history decode")).unwrap(), selected);
        let selected = Record::latest_save_in(files, &mut cache, 2, |record| record.validate(456).is_ok()).unwrap();
        assert_eq!(selected.0, incompatible, "changed assets must invalidate negative compatibility results too");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn continue_cache_is_bounded_and_rechecks_replaced_files() {
        use std::time::{Duration, SystemTime};
        let dir = std::env::temp_dir().join(format!("ta-record-cache-{}", ta_config::profiles::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let time = SystemTime::UNIX_EPOCH;
        let mut files = Vec::new();
        for index in 0..LATEST_SAVE_CACHE_LIMIT + 3 {
            let path = dir.join(format!("broken{index}.tasav"));
            std::fs::write(&path, b"broken").unwrap();
            files.push((path, time));
        }
        let valid = fixture().write_in(&dir, "valid").unwrap();
        files.push((valid.clone(), time));
        let mut cache = LatestSaveCache::default();
        assert_eq!(Record::latest_save_in(files.clone(), &mut cache, 123, |r| r.validate(123).is_ok()).unwrap().0, valid);
        assert_eq!(cache.entries.len(), LATEST_SAVE_CACHE_LIMIT);
        assert!(cache.entries.iter().any(|entry| entry.path == valid), "retain the selected save beyond the cache limit");
        assert_eq!(Record::latest_save_in(files.clone(), &mut cache, 123, |_| panic!("selected save stays cached")).unwrap().0, valid);

        // A file's path alone is not an identity: replacing a failed autosave must make
        // it eligible even if coarse filesystem timestamps happen to be unchanged.
        let repaired = files[0].0.clone();
        std::fs::write(&repaired, fixture().encode().unwrap()).unwrap();
        assert_eq!(Record::latest_save_in(files.clone(), &mut cache, 123, |r| r.validate(123).is_ok()).unwrap().0, repaired);
        std::fs::write(&repaired, b"broken").unwrap();
        files[0].1 = time + Duration::from_secs(1);
        assert_eq!(Record::latest_save_in(files, &mut cache, 123, |r| r.validate(123).is_ok()).unwrap().0, valid);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Unknown layouts are refused before inspecting any bytes beyond their version.
    #[test]
    fn records_of_other_versions_are_refused() {
        let path = std::env::temp_dir().join(format!("ta-reborn-old-record-{}.tarep", std::process::id()));
        let mut bytes = bincode::serialize(&(VERSION + 1)).unwrap();
        bytes.extend([0xFF; 64]);
        std::fs::write(&path, &bytes).unwrap();
        let read = Record::read(&path);
        std::fs::remove_file(&path).ok();
        assert_eq!(read.unwrap_err().to_string(), "made by an unsupported version of the game");
    }

    #[test]
    fn malformed_records_are_rejected_before_execution() {
        let mut record = Record::new(RecordKind::Save, 123, Vec::new(), GameSetup::duel("test"), vec![(1, vec![]), (3, vec![])], 3, "test".into());
        assert!(Record::decode(&record.encode().unwrap()).is_ok(), "fixed-width codec stays compatible");
        for ticks in [vec![(0, vec![])], vec![(3, vec![]), (2, vec![])], vec![(2, vec![]), (2, vec![])], vec![(4, vec![])]] {
            record.ticks = ticks;
            assert!(Record::decode(&record.encode().unwrap()).is_err());
        }
        record.ticks.clear();
        record.end_tick = u64::MAX - 1;
        assert!(record.validate(123).is_err());
        record.end_tick = 3;
        record.setup.local_player = Some(2);
        assert!(record.validate(123).is_err());
        record.setup.local_player = None;
        assert!(record.validate(123).is_ok(), "spectators remain valid");
        record.setup.players.clear();
        assert!(record.validate(123).is_err());
        record.setup = GameSetup::duel("test");
        record.setup.players = vec![record.setup.players[0].clone(); ta_sim::unit::MAX_PLAYERS + 1];
        assert!(record.validate(123).is_err());
    }

    #[test]
    fn record_decoder_rejects_trailing_bytes_and_forged_lengths() {
        let record = Record::new(RecordKind::Replay, 1, Vec::new(), GameSetup::duel("test"), vec![], 0, "test".into());
        let mut bytes = record.encode().unwrap();
        bytes.push(0);
        assert!(Record::decode(&bytes).is_err());
        // version + kind + data hash precede the setup's map string length.
        bytes.truncate(24);
        bytes[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(Record::decode(&bytes).is_err());
    }

    #[test]
    fn career_trailer_retains_identity_without_changing_the_simulation_body() {
        let mut record = Record::new(RecordKind::Save, 123, Vec::new(), GameSetup::duel("test"), vec![], 900, "profile save".into());
        let old_body = bincode::serialize(&record).unwrap();
        let without_profile = record.encode().unwrap();
        let id = ta_config::profiles::new_id();
        record.profile_match = Some((id.clone(), true));
        let bytes = record.encode().unwrap();
        assert_eq!(&bytes[..old_body.len()], old_body);
        let decoded = Record::decode(&bytes).unwrap();
        assert_eq!(decoded.profile_match, Some((id, true)));
        assert_eq!(decoded.end_tick, 900);
        assert_eq!(decoded.version, VERSION);
        let legacy = Record::decode(&without_profile).unwrap();
        assert_eq!(legacy.profile_match, Record::decode(&without_profile).unwrap().profile_match, "derived identity follows content, never file path or load time");
        assert_ne!(legacy.profile_match, decoded.profile_match);
    }

    #[test]
    fn malformed_profile_trailers_are_not_silently_discarded() {
        let mut record = Record::new(RecordKind::Save, 1, Vec::new(), GameSetup::duel("test"), vec![], 0, "test".into());
        record.profile_match = Some((ta_config::profiles::new_id(), false));
        let mut bytes = record.encode().unwrap();
        *bytes.last_mut().unwrap() = 7;
        assert!(Record::decode(&bytes).is_err());
        bytes.truncate(bytes.len() - 1);
        assert!(Record::decode(&bytes).is_err());
    }

    #[test]
    fn oversized_record_is_rejected_before_reading_it() {
        let path = std::env::temp_dir().join(format!("ta-reborn-oversized-record-{}.tarep", std::process::id()));
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_RECORD_BYTES + 1).unwrap();
        drop(file);
        let result = Record::read(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.unwrap_err().to_string(), "record exceeds size limit");
    }
}
