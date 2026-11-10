//! The lobby's traffic over a real loopback connection: the host measures every peer's round
//! trip and shares the table, peers answer on their own; a picture too large is refused.

use std::time::{Duration, Instant};

use ta_net::{Client, Host, HostEvent, Msg};

/// A host and a client it has welcomed.
fn pair() -> (Host, Client) {
    let mut host = Host::bind(0, 7).expect("bind");
    let addr = ta_net::resolve(&format!("127.0.0.1:{}", host.port)).unwrap();
    let mut client = Client::connect(addr, "Ironside", 7).expect("connect");
    let t0 = Instant::now();
    let mut joined = false;
    while (!joined || client.peer.is_none()) && t0.elapsed() < Duration::from_secs(5) {
        joined |= host.poll().iter().any(|e| matches!(e, HostEvent::Joined(..)));
        client.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(joined && client.peer.is_some(), "the client is welcomed");
    (host, client)
}

#[test]
fn the_host_measures_and_shares_everyones_ping() {
    let (mut host, mut client) = pair();
    let peer = client.peer.unwrap();
    // Within a couple of ping intervals the host has the client's round trip, and the client
    // has the host's table (the host's own 0, its own measured).
    let t0 = Instant::now();
    while (client.pings().len() < 2) && t0.elapsed() < ta_net::PING_INTERVAL * 4 {
        host.poll();
        client.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    let table = client.pings().to_vec();
    assert!(table.iter().any(|&(p, ms)| p == 0 && ms == 0), "{table:?}");
    let ours = table.iter().find(|(p, _)| *p == peer).map(|(_, ms)| *ms);
    assert!(ours.is_some_and(|ms| ms < 1000), "a loopback round trip: {table:?}");
    assert!(host.pings().iter().any(|(p, _)| *p == peer));
}

#[test]
fn a_seat_and_a_picture_reach_the_host_and_a_huge_picture_ends_the_connection() {
    let (mut host, mut client) = pair();
    let peer = client.peer.unwrap();
    client.send(&Msg::TakeSlot(3));
    client.send(&Msg::Portrait(vec![1; 100]));
    let mut seen = Vec::new();
    let t0 = Instant::now();
    while seen.len() < 2 && t0.elapsed() < Duration::from_secs(5) {
        for e in host.poll() {
            if let HostEvent::Message(p, m) = e {
                assert_eq!(p, peer);
                seen.push(m);
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(matches!(seen.as_slice(), [Msg::TakeSlot(3), Msg::Portrait(png)] if png.len() == 100), "{seen:?}");
    client.send(&Msg::Portrait(vec![1; ta_net::PORTRAIT_BYTES + 1]));
    let t0 = Instant::now();
    let mut left = false;
    while !left && t0.elapsed() < Duration::from_secs(5) {
        left = host.poll().iter().any(|e| matches!(e, HostEvent::Left(p, _) if *p == peer));
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(left, "a picture over the limit closes the connection");
}

#[test]
fn a_removed_peer_is_told_so() {
    let (mut host, mut client) = pair();
    let peer = client.peer.unwrap();
    host.send_to(peer, &Msg::Removed);
    host.kick(peer);
    let t0 = Instant::now();
    while !client.removed && t0.elapsed() < Duration::from_secs(5) {
        host.poll();
        client.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(client.removed);
}

#[test]
fn a_locked_lobby_takes_only_its_password() {
    let mut host = Host::bind(0, 7).expect("bind");
    host.set_password(Some("  shield  "));
    assert!(host.locked());
    let addr = ta_net::resolve(&format!("127.0.0.1:{}", host.port)).unwrap();
    let mut wrong = Client::connect_with_token(addr, "Nova", 7, None, Some("sword")).expect("connect");
    let mut right = Client::connect_with_token(addr, "Hex", 7, None, Some("shield")).expect("connect");
    let t0 = Instant::now();
    while (wrong.rejected.is_none() || right.peer.is_none()) && t0.elapsed() < Duration::from_secs(5) {
        host.poll();
        wrong.poll();
        right.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(wrong.rejected, Some(ta_net::Refusal::Password));
    assert!(right.peer.is_some(), "the right password is welcomed: {:?}", right.rejected);
}

/// A player whose data differs from the host's is turned away with the host's mods, to be told
/// which of theirs differ; one whose data is the host's is let in.
#[test]
fn a_player_with_other_data_is_told_the_hosts_mods() {
    let mut host = Host::bind(0, 7).expect("bind");
    let esc = ta_net::NetMod { id: "escalation".into(), name: "TA: Escalation\u{7}".into(), version: "10.2".into(), digest: [3; 16], data: true };
    host.set_mods(vec![esc.clone()]);
    let addr = ta_net::resolve(&format!("127.0.0.1:{}", host.port)).unwrap();
    let mut other = Client::connect(addr, "Nova", 8).expect("connect");
    let mut same = Client::connect(addr, "Hex", 7).expect("connect");
    let t0 = Instant::now();
    while (other.rejected.is_none() || same.peer.is_none()) && t0.elapsed() < Duration::from_secs(5) {
        host.poll();
        other.poll();
        same.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    // (Its words cleaned for the screen.)
    let told = ta_net::NetMod { name: "TA: Escalation".into(), ..esc };
    assert_eq!(other.rejected, Some(ta_net::Refusal::Data { mods: vec![told] }));
    assert!(same.peer.is_some(), "the same data is welcomed: {:?}", same.rejected);
}

/// The maps a player tells the host are kept for the connection: the host can say, for any map
/// it chooses, whether they have it and the same contents. Too many maps end the connection.
#[test]
fn the_host_keeps_the_maps_a_player_has() {
    let (mut host, mut client) = pair();
    let peer = client.peer.unwrap();
    assert_eq!(host.peer_map(peer, "coast to coast"), None, "not told yet");
    client.send(&Msg::Maps(vec![("coast to coast".into(), 5), ("Pack Map".into(), 9)]));
    let t0 = Instant::now();
    while host.peer_map(peer, "pack map").is_none() && t0.elapsed() < Duration::from_secs(5) {
        host.poll();
        client.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(host.peer_map(peer, "Pack Map"), Some(Some(9)), "by its stem, in any case");
    assert_eq!(host.peer_map(peer, "coast to coast"), Some(Some(5)));
    assert_eq!(host.peer_map(peer, "painted desert"), Some(None), "told, and not among them");
    assert_eq!(host.peer_map(99, "coast to coast"), None, "nobody of that number");
    client.send(&Msg::Maps((0..=ta_net::MAX_MAPS).map(|i| (format!("map {i}"), i as u64)).collect()));
    let t0 = Instant::now();
    let mut left = false;
    while !left && t0.elapsed() < Duration::from_secs(5) {
        left = host.poll().iter().any(|e| matches!(e, HostEvent::Left(p, _) if *p == peer));
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(left, "a list over the limit closes the connection");
}

#[test]
fn a_joiner_waits_for_the_hosts_answer() {
    let mut host = Host::bind(0, 7).expect("bind");
    host.set_password(Some("shield"));
    let addr = ta_net::resolve(&format!("127.0.0.1:{}", host.port)).unwrap();
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let polling = {
        let done = done.clone();
        std::thread::spawn(move || {
            let t0 = Instant::now();
            while !done.load(std::sync::atomic::Ordering::Relaxed) && t0.elapsed() < Duration::from_secs(10) {
                host.poll();
                std::thread::sleep(Duration::from_millis(2));
            }
        })
    };
    let mut wrong = Client::connect_with_token(addr, "Nova", 7, None, Some("sword")).expect("connect");
    wrong.wait_for_answer(Duration::from_secs(5)).expect("an answer");
    assert_eq!(wrong.rejected, Some(ta_net::Refusal::Password));
    let mut right = Client::connect_with_token(addr, "Hex", 7, None, Some("shield")).expect("connect");
    right.wait_for_answer(Duration::from_secs(5)).expect("an answer");
    assert_eq!(right.rejected, None);
    assert!(right.peer.is_none(), "the welcome is left for poll");
    right.poll();
    assert!(right.peer.is_some());
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    polling.join().unwrap();
}
