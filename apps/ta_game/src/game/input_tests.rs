//! Optional native controller regression: local original data and a graphics adapter.
//! Run with `cargo test --release -p ta_game native_queued_orders -- --ignored --nocapture`.

use super::*;
use std::sync::Arc;
use winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
use winit::keyboard::ModifiersState;
use winit::platform::windows::EventLoopBuilderExtWindows;

fn modifiers(input: &mut Input, state: ModifiersState) {
    input.on_event(&WindowEvent::ModifiersChanged(state.into()), false, false);
}

fn button(input: &mut Input, state: ElementState) {
    input.on_event(&WindowEvent::MouseInput { device_id: DeviceId::dummy(), state, button: MouseButton::Left }, false, false);
}

fn patrol_command(game: &Game, unit: UnitId, point: Vec3, queue: bool) {
    assert!(matches!(game.commands.last(), Some(Command::Order { units, order: Order::Patrol(p), queue: queued, .. })
        if units == &[unit] && *p == fx_vec(point) && *queued == queue), "expected Patrol(queue={queue}), got {:?}", game.commands.last());
}

#[test]
#[ignore = "needs installed Total Annihilation data and a Windows graphics adapter"]
#[allow(deprecated)]
fn native_multiplayer_drains_paused_ticks_and_resumed_history() {
    use std::time::{Duration, Instant};
    use ta_net::{Client, Host, Msg, Session};

    let install = ta_vfs::install::discover(None).expect("Total Annihilation install");
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let event_loop = winit::event_loop::EventLoop::builder().with_any_thread(true).build().unwrap();
    let window = Arc::new(event_loop.create_window(winit::window::Window::default_attributes()
        .with_visible(false).with_inner_size(winit::dpi::PhysicalSize::new(1024, 768))).unwrap());
    let mut renderer = Renderer::new(window, 1024, 768).unwrap();
    let assets = Assets::load(Arc::new(vfs), Default::default(), &mut renderer, None, None, &Default::default()).unwrap();
    let mut host = Host::bind(0, assets.data_hash).unwrap();
    let address = ([127, 0, 0, 1], host.port).into();
    let mut client = Client::connect(address, "Guest", assets.data_hash).unwrap();
    let wait = |done: &mut dyn FnMut() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < deadline, "network event did not arrive");
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    wait(&mut || { host.poll(); client.poll(); client.peer.is_some() });
    let token = client.reconnect_token();
    let slot = |peer, name: &str, color| ta_net::LobbySlot {
        kind: ta_net::SlotKind::Human { peer, name: name.into() }, side: "ARM".into(), color, team: color, ready: true,
    };
    let info = ta_net::StartInfo { map: "Painted Desert".into(), seed: 7,
        slots: vec![slot(0, "Host", 0), slot(client.peer.unwrap(), "Guest", 1)],
        spectators: Vec::new(), rules: Default::default() };
    let setup = |player| {
        let mut setup = GameSetup::duel(&info.map);
        setup.seed = info.seed;
        setup.local_player = Some(player);
        for slot in &mut setup.players { slot.side = "ARM".into(); slot.controller = Controller::Human; }
        setup
    };
    let mut host_game = Game::new(&assets, &mut renderer, &setup(0)).unwrap();
    let mut client_game = Game::new(&assets, &mut renderer, &setup(1)).unwrap();
    host_game.net = Some(Session::host(host, &info));
    client_game.net = Some(Session::client(client, 1).with_roster(&info));
    let commander = host_game.local_commander().unwrap();
    let origin = host_game.sim.unit(commander).unwrap().pos;
    host_game.issue(Command::Order { player: 0, units: vec![commander],
        order: Order::Move(origin + FxVec3::from_ints(128, 0, 128)), queue: false });
    // Pause with a few already-sealed ticks still in flight to the client.
    for _ in 0..15 { host_game.update(TICK_SECONDS); }
    host_game.set_pace(Some(true), None);
    wait(&mut || {
        let net = client_game.net.as_mut().unwrap();
        net.poll();
        net.paused && net.backlog() == 15
    });
    client_game.update(1.0);
    assert_eq!(client_game.sim.tick, host_game.sim.tick, "pause must drain every authoritative tick, including the last three");
    assert_eq!(client_game.sim.checksum(), host_game.sim.checksum());
    assert!(client_game.paused);

    // A rejoin replays the same battle to the paused boundary, without needing RESUME.
    let mut leaving = client_game.net.take().unwrap();
    leaving.leave();
    drop(leaving);
    wait(&mut || { let net = host_game.net.as_mut().unwrap(); net.poll(); !net.away.is_empty() });
    let mut back = Client::connect_with_token(address, "Guest", assets.data_hash, token, None).unwrap();
    let mut resumed = None;
    wait(&mut || {
        host_game.net.as_mut().unwrap().poll();
        for msg in back.poll() {
            if let Msg::Resume { player, upto, history, paused, speed, .. } = msg {
                resumed = Some((player, upto, history, paused, speed));
            }
        }
        resumed.is_some()
    });
    let (player, upto, history, paused, speed) = resumed.unwrap();
    let mut rejoined = Game::new(&assets, &mut renderer, &setup(1)).unwrap();
    rejoined.net = Some(Session::resumed(back, player, upto, history, paused, speed).with_roster(&info));
    rejoined.update(0.0);
    assert_eq!(rejoined.sim.tick, host_game.sim.tick, "paused reconnect must finish catching up");
    assert_eq!(rejoined.sim.checksum(), host_game.sim.checksum());
    assert!(rejoined.paused);
    let tick = rejoined.sim.tick;
    rejoined.update(1.0);
    assert_eq!(rejoined.sim.tick, tick, "catching up must not create ticks while paused");
}

/// The host's leaving ends a multiplayer battle for everyone (every machine takes its ticks
/// from the host's), so the host is asked first while anyone still fights it: beaten, or
/// watching, as well as playing. A player's leaving costs only them, and a battle over for
/// everyone asks nobody.
#[test]
#[ignore = "needs installed Total Annihilation data and a Windows graphics adapter"]
#[allow(deprecated)]
fn native_a_host_is_asked_before_leaving_a_battle_others_fight() {
    use ta_net::{Client, Host, Session};
    let install = ta_vfs::install::discover(None).expect("Total Annihilation install");
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let event_loop = winit::event_loop::EventLoop::builder().with_any_thread(true).build().unwrap();
    let window = Arc::new(event_loop.create_window(winit::window::Window::default_attributes()
        .with_visible(false).with_inner_size(winit::dpi::PhysicalSize::new(1024, 768))).unwrap());
    let mut renderer = Renderer::new(window, 1024, 768).unwrap();
    let assets = Assets::load(Arc::new(vfs), Default::default(), &mut renderer, None, None, &Default::default()).unwrap();
    let mut host = Host::bind(0, assets.data_hash).unwrap();
    let mut client = Client::connect(([127, 0, 0, 1], host.port).into(), "Guest", assets.data_hash).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while client.peer.is_none() {
        host.poll();
        client.poll();
        assert!(std::time::Instant::now() < deadline, "the guest is welcomed");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    // The host, a guest and a computer player, each on a team of their own.
    let slot = |kind, color| ta_net::LobbySlot { kind, side: "ARM".into(), color, team: color, ready: true };
    let info = ta_net::StartInfo { map: "Painted Desert".into(), seed: 7, spectators: Vec::new(), rules: Default::default(),
        slots: vec![slot(ta_net::SlotKind::Human { peer: 0, name: "Host".into() }, 0), slot(ta_net::SlotKind::Human { peer: client.peer.unwrap(), name: "Guest".into() }, 1),
            slot(ta_net::SlotKind::Ai { difficulty: 1 }, 2)] };
    let mut setup = GameSetup::duel(&info.map);
    let mut third = setup.players[1].clone();
    (third.team, third.start, third.color) = (2, 2, 2);
    setup.players[1].controller = Controller::Human;
    setup.players.push(third);
    let mut host_game = Game::new(&assets, &mut renderer, &setup).unwrap();
    setup.local_player = Some(1);
    let mut guest_game = Game::new(&assets, &mut renderer, &setup).unwrap();
    host_game.net = Some(Session::host(host, &info));
    guest_game.net = Some(Session::client(client, 1).with_roster(&info));
    assert!(host_game.hosting_others() && host_game.leaving_asks(), "the host, playing");
    assert!(!guest_game.hosting_others() && guest_game.leaving_asks(), "a player in a live battle is asked as ever");
    // Beaten, its result on the screen, or watching on: the others still fight.
    host_game.sim.players[0].defeated = true;
    host_game.outcome = Some(Outcome::Defeat);
    assert!(host_game.hosting_others() && host_game.leaving_asks(), "the beaten host");
    host_game.outcome = None;
    host_game.spectator = true;
    assert!(host_game.leaving_asks(), "the host watching on");
    guest_game.sim.players[1].defeated = true;
    guest_game.outcome = Some(Outcome::Defeat);
    assert!(!guest_game.leaving_asks(), "a beaten player's leaving costs nobody anything");
    // One side standing: over for everyone.
    host_game.sim.players[1].defeated = true;
    assert!(!host_game.hosting_others() && !host_game.leaving_asks(), "the battle is over");
}

#[test]
#[ignore = "needs installed Total Annihilation data and a Windows graphics adapter"]
#[allow(deprecated)]
fn native_watching_a_skirmish_team_requires_a_survivor_for_victory() {
    let install = ta_vfs::install::discover(None).expect("Total Annihilation install");
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let event_loop = winit::event_loop::EventLoop::builder().with_any_thread(true).build().unwrap();
    let window = Arc::new(event_loop.create_window(winit::window::Window::default_attributes()
        .with_visible(false).with_inner_size(winit::dpi::PhysicalSize::new(1024, 768))).unwrap());
    let mut renderer = Renderer::new(window, 1024, 768).unwrap();
    let assets = Assets::load(Arc::new(vfs), Default::default(), &mut renderer, None, None, &Default::default()).unwrap();
    let mut setup = GameSetup::duel("Painted Desert");
    setup.players[1].team = 0;
    let mut enemy = setup.players[1].clone();
    enemy.team = 1;
    enemy.start = 2;
    setup.players.push(enemy);
    let mut game = Game::new(&assets, &mut renderer, &setup).unwrap();
    // A minute in, beaten and watching on (KEEP WATCHING: a spectator now): the battle was
    // ours, and its replay is kept as we leave.
    for _ in 0..30 * 60 {
        game.sim.step(&[]);
    }
    assert!(game.keeps_replay());
    game.spectator = true;
    assert!(game.keeps_replay(), "a player watching on after a defeat keeps the battle's replay");
    for (ally_lost, enemy_lost, expected) in [
        (false, false, None), (false, true, Some(Outcome::Victory)),
        (true, false, Some(Outcome::Defeat)), (true, true, Some(Outcome::Defeat)),
    ] {
        game.sim.players[0].defeated = true;
        game.sim.players[1].defeated = ally_lost;
        game.sim.players[2].defeated = enemy_lost;
        game.spectator = true;
        game.watching_team = true;
        game.team_won = false;
        game.outcome = None;
        game.after_update();
        assert_eq!(game.outcome, expected, "ally lost={ally_lost}, enemy lost={enemy_lost}");
        assert_eq!(game.team_won, expected == Some(Outcome::Victory));
    }
}

#[test]
#[ignore = "needs installed Total Annihilation data and a Windows graphics adapter"]
#[allow(deprecated)]
fn native_entire_unit_roster_has_its_contextual_commands() {
    use crate::radial::{Id, Menu, Radial};
    use crate::orders::Kind;
    let install = ta_vfs::install::discover(None).expect("Total Annihilation install");
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let event_loop = winit::event_loop::EventLoop::builder().with_any_thread(true).build().unwrap();
    let window = Arc::new(event_loop.create_window(winit::window::Window::default_attributes()
        .with_visible(false).with_inner_size(winit::dpi::PhysicalSize::new(1024, 768))).unwrap());
    let mut renderer = Renderer::new(window, 1024, 768).unwrap();
    let assets = Assets::load(Arc::new(vfs), Default::default(), &mut renderer, None, None, &Default::default()).unwrap();
    let mut game = Game::new(&assets, &mut renderer, &GameSetup::duel("Painted Desert")).unwrap();
    let commander = game.local_commander().unwrap();
    let origin = game.sim.unit(commander).unwrap().pos;
    let mut checked = 0;
    let mut silos = 0;
    for type_id in 0..game.sim.types.types.len() {
        let id = game.sim.spawn_unit(type_id, game.local_player, origin, 0, true).unwrap();
        game.selection = vec![id];
        let mut ring = Radial::open(id, glam::Vec2::ZERO, false);
        let mut shown = Vec::new();
        loop {
            let entries = crate::radial::entries(&game, &ring);
            shown.extend(entries.iter().map(|e| e.id));
            match entries.last().map(|e| e.id) {
                Some(Id::Open(Menu::More(page))) => ring.turn_to(Menu::More(page)),
                _ => break,
            }
        }
        let ty = game.sim.types.get(type_id);
        for order in crate::orders::available(&game).iter().filter(|d| matches!(d.kind, Kind::Mode(_))) {
            assert!(shown.contains(&Id::Order(order.kind)), "{} omitted {:?}", ty.name, order.kind);
        }
        if ty.stockpile && ty.weapons.iter().flatten().any(|&w| !game.sim.weapon_types.get(w).def.interceptor) {
            silos += 1;
            assert!(shown.contains(&Id::Order(Kind::Mode(CommandMode::Attack))), "{} needs Attack", ty.name);
            assert!(!shown.contains(&Id::Order(Kind::Mode(CommandMode::DGun))), "{} is not a D-gun", ty.name);
            let order = Order::Attack(ta_sim::Target::Ground(origin));
            assert_eq!(game.units_for(&order, None), [id], "{} dispatches Attack", ty.name);
        }
        checked += 1;
        game.sim.remove_unit(id);
    }
    assert!(checked > 100 && silos > 0, "checked {checked} unit types, including {silos} silos");

    // A building's role must not hide the other selected units' commands.
    let solar = game.sim.types.id("ARMSOLAR").unwrap();
    let solar = game.sim.spawn_unit(solar, game.local_player, origin, 0, true).unwrap();
    game.selection = vec![solar, commander];
    let mut ring = Radial::open(solar, glam::Vec2::ZERO, false);
    let mut shown = Vec::new();
    loop {
        let entries = crate::radial::entries(&game, &ring);
        shown.extend(entries.iter().map(|e| e.id));
        match entries.last().map(|e| e.id) {
            Some(Id::Open(Menu::More(page))) => ring.turn_to(Menu::More(page)),
            _ => break,
        }
    }
    for mode in [CommandMode::Move, CommandMode::Repair, CommandMode::Reclaim, CommandMode::DGun] {
        assert!(shown.contains(&Id::Order(Kind::Mode(mode))), "mixed selection omitted {mode:?}");
    }
    assert_eq!(CommandMode::DGun.label_for(&game), "D-Gun");
    println!("checked {checked} unit types, {silos} nuclear launchers and a mixed selection");
}

#[test]
#[ignore = "needs installed Total Annihilation data and a Windows graphics adapter"]
#[allow(deprecated)]
fn native_queued_orders_keep_patrol_targets_and_finish_repeat_modes() {
    let install = ta_vfs::install::discover(None).expect("Total Annihilation install");
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let event_loop = winit::event_loop::EventLoop::builder().with_any_thread(true).build().unwrap();
    let window = Arc::new(event_loop.create_window(winit::window::Window::default_attributes()
        .with_visible(false).with_inner_size(winit::dpi::PhysicalSize::new(1024, 768))).unwrap());
    let mut renderer = Renderer::new(window, 1024, 768).unwrap();
    let assets = Assets::load(Arc::new(vfs), Default::default(), &mut renderer, None, None, &Default::default()).unwrap();
    let mut game = Game::new(&assets, &mut renderer, &GameSetup::duel("Coast To Coast")).unwrap();
    let unit = game.local_commander().unwrap();
    let origin = Vec3::from_array(game.sim.unit(unit).unwrap().pos.to_f32());
    let point = origin + Vec3::new(120.0, 0.0, 80.0);
    let mut camera = Camera::new(Vec2::new(1024.0, 768.0));
    camera.focus = origin;
    let mut input = Input::default();
    input.mouse = Vec2::new(800.0, 500.0);
    game.selection = vec![unit];
    game.cursor_override = Some(point);

    // A completed Ctrl+digit assigns the group even after Ctrl is released before
    // redraw. The next plain digit recalls it instead of inheriting that modifier.
    modifiers(&mut input, ModifiersState::CONTROL);
    input.press(winit::keyboard::KeyCode::Digit1);
    modifiers(&mut input, ModifiersState::empty());
    game.handle_input(&input, &camera, &renderer, false);
    assert_eq!(game.groups[1], [unit]);
    input.end_frame();
    game.selection.clear();
    input.press(winit::keyboard::KeyCode::Digit1);
    game.handle_input(&input, &camera, &renderer, false);
    assert_eq!(game.selection, [unit]);
    input.end_frame();
    game.set_command_mode(Some(CommandMode::Patrol));

    // Keep a queued move already waiting: a Shift click must not replace it even when
    // Shift is released in the event queue before the next redraw.
    game.issue(Command::Order { player: game.local_player, units: vec![unit], order: Order::Move(fx_vec(origin)), queue: false });
    modifiers(&mut input, ModifiersState::SHIFT);
    button(&mut input, ElementState::Pressed);
    game.handle_input(&input, &camera, &renderer, false);
    input.end_frame();
    input.mouse.x += 12.0; // more than the old selection-box threshold
    game.handle_input(&input, &camera, &renderer, false);
    assert!(game.drag_rect.is_none(), "dragging an armed Patrol must not select a box");
    input.end_frame();
    button(&mut input, ElementState::Released);
    modifiers(&mut input, ModifiersState::empty());
    game.handle_input(&input, &camera, &renderer, false);
    patrol_command(&game, unit, point, true);
    assert_eq!(game.commands.len(), 2, "the earlier move is preserved");
    assert_eq!(game.selection, [unit]);
    assert!(game.command_mode.is_none() && !game.repeat_order, "releasing Shift finishes repeat mode after dispatch");

    // The minimap uses the same target dispatch, including queue flags. UI-owned release
    // metadata keeps the mode until the HUD has processed the last queued click.
    input.end_frame();
    game.set_command_mode(Some(CommandMode::Patrol));
    game.order_selection_from_map(point, 0.0, true, true);
    patrol_command(&game, unit, point, true);
    assert_eq!(game.command_mode, Some(CommandMode::Patrol));
    modifiers(&mut input, ModifiersState::SHIFT);
    input.on_event(&WindowEvent::MouseInput { device_id: DeviceId::dummy(), state: ElementState::Pressed, button: MouseButton::Left }, true, false);
    input.on_event(&WindowEvent::MouseInput { device_id: DeviceId::dummy(), state: ElementState::Released, button: MouseButton::Left }, true, false);
    modifiers(&mut input, ModifiersState::empty());
    game.handle_input(&input, &camera, &renderer, true);
    assert_eq!(game.command_mode, Some(CommandMode::Patrol));
    let second = point + Vec3::new(80.0, 0.0, 0.0);
    game.order_selection_from_map(second, 0.0, true, false);
    patrol_command(&game, unit, second, true);
    assert!(game.command_mode.is_none() && !game.repeat_order);

    // A friendly unit under a ground target does not remove it from the selected group.
    game.hovered = Some(unit);
    game.cursor_ground = Some(origin);
    assert!(game.target_command(CommandMode::Patrol, true));
    patrol_command(&game, unit, origin, true);

    // Unit-target commands cannot become accidental minimap move orders.
    let count = game.commands.len();
    game.set_command_mode(Some(CommandMode::Guard));
    game.order_selection_from_map(point, 0.0, false, false);
    assert_eq!(game.commands.len(), count);
    assert_eq!(game.command_mode, Some(CommandMode::Guard));
    // Spectators may inspect a selected unit but armed commands must not take ownership
    // of the minimap; it remains available for camera navigation.
    game.spectator = true;
    game.set_command_mode(Some(CommandMode::Patrol));
    assert!(!game.can_order());
    game.order_selection_from_map(point, 0.0, false, false);
    assert_eq!(game.commands.len(), count);
    println!("native queued input: delayed Shift release, Patrol drag, minimap waypoints, repeat completion and friendly-ground targeting passed");
}

/// The red "Cannot comply" notice and the unit's voice are for the player's own orders: one of the
/// duty officer's that the simulation refuses (a site that has been taken, a wreck out of reach) is a job
/// it will find another for, and no news.
/// Run with `cargo test --release -p ta_game no_news_to_the_player -- --ignored --nocapture`.
#[test]
#[ignore = "needs installed Total Annihilation data and a Windows graphics adapter"]
#[allow(deprecated)]
fn an_order_of_the_officers_that_is_refused_is_no_news_to_the_player() {
    let install = ta_vfs::install::discover(None).expect("Total Annihilation install");
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let event_loop = winit::event_loop::EventLoop::builder().with_any_thread(true).build().unwrap();
    let window = Arc::new(event_loop.create_window(winit::window::Window::default_attributes()
        .with_visible(false).with_inner_size(winit::dpi::PhysicalSize::new(1024, 768))).unwrap());
    let mut renderer = Renderer::new(window, 1024, 768).unwrap();
    let assets = Assets::load(Arc::new(vfs), Default::default(), &mut renderer, None, None, &Default::default()).unwrap();
    let mut game = Game::new(&assets, &mut renderer, &GameSetup::duel("Coast To Coast")).unwrap();
    let commander = game.local_commander().unwrap();
    let solar = game.sim.types.id("ARMSOLAR").expect("a solar collector");
    let site = game.sim.unit(commander).unwrap().pos + ta_sim::FxVec3::from_ints(70, 0, 0);
    game.sim.spawn_unit(solar, game.local_player, site, 0, true).expect("a building on the site");
    let refused = Command::Order { player: game.local_player, units: vec![commander], order: Order::Build { type_id: solar, pos: site }, queue: false };
    let told = |game: &Game| game.messages.iter().any(|m| m.text.starts_with("Cannot comply"));

    // The player's own order for the Commander: refused, and the player is told.
    game.run_tick(vec![refused.clone()]);
    for _ in 0..3 {
        game.run_tick(Vec::new());
    }
    assert!(told(&game), "the player's own refused order is told: {:?}", game.messages.iter().map(|m| &m.text).collect::<Vec<_>>());
    assert!(game.events_out.iter().any(|e| matches!(e, SimEvent::OrderFailed { .. })), "and the unit's voice is sent on");
    game.messages.clear();
    game.events_out.clear();

    // The same refusal of an order of the officer's (the Commander on a duty, the order not through `issue`).
    game.selection = vec![commander];
    game.set_duty_of(&[commander], Some(ta_ai::Duty::Economy));
    assert!(game.on_duty(commander));
    game.run_tick(vec![refused]);
    for _ in 0..3 {
        game.run_tick(Vec::new());
    }
    assert!(!matches!(game.sim.unit(commander).unwrap().orders.front(), Some(Order::Build { pos, .. }) if *pos == site), "the order was refused");
    assert!(!told(&game), "the officer's refused order is no news: {:?}", game.messages.iter().map(|m| &m.text).collect::<Vec<_>>());
    assert!(!game.events_out.iter().any(|e| matches!(e, SimEvent::OrderFailed { .. })), "and the unit does not say it cannot");
}

/// A building put down again on the selection's own plan undoes it; another put down over it
/// takes its place (with Shift, the rest of the queue kept); and Shift with the right button
/// while placing ends the placing and queues the order there, as it does without one.
/// Run with `cargo test --release -p ta_game native_placing -- --ignored --nocapture`.
#[test]
#[ignore = "needs installed Total Annihilation data and a Windows graphics adapter"]
#[allow(deprecated)]
fn native_placing_again_undoes_a_plan_and_another_building_takes_its_place() {
    let install = ta_vfs::install::discover(None).expect("Total Annihilation install");
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let event_loop = winit::event_loop::EventLoop::builder().with_any_thread(true).build().unwrap();
    let window = Arc::new(event_loop.create_window(winit::window::Window::default_attributes()
        .with_visible(false).with_inner_size(winit::dpi::PhysicalSize::new(1024, 768))).unwrap());
    let mut renderer = Renderer::new(window, 1024, 768).unwrap();
    let assets = Assets::load(Arc::new(vfs), Default::default(), &mut renderer, None, None, &Default::default()).unwrap();
    let mut game = Game::new(&assets, &mut renderer, &GameSetup::duel("Coast To Coast")).unwrap();
    game.order_button = ta_config::OrderButton::Right;
    let commander = game.local_commander().unwrap();
    let (solar, wind) = (game.sim.types.id("ARMSOLAR").expect("a solar collector"), game.sim.types.id("ARMWIN").expect("a wind generator"));
    let origin = game.sim.unit(commander).unwrap().pos;
    // A site out of the Commander's reach (it walks there first: the plan is not started), where
    // both can stand.
    let site = (10..30)
        .flat_map(|r: i64| [(r * 32, 0), (0, r * 32), (-r * 32, 0), (0, -r * 32)])
        .map(|(dx, dz)| game.sim.snap_to_grid(solar, origin + FxVec3::from_ints(dx, 0, dz)))
        .find(|&p| game.sim.can_place(solar, p) && game.sim.can_place(wind, game.sim.snap_to_grid(wind, p)))
        .expect("a site for both");
    let mut camera = Camera::new(Vec2::new(1024.0, 768.0));
    camera.focus = Vec3::from_array(origin.to_f32());
    let mut input = Input::default();
    input.mouse = Vec2::new(500.0, 400.0);
    game.selection = vec![commander];
    game.cursor_override = Some(Vec3::from_array(site.to_f32()));
    // One placing of `kind` at the site, clicked with the left button, Shift held or not; the
    // orders then reach the simulation.
    let place = |game: &mut Game, input: &mut Input, kind: usize, shift: bool| {
        game.begin_placement(kind);
        modifiers(input, if shift { ModifiersState::SHIFT } else { ModifiersState::empty() });
        game.handle_input(input, &camera, &renderer, false);
        input.end_frame();
        button(input, ElementState::Pressed);
        game.handle_input(input, &camera, &renderer, false);
        input.end_frame();
        button(input, ElementState::Released);
        game.handle_input(input, &camera, &renderer, false);
        input.end_frame();
        let shifted = game.placement.is_some();
        modifiers(input, ModifiersState::empty());
        game.handle_input(input, &camera, &renderer, false);
        input.end_frame();
        let commands = std::mem::take(&mut game.commands);
        game.run_tick(commands);
        shifted
    };
    let plans = |game: &Game| -> Vec<usize> {
        game.sim.unit(commander).unwrap().orders.iter().filter_map(|o| if let Order::Build { type_id, .. } = o { Some(*type_id) } else { None }).collect()
    };

    // With Shift: put down, then put down again on it: undone.
    assert!(place(&mut game, &mut input, solar, true), "Shift keeps the placing");
    assert_eq!(plans(&game), [solar]);
    place(&mut game, &mut input, solar, true);
    assert_eq!(plans(&game), [] as [usize; 0], "the same building on its own plan undoes it");
    // Another building over it takes its place.
    place(&mut game, &mut input, solar, true);
    place(&mut game, &mut input, wind, true);
    assert_eq!(plans(&game), [wind], "a wind generator in the solar collector's place");
    // Without Shift as well: undone, and the placing over.
    assert!(!place(&mut game, &mut input, wind, false));
    assert_eq!(plans(&game), [] as [usize; 0]);
    place(&mut game, &mut input, solar, false);
    place(&mut game, &mut input, wind, false);
    assert_eq!(plans(&game), [wind]);
    assert!(game.sim.unit(commander).unwrap().build.is_none(), "(not started: still on its way)");

    // Shift and the right button while placing: the placing ends, and the Commander is sent
    // there after its plans, as a Shift right-click does without a placing.
    let there = Vec3::from_array(origin.to_f32()) + Vec3::new(-96.0, 0.0, 64.0);
    game.cursor_override = Some(there);
    game.begin_placement(solar);
    modifiers(&mut input, ModifiersState::SHIFT);
    game.handle_input(&input, &camera, &renderer, false);
    input.end_frame();
    let right = |input: &mut Input, state| input.on_event(&WindowEvent::MouseInput { device_id: DeviceId::dummy(), state, button: MouseButton::Right }, false, false);
    right(&mut input, ElementState::Pressed);
    game.handle_input(&input, &camera, &renderer, false);
    input.end_frame();
    right(&mut input, ElementState::Released);
    game.handle_input(&input, &camera, &renderer, false);
    input.end_frame();
    assert!(game.placement.is_none(), "the right button ends the placing");
    assert!(
        matches!(game.commands.last(), Some(Command::Order { units, order: Order::Move(_), queue: true, .. }) if units == &[commander]),
        "and Shift queues the order there: {:?}",
        game.commands.last()
    );
    let commands = std::mem::take(&mut game.commands);
    game.run_tick(commands);
    let orders = &game.sim.unit(commander).unwrap().orders;
    assert!(matches!(orders.front(), Some(Order::Build { type_id, .. }) if *type_id == wind) && matches!(orders.back(), Some(Order::Move(_))), "{orders:?}");
    // Without Shift it only ends the placing.
    modifiers(&mut input, ModifiersState::empty());
    game.begin_placement(solar);
    game.handle_input(&input, &camera, &renderer, false);
    input.end_frame();
    let count = game.commands.len();
    right(&mut input, ElementState::Pressed);
    game.handle_input(&input, &camera, &renderer, false);
    input.end_frame();
    right(&mut input, ElementState::Released);
    game.handle_input(&input, &camera, &renderer, false);
    assert!(game.placement.is_none());
    assert_eq!(game.commands.len(), count, "no order");
    println!("native placing: undo, replace, and Shift with the right button passed");
}

#[test]
#[ignore = "needs installed Total Annihilation data and a Windows graphics adapter"]
#[allow(deprecated)]
fn native_minimap_orders_aim_at_the_units_marked_there() {
    let install = ta_vfs::install::discover(None).expect("Total Annihilation install");
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let event_loop = winit::event_loop::EventLoop::builder().with_any_thread(true).build().unwrap();
    let window = Arc::new(event_loop.create_window(winit::window::Window::default_attributes()
        .with_visible(false).with_inner_size(winit::dpi::PhysicalSize::new(1024, 768))).unwrap());
    let mut renderer = Renderer::new(window, 1024, 768).unwrap();
    let assets = Assets::load(Arc::new(vfs), Default::default(), &mut renderer, None, None, &Default::default()).unwrap();
    let mut game = Game::new(&assets, &mut renderer, &GameSetup::duel("Coast To Coast")).unwrap();
    let (me, commander) = (game.local_player, game.local_commander().unwrap());
    let foe = (0..game.sim.players.len() as u8).find(|&p| game.sim.is_enemy(me, p)).expect("an enemy");
    let origin = game.sim.unit(commander).unwrap().pos;
    let (raider, builder) = (game.sim.types.id("CORAK").expect("an A.K."), game.sim.types.id("ARMCK").expect("a construction kbot"));
    let site = |game: &Game, kind, dx, dz| game.sim.find_build_site(kind, origin + FxVec3::from_ints(dx, 0, dz), 10).unwrap();
    let enemy_at = site(&game, raider, 160, 0);
    let enemy = game.sim.spawn_unit(raider, foe, enemy_at, 0, true).unwrap();
    let friend_at = site(&game, builder, -120, 60);
    let friend = game.sim.spawn_unit(builder, me, friend_at, 0, true).unwrap();
    game.run_tick(Vec::new());
    assert_eq!(game.seen(game.sim.unit(enemy).unwrap()), Seen::Visible, "the enemy in the Commander's sight");
    game.selection = vec![commander];
    let ground = |p: FxVec3| Vec3::from_array(p.to_f32());
    let last = |game: &Game| game.commands.last().cloned();

    // A click on the enemy's mark: it is attacked. Beside it, on nothing: a move there.
    game.order_selection_from_map(ground(enemy_at) + Vec3::new(8.0, 0.0, 0.0), 40.0, false, false);
    assert!(matches!(last(&game), Some(Command::Order { order: Order::Attack(ta_sim::Target::Unit(t)), .. }) if t == enemy), "{:?}", last(&game));
    game.order_selection_from_map(ground(enemy_at) + Vec3::new(0.0, 0.0, 200.0), 40.0, false, false);
    assert!(matches!(last(&game), Some(Command::Order { order: Order::Move(_), .. })), "{:?}", last(&game));
    // Our own unit's mark: still a move, not a guard.
    game.order_selection_from_map(ground(friend_at), 40.0, false, false);
    assert!(matches!(last(&game), Some(Command::Order { order: Order::Move(_), .. })), "{:?}", last(&game));
    // Armed, a command takes its own kind of unit: Guard ours, Attack the enemy.
    game.set_command_mode(Some(CommandMode::Guard));
    game.order_selection_from_map(ground(friend_at), 40.0, false, false);
    assert!(matches!(last(&game), Some(Command::Order { order: Order::Guard(t), .. }) if t == friend), "{:?}", last(&game));
    game.set_command_mode(Some(CommandMode::Attack));
    game.order_selection_from_map(ground(enemy_at), 40.0, false, false);
    assert!(matches!(last(&game), Some(Command::Order { order: Order::Attack(ta_sim::Target::Unit(t)), .. }) if t == enemy), "{:?}", last(&game));
    println!("native minimap orders: attack, move, guard and armed attack passed");
}

#[test]
#[ignore = "needs installed Total Annihilation data and a Windows graphics adapter"]
#[allow(deprecated)]
fn native_shift_giving_a_queued_reclaim_or_move_again_takes_it_back() {
    let install = ta_vfs::install::discover(None).expect("Total Annihilation install");
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let event_loop = winit::event_loop::EventLoop::builder().with_any_thread(true).build().unwrap();
    let window = Arc::new(event_loop.create_window(winit::window::Window::default_attributes()
        .with_visible(false).with_inner_size(winit::dpi::PhysicalSize::new(1024, 768))).unwrap());
    let mut renderer = Renderer::new(window, 1024, 768).unwrap();
    let assets = Assets::load(Arc::new(vfs), Default::default(), &mut renderer, None, None, &Default::default()).unwrap();
    let mut game = Game::new(&assets, &mut renderer, &GameSetup::duel("Coast To Coast")).unwrap();
    let commander = game.local_commander().unwrap();
    let origin = Vec3::from_array(game.sim.unit(commander).unwrap().pos.to_f32());
    game.selection = vec![commander];
    let orders = |game: &Game| game.sim.unit(commander).unwrap().orders.iter().cloned().collect::<Vec<_>>();
    // A Shift right-click there, and the orders on to the simulation.
    let click = |game: &mut Game, at: Vec3| {
        game.hovered = None;
        game.cursor_ground = Some(at);
        game.context_order(true);
        let commands = std::mem::take(&mut game.commands);
        game.run_tick(commands);
    };

    // A wreck to reclaim, given again with Shift: taken back.
    let wreck = [(200.0, 0.0), (0.0, 200.0), (-200.0, 0.0), (0.0, -200.0)]
        .into_iter()
        .find_map(|(dx, dz)| game.sim.place_feature("ARMSOLAR_DEAD", fx_vec(origin + Vec3::new(dx, 0.0, dz)), 0))
        .expect("a solar collector's wreck put down");
    let at = Vec3::from_array(game.sim.features[wreck].pos.to_f32());
    click(&mut game, at);
    assert!(orders(&game).contains(&Order::Reclaim(wreck)), "{:?}", orders(&game));
    click(&mut game, at);
    assert!(!orders(&game).contains(&Order::Reclaim(wreck)), "taken back: {:?}", orders(&game));

    // Two waypoints set with Shift, on open ground; a Shift-click beside the first takes it back.
    let open = |game: &Game, p: Vec3| game.reclaimable_feature_at(p).is_none();
    let spots: Vec<Vec3> = (3..20)
        .flat_map(|r| [Vec3::new(r as f32 * 40.0, 0.0, 0.0), Vec3::new(-(r as f32) * 40.0, 0.0, 0.0), Vec3::new(0.0, 0.0, r as f32 * 40.0)])
        .map(|d| origin + d)
        .filter(|&p| open(&game, p) && open(&game, p + Vec3::new(20.0, 0.0, 10.0)))
        .collect();
    let (a, b) = (spots[0], *spots.iter().find(|p| p.distance(spots[0]) > 200.0).expect("a second waypoint"));
    click(&mut game, a);
    click(&mut game, b);
    let moves = |game: &Game| orders(game).into_iter().filter(|o| matches!(o, Order::Move(_))).collect::<Vec<_>>();
    assert_eq!(moves(&game).len(), 2, "{:?}", orders(&game));
    click(&mut game, a + Vec3::new(20.0, 0.0, 10.0));
    // (A move order keeps the point on the ground plane: x and z.)
    assert!(matches!(moves(&game).as_slice(), [Order::Move(p)] if p.x == fx_vec(b).x && p.z == fx_vec(b).z), "the first taken back, the second kept: {:?}", orders(&game));

    // Two units sent apart, each on its own: a Shift-click on one's waypoint takes back that one
    // alone. Sent together: a Shift-click on one's place takes back the group's.
    let kbot = game.sim.types.id("ARMCK").expect("a construction kbot");
    let site = game.sim.find_build_site(kbot, fx_vec(origin + Vec3::new(-80.0, 0.0, 80.0)), 10).unwrap();
    let builder = game.sim.spawn_unit(kbot, game.local_player, site, 0, true).unwrap();
    let stop = |game: &mut Game| {
        let units = vec![commander, builder];
        game.issue(Command::Order { player: game.local_player, units, order: Order::Stop, queue: false });
        let commands = std::mem::take(&mut game.commands);
        game.run_tick(commands);
    };
    stop(&mut game);
    let moves_of = |game: &Game, id: UnitId| -> Vec<Vec2> {
        game.sim.unit(id).unwrap().orders.iter().filter_map(|o| if let Order::Move(p) = o { Some(Vec2::new(p.x.to_f32(), p.z.to_f32())) } else { None }).collect()
    };
    let near = |list: &[Vec2], p: Vec3, r: f32| list.iter().any(|q| q.distance(Vec2::new(p.x, p.z)) <= r);
    let p1 = spots[0];
    let p2 = *spots.iter().find(|q| (150.0..300.0).contains(&q.distance(p1))).expect("a place 150 to 300 px off");
    game.selection = vec![commander];
    click(&mut game, p1);
    game.selection = vec![builder];
    click(&mut game, p2);
    game.selection = vec![commander, builder];
    click(&mut game, p1 + Vec3::new(10.0, 0.0, 10.0));
    assert!(!near(&moves_of(&game, commander), p1, 48.0), "the Commander's own waypoint taken back: {:?}", orders(&game));
    assert!(near(&moves_of(&game, builder), p2, 2.0), "the other unit's, given apart, kept: {:?}", moves_of(&game, builder));
    stop(&mut game);
    let p3 = *spots.iter().find(|q| q.distance(origin) > 300.0).expect("a place for the group");
    click(&mut game, p3);
    let place = *moves_of(&game, commander).last().expect("the Commander sent with the group");
    assert!(!moves_of(&game, builder).is_empty(), "the kbot sent with the group");
    click(&mut game, Vec3::new(place.x + 5.0, 0.0, place.y));
    assert!(moves_of(&game, commander).is_empty() && moves_of(&game, builder).is_empty(), "the group's move taken back together: {:?} {:?}", moves_of(&game, commander), moves_of(&game, builder));
    println!("native take-back: reclaim and move passed");
}
