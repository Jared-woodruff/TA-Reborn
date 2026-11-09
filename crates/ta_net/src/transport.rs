//! TCP transport: length-prefixed bincode frames, with reader/writer workers per connection.
//! The game thread only accepts ready connections and accesses bounded outbound queues.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use bincode::Options;

use crate::{Msg, NetMod, PROTOCOL, ReconnectToken, Refusal};

/// Largest frame accepted: a tick's bundle of a few thousand orders is far smaller, and a
/// returning player's history comes in parts of at most [`crate::BATCH_BYTES`].
pub const MAX_FRAME: usize = 4 << 20;
/// The largest frame a connection may send before it is welcomed (its hello and password).
const MAX_HELLO_FRAME: usize = 64 << 10;
/// Includes the writer's in-flight frame. A peer cannot accumulate an unlimited backlog.
const MAX_QUEUED_BYTES: usize = 128 << 20;
const MAX_QUEUED_FRAMES: usize = 4096;
const MAX_INCOMING_FRAMES: usize = 2048;
const MAX_POLL_MESSAGES: usize = 256;
const MAX_POLL_BYTES: usize = 8 << 20;
pub(crate) const MAX_MESSAGE_MEMORY: usize = 32 << 20;
const MAX_INCOMING_BYTES: usize = 256 << 20;
const MAX_CONNECTIONS: usize = 32;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
use crate::MAX_HISTORY_BYTES;

/// The charge includes decoded storage, rather than just the sometimes much smaller wire
/// representation (for example a vector of tiny enum variants).
struct Incoming {
    message: Option<Msg>,
    bytes: usize,
    incoming: Arc<AtomicUsize>,
}

impl Drop for Incoming {
    fn drop(&mut self) { self.incoming.fetch_sub(self.bytes, Ordering::Relaxed); }
}

#[cfg(test)]
impl Incoming {
    fn test(message: Option<Msg>) -> Self { Self { message, bytes: 0, incoming: Arc::new(AtomicUsize::new(0)) } }
}

fn decode(bytes: &[u8]) -> bincode::Result<Msg> {
    bincode::DefaultOptions::new().with_fixint_encoding().with_limit(bytes.len() as u64).reject_trailing_bytes().deserialize(bytes)
}

fn encode(msg: &Msg) -> Arc<[u8]> {
    let body = bincode::serialize(msg).unwrap_or_default();
    let mut frame = Vec::with_capacity(body.len() + 4);
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(&body);
    frame.into()
}

fn shutdown_socket(stream: &TcpStream) {
    // On macOS, shutting down Both can stop at an already-closed read half
    // without closing writes. Close each half independently, including after EOF.
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let _ = stream.shutdown(std::net::Shutdown::Read);
}

fn write_loop(mut stream: TcpStream, rx: Receiver<Arc<[u8]>>, queued: Arc<AtomicUsize>, failed: Arc<AtomicBool>) {
    while let Ok(frame) = rx.recv() {
        let result = stream.write_all(&frame);
        queued.fetch_sub(frame.len(), Ordering::Relaxed);
        if result.is_err() {
            failed.store(true, Ordering::Release);
            break;
        }
    }
    // Dropping a connection closes the queue: finish its accepted frames (including Bye or
    // Reject) before closing the socket. A stalled writer is bounded by its own timeout.
    shutdown_socket(&stream);
}

fn read_loop(mut stream: TcpStream, tx: SyncSender<Incoming>, incoming: Arc<AtomicUsize>, failed: Arc<AtomicBool>, welcomed: Arc<AtomicBool>) {
    let result = (|| -> io::Result<()> { loop {
        let mut len = [0u8; 4];
        stream.read_exact(&mut len)?;
        let n = u32::from_le_bytes(len) as usize;
        if n > MAX_FRAME || !welcomed.load(Ordering::Acquire) && n > MAX_HELLO_FRAME {
            return Err(io::ErrorKind::InvalidData.into());
        }
        // Reserve a conservative decoding budget before reading. Command enum vectors can
        // be much larger in memory than on the wire. Hosts share this budget across peers.
        let reservation = n.saturating_mul(64).saturating_add(std::mem::size_of::<Msg>()).min(MAX_INCOMING_BYTES);
        incoming.try_update(Ordering::Relaxed, Ordering::Relaxed, |used| used.checked_add(reservation).filter(|&n| n <= MAX_INCOMING_BYTES))
            .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
        let reservation_guard = IncomingReservation { bytes: reservation, incoming: &incoming };
        // Do not reserve the announced size before the sender supplies its bytes. An idle
        // connection can stay open, but a partially supplied frame has a read timeout.
        stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
        let mut buf = Vec::with_capacity(n.min(8192));
        (&mut stream).take(n as u64).read_to_end(&mut buf)?;
        if buf.len() != n { return Err(io::ErrorKind::UnexpectedEof.into()); }
        stream.set_read_timeout(None)?;
        let msg = decode(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let bytes = crate::message_memory(&msg).saturating_add(n);
        if bytes > reservation || bytes > MAX_MESSAGE_MEMORY { return Err(io::ErrorKind::OutOfMemory.into()); }
        incoming.fetch_sub(reservation - bytes, Ordering::Relaxed);
        std::mem::forget(reservation_guard);
        if tx.try_send(Incoming { message: Some(msg), bytes, incoming: incoming.clone() }).is_err() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
    } })();
    if result.is_err() {
        failed.store(true, Ordering::Release);
        let _ = tx.try_send(Incoming { message: None, bytes: 0, incoming });
        shutdown_socket(&stream);
    }
}

struct IncomingReservation<'a> { bytes: usize, incoming: &'a AtomicUsize }
impl Drop for IncomingReservation<'_> {
    fn drop(&mut self) { self.incoming.fetch_sub(self.bytes, Ordering::Relaxed); }
}

/// One TCP connection.
struct Connection {
    socket: TcpStream,
    outgoing: Option<SyncSender<Arc<[u8]>>>,
    queued: Arc<AtomicUsize>,
    failed: Arc<AtomicBool>,
    rx: Receiver<Incoming>,
    #[cfg(test)]
    incoming: Arc<AtomicUsize>,
    pending: Option<Incoming>,
    open: bool,
    /// Said hello and was let in (until then its frames are small).
    welcomed: Arc<AtomicBool>,
}

impl Connection {
    /// A connection to a host (whose frames may be large from the first).
    fn new(stream: TcpStream) -> io::Result<Connection> {
        Self::with_budget(stream, Arc::new(AtomicUsize::new(0)), true)
    }

    /// `welcomed`: whether its frames may be large before it is let in (a host's peers: not).
    fn with_budget(stream: TcpStream, incoming: Arc<AtomicUsize>, welcomed: bool) -> io::Result<Connection> {
        // Accepted sockets may inherit the listener's mode on some platforms. Only the
        // worker threads use them, and read_exact/write_all require blocking sockets here.
        stream.set_nonblocking(false)?;
        stream.set_nodelay(true)?;
        // Only the writer worker waits on the socket; this also bounds its lifetime when a
        // connection is dropped while a peer has stopped reading.
        stream.set_write_timeout(Some(Duration::from_secs(10)))?;
        let socket = stream.try_clone()?;
        let reader = stream.try_clone()?;
        let (tx, rx) = mpsc::sync_channel(MAX_INCOMING_FRAMES);
        let (outgoing, writes) = mpsc::sync_channel(MAX_QUEUED_FRAMES);
        let queued = Arc::new(AtomicUsize::new(0));
        let failed = Arc::new(AtomicBool::new(false));
        let (bytes, error) = (queued.clone(), failed.clone());
        thread::Builder::new().name("ta-net-writer".into()).spawn(move || write_loop(stream, writes, bytes, error))?;
        let welcomed = Arc::new(AtomicBool::new(welcomed));
        let (read_bytes, read_error, read_welcomed) = (incoming.clone(), failed.clone(), welcomed.clone());
        if let Err(e) = thread::Builder::new().name("ta-net-reader".into()).spawn(move || read_loop(reader, tx, read_bytes, read_error, read_welcomed)) {
            shutdown_socket(&socket);
            return Err(e);
        }
        Ok(Connection { socket, outgoing: Some(outgoing), queued, failed, rx, #[cfg(test)] incoming, pending: None, open: true, welcomed })
    }

    /// Let in: its frames may be as large as any now.
    fn welcome(&self) {
        self.welcomed.store(true, Ordering::Release);
    }

    fn send_frame(&mut self, frame: &Arc<[u8]>) {
        if self.try_send_frame(frame).is_ok() {
            return;
        }
        // Never make the game thread wait for a peer to consume its backlog.
        log::warn!("disconnecting peer whose outgoing queue is full or closed");
        self.open = false;
        self.failed.store(true, Ordering::Release);
        shutdown_socket(&self.socket);
    }

    /// Transfers that can pause production retry WouldBlock on a later game frame.
    fn try_send_frame(&mut self, frame: &Arc<[u8]>) -> io::Result<()> {
        if !self.open || self.failed.load(Ordering::Acquire) {
            return Err(io::ErrorKind::NotConnected.into());
        }
        if frame.len() > MAX_FRAME + 4 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        self.queued.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(frame.len()).filter(|&n| n <= MAX_QUEUED_BYTES))
            .map_err(|_| io::Error::from(io::ErrorKind::WouldBlock))?;
        let result = self.outgoing.as_ref().map(|tx| tx.try_send(frame.clone()));
        match result {
            Some(Ok(())) => Ok(()),
            result => {
                self.queued.fetch_sub(frame.len(), Ordering::Relaxed);
                Err(match result {
                    Some(Err(mpsc::TrySendError::Full(_))) => io::ErrorKind::WouldBlock,
                    _ => io::ErrorKind::BrokenPipe,
                }.into())
            }
        }
    }

    fn poll(&mut self) -> Vec<Msg> {
        if self.failed.load(Ordering::Acquire) {
            self.open = false;
        }
        let mut out = Vec::new();
        let mut bytes = 0usize;
        for _ in 0..MAX_POLL_MESSAGES {
            let Some(mut m) = self.pending.take().or_else(|| self.rx.try_recv().ok()) else { break };
            if !out.is_empty() && bytes.saturating_add(m.bytes) > MAX_POLL_BYTES {
                self.pending = Some(m);
                break;
            }
            bytes += m.bytes;
            match m.message.take() {
                Some(msg) => out.push(msg),
                None => self.open = false,
            }
        }
        if !self.open {
            self.outgoing.take();
        }
        out
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.outgoing.take();
    }
}

impl Connection {
    /// Everything sent has been written (or the connection failed, and nothing more will be).
    fn written(&self) -> bool {
        self.queued.load(Ordering::Relaxed) == 0 || self.failed.load(Ordering::Acquire)
    }
}

/// Waits, at most `timeout`, until `done`.
fn wait_until(timeout: Duration, done: impl Fn() -> bool) {
    let start = Instant::now();
    while !done() && start.elapsed() < timeout {
        thread::sleep(Duration::from_millis(2));
    }
}

/// A host's refusal as a player is told it: the mods it names fit for the screen, and no more of
/// them than [`crate::MAX_MODS`].
fn cleaned(reason: &Refusal) -> Refusal {
    match reason {
        Refusal::Data { mods } => Refusal::Data { mods: mods.iter().take(crate::MAX_MODS).map(NetMod::cleaned).collect() },
        Refusal::Map { map, other } => Refusal::Map { map: crate::clean(map, crate::MAP_NAME_MAX), other: *other },
        other => other.clone(),
    }
}

/// A connection to a host.
pub struct Client {
    conn: Connection,
    /// Our peer id, once the host has welcomed us.
    pub peer: Option<u8>,
    /// Why the host turned us away, if it did.
    pub rejected: Option<crate::Refusal>,
    /// The parts of a returning player's history received so far (see [`Msg::History`]).
    history: Vec<(u64, Vec<ta_sim::Command>)>,
    /// Messages after a Start/Resume belong to the new session, not its old lobby.
    pending: VecDeque<Msg>,
    reconnect: Option<ReconnectToken>,
    history_bytes: usize,
    in_game: bool,
    /// Everyone's round trip to the host, as it last told us: (peer, milliseconds).
    pings: Vec<(u8, u16)>,
    /// The host took us out of the lobby.
    pub removed: bool,
    /// When anything last came from the host (its pings included).
    heard: Option<Instant>,
}

impl Client {
    /// Connects and introduces ourselves; the host answers with `Welcome` or `Reject`.
    pub fn connect(addr: SocketAddr, name: &str, data_hash: u64) -> io::Result<Client> {
        Self::connect_with_token(addr, name, data_hash, None, "")
    }

    /// Connects with a credential to come back to a battle, and the lobby's password (empty:
    /// none).
    pub fn connect_with_token(addr: SocketAddr, name: &str, data_hash: u64, reconnect: Option<ReconnectToken>, password: &str) -> io::Result<Client> {
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(4))?;
        let mut conn = Connection::new(stream)?;
        conn.send_frame(&encode(&Msg::Hello { name: name.to_string(), protocol: PROTOCOL, data_hash, reconnect, password: password.to_string() }));
        Ok(Client { conn, peer: None, rejected: None, history: Vec::new(), pending: VecDeque::new(), reconnect: None, history_bytes: 0, in_game: false, pings: Vec::new(), removed: false, heard: None })
    }

    /// Waits up to `timeout` for the host's answer to our hello: a welcome (left, with what
    /// follows it, for [`Client::poll`]) or a refusal (kept in `rejected`). Fails when the host
    /// closes the connection or does not answer.
    pub fn wait_for_answer(&mut self, timeout: Duration) -> io::Result<()> {
        let t0 = Instant::now();
        loop {
            self.pending.extend(self.conn.poll());
            match self.pending.front() {
                Some(Msg::Reject { reason }) => {
                    self.rejected = Some(cleaned(reason));
                    self.conn.open = false;
                    self.conn.outgoing.take();
                    self.pending.clear();
                    return Ok(());
                }
                Some(_) => return Ok(()),
                None if !self.is_connected() => return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "the host closed the connection")),
                None if t0.elapsed() >= timeout => return Err(io::Error::new(io::ErrorKind::TimedOut, "the host did not answer")),
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    }

    /// Everyone's round trip to the host, milliseconds, as the host last measured it (ours
    /// among them; the host's own 0).
    /// Waits, at most `timeout`, for what was sent to be written (the program closing just
    /// after a goodbye).
    pub fn drain(&self, timeout: Duration) {
        wait_until(timeout, || self.conn.written());
    }

    /// Coming back to a battle: the last tick of it received so far (None before any, or
    /// once it is all here).
    pub fn history_received(&self) -> Option<u64> {
        self.history.last().map(|(tick, _)| *tick)
    }

    pub fn pings(&self) -> &[(u8, u16)] {
        &self.pings
    }

    /// Save this credential when welcomed to support reconnecting after a process restart.
    pub fn reconnect_token(&self) -> Option<ReconnectToken> {
        self.reconnect
    }

    pub(crate) fn enter_game(&mut self) { self.in_game = true; }

    /// A result-screen session read the next Start just as the player chose the lobby.
    /// Let that lobby receive it again, ahead of all of the next battle's live messages.
    pub(crate) fn defer_start(&mut self, start: crate::StartInfo) {
        self.in_game = false;
        self.pending.push_front(Msg::Start(start));
    }

    pub(crate) fn disconnect(&mut self) {
        self.conn.open = false;
        self.conn.outgoing.take();
        self.pending.clear();
        self.history.clear();
        self.history_bytes = 0;
        shutdown_socket(&self.conn.socket);
    }

    pub fn send(&mut self, msg: &Msg) {
        self.conn.send_frame(&encode(msg));
    }

    /// Messages received since the last call. The parts of a history come back as the one
    /// [`Msg::Resume`] they lead up to. A Start/Resume ends the batch so a lobby can hand
    /// this client to a session without discarding any following ticks or pace messages.
    /// When anything last came from the host (none yet: never).
    pub fn heard(&self) -> Option<Instant> {
        self.heard
    }

    pub fn poll(&mut self) -> Vec<Msg> {
        let mut msgs = Vec::new();
        let before = self.pending.len();
        self.pending.extend(self.conn.poll());
        if self.pending.len() > before {
            self.heard = Some(Instant::now());
        }
        while let Some(mut m) = self.pending.pop_front() {
            if !crate::validation::host_message(&m) {
                self.disconnect();
                break;
            }
            match &mut m {
                Msg::Chat { from, text } => { *from = crate::clean(from, crate::NAME_MAX); *text = crate::clean(text, crate::CHAT_MAX); }
                Msg::Pace { by, .. } => *by = crate::clean(by, crate::NAME_MAX),
                Msg::Waiting(names) => {
                    if names.len() > MAX_CONNECTIONS { self.disconnect(); break; }
                    for name in names { *name = crate::clean(name, crate::NAME_MAX); }
                }
                _ => {},
            }
            match m {
                Msg::Welcome { peer, reconnect } => {
                    if self.peer.is_some() || peer == 0 { self.disconnect(); break; }
                    self.peer = Some(peer);
                    self.reconnect = Some(reconnect);
                }
                // (The host measuring our round trip: answered at once, in the lobby and in
                // the game alike; and what it measured, kept.)
                Msg::Ping(n) => {
                    self.send(&Msg::Pong(n));
                    continue;
                }
                Msg::Pings(table) => {
                    self.pings = table;
                    continue;
                }
                Msg::Removed => self.removed = true,
                // Back to the lobby: the battle (and what it sent ahead of a return) is over.
                Msg::Rematch(_) => {
                    self.in_game = false;
                    self.history.clear();
                    self.history_bytes = 0;
                }
                Msg::Reject { ref reason } => {
                    let reason = cleaned(reason);
                    self.rejected = Some(reason.clone());
                    self.conn.open = false;
                    self.conn.outgoing.take();
                    self.pending.clear();
                    msgs.push(Msg::Reject { reason });
                    break;
                }
                Msg::History(part) => {
                    let previous = self.history.last().map_or(0, |(tick, _)| *tick);
                    let bytes = crate::validation::history_memory(&part);
                    if self.in_game || !crate::validation::valid_history(&part, u64::MAX - 1, previous)
                        || self.history_bytes.saturating_add(bytes) > MAX_HISTORY_BYTES {
                        self.disconnect();
                        break;
                    }
                    self.history_bytes += bytes;
                    self.history.extend(part);
                    continue;
                }
                Msg::Resume { start, player, upto, history, paused, speed } => {
                    let previous = self.history.last().map_or(0, |(tick, _)| *tick);
                    if self.in_game || upto == u64::MAX || previous > upto
                        || !crate::validation::valid_history(&history, upto, previous)
                        || self.history_bytes.saturating_add(crate::validation::history_memory(&history)) > MAX_HISTORY_BYTES {
                        self.disconnect();
                        break;
                    }
                    let mut all = std::mem::take(&mut self.history);
                    self.history_bytes = 0;
                    self.in_game = true;
                    all.extend(history);
                    msgs.push(Msg::Resume { start, player, upto, history: all, paused, speed });
                    break;
                }
                Msg::Start(start) => {
                    if self.in_game || !self.history.is_empty() { self.disconnect(); break; }
                    self.in_game = true;
                    msgs.push(Msg::Start(start));
                    break;
                }
                _ => {}
            }
            msgs.push(m);
        }
        msgs
    }

    pub fn is_connected(&self) -> bool {
        self.conn.open && !self.conn.failed.load(Ordering::Acquire)
    }
}

/// What happened on the host since the last poll.
#[derive(Debug)]
pub enum HostEvent {
    Joined(u8, String),
    Left(u8, String),
    Message(u8, Msg),
}

struct Peer {
    id: u8,
    name: String,
    conn: Connection,
    greeted: bool,
    accepted: Instant,
    /// The ping on its way (its number, when sent), and the last round trip measured (ms).
    ping: Option<(u32, Instant)>,
    rtt: Option<u16>,
    /// The maps it told us it has, by file stem (lower case), with their digests (none yet:
    /// not told). Kept as long as the connection, battles included.
    maps: Option<BTreeMap<String, u64>>,
}

/// The hosting end: accepts peers and relays messages.
pub struct Host {
    listener: TcpListener,
    peers: Vec<Peer>,
    next_peer: u8,
    data_hash: u64,
    /// New peers are turned away once the game has started...
    pub accepting: bool,
    /// ...except players coming back to it (names of those who left).
    pub rejoin: Vec<String>,
    pub port: u16,
    incoming: Arc<AtomicUsize>,
    credentials: BTreeMap<String, ReconnectToken>,
    /// Battle roster entries outlive their sockets. Do not assign an absent player's
    /// number to another connection when the eight-bit allocator wraps.
    pub(crate) roster_peers: BTreeSet<u8>,
    host_name: Option<String>,
    /// The lobby's password: newcomers must give it (players coming back to the battle show
    /// their credential instead).
    password: Option<String>,
    /// When the peers were last pinged, and the next ping's number.
    pinged: Instant,
    ping_number: u32,
    /// The mods we play with, in load order: a player turned away for other data is told them.
    mods: Vec<NetMod>,
}

impl Host {
    /// Listens on all interfaces. Port 0 picks a free port (see [`Host::port`]). `data_hash` is
    /// what joining players must match: the game's data without its maps.
    pub fn bind(port: u16, data_hash: u64) -> io::Result<Host> {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        Ok(Host { listener, peers: Vec::new(), next_peer: 1, data_hash, accepting: true, rejoin: Vec::new(), port, incoming: Arc::new(AtomicUsize::new(0)), credentials: BTreeMap::new(), roster_peers: BTreeSet::new(), host_name: None, password: None, pinged: Instant::now(), ping_number: 0, mods: Vec::new() })
    }

    /// The mods we play with, in load order (at most [`crate::MAX_MODS`] are told): a player whose
    /// data differs is told them, to see which of theirs differ.
    pub fn set_mods(&mut self, mods: Vec<NetMod>) {
        self.mods = mods.iter().take(crate::MAX_MODS).map(NetMod::cleaned).collect();
    }

    /// What `peer` told us of the map `name` (a file stem, any case): `None` before it has told
    /// its maps, `Some(None)` when it has no map of that name, else its digest of it.
    pub fn peer_map(&self, peer: u8, name: &str) -> Option<Option<u64>> {
        let maps = self.peers.iter().find(|p| p.id == peer && p.greeted)?.maps.as_ref()?;
        Some(maps.get(&name.trim().to_ascii_lowercase()).copied())
    }

    /// The lobby's password (none, or an empty one: anybody may join).
    pub fn set_password(&mut self, password: Option<&str>) {
        self.password = password.map(|p| crate::clean(p.trim(), 64)).filter(|p| !p.is_empty());
    }

    /// Whether newcomers need a password.
    pub fn locked(&self) -> bool {
        self.password.is_some()
    }

    /// Reserves the local host's display name, which has no remote connection of its own.
    pub fn set_host_name(&mut self, name: &str) {
        let name = crate::clean(name.trim(), 24).to_ascii_lowercase();
        for peer in &mut self.peers {
            if peer.greeted && peer.name.eq_ignore_ascii_case(&name) {
                peer.conn.send_frame(&encode(&Msg::Reject { reason: crate::Refusal::Name }));
                peer.conn.open = false;
            }
        }
        self.host_name = Some(name);
    }

    /// Accepts connections, handles handshakes and returns peer events.
    pub fn poll(&mut self) -> Vec<HostEvent> {
        // Owning the nonblocking listener makes dropping a host release its port at once.
        // Limit accepts per poll so incoming connections cannot monopolize a game frame.
        for _ in 0..32 {
            if self.peers.len() >= MAX_CONNECTIONS { break; }
            let stream = match self.listener.accept() {
                Ok((stream, _)) => stream,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    log::warn!("accept failed: {e}");
                    break;
                }
            };
            let Some(id) = (0..255u16).map(|n| (self.next_peer as u16 - 1 + n) % 255 + 1)
                .map(|n| n as u8).find(|id| !self.roster_peers.contains(id) && !self.peers.iter().any(|p| p.id == *id)) else { continue };
            match Connection::with_budget(stream, self.incoming.clone(), false) {
                Ok(conn) => {
                    self.next_peer = id.wrapping_add(1).max(1);
                    self.peers.push(Peer { id, name: String::new(), conn, greeted: false, accepted: Instant::now(), ping: None, rtt: None, maps: None });
                }
                Err(e) => log::warn!("connection setup failed: {e}"),
            }
        }
        let mut events = Vec::new();
        // A busy low-numbered peer must not monopolize the bounded game-thread batch.
        if !self.peers.is_empty() { self.peers.rotate_left(1); }
        let mut polled_bytes = 0usize;
        let mut names: Vec<String> = self.peers.iter().filter(|p| p.greeted).map(|p| p.name.to_ascii_lowercase()).collect();
        if let Some(name) = &self.host_name { names.push(name.clone()); }
        // Players back on a new connection while their old one lingers (name, new peer, where
        // their Joined went): the old one is closed and its Left put first.
        let mut replaced: Vec<(String, u8, usize)> = Vec::new();
        for p in &mut self.peers {
            if polled_bytes >= MAX_POLL_BYTES { break; }
            if !p.conn.open { continue; }
            if !p.greeted && p.accepted.elapsed() >= HANDSHAKE_TIMEOUT { p.conn.open = false; continue; }
            for msg in p.conn.poll() {
                polled_bytes = polled_bytes.saturating_add(crate::message_memory(&msg));
                if !p.greeted {
                    match msg {
                        Msg::Hello { name, protocol, data_hash, reconnect, password } => {
                            let short = crate::clean(name.trim(), 24);
                            let key = short.to_ascii_lowercase();
                            let returning = self.rejoin.iter().any(|n| n.eq_ignore_ascii_case(&short));
                            // A connected player's own credential: their old connection is dead
                            // (they are back before we noticed it go).
                            let stale = !returning && names.contains(&key) && self.host_name.as_deref() != Some(key.as_str())
                                && reconnect.is_some() && reconnect.as_ref() == self.credentials.get(&key);
                            let reason = if protocol != PROTOCOL {
                                Some(Refusal::Version { host: PROTOCOL, yours: protocol })
                            } else if data_hash != self.data_hash {
                                // (Our mods, for the player to see which of theirs differ.)
                                Some(Refusal::Data { mods: self.mods.clone() })
                            } else if !self.accepting && !returning && !stale {
                                Some(Refusal::Started)
                            } else if returning && (reconnect.is_none() || reconnect.as_ref() != self.credentials.get(&key)) {
                                Some(Refusal::Credential)
                            } else if !returning && !stale && self.password.as_ref().is_some_and(|p| *p != crate::clean(password.trim(), 64)) {
                                Some(Refusal::Password)
                            } else if short.is_empty() || names.contains(&key) && !stale {
                                Some(Refusal::Name)
                            } else {
                                None
                            };
                            if let Some(reason) = reason {
                                p.conn.send_frame(&encode(&Msg::Reject { reason }));
                                p.conn.open = false;
                                break;
                            } else {
                                let token = if returning || stale {
                                    self.credentials.get(&key).copied()
                                } else { ReconnectToken::generate().ok() };
                                let Some(token) = token else { p.conn.open = false; break };
                                self.credentials.insert(key.clone(), token);
                                names.push(key.clone());
                                p.greeted = true;
                                p.conn.welcome();
                                p.name = short;
                                p.conn.send_frame(&encode(&Msg::Welcome { peer: p.id, reconnect: token }));
                                if stale {
                                    replaced.push((key.clone(), p.id, events.len()));
                                }
                                events.push(HostEvent::Joined(p.id, p.name.clone()));
                            }
                        }
                        _ => { p.conn.open = false; break; }
                    }
                } else if matches!(msg, Msg::Bye) {
                    p.conn.open = false;
                    break;
                } else if let Msg::Pong(n) = msg {
                    if let Some((sent, at)) = p.ping
                        && sent == n
                    {
                        p.rtt = Some(at.elapsed().as_millis().min(u16::MAX as u128) as u16);
                        p.ping = None;
                    }
                } else if let Msg::Portrait(png) = &msg
                    && png.len() > crate::PORTRAIT_BYTES
                {
                    p.conn.open = false;
                    break;
                } else if let Msg::Maps(maps) = msg {
                    // The maps it has: kept for the connection (the lobby asks `peer_map`).
                    if maps.len() > crate::MAX_MAPS || maps.iter().any(|(name, _)| name.len() > crate::MAP_NAME_MAX) {
                        p.conn.open = false;
                        break;
                    }
                    p.maps = Some(maps.into_iter().map(|(name, digest)| (name.trim().to_ascii_lowercase(), digest)).collect());
                } else if let Msg::Chat { text, .. } = msg {
                    events.push(HostEvent::Message(p.id, Msg::Chat { from: p.name.clone(), text: crate::clean(&text, crate::CHAT_MAX) }));
                } else {
                    if !matches!(msg, Msg::SetSlot { .. } | Msg::SetSpectator(_) | Msg::Commands(_) | Msg::Hash { .. } | Msg::SetPace { .. } | Msg::TakeSlot(_) | Msg::Portrait(_)) {
                        p.conn.open = false;
                        break;
                    }
                    events.push(HostEvent::Message(p.id, msg));
                }
            }
        }
        for (key, new, at) in replaced.into_iter().rev() {
            if let Some(i) = self.peers.iter().position(|p| p.greeted && p.id != new && p.name.eq_ignore_ascii_case(&key)) {
                let old = self.peers.remove(i);
                shutdown_socket(&old.conn.socket);
                events.insert(at, HostEvent::Left(old.id, old.name.clone()));
            }
        }
        self.peers.retain(|p| {
            if !p.conn.open && p.greeted {
                events.push(HostEvent::Left(p.id, p.name.clone()));
            }
            p.conn.open
        });
        if self.accepting {
            self.credentials.retain(|name, _| self.peers.iter().any(|p| p.greeted && p.name.eq_ignore_ascii_case(name)));
        }
        // Every peer's round trip, measured and shared every PING_INTERVAL: a ping still
        // unanswered counts as at least as long as it has been out (a peer gone quiet shows it).
        if self.pinged.elapsed() >= crate::PING_INTERVAL {
            self.pinged = Instant::now();
            self.ping_number = self.ping_number.wrapping_add(1);
            let n = self.ping_number;
            for p in self.peers.iter_mut().filter(|p| p.greeted && p.conn.open) {
                if let Some((_, at)) = p.ping {
                    let waited = at.elapsed().as_millis().min(u16::MAX as u128) as u16;
                    p.rtt = Some(p.rtt.unwrap_or(0).max(waited));
                } else {
                    p.ping = Some((n, Instant::now()));
                }
                p.conn.send_frame(&encode(&Msg::Ping(n)));
            }
            let table = Msg::Pings(self.pings());
            self.broadcast(&table);
        }
        events
    }

    /// Everyone's round trip to us, milliseconds: ours 0, a peer's as last measured (those
    /// not measured yet left out).
    pub fn pings(&self) -> Vec<(u8, u16)> {
        std::iter::once((0, 0)).chain(self.peers.iter().filter(|p| p.greeted).filter_map(|p| p.rtt.map(|ms| (p.id, ms)))).collect()
    }

    /// Sends to every welcomed peer.
    pub fn broadcast(&mut self, msg: &Msg) {
        self.broadcast_where(msg, |_| true);
    }

    pub(crate) fn broadcast_where(&mut self, msg: &Msg, mut include: impl FnMut(u8) -> bool) {
        let frame = encode(msg);
        for p in self.peers.iter_mut().filter(|p| p.greeted && include(p.id)) {
            p.conn.send_frame(&frame);
        }
    }

    /// Waits, at most `timeout`, for what was sent to everyone to be written (the program
    /// closing just after a goodbye).
    pub fn drain(&self, timeout: Duration) {
        wait_until(timeout, || self.peers.iter().all(|p| p.conn.written()));
    }

    /// Closes a peer's connection; it leaves on the next poll.
    pub fn kick(&mut self, peer: u8) {
        if let Some(p) = self.peers.iter_mut().find(|p| p.id == peer) {
            p.conn.send_frame(&encode(&Msg::Bye));
            p.conn.open = false;
        }
    }

    pub fn send_to(&mut self, peer: u8, msg: &Msg) {
        let frame = encode(msg);
        if let Some(p) = self.peers.iter_mut().find(|p| p.id == peer && p.greeted) {
            p.conn.send_frame(&frame);
        }
    }

    /// Offers a resumable transfer without disconnecting a peer whose queue is full.
    pub(crate) fn try_send_to(&mut self, peer: u8, msg: &Msg) -> io::Result<()> {
        let p = self.peers.iter_mut().find(|p| p.id == peer && p.greeted)
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotConnected))?;
        let frame = encode(msg);
        // Leave room for ordinary broadcasts while history is waiting for the network.
        // An individual tick larger than BATCH_BYTES may still travel alone.
        let transfer_limit = (2 * crate::BATCH_BYTES as usize).max(frame.len());
        if p.conn.queued.load(Ordering::Relaxed).saturating_add(frame.len()) > transfer_limit {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        p.conn.try_send_frame(&frame)
    }

    /// Welcomed peers: (id, name).
    pub fn peers(&self) -> Vec<(u8, String)> {
        self.peers.iter().filter(|p| p.greeted).map(|p| (p.id, p.name.clone())).collect()
    }

    pub(crate) fn forget_rejoin(&mut self, name: &str) {
        self.rejoin.retain(|n| !n.eq_ignore_ascii_case(name));
        self.credentials.remove(&name.to_ascii_lowercase());
    }

    pub(crate) fn clear_rejoin(&mut self) {
        // (Those still here keep their credentials: the next battle's rejoining needs them.)
        let here: std::collections::BTreeSet<String> = self.peers.iter().filter(|p| p.greeted).map(|p| p.name.to_ascii_lowercase()).collect();
        self.credentials.retain(|name, _| here.contains(name));
        self.rejoin.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Role, Session, StartInfo};
    use std::time::Instant;
    use ta_sim::Command;

    fn connection() -> (Connection, TcpStream) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (peer, _) = listener.accept().unwrap();
        (Connection::new(stream).unwrap(), peer)
    }

    fn start() -> StartInfo {
        let slot = |peer| crate::LobbySlot { kind: crate::SlotKind::Human { peer, name: format!("Player{peer}") }, side: "ARM".into(), color: peer, team: peer, ready: true };
        StartInfo { map: "Test".into(), seed: 1, slots: vec![slot(0), slot(1)], rules: Default::default(), spectators: Vec::new() }
    }

    fn client_with(messages: Vec<Msg>) -> (Client, TcpStream) {
        let (conn, peer) = connection();
        (Client { conn, peer: Some(1), rejected: None, history: Vec::new(), pending: messages.into(), reconnect: None, history_bytes: 0, in_game: false, pings: Vec::new(), removed: false, heard: None }, peer)
    }

    #[test]
    fn start_handoff_keeps_ticks_and_pace_from_the_same_read() {
        let command = Command::Resign { player: 1 };
        let (mut client, _peer) = client_with(vec![
            Msg::Start(start()),
            Msg::Tick { tick: 1, commands: vec![command.clone()] },
            Msg::Pace { paused: true, speed: 200, by: "Host".into() },
        ]);
        let lobby = client.poll();
        assert!(matches!(lobby.as_slice(), [Msg::Start(_)]));
        let mut session = Session::client(client, 1);
        session.poll();
        assert_eq!(session.take_tick(1), Some(vec![command]));
        assert!(session.paused);
        assert_eq!(session.speed, 200);
    }

    #[test]
    fn resume_handoff_keeps_history_and_following_live_ticks() {
        let command = Command::Resign { player: 1 };
        let (mut client, _peer) = client_with(vec![
            Msg::History(vec![(1, vec![command.clone()])]),
            Msg::Resume { start: start(), player: 1, upto: 2, history: vec![], paused: false, speed: 100 },
            Msg::Tick { tick: 3, commands: vec![command.clone()] },
            Msg::Pace { paused: true, speed: 50, by: "Host".into() },
        ]);
        let mut messages = client.poll().into_iter();
        let Msg::Resume { player, upto, history, paused, speed, .. } = messages.next().unwrap() else { panic!("expected resume") };
        assert!(messages.next().is_none());
        let mut session = Session::resumed(client, player, upto, history, paused, speed);
        session.poll();
        assert_eq!(session.take_tick(1), Some(vec![command.clone()]));
        assert_eq!(session.take_tick(2), Some(vec![]));
        assert_eq!(session.take_tick(3), Some(vec![command]));
        assert!(session.paused);
        assert_eq!(session.speed, 50);
    }

    #[test]
    fn dropping_an_idle_host_releases_its_port_immediately() {
        let host = Host::bind(0, 1).unwrap();
        let port = host.port;
        drop(host);
        let _next = Host::bind(port, 1).expect("dropping the host must close its listener without another connection");
    }

    #[test]
    fn a_peer_that_stops_reading_cannot_block_the_game_thread() {
        let (mut conn, _peer) = connection();
        let frame: Arc<[u8]> = vec![0u8; 1 << 20].into();
        let started = Instant::now();
        for _ in 0..512 {
            conn.send_frame(&frame);
            if !conn.open {
                break;
            }
        }
        assert!(started.elapsed() < Duration::from_secs(2), "outbound backpressure must never wait on socket writes");
        assert!(!conn.open, "a nonreading peer must be dropped once its queue is full");
        assert!(conn.queued.load(Ordering::Relaxed) <= MAX_QUEUED_BYTES);
    }

    #[test]
    fn dropping_a_connection_delivers_its_final_frame() {
        let (mut conn, mut peer) = connection();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let expected = encode(&Msg::Reject { reason: crate::Refusal::Data { mods: vec![crate::NetMod { id: "escalation".into(), name: "TA: Escalation".into(), version: "10.2".into(), digest: [7; 16], data: true }] } });
        conn.send_frame(&expected);
        drop(conn);
        let mut received = vec![0; expected.len()];
        peer.read_exact(&mut received).unwrap();
        assert_eq!(&received[..], &expected[..]);
        assert_eq!(peer.read(&mut [0u8; 1]).unwrap(), 0, "the writer closes the socket after draining");
    }

    #[test]
    fn receiving_eof_closes_the_writer_while_the_connection_is_retained() {
        let (mut conn, mut peer) = connection();
        // macOS may reject socket option changes after both halves have closed.
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        peer.shutdown(std::net::Shutdown::Write).unwrap();
        let started = Instant::now();
        while conn.open && started.elapsed() < Duration::from_secs(2) {
            conn.poll();
            thread::sleep(Duration::from_millis(1));
        }
        assert!(!conn.open && conn.outgoing.is_none());
        assert_eq!(peer.read(&mut [0u8; 1]).unwrap(), 0);
    }

    #[test]
    fn shutdown_closes_writes_when_the_read_half_is_already_closed() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        stream.shutdown(std::net::Shutdown::Read).unwrap();
        shutdown_socket(&stream);
        shutdown_socket(&stream); // Closing an already-closed half remains harmless.
        assert_eq!(peer.read(&mut [0u8; 1]).unwrap(), 0, "the peer sees EOF while the socket handle is retained");
        drop(stream);
    }

    /// A one-frame writer queue drained explicitly by the test, so backpressure is
    /// deterministic and requires neither a huge history nor a slow loopback socket.
    struct ResumeTest {
        session: Session,
        incoming: SyncSender<Incoming>,
        frames: Receiver<Arc<[u8]>>,
        queued: Arc<AtomicUsize>,
        _peer: TcpStream,
    }

    impl ResumeTest {
        fn new() -> Self {
            use crate::{LobbySlot, SlotKind};
            let mut host = Host::bind(0, 1).unwrap();
            let peer = TcpStream::connect(("127.0.0.1", host.port)).unwrap();
            let (socket, _) = host.listener.accept().unwrap();
            let (incoming, rx) = mpsc::sync_channel(8);
            let (outgoing, frames) = mpsc::sync_channel(1);
            let queued = Arc::new(AtomicUsize::new(0));
            let conn = Connection { socket, outgoing: Some(outgoing), queued: queued.clone(), failed: Arc::new(AtomicBool::new(false)), rx, incoming: Arc::new(AtomicUsize::new(0)), pending: None, open: true, welcomed: Arc::new(AtomicBool::new(true)) };
            host.peers.push(Peer { id: 2, name: String::new(), conn, greeted: false, accepted: Instant::now(), ping: None, rtt: None, maps: None });
            host.accepting = false;
            host.rejoin.push("Guest".into());
            let token = ReconnectToken::generate().unwrap();
            host.credentials.insert("guest".into(), token);
            let slot = |peer, name: &str| LobbySlot { kind: SlotKind::Human { peer, name: name.into() }, side: "ARM".into(), color: peer, team: peer, ready: true };
            let start = StartInfo { slots: vec![slot(0, "Host"), slot(1, "Guest")], ..start() };
            let mut session = Session::host(host, &start);
            session.peer_players.remove(&1);
            session.peer_ticks.remove(&1);
            session.absent.push(("Guest".into(), 1, Instant::now()));
            // Each tick is a little over half a batch; the whole history cannot fit in
            // the one-frame queue. No peers are greeted yet, so these are just history.
            for generation in 1..=5 {
                session.submit(Command::Order {
                    player: 0,
                    units: (0..70_000).map(|slot| ta_sim::UnitId { slot, generation }).collect(),
                    order: ta_sim::Order::Stop, queue: false,
                });
                session.produce_tick();
            }
            incoming.send(Incoming::test(Some(Msg::Hello { name: "Guest".into(), protocol: PROTOCOL, data_hash: 1, reconnect: Some(token), password: String::new() }))).unwrap();
            Self { session, incoming, frames, queued, _peer: peer }
        }

        fn take(&self) -> Msg {
            let frame = self.frames.try_recv().expect("one accepted frame");
            self.queued.fetch_sub(frame.len(), Ordering::Relaxed);
            bincode::deserialize(&frame[4..]).unwrap()
        }
    }

    #[test]
    fn resume_retries_a_small_queue_and_includes_ticks_produced_during_transfer() {
        let mut test = ResumeTest::new();
        test.session.poll();
        assert_eq!(test.session.resuming[&2].next_history, 0, "Welcome occupies the queue");
        test.session.poll();
        assert_eq!(test.session.resuming[&2].next_history, 0, "a blocked send does not advance");
        assert!(matches!(test.take(), Msg::Welcome { peer: 2, .. }));
        let mut history = Vec::new();
        let mut resume = None;
        for _ in 0..10 {
            test.session.poll();
            if history.is_empty() {
                test.session.submit(Command::SetStates { player: 0, units: Vec::new(), fire: Some(1), movement: None });
                test.session.produce_tick();
                test.session.produce_tick(); // an empty tick must also count toward upto
                test.session.paused = true;
                test.session.speed = 200;
            }
            // Leave the accepted frame queued for another poll to force another retry.
            test.session.poll();
            match test.take() {
                Msg::History(part) => history.extend(part),
                Msg::Resume { upto, history: last, player, paused, speed, .. } => {
                    history.extend(last);
                    resume = Some((upto, player, paused, speed));
                    break;
                }
                other => panic!("live ticks must wait until after Resume: {other:?}"),
            }
        }
        assert_eq!(resume, Some((7, 1, true, 200)));
        assert_eq!(history, test.session.history, "every command appears once and in order");
        assert!(test.session.resuming.is_empty());
        let Role::Host(host) = &test.session.role else { unreachable!() };
        assert!(host.peers[0].conn.open, "temporary backpressure must not disconnect the peer");
        test.session.produce_tick();
        assert!(matches!(test.take(), Msg::Tick { tick: 8, .. }));
    }

    #[test]
    fn disconnecting_during_resume_discards_the_transfer_cursor() {
        let mut test = ResumeTest::new();
        test.session.poll();
        assert!(test.session.resuming.contains_key(&2));
        test.incoming.send(Incoming::test(None)).unwrap();
        test.session.poll();
        assert!(test.session.resuming.is_empty());
        assert_eq!(test.session.departed, ["Guest"]);
        assert!(matches!(test.take(), Msg::Welcome { .. }));
        assert!(matches!(test.frames.try_recv(), Err(mpsc::TryRecvError::Disconnected)));
    }

    #[test]
    fn a_sender_cannot_build_an_unbounded_unread_queue() {
        let (mut conn, mut peer) = connection();
        let frame = encode(&Msg::Bye);
        let flood = frame.repeat(MAX_INCOMING_FRAMES + 1);
        peer.write_all(&flood).unwrap(); // less than 20 KiB, with no large-allocation repro
        let deadline = Instant::now() + Duration::from_secs(3);
        while !conn.failed.load(Ordering::Acquire) && Instant::now() < deadline { thread::sleep(Duration::from_millis(1)); }
        assert!(conn.failed.load(Ordering::Acquire));
        assert!(conn.incoming.load(Ordering::Relaxed) <= MAX_INCOMING_BYTES);
        assert!(conn.poll().len() <= MAX_POLL_MESSAGES);
        let budget = conn.incoming.clone();
        drop(conn);
        assert_eq!(budget.load(Ordering::Relaxed), 0, "dropping an unread queue releases all its charges");
    }

    #[test]
    fn malformed_frames_and_trailing_bytes_are_rejected() {
        let mut frame = bincode::serialize(&Msg::Bye).unwrap();
        frame.push(0);
        assert!(decode(&frame).is_err());
        // Commands vector claims an impossible length, but supplies no commands.
        let mut frame = bincode::serialize(&Msg::Commands(Vec::new())).unwrap();
        frame[4..12].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(decode(&frame).is_err());
    }

    #[test]
    fn rejected_handshake_cannot_be_followed_by_a_valid_one_in_the_same_read() {
        let mut test = ResumeTest::new();
        let Role::Host(host) = &mut test.session.role else { unreachable!() };
        let peer = &mut host.peers[0];
        // Discard the valid fixture hello, then supply a rejection followed by a valid hello.
        let valid = peer.conn.rx.try_recv().unwrap();
        test.incoming.send(Incoming::test(Some(Msg::Hello { name: "Guest".into(), protocol: 0, data_hash: 1, reconnect: None, password: String::new() }))).unwrap();
        test.incoming.send(valid).unwrap();
        assert!(host.poll().iter().all(|e| !matches!(e, HostEvent::Joined(..))));
        assert!(host.peers.is_empty());
        assert!(matches!(test.take(), Msg::Reject { .. }));
    }

    #[test]
    fn idle_handshakes_expire_without_consuming_a_peer_slot_forever() {
        let mut test = ResumeTest::new();
        let Role::Host(host) = &mut test.session.role else { unreachable!() };
        host.peers[0].accepted = Instant::now() - HANDSHAKE_TIMEOUT;
        assert!(host.poll().is_empty());
        assert!(host.peers.is_empty());
    }

    #[test]
    fn resume_rejects_invalid_order_overflow_and_excess_accumulated_history() {
        for history in [vec![(0, vec![])], vec![(2, vec![]), (1, vec![])], vec![(1, vec![]), (1, vec![])]] {
            let (mut client, _peer) = client_with(vec![Msg::History(history)]);
            assert!(client.poll().is_empty());
            assert!(!client.is_connected());
        }
        let (mut client, _peer) = client_with(vec![Msg::Resume { start: start(), player: 1, upto: u64::MAX, history: vec![], paused: false, speed: 100 }]);
        assert!(client.poll().is_empty());
        assert!(!client.is_connected());
        let (mut client, _peer) = client_with(vec![Msg::History(vec![(1, vec![Command::Resign { player: 1 }])])]);
        client.history_bytes = MAX_HISTORY_BYTES;
        assert!(client.poll().is_empty());
        assert!(!client.is_connected());
    }

    #[test]
    fn reconnect_tokens_are_random_and_debug_redacted() {
        let first = ReconnectToken::generate().unwrap();
        let second = ReconnectToken::generate().unwrap();
        assert_ne!(first, second);
        assert_eq!(format!("{first:?}"), "ReconnectToken([redacted])");
        assert!(format!("{:?}", Msg::Welcome { peer: 1, reconnect: first }).contains("[redacted]"));
    }

    #[test]
    fn connection_slots_are_bounded_and_timed_out_slots_can_be_reused() {
        let mut host = Host::bind(0, 1).unwrap();
        let mut sockets = Vec::new();
        for _ in 0..MAX_CONNECTIONS {
            sockets.push(TcpStream::connect(("127.0.0.1", host.port)).unwrap());
            host.poll();
        }
        assert_eq!(host.peers.len(), MAX_CONNECTIONS);
        sockets.push(TcpStream::connect(("127.0.0.1", host.port)).unwrap());
        host.poll();
        assert_eq!(host.peers.len(), MAX_CONNECTIONS);
        host.peers[0].accepted = Instant::now() - HANDSHAKE_TIMEOUT;
        host.poll();
        assert_eq!(host.peers.len(), MAX_CONNECTIONS - 1);
        host.poll();
        assert_eq!(host.peers.len(), MAX_CONNECTIONS);
    }

    #[test]
    fn exhausting_host_history_disables_rejoin_without_stopping_live_ticks() {
        let mut test = ResumeTest::new();
        test.session.poll();
        assert!(matches!(test.take(), Msg::Welcome { .. }));
        test.session.history_bytes = MAX_HISTORY_BYTES;
        test.session.submit(Command::Resign { player: 0 });
        test.session.produce_tick();
        assert!(!test.session.reconnect_available);
        assert!(test.session.history.is_empty() && test.session.resuming.is_empty());
        assert_eq!(test.session.notices.last(), Some(&crate::Notice::ReconnectLimit));
        assert!(matches!(test.take(), Msg::ReconnectUnavailable));
        test.session.poll();
        let Role::Host(host) = &test.session.role else { unreachable!() };
        assert!(host.rejoin.is_empty(), "a later departure cannot reenable incomplete history");
        test.session.produce_tick();
        assert_eq!(test.session.take_tick(7), Some(vec![]), "the active match continues");
    }
}
