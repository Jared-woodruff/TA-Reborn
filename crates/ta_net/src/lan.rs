//! Finding games on the local network: a hosting lobby announces itself once a second by UDP
//! broadcast (and to this computer), and the connect page lists what it hears, so nobody has
//! to type an address on a LAN.

use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use bincode::Options;

/// The UDP port lobbies announce themselves on.
pub const LAN_PORT: u16 = 7891;
/// Start of every announcement (and a version of their layout: 2 numbers the lobby, names
/// the battle, counts its seats and says whether it has a password).
const MAGIC: [u8; 4] = *b"TAR2";
const EVERY: Duration = Duration::from_secs(1);
/// A game not heard from for this long is gone.
const FORGET: Duration = Duration::from_secs(4);
const MAX_DATAGRAMS_PER_POLL: usize = 128;
const MAX_GAMES: usize = 256;

/// What a hosting lobby tells the network about itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Announcement {
    /// [`crate::PROTOCOL`] and the game data hash: only equal ones can play together.
    pub protocol: u32,
    pub data_hash: u64,
    /// The TCP port the lobby accepts players on.
    pub port: u16,
    pub host: String,
    pub map: String,
    /// Players in the lobby, and places for more.
    pub players: u8,
    pub open: u8,
    /// The battle's name, its seats (the map's start positions) and whether joining takes a
    /// password.
    pub title: String,
    pub seats: u8,
    pub locked: bool,
}

impl Announcement {
    /// The announcement after the lobby's number (`id`: one lobby heard two ways, over the
    /// network and on this computer, is one game).
    fn encode(&self, id: u64) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend(bincode::serialize(&(id, self)).unwrap_or_default());
        out
    }

    fn decode(bytes: &[u8]) -> Option<(u64, Announcement)> {
        let body = bytes.strip_prefix(&MAGIC)?;
        let (id, a): (u64, Announcement) = bincode::DefaultOptions::new().with_fixint_encoding().with_limit(body.len() as u64).reject_trailing_bytes().deserialize(body).ok()?;
        // Names from the network are shown on screen: keep them short and printable.
        Some((id, Announcement { host: crate::clean(&a.host, crate::NAME_MAX), map: crate::clean(&a.map, 64), title: crate::clean(&a.title, 64), ..a }))
    }
}

/// The hosting side: sends the lobby's announcement at most once a second.
pub struct Beacon {
    socket: UdpSocket,
    last: Option<Instant>,
    /// The lobby's number, drawn at random.
    id: u64,
}

impl Beacon {
    pub fn new() -> io::Result<Beacon> {
        use std::hash::{BuildHasher, Hasher};
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        socket.set_broadcast(true)?;
        let id = std::collections::hash_map::RandomState::new().build_hasher().finish();
        Ok(Beacon { socket, last: None, id })
    }

    /// Announces the lobby (to the local network and this computer) if a second has passed.
    pub fn announce(&mut self, a: &Announcement) {
        if self.last.is_some_and(|t| t.elapsed() < EVERY) {
            return;
        }
        self.last = Some(Instant::now());
        let bytes = a.encode(self.id);
        for to in [Ipv4Addr::BROADCAST, Ipv4Addr::LOCALHOST] {
            // A network without broadcast (or no network) only loses the announcement.
            let _ = self.socket.send_to(&bytes, SocketAddrV4::new(to, LAN_PORT));
        }
    }
}

/// A game heard on the network.
#[derive(Clone, Debug)]
pub struct LanGame {
    /// Where to connect (the sender's address and the announced port).
    pub address: SocketAddr,
    pub info: Announcement,
    id: u64,
    seen: Instant,
}

/// The joining side: listens for announcements and keeps the games heard lately.
pub struct LanBrowser {
    socket: UdpSocket,
    games: Vec<LanGame>,
}

impl LanBrowser {
    /// Listens on [`LAN_PORT`]; fails when another program on this computer already does
    /// (e.g. a second copy of the game at its connect page).
    pub fn new() -> io::Result<LanBrowser> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, LAN_PORT))?;
        socket.set_nonblocking(true)?;
        Ok(LanBrowser { socket, games: Vec::new() })
    }

    /// Reads what has arrived and returns the games heard in the last few seconds, by host
    /// name.
    pub fn poll(&mut self) -> &[LanGame] {
        let mut buf = [0u8; 1024];
        self.games.retain(|g| g.seen.elapsed() < FORGET);
        for _ in 0..MAX_DATAGRAMS_PER_POLL {
            let Ok((n, from)) = self.socket.recv_from(&mut buf) else { break };
            let Some((id, info)) = Announcement::decode(&buf[..n]) else { continue };
            let address = SocketAddr::new(from.ip(), info.port);
            let now = Instant::now();
            let has_room = self.games.len() < MAX_GAMES;
            match self.games.iter_mut().find(|g| g.id == id) {
                Some(g) => {
                    // (A lobby on this computer is heard over the network too: joined by
                    // this computer's own address.)
                    if address.ip().is_loopback() || !g.address.ip().is_loopback() {
                        g.address = address;
                    }
                    g.info = info;
                    g.seen = now;
                }
                None if has_room => self.games.push(LanGame { address, info, id, seen: now }),
                None => {},
            }
        }
        self.games.sort_by(|a, b| a.info.host.to_lowercase().cmp(&b.info.host.to_lowercase()).then(a.address.cmp(&b.address)));
        &self.games
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tests listening on the LAN port take turns (only one can hold it).
    static PORT: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn announcement() -> Announcement {
        Announcement { protocol: crate::PROTOCOL, data_hash: 7, port: 7890, host: "Ann".into(), map: "Coast To Coast".into(), players: 1, open: 3, title: "Ann's battle".into(), seats: 4, locked: false }
    }

    #[test]
    fn announcements_round_trip_and_reject_strangers() {
        let a = announcement();
        assert_eq!(Announcement::decode(&a.encode(42)), Some((42, a)));
        assert_eq!(Announcement::decode(b"HELLO"), None);
        assert_eq!(Announcement::decode(b"TAR1\\x01"), None);
    }

    /// One lobby heard over the network and on this computer is listed once, joined by this
    /// computer's address; another lobby is listed beside it.
    #[test]
    fn a_lobby_heard_two_ways_is_one_game() {
        let _turn = PORT.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(mut browser) = LanBrowser::new() else {
            eprintln!("LAN port busy; skipping");
            return;
        };
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let to = SocketAddrV4::new(Ipv4Addr::LOCALHOST, LAN_PORT);
        socket.send_to(&announcement().encode(7), to).unwrap();
        socket.send_to(&announcement().encode(7), to).unwrap();
        socket.send_to(&Announcement { host: "Bo".into(), ..announcement() }.encode(8), to).unwrap();
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(3) {
            let games = browser.poll();
            if games.len() >= 2 {
                assert_eq!(games.len(), 2, "{games:?}");
                assert!(games.iter().all(|g| g.address.ip().is_loopback()));
                assert_eq!(games[0].info.host, "Ann");
                assert_eq!(games[1].info.host, "Bo");
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the announcements never arrived");
    }

    /// A lobby on this computer shows up in the browser (skipped when the LAN port is taken,
    /// e.g. by a running copy of the game).
    #[test]
    fn a_lobby_on_this_computer_is_heard() {
        let _turn = PORT.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(mut browser) = LanBrowser::new() else {
            eprintln!("LAN port busy; skipping");
            return;
        };
        let mut beacon = Beacon::new().unwrap();
        beacon.announce(&announcement());
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(3) {
            if let Some(g) = browser.poll().first() {
                assert_eq!(g.info, announcement());
                assert_eq!(g.address.port(), 7890);
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the announcement never arrived");
    }
}
