//! Asset-free playback regressions shared by ordinary playback and fast-forwarding.

use super::*;

fn fixture() -> (Sim, [UnitId; 2]) {
    let mut data = ta_data::GameData::load(&ta_vfs::Vfs::new());
    data.units.push(ta_data::unit::UnitDef {
        name: "SCOUT".into(), max_damage: 100.0, can_move: true, max_velocity: 2.0,
        acceleration: 0.5, brake_rate: 0.5, turn_rate: 2048.0,
        footprint_x: 1, footprint_z: 1, max_slope: 90.0,
        ..Default::default()
    });
    let types = std::sync::Arc::new(ta_sim::UnitTypes::compile(&data, &|_| None));
    let data = std::sync::Arc::new(data);
    let map = ta_formats::tnt::TntMap {
        width: 32, height: 32, sea_level: 0, heights: vec![60; 32 * 32],
        features: vec![ta_formats::tnt::FEATURE_NONE; 32 * 32], tiles: vec![],
        tile_map: vec![], feature_names: vec![], minimap: Default::default(),
        flags: Vec::new(),
    };
    let players: Vec<_> = (0..2).map(|p| PlayerSetup {
        side: "ARM".into(), color: p, start: FxVec3::from_ints(80 + p as i64 * 160, 60, 80),
        metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: p == 1,
    }).collect();
    let mut sim = Sim::new(types, data, &map, &ta_data::MapInfo::default(), 0, &players, 7);
    let units = std::array::from_fn(|p| sim.spawn_unit(0, p as u8, players[p].start, 0, true).unwrap());
    (sim, units)
}

fn playback(ticks: Vec<(u64, Vec<Command>)>, full_commands: bool) -> Playback {
    Playback { ticks: ticks.into(), end_tick: 90, full_commands, resume: false, source: None, seek_to: None, duties: Vec::new() }
}

fn move_order(player: u8, unit: UnitId, x: i64, z: i64) -> Command {
    Command::Order { player, units: vec![unit], order: Order::Move(FxVec3::from_ints(x, 60, z)), queue: false }
}

#[test]
fn complete_replay_preserves_the_battle_when_ai_decisions_change() {
    let (mut live, units) = fixture();
    let mut legacy = live.clone();
    let mut replay = live.clone();
    let human = move_order(0, units[0], 80, 400);
    let ai = move_order(1, units[1], 400, 400);
    let mut old = playback(vec![(1, vec![human.clone()])], false);
    let mut current = playback(vec![(1, vec![human.clone()]), (2, vec![ai.clone()])], true);
    let mut ai_ticks = 0;
    for tick in 1..=90 {
        let human_orders = if tick == 1 { vec![human.clone()] } else { Vec::new() };
        let ai_orders = if tick == 2 { vec![ai.clone()] } else { Vec::new() };
        let commands = commands_for_tick(None, tick, human_orders, ai_orders.clone());
        live.step(&commands);

        let commands = commands_for_tick(Some(&mut old), tick, vec![Command::Resign { player: 0 }], ai_orders);
        legacy.step(&commands);
        assert_eq!(legacy.checksum(), live.checksum(), "legacy human-only replay diverged at tick {tick}");

        // A changed AI now wants to resign every tick. None of those replacement
        // decisions may leak into the full historical stream, even on silent ticks.
        let changed_ai = std::iter::once_with(|| {
            ai_ticks += 1;
            Command::Resign { player: 1 }
        });
        let commands = commands_for_tick(Some(&mut current), tick, vec![Command::Resign { player: 0 }], changed_ai);
        replay.step(&commands);
        assert_eq!(replay.checksum(), live.checksum(), "complete replay diverged at tick {tick}");
        live.take_events();
        legacy.take_events();
        replay.take_events();
    }
    assert_eq!(ai_ticks, 90, "save resumes must still advance each AI brain");
    assert!(old.ticks.is_empty() && current.ticks.is_empty());
    assert!(current.finished(replay.tick));
    assert_ne!(live.unit(units[1]).unwrap().pos, FxVec3::from_ints(240, 60, 80), "the fixture must exercise AI movement");
}

#[test]
fn legacy_save_catch_up_records_complete_history_before_resuming() {
    let (_, units) = fixture();
    let human = move_order(0, units[0], 80, 400);
    let ai = move_order(1, units[1], 400, 400);
    let mut old = playback(vec![(1, vec![human.clone()])], false);
    old.resume = true;
    let mut recording = crate::replay::Recording::default();
    let mut brain_tick = 0;
    for tick in 1..=3 {
        let generated = std::iter::once_with(|| {
            brain_tick += 1;
            ai.clone()
        });
        let commands = commands_for_tick(Some(&mut old), tick, Vec::new(), generated);
        assert_eq!(commands.len(), if tick == 1 { 2 } else { 1 });
        assert!(!recording.append(tick, &commands));
    }
    let history = recording.snapshot().unwrap();
    assert_eq!(history.len(), 3, "AI-only ticks must survive a legacy save conversion");
    assert_eq!(history[0].1.len(), 2);
    let mut converted = playback(history, true);
    for tick in 1..=3 {
        let commands = commands_for_tick(Some(&mut converted), tick, Vec::new(), [Command::Resign { player: 1 }]);
        assert_eq!(commands.len(), if tick == 1 { 2 } else { 1 }, "AI orders must not be duplicated when reading a converted save");
        assert!(!commands.iter().any(|c| matches!(c, Command::Resign { .. })));
    }
    let live = commands_for_tick(None, 4, vec![human], std::iter::once_with(|| {
        brain_tick += 1;
        ai
    }));
    assert_eq!(brain_tick, 4);
    assert_eq!(live.len(), 2, "the next live tick uses new player and AI orders again");
}
