//! Many orders at once, and a long game's history, over loopback (no game data needed).

use std::time::{Duration, Instant};

use ta_net::{BATCH_BYTES, Client, Host, HostEvent, LobbySlot, Msg, Session, SlotKind, StartInfo};
use ta_sim::{Command, FxVec3, Order, UnitId};

/// A host and a client that joined it: (host, its id for the client, the client).
fn joined(name: &str) -> (Host, u8, Client) {
    let mut host = Host::bind(0, 1).expect("bind");
    let addr = ta_net::resolve(&format!("127.0.0.1:{}", host.port)).unwrap();
    let mut client = Client::connect(addr, name, 1).expect("connect");
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
    (host, peer.expect("the client should join"), client)
}

fn start(peer: u8) -> StartInfo {
    let slot = |kind: SlotKind, color: u8| LobbySlot { kind, side: "ARM".into(), color, ready: true, team: color };
    StartInfo {
        map: "Test".into(),
        seed: 1,
        slots: vec![slot(SlotKind::Human { peer: 0, name: "Host".into() }, 0), slot(SlotKind::Human { peer, name: "Guest".into() }, 1)],
        rules: Default::default(),
        spectators: Vec::new(),
    }
}

/// A move order for one unit, each to a spot of its own (as a formation move sends them).
fn order(i: u32) -> Command {
    Command::Order { player: 1, units: vec![UnitId { slot: i, generation: 1 }], order: Order::Move(FxVec3::from_ints(i as i64, 0, 7)), queue: false }
}

/// Sending 5,000 units each somewhere of its own is one message, not 5,000, and the host gets
/// them all in order.
#[test]
fn a_frame_of_orders_goes_in_one_message() {
    let (mut host, _, client) = joined("Guest");
    let mut session = Session::client(client, 1);
    let sent: Vec<Command> = (0..5000).map(order).collect();
    for c in &sent {
        session.submit(c.clone());
    }
    session.flush();
    let (mut got, mut messages) = (Vec::new(), 0);
    let t = Instant::now();
    while got.len() < sent.len() && t.elapsed() < Duration::from_secs(5) {
        for e in host.poll() {
            if let HostEvent::Message(_, Msg::Commands(cmds)) = e {
                messages += 1;
                got.extend(cmds);
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(got, sent, "every order, in order");
    assert_eq!(messages, 1, "one message for the lot");
    // Nothing more goes out until something new is ordered.
    session.flush();
    session.poll();
    std::thread::sleep(Duration::from_millis(50));
    assert!(!host.poll().iter().any(|e| matches!(e, HostEvent::Message(_, Msg::Commands(_)))));
}

/// More than a message holds goes in several, each within the budget.
#[test]
fn orders_past_the_budget_are_split_in_order() {
    let big: Vec<Command> = (0..40).map(|k| Command::Order { player: 1, units: (0..8000).map(|i| UnitId { slot: i, generation: k }).collect(), order: Order::Stop, queue: false }).collect();
    let parts = ta_net::batches(big.clone(), BATCH_BYTES);
    assert!(parts.len() > 1, "{} bytes of orders", bincode::serialized_size(&big).unwrap());
    for p in &parts {
        assert!(p.len() == 1 || bincode::serialized_size(p).unwrap() <= BATCH_BYTES + 8);
    }
    assert_eq!(parts.concat(), big);
    // One larger than the budget goes alone.
    let huge = Command::Order { player: 1, units: (0..200_000).map(|i| UnitId { slot: i, generation: 0 }).collect(), order: Order::Stop, queue: false };
    let parts = ta_net::batches(vec![order(1), huge.clone(), order(2)], BATCH_BYTES);
    assert_eq!(parts, vec![vec![order(1)], vec![huge], vec![order(2)]]);
}

/// A player coming back to a long game (more history than a frame used to hold, 8 MiB) gets
/// all of it, in parts put back together.
#[test]
fn a_long_history_comes_back_whole() {
    let (host, peer, client) = joined("Guest");
    let reconnect = client.reconnect_token();
    let start = start(peer);
    let mut host = Session::host(host, &start);
    let mut client = Session::client(client, 1);
    // 1,500 ticks of the host's orders to a thousand units each: about 12 MB.
    let mut sent = Vec::new();
    for tick in 1..=1500u32 {
        let c = Command::Order { player: 0, units: (0..1000).map(|i| UnitId { slot: i, generation: tick }).collect(), order: Order::Stop, queue: false };
        host.submit(c.clone());
        host.produce_tick();
        sent.push((tick as u64, vec![c]));
    }
    assert!(bincode::serialized_size(&sent).unwrap() > 8 << 20);
    assert!(ta_net::batches(sent.clone(), BATCH_BYTES).len() > 8, "it goes in parts");
    // The guest drops out and comes back.
    client.leave();
    drop(client);
    let t = Instant::now();
    while host.departed.is_empty() && t.elapsed() < Duration::from_secs(5) {
        host.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    let addr = ta_net::resolve(&format!("127.0.0.1:{}", match &host.role {
        ta_net::Role::Host(h) => h.port,
        _ => unreachable!(),
    }))
    .unwrap();
    let mut back = Client::connect_with_token(addr, "Guest", 1, reconnect, "").unwrap();
    let mut resume = None;
    let t = Instant::now();
    while resume.is_none() && t.elapsed() < Duration::from_secs(10) {
        host.poll();
        for m in back.poll() {
            if let Msg::Resume { upto, history, .. } = m {
                resume = Some((upto, history));
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let (upto, history) = resume.expect("the host sends the game so far");
    assert_eq!(upto, 1500);
    assert_eq!(history.len(), sent.len());
    assert!(history == sent, "every tick's orders, in order");
}
