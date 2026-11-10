//! Lifecycle regressions over real sockets, without original game data.
use std::time::{Duration, Instant};

use ta_net::{Client, Host, LobbySlot, Msg, Session, SlotKind, StartInfo};

fn until(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline, "network event did not arrive");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn joined() -> (Host, Client, StartInfo) {
    let mut host = Host::bind(0, 7).unwrap();
    let mut client = Client::connect(([127, 0, 0, 1], host.port).into(), "Guest", 7).unwrap();
    until(|| { host.poll(); client.poll(); client.peer.is_some() });
    let slot = |peer, name: &str, color| LobbySlot {
        kind: SlotKind::Human { peer, name: name.into() }, side: "ARM".into(), color, team: color, ready: true,
    };
    let start = StartInfo { map: "Test".into(), seed: 7,
        slots: vec![slot(0, "Host", 0), slot(client.peer.unwrap(), "Guest", 1)],
        spectators: Vec::new(), rules: Default::default() };
    (host, client, start)
}

#[test]
fn a_credential_replaces_a_stale_connection_without_the_lobby_password() {
    let (mut host, client, start) = joined();
    host.set_password(Some("shield"));
    let address = ([127, 0, 0, 1], host.port).into();
    let mut back = Client::connect_with_token(address, "Guest", 7, client.reconnect_token(), None).unwrap();
    let mut host = Session::host(host, &start);
    let mut resumed = false;
    until(|| {
        host.poll();
        resumed |= back.poll().iter().any(|m| matches!(m, Msg::Resume { player: 1, .. }));
        assert!(back.rejected.is_none(), "a returning credential needs no password: {:?}", back.rejected);
        resumed
    });
    assert_ne!(back.peer, client.peer);
    assert!(host.away.is_empty());
}

#[test]
fn an_early_peer_hash_is_compared_when_the_host_reports() {
    let (host, mut client, start) = joined();
    let mut host = Session::host(host, &start);
    for _ in 0..15 { host.produce_tick(); }
    client.send(&Msg::Hash { tick: 15, hash: 999 });
    client.send(&Msg::Chat { from: String::new(), text: "after hash".into() });
    until(|| { host.poll(); !host.chat.is_empty() });
    assert!(host.desync.is_none());
    client.send(&Msg::Bye);
    until(|| { host.poll(); !host.departed.is_empty() });
    host.report_hash(15, 123);
    assert_eq!(host.desync, Some(15), "arrival order must not hide divergence");
}

#[test]
fn an_early_hash_mismatch_is_announced_to_connected_players() {
    let (host, mut client, start) = joined();
    let mut host = Session::host(host, &start);
    for _ in 0..15 { host.produce_tick(); }
    client.send(&Msg::Hash { tick: 15, hash: 999 });
    client.send(&Msg::Chat { from: String::new(), text: "after hash".into() });
    until(|| { host.poll(); !host.chat.is_empty() });
    host.report_hash(15, 123);
    let mut announced = false;
    until(|| {
        announced |= client.poll().iter().any(|m| matches!(m, Msg::Desync { tick: 15 }));
        announced
    });
}

#[test]
fn a_large_local_command_burst_does_not_disconnect_the_other_players() {
    let (host, client, start) = joined();
    let mut host = Session::host(host, &start);
    let mut client = Session::client(client, 1);
    for _ in 0..240_000 {
        host.submit(ta_sim::Command::SetActive { player: 0, units: Vec::new(), on: true });
    }
    host.produce_tick();
    let expected = host.take_tick(1).unwrap();
    assert!(!expected.is_empty());
    let mut commands = None;
    until(|| {
        client.poll();
        commands = client.take_tick(1).or(commands.take());
        assert!(!client.host_lost, "a local burst must stay within the receiving client's budget");
        commands.is_some()
    });
    assert_eq!(commands.unwrap(), expected, "every peer executes the same accepted prefix");
}

#[test]
fn a_full_wire_batch_keeps_its_first_orders() {
    let (host, client, start) = joined();
    let mut host = Session::host(host, &start);
    let mut client = Session::client(client, 1);
    // Large unit lists fill a wire batch before reaching the command-count allowance.
    for i in 0..240 {
        client.submit(ta_sim::Command::SetActive { player: 1,
            units: (0..1138).map(|slot| ta_sim::UnitId { slot, generation: i }).collect(), on: true });
    }
    client.flush();
    client.send_chat("Guest", "after orders");
    until(|| { host.poll(); client.poll(); !host.chat.is_empty() });
    host.produce_tick();
    let accepted = host.take_tick(1).unwrap();
    assert!(!accepted.is_empty());
    assert!(matches!(&accepted[0], ta_sim::Command::SetActive { units, .. } if units[0].generation == 0),
        "the batch's container overhead must not discard its earliest orders");
}

#[test]
fn simultaneous_player_bursts_fit_every_receivers_memory_budget() {
    let (mut host, first, mut start) = joined();
    let address = ([127, 0, 0, 1], host.port).into();
    let mut clients = vec![first];
    for player in 2..10 {
        let name = format!("Guest{player}");
        let mut client = Client::connect(address, &name, 7).unwrap();
        until(|| { host.poll(); client.poll(); client.peer.is_some() });
        start.slots.push(LobbySlot { kind: SlotKind::Human { peer: client.peer.unwrap(), name },
            side: "ARM".into(), color: player, team: player, ready: true });
        clients.push(client);
    }
    let mut host = Session::host(host, &start);
    for (i, client) in clients.iter_mut().enumerate() {
        client.send(&Msg::Commands(vec![ta_sim::Command::SetActive { player: (i + 1) as u8, units: Vec::new(), on: true }; 20_000]));
        client.send(&Msg::Chat { from: String::new(), text: "after burst".into() });
    }
    until(|| { host.poll(); host.chat.len() == clients.len() });
    host.produce_tick();
    let expected = host.take_tick(1).unwrap();
    assert!(!expected.is_empty());
    for (i, client) in clients.into_iter().enumerate() {
        let mut client = Session::client(client, (i + 1) as u8);
        let mut got = None;
        until(|| {
            client.poll();
            got = client.take_tick(1).or(got.take());
            assert!(!client.host_lost, "player {i} must be able to receive the host's accepted bundle");
            got.is_some()
        });
        assert_eq!(got.unwrap(), expected);
    }
}

#[test]
fn returning_to_the_lobby_cannot_lose_a_rematch_that_already_started() {
    let (host, client, start) = joined();
    let lobby = ta_net::Lobby { map: start.map.clone(), slots: start.slots.clone(), ..Default::default() };
    let host = Session::host(host, &start).with_lobby(&lobby);
    let (mut host, _) = host.into_host().unwrap();
    let mut client = Session::client(client, 1);
    host.broadcast(&Msg::Rematch(lobby));
    let next = StartInfo { map: "Next".into(), seed: 8, ..start };
    host.broadcast(&Msg::Start(next.clone()));
    host.broadcast(&Msg::Tick { tick: 1, commands: vec![ta_sim::Command::Resign { player: 0 }] });
    until(|| { host.poll(); client.poll(); client.next_battle.is_some() });
    // The user clicked BACK TO THE LOBBY in the frame that read the next Start.
    let (mut client, _) = client.into_client().unwrap();
    let messages = client.poll();
    assert!(matches!(messages.as_slice(), [Msg::Start(info)] if *info == next), "the lobby must still receive the next battle's start");
    let mut client = Session::client(client, 1);
    let mut tick = None;
    until(|| { host.poll(); client.poll(); tick = client.take_tick(1).or(tick.take()); tick.is_some() });
    assert_eq!(tick.unwrap(), [ta_sim::Command::Resign { player: 0 }]);
    assert!(!client.host_lost);
}

#[test]
fn a_disconnected_players_id_is_not_reused_by_strangers_during_a_battle() {
    let (mut host, mut client, mut start) = joined();
    let address = ([127, 0, 0, 1], host.port).into();
    let old = client.peer.unwrap();
    let mut other = Client::connect(address, "Other", 7).unwrap();
    until(|| { host.poll(); other.poll(); other.peer.is_some() });
    let token = other.reconnect_token();
    start.slots.push(LobbySlot { kind: SlotKind::Human { peer: other.peer.unwrap(), name: "Other".into() },
        side: "CORE".into(), color: 2, team: 2, ready: true });
    let mut host = Session::host(host, &start);
    client.send(&Msg::Bye);
    other.send(&Msg::Bye);
    drop(client);
    drop(other);
    until(|| { host.poll(); host.departed.len() == 2 });
    // Rejected handshakes also consume peer numbers; wrap the allocator around.
    for _ in 0..253 {
        let mut stranger = Client::connect(address, "Stranger", 7).unwrap();
        until(|| { host.poll(); stranger.poll(); stranger.rejected.is_some() });
    }
    let mut back = Client::connect_with_token(address, "Other", 7, token, None).unwrap();
    let mut info = None;
    until(|| {
        host.poll();
        for message in back.poll() {
            if let Msg::Resume { start, .. } = message { info = Some(start); }
        }
        assert!(back.is_connected(), "the returning roster must have unique peer numbers");
        info.is_some()
    });
    let info = info.unwrap();
    assert_eq!(info.player_of(back.peer.unwrap()), Some(2));
    assert_ne!(back.peer, Some(old), "Guest still owns this roster number");
    assert_eq!(host.player_name(1), Some("Guest"));
    assert_eq!(host.player_name(2), Some("Other"));
    assert_eq!(host.host_player(), Some(0));
    assert_ne!(old, 0);
}
