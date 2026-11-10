//! Bounded, asset-free probes of the multiplayer trust boundary.
use std::time::{Duration, Instant};
use ta_net::{Client, Host, HostEvent, LobbySlot, Msg, Role, Session, SlotKind, StartInfo};
use ta_sim::{Command, cheats::Cheat};

fn until(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !done() {
        assert!(Instant::now() < deadline, "network event did not arrive");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn joined() -> (Host, Client, u8) {
    let mut host = Host::bind(0, 7).unwrap();
    let mut client = Client::connect(([127, 0, 0, 1], host.port).into(), "Guest", 7).unwrap();
    let mut id = None;
    until(|| {
        for event in host.poll() { if let HostEvent::Joined(peer, _) = event { id = Some(peer); } }
        client.poll();
        id.is_some() && client.peer.is_some()
    });
    (host, client, id.unwrap())
}

fn start(peer: u8, spectator: bool) -> StartInfo {
    let slot = |peer, name: &str, color| LobbySlot { kind: SlotKind::Human { peer, name: name.into() }, side: "ARM".into(), color, team: color, ready: true };
    let mut slots = vec![slot(0, "Host", 0)];
    if !spectator { slots.push(slot(peer, "Guest", 1)); }
    StartInfo { map: "Test".into(), seed: 1, slots, rules: Default::default(), spectators: if spectator { vec![(peer, "Guest".into())] } else { vec![] } }
}

fn deliver(host: &mut Session, client: &mut Client) {
    let before = host.chat.len();
    client.send(&Msg::Chat { from: "Host".into(), text: "marker".into() });
    until(|| { host.poll(); client.poll(); host.chat.len() > before });
}

#[test]
fn future_hashes_cannot_overflow_or_bypass_waiting() {
    let (host, mut client, peer) = joined();
    let mut host = Session::host(host, &start(peer, false));
    while host.can_produce() { host.produce_tick(); }
    client.send(&Msg::Hash { tick: u64::MAX, hash: 0 });
    client.send(&Msg::Hash { tick: 150, hash: 0 });
    deliver(&mut host, &mut client);
    assert!(!host.can_produce());
    assert_eq!(host.waiting_for, ["Guest"]);
    client.send(&Msg::Hash { tick: 75, hash: 0 });
    deliver(&mut host, &mut client);
    assert!(host.can_produce(), "valid progress still resumes the match");
}

#[test]
fn spectators_cannot_inject_desyncs_or_control_pace() {
    let (host, mut client, peer) = joined();
    let mut host = Session::host(host, &start(peer, true));
    for _ in 0..15 { host.produce_tick(); }
    host.report_hash(15, 123);
    client.send(&Msg::Hash { tick: 15, hash: 999 });
    client.send(&Msg::SetPace { paused: Some(true), speed: Some(1000) });
    deliver(&mut host, &mut client);
    assert!(host.desync.is_none());
    assert!(!host.paused);
    assert_eq!(host.speed, 100);
    assert_eq!(host.chat.last().unwrap().0, "Guest", "chat identity comes from the connection");
}

#[test]
fn only_normal_commands_for_the_connected_player_are_bundled() {
    let (host, mut client, peer) = joined();
    let mut host = Session::host(host, &start(peer, false));
    client.send(&Msg::Commands(vec![Command::Abandon { player: 1 }, Command::Cheat { player: 1, cheat: Cheat::Resources }, Command::Resign { player: 0 }, Command::Resign { player: 1 }]));
    deliver(&mut host, &mut client);
    host.produce_tick();
    assert_eq!(host.take_tick(1), Some(vec![Command::Resign { player: 1 }]));
}

#[test]
fn commands_cannot_accumulate_without_bound_while_paused() {
    let (host, mut client, peer) = joined();
    let mut host = Session::host(host, &start(peer, false));
    host.paused = true;
    let order = Command::SetStates { player: 1, units: Vec::new(), fire: Some(1), movement: None };
    client.send(&Msg::Commands(vec![order.clone(); 11_000]));
    deliver(&mut host, &mut client);
    // The second batch fills the remaining allowance; its excess is dropped, not kept
    // (nor the sender thrown out for orders piling up).
    client.send(&Msg::Commands(vec![order; 11_000]));
    deliver(&mut host, &mut client);
    assert!(client.is_connected());
    host.produce_tick();
    assert_eq!(host.take_tick(1).unwrap().len(), 20_000, "only the bounded command prefix was accepted");
    host.produce_tick();
    assert_eq!(host.take_tick(2), Some(Vec::new()), "excess orders cannot leak into a later tick");
}

#[test]
fn a_stranger_announcing_a_huge_frame_cannot_cut_the_players_off() {
    use std::io::Write;
    let (mut host, mut client, peer) = joined();
    // Before saying hello, a connection's frames are small: one announcing 4 MiB is let go,
    // and takes nobody's decoding budget with it.
    let mut stranger = std::net::TcpStream::connect(("127.0.0.1", host.port)).unwrap();
    stranger.write_all(&(4u32 << 20).to_le_bytes()).unwrap();
    let settle = Instant::now();
    while settle.elapsed() < Duration::from_millis(300) {
        host.poll();
        client.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    client.send(&Msg::Chat { from: String::new(), text: "still here".into() });
    let mut heard = false;
    until(|| {
        for event in host.poll() {
            match event {
                HostEvent::Message(p, Msg::Chat { .. }) if p == peer => heard = true,
                HostEvent::Left(p, _) => assert_ne!(p, peer, "the guest was cut off"),
                _ => {}
            }
        }
        client.poll();
        heard
    });
    assert!(client.is_connected());
}

#[test]
fn a_player_resigns_once_however_often_they_ask() {
    let (host, mut client, peer) = joined();
    let mut host = Session::host(host, &start(peer, false));
    client.send(&Msg::Commands(vec![Command::Resign { player: 1 }; 5_000]));
    deliver(&mut host, &mut client);
    client.send(&Msg::Commands(vec![Command::Resign { player: 1 }; 5_000]));
    deliver(&mut host, &mut client);
    host.produce_tick();
    assert_eq!(host.take_tick(1), Some(vec![Command::Resign { player: 1 }]), "one resignation, not ten thousand in every rejoin's history");
}

#[test]
fn a_name_without_the_secret_cannot_reclaim_an_army() {
    let (host, mut client, peer) = joined();
    let addr = ([127, 0, 0, 1], host.port).into();
    let token = client.reconnect_token();
    let mut host = Session::host(host, &start(peer, false));
    client.send(&Msg::Bye);
    drop(client);
    until(|| { host.poll(); !host.departed.is_empty() });
    let mut impostor = Client::connect(addr, "gUeSt", 7).unwrap();
    until(|| { host.poll(); impostor.poll(); impostor.rejected.is_some() });
    assert!(impostor.peer.is_none());
    let mut returning = Client::connect_with_token(addr, "guest", 7, token, None).unwrap();
    let mut resumed = false;
    until(|| {
        host.poll();
        resumed |= returning.poll().iter().any(|m| matches!(m, Msg::Resume { player: 1, .. }));
        resumed
    });
    assert!(returning.is_connected());
}

#[test]
fn duplicate_names_and_host_messages_from_clients_are_rejected() {
    let (mut host, mut client, peer) = joined();
    let mut duplicate = Client::connect(([127, 0, 0, 1], host.port).into(), " gUEST ", 7).unwrap();
    until(|| { host.poll(); duplicate.poll(); duplicate.rejected.is_some() });
    client.send(&Msg::Tick { tick: 1, commands: vec![] });
    let mut left = false;
    until(|| {
        left |= host.poll().iter().any(|e| matches!(e, HostEvent::Left(p, _) if *p == peer));
        client.poll();
        left
    });
}

#[test]
fn the_host_name_is_reserved_before_accepting_remote_players() {
    let mut host = Host::bind(0, 7).unwrap();
    host.set_host_name("Host");
    let mut impostor = Client::connect(([127, 0, 0, 1], host.port).into(), " hOST ", 7).unwrap();
    until(|| { host.poll(); impostor.poll(); impostor.rejected.is_some() });
    assert!(impostor.peer.is_none());
    assert!(host.peers().is_empty());
}

#[test]
fn malformed_tick_sequences_disconnect_instead_of_stalling() {
    for ticks in [vec![2], vec![1, 1], vec![1, 3]] {
        let (mut host, client, peer) = joined();
        let mut client = Session::client(client, 1);
        for tick in ticks { host.send_to(peer, &Msg::Tick { tick, commands: vec![] }); }
        until(|| { client.poll(); client.host_lost });
        assert!(client.backlog() <= 1);
    }
}

#[test]
fn sparse_resume_does_not_allocate_an_empty_bundle_per_tick() {
    let (_host, client, _peer) = joined();
    let mut client = Session::resumed(client, 1, 1_000_000_000_000, vec![(2, vec![Command::Resign { player: 1 }])], false, 100);
    assert_eq!(client.backlog() as u64, 1_000_000_000_000);
    assert_eq!(client.take_tick(1), Some(vec![]));
    assert_eq!(client.take_tick(2), Some(vec![Command::Resign { player: 1 }]));
    assert_eq!(client.take_tick(3), Some(vec![]));
    assert!(matches!(client.role, Role::Client(_)));
}

#[test]
fn ally_chat_and_pings_use_the_connected_player_and_never_reach_enemies() {
    let mut transport = Host::bind(0, 7).unwrap();
    transport.set_host_name("Host");
    let mut clients = Vec::new();
    for name in ["AllyA", "AllyB", "Enemy", "Watcher"] {
        let mut client = Client::connect(([127, 0, 0, 1], transport.port).into(), name, 7).unwrap();
        until(|| { transport.poll(); client.poll(); client.peer.is_some() });
        clients.push(client);
    }
    let slot = |peer, name: &str, team| LobbySlot { kind: SlotKind::Human { peer, name: name.into() }, side: "ARM".into(), color: team, team, ready: true };
    let start = StartInfo {
        map: "Test".into(), seed: 1, rules: Default::default(),
        slots: vec![slot(0, "Host", 0), slot(clients[0].peer.unwrap(), "AllyA", 0), slot(clients[1].peer.unwrap(), "AllyB", 0), slot(clients[2].peer.unwrap(), "Enemy", 1)],
        spectators: vec![(clients[3].peer.unwrap(), "Watcher".into())],
    };
    let mut host = Session::host(transport, &start);
    let receive = |host: &mut Session, clients: &mut Vec<Client>, marker: &str| {
        let mut received = vec![Vec::<(String, String)>::new(); clients.len()];
        until(|| {
            host.poll();
            for (i, client) in clients.iter_mut().enumerate() {
                received[i].extend(client.poll().into_iter().filter_map(|m| match m { Msg::Chat { from, text } => Some((from, text)), _ => None }));
            }
            received.iter().all(|messages| messages.iter().any(|(_, text)| text == marker))
        });
        received
    };
    let send = |client: &mut Client, text: String, marker: &str| {
        client.send(&Msg::Chat { from: "Host".into(), text });
        client.send(&Msg::Chat { from: "Host".into(), text: marker.into() });
    };

    send(&mut clients[0], format!("{}3 private plan", ta_net::ALLY_PREFIX), "first");
    let messages = receive(&mut host, &mut clients, "first");
    let canonical = ("AllyA".to_owned(), format!("{}1 private plan", ta_net::ALLY_PREFIX));
    for i in [0, 1, 3] { assert!(messages[i].contains(&canonical)); }
    assert!(!messages[2].iter().any(|(_, text)| text.contains("private plan")));
    assert!(host.chat.contains(&canonical));

    send(&mut clients[2], format!("{}0 enemy plan", ta_net::ALLY_PREFIX), "second");
    let messages = receive(&mut host, &mut clients, "second");
    let canonical = ("Enemy".to_owned(), format!("{}3 enemy plan", ta_net::ALLY_PREFIX));
    for i in [2, 3] { assert!(messages[i].contains(&canonical)); }
    for i in [0, 1] { assert!(!messages[i].iter().any(|(_, text)| text.contains("enemy plan"))); }
    assert!(!host.chat.contains(&canonical), "a playing host does not display the other team's chat");

    send(&mut clients[0], format!("{}3 10 20", ta_net::PING_PREFIX), "third");
    let messages = receive(&mut host, &mut clients, "third");
    let canonical = ("AllyA".to_owned(), format!("{}1 10 20", ta_net::PING_PREFIX));
    for i in [0, 1, 3] { assert!(messages[i].contains(&canonical)); }
    assert!(!messages[2].iter().any(|(_, text)| text.starts_with(ta_net::PING_PREFIX)));

    send(&mut clients[3], format!("{}0 forged spectator plan", ta_net::ALLY_PREFIX), "fourth");
    let messages = receive(&mut host, &mut clients, "fourth");
    assert!(messages.iter().flatten().all(|(_, text)| !text.contains("forged spectator plan")));

    host.send_chat("Host", &format!("{}3 host plan", ta_net::ALLY_PREFIX));
    host.send_chat("Host", "fifth");
    let messages = receive(&mut host, &mut clients, "fifth");
    let canonical = ("Host".to_owned(), format!("{}0 host plan", ta_net::ALLY_PREFIX));
    for i in [0, 1, 3] { assert!(messages[i].contains(&canonical)); }
    assert!(!messages[2].iter().any(|(_, text)| text.contains("host plan")));
}
