//! Lockstep multiplayer over TCP.
//!
//! The host is the pacemaker. Every tick it broadcasts a bundle holding the commands it has
//! received since the previous bundle (its own included). Every peer, host included, steps
//! its own copy of the deterministic simulation with exactly those bundles in order, so all
//! copies stay identical; AI players run on every machine from the same state and need no
//! traffic. Peers report a state hash every [`HASH_INTERVAL`] ticks and the host announces
//! any mismatch as a desync.
//!
//! Commands never execute locally before the host has bundled them, so each order takes one
//! round trip to the host (plus up to a tick) to act — imperceptible on a LAN and similar to
//! the original game online. A peer sends what it ordered since it last read the network
//! together (an order to thousands of units, each sent somewhere of its own, is one message),
//! and a player coming back gets the game so far in parts of at most [`BATCH_BYTES`].
//!
//! Pause and game speed belong to the whole game: anyone may change them, the host applies
//! the change and announces it. The host also holds back new ticks while a peer has fallen
//! more than [`MAX_LEAD`] ticks behind (a slow machine, or one still loading), so everybody
//! plays at the pace of the slowest machine, as in the original.

pub mod lan;
mod transport;
mod validation;
pub use validation::command_memory;
use validation::message_memory;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;
use std::net::SocketAddr;

use serde::{Deserialize, Serialize};
use ta_sim::{Command, GameRules};

pub use transport::{Client, Host, HostEvent};

/// Bumped whenever the wire format changes, or the simulation or the AI (which every peer
/// runs) would play out differently: peers of different builds must not meet. (46: factory
/// ownership, transport recovery and AI cancelled-job fixes; 48: vertically launched missiles
/// gather speed as they climb; 49: pings, portraits, closed slots, seats and the lobby's title
/// and countdown; 50: a lobby's password, and why a host turns a player away; 51: who is away
/// from the battle, and for how much longer; 52: back to the lobby after a battle; 53: signals,
/// and computer allies that act on them; 54: computer allies calling for help and to attacks;
/// 55: allies start side by side, resigning leaves the army to an ally; 56: a returning player's
/// new peer number told to everyone, a dead connection replaced by its player; 57: the engine
/// and skirmish reviews' simulation and computer player fixes; 58: the target searches'
/// grid knows a unit captured mid-tick; 59: a queue's entry moved or dropped as the queue stands
/// when the command runs; 60: computer players that know only what they see; 61: swept
/// nuclear interception, mobile anti-nuke intelligence and complete missile-state hashes;
/// 62: construction/repair/resurrection fixes and construction-state hashes;
/// 63: aircraft flight, landing decks, transport lifecycle and air-state hashes;
/// 64: ground movement clearance, route endpoints, collision reach and movement-state hashes;
/// 65: naval sensors, physical waterline, torpedo flight, launch permissions and fleet duties;
/// 66: building yard states/reservations, activation callbacks, power and building-state hashes;
/// 67: skirmish AI scouting, weapon/repair compatibility, strike accounting and allied requests;
/// 68: scenario hold areas, hunter ownership, scheduled commanders and mission-state hashes;
/// 69: strategic missiles' cruise and dive, and vertical launches turning towards their target;
/// 70: aircraft land on level ground only, the nearest place clear of the others';
/// 71: a spin to nothing about an idle axis stops a piece's other spins (radar vehicles' dishes);
/// 72: units placed together kept apart, waves' aircraft arriving in flight, scenario layouts;
/// 73: VTOL aircraft brake at their brake rate and stop where they were sent (transports unload);
/// 74: strategic missiles fired at a unit come down where it stood at their launch;
/// 75: no unit picks, answers or chases its bad targets of its own accord (TA's targeting);
/// 76: `NoAutoFire=1` units set to hold fire on construction (the Doomsday Machine among them);
/// 77: a mission script's `o` is its move order, then its fire order (was read the other way);
/// 78: a weapon whose bad targets are every unit it could hit takes on what it can (mods' flak);
/// 79: a mission's units placed for the player keep their file's fire orders (never built);
/// 80: a scenario wave's bombers go for what their side sees, and bomb it;
/// 81: a unit runs its own script before its model's (mods' units borrowing another's model);
/// 82: a unit is finished once: work on one being taken apart only mends it (`Unit::finished`);
/// 83: the data hash leaves the maps out, the lobby checks the battle's map (`Msg::Maps`,
/// `Lobby::map_status`), and a refusal for other data carries the host's mods (`Refusal::Data`),
/// as one for coming back without the battle's map does (`Refusal::Map`);
/// 84: a weapon's bad targets are lifted when they are every unit a player can field it could hit;
/// 85: bad targets taken on last, `Command::SetShootAll` (`+shootall`), abandoned construction
/// un-builds, reclaim by metal, planes circle over the sea and fan out on attack runs; 86: bombers
/// on patrol or attack-move bomb what they come upon; 87: off a new site clear of the builder's
/// next ones, out of buildings; 88: ships built at the surface, construction aircraft drift;
/// 89: hovering aircraft circle over the sea too; 90: damaged aircraft on a mission go to a free
/// repair deck, pads switched off unused, pads mend only the aircraft on their decks; 91:
/// scripts' questions about units by id answered, as TA 3.1 did; 92: mods' buildings make and
/// take on their own upgrade modules, EnergyUse only while switched on, mines start cloaked.)
/// [`Msg::Hello`] and [`Refusal::Version`] keep their layout, so builds of other protocols are
/// still told why they cannot meet.
pub const PROTOCOL: u32 = 92;
pub const DEFAULT_PORT: u16 = 7890;
/// Ticks between state hash reports (which double as progress reports).
pub const HASH_INTERVAL: u64 = 15;
/// How far (ticks) the host runs ahead of the slowest peer's last report before it waits.
pub const MAX_LEAD: u64 = 75;
/// Game speed limits, percent of normal.
pub const SPEED_MIN: u16 = 25;
pub const SPEED_MAX: u16 = 1000;
/// Most bytes of commands one message carries (a peer's orders, a returning player's
/// history): more goes in several, well within the transport's frames.
pub const BATCH_BYTES: u64 = 1 << 20;
const MAX_HISTORY_BYTES: usize = 128 << 20;
/// Private chat envelopes; the host replaces the claimed player and routes by team.
pub const PING_PREFIX: &str = "\u{E000}ping ";
pub const ALLY_PREFIX: &str = "\u{E001}ally ";
/// How long a client in a battle hears nothing from its host (whose pings come every
/// [`PING_INTERVAL`]) before it takes the connection as lost.
pub const HOST_SILENT: std::time::Duration = std::time::Duration::from_secs(45);
/// How often the host measures every peer's round trip ([`Msg::Ping`]) and tells everyone
/// the latest ([`Msg::Pings`]).
pub const PING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
/// A player's picture on the wire: a PNG this many pixels square, of at most
/// [`PORTRAIT_BYTES`].
pub const PORTRAIT_SIZE: u32 = 64;
pub const PORTRAIT_BYTES: usize = 32 * 1024;
/// The most maps a player tells the host it has ([`Msg::Maps`]), and the longest name of one.
pub const MAX_MAPS: usize = 16_384;
pub const MAP_NAME_MAX: usize = 255;
/// The most mods a refusal names ([`Refusal::Data`]), and the longest id, name or version of one.
pub const MAX_MODS: usize = 256;
pub const MOD_TEXT_MAX: usize = 255;

/// A mod the host plays with, as a refusal for other data tells it ([`Refusal::Data`]): the
/// joining player's game compares them with its own to say which differ.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetMod {
    /// Its id (its folder's name in lower case, as load orders have it).
    pub id: String,
    pub name: String,
    pub version: String,
    /// The digest of its files as records keep it (all zero: not known).
    pub digest: [u8; 16],
    /// It gives the game something besides maps (a map pack of maps alone does not: it cannot be
    /// why two computers' data differ).
    pub data: bool,
}

impl NetMod {
    /// The same, its words cleaned for the screen (no control characters, not too long).
    pub fn cleaned(&self) -> NetMod {
        NetMod { id: clean(&self.id, MOD_TEXT_MAX), name: clean(&self.name, MOD_TEXT_MAX), version: clean(&self.version, MOD_TEXT_MAX), ..self.clone() }
    }
}

/// Whether a person in the lobby has the battle's map, as the host finds it ([`Lobby::map_status`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MapStatus {
    /// They have not told the host their maps yet (just joined).
    Checking,
    /// The host's map, the same contents.
    Ready,
    /// No map of that name.
    Missing,
    /// A map of that name, with other contents (another version of it).
    Other,
}

impl MapStatus {
    /// What a person's map (`theirs`: their digest of it, `None` without one) is against the
    /// host's (`ours`); `None` for `theirs` itself when they have not told their maps yet.
    pub fn of(ours: Option<u64>, theirs: Option<Option<u64>>) -> MapStatus {
        match (ours, theirs) {
            (_, None) => MapStatus::Checking,
            (Some(a), Some(Some(b))) if a == b => MapStatus::Ready,
            (_, Some(None)) | (None, _) => MapStatus::Missing,
            (Some(_), Some(Some(_))) => MapStatus::Other,
        }
    }
}

/// A secret issued by one host to one player, used only to reclaim that player's slot.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconnectToken([u8; 32]);

impl std::fmt::Debug for ReconnectToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("ReconnectToken([redacted])") }
}

impl ReconnectToken {
    fn generate() -> io::Result<Self> {
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
        Ok(Self(bytes))
    }
}

/// Everything sent over the wire.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Msg {
    /// Peer → host, first message: who we are, what game data we run (the hash of everything but
    /// the maps: `ta_data::Compatibility::network`), and the lobby's password as we were given it
    /// (empty: none). (Its layout never changes: a host of another protocol reads it to say so.)
    Hello { name: String, protocol: u32, data_hash: u64, reconnect: Option<ReconnectToken>, password: String },
    /// Host → peer: accepted, with the peer id assigned.
    Welcome { peer: u8, reconnect: ReconnectToken },
    Reject { reason: Refusal },
    /// Host → all: the current lobby.
    Lobby(Lobby),
    /// Peer → host: changes to the peer's own slot.
    SetSlot { side: String, color: u8, ready: bool },
    /// Peer → host: watch the game instead of playing (true), or ask for a slot again.
    SetSpectator(bool),
    Chat { from: String, text: String },
    /// Host → all: the game begins.
    Start(StartInfo),
    /// Peer → host: local commands to include in the next bundle (everything ordered since
    /// the peer last read the network, in order; several messages when it is a lot).
    Commands(Vec<Command>),
    /// Host → all: the commands every peer executes at `tick`.
    Tick { tick: u64, commands: Vec<Command> },
    /// Peer → host: state hash after `tick`.
    Hash { tick: u64, hash: u64 },
    /// Host → all: hashes differed at `tick`.
    Desync { tick: u64 },
    /// Peer → host: pause or resume, and/or a new game speed (percent of normal).
    SetPace { paused: Option<bool>, speed: Option<u16> },
    /// Host → all: the game's pause state and speed, and who changed them.
    Pace { paused: bool, speed: u16, by: String },
    /// Host → all: the players everyone is waiting for (empty once they caught up).
    Waiting(Vec<String>),
    /// Host → a player coming back to the game: how it began, which player they are, and
    /// every tick's commands so far (`upto` is the last tick bundled; ticks missing from
    /// `history` had none), to catch up by simulating; then the ticks follow as usual. A long
    /// history comes ahead of it in [`Msg::History`] parts, which [`Client::poll`] puts back
    /// in front of this message's own (its last part).
    Resume { start: StartInfo, player: u8, upto: u64, history: Vec<(u64, Vec<Command>)>, paused: bool, speed: u16 },
    Bye,
    /// Host → a player coming back: a part of the game's history, in order, ahead of
    /// [`Msg::Resume`]. (Last: older messages keep their numbers.)
    History(Vec<(u64, Vec<Command>)>),
    /// Host → all: the match continues, but its bounded reconnect history is exhausted.
    ReconnectUnavailable,
    /// Host → peer, every [`PING_INTERVAL`]: answered at once with [`Msg::Pong`] (the
    /// transport does both; its round trip is the peer's ping).
    Ping(u32),
    /// Peer → host: the [`Msg::Ping`] answered.
    Pong(u32),
    /// Host → all: everyone's round trip to the host, milliseconds (the host's own 0).
    Pings(Vec<(u8, u16)>),
    /// Host → a peer: the host took them out of the lobby.
    Removed,
    /// Peer → host: move to this open seat (the slot's index).
    TakeSlot(u8),
    /// Peer → host: our picture ([`PORTRAIT_SIZE`] PNG; empty: none).
    Portrait(Vec<u8>),
    /// Host → all: the people's pictures (peer, PNG), those who have one.
    Portraits(Vec<(u8, Vec<u8>)>),
    /// Host → all: the players who left the battle and may still come back, with the seconds
    /// before an ally takes over their army (told once a second while anyone is away).
    Away(Vec<(u8, u16)>),
    /// Host → all, after a battle: the host is back in the lobby, as it now stands; the battle
    /// is over for everyone (a peer may follow, or leave).
    Rematch(Lobby),
    /// Host → all: a player who came back to the battle on a new connection: their old peer
    /// number, and their new one.
    Returned { old: u8, new: u8 },
    /// Peer → host, once welcomed: the maps we have, each by its file stem (lower case) with the
    /// digest of its contents (`ta_data::Compatibility::map_digests`; at most [`MAX_MAPS`]). The
    /// host keeps it for the connection, to tell who has the battle's map ([`Lobby::map_status`]).
    Maps(Vec<(String, u64)>),
}

/// Why a host turned a player away (told in the player's own language).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    /// Different builds: the host's protocol and ours. (Its layout never changes.)
    Version { host: u32, yours: u32 },
    /// Different game data or mods: the host's mods in its load order (at most [`MAX_MODS`]),
    /// for the player to be told which differ from theirs.
    Data { mods: Vec<NetMod> },
    /// The battle has begun: only its own players get back in.
    Started,
    /// Coming back to the battle without the credential the host gave.
    Credential,
    /// No name, or one already taken (the host's own among them).
    Name,
    /// The lobby has a password, and this was not it.
    Password,
    /// Coming back to the battle without its map, or (`other`) with another version of it than
    /// the host's: `map` names it as the host's copy does ([`Session::with_map`]).
    Map { map: String, other: bool },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Lobby {
    pub map: String,
    pub slots: Vec<LobbySlot>,
    pub rules: GameRules,
    /// People who watch instead of playing: (peer id, name).
    pub spectators: Vec<(u8, String)>,
    /// The battle's name, as the host gave it (shown on the network's list).
    pub title: String,
    /// Seconds until the battle starts, counting down once the host has started it; none
    /// while waiting (any change to the players calls it off).
    pub countdown: Option<u8>,
    /// Whether each person in the lobby but the host (players and spectators, by peer) has the
    /// battle's map, as the host finds it from their [`Msg::Maps`]: the battle starts only once
    /// everyone has it.
    pub map_status: Vec<(u8, MapStatus)>,
    /// The battle's map's name as the host's copy gives it (`map` is its file stem): what those
    /// without a copy of it are told.
    pub map_title: String,
}

impl Lobby {
    /// What the host found of `peer`'s copy of the battle's map (the host's own: ready; someone
    /// it has not heard about: still checking).
    pub fn map_status_of(&self, peer: u8) -> MapStatus {
        if peer == 0 {
            return MapStatus::Ready;
        }
        self.map_status.iter().find(|(p, _)| *p == peer).map_or(MapStatus::Checking, |(_, s)| *s)
    }

    /// Everyone in the lobby has the battle's map (the host's own copy of it).
    pub fn everyone_has_the_map(&self) -> bool {
        let mut people = self.slots.iter().filter_map(|s| match &s.kind {
            SlotKind::Human { peer, .. } => Some(*peer),
            _ => None,
        }).chain(self.spectators.iter().map(|(peer, _)| *peer));
        people.all(|peer| self.map_status_of(peer) == MapStatus::Ready)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LobbySlot {
    pub kind: SlotKind,
    pub side: String,
    pub color: u8,
    pub ready: bool,
    /// Alliance, chosen by the host.
    pub team: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlotKind {
    Open,
    /// A person, identified by peer id (0 = the host).
    Human { peer: u8, name: String },
    /// A computer player: 0 easy, 1 normal, 2 hard.
    Ai { difficulty: u8 },
    /// Nobody plays here and nobody may sit here (the host closed it).
    Closed,
}

impl SlotKind {
    /// Whether the slot is a player in the battle (a person or a computer).
    pub fn plays(&self) -> bool {
        matches!(self, SlotKind::Human { .. } | SlotKind::Ai { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StartInfo {
    pub map: String,
    pub seed: u64,
    pub slots: Vec<LobbySlot>,
    pub rules: GameRules,
    /// People watching: (peer id, name). Every connected peer without a player slot
    /// watches, spectators or not.
    pub spectators: Vec<(u8, String)>,
}

impl StartInfo {
    /// The player index (slot among the non-open ones) a peer controls.
    pub fn player_of(&self, peer: u8) -> Option<u8> {
        self.slots
            .iter()
            .filter(|s| s.kind.plays())
            .position(|s| matches!(&s.kind, SlotKind::Human { peer: p, .. } if *p == peer))
            .map(|i| i as u8)
    }
}

/// Which end of the connection we are.
pub enum Role {
    Host(Host),
    Client(Client),
}

impl Role {
    /// Says goodbye (the host to everyone) and waits, at most `timeout`, for it to be
    /// written: the program is closing.
    pub fn goodbye(&mut self, timeout: std::time::Duration) {
        match self {
            Role::Host(h) => {
                h.broadcast(&Msg::Bye);
                h.drain(timeout);
            }
            Role::Client(c) => {
                c.send(&Msg::Bye);
                c.drain(timeout);
            }
        }
    }
}

/// A returning peer's position in the host's append-only command history. Keep only the
/// next chunk while blocked, rather than cloning the complete history for every rejoin.
struct ResumeProgress {
    next_history: usize,
    pending: Option<(usize, Msg)>,
}

/// What the session tells the players, for the game to word in their language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// Who paused the game, or resumed it.
    Paused(String),
    Resumed(String),
    /// Who set the game's speed, and to what (percent of normal).
    Speed(String, u16),
    /// A player came back to the battle.
    Back(String),
    /// The match goes on, but nobody can come back to it any more: its history for players
    /// coming back outgrew its memory.
    ReconnectLimit,
}

/// Everyone in a battle by peer: the player they command (none: they watch) and their name.
fn roster_of(start: &StartInfo) -> BTreeMap<u8, (Option<u8>, String)> {
    let mut roster: BTreeMap<u8, (Option<u8>, String)> = start.spectators.iter().map(|(peer, name)| (*peer, (None, clean(name, NAME_MAX)))).collect();
    for (player, slot) in start.slots.iter().filter(|s| s.kind.plays()).enumerate() {
        if let SlotKind::Human { peer, name } = &slot.kind {
            roster.insert(*peer, (Some(player as u8), clean(name, NAME_MAX)));
        }
    }
    roster
}

/// A peer number changed in a lobby's or a start's seats (a player back on a new connection).
fn renumber(slots: &mut [LobbySlot], spectators: &mut [(u8, String)], old: u8, new: u8) {
    for slot in slots {
        if let SlotKind::Human { peer, .. } = &mut slot.kind
            && *peer == old
        {
            *peer = new;
        }
    }
    for (peer, _) in spectators {
        if *peer == old {
            *peer = new;
        }
    }
}

/// The host's name, from the roster ("host" before it is known).
fn host_name(roster: &BTreeMap<u8, (Option<u8>, String)>) -> String {
    roster.get(&0).map_or_else(|| "host".to_string(), |(_, name)| name.clone())
}

/// An in-game lockstep session.
pub struct Session {
    pub role: Role,
    /// Our player index in the simulation.
    pub local_player: u8,
    /// Host: peer id → player index.
    peer_players: BTreeMap<u8, u8>,
    /// Host: commands collected for the next bundle.
    collected: Vec<Command>,
    pending_bytes: usize,
    pending_memory: usize,
    peer_pending: BTreeMap<u8, CommandBudget>,
    /// Client: commands submitted since the network was last read, sent together.
    outgoing: Vec<Command>,
    /// Bundles ready to execute, in tick order.
    ready: VecDeque<(u64, Vec<Command>)>,
    ready_bytes: usize,
    resume_next: u64,
    resume_upto: u64,
    resume_history: VecDeque<(u64, Vec<Command>)>,
    /// Host: next tick to bundle. Client: next live tick expected from the host.
    next_tick: u64,
    /// Host: our own hashes, to compare with the peers' reports.
    hashes: BTreeMap<u64, u64>,
    /// Reports that arrived before the host recorded its own hash, bounded by report ticks.
    peer_hashes: BTreeMap<u64, BTreeMap<u8, u64>>,
    /// First tick at which a desync was detected.
    pub desync: Option<u64>,
    /// Peers that left mid-game (names).
    pub departed: Vec<String>,
    /// Client: the host is gone, so no more ticks will come.
    pub host_lost: bool,
    /// Chat lines received (sender, text), oldest first; the game drains them.
    pub chat: Vec<(String, String)>,
    /// The game is paused (for everyone).
    pub paused: bool,
    /// Game speed, percent of normal.
    pub speed: u16,
    /// Players the game is waiting for, because they fell behind.
    pub waiting_for: Vec<String>,
    /// Who paused the game (while it is).
    pub paused_by: Option<String>,
    /// Players who left and may still come back: player, seconds before an ally takes over
    /// their army (the host's count, told to everyone).
    pub away: Vec<(u8, u16)>,
    /// Announcements ("Ann paused the game"); the game drains them.
    pub notices: Vec<Notice>,
    /// Everyone in the battle by peer: their player and name (from how it began).
    roster: BTreeMap<u8, (Option<u8>, String)>,
    /// Host: when `away` was last told.
    away_told: Option<std::time::Instant>,
    /// The lobby the battle came from (the host's, as it began), and, once the host has gone
    /// back to it, the lobby as it now stands (a client's, kept up to date).
    lobby: Option<Lobby>,
    /// Client: the host went back to the lobby (the battle is over).
    pub rematch: bool,
    /// Client: the next battle, begun while we were still in this one's result (the host went
    /// on from the lobby); the connection waits for it, its ticks unread.
    pub next_battle: Option<StartInfo>,
    /// Client: the address we joined, to come back to.
    pub rejoin_address: Option<String>,
    /// Client: the host went on without us (dropped for falling behind), or our connection to
    /// it failed (no word from it). Either way no more ticks come (`host_lost`); unlike the
    /// host leaving, we may come back.
    pub dropped: bool,
    pub link_failed: bool,
    /// Client: the battle's ticks have begun coming (the silent-host watch is on).
    ticking: bool,
    /// Host: the last tick each peer reported having simulated.
    peer_ticks: BTreeMap<u8, u64>,
    /// Host: names by peer id.
    peer_names: BTreeMap<u8, String>,
    /// Since when the game has been waiting for someone (wall clock; the network layer
    /// only, never the simulation).
    waiting_since: Option<std::time::Instant>,
    /// Host: how the game began, and every tick that had commands (for players who come
    /// back).
    start: Option<StartInfo>,
    history: Vec<(u64, Vec<Command>)>,
    history_bytes: usize,
    reconnect_available: bool,
    resuming: BTreeMap<u8, ResumeProgress>,
    /// Host: players who left, whose army waits for them: name, player, since when. After
    /// `abandon_after` it goes to an ally (`Command::Abandon`).
    absent: Vec<(String, u8, std::time::Instant)>,
    pub abandon_after: std::time::Duration,
    /// Host: the battle's map as the host has it (its name, and the digest of its contents): a
    /// player coming back must have the same ([`Session::with_map`]).
    map_check: Option<(String, u64)>,
    /// Host: players back on a new connection, let in once their copy of the battle's map is
    /// found to be the host's (they tell their maps just after the welcome): peer, name, since.
    arriving: Vec<(u8, String, std::time::Instant)>,
    /// How long they have to tell their maps ([`ARRIVAL_WAIT`]).
    pub arrival_wait: std::time::Duration,
    /// Host: connections turned away for want of the battle's map (their going is nobody's news:
    /// they never came back).
    refused: BTreeSet<u8>,
}

/// How long a player who left has to come back before an ally takes over their army.
pub const ABANDON_AFTER: std::time::Duration = std::time::Duration::from_secs(60);
/// How long a player coming back has to tell the host their maps ([`Msg::Maps`]) before they are
/// taken to have no copy of the battle's.
pub const ARRIVAL_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Longest chat line and sender name accepted.
const CHAT_MAX: usize = 200;
const NAME_MAX: usize = 32;
const MAX_PEER_COMMAND_BYTES: usize = 1 << 20;
const MAX_PEER_COMMANDS: usize = 20_000;
// Decoded commands can occupy much more space than their wire representation.
// Leave room for the tick envelope, vector capacity and serialized bytes at the reader.
const MAX_TICK_COMMAND_MEMORY: usize = transport::MAX_MESSAGE_MEMORY / 4;
const MAX_READY_TICKS: usize = 8192;
const MAX_READY_BYTES: usize = 128 << 20;

#[derive(Default)]
struct CommandBudget {
    bytes: usize,
    commands: usize,
    resigned: bool,
}

fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

/// Keep accepted orders within both the sender's allowance and every receiver's
/// memory budget. Container headers are not orders and do not consume the sender's quota.
fn collect_commands(player: u8, commands: impl IntoIterator<Item = Command>, collected: &mut Vec<Command>,
    pending_bytes: &mut usize, pending_memory: &mut usize, pending: &mut CommandBudget) {
    for command in commands {
        if command_player(&command) != player || !validation::player_command(&command) { continue; }
        let resign = matches!(command, Command::Resign { .. });
        if resign && pending.resigned { continue; }
        let bytes = bincode::serialized_size(&command).unwrap_or(u64::MAX) as usize;
        let memory = command_memory(&command);
        if pending.bytes.saturating_add(bytes) > MAX_PEER_COMMAND_BYTES || pending.commands >= MAX_PEER_COMMANDS
            || pending_bytes.saturating_add(bytes) > transport::MAX_FRAME - 128
            || pending_memory.saturating_add(memory) > MAX_TICK_COMMAND_MEMORY {
            continue;
        }
        if resign && collected.iter().any(|c| matches!(c, Command::Resign { player: p } if *p == player)) {
            pending.resigned = true;
            continue;
        }
        pending.bytes += bytes;
        pending.commands += 1;
        *pending_bytes += bytes;
        *pending_memory += memory;
        pending.resigned |= resign;
        collected.push(command);
    }
}

impl Session {
    pub fn host(mut host: Host, start: &StartInfo) -> Session {
        host.accepting = false;
        if let Some(name) = start.slots.iter().find_map(|s| match &s.kind {
            SlotKind::Human { peer: 0, name } => Some(name),
            _ => None,
        }).or_else(|| start.spectators.iter().find(|(peer, _)| *peer == 0).map(|(_, name)| name)) {
            host.set_host_name(name);
        }
        let humans = || {
            start.slots.iter().filter(|s| s.kind.plays()).enumerate().filter_map(|(i, s)| match &s.kind {
                SlotKind::Human { peer, name } => Some((*peer, i as u8, name)),
                _ => None,
            })
        };
        let peer_players = humans().map(|(peer, player, _)| (peer, player)).collect();
        let mut session = Session::new(Role::Host(host), start.player_of(0).unwrap_or(0), peer_players);
        for (peer, _, name) in humans().filter(|(peer, ..)| *peer != 0) {
            session.peer_ticks.insert(peer, 0);
            session.peer_names.insert(peer, clean(name, NAME_MAX));
        }
        session.start = Some(start.clone());
        session.roster = roster_of(start);
        if let Role::Host(host) = &mut session.role {
            host.roster_peers = session.roster.keys().copied().collect();
        }
        session
    }

    /// A client's session told who is in the battle (names and players, for its pings, who
    /// is away and the host's name), from how it began.
    pub fn with_roster(mut self, start: &StartInfo) -> Session {
        self.roster = roster_of(start);
        self
    }

    /// Client: the address to come back to, should the connection fail.
    pub fn with_rejoin(mut self, address: Option<String>) -> Session {
        self.rejoin_address = address;
        self
    }

    /// The lobby the battle came from (the host keeps it, to go back to after the battle).
    pub fn with_lobby(mut self, lobby: &Lobby) -> Session {
        self.lobby = Some(lobby.clone());
        self
    }

    /// Host: the battle's map as the host has it (`title`: its name, `digest`: the digest of its
    /// contents, `ta_data::Compatibility::map_digest`). A player coming back to the battle is let
    /// in only with the same copy of it (they tell their maps just after the welcome): without
    /// one, or with another version of it, they are turned away ([`Refusal::Map`]), their army
    /// waiting on, the battle undisturbed. (None: no check, as for a battle on data that has no
    /// maps of its own.)
    pub fn with_map(mut self, title: &str, digest: Option<u64>) -> Session {
        self.map_check = digest.map(|digest| (clean(title, MAP_NAME_MAX), digest));
        self
    }

    /// The lobby the battle came from, or (a client's, once the host has gone back) the lobby
    /// as it now stands.
    pub fn lobby(&self) -> Option<&Lobby> {
        self.lobby.as_ref()
    }

    /// Host, after the battle: the host taking newcomers again, with the lobby the battle
    /// came from. (None for a client.) Those who left the battle come back as newcomers;
    /// the credentials of those still here stay theirs, for the next battle.
    pub fn into_host(self) -> Option<(Host, Option<Lobby>)> {
        let Role::Host(mut host) = self.role else { return None };
        // (Those still on their way back came back to a battle that is over: let go.)
        for (peer, ..) in &self.arriving {
            host.send_to(*peer, &Msg::Reject { reason: Refusal::Started });
            host.kick(*peer);
        }
        host.accepting = true;
        host.rejoin.clear();
        host.roster_peers.clear();
        Some((host, self.lobby))
    }

    /// Client: the connection and how the next battle begins, once it has (see
    /// `next_battle`).
    pub fn into_next_battle(self) -> Option<(Client, StartInfo)> {
        let Role::Client(client) = self.role else { return None };
        Some((client, self.next_battle?))
    }

    /// Client, once the host has gone back to the lobby: the connection, and the lobby as it
    /// now stands. (None for the host, or before the host has gone back.)
    pub fn into_client(self) -> Option<(Client, Lobby)> {
        let Role::Client(mut client) = self.role else { return None };
        if !self.rematch {
            return None;
        }
        if let Some(start) = self.next_battle { client.defer_start(start); }
        Some((client, self.lobby?))
    }

    /// The host's name (from the roster; "host" before it is known).
    pub fn host_name(&self) -> String {
        host_name(&self.roster)
    }

    /// The host's player (none: the host watches).
    pub fn host_player(&self) -> Option<u8> {
        self.roster.get(&0).and_then(|(player, _)| *player)
    }

    /// The name of the person commanding `player` (none: a computer, or nobody).
    pub fn player_name(&self, player: u8) -> Option<&str> {
        self.roster.values().find(|(p, _)| *p == Some(player)).map(|(_, name)| name.as_str())
    }

    /// Everyone's round trip to the host, milliseconds, by player, as last measured (the
    /// host's own 0; spectators left out).
    pub fn player_pings(&self) -> Vec<(u8, u16)> {
        let table = match &self.role {
            Role::Host(h) => h.pings(),
            Role::Client(c) => c.pings().to_vec(),
        };
        table.into_iter().filter_map(|(peer, ms)| Some((self.roster.get(&peer)?.0?, ms))).collect()
    }

    pub fn client(mut client: Client, local_player: u8) -> Session {
        client.enter_game();
        Session::new(Role::Client(client), local_player, BTreeMap::new())
    }

    /// A player coming back to a game in progress (after the host's `Msg::Resume`): every
    /// tick up to `upto` is ready at once. Empty ticks are represented lazily while the game
    /// catches up, so a sparse history does not allocate one object per elapsed tick.
    pub fn resumed(client: Client, local_player: u8, upto: u64, history: Vec<(u64, Vec<Command>)>, paused: bool, speed: u16) -> Session {
        let mut session = Session::client(client, local_player);
        if upto == u64::MAX || !validation::valid_history(&history, upto, 0) {
            session.host_lost = true;
            return session;
        }
        session.resume_history = history.into();
        session.resume_next = 1;
        session.resume_upto = upto;
        session.next_tick = upto + 1;
        (session.paused, session.speed) = (paused, speed.clamp(SPEED_MIN, SPEED_MAX));
        session
    }

    fn new(role: Role, local_player: u8, peer_players: BTreeMap<u8, u8>) -> Session {
        Session {
            role,
            local_player,
            peer_players,
            collected: Vec::new(),
            pending_bytes: 0,
            pending_memory: 0,
            peer_pending: BTreeMap::new(),
            outgoing: Vec::new(),
            ready: VecDeque::new(),
            ready_bytes: 0,
            resume_next: 1,
            resume_upto: 0,
            resume_history: VecDeque::new(),
            next_tick: 1,
            hashes: BTreeMap::new(),
            peer_hashes: BTreeMap::new(),
            desync: None,
            departed: Vec::new(),
            host_lost: false,
            chat: Vec::new(),
            paused: false,
            speed: 100,
            waiting_for: Vec::new(),
            paused_by: None,
            away: Vec::new(),
            notices: Vec::new(),
            roster: BTreeMap::new(),
            away_told: None,
            lobby: None,
            rematch: false,
            next_battle: None,
            rejoin_address: None,
            dropped: false,
            link_failed: false,
            ticking: false,
            peer_ticks: BTreeMap::new(),
            peer_names: BTreeMap::new(),
            waiting_since: None,
            start: None,
            history: Vec::new(),
            history_bytes: 0,
            reconnect_available: true,
            resuming: BTreeMap::new(),
            absent: Vec::new(),
            abandon_after: ABANDON_AFTER,
            map_check: None,
            arriving: Vec::new(),
            arrival_wait: ARRIVAL_WAIT,
            refused: BTreeSet::new(),
        }
    }

    /// Pauses or resumes the game and/or changes its speed, for everyone. The host applies
    /// it at once; a client asks the host, and sees the change when the host announces it.
    pub fn set_pace(&mut self, paused: Option<bool>, speed: Option<u16>, name: &str) {
        match &mut self.role {
            Role::Host(_) => self.apply_pace(paused, speed, &clean(name, NAME_MAX)),
            Role::Client(c) => c.send(&Msg::SetPace { paused, speed }),
        }
    }

    /// Host: applies a pace change and tells everyone.
    fn apply_pace(&mut self, paused: Option<bool>, speed: Option<u16>, by: &str) {
        let Role::Host(host) = &mut self.role else { return };
        let (was_paused, was_speed) = (self.paused, self.speed);
        self.paused = paused.unwrap_or(self.paused);
        self.speed = speed.map_or(self.speed, |s| s.clamp(SPEED_MIN, SPEED_MAX));
        if (self.paused, self.speed) != (was_paused, was_speed) {
            host.broadcast(&Msg::Pace { paused: self.paused, speed: self.speed, by: by.to_string() });
            announce_pace(&mut self.notices, &mut self.paused_by, (was_paused, was_speed), (self.paused, self.speed), by);
        }
    }

    /// Host: those back on a new connection, in the order they came. A player coming back to
    /// their army is let in once their copy of the battle's map is found to be the host's (at
    /// once where the battle checks none), and turned away with none or another version of it, or
    /// with no word of their maps after [`ARRIVAL_WAIT`] (they would fail to load it, or play
    /// another battle): their army waits on, nobody is told they came, the battle goes on. Anyone
    /// else is let go by `come_back`, map or not.
    fn arrivals(&mut self) {
        enum Arrival {
            Waiting,
            In,
            Refused(Refusal),
        }
        for (peer, name, since) in std::mem::take(&mut self.arriving) {
            let Role::Host(host) = &mut self.role else { return };
            let returning = self.reconnect_available && self.absent.iter().any(|(n, ..)| n.eq_ignore_ascii_case(&name));
            let arrival = match (&self.map_check, &self.start) {
                (Some((title, digest)), Some(start)) if returning => match host.peer_map(peer, &start.map) {
                    Some(Some(theirs)) if theirs == *digest => Arrival::In,
                    Some(theirs) => Arrival::Refused(Refusal::Map { map: title.clone(), other: theirs.is_some() }),
                    None if since.elapsed() >= self.arrival_wait => Arrival::Refused(Refusal::Map { map: title.clone(), other: false }),
                    None => Arrival::Waiting,
                },
                _ => Arrival::In,
            };
            match arrival {
                Arrival::Waiting => self.arriving.push((peer, name, since)),
                Arrival::In => self.come_back(peer, name),
                Arrival::Refused(reason) => {
                    log::info!("{name} came back without the battle's map as the host has it ({reason:?}): turned away");
                    host.send_to(peer, &Msg::Reject { reason });
                    host.kick(peer);
                    self.refused.insert(peer);
                }
            }
        }
    }

    /// Host: someone back on a new connection, let in: a player coming back to their army
    /// catches up from the battle's start with every tick's commands (`resuming`). Anyone else (a
    /// spectator on a new connection), or anyone once rejoining has closed (no history to catch
    /// up with), is told so and let go.
    fn come_back(&mut self, peer: u8, name: String) {
        let Role::Host(host) = &mut self.role else { return };
        let away = self.absent.iter().position(|(n, ..)| n.eq_ignore_ascii_case(&name));
        let Some(i) = away.filter(|_| self.reconnect_available) else {
            if self.start.is_some() {
                host.send_to(peer, &Msg::Reject { reason: Refusal::Started });
                host.kick(peer);
            }
            return;
        };
        let (_, player, _) = self.absent.remove(i);
        host.rejoin.retain(|n| !n.eq_ignore_ascii_case(&name));
        self.peer_players.insert(peer, player);
        self.peer_names.insert(peer, clean(&name, NAME_MAX));
        self.peer_ticks.insert(peer, 0);
        if self.start.is_some() {
            self.resuming.insert(peer, ResumeProgress { next_history: 0, pending: None });
        }
        // Their old peer number gives way to the new one: in the roster, the lobby to go back
        // to, the start sent to whoever comes back (them included), and everyone else's rosters.
        if let Some(old) = self.roster.iter().find(|(p, (_, n))| **p != peer && n.eq_ignore_ascii_case(&name)).map(|(p, _)| *p) {
            if let Some(entry) = self.roster.remove(&old) {
                self.roster.insert(peer, entry);
            }
            if let Some(lobby) = self.lobby.as_mut() {
                renumber(&mut lobby.slots, &mut lobby.spectators, old, peer);
                // (Their map as it was found: the host's lobby works it out again from the new
                // connection's word.)
                lobby.map_status.retain(|(p, _)| *p != old && *p != peer);
            }
            if let Some(start) = self.start.as_mut() {
                renumber(&mut start.slots, &mut start.spectators, old, peer);
            }
            host.broadcast_where(&Msg::Returned { old, new: peer }, |p| p != peer);
        }
        host.roster_peers = self.roster.keys().copied().collect();
        self.notices.push(Notice::Back(clean(&name, NAME_MAX)));
    }

    /// Host: whether the next tick may be produced — not while paused, nor while a peer
    /// is too far behind (then `waiting_for` names them, for every player to see).
    pub fn can_produce(&mut self) -> bool {
        let produced = self.next_tick - 1;
        let behind: Vec<String> = self
            .peer_ticks
            .iter()
            .filter(|&(_, &t)| produced.saturating_sub(t) > MAX_LEAD)
            .map(|(p, _)| self.peer_names.get(p).cloned().unwrap_or_else(|| format!("player {p}")))
            .collect();
        if behind != self.waiting_for {
            if let Role::Host(host) = &mut self.role {
                host.broadcast(&Msg::Waiting(behind.clone()));
            }
            self.waiting_for = behind;
        }
        !self.paused && self.waiting_for.is_empty()
    }

    /// How long the game has been waiting for slow players (zero when it is not).
    pub fn waiting_secs(&mut self) -> f32 {
        if self.waiting_for.is_empty() {
            self.waiting_since = None;
            return 0.0;
        }
        self.waiting_since.get_or_insert_with(std::time::Instant::now).elapsed().as_secs_f32()
    }

    /// Host: drops the players the game is waiting for (their connections close; their units
    /// stay, idle).
    pub fn drop_waiting(&mut self) {
        let Role::Host(host) = &mut self.role else { return };
        for (&peer, name) in &self.peer_names {
            if self.waiting_for.contains(name) && self.peer_ticks.contains_key(&peer) {
                log::info!("dropping {name}, who fell behind");
                // (Told so first: not "the host left".)
                host.send_to(peer, &Msg::Removed);
                host.kick(peer);
            }
        }
    }

    /// Sends a chat line to everyone. The host relays it, so clients see their own lines
    /// when the relay comes back.
    pub fn send_chat(&mut self, from: &str, text: &str) {
        let (from, text) = (clean(from, NAME_MAX), clean(text.trim(), CHAT_MAX));
        if text.is_empty() {
            return;
        }
        match &mut self.role {
            Role::Host(h) => {
                let source = self.peer_players.get(&0).copied();
                let Some((text, private)) = validation::authenticate_chat(&text, source) else { return };
                if validation::chat_visible(self.start.as_ref(), source, source, private) {
                    self.chat.push((from.clone(), text.clone()));
                }
                h.broadcast_where(&Msg::Chat { from, text }, |peer| validation::chat_visible(self.start.as_ref(), source, self.peer_players.get(&peer).copied(), private));
            }
            Role::Client(c) => c.send(&Msg::Chat { from, text }),
        }
    }

    pub fn is_host(&self) -> bool {
        matches!(self.role, Role::Host(_))
    }

    /// Queues a local command; it runs once the host has bundled it. Spectators command
    /// nobody (the host drops what they send; a watching host drops its own). A client sends
    /// what it queued on the next [`Session::flush`] or [`Session::poll`].
    pub fn submit(&mut self, command: Command) {
        match &mut self.role {
            Role::Host(_) => {
                if let Some(&player) = self.peer_players.get(&0) {
                    collect_commands(player, [command], &mut self.collected, &mut self.pending_bytes,
                        &mut self.pending_memory, self.peer_pending.entry(0).or_default());
                }
            }
            Role::Client(_) => self.outgoing.push(command),
        }
    }

    /// Client: sends the commands submitted since the last time, together (in as few messages
    /// of at most [`BATCH_BYTES`] as they fit in).
    pub fn flush(&mut self) {
        let Role::Client(c) = &mut self.role else { return };
        if self.outgoing.is_empty() {
            return;
        }
        for batch in batches(std::mem::take(&mut self.outgoing), BATCH_BYTES) {
            c.send(&Msg::Commands(batch));
        }
    }

    /// Reads the network: collects peers' commands (host) or bundles (client). A client
    /// first sends what it has submitted.
    pub fn poll(&mut self) {
        self.flush();
        match &mut self.role {
            Role::Host(host) => {
                let mut pace = Vec::new();
                for event in host.poll() {
                    match event {
                        HostEvent::Message(peer, Msg::Commands(cmds)) => {
                            // Peers may only command their own player.
                            let Some(&player) = self.peer_players.get(&peer) else { continue };
                            collect_commands(player, cmds, &mut self.collected, &mut self.pending_bytes,
                                &mut self.pending_memory, self.peer_pending.entry(peer).or_default());
                        }
                        HostEvent::Message(peer, Msg::Hash { tick, hash }) => {
                            let Some(t) = self.peer_ticks.get_mut(&peer) else { continue };
                            if tick == 0 || tick >= self.next_tick || !tick.is_multiple_of(HASH_INTERVAL) { continue; }
                            *t = (*t).max(tick);
                            if self.hashes.get(&tick).is_some_and(|&h| h != hash) && self.desync.is_none() {
                                log::error!("desync: peer {peer} differs at tick {tick}");
                                self.desync = Some(tick);
                                host.broadcast(&Msg::Desync { tick });
                            } else if self.hashes.last_key_value().is_none_or(|(&last, _)| tick > last) {
                                self.peer_hashes.entry(tick).or_default().entry(peer).or_insert(hash);
                                while self.peer_hashes.len() > 64 { self.peer_hashes.pop_first(); }
                            }
                        }
                        HostEvent::Message(peer, Msg::Chat { from, text }) => {
                            let (from, text) = (clean(&from, NAME_MAX), clean(&text, CHAT_MAX));
                            if !text.trim().is_empty() {
                                let source = self.peer_players.get(&peer).copied();
                                let Some((text, private)) = validation::authenticate_chat(&text, source) else { continue };
                                if validation::chat_visible(self.start.as_ref(), source, self.peer_players.get(&0).copied(), private) {
                                    self.chat.push((from.clone(), text.clone()));
                                }
                                host.broadcast_where(&Msg::Chat { from, text }, |peer| validation::chat_visible(self.start.as_ref(), source, self.peer_players.get(&peer).copied(), private));
                            }
                        }
                        HostEvent::Message(peer, Msg::SetPace { paused, speed }) => {
                            // Only the players in the game (not stray connections) set the pace.
                            if let Some(name) = self.peer_names.get(&peer).cloned() {
                                pace.push((paused, speed, name));
                            }
                        }
                        HostEvent::Left(peer, name) => {
                            // (One that never got back in, still on its way or turned away for
                            // want of the map: nobody was told it came, nor is told it went.)
                            let arriving = self.arriving.iter().position(|(p, ..)| *p == peer);
                            if let Some(i) = arriving {
                                self.arriving.remove(i);
                                continue;
                            }
                            if self.refused.remove(&peer) {
                                continue;
                            }
                            self.peer_ticks.remove(&peer);
                            self.peer_names.remove(&peer);
                            self.peer_pending.remove(&peer);
                            self.resuming.remove(&peer);
                            // A player's army waits a while for them to come back (see
                            // `absent`), then goes to an ally.
                            if let Some(player) = self.peer_players.remove(&peer) {
                                self.absent.push((name.clone(), player, std::time::Instant::now()));
                                if self.reconnect_available {
                                    host.rejoin.push(name.clone());
                                } else {
                                    host.forget_rejoin(&name);
                                }
                            }
                            self.departed.push(name);
                        }
                        // Someone back on a new connection: let in (or not) below, in turn, once
                        // their copy of the battle's map is known (`come_back`).
                        HostEvent::Joined(peer, name) => self.arriving.push((peer, name, std::time::Instant::now())),
                        _ => {}
                    }
                }
                for (paused, speed, name) in pace {
                    self.apply_pace(paused, speed, &name);
                }
                self.arrivals();
                // Players who stayed away too long: an ally takes over their army (the
                // simulation picks which, the same on every machine).
                let after = self.abandon_after;
                let (gone, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.absent).into_iter().partition(|(_, _, since)| since.elapsed() >= after);
                self.absent = waiting;
                if let Role::Host(host) = &mut self.role {
                    for (name, player, _) in gone {
                        host.forget_rejoin(&name);
                        self.collected.push(Command::Abandon { player });
                    }
                    // Who is away, and for how much longer: told when it changes, and once a
                    // second while anyone is.
                    let away: Vec<(u8, u16)> = self.absent.iter().map(|(_, player, since)| (*player, after.saturating_sub(since.elapsed()).as_secs().min(u16::MAX as u64) as u16)).collect();
                    let due = self.away_told.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(1));
                    if away != self.away || (!away.is_empty() && due) {
                        host.broadcast(&Msg::Away(away.clone()));
                        self.away_told = Some(std::time::Instant::now());
                        self.away = away;
                    }
                }
            }
            // (The next battle begun: its messages wait for its session.)
            Role::Client(_) if self.next_battle.is_some() => {}
            Role::Client(client) => {
                for msg in client.poll() {
                    match msg {
                        Msg::Tick { tick, commands } => {
                            let bytes = commands.iter().map(command_memory).sum::<usize>();
                            if self.host_lost || tick != self.next_tick || tick == u64::MAX || self.ready.len() >= MAX_READY_TICKS
                                || self.ready_bytes.saturating_add(bytes) > MAX_READY_BYTES {
                                client.disconnect();
                                // (Too far behind to follow: we let go, the host did not leave.)
                                self.dropped = !self.host_lost && tick == self.next_tick;
                                self.host_lost = true;
                                break;
                            }
                            self.ticking = true;
                            self.next_tick += 1;
                            self.ready_bytes += bytes;
                            self.ready.push_back((tick, commands));
                        }
                        Msg::Desync { tick } => self.desync = self.desync.or(Some(tick)),
                        Msg::Chat { from, text } => self.chat.push((clean(&from, NAME_MAX), clean(&text, CHAT_MAX))),
                        Msg::Pace { paused, speed, by } => {
                            let was = (self.paused, self.speed);
                            (self.paused, self.speed) = (paused, speed.clamp(SPEED_MIN, SPEED_MAX));
                            announce_pace(&mut self.notices, &mut self.paused_by, was, (self.paused, self.speed), &clean(&by, NAME_MAX));
                        }
                        Msg::Waiting(names) => self.waiting_for = names.iter().map(|n| clean(n, NAME_MAX)).collect(),
                        Msg::Away(away) => self.away = away,
                        // The host went back to the lobby: the battle is over; the lobby, kept
                        // as it changes, is where this player can follow.
                        Msg::Rematch(lobby) => {
                            self.rematch = true;
                            self.lobby = Some(lobby);
                        }
                        Msg::Lobby(lobby) if self.rematch => self.lobby = Some(lobby),
                        // The host went on to the next battle while we were still here: it
                        // waits for us (`next_battle`).
                        Msg::Start(info) if self.rematch => {
                            self.next_battle = Some(info);
                            break;
                        }
                        Msg::Returned { old, new } => {
                            if let Some(entry) = self.roster.remove(&old) {
                                self.roster.insert(new, entry);
                            }
                        }
                        Msg::ReconnectUnavailable => self.notices.push(Notice::ReconnectLimit),
                        // The host's goodbye: it left, or (having said so first) it went on
                        // without us.
                        Msg::Bye if !self.host_lost => {
                            self.host_lost = true;
                            if client.removed {
                                self.dropped = true;
                            } else {
                                self.departed.push(host_name(&self.roster));
                            }
                        }
                        _ => {}
                    }
                }
                // Nothing from the host (not even its pings, every two seconds) for a long while
                // in a battle: as good as a failed connection (the dialog offers REJOIN).
                if self.ticking && !self.host_lost && self.next_battle.is_none() && client.heard().is_some_and(|at| at.elapsed() >= HOST_SILENT) {
                    log::warn!("no word from the host for {} s: the connection is taken as lost", HOST_SILENT.as_secs());
                    client.disconnect();
                }
                // No goodbye: our connection failed.
                if !client.is_connected() && !self.host_lost && self.next_battle.is_none() {
                    self.host_lost = true;
                    if client.removed {
                        self.dropped = true;
                    } else {
                        self.link_failed = true;
                    }
                }
            }
        }
        self.send_resume_parts();
    }

    /// At most one history chunk per returning peer per poll (see [`history_part_end`]). Live
    /// ticks are withheld
    /// until Resume is queued, so commands accumulated during transfer join the history
    /// first; Resume then snapshots the current tick and pace without a gap or duplicate.
    fn send_resume_parts(&mut self) {
        let Role::Host(host) = &mut self.role else { return };
        let Some(start) = &self.start else { return };
        self.resuming.retain(|&peer, progress| {
            let Some(&player) = self.peer_players.get(&peer) else { return false };
            if progress.pending.is_none() && progress.next_history < self.history.len() {
                let end = history_part_end(&self.history, progress.next_history);
                progress.pending = Some((end, Msg::History(self.history[progress.next_history..end].to_vec())));
            }
            let result = if let Some((_, msg)) = &progress.pending {
                host.try_send_to(peer, msg)
            } else {
                host.try_send_to(peer, &Msg::Resume {
                    start: start.clone(), player, upto: self.next_tick - 1,
                    history: Vec::new(), paused: self.paused, speed: self.speed,
                })
            };
            match result {
                Ok(()) => match progress.pending.take() {
                    Some((end, _)) => {
                        progress.next_history = end;
                        true
                    }
                    None => false, // Resume precedes all subsequent live ticks in the queue.
                },
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => true,
                Err(_) => {
                    host.kick(peer);
                    false
                }
            }
        });
    }

    /// Host: seals the bundle for the next tick and sends it to every peer.
    pub fn produce_tick(&mut self) {
        let Role::Host(host) = &mut self.role else { return };
        let tick = self.next_tick;
        self.next_tick += 1;
        let commands = std::mem::take(&mut self.collected);
        self.pending_bytes = 0;
        self.pending_memory = 0;
        self.peer_pending.clear();
        // (Not to those catching up, whose ticks come after their history, nor to those still
        // on their way back, who may never be let in.)
        let arriving = &self.arriving;
        host.broadcast_where(&Msg::Tick { tick, commands: commands.clone() }, |peer| !self.resuming.contains_key(&peer) && !arriving.iter().any(|(p, ..)| *p == peer));
        if self.reconnect_available && !commands.is_empty() {
            let bytes = validation::history_entry_memory(&commands);
            if self.history_bytes.saturating_add(bytes) <= MAX_HISTORY_BYTES {
                self.history_bytes += bytes;
                self.history.push((tick, commands.clone()));
            } else {
                self.reconnect_available = false;
                self.history = Vec::new();
                self.history_bytes = 0;
                host.clear_rejoin();
                host.broadcast(&Msg::ReconnectUnavailable);
                for &peer in self.resuming.keys() {
                    host.send_to(peer, &Msg::Reject { reason: Refusal::Started });
                    host.kick(peer);
                }
                self.resuming.clear();
                self.notices.push(Notice::ReconnectLimit);
            }
        }
        self.ready.push_back((tick, commands));
    }

    /// The bundle for `tick`, if it has arrived (bundles are consumed in order).
    pub fn take_tick(&mut self, tick: u64) -> Option<Vec<Command>> {
        if tick >= self.resume_next && tick <= self.resume_upto {
            self.resume_next = tick + 1;
            while self.resume_history.front().is_some_and(|(t, _)| *t < tick) { self.resume_history.pop_front(); }
            return Some(if self.resume_history.front().is_some_and(|(t, _)| *t == tick) {
                self.resume_history.pop_front().unwrap().1
            } else { Vec::new() });
        }
        while self.ready.front().is_some_and(|(t, _)| *t < tick) {
            if let Some((_, commands)) = self.ready.pop_front() {
                self.ready_bytes = self.ready_bytes.saturating_sub(commands.iter().map(command_memory).sum::<usize>());
            }
        }
        match self.ready.front() {
            Some((t, _)) if *t == tick => self.ready.pop_front().map(|(_, c)| {
                self.ready_bytes = self.ready_bytes.saturating_sub(c.iter().map(command_memory).sum::<usize>());
                c
            }),
            _ => None,
        }
    }

    /// Host: retained reconnect commands, in order (empty after its memory limit is reached).
    pub fn history_commands(&self) -> Vec<Command> {
        self.history.iter().flat_map(|(_, c)| c.iter().cloned()).collect()
    }

    /// Bundles received but not yet executed (clients fast-forward when this grows).
    pub fn backlog(&self) -> usize {
        let remaining = self.resume_upto.saturating_sub(self.resume_next.saturating_sub(1));
        self.ready.len().saturating_add(usize::try_from(remaining).unwrap_or(usize::MAX))
    }

    /// Records our state hash after `tick` (every [`HASH_INTERVAL`] ticks).
    pub fn report_hash(&mut self, tick: u64, hash: u64) {
        if !tick.is_multiple_of(HASH_INTERVAL) {
            return;
        }
        match &mut self.role {
            Role::Host(host) => {
                if self.peer_hashes.remove(&tick).is_some_and(|reports| reports.values().any(|&h| h != hash)) && self.desync.is_none() {
                    self.desync = Some(tick);
                    host.broadcast(&Msg::Desync { tick });
                }
                self.hashes.insert(tick, hash);
                while self.hashes.len() > 64 {
                    self.hashes.pop_first();
                }
            }
            Role::Client(c) => c.send(&Msg::Hash { tick, hash }),
        }
    }

    /// Tells the other side we are leaving (a client sends what it still had to first).
    pub fn leave(&mut self) {
        self.flush();
        match &mut self.role {
            Role::Host(h) => h.broadcast(&Msg::Bye),
            Role::Client(c) => c.send(&Msg::Bye),
        }
    }

    /// The program closing: [`Session::leave`], the goodbye given at most `timeout` to be
    /// written (else the others would take it for a failed connection).
    pub fn goodbye(&mut self, timeout: std::time::Duration) {
        self.flush();
        self.role.goodbye(timeout);
    }
}

/// Notices for a pace change (`was` and `now` are (paused, speed)), and who paused.
fn announce_pace(notices: &mut Vec<Notice>, paused_by: &mut Option<String>, was: (bool, u16), now: (bool, u16), by: &str) {
    if now.0 != was.0 {
        notices.push(if now.0 { Notice::Paused(by.to_string()) } else { Notice::Resumed(by.to_string()) });
        *paused_by = now.0.then(|| by.to_string());
    }
    if now.1 != was.1 {
        notices.push(Notice::Speed(by.to_string(), now.1));
    }
}

/// Splits `items` into runs, in order, of at most `budget` bytes as bincode writes them (one
/// larger than that goes alone).
pub fn batches<T: Serialize>(items: Vec<T>, budget: u64) -> Vec<Vec<T>> {
    let mut out: Vec<Vec<T>> = Vec::new();
    let mut size = 0;
    for item in items {
        let n = bincode::serialized_size(&item).unwrap_or(0);
        match out.last_mut() {
            Some(run) if size + n <= budget => run.push(item),
            _ => {
                out.push(vec![item]);
                size = 0;
            }
        }
        size += n;
    }
    out
}

/// What a rejoin's history part holds in memory at most, as the returning player counts it
/// (`validation::history_memory`).
const HISTORY_PART_MEMORY: usize = transport::MAX_MESSAGE_MEMORY / 4;

/// Where the part of `history` that starts at `from` ends: at [`BATCH_BYTES`] on the wire, or
/// [`HISTORY_PART_MEMORY`] in memory, whichever comes first (a tick's commands at least). (Cut by
/// bytes alone, a part of thousands of tiny commands, a flood of them, held many times its bytes
/// and was more than the returning player takes in one message: it could never come back.)
fn history_part_end(history: &[(u64, Vec<Command>)], from: usize) -> usize {
    let (mut end, mut bytes, mut memory) = (from, 0, 0);
    while let Some(item) = history.get(end) {
        let size = bincode::serialized_size(item).unwrap_or(0);
        let held = validation::history_entry_memory(&item.1);
        if end > from && (bytes + size > BATCH_BYTES || memory + held > HISTORY_PART_MEMORY) {
            break;
        }
        bytes += size;
        memory += held;
        end += 1;
    }
    end
}

/// The player a command is issued for.
pub fn command_player(c: &Command) -> u8 {
    match c {
        Command::Order { player, .. }
        | Command::Produce { player, .. }
        | Command::SetRally { player, .. }
        | Command::SetActive { player, .. }
        | Command::SetCloak { player, .. }
        | Command::SetStates { player, .. }
        | Command::SelfDestruct { player, .. }
        | Command::SetRepeat { player, .. }
        | Command::Share { player, .. }
        | Command::GiveUnits { player, .. }
        | Command::QueueFront { player, .. }
        | Command::EditQueue { player, .. }
        | Command::Cheat { player, .. }
        | Command::SetSharing { player, .. }
        | Command::SetShootAll { player, .. }
        | Command::Formation { player, .. }
        | Command::Signal { player, .. }
        | Command::Abandon { player }
        | Command::Resign { player } => *player,
    }
}

/// Resolves "host:port" or "host" (default port) to a socket address.
pub fn resolve(address: &str) -> io::Result<SocketAddr> {
    use std::net::ToSocketAddrs;
    let address = address.trim();
    let host = address.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(address);
    if let Ok(ip) = host.parse::<std::net::IpAddr>() { return Ok(SocketAddr::new(ip, DEFAULT_PORT)); }
    let with_port = if address.contains(':') { address.to_string() } else { format!("{address}:{DEFAULT_PORT}") };
    preferred_address(with_port.to_socket_addrs()?)
}

fn preferred_address(mut addresses: impl Iterator<Item = SocketAddr>) -> io::Result<SocketAddr> {
    let first = addresses.next().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "address not found"))?;
    // Reborn hosts listen on IPv4. A dual-stack DNS answer must not select IPv6
    // merely because it comes first; explicit/IPv6-only addresses still work.
    Ok(if first.is_ipv4() { first } else { addresses.find(SocketAddr::is_ipv4).unwrap_or(first) })
}

#[cfg(test)]
mod address_tests {
    use super::*;

    #[test]
    fn literal_addresses_accept_default_and_explicit_ports() {
        for (input, expected) in [
            ("127.0.0.1", "127.0.0.1:7890"), ("127.0.0.1:8123", "127.0.0.1:8123"),
            ("::1", "[::1]:7890"), ("[::1]", "[::1]:7890"), ("[::1]:8123", "[::1]:8123"),
            (" 2001:db8::1 ", "[2001:db8::1]:7890"),
        ] { assert_eq!(resolve(input).unwrap(), expected.parse::<SocketAddr>().unwrap(), "{input}"); }
        assert!(resolve("[::1]:not-a-port").is_err());
    }

    #[test]
    fn hostname_answers_prefer_the_host_listeners_family_without_losing_ipv6_fallback() {
        let v6 = "[::1]:7890".parse::<SocketAddr>().unwrap();
        let v4 = "127.0.0.1:7890".parse::<SocketAddr>().unwrap();
        assert_eq!(preferred_address([v6, v4].into_iter()).unwrap(), v4);
        assert_eq!(preferred_address([v4, v6].into_iter()).unwrap(), v4);
        assert_eq!(preferred_address([v6].into_iter()).unwrap(), v6);
        assert_eq!(preferred_address([].into_iter()).unwrap_err().kind(), io::ErrorKind::NotFound);
    }
}

#[cfg(test)]
mod data_tests {
    use super::*;

    /// A build of another protocol reads our hello and our "another version" refusal as its own
    /// do (they are the first thing either side reads): their bytes never change.
    #[test]
    fn the_hello_and_the_version_refusal_keep_their_layout() {
        let hello = Msg::Hello { name: "Ann".into(), protocol: 79, data_hash: 0x0102_0304_0506_0708, reconnect: None, password: "pw".into() };
        let mut expected = vec![0, 0, 0, 0]; // the first message
        expected.extend(3u64.to_le_bytes());
        expected.extend(b"Ann");
        expected.extend(79u32.to_le_bytes());
        expected.extend(0x0102_0304_0506_0708u64.to_le_bytes());
        expected.push(0); // no credential
        expected.extend(2u64.to_le_bytes());
        expected.extend(b"pw");
        assert_eq!(bincode::serialize(&hello).unwrap(), expected);
        let version = Msg::Reject { reason: Refusal::Version { host: 79, yours: 78 } };
        let mut expected = vec![2, 0, 0, 0, 0, 0, 0, 0]; // the third message, the first refusal
        expected.extend(79u32.to_le_bytes());
        expected.extend(78u32.to_le_bytes());
        assert_eq!(bincode::serialize(&version).unwrap(), expected);
    }

    #[test]
    fn a_persons_map_is_ready_only_when_it_is_the_hosts() {
        assert_eq!(MapStatus::of(Some(5), None), MapStatus::Checking, "not told yet");
        assert_eq!(MapStatus::of(Some(5), Some(Some(5))), MapStatus::Ready);
        assert_eq!(MapStatus::of(Some(5), Some(None)), MapStatus::Missing);
        assert_eq!(MapStatus::of(Some(5), Some(Some(6))), MapStatus::Other, "the same name, other contents");
        assert_eq!(MapStatus::of(None, Some(Some(6))), MapStatus::Missing, "a map the host has not: nobody can play it");
    }

    #[test]
    fn the_battle_waits_for_everyone_to_have_the_map_spectators_too() {
        let human = |peer: u8| LobbySlot { kind: SlotKind::Human { peer, name: format!("P{peer}") }, side: "ARM".into(), color: peer, ready: true, team: peer };
        let mut lobby = Lobby { slots: vec![human(0), human(1), LobbySlot { kind: SlotKind::Ai { difficulty: 1 }, ..human(9) }], spectators: vec![(2, "W".into())], ..Default::default() };
        assert!(!lobby.everyone_has_the_map(), "nobody has told the host yet");
        assert_eq!(lobby.map_status_of(0), MapStatus::Ready, "the host's own");
        lobby.map_status = vec![(1, MapStatus::Ready), (2, MapStatus::Missing)];
        assert!(!lobby.everyone_has_the_map(), "a spectator without it");
        lobby.map_status[1].1 = MapStatus::Ready;
        assert!(lobby.everyone_has_the_map());
        lobby.map_status[0].1 = MapStatus::Other;
        assert!(!lobby.everyone_has_the_map());
    }
}

#[cfg(test)]
mod history_part_tests {
    use super::*;

    /// Every part of a rejoin's history fits what the returning player takes in: its reader's
    /// budget (64 times the bytes) and the memory a message may hold. A flood of tiny commands
    /// as well as an army's orders.
    #[test]
    fn rejoin_history_parts_fit_the_reader() {
        let tiny = Command::Abandon { player: 1 };
        let big = Command::Order { player: 0, units: (0..400).map(|i| ta_sim::UnitId { slot: i, generation: 1 }).collect(), order: ta_sim::Order::Stop, queue: false };
        for (command, per_tick) in [(tiny, 2_000), (big, 4)] {
            let history: Vec<(u64, Vec<Command>)> = (1..=600u64).map(|t| (t, vec![command.clone(); per_tick])).collect();
            let (mut from, mut parts) = (0, 0);
            while from < history.len() {
                let end = history_part_end(&history, from);
                assert!(end > from, "a part holds a tick at least");
                let msg = Msg::History(history[from..end].to_vec());
                let wire = bincode::serialized_size(&msg).unwrap() as usize;
                let memory = message_memory(&msg) + wire;
                assert!(memory <= transport::MAX_MESSAGE_MEMORY, "{memory} in memory");
                assert!(memory <= wire * 64 + std::mem::size_of::<Msg>(), "{memory} for {wire} bytes");
                from = end;
                parts += 1;
            }
            assert!(parts > 1, "cut into parts");
        }
    }
}
