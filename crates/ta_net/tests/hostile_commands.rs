//! A peer's commands are whatever its program sends: a modified or broken build may send any
//! command with any values. The host bundles those for its own player and every machine runs
//! them, so none may stop the battle (a panic would stop it on every machine at once) or play
//! out differently on two machines. Real game data over loopback TCP (skipped without a TA
//! install).

use std::sync::Arc;
use std::time::{Duration, Instant};

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_net::{Client, Host, HostEvent, LobbySlot, Session, SlotKind, StartInfo};
use ta_sim::formation::Shape;
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sharing, SignalKind, Sim, Target, UnitId, UnitTypes};

/// xorshift64*: the same values on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, one_in: u64) -> bool {
        self.below(one_in) == 0
    }
}

const MAP: &str = "Coast To Coast";

fn build_sim(vfs: &ta_vfs::Vfs, data: &Arc<GameData>, types: &Arc<UnitTypes>, seed: u64) -> Sim {
    let map = TntMap::parse(&vfs.read(&format!("maps/{MAP}.tnt")).unwrap()).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read(&format!("maps/{MAP}.ota")).unwrap()));
    let starts: Vec<FxVec3> = info.schemas[0].start_positions.iter().map(|&(_, x, z)| FxVec3::new(Fx::from_f32(x), Fx::ZERO, Fx::from_f32(z))).collect();
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup { side: if i == 0 { "ARM".into() } else { "CORE".into() }, color: i as u8, start: starts[i], metal: Fx::from_int(5000), energy: Fx::from_int(5000), ai: false })
        .collect();
    let mut sim = Sim::new(types.clone(), data.clone(), &map, &info, 0, &players, seed);
    // (The battle goes on whatever happens to the Commanders: there is more to send.)
    sim.commander_ends_game = false;
    sim.players.iter_mut().for_each(|p| p.endless = true);
    spawn_armies(&mut sim, 0);
    sim
}

/// Each side's army (a Commander, factories, builders, a transport, switchable and stockpiling
/// buildings, guns), the same on every machine; again every so often (`round`), as the
/// commands take them apart.
fn spawn_armies(sim: &mut Sim, round: i64) {
    let armies = [
        ["ARMCOM", "ARMLAB", "ARMVP", "ARMAP", "ARMCK", "ARMATLAS", "ARMMAKR", "ARMSILO", "ARMRAD", "ARMLLT", "ARMPW", "ARMSOLAR"],
        ["CORCOM", "CORLAB", "CORVP", "CORAP", "CORCK", "CORVALK", "CORMAKR", "CORSILO", "CORRAD", "CORLLT", "CORAK", "CORSOLAR"],
    ];
    let starts: Vec<FxVec3> = sim.players.iter().map(|p| p.start).collect();
    for (owner, army) in armies.iter().enumerate() {
        for (i, name) in army.iter().enumerate() {
            let Some(t) = sim.types.id(name) else { continue };
            let at = starts[owner] + FxVec3::from_ints((i as i64 % 4) * 96 - 144 + round * 24, 0, (i as i64 / 4) * 96 - 96 + round * 16);
            sim.spawn_unit(t, owner as u8, at, 0, true);
        }
    }
}

/// What one machine knows to send commands about: the units there are, the types and the map.
struct Known {
    units: Vec<UnitId>,
    types: usize,
    size: i64,
}

impl Known {
    fn of(sim: &Sim) -> Known {
        let units = sim.units().map(|u| u.id).collect();
        let size = sim.world.width.max(sim.world.height) as i64 * 16;
        Known { units, types: sim.types.types.len(), size }
    }
}

fn fx(rng: &mut Rng) -> Fx {
    match rng.below(8) {
        0 => Fx::MAX,
        1 => Fx::MIN,
        2 => Fx(-(rng.below(1 << 30) as i64)),
        3 => Fx::ZERO,
        _ => Fx::from_int(rng.below(20_000) as i64),
    }
}

fn pos(rng: &mut Rng, k: &Known) -> FxVec3 {
    if rng.chance(6) {
        return FxVec3::new(fx(rng), fx(rng), fx(rng));
    }
    let coord = |rng: &mut Rng| Fx::from_int(rng.below(k.size as u64 + 400) as i64 - 200);
    FxVec3::new(coord(rng), Fx::from_int(rng.below(400) as i64), coord(rng))
}

fn unit(rng: &mut Rng, k: &Known) -> UnitId {
    match rng.below(6) {
        0 => UnitId { slot: rng.next() as u32, generation: rng.next() as u32 },
        1 => UnitId { slot: rng.below(64) as u32, generation: rng.below(3) as u32 },
        _ if !k.units.is_empty() => k.units[rng.below(k.units.len() as u64) as usize],
        _ => UnitId { slot: 0, generation: 0 },
    }
}

fn units(rng: &mut Rng, k: &Known) -> Vec<UnitId> {
    let n = match rng.below(10) {
        0 => 0,
        1 => 40,
        _ => 1 + rng.below(4) as usize,
    };
    (0..n).map(|_| unit(rng, k)).collect()
}

fn type_id(rng: &mut Rng, k: &Known) -> usize {
    match rng.below(10) {
        0 => usize::MAX,
        1 => k.types + rng.below(10) as usize,
        _ => rng.below(k.types as u64) as usize,
    }
}

fn index(rng: &mut Rng) -> usize {
    match rng.below(5) {
        0 => usize::MAX,
        1 => rng.next() as usize,
        _ => rng.below(400) as usize,
    }
}

fn target(rng: &mut Rng, k: &Known) -> Target {
    match rng.below(4) {
        0 => Target::Projectile(rng.next() as u32),
        1 => Target::Ground(pos(rng, k)),
        _ => Target::Unit(unit(rng, k)),
    }
}

fn order(rng: &mut Rng, k: &Known) -> Order {
    match rng.below(19) {
        0 => Order::Move(pos(rng, k)),
        1 => Order::Attack(target(rng, k)),
        2 => Order::Fight(pos(rng, k)),
        3 => Order::Reclaim(index(rng)),
        4 => Order::Resurrect(index(rng)),
        5 => Order::ReclaimUnit(unit(rng, k)),
        6 => Order::ManualFire(target(rng, k)),
        7 => Order::Build { type_id: type_id(rng, k), pos: pos(rng, k) },
        8 => Order::Repair(unit(rng, k)),
        9 => Order::Patrol(pos(rng, k)),
        10 => Order::Guard(unit(rng, k)),
        11 => Order::Capture(unit(rng, k)),
        12 => Order::Wait(if rng.chance(2) { u32::MAX } else { rng.below(200) as u32 }),
        13 => Order::WaitAttacked(if rng.chance(2) { Some(unit(rng, k)) } else { None }),
        14 => Order::AttackType(type_id(rng, k)),
        15 => Order::SetStates { fire: rng.next() as u8, movement: rng.next() as u8 },
        16 => Order::Load(unit(rng, k)),
        17 => Order::Unload(pos(rng, k)),
        _ => Order::Stop,
    }
}

fn shape(rng: &mut Rng, k: &Known) -> Shape {
    match rng.below(4) {
        0 => Shape::Rows { facing: Some(rng.next() as i32) },
        1 => Shape::Rows { facing: None },
        2 => Shape::Keep,
        _ => Shape::Spots((0..rng.below(50)).map(|_| pos(rng, k)).collect()),
    }
}

/// Any command a peer might send for `player` (sometimes for somebody else, which the host
/// must refuse); never a resignation, which would leave nothing to command.
fn command(rng: &mut Rng, k: &Known, player: u8) -> Command {
    let player = if rng.chance(12) { rng.next() as u8 } else { player };
    match rng.below(20) {
        0..=3 => Command::Order { player, units: units(rng, k), order: order(rng, k), queue: rng.chance(2) },
        4 => Command::Produce { player, factory: unit(rng, k), type_id: type_id(rng, k), count: match rng.below(4) { 0 => i32::MIN, 1 => i32::MAX, _ => rng.below(9) as i32 - 4 } },
        5 => Command::SetRally { player, factory: unit(rng, k), pos: pos(rng, k) },
        6 => Command::SetActive { player, units: units(rng, k), on: rng.chance(2) },
        7 => Command::SetCloak { player, units: units(rng, k), on: rng.chance(2) },
        8 => Command::SetStates { player, units: units(rng, k), fire: rng.chance(2).then(|| rng.next() as u8), movement: rng.chance(2).then(|| rng.next() as u8) },
        // (Seldom: each one blows up what it names.)
        9 if rng.chance(8) => Command::SelfDestruct { player, units: units(rng, k) },
        9 | 10 => Command::SetRepeat { player, factories: units(rng, k), on: rng.chance(2) },
        11 => Command::Share { player, to: rng.next() as u8 % 4, metal: fx(rng), energy: fx(rng) },
        12 => Command::GiveUnits { player, units: units(rng, k), to: rng.next() as u8 % 4 },
        13 => Command::QueueFront { player, factory: unit(rng, k), index: rng.next() as u32 },
        14 => Command::SetSharing { player, sharing: Sharing { metal: rng.chance(2), energy: rng.chance(2), keep_metal: rng.chance(2).then(|| fx(rng)), keep_energy: rng.chance(2).then(|| fx(rng)) } },
        15 => Command::Formation { player, units: units(rng, k), order: order(rng, k), queue: rng.chance(2), shape: shape(rng, k) },
        16 => Command::Signal { player, kind: SignalKind::ALL[rng.below(6) as usize], at: pos(rng, k) },
        17 => Command::EditQueue { player, unit: unit(rng, k), index: rng.next() as u32, type_id: type_id(rng, k), pos: rng.chance(2).then(|| pos(rng, k)), to_front: rng.chance(2) },
        18 if rng.chance(2) => Command::SetShootAll { player, on: rng.chance(2) },
        // (The host's own commands, and cheats: never a player's.)
        18 => Command::Abandon { player },
        _ => Command::Cheat { player, cheat: ta_sim::Cheat::Destroy(units(rng, k)) },
    }
}

#[test]
fn a_peers_hostile_commands_never_stop_or_split_the_battle() {
    let Some(install) = ta_vfs::install::discover(None).ok() else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));

    let mut host = Host::bind(0, 42).expect("bind");
    let addr = ta_net::resolve(&format!("127.0.0.1:{}", host.port)).unwrap();
    let mut client = Client::connect(addr, "Guest", 42).expect("connect");
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
    let peer = peer.expect("the guest joins");
    let slot = |kind: SlotKind, side: &str, color: u8| LobbySlot { kind, side: side.into(), color, ready: true, team: color };
    let start = StartInfo {
        map: MAP.into(),
        seed: 77,
        slots: vec![slot(SlotKind::Human { peer: 0, name: "Host".into() }, "ARM", 0), slot(SlotKind::Human { peer, name: "Guest".into() }, "CORE", 1)],
        rules: Default::default(),
        spectators: Vec::new(),
    };
    host.accepting = false;
    let mut host_sess = Session::host(host, &start);
    let mut client_sess = Session::client(client, 1);
    let mut host_sim = build_sim(&vfs, &data, &types, start.seed);
    let mut client_sim = build_sim(&vfs, &data, &types, start.seed);

    // Both players send anything at all, a few commands a tick: the guest's go to the host
    // over the network, the host's are its own.
    let (mut guest_rng, mut host_rng) = (Rng(0x9e37_79b9_7f4a_7c15), Rng(0xd1b5_4a32_d192_ed03));
    let ticks = 1800;
    let mut sent = 0usize;
    let mut executed = 0usize;
    for tick in 1..=ticks {
        if tick % 200 == 0 {
            for sim in [&mut host_sim, &mut client_sim] {
                spawn_armies(sim, (tick / 200) as i64);
            }
        }
        let known = Known::of(&client_sim);
        for _ in 0..guest_rng.below(4) {
            client_sess.submit(command(&mut guest_rng, &known, 1));
            sent += 1;
        }
        let known = Known::of(&host_sim);
        for _ in 0..host_rng.below(3) {
            host_sess.submit(command(&mut host_rng, &known, 0));
            sent += 1;
        }
        client_sess.flush();
        // The host takes in what has come (the guest's commands a frame or so later).
        let wait = Instant::now();
        while wait.elapsed() < Duration::from_millis(1) {
            host_sess.poll();
        }
        host_sess.produce_tick();
        let cmds = host_sess.take_tick(tick).expect("the host's own bundle");
        executed += cmds.len();
        assert!(cmds.iter().all(|c| !matches!(c, Command::Abandon { .. } | Command::Cheat { .. })), "a player's abandonment and cheats are the host's to refuse");
        host_sim.step(&cmds);
        host_sess.report_hash(tick, host_sim.checksum());
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
        if tick % 150 == 0 {
            assert_eq!(host_sim.checksum(), client_sim.checksum(), "the machines differ by tick {tick}");
        }
    }
    let t = Instant::now();
    while t.elapsed() < Duration::from_millis(200) {
        host_sess.poll();
        client_sess.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    println!("{ticks} ticks, {sent} commands sent, {executed} executed; units {}; desync {:?}", host_sim.units().count(), host_sess.desync);
    assert!(executed as u64 > ticks / 2, "commands must get through for the test to mean anything");
    assert_eq!(host_sim.checksum(), client_sim.checksum());
    assert!(host_sess.desync.is_none() && client_sess.desync.is_none());
    assert!(client_sess.backlog() == 0 && !client_sess.host_lost, "the guest stays in the battle");
}
