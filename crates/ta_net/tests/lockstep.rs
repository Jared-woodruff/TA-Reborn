//! Two simulations kept in lockstep over a real loopback TCP connection (skipped without
//! a TA install). Each side's orders come from an AI that only exists on that machine, so
//! every command crosses the network exactly like a human's would.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_net::{Client, Host, HostEvent, LobbySlot, Msg, Session, SlotKind, StartInfo};
use ta_sim::{Fx, FxVec3, PlayerSetup, Sim, UnitTypes};

fn build_sim(vfs: &ta_vfs::Vfs, data: &Arc<GameData>, types: &Arc<UnitTypes>, map_name: &str, seed: u64) -> Sim {
    let map = TntMap::parse(&vfs.read(&format!("maps/{map_name}.tnt")).unwrap()).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read(&format!("maps/{map_name}.ota")).unwrap()));
    let starts: Vec<FxVec3> = info.schemas[0].start_positions.iter().map(|&(_, x, z)| FxVec3::new(Fx::from_f32(x), Fx::ZERO, Fx::from_f32(z))).collect();
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup {
            side: if i == 0 { "ARM".into() } else { "CORE".into() },
            color: i as u8,
            start: starts[i],
            metal: Fx::from_int(1000),
            energy: Fx::from_int(1000),
            ai: false,
        })
        .collect();
    let mut sim = Sim::new(types.clone(), data.clone(), &map, &info, 0, &players, seed);
    sim.commander_ends_game = true;
    for (i, name) in ["ARMCOM", "CORCOM"].iter().enumerate() {
        sim.spawn_unit(types.id(name).unwrap(), i as u8, starts[i], 0, true);
    }
    sim
}

#[test]
fn two_peers_stay_in_sync() {
    let Some(install) = ta_vfs::install::discover(None).ok() else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));

    // Lobby handshake over loopback.
    let mut host = Host::bind(0, 42).expect("bind");
    let addr = ta_net::resolve(&format!("127.0.0.1:{}", host.port)).unwrap();
    let mut client = Client::connect(addr, "Tester", 42).expect("connect");
    let t0 = Instant::now();
    let mut joined = None;
    while (joined.is_none() || client.peer.is_none()) && t0.elapsed() < Duration::from_secs(5) {
        for e in host.poll() {
            if let HostEvent::Joined(id, name) = e {
                joined = Some((id, name));
            }
        }
        client.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    let (peer, name) = joined.expect("the client should join");
    assert_eq!(name, "Tester");
    assert_eq!(client.peer, Some(peer));

    // A second client with different game data is turned away.
    let mut stranger = Client::connect(addr, "Modded", 7).unwrap();
    let t1 = Instant::now();
    while stranger.rejected.is_none() && t1.elapsed() < Duration::from_secs(5) {
        host.poll();
        stranger.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(stranger.rejected.is_some(), "mismatched data must be rejected");

    let map = "Coast To Coast".to_string();
    let slot = |kind: SlotKind, side: &str, color: u8| LobbySlot { kind, side: side.into(), color, ready: true, team: color };
    let start = StartInfo {
        map: map.clone(),
        seed: 1234,
        slots: vec![slot(SlotKind::Human { peer: 0, name: "Host".into() }, "ARM", 0), slot(SlotKind::Human { peer, name }, "CORE", 1)],
        rules: Default::default(),
        spectators: Vec::new(),
    };
    host.accepting = false;
    host.broadcast(&Msg::Start(start.clone()));
    let mut got_start = None;
    let t2 = Instant::now();
    while got_start.is_none() && t2.elapsed() < Duration::from_secs(5) {
        for m in client.poll() {
            if let Msg::Start(s) = m {
                got_start = Some(s);
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let got_start = got_start.expect("start info");
    assert_eq!(got_start, start);
    let client_player = got_start.player_of(peer).unwrap();
    assert_eq!(client_player, 1);

    let mut host_sess = Session::host(host, &start);
    let mut client_sess = Session::client(client, client_player);
    let mut host_sim = build_sim(&vfs, &data, &types, &map, start.seed);
    let mut client_sim = build_sim(&vfs, &data, &types, &map, start.seed);
    // Each "human" exists on one machine only.
    let mut brain0 = ta_ai::SkirmishAi::new(0, &host_sim, ta_ai::Difficulty::Hard);
    let mut brain1 = ta_ai::SkirmishAi::new(1, &client_sim, ta_ai::Difficulty::Hard);

    let ticks = 30 * 60 * 4;
    let mut sent = [0usize; 2];
    let started = Instant::now();
    for tick in 1..=ticks {
        // Host machine: its player's orders, then the bundle for this tick.
        for c in brain0.think(&host_sim) {
            sent[0] += 1;
            host_sess.submit(c);
        }
        host_sess.poll();
        host_sess.produce_tick();
        let cmds = host_sess.take_tick(tick).expect("host has its own bundle");
        host_sim.step(&cmds);
        host_sess.report_hash(tick, host_sim.checksum());

        // Client machine: its player's orders, then wait for the host's bundle.
        for c in brain1.think(&client_sim) {
            sent[1] += 1;
            client_sess.submit(c);
        }
        let wait = Instant::now();
        let cmds = loop {
            client_sess.poll();
            if let Some(c) = client_sess.take_tick(tick) {
                break c;
            }
            assert!(wait.elapsed() < Duration::from_secs(5), "bundle {tick} never arrived");
            std::thread::yield_now();
        };
        client_sim.step(&cmds);
        client_sess.report_hash(tick, client_sim.checksum());
        if tick % 1800 == 0 {
            assert_eq!(host_sim.checksum(), client_sim.checksum(), "diverged by tick {tick}");
        }
    }
    // Let the last hash reports reach the host.
    let t3 = Instant::now();
    while t3.elapsed() < Duration::from_millis(200) {
        host_sess.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    let units = |s: &Sim, p: u8| s.units().filter(|u| u.owner == p).count();
    println!(
        "{ticks} ticks in {:.1} s; orders sent {sent:?}; units host view {}+{}, client view {}+{}; desync {:?}",
        started.elapsed().as_secs_f64(),
        units(&host_sim, 0),
        units(&host_sim, 1),
        units(&client_sim, 0),
        units(&client_sim, 1),
        host_sess.desync
    );
    assert!(sent[0] > 10 && sent[1] > 10, "both players should have issued orders");
    assert!(units(&host_sim, 1) > 3, "the client's orders must reach the host's simulation");
    assert_eq!(host_sim.checksum(), client_sim.checksum(), "peers must stay identical");
    assert!(host_sess.desync.is_none());

    // In-game chat: a client's line reaches the host and comes back relayed; the host's
    // own lines go straight out.
    client_sess.send_chat("Guest", "  gg\u{7} wp  ");
    host_sess.send_chat("Host", "thanks");
    let t4 = Instant::now();
    while t4.elapsed() < Duration::from_secs(2) && (host_sess.chat.len() < 2 || client_sess.chat.len() < 2) {
        host_sess.poll();
        client_sess.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    println!("host chat {:?}; client chat {:?}", host_sess.chat, client_sess.chat);
    let line = |who: &str, text: &str| (who.to_string(), text.to_string());
    assert!(host_sess.chat.contains(&line("Tester", "gg wp")), "control characters and padding are dropped, and the connected name replaces a claimed sender");
    assert!(client_sess.chat.contains(&line("Tester", "gg wp")), "the sender sees its own line relayed");
    assert!(client_sess.chat.contains(&line("Host", "thanks")));
}
