//! A player coming back to a battle in progress must have the battle's map as the host has it:
//! the lobby's check does not see them again, and maps are not in the data a connection is let in
//! on. Without it, or with another version of it, they are turned away and the battle goes on,
//! their army waiting; with the host's copy they come back as before.

use std::time::{Duration, Instant};

use ta_net::{Client, Host, HostEvent, LobbySlot, Msg, Notice, Refusal, Session, SlotKind, StartInfo};

fn until(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline, "network event did not arrive");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// A host and a guest it has welcomed (data hash 7).
fn joined() -> (Host, Client, u8) {
    let mut host = Host::bind(0, 7).unwrap();
    let mut client = Client::connect(([127, 0, 0, 1], host.port).into(), "Guest", 7).unwrap();
    let mut id = None;
    until(|| {
        for event in host.poll() {
            if let HostEvent::Joined(peer, _) = event {
                id = Some(peer);
            }
        }
        client.poll();
        id.is_some() && client.peer.is_some()
    });
    (host, client, id.unwrap())
}

fn start(peer: u8) -> StartInfo {
    let slot = |peer, name: &str, color| LobbySlot { kind: SlotKind::Human { peer, name: name.into() }, side: "ARM".into(), color, team: color, ready: true };
    StartInfo { map: "pack map".into(), seed: 1, slots: vec![slot(0, "Host", 0), slot(peer, "Guest", 1)], rules: Default::default(), spectators: Vec::new() }
}

/// The guest comes back on a new connection with its credential and tells its maps (as its
/// lobby does on the welcome); what the host answers, the host's session polled meanwhile.
fn come_back(session: &mut Session, address: std::net::SocketAddr, token: Option<ta_net::ReconnectToken>, maps: Vec<(String, u64)>) -> Client {
    let mut back = Client::connect_with_token(address, "Guest", 7, token, None).unwrap();
    until(|| {
        session.poll();
        back.poll();
        back.peer.is_some() || back.rejected.is_some()
    });
    back.send(&Msg::Maps(maps));
    back
}

#[test]
fn a_player_back_without_the_battles_map_is_turned_away_and_the_battle_goes_on() {
    let (host, client, peer) = joined();
    let address = ([127, 0, 0, 1], host.port).into();
    let token = client.reconnect_token();
    let mut session = Session::host(host, &start(peer)).with_map("Pack Map", Some(5));
    drop(client);
    until(|| {
        session.poll();
        !session.departed.is_empty()
    });
    for _ in 0..3 {
        session.produce_tick();
    }
    let departed = session.departed.clone();
    // No copy of the map, then another version of it (the same name, other contents).
    for (maps, other) in [(vec![("coast to coast".to_string(), 9)], false), (vec![("Pack Map".to_string(), 6)], true)] {
        let mut back = come_back(&mut session, address, token, maps);
        until(|| {
            session.poll();
            back.poll();
            back.rejected.is_some()
        });
        assert_eq!(back.rejected, Some(Refusal::Map { map: "Pack Map".into(), other }));
        // The battle goes on as it was: nobody waited for, the player still away, nobody told
        // they came back or left again.
        drop(back);
        let settle = Instant::now();
        while settle.elapsed() < Duration::from_millis(150) {
            session.poll();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(session.can_produce(), "the battle does not wait for them");
        session.produce_tick();
        assert_eq!(session.departed, departed, "no second departure");
        assert!(!session.notices.iter().any(|n| matches!(n, Notice::Back(_))), "{:?}", session.notices);
        assert!(session.away.iter().any(|(player, _)| *player == 1), "their army waits for them: {:?}", session.away);
    }
    // With the host's copy (as their lobby tells it, in any case): back in, the battle so far sent.
    let mut back = come_back(&mut session, address, token, vec![("pack map".to_string(), 5)]);
    let mut resumed = None;
    until(|| {
        session.poll();
        for m in back.poll() {
            if let Msg::Resume { player, upto, .. } = m {
                resumed = Some((player, upto));
            }
        }
        resumed.is_some()
    });
    assert_eq!(resumed.map(|(player, _)| player), Some(1));
    assert!(resumed.is_some_and(|(_, upto)| upto >= 4), "every tick so far: {resumed:?}");
    assert!(session.notices.iter().any(|n| matches!(n, Notice::Back(name) if name == "Guest")));
}

/// A player who never tells their maps is taken to have no copy, after a while; a battle that
/// checks no map (one made without `with_map`) lets a returning player straight back in.
#[test]
fn no_word_of_the_maps_is_no_map_and_a_battle_without_the_check_lets_them_in() {
    for checked in [true, false] {
        let (host, client, peer) = joined();
        let address = ([127, 0, 0, 1], host.port).into();
        let token = client.reconnect_token();
        let mut session = Session::host(host, &start(peer));
        if checked {
            session = session.with_map("Pack Map", Some(5));
            session.arrival_wait = Duration::from_millis(200);
        }
        drop(client);
        until(|| {
            session.poll();
            !session.departed.is_empty()
        });
        // (Says nothing of its maps.)
        let mut back = Client::connect_with_token(address, "Guest", 7, token, None).unwrap();
        let mut resumed = false;
        until(|| {
            session.poll();
            resumed |= back.poll().iter().any(|m| matches!(m, Msg::Resume { player: 1, .. }));
            resumed || back.rejected.is_some()
        });
        if checked {
            assert_eq!(back.rejected, Some(Refusal::Map { map: "Pack Map".into(), other: false }));
        } else {
            assert!(resumed && back.is_connected());
        }
    }
}
