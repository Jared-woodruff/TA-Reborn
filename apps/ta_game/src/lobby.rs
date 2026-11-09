//! Multiplayer front end: host or join, then the lobby (seats, sides, colours, computer
//! players, the map and who has it, pings, pictures, ready states, the countdown, chat) until
//! the host starts the game. The page itself is `lobby/page.rs`.

use std::collections::BTreeMap;

use ta_net::lan::{Announcement, Beacon};
use ta_net::{Client, Host, HostEvent, Lobby, LobbySlot, MapStatus, Msg, Session, SlotKind, StartInfo};

use crate::setup::{Assets, Controller, GameSetup, SlotSetup};
const MAX_SLOTS: usize = 10;
mod chat;
mod connect;
mod data;
mod page;
mod portraits;
mod reconnect;
pub(crate) mod signal;

pub use connect::{ConnectAction, ConnectForm, Keep};

/// How many games joined lately the Multiplayer page keeps.
pub fn connect_recent() -> usize {
    connect::RECENT
}

// (One action a frame at most: the size difference between its variants costs nothing.)
#[allow(clippy::large_enum_variant)]
pub enum LobbyAction {
    None,
    /// Leave the lobby (back to the connect page).
    Leave,
    /// The host removed us or the connection ended: back to the connect page, saying so.
    Closed(String),
    Start(GameSetup, Session),
}

/// Whether a battle is over for everyone: one team still standing, or none.
pub fn battle_over(game: &crate::game::Game) -> bool {
    game.sim.players.iter().filter(|p| !p.defeated).map(|p| p.team).collect::<std::collections::BTreeSet<_>>().len() <= 1
}

/// The keys of the sides the game data offers (ARM, CORE; a mod's own).
fn side_keys(assets: &Assets) -> Vec<String> {
    crate::setup::side_choices(&assets.data).into_iter().map(|s| s.key).collect()
}

/// The side a new seat starts with: the data's sides in turn.
pub(crate) fn seat_side(sides: &[String], seat: usize) -> String {
    match sides.len() {
        0 => if seat.is_multiple_of(2) { "ARM" } else { "CORE" }.to_string(),
        n => sides[seat % n].clone(),
    }
}

/// The name of the person at `peer` (seated or watching), as the lobby knows them.
fn person_name(state: &Lobby, peer: u8) -> Option<String> {
    state.slots.iter().find_map(|s| match &s.kind {
        SlotKind::Human { peer: p, name } if *p == peer => Some(name.clone()),
        _ => None,
    }).or_else(|| state.spectators.iter().find(|(p, _)| *p == peer).map(|(_, n)| n.clone()))
}

/// How long a player joining waits for the host's answer.
pub(crate) const ANSWER: std::time::Duration = std::time::Duration::from_secs(8);

enum Role {
    Host(Host),
    Client(Client),
}

pub struct NetLobby {
    role: Option<Role>,
    pub state: Lobby,
    /// Our peer id (0 = host).
    me: u8,
    name: String,
    chat: chat::Chat,
    filter: String,
    seed: u64,
    /// Mark ourselves ready as soon as we have a slot (testing).
    pub auto_ready: bool,
    /// Ask to watch as soon as we are in (`--watch`).
    pub auto_watch: bool,
    /// Host: tells the local network about the lobby (with our port and data hash).
    beacon: Option<(Beacon, u16, u64)>,
    reconnect_key: Option<reconnect::Key>,
    /// Everyone's round trip to the host (peer, ms), as last measured.
    pings: Vec<(u8, u16)>,
    /// The people's pictures (peer → PNG): the host keeps them and shares them.
    portraits: BTreeMap<u8, Vec<u8>>,
    textures: portraits::Textures,
    /// Our own picture has gone to the host (or into the host's own table).
    portrait_sent: bool,
    /// Host: when the countdown last went down a second.
    countdown_at: Option<std::time::Instant>,
    /// Host: the names of those it took out (said so in the chat).
    removed: Vec<String>,
    /// Host: where players on this network reach the lobby ("192.168.1.20:7890").
    address: Option<String>,
    /// The lobby as last seen, for the chat's notices of what changed.
    seen: Lobby,
    /// The countdown's second last heard.
    spoken: Option<u8>,
    /// The address we joined (for the games joined lately), and the battle's name once the
    /// lobby tells it, for the settings to keep (the app takes it).
    joined: Option<String>,
    learned: Option<(String, String)>,
    /// Client: the address we joined (the battle's sessions keep it, to come back to).
    server: Option<String>,
    /// Host: the sides the game data offers (their keys), dealt in turn to new seats.
    sides: Vec<String>,
    /// Client: "connected" told, with the first lobby heard (a player coming back to a battle
    /// hears none).
    told_connected: bool,
    /// Host: the way out asked about (it sends everyone else in the lobby away).
    closing: bool,
    /// Host: our maps' digests by file stem, to tell who has the battle's map, and their names
    /// (told with the lobby, for those without a copy).
    own_maps: BTreeMap<String, u64>,
    titles: BTreeMap<String, String>,
}

/// A host for a lobby on `port`: joining players must have our game data but its maps (the
/// battle's map is checked in the lobby), and one turned away for other data is told our mods.
pub(crate) fn bind_host(port: u16, assets: &Assets) -> std::io::Result<Host> {
    let mut host = Host::bind(port, assets.network_hash())?;
    host.set_mods(data::our_mods(assets));
    Ok(host)
}

/// Host: whether each person in the lobby (by peer, in order) has the battle's map, from what
/// they told us of their maps (`ours`: our digest of it).
fn map_status(host: &Host, state: &Lobby, ours: Option<u64>) -> Vec<(u8, MapStatus)> {
    let mut status: Vec<(u8, MapStatus)> = host.peers().into_iter().map(|(peer, _)| (peer, MapStatus::of(ours, host.peer_map(peer, &state.map)))).collect();
    status.sort_by_key(|(peer, _)| *peer);
    status
}

/// Why the host turned us away, in our own words (for other data: which of our mods differ
/// from the host's, and what to do).
fn refusal_text(reason: &ta_net::Refusal, assets: &Assets) -> String {
    use ta_net::Refusal as R;
    match reason {
        R::Version { host, yours } => ta_ui::i18n::tf("The host runs another version of TA Reborn (network {host}, yours {yours}): you both need the same one.", &[("host", host), ("yours", yours)]),
        R::Data { mods } => {
            let installed = crate::record_mods::installed(&ta_config::Config::load());
            data::refusal(mods, &data::our_mods(assets), &installed)
        }
        R::Started => ta_ui::i18n::t("The battle has already begun.").to_string(),
        R::Credential => ta_ui::i18n::t("Only this name's own player can come back to the battle, from their computer.").to_string(),
        R::Name => ta_ui::i18n::t("Someone in the lobby already has your name: choose another.").to_string(),
        R::Password => ta_ui::i18n::t("That is not the battle's password.").to_string(),
        R::Map { map, other: false } => ta_ui::i18n::tf("You do not have this battle's map, {map}: turn on its map pack in the launcher to come back.", &[("map", map)]),
        R::Map { map, other: true } => ta_ui::i18n::tf("Your copy of this battle's map, {map}, is not the host's version: you need the host's to come back.", &[("map", map)]),
    }
}

impl NetLobby {
    fn hosting(host: Host, name: &str, assets: &Assets, title: &str) -> NetLobby {
        let map = assets.selectable_maps.iter().find(|m| m.name.eq_ignore_ascii_case("Painted Desert")).or(assets.selectable_maps.first()).map(|m| m.name.clone()).unwrap_or_default();
        let sides = side_keys(assets);
        let slot = |kind, seat: usize| LobbySlot { kind, side: seat_side(&sides, seat), color: seat as u8, ready: false, team: seat as u8 };
        let state = Lobby {
            map,
            slots: vec![
                LobbySlot { ready: true, ..slot(SlotKind::Human { peer: 0, name: name.to_string() }, 0) },
                slot(SlotKind::Open, 1),
            ],
            rules: Default::default(),
            spectators: Vec::new(),
            title: title.to_string(),
            countdown: None,
            map_status: Vec::new(),
            map_title: String::new(),
        };
        NetLobby::hosting_with(host, name, assets, state)
    }

    /// The host's lobby as `state` has it.
    fn hosting_with(mut host: Host, name: &str, assets: &Assets, state: Lobby) -> NetLobby {
        host.set_host_name(name);
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
        let beacon = Beacon::new().map_err(|e| log::info!("not announcing the lobby on the LAN: {e}")).ok().map(|b| (b, host.port, assets.network_hash()));
        let address = local_address(host.port);
        let own_maps = assets.compatibility.map_digests().into_iter().collect();
        let titles = assets.maps.iter().map(|m| (m.name.clone(), m.title.clone())).collect();
        NetLobby { beacon, address, seed, own_maps, titles, sides: side_keys(assets), seen: state.clone(), state, me: 0, ..NetLobby::new(Role::Host(host), name, None) }
    }

    fn joined(client: Client, name: &str, key: reconnect::Key, address: Option<String>) -> NetLobby {
        let mut lobby = NetLobby { me: u8::MAX, server: address.clone(), joined: address, ..NetLobby::new(Role::Client(client), name, Some(key)) };
        lobby.chat.push(chat::What::Connecting);
        lobby
    }

    /// A battle begun while this player was still in the last one's result (the host went on
    /// from the lobby): straight into it.
    pub fn next_battle(session: Session, assets: &Assets) -> Option<(GameSetup, Session)> {
        let rejoin = session.rejoin_address.clone();
        let (client, info) = session.into_next_battle()?;
        let me = client.peer?;
        let player = info.player_of(me);
        let setup = setup_from(&info, player, start_positions(assets, &info.map), &crate::setup::start_spots(assets, &info.map));
        Some((setup, Session::client(client, player.unwrap_or(u8::MAX)).with_roster(&info).with_rejoin(rejoin)))
    }

    /// Back from a battle to its lobby. The host takes everyone still connected back (their
    /// seats and choices kept, nobody ready, the seats of those who left open again) and tells
    /// them; a player follows the host, once it has gone back. None when there is no lobby to
    /// go back to (a player whose host has not gone back, or is gone).
    pub fn after_battle(session: Session, name: &str, assets: &Assets) -> Option<NetLobby> {
        if !session.is_host() {
            let server = session.rejoin_address.clone();
            let (client, state) = session.into_client()?;
            let me = client.peer.unwrap_or(u8::MAX);
            let mut lobby = NetLobby { me, server, told_connected: true, seen: state.clone(), state, ..NetLobby::new(Role::Client(client), name, None) };
            lobby.chat.push(chat::What::Back);
            return Some(lobby);
        }
        let (mut host, state) = session.into_host()?;
        let mut state = state?;
        let here: std::collections::BTreeSet<u8> = host.peers().into_iter().map(|(peer, _)| peer).chain([0]).collect();
        for s in &mut state.slots {
            if let SlotKind::Human { peer, .. } = &s.kind
                && !here.contains(peer)
            {
                s.kind = SlotKind::Open;
            }
            // (The host is ready as a lobby opens: the others make up their minds again.)
            s.ready = matches!(&s.kind, SlotKind::Human { peer: 0, .. });
        }
        state.spectators.retain(|(peer, _)| here.contains(peer));
        state.map_status.retain(|(peer, _)| here.contains(peer));
        state.countdown = None;
        host.broadcast(&Msg::Rematch(state.clone()));
        let mut lobby = NetLobby::hosting_with(host, name, assets, state);
        lobby.chat.push(chat::What::Back);
        Some(lobby)
    }

    fn new(role: Role, name: &str, reconnect_key: Option<reconnect::Key>) -> NetLobby {
        NetLobby {
            role: Some(role),
            state: Lobby::default(),
            me: 0,
            name: name.to_string(),
            chat: chat::Chat::default(),
            filter: String::new(),
            seed: 0,
            auto_ready: false,
            auto_watch: false,
            beacon: None,
            reconnect_key,
            pings: Vec::new(),
            portraits: BTreeMap::new(),
            textures: portraits::Textures::default(),
            portrait_sent: false,
            countdown_at: None,
            removed: Vec::new(),
            address: None,
            seen: Lobby::default(),
            spoken: None,
            joined: None,
            learned: None,
            server: None,
            sides: Vec::new(),
            told_connected: false,
            closing: false,
            own_maps: BTreeMap::new(),
            titles: BTreeMap::new(),
        }
    }

    /// The battle's map's name: our copy's, else (a guest without one) the host's, else its
    /// file stem.
    fn map_title(&self, assets: &Assets) -> String {
        match assets.maps.iter().find(|m| m.name == self.state.map) {
            Some(m) => m.title.clone(),
            None if !self.state.map_title.trim().is_empty() => self.state.map_title.clone(),
            None => self.state.map.clone(),
        }
    }

    /// The address we joined and the battle's name, once the lobby has told it (for the games
    /// joined lately).
    pub fn take_learned(&mut self) -> Option<(String, String)> {
        self.learned.take()
    }

    /// Hosts a lobby directly (command line; `TA_LOBBY_PASSWORD` locks it, `TA_LOBBY_MAP=<map
    /// file stem>` chooses its battlefield, for captures).
    pub fn host_now(port: u16, name: &str, assets: &Assets) -> std::io::Result<NetLobby> {
        let mut host = bind_host(port, assets)?;
        host.set_password(std::env::var("TA_LOBBY_PASSWORD").ok().as_deref());
        let mut lobby = NetLobby::hosting(host, name, assets, &ta_ui::i18n::tf("{name}'s battle", &[("name", &name)]));
        if let Ok(wanted) = std::env::var("TA_LOBBY_MAP")
            && let Some(map) = assets.selectable_maps.iter().find(|m| m.name.eq_ignore_ascii_case(wanted.trim()) || m.title.eq_ignore_ascii_case(wanted.trim()))
        {
            lobby.state.map = map.name.clone();
            lobby.seen.map = map.name.clone();
        }
        Ok(lobby)
    }

    /// Joins a lobby directly (command line; with `TA_LOBBY_PASSWORD`), once the host has
    /// welcomed us.
    pub fn join_now(address: &str, name: &str, assets: &Assets) -> std::io::Result<NetLobby> {
        let password = std::env::var("TA_LOBBY_PASSWORD").unwrap_or_default();
        // (Told as the Multiplayer page tells them.)
        let failed = |e: std::io::Error| std::io::Error::new(e.kind(), connect::connect_error(&e, address));
        let (mut client, key) = reconnect::connect(ta_net::resolve(address).map_err(failed)?, name, assets.network_hash(), &password).map_err(failed)?;
        client.wait_for_answer(ANSWER).map_err(failed)?;
        if let Some(reason) = &client.rejected {
            log::info!("{address} turned us away: {reason:?}");
            return Err(std::io::Error::other(refusal_text(reason, assets)));
        }
        Ok(NetLobby::joined(client, name, key, Some(address.to_string())))
    }

    /// Host: starts once `players` humans are present and ready (testing).
    pub fn try_auto_start(&mut self, players: usize, assets: &Assets) -> Option<LobbyAction> {
        if let Some(a) = self.pump(assets) {
            return Some(a);
        }
        if !self.is_host() || !assets.selectable_maps.iter().any(|map| map.name == self.state.map) {
            return None;
        }
        let humans: Vec<&LobbySlot> = self.state.slots.iter().filter(|s| matches!(s.kind, SlotKind::Human { .. })).collect();
        let ready = humans.iter().all(|s| s.ready || matches!(&s.kind, SlotKind::Human { peer: 0, .. }));
        if humans.len() >= players && ready && self.state.everyone_has_the_map() {
            let max = max_slots(&self.state, assets);
            // `TA_NET_AIS=n`: n hard computer players join them (their orders are derived on
            // every machine: the check covers them).
            let ais: usize = std::env::var("TA_NET_AIS").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
            for _ in 0..ais.min(max.saturating_sub(self.state.slots.len())) {
                let team = first_free_team(&self.state);
                let side = seat_side(&self.sides, self.state.slots.len());
                self.state.slots.push(LobbySlot { kind: SlotKind::Ai { difficulty: 2 }, side, color: first_free_color(&self.state), ready: true, team });
            }
            return Some(self.start(max, assets));
        }
        None
    }

    fn is_host(&self) -> bool {
        matches!(self.role, Some(Role::Host(_)))
    }

    fn broadcast_lobby(&mut self) {
        if let Some(Role::Host(h)) = &mut self.role {
            // (Who has the map as it now is: a map just chosen goes out with its own, and its name.)
            self.state.map_status = map_status(h, &self.state, self.own_maps.get(&self.state.map.to_ascii_lowercase()).copied());
            self.state.map_title = self.titles.get(&self.state.map).cloned().unwrap_or_default();
            h.broadcast(&Msg::Lobby(self.state.clone()));
        }
    }

    fn broadcast_portraits(&mut self) {
        let table: Vec<(u8, Vec<u8>)> = self.portraits.iter().map(|(peer, png)| (*peer, png.clone())).collect();
        if let Some(Role::Host(h)) = &mut self.role {
            h.broadcast(&Msg::Portraits(table));
        }
    }

    /// Host: something about the players changed: the lobby goes out again, and a countdown
    /// under way is called off (everyone ready again, the host starts it again).
    fn changed(&mut self) {
        if self.state.countdown.take().is_some() {
            self.countdown_at = None;
        }
        self.broadcast_lobby();
    }

    /// Our own picture, once (it needs the profile, which the interface knows).
    pub(crate) fn own_portrait(&mut self, ctx: &egui::Context) {
        if self.portrait_sent {
            return;
        }
        match &mut self.role {
            Some(Role::Host(_)) => {
                self.portrait_sent = true;
                if let Some(png) = portraits::own(ctx) {
                    self.portraits.insert(0, png);
                    self.broadcast_portraits();
                }
            }
            Some(Role::Client(c)) if c.peer.is_some() => {
                self.portrait_sent = true;
                if let Some(png) = portraits::own(ctx) {
                    c.send(&Msg::Portrait(png));
                }
            }
            _ => {}
        }
    }

    /// Handles network traffic; may produce a start (clients) or the countdown's end (host).
    fn pump(&mut self, assets: &Assets) -> Option<LobbyAction> {
        let action = match self.role.as_mut()? {
            Role::Host(host) => {
                let mut changed = false;
                let mut pictures = false;
                let events = host.poll();
                self.pings = host.pings();
                let locked = host.locked();
                for e in events {
                    match e {
                        HostEvent::Joined(peer, name) => {
                            let slot = LobbySlot { kind: SlotKind::Human { peer, name }, side: seat_side(&self.sides, self.state.slots.len()), color: first_free_color(&self.state), ready: false, team: first_free_team(&self.state) };
                            let max = max_slots(&self.state, assets);
                            place_player(&mut self.state, slot, max);
                            changed = true;
                            pictures = true;
                        }
                        HostEvent::Left(peer, _) => {
                            vacate(&mut self.state, peer);
                            self.state.spectators.retain(|(p, _)| *p != peer);
                            pictures |= self.portraits.remove(&peer).is_some();
                            changed = true;
                        }
                        HostEvent::Message(peer, Msg::SetSpectator(watch)) => {
                            let max = max_slots(&self.state, assets);
                            changed |= set_spectator(&mut self.state, peer, watch, max, &self.sides);
                        }
                        HostEvent::Message(peer, Msg::TakeSlot(seat)) => {
                            let max = max_slots(&self.state, assets);
                            changed |= take_seat(&mut self.state, peer, seat as usize, max);
                        }
                        HostEvent::Message(peer, Msg::SetSlot { side, color, ready }) => {
                            let updated = set_slot(&mut self.state, peer, side, color, ready, &self.sides);
                            changed |= updated;
                            if !updated {
                                // Correct an optimistic client whose colour was taken first,
                                // without cancelling an otherwise unchanged countdown.
                                host.send_to(peer, &Msg::Lobby(self.state.clone()));
                            }
                        }
                        HostEvent::Message(peer, Msg::Portrait(png)) => {
                            // (Only a picture that reads goes round.)
                            if png.is_empty() {
                                pictures |= self.portraits.remove(&peer).is_some();
                            } else if portraits::decode(&png).is_some() {
                                self.portraits.insert(peer, png);
                                pictures = true;
                            }
                        }
                        HostEvent::Message(peer, Msg::Chat { from, text }) => {
                            if text.starts_with(ta_net::PING_PREFIX) || text.starts_with(ta_net::ALLY_PREFIX) {
                                continue;
                            }
                            let from = person_name(&self.state, peer).unwrap_or(from);
                            self.chat.said(&from, &text);
                            host.broadcast(&Msg::Chat { from, text });
                        }
                        HostEvent::Message(..) => {}
                    }
                }
                // Who has the battle's map, as they have told us their maps: told when it
                // changes (that alone calls off no countdown: it can only have become ready).
                let status = map_status(host, &self.state, self.own_maps.get(&self.state.map.to_ascii_lowercase()).copied());
                let found = status != self.state.map_status;
                if pictures {
                    self.broadcast_portraits();
                }
                if changed {
                    self.changed();
                } else if found {
                    self.broadcast_lobby();
                }
                // The countdown, a second at a time; at its end the battle starts.
                if let (Some(n), Some(at)) = (self.state.countdown, self.countdown_at)
                    && at.elapsed() >= std::time::Duration::from_secs(1)
                {
                    if n <= 1 {
                        let max = max_slots(&self.state, assets);
                        return Some(self.start(max, assets));
                    }
                    self.state.countdown = Some(n - 1);
                    self.countdown_at = Some(std::time::Instant::now());
                    self.broadcast_lobby();
                }
                if let Some((beacon, port, data_hash)) = &mut self.beacon {
                    let count = |f: fn(&SlotKind) -> bool| self.state.slots.iter().filter(|s| f(&s.kind)).count() as u8;
                    beacon.announce(&Announcement {
                        protocol: ta_net::PROTOCOL,
                        data_hash: *data_hash,
                        port: *port,
                        host: self.name.clone(),
                        map: self.state.map.clone(),
                        players: count(|k| matches!(k, SlotKind::Human { .. })),
                        open: count(|k| matches!(k, SlotKind::Open)),
                        title: self.state.title.clone(),
                        seats: max_slots(&self.state, assets) as u8,
                        locked,
                    });
                }
                None
            }
            Role::Client(client) => {
                let mut start = None;
                for m in client.poll() {
                    match m {
                        Msg::Welcome { peer, reconnect } => {
                            self.me = peer;
                            if let Some(key) = &self.reconnect_key
                                && let Err(error) = reconnect::remember(key.clone(), reconnect)
                            {
                                log::warn!("could not save reconnect credential: {error}");
                            }
                            if let Some(Role::Client(c)) = &mut self.role {
                                // The maps we have, for the host to see whether we have the
                                // battle's (whichever it chooses, as long as we are connected).
                                c.send(&Msg::Maps(data::our_maps(assets)));
                                if self.auto_watch {
                                    c.send(&Msg::SetSpectator(true));
                                }
                            }
                        }
                        // (A Rematch: the host went back to the lobby while we were still
                        // catching up with the battle.)
                        Msg::Lobby(l) | Msg::Rematch(l) => {
                            if let Some(address) = self.joined.take() {
                                self.learned = Some((address, l.title.clone()));
                            }
                            if !self.told_connected {
                                self.told_connected = true;
                                self.chat.push(chat::What::Connected);
                            }
                            self.state = l;
                            if self.auto_ready {
                                let me = self.me;
                                if let Some(s) = self.state.slots.iter().find(|s| matches!(&s.kind, SlotKind::Human { peer, .. } if *peer == me) && !s.ready) {
                                    let msg = Msg::SetSlot { side: s.side.clone(), color: s.color, ready: true };
                                    if let Some(Role::Client(c)) = &mut self.role {
                                        c.send(&msg);
                                    }
                                }
                            }
                        }
                        Msg::Portraits(list) => {
                            self.portraits = list.into_iter().filter(|(_, png)| !png.is_empty()).collect();
                        }
                        Msg::Chat { from, text } => self.chat.said(&from, &text),
                        Msg::Start(info) => {
                            // No player slot: we watch.
                            let player = info.player_of(self.me);
                            let Some(Role::Client(client)) = self.role.take() else { unreachable!() };
                            let setup = setup_from(&info, player, start_positions(assets, &info.map), &crate::setup::start_spots(assets, &info.map));
                            start = Some(LobbyAction::Start(setup, Session::client(client, player.unwrap_or(u8::MAX)).with_roster(&info).with_rejoin(self.server.clone())));
                            break;
                        }
                        Msg::Resume { start: info, player, upto, history, paused, speed } => {
                            // Back in a game in progress: it replays from the start up to now.
                            let Some(Role::Client(client)) = self.role.take() else { unreachable!() };
                            let setup = setup_from(&info, Some(player), start_positions(assets, &info.map), &crate::setup::start_spots(assets, &info.map));
                            start = Some(LobbyAction::Start(setup, Session::resumed(client, player, upto, history, paused, speed).with_roster(&info).with_rejoin(self.server.clone())));
                            break;
                        }
                        // The host left its lobby (not the network failing).
                        Msg::Bye => {
                            let removed = matches!(&self.role, Some(Role::Client(c)) if c.removed);
                            let why = if removed { ta_ui::i18n::t("The host removed you from the lobby.") } else { ta_ui::i18n::t("The host closed the lobby.") };
                            start = Some(LobbyAction::Closed(why.to_string()));
                            break;
                        }
                        _ => {}
                    }
                }
                if start.is_some() {
                    return start;
                }
                if let Some(Role::Client(c)) = &self.role {
                    self.pings = c.pings().to_vec();
                    if c.removed {
                        return Some(LobbyAction::Closed(ta_ui::i18n::t("The host removed you from the lobby.").to_string()));
                    } else if let Some(reason) = &c.rejected {
                        return Some(LobbyAction::Closed(refusal_text(reason, assets)));
                    } else if !c.is_connected() {
                        return Some(LobbyAction::Closed(ta_ui::i18n::t("The connection to the host was lost.").to_string()));
                    }
                }
                None
            }
        };
        // What changed, told in the chat (in our language, from the lobby we now see).
        if self.state != self.seen {
            // (A map we have no copy of by the name the host's copy gives it.)
            let host_title = (!self.state.map_title.trim().is_empty()).then(|| (self.state.map.clone(), self.state.map_title.clone()));
            let title = |name: &str| match assets.maps.iter().find(|m| m.name == name) {
                Some(m) => m.title.clone(),
                None => host_title.as_ref().filter(|(map, _)| map == name).map_or(name.to_string(), |(_, title)| title.clone()),
            };
            // (Not the first lobby a guest sees: everyone in it was there before them.)
            if !self.seen.slots.is_empty() {
                self.chat.changes(&self.seen, &self.state, &self.removed, title);
            }
            self.seen = self.state.clone();
            // (Removals told once they show: someone of that name leaving later just left.)
            let here = |name: &String| self.state.slots.iter().any(|s| matches!(&s.kind, SlotKind::Human { name: n, .. } if n == name)) || self.state.spectators.iter().any(|(_, n)| n == name);
            self.removed.retain(here);
        }
        action
    }

    /// The program closing: everyone told we left (by the host: that the lobby closed).
    pub fn goodbye(&mut self, timeout: std::time::Duration) {
        match &mut self.role {
            Some(Role::Host(h)) => {
                h.broadcast(&Msg::Bye);
                h.drain(timeout);
            }
            Some(Role::Client(c)) => {
                c.send(&Msg::Bye);
                c.drain(timeout);
            }
            None => {}
        }
    }

    /// Host: takes a person out of the lobby (they are told so).
    fn remove(&mut self, peer: u8) {
        let name = self.state.slots.iter().find_map(|s| match &s.kind {
            SlotKind::Human { peer: p, name } if *p == peer => Some(name.clone()),
            _ => None,
        }).or_else(|| self.state.spectators.iter().find(|(p, _)| *p == peer).map(|(_, n)| n.clone()));
        if let (Some(name), Some(Role::Host(h))) = (name, &mut self.role) {
            h.send_to(peer, &Msg::Removed);
            h.kick(peer);
            self.removed.push(name);
        }
    }

    /// Host: the countdown begins (or is called off).
    fn countdown(&mut self, on: bool) {
        self.state.countdown = on.then_some(COUNTDOWN);
        self.countdown_at = on.then(std::time::Instant::now);
        self.broadcast_lobby();
    }

    /// Host: freeze the lobby into a start message and begin.
    fn start(&mut self, max: usize, assets: &Assets) -> LobbyAction {
        let Some(Role::Host(mut host)) = self.role.take() else { return LobbyAction::None };
        self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let slots: Vec<LobbySlot> = self.state.slots.iter().take(max).cloned().collect();
        // Players whose slots the map has no room for watch, too.
        let mut spectators = self.state.spectators.clone();
        for s in self.state.slots.iter().skip(max) {
            if let SlotKind::Human { peer, name } = &s.kind {
                spectators.push((*peer, name.clone()));
            }
        }
        let info = StartInfo { map: self.state.map.clone(), seed: self.seed, slots, rules: self.state.rules.clone(), spectators };
        host.accepting = false;
        host.broadcast(&Msg::Start(info.clone()));
        let setup = setup_from(&info, info.player_of(0), max, &crate::setup::start_spots(assets, &info.map));
        // (A player coming back to the battle must have the host's copy of its map: the lobby's
        // check does not see them again.)
        let title = self.titles.get(&info.map).cloned().unwrap_or_else(|| info.map.clone());
        let digest = self.own_maps.get(&info.map.to_ascii_lowercase()).copied();
        LobbyAction::Start(setup, Session::host(host, &info).with_lobby(&self.state).with_map(&title, digest))
    }
}

/// Seconds the battle counts down from once the host starts it.
const COUNTDOWN: u8 = 5;

/// Where players on this computer's network reach a lobby on `port`: the address the computer
/// goes out on. (Connecting a UDP socket sends nothing: the system only picks the way out,
/// here towards a documentation address no one owns.)
fn local_address(port: u16) -> Option<String> {
    let socket = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("192.0.2.1", 9)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then(|| format!("{ip}:{port}"))
}

/// Player slots the lobby offers: the chosen map's start positions.
fn max_slots(state: &Lobby, assets: &Assets) -> usize {
    start_positions(assets, &state.map)
}

fn start_positions(assets: &Assets, map: &str) -> usize {
    assets.maps.iter().find(|m| m.name == map).map_or(2, |m| m.start_positions.clamp(2, MAX_SLOTS))
}

/// Seats a person: the first open slot, a new one while the map has room, else the
/// spectators' bench.
fn place_player(state: &mut Lobby, slot: LobbySlot, max: usize) {
    match state.slots.iter().take(max).position(|s| s.kind == SlotKind::Open) {
        Some(i) => state.slots[i] = LobbySlot { side: state.slots[i].side.clone(), ..slot },
        None if state.slots.len() < max => state.slots.push(slot),
        None => {
            if let SlotKind::Human { peer, name } = slot.kind {
                state.spectators.push((peer, name));
            }
        }
    }
}

/// Apply a guest's choices in host order. Two clients may request the same colour
/// from the same old lobby snapshot; only the first can take it.
fn set_slot(state: &mut Lobby, peer: u8, side: String, color: u8, ready: bool, sides: &[String]) -> bool {
    let Some(index) = state.slots.iter().position(|s| matches!(&s.kind, SlotKind::Human { peer: p, .. } if *p == peer)) else { return false };
    let color_free = color < 10 && !state.slots.iter().enumerate().any(|(i, s)| i != index && s.kind.plays() && s.color == color);
    let slot = &mut state.slots[index];
    let side = if sides.contains(&side) { side } else { slot.side.clone() };
    let color = if color_free { color } else { slot.color };
    let changed = (slot.side.as_str(), slot.color, slot.ready) != (side.as_str(), color, ready);
    (slot.side, slot.color, slot.ready) = (side, color, ready);
    changed
}

/// Host: what sits at a seat nobody holds (open, closed, a computer of a difficulty); true if it
/// changed. A computer is ready at once, in a colour of its own: an open seat's colour is
/// nobody's, so a player may have taken it meanwhile.
fn set_seat_kind(state: &mut Lobby, seat: usize, kind: SlotKind) -> bool {
    let Some(slot) = state.slots.get(seat) else { return false };
    if slot.kind == kind || matches!(slot.kind, SlotKind::Human { .. }) || matches!(kind, SlotKind::Human { .. }) {
        return false;
    }
    let taken = state.slots.iter().enumerate().any(|(i, s)| i != seat && s.kind.plays() && s.color == slot.color);
    // (Before the seat plays: its own colour is not counted as taken.)
    let color = if kind.plays() && taken { first_free_color(state) } else { slot.color };
    let slot = &mut state.slots[seat];
    slot.kind = kind;
    slot.ready = true;
    slot.color = color;
    true
}

/// Seats a person (a player elsewhere, or watching) at an open seat within the map's `max`,
/// with their side and colour and the seat's team; true if they moved.
fn take_seat(state: &mut Lobby, peer: u8, seat: usize, max: usize) -> bool {
    if seat >= max.min(state.slots.len()) || state.slots[seat].kind != SlotKind::Open {
        return false;
    }
    let held = state.slots.iter().position(|s| matches!(&s.kind, SlotKind::Human { peer: p, .. } if *p == peer));
    let (name, side, color, ready) = match held {
        Some(i) => {
            let s = &mut state.slots[i];
            let SlotKind::Human { name, .. } = std::mem::replace(&mut s.kind, SlotKind::Open) else { return false };
            let was = (name, s.side.clone(), s.color, s.ready && peer == 0);
            s.ready = false;
            was
        }
        None => match state.spectators.iter().position(|(p, _)| *p == peer) {
            Some(i) => {
                let (_, name) = state.spectators.remove(i);
                (name, state.slots[seat].side.clone(), first_free_color(state), peer == 0)
            }
            None => return false,
        },
    };
    let s = &mut state.slots[seat];
    s.kind = SlotKind::Human { peer, name };
    s.side = side;
    s.color = color;
    s.ready = ready;
    true
}

/// Opens the slot a peer held; returns their name.
fn vacate(state: &mut Lobby, peer: u8) -> Option<String> {
    let s = state.slots.iter_mut().find(|s| matches!(&s.kind, SlotKind::Human { peer: p, .. } if *p == peer))?;
    let SlotKind::Human { name, .. } = std::mem::replace(&mut s.kind, SlotKind::Open) else { return None };
    s.ready = false;
    Some(name)
}

/// Moves a peer between the players and the spectators; true if anything changed.
fn set_spectator(state: &mut Lobby, peer: u8, watch: bool, max: usize, sides: &[String]) -> bool {
    let spectating = state.spectators.iter().position(|(p, _)| *p == peer);
    match (watch, spectating) {
        (true, None) => match vacate(state, peer) {
            Some(name) => {
                state.spectators.push((peer, name));
                true
            }
            None => false,
        },
        (false, Some(i)) => {
            // Only when there is a seat; otherwise they stay on the bench.
            let has_seat = state.slots.iter().take(max).any(|s| s.kind == SlotKind::Open) || state.slots.len() < max;
            if !has_seat {
                return false;
            }
            let (peer, name) = state.spectators.remove(i);
            let slot = LobbySlot { kind: SlotKind::Human { peer, name }, side: seat_side(sides, state.slots.len()), color: first_free_color(state), ready: peer == 0, team: first_free_team(state) };
            place_player(state, slot, max);
            true
        }
        _ => false,
    }
}

/// The lowest team nobody is on yet (new players start on their own).
fn first_free_team(state: &Lobby) -> u8 {
    (0..10u8).find(|t| !state.slots.iter().any(|s| s.kind.plays() && s.team == *t)).unwrap_or(0)
}

fn first_free_color(l: &Lobby) -> u8 {
    (0..10u8).find(|c| !l.slots.iter().any(|s| s.kind.plays() && s.color == *c)).unwrap_or(0)
}

/// The game setup every peer derives from the host's start message (`local` None: we
/// watch; `available`: the map's start positions).
fn setup_from(info: &StartInfo, local: Option<u8>, available: usize, spots: &[(f32, f32)]) -> GameSetup {
    let mut players: Vec<SlotSetup> = info
        .slots
        .iter()
        .filter(|s| s.kind.plays())
        .enumerate()
        .map(|(i, s)| SlotSetup {
            name: match &s.kind {
                SlotKind::Human { name, .. } => name.clone(),
                _ => String::new(),
            },
            side: s.side.clone(),
            color: s.color,
            controller: match s.kind {
                SlotKind::Ai { difficulty: 0 } => Controller::Ai(ta_ai::Difficulty::Easy),
                SlotKind::Ai { difficulty: 2 } => Controller::Ai(ta_ai::Difficulty::Hard),
                SlotKind::Ai { .. } => Controller::Ai(ta_ai::Difficulty::Normal),
                _ => Controller::Human,
            },
            start: i,
            team: s.team,
        })
        .collect();
    crate::setup::assign_starts(&mut players, available, info.seed, info.rules.random_starts, spots);
    GameSetup { map: info.map.clone(), players, local_player: local, seed: info.seed, mission: None, rules: info.rules.clone(), conquest: None, scenario: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_assets() -> Assets {
        let vfs = std::sync::Arc::new(ta_vfs::Vfs::new());
        let data = std::sync::Arc::new(ta_data::GameData::load(&vfs));
        let types = std::sync::Arc::new(ta_sim::UnitTypes::compile(&data, &|_| None));
        Assets { vfs, data, types, maps: Vec::new(), selectable_maps: Vec::new(), campaigns: Vec::new(),
            data_hash: 7, compatibility: Default::default(), mods: Default::default() }
    }

    #[test]
    fn unchanged_guest_choices_do_not_cancel_a_countdown_or_steal_a_colour() {
        use std::time::{Duration, Instant};
        let assets = empty_assets();
        let host = Host::bind(0, 7).unwrap();
        let mut client = Client::connect(([127, 0, 0, 1], host.port).into(), "Guest", 7).unwrap();
        let mut lobby = NetLobby::hosting(host, "Host", &assets, "Test");
        let deadline = Instant::now() + Duration::from_secs(5);
        while client.peer.is_none() {
            lobby.pump(&assets);
            client.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let peer = client.peer.unwrap();
        let index = lobby.state.slots.iter().position(|s| matches!(&s.kind, SlotKind::Human { peer: p, .. } if *p == peer)).unwrap();
        lobby.state.slots[index].ready = true;
        let guest = lobby.state.slots[index].clone();
        lobby.countdown(true);
        // A repeat of choices already accepted, followed by a conflicting colour claim.
        client.send(&Msg::SetSlot { side: guest.side.clone(), color: guest.color, ready: true });
        client.send(&Msg::SetSlot { side: guest.side.clone(), color: lobby.state.slots[0].color, ready: true });
        client.send(&Msg::Chat { from: String::new(), text: "after choices".into() });
        while !lobby.chat.lines.iter().any(|line| matches!(&line.what, chat::What::Said { text, .. } if text == "after choices")) {
            lobby.pump(&assets);
            client.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(lobby.state.slots[index], guest, "the host keeps the guest's available colour");
        assert_eq!(lobby.state.countdown, Some(COUNTDOWN), "no real change means no cancellation");
    }

    /// A folder of game data for a test, gone with it.
    struct Folder(std::path::PathBuf);

    impl Folder {
        fn new(tag: &str) -> Folder {
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            Folder(std::env::temp_dir().join(format!("ta-lobby-{tag}-{}-{nanos}", std::process::id())))
        }

        fn put(&self, path: &str, text: &str) -> &Folder {
            let file = self.0.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
            self
        }

        /// A map of two start positions, `title` its name, `terrain` its contents.
        fn map(&self, stem: &str, title: &str, terrain: &str) -> &Folder {
            self.put(&format!("maps/{stem}.tnt"), terrain);
            self.put(&format!("maps/{stem}.ota"), &format!("[GlobalHeader]{{missionname={title};numplayers=2;[Schema 0]{{[specials]{{[special0]{{specialwhat=StartPos1;XPos=100;ZPos=100;}}[special1]{{specialwhat=StartPos2;XPos=900;ZPos=900;}}}}}}}}"))
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A computer's game data: `layers` (folder, layer) mounted in order, `mods` those of them
    /// that are mods (with a manifest).
    fn assets_of(layers: &[(&std::path::Path, &str)], mods: Vec<ta_vfs::mods::ModInfo>) -> Assets {
        let mut vfs = ta_vfs::Vfs::new();
        for (dir, layer) in layers {
            vfs.mount_mod_dir(dir, layer).unwrap();
        }
        let vfs = std::sync::Arc::new(vfs);
        let data = std::sync::Arc::new(ta_data::GameData::load(&vfs));
        let types = std::sync::Arc::new(ta_sim::UnitTypes::compile(&data, &|_| None));
        let compatibility = ta_data::Compatibility::load(&vfs);
        let maps = crate::setup::MapEntry::catalogue(&vfs);
        Assets { vfs, data, types, maps: maps.clone(), selectable_maps: maps, campaigns: Vec::new(), data_hash: compatibility.global(), compatibility, mods: crate::record_mods::InPlay::new(mods) }
    }

    /// The install every computer of a test has: a unit and two maps.
    fn install() -> Folder {
        let base = Folder::new("base");
        base.put("units/armcom.fbi", "[UNITINFO]{UnitName=ARMCOM;Name=Commander;}").map("battle", "Battle", "battle terrain").map("edited", "Edited", "edited terrain");
        base
    }

    /// Both lobbies read the network until `done` (at most five seconds).
    fn settle(host: &mut NetLobby, host_assets: &Assets, guest: &mut NetLobby, guest_assets: &Assets, done: impl Fn(&NetLobby, &NetLobby) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done(host, guest) {
            assert!(std::time::Instant::now() < deadline, "the lobbies never got there");
            assert!(host.pump(host_assets).is_none() && guest.pump(guest_assets).is_none(), "nobody leaves or starts");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// A map pack on one side only keeps nobody out (the network's hash leaves the maps out), and
    /// the lobby tells, host and guest alike, whether the guest has the battle's map: the same,
    /// none (the host's map pack), or another version of it. The battle waits for it.
    #[test]
    fn a_map_pack_on_one_side_joins_and_the_lobby_tells_who_has_the_map() {
        let base = install();
        let pack = Folder::new("pack");
        pack.map("pack map", "Pack Map", "the pack's terrain");
        let edited = Folder::new("edited");
        edited.map("edited", "Edited", "another version of its terrain");
        let host_assets = assets_of(&[(&base.0, "base"), (&pack.0, "map-pack-1f")], Vec::new());
        let guest_assets = assets_of(&[(&base.0, "base"), (&edited.0, "map-pack-2e")], Vec::new());
        assert_eq!(host_assets.network_hash(), guest_assets.network_hash(), "maps are not the network's data");
        assert_ne!(host_assets.data_hash, guest_assets.data_hash, "records' hash still tells them apart");
        let transport = bind_host(0, &host_assets).unwrap();
        let port = transport.port;
        let mut host = NetLobby::hosting(transport, "Host", &host_assets, "Test");
        assert_eq!(host.state.map, "battle");
        let client = Client::connect(([127, 0, 0, 1], port).into(), "Guest", guest_assets.network_hash()).unwrap();
        let mut guest = NetLobby { me: u8::MAX, ..NetLobby::new(Role::Client(client), "Guest", None) };
        settle(&mut host, &host_assets, &mut guest, &guest_assets, |h, g| g.me != u8::MAX && h.state.map_status_of(g.me) != MapStatus::Checking && g.state.map_status_of(g.me) != MapStatus::Checking);
        let peer = guest.me;
        assert_eq!(host.state.map_status_of(peer), MapStatus::Ready);
        assert!(host.state.everyone_has_the_map());
        // The guest is ready; the host chooses its pack's map, then the map the guest has
        // another version of, then one they both have.
        let seat = host.state.slots.iter().position(|s| matches!(&s.kind, SlotKind::Human { peer: p, .. } if *p == peer)).unwrap();
        host.state.slots[seat].ready = true;
        for (map, status) in [("pack map", MapStatus::Missing), ("edited", MapStatus::Other), ("battle", MapStatus::Ready)] {
            host.state.map = map.into();
            host.changed();
            settle(&mut host, &host_assets, &mut guest, &guest_assets, |_, g| g.state.map == map);
            assert_eq!(host.state.map_status_of(peer), status, "{map}: the host's view");
            assert_eq!(guest.state.map_status_of(peer), status, "{map}: the guest's own");
            assert_eq!(guest.map_title(&guest_assets), host.map_title(&host_assets), "{map}: by its name, a copy or none");
            assert_eq!(host.state.everyone_has_the_map(), status == MapStatus::Ready, "{map}");
            if status != MapStatus::Ready {
                assert!(host.try_auto_start(2, &host_assets).is_none(), "{map}: the battle waits");
            }
        }
        let told: Vec<&chat::What> = guest.chat.lines.iter().map(|l| &l.what).collect();
        assert!(told.contains(&&chat::What::NoMap("Guest".into())) && told.contains(&&chat::What::OtherMap("Guest".into())), "{told:?}");
        assert!(matches!(host.try_auto_start(2, &host_assets), Some(LobbyAction::Start(..))), "everyone has it: the battle starts");
    }

    /// A mod on one side only: the player is turned away, told which mod and what to do (the
    /// host's: not installed here; one of theirs the host lacks: turn it off).
    #[test]
    fn a_mod_on_one_side_is_refused_naming_the_mod() {
        let base = install();
        let mods = Folder::new("mods");
        mods.put("escalation/mod.toml", "id = \"escalation\"\nname = \"TA: Escalation\"\nversion = \"10.2\"\n").put("escalation/units/escpw.fbi", "[UNITINFO]{UnitName=ESCPW;}");
        let (found, problems) = ta_vfs::mods::scan(&mods.0);
        assert!(problems.is_empty(), "{problems:?}");
        let with = assets_of(&[(&base.0, "base"), (&mods.0.join("escalation"), "escalation")], found);
        let without = assets_of(&[(&base.0, "base")], Vec::new());
        assert_ne!(with.network_hash(), without.network_hash());
        for (host_assets, guest_assets, expected) in [
            (&with, &without, "TA: Escalation 10.2 is not installed here, and the host plays with it"),
            (&without, &with, "Turn off TA: Escalation 10.2 in the launcher's Mods page: the host plays without it."),
        ] {
            let mut host = bind_host(0, host_assets).unwrap();
            let mut client = Client::connect(([127, 0, 0, 1], host.port).into(), "Guest", guest_assets.network_hash()).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while client.rejected.is_none() {
                assert!(std::time::Instant::now() < deadline, "no answer");
                host.poll();
                client.poll();
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            let Some(ta_net::Refusal::Data { mods }) = &client.rejected else { panic!("{:?}", client.rejected) };
            let text = data::refusal(mods, &data::our_mods(guest_assets), &[]);
            assert!(text.contains(expected), "{text}");
        }
    }

    #[test]
    fn concurrent_colour_choices_are_resolved_by_the_host() {
        let mut lobby = Lobby { slots: vec![human(0, "Host"), human(1, "Guest"), human(2, "Other")], ..Default::default() };
        for (i, slot) in lobby.slots.iter_mut().enumerate() { slot.color = i as u8; }
        let sides = vec!["ARM".to_string(), "CORE".to_string()];
        assert!(set_slot(&mut lobby, 1, "CORE".into(), 3, true, &sides));
        assert!(!set_slot(&mut lobby, 2, "ARM".into(), 3, true, &sides));
        assert_eq!(lobby.slots.iter().map(|s| s.color).collect::<Vec<_>>(), [0, 3, 2]);
        assert!(!set_slot(&mut lobby, 2, "INVALID".into(), 255, true, &sides));
        assert!(!set_slot(&mut lobby, 9, "ARM".into(), 4, true, &sides));
    }

    #[test]
    fn client_map_visibility_cannot_change_network_start_positions() {
        let vfs = std::sync::Arc::new(ta_vfs::Vfs::new());
        let data = std::sync::Arc::new(ta_data::GameData::load(&vfs));
        let types = std::sync::Arc::new(ta_sim::UnitTypes::compile(&data, &|_| None));
        let map = crate::setup::MapEntry { name: "ten-starts".into(), title: "Ten starts".into(), description: String::new(), planet: String::new(), size: String::new(), start_positions: 10, info: Default::default() };
        let host = Assets { vfs, data, types, maps: vec![map.clone()], selectable_maps: vec![map], campaigns: Vec::new(), data_hash: 7, compatibility: Default::default(), mods: Default::default() };
        let mut client = host.clone();
        client.selectable_maps.clear(); // this client deactivated the host's chosen map
        let info = StartInfo { map: "ten-starts".into(), seed: 123, slots: (0..4).map(|_| slot(SlotKind::Ai { difficulty: 1 })).collect(),
            rules: ta_sim::GameRules { random_starts: true, ..Default::default() }, spectators: Vec::new() };
        assert_eq!(start_positions(&host, &info.map), 10);
        assert_eq!(start_positions(&client, &info.map), 10);
        let host_setup = setup_from(&info, Some(0), start_positions(&host, &info.map), &[]);
        let client_setup = setup_from(&info, Some(1), start_positions(&client, &info.map), &[]);
        assert_eq!(host_setup.players.iter().map(|p| p.start).collect::<Vec<_>>(), client_setup.players.iter().map(|p| p.start).collect::<Vec<_>>());
    }

    fn slot(kind: SlotKind) -> LobbySlot {
        LobbySlot { kind, side: "ARM".into(), color: 0, ready: true, team: 0 }
    }

    fn human(peer: u8, name: &str) -> LobbySlot {
        slot(SlotKind::Human { peer, name: name.into() })
    }

    #[test]
    fn a_person_takes_an_open_seat_only() {
        let mut lobby = Lobby {
            slots: vec![human(0, "Valkyrie"), human(1, "Ironside"), slot(SlotKind::Open), slot(SlotKind::Closed), slot(SlotKind::Open)],
            spectators: vec![(2, "Hex".into())],
            ..Default::default()
        };
        // From a seat to an open one: the old seat opens.
        assert!(take_seat(&mut lobby, 1, 2, 5));
        assert_eq!(lobby.slots[1].kind, SlotKind::Open);
        assert!(matches!(&lobby.slots[2].kind, SlotKind::Human { peer: 1, .. }));
        // Not a closed seat, a taken one, or one past the map's room.
        assert!(!take_seat(&mut lobby, 1, 3, 5));
        assert!(!take_seat(&mut lobby, 1, 0, 5));
        assert!(!take_seat(&mut lobby, 1, 4, 4));
        // A spectator sits down; a stranger cannot.
        assert!(take_seat(&mut lobby, 2, 4, 5));
        assert!(lobby.spectators.is_empty());
        assert!(matches!(&lobby.slots[4].kind, SlotKind::Human { peer: 2, .. }));
        assert!(!take_seat(&mut lobby, 9, 1, 5));
        // Someone joining takes the first open seat, never a closed one.
        lobby.slots[1].kind = SlotKind::Closed;
        place_player(&mut lobby, human(3, "Nova"), 5);
        assert_eq!(lobby.slots[1].kind, SlotKind::Closed);
        assert_eq!(lobby.spectators, [(3, "Nova".into())], "no open seat left: Nova watches");
    }

    /// An open seat keeps the colour it was added with, which is nobody's: a player may take it
    /// meanwhile. A computer put there takes a colour of its own.
    #[test]
    fn a_computer_in_an_open_seat_takes_a_colour_nobody_has() {
        let mut lobby = Lobby { slots: vec![human(0, "Host"), human(1, "Guest"), slot(SlotKind::Open)], ..Default::default() };
        let sides = vec!["ARM".to_string(), "CORE".to_string()];
        (lobby.slots[0].color, lobby.slots[1].color, lobby.slots[2].color) = (0, 1, 2);
        // The guest takes the open seat's colour (it plays nobody).
        assert!(set_slot(&mut lobby, 1, "ARM".into(), 2, false, &sides));
        assert!(set_seat_kind(&mut lobby, 2, SlotKind::Ai { difficulty: 2 }));
        let colours: Vec<u8> = lobby.slots.iter().map(|s| s.color).collect();
        assert_eq!(colours, [0, 2, 1], "the computer takes the colour the guest left");
        assert!(lobby.slots[2].ready);
        // Another difficulty, or a closed seat, keeps the colour; people's seats are not the host's to fill.
        assert!(set_seat_kind(&mut lobby, 2, SlotKind::Ai { difficulty: 0 }));
        assert!(set_seat_kind(&mut lobby, 2, SlotKind::Closed));
        assert_eq!(lobby.slots[2].color, 1);
        assert!(!set_seat_kind(&mut lobby, 2, SlotKind::Closed), "no change");
        assert!(!set_seat_kind(&mut lobby, 1, SlotKind::Ai { difficulty: 1 }), "a person's seat");
        assert!(!set_seat_kind(&mut lobby, 2, SlotKind::Human { peer: 9, name: "X".into() }));
        assert!(!set_seat_kind(&mut lobby, 7, SlotKind::Open), "no such seat");
    }

    #[test]
    fn closed_seats_take_no_part_in_the_battle() {
        let info = StartInfo { map: "m".into(), seed: 1, slots: vec![human(0, "Valkyrie"), slot(SlotKind::Closed), slot(SlotKind::Ai { difficulty: 1 }), slot(SlotKind::Open)], rules: Default::default(), spectators: Vec::new() };
        let setup = setup_from(&info, info.player_of(0), 4, &[]);
        assert_eq!(setup.players.len(), 2);
        assert_eq!(info.player_of(0), Some(0));
    }

    #[test]
    fn joining_a_smaller_map_does_not_take_a_slot_outside_its_capacity() {
        let mut lobby = Lobby {
            slots: vec![slot(SlotKind::Ai { difficulty: 1 }), slot(SlotKind::Ai { difficulty: 1 }), slot(SlotKind::Open)],
            ..Default::default()
        };
        place_player(&mut lobby, slot(SlotKind::Human { peer: 2, name: "Guest".into() }), 2);
        assert_eq!(lobby.slots[2].kind, SlotKind::Open);
        assert_eq!(lobby.spectators, [(2, "Guest".into())]);
        let sides = ["ARM".to_string(), "CORE".to_string()];
        assert!(!set_spectator(&mut lobby, 2, false, 2, &sides), "an unavailable extra slot must not offer a place to play");
        assert_eq!(lobby.spectators, [(2, "Guest".into())]);

        lobby.slots[1].kind = SlotKind::Open;
        assert!(set_spectator(&mut lobby, 2, false, 2, &sides));
        assert!(lobby.spectators.is_empty());
        assert!(matches!(lobby.slots[1].kind, SlotKind::Human { peer: 2, .. }));
        assert_eq!(lobby.slots[2].kind, SlotKind::Open);
    }

    #[test]
    fn new_seats_are_dealt_the_sides_the_game_data_has() {
        // A mod's three sides, in turn; TA's own two when none were read.
        let sides = ["TAL".to_string(), "GOK".to_string(), "ARM".to_string()];
        assert_eq!((0..4).map(|seat| seat_side(&sides, seat)).collect::<Vec<_>>(), ["TAL", "GOK", "ARM", "TAL"]);
        assert_eq!((0..2).map(|seat| seat_side(&[], seat)).collect::<Vec<_>>(), ["ARM", "CORE"]);
        // Someone watching who takes a new seat: its side is the mod's, not CORE.
        let mut lobby = Lobby { slots: vec![human(0, "Host")], spectators: vec![(2, "Guest".into())], ..Default::default() };
        assert!(set_spectator(&mut lobby, 2, false, 4, &sides));
        assert_eq!((&lobby.slots[1].kind, lobby.slots[1].side.as_str()), (&SlotKind::Human { peer: 2, name: "Guest".into() }, "GOK"));
    }
}
