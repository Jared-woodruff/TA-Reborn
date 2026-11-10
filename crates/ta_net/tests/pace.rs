//! Pause, game speed and waiting for a peer that falls behind, over loopback (no game data
//! needed).

use std::time::{Duration, Instant};

use ta_net::{Client, HASH_INTERVAL, Host, HostEvent, LobbySlot, MAX_LEAD, Msg, Notice, SPEED_MAX, Session, SlotKind, StartInfo};

/// Polls both ends until `done` holds (or five seconds pass).
fn pump(host: &mut Session, client: &mut Session, mut done: impl FnMut(&mut Session, &mut Session) -> bool) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(5) {
        host.poll();
        client.poll();
        if done(host, client) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

fn connected_pair() -> (Session, Session) {
    let mut host = Host::bind(0, 1).expect("bind");
    let addr = ta_net::resolve(&format!("127.0.0.1:{}", host.port)).unwrap();
    let mut client = Client::connect(addr, "Guest", 1).expect("connect");
    let mut peer = None;
    let t = Instant::now();
    while (peer.is_none() || client.peer.is_none()) && t.elapsed() < Duration::from_secs(5) {
        for e in host.poll() {
            if let HostEvent::Joined(id, _) = e {
                peer = Some(id);
            }
        }
        client.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    let peer = peer.expect("the client should join");
    let slot = |kind: SlotKind, color: u8| LobbySlot { kind, side: "ARM".into(), color, ready: true, team: color };
    let start = StartInfo {
        map: "Test".into(),
        seed: 1,
        slots: vec![slot(SlotKind::Human { peer: 0, name: "Host".into() }, 0), slot(SlotKind::Human { peer, name: "Guest".into() }, 1)],
        rules: Default::default(),
        spectators: Vec::new(),
    };
    host.accepting = false;
    (Session::host(host, &start), Session::client(client, 1))
}

#[test]
fn the_host_waits_for_a_peer_that_falls_behind() {
    let (mut host, mut client) = connected_pair();
    // The host runs ahead of a silent peer until it is MAX_LEAD ticks behind...
    let mut produced = 0;
    while host.can_produce() {
        host.produce_tick();
        produced += 1;
        assert!(produced < 1000, "the host must stop");
    }
    assert_eq!(produced, MAX_LEAD + 1);
    assert_eq!(host.waiting_for, ["Guest"]);
    // ...and everyone is told whom the game waits for.
    assert!(pump(&mut host, &mut client, |_, c| c.waiting_for == ["Guest"]));

    // The peer catches up, reporting its progress as it goes; the host carries on.
    for tick in 1..=produced {
        assert!(pump(&mut host, &mut client, |_, c| c.backlog() > 0), "bundle {tick} should arrive");
        assert!(client.take_tick(tick).is_some());
        client.report_hash(tick, 7);
    }
    assert!(pump(&mut host, &mut client, |h, _| h.can_produce()), "the host should resume");
    assert!(host.waiting_for.is_empty());
    assert!(pump(&mut host, &mut client, |_, c| c.waiting_for.is_empty()));
    assert!(host.desync.is_none(), "the host never hashed; nothing to compare");
    assert_eq!(HASH_INTERVAL, 15, "progress reports every half second keep the wait short");
}

#[test]
fn anyone_can_pause_and_set_the_speed() {
    let (mut host, mut client) = connected_pair();
    // A peer asks; the host applies it and says who did it.
    client.set_pace(Some(true), None, "(ignored: the host knows who we are)");
    assert!(pump(&mut host, &mut client, |h, c| h.paused && c.paused));
    assert!(!host.can_produce(), "no ticks while paused");
    assert_eq!(host.notices, [Notice::Paused("Guest".into())]);
    assert_eq!(client.notices, [Notice::Paused("Guest".into())]);
    assert_eq!(client.paused_by.as_deref(), Some("Guest"));

    // The host resumes and speeds up; speeds are clamped.
    host.set_pace(Some(false), Some(5000), "Host");
    assert!(pump(&mut host, &mut client, |_, c| !c.paused && c.speed == SPEED_MAX));
    assert!(host.can_produce());
    assert_eq!(&client.notices[1..], [Notice::Resumed("Host".into()), Notice::Speed("Host".into(), 1000)]);
    assert_eq!(client.paused_by, None);

    // Asking for what is already so announces nothing.
    let before = client.notices.len();
    client.set_pace(Some(false), Some(SPEED_MAX), "Guest");
    for _ in 0..50 {
        host.poll();
        client.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    host.set_pace(None, Some(100), "Host");
    assert!(pump(&mut host, &mut client, |_, c| c.speed == 100));
    assert_eq!(&client.notices[before..], [Notice::Speed("Host".into(), 100)]);
}

#[test]
fn the_host_can_go_on_without_a_stuck_peer() {
    let (mut host, mut client) = connected_pair();
    while host.can_produce() {
        host.produce_tick();
    }
    assert_eq!(host.waiting_for, ["Guest"]);
    assert!(host.waiting_secs() >= 0.0);
    // No waiting for them to come back in this test.
    host.abandon_after = Duration::ZERO;
    host.drop_waiting();
    // The peer's connection closes: it leaves, and the game goes on.
    assert!(pump(&mut host, &mut client, |h, _| h.can_produce()), "the host should resume without the dropped peer");
    assert_eq!(host.departed, ["Guest"]);
    assert_eq!(host.waiting_secs(), 0.0);
    assert!(pump(&mut host, &mut client, |_, c| c.host_lost), "the dropped peer is told");
    // The next bundle hands the departed player's army to an ally.
    host.produce_tick();
    let commands: Vec<_> = (0..200).filter_map(|t| host.take_tick(t)).flatten().collect();
    assert_eq!(commands, [ta_sim::Command::Abandon { player: 1 }]);
}

/// A player who drops out can come back: the host checks their secret, sends every tick's
/// commands so far, and their army is still theirs; one who stays away is replaced.
#[test]
fn a_player_who_left_can_come_back() {
    let mut host = Host::bind(0, 1).expect("bind");
    let addr = ta_net::resolve(&format!("127.0.0.1:{}", host.port)).unwrap();
    let mut client = Client::connect(addr, "Guest", 1).expect("connect");
    let mut peer = None;
    let t = Instant::now();
    while (peer.is_none() || client.peer.is_none()) && t.elapsed() < Duration::from_secs(5) {
        for e in host.poll() {
            if let HostEvent::Joined(id, _) = e {
                peer = Some(id);
            }
        }
        client.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    let peer = peer.expect("the client should join");
    let reconnect = client.reconnect_token();
    let slot = |kind: SlotKind, color: u8| LobbySlot { kind, side: "ARM".into(), color, ready: true, team: color };
    let start = StartInfo {
        map: "Test".into(),
        seed: 1,
        slots: vec![slot(SlotKind::Human { peer: 0, name: "Host".into() }, 0), slot(SlotKind::Human { peer, name: "Guest".into() }, 1)],
        rules: Default::default(),
        spectators: Vec::new(),
    };
    host.accepting = false;
    let (mut host, mut client) = (Session::host(host, &start), Session::client(client, 1));
    // A few ticks, one with an order of the host's.
    let order = ta_sim::Command::SetStates { player: 0, units: Vec::new(), fire: Some(1), movement: None };
    host.produce_tick();
    host.submit(order.clone());
    host.produce_tick();
    host.produce_tick();
    // The guest drops out; a stranger can't get in, the guest can.
    client.leave();
    drop(client);
    assert!(pump_host(&mut host, |h| !h.departed.is_empty()), "the host notices the guest left");
    let mut stranger = Client::connect(addr, "Stranger", 1).unwrap();
    let t = Instant::now();
    while stranger.rejected.is_none() && t.elapsed() < Duration::from_secs(5) {
        host.poll();
        stranger.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(stranger.rejected.is_some(), "only players of the game get back in");
    let mut back = Client::connect_with_token(addr, "guest", 1, reconnect, None).unwrap();
    let mut resume = None;
    let t = Instant::now();
    while resume.is_none() && t.elapsed() < Duration::from_secs(5) {
        host.poll();
        for m in back.poll() {
            if let Msg::Resume { start, player, upto, history, paused, speed } = m {
                resume = Some((start, player, upto, history, paused, speed));
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let (got_start, player, upto, history, paused, speed) = resume.expect("the host sends the game so far");
    // The game's start, their seat under their new peer number.
    let mut renumbered = start.clone();
    renumbered.slots[1].kind = SlotKind::Human { peer: back.peer.expect("welcomed"), name: "Guest".into() };
    assert_eq!((got_start, player, upto), (renumbered, 1, 3));
    let mut back = Session::resumed(back, player, upto, history, paused, speed);
    assert_eq!(back.backlog(), 3, "three ticks to catch up on");
    assert_eq!(back.take_tick(1), Some(vec![]));
    assert_eq!(back.take_tick(2), Some(vec![order]));
    assert_eq!(back.take_tick(3), Some(vec![]));
    // The army was kept: no Abandon in the next bundle, and the host waits for the guest.
    host.produce_tick();
    assert!(host.notices.iter().any(|n| *n == Notice::Back("guest".into())), "{:?}", host.notices);
    assert!(!host.history_commands().iter().any(|c| matches!(c, ta_sim::Command::Abandon { .. })));
}

/// Polls the host until `done` holds (or five seconds pass).
fn pump_host(host: &mut Session, mut done: impl FnMut(&mut Session) -> bool) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(5) {
        host.poll();
        if done(host) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

#[test]
fn after_a_battle_everyone_goes_back_to_the_lobby_and_can_start_again() {
    let (host, mut client) = connected_pair();
    let lobby = ta_net::Lobby { map: "Test".into(), title: "Rematch".into(), ..Default::default() };
    // The host goes back to the lobby: it takes newcomers again and tells everyone.
    let (mut host, kept) = host.with_lobby(&lobby).into_host().expect("the host's session");
    assert!(host.accepting);
    assert_eq!(kept.as_ref().map(|l| l.title.as_str()), Some("Rematch"));
    host.broadcast(&Msg::Rematch(lobby.clone()));
    // Lobby changes after that reach the player still on their result screen.
    host.broadcast(&Msg::Lobby(ta_net::Lobby { map: "Other".into(), ..lobby }));
    let t = Instant::now();
    while client.lobby().is_none_or(|l| l.map != "Other") && t.elapsed() < Duration::from_secs(5) {
        host.poll();
        client.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(client.rematch, "the player hears the host went back");
    let (mut client, now) = client.into_client().expect("a player follows the host back");
    assert_eq!(now.map, "Other");
    // A new battle starts on the same connection.
    let start = StartInfo { map: "Other".into(), seed: 2, slots: Vec::new(), rules: Default::default(), spectators: Vec::new() };
    host.broadcast(&Msg::Start(start));
    let t = Instant::now();
    let mut started = false;
    while !started && t.elapsed() < Duration::from_secs(5) {
        host.poll();
        started = client.poll().iter().any(|m| matches!(m, Msg::Start(s) if s.map == "Other"));
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(started && client.is_connected(), "a second battle starts on the same connection");
}

#[test]
fn after_a_rematch_a_player_can_still_come_back_to_the_next_battle() {
    let mut host = Host::bind(0, 1).expect("bind");
    let addr = ta_net::resolve(&format!("127.0.0.1:{}", host.port)).unwrap();
    let mut client = Client::connect(addr, "Guest", 1).expect("connect");
    let mut peer = None;
    let t = Instant::now();
    while (peer.is_none() || client.peer.is_none()) && t.elapsed() < Duration::from_secs(5) {
        for e in host.poll() {
            if let HostEvent::Joined(id, _) = e {
                peer = Some(id);
            }
        }
        client.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    let peer = peer.expect("the client should join");
    let token = client.reconnect_token();
    let slot = |kind: SlotKind, color: u8| LobbySlot { kind, side: "ARM".into(), color, ready: true, team: color };
    let start = StartInfo {
        map: "Test".into(),
        seed: 1,
        slots: vec![slot(SlotKind::Human { peer: 0, name: "Host".into() }, 0), slot(SlotKind::Human { peer, name: "Guest".into() }, 1)],
        rules: Default::default(),
        spectators: Vec::new(),
    };
    // A battle, back to the lobby, and a second battle with the same people.
    host.accepting = false;
    let (mut host, _) = Session::host(host, &start).with_lobby(&ta_net::Lobby::default()).into_host().expect("the host's session");
    host.accepting = false;
    let mut host = Session::host(host, &start);
    // The guest drops out of the second battle and comes back with the credential of its
    // first welcome.
    drop(client);
    assert!(pump_host(&mut host, |h| !h.departed.is_empty()), "the host notices the guest left");
    let mut back = Client::connect_with_token(addr, "Guest", 1, token, None).expect("connect");
    let t = Instant::now();
    while back.peer.is_none() && back.rejected.is_none() && t.elapsed() < Duration::from_secs(5) {
        host.poll();
        back.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(back.peer.is_some(), "the guest comes back: {:?}", back.rejected);
}

#[test]
fn a_dropped_player_is_told_so_not_that_the_host_left() {
    let (mut host, mut client) = connected_pair();
    // The host waits for its silent guest, then goes on without them.
    while host.can_produce() {
        host.produce_tick();
    }
    host.drop_waiting();
    assert!(pump(&mut host, &mut client, |_, c| c.host_lost), "the guest's battle ends");
    assert!(client.dropped && !client.link_failed, "dropped, not a failed link");
    assert!(client.departed.is_empty(), "the host did not leave");
}

#[test]
fn a_player_still_in_the_last_battle_goes_into_the_next() {
    let (host, mut client) = connected_pair();
    let lobby = ta_net::Lobby::default();
    let (mut host, _) = host.with_lobby(&lobby).into_host().expect("the host's session");
    host.broadcast(&Msg::Rematch(lobby));
    let slot = |kind: SlotKind, color: u8| LobbySlot { kind, side: "ARM".into(), color, ready: true, team: color };
    let peer = host.peers()[0].0;
    let start = StartInfo { map: "Next".into(), seed: 3, slots: vec![slot(SlotKind::Human { peer: 0, name: "Host".into() }, 0), slot(SlotKind::Human { peer, name: "Guest".into() }, 1)], rules: Default::default(), spectators: Vec::new() };
    host.broadcast(&Msg::Start(start));
    host.broadcast(&Msg::Tick { tick: 1, commands: Vec::new() });
    let t = Instant::now();
    while client.next_battle.is_none() && t.elapsed() < Duration::from_secs(5) {
        host.poll();
        client.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!client.host_lost, "the next battle's ticks are not taken for this one's");
    let (c, info) = client.into_next_battle().expect("the next battle waits");
    assert_eq!(info.map, "Next");
    let mut next = Session::client(c, 1);
    let t = Instant::now();
    while next.backlog() == 0 && t.elapsed() < Duration::from_secs(5) {
        host.poll();
        next.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(next.take_tick(1).is_some(), "its first tick, kept for it");
}

#[test]
fn a_player_back_before_their_old_connection_is_noticed_takes_its_place() {
    let mut host = Host::bind(0, 1).expect("bind");
    let addr = ta_net::resolve(&format!("127.0.0.1:{}", host.port)).unwrap();
    let mut old = Client::connect(addr, "Guest", 1).expect("connect");
    let mut peer = None;
    let t = Instant::now();
    while (peer.is_none() || old.peer.is_none()) && t.elapsed() < Duration::from_secs(5) {
        for e in host.poll() {
            if let HostEvent::Joined(id, _) = e {
                peer = Some(id);
            }
        }
        old.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    let peer = peer.expect("the guest joins");
    let token = old.reconnect_token();
    let slot = |kind: SlotKind, color: u8| LobbySlot { kind, side: "ARM".into(), color, ready: true, team: color };
    let start = StartInfo { map: "Test".into(), seed: 1, slots: vec![slot(SlotKind::Human { peer: 0, name: "Host".into() }, 0), slot(SlotKind::Human { peer, name: "Guest".into() }, 1)], rules: Default::default(), spectators: Vec::new() };
    host.accepting = false;
    let mut host = Session::host(host, &start);
    // The old connection says nothing more (as a dead link); the guest comes back on a new one
    // with its credential: let in at once, the old one gone.
    let mut back = Client::connect_with_token(addr, "Guest", 1, token, None).expect("connect");
    let t = Instant::now();
    while back.peer.is_none() && back.rejected.is_none() && t.elapsed() < Duration::from_secs(5) {
        host.poll();
        back.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(back.peer.is_some(), "the guest is back: {:?}", back.rejected);
    assert!(host.notices.contains(&Notice::Back("Guest".into())), "{:?}", host.notices);
    drop(old);
}
