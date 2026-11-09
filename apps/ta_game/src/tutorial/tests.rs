//! Portable lesson checks, plus a native regression through the game's real input dispatch.
use super::*;
use std::sync::Arc;
use ta_sim::{Command, Order, Sim};

struct Battle {
    sim: Sim,
    selection: Vec<UnitId>,
    groups: [Vec<UnitId>; 10],
    used: Used,
    overlay: Option<Overlay>,
}

impl Battle {
    fn new() -> Self {
        use ta_formats::tnt::{FEATURE_NONE, TntMap};
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("ta-tutorial-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        let units = [
            ("ARMCOM", "Commander=1; Builder=1; CanMove=1; MaxVelocity=1; CanReclamate=1;"),
            ("ARMCK", "Builder=1; CanMove=1; MaxVelocity=1; CanReclamate=1;"),
            ("ARMPW", "CanMove=1; MaxVelocity=1; Weapon1=LASER;"),
            ("ARMMEX", "ExtractsMetal=1;"),
            ("ARMSOLAR", "EnergyMake=20;"),
            ("ARMLAB", "Builder=1;"),
            ("ARMLLT", "Weapon1=LASER;"),
            ("ARMRAD", "RadarDistance=1000;"),
        ];
        std::fs::create_dir_all(dir.join("units")).unwrap();
        std::fs::create_dir_all(dir.join("weapons")).unwrap();
        for (name, properties) in units {
            std::fs::write(dir.join(format!("units/{name}.fbi")), format!("[UNITINFO] {{ UnitName={name}; MaxDamage=1000; BuildTime=1000; WorkerTime=100; {properties} }}")).unwrap();
        }
        std::fs::write(dir.join("weapons/test.tdf"), "[LASER] { Range=300; [DAMAGE] { default=10; } }").unwrap();
        std::fs::create_dir_all(dir.join("download")).unwrap();
        std::fs::write(dir.join("download/lab.tdf"), "[MENUENTRY1] { UNITMENU=ARMLAB; UNITNAME=ARMPW; MENU=1; BUTTON=0; } [MENUENTRY2] { UNITMENU=ARMLAB; UNITNAME=ARMCK; MENU=1; BUTTON=1; }").unwrap();
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_mod_dir(&dir, "tutorial-tests").unwrap();
        let data = Arc::new(ta_data::GameData::load(&vfs));
        let types = Arc::new(ta_sim::UnitTypes::compile(&data, &|_| None));
        std::fs::remove_dir_all(&dir).unwrap();
        let map = TntMap { width: 64, height: 64, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![60; 4096], features: vec![FEATURE_NONE; 4096], feature_names: Vec::new(), minimap: Default::default() };
        let players: Vec<_> = (0..2).map(|color| ta_sim::PlayerSetup { side: "ARM".into(), color, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }).collect();
        Self { sim: Sim::new(types, data, &map, &Default::default(), 0, &players, 7), selection: Vec::new(), groups: Default::default(), used: Used::default(), overlay: None }
    }

    fn spawn(&mut self, name: &str, owner: u8, complete: bool) -> UnitId {
        let type_id = self.sim.types.id(name).unwrap();
        let x = 100 + self.sim.units().count() as i64 * 50;
        self.sim.spawn_unit(type_id, owner, FxVec3::from_ints(x, 60, 100), 0, complete).unwrap()
    }

    fn progress(&self) -> Progress<'_> {
        Progress { sim: &self.sim, me: 0, selection: &self.selection, groups: &self.groups, used: &self.used, overlay: self.overlay }
    }

    fn order(&mut self, units: &[UnitId], order: Order, formation: bool) {
        let command = if formation {
            Command::Formation { player: 0, units: units.to_vec(), order, queue: false, shape: ta_sim::formation::Shape::Rows { facing: None } }
        } else {
            Command::Order { player: 0, units: units.to_vec(), order, queue: false }
        };
        self.used.record(&self.sim, 0, &command);
    }
}

fn lesson(n: usize, step: usize) -> Tutorial {
    let mut tutorial = super::tests::lesson(n);
    tutorial.step = step;
    tutorial
}

fn camera() -> Camera {
    Camera::new(glam::Vec2::new(1024.0, 768.0))
}

#[test]
fn attack_move_counts_single_units_and_formations_but_not_empty_or_enemy_orders() {
    for formation in [false, true] {
        let mut battle = Battle::new();
        let squad: Vec<_> = (0..4).map(|_| battle.spawn("ARMPW", 0, true)).collect();
        let enemy = battle.spawn("ARMPW", 1, true);
        let mut tutorial = lesson(1, 2);
        battle.order(&[], Order::Fight(FxVec3::ZERO), formation);
        battle.order(&[enemy], Order::Fight(FxVec3::ZERO), formation);
        assert!(!tutorial.met(battle.progress(), &camera()));
        battle.order(&squad, Order::Move(FxVec3::ZERO), formation);
        assert!(!tutorial.met(battle.progress(), &camera()), "moving does not teach attack-move");
        battle.order(&squad, Order::Fight(FxVec3::ZERO), formation);
        assert!(tutorial.met(battle.progress(), &camera()), "formation={formation}");
    }
}

#[test]
fn selection_and_group_one_require_our_live_combat_squad() {
    let mut battle = Battle::new();
    let enemy_commander = battle.spawn("ARMCOM", 1, true);
    let commander = battle.spawn("ARMCOM", 0, true);
    battle.selection = vec![enemy_commander];
    assert!(!lesson(0, 0).met(battle.progress(), &camera()));
    battle.selection = vec![commander];
    assert!(lesson(0, 0).met(battle.progress(), &camera()));
    let ours: Vec<_> = (0..4).map(|_| battle.spawn("ARMPW", 0, true)).collect();
    battle.selection = (0..4).map(|_| battle.spawn("ARMPW", 1, true)).collect();
    assert!(!lesson(1, 0).met(battle.progress(), &camera()));
    battle.selection = ours.clone();
    assert!(lesson(1, 0).met(battle.progress(), &camera()));
    battle.groups[2] = ours.clone();
    assert!(!lesson(1, 1).met(battle.progress(), &camera()), "the hint teaches Ctrl+1");
    battle.groups[1] = ours.clone();
    assert!(lesson(1, 1).met(battle.progress(), &camera()));
    battle.sim.remove_unit(ours[0]);
    assert!(!lesson(1, 0).met(battle.progress(), &camera()));
    assert!(!lesson(1, 1).met(battle.progress(), &camera()));
}

#[test]
fn looking_around_counts_panning_zooming_and_both_orbit_directions() {
    let battle = Battle::new();
    for gesture in 0..4 {
        let mut tutorial = lesson(0, 1);
        let mut camera = camera();
        assert!(!tutorial.met(battle.progress(), &camera));
        match gesture {
            0 => camera.focus.x += 300.0,
            1 => camera.zoom *= 1.5,
            2 => camera.yaw += 0.3,
            _ => camera.pitch -= 0.3,
        }
        assert!(tutorial.met(battle.progress(), &camera), "gesture {gesture}");
    }
    let mut tutorial = lesson(0, 1);
    let mut camera = camera();
    camera.yaw = std::f32::consts::PI - 0.01;
    assert!(!tutorial.met(battle.progress(), &camera));
    camera.yaw = -std::f32::consts::PI + 0.01;
    assert!(!tutorial.met(battle.progress(), &camera), "angle wrapping is not a full turn");
}

#[test]
fn construction_lessons_distinguish_started_finished_enemy_and_dead_units() {
    for (n, step, name, started_counts) in [
        (0, 2, "ARMMEX", true), (0, 3, "ARMSOLAR", true), (0, 4, "ARMLAB", false),
        (3, 0, "ARMLLT", false), (3, 1, "ARMRAD", false), (4, 0, "ARMCK", false),
    ] {
        let mut battle = Battle::new();
        let mut tutorial = lesson(n, step);
        assert!(!tutorial.met(battle.progress(), &camera()));
        battle.spawn(name, 1, true);
        assert!(!tutorial.met(battle.progress(), &camera()));
        let frame = battle.spawn(name, 0, false);
        assert_eq!(tutorial.met(battle.progress(), &camera()), started_counts);
        battle.sim.remove_unit(frame);
        let ready = battle.spawn(name, 0, true);
        assert!(tutorial.met(battle.progress(), &camera()), "lesson {} step {}", n + 1, step + 1);
        battle.sim.remove_unit(ready);
        assert!(!tutorial.met(battle.progress(), &camera()));
    }
    let mut battle = Battle::new();
    let mut tutorial = lesson(0, 5);
    for _ in 0..2 { battle.spawn("ARMPW", 0, true); }
    battle.spawn("ARMPW", 0, false);
    assert!(!tutorial.met(battle.progress(), &camera()));
    battle.spawn("ARMPW", 0, true);
    assert!(tutorial.met(battle.progress(), &camera()));
}

#[test]
fn builder_and_factory_lessons_credit_the_units_they_teach() {
    let mut battle = Battle::new();
    let commander = battle.spawn("ARMCOM", 0, true);
    let builder = battle.spawn("ARMCK", 0, true);
    let soldier = battle.spawn("ARMPW", 0, true);
    let factory = battle.spawn("ARMLAB", 0, true);
    let enemy_factory = battle.spawn("ARMLAB", 1, true);
    battle.order(&[soldier], Order::Guard(factory), false);
    battle.order(&[commander], Order::Guard(factory), false);
    battle.order(&[builder], Order::Guard(enemy_factory), false);
    assert!(!lesson(4, 1).met(battle.progress(), &camera()));
    battle.order(&[builder], Order::Guard(factory), false);
    assert!(lesson(4, 1).met(battle.progress(), &camera()));
    battle.order(&[commander], Order::Patrol(FxVec3::ZERO), false);
    assert!(!lesson(4, 4).met(battle.progress(), &camera()));
    battle.order(&[builder, soldier], Order::Patrol(FxVec3::ZERO), true);
    assert!(lesson(4, 4).met(battle.progress(), &camera()), "a constructor in a formation also patrols");
    battle.used.record(&battle.sim, 0, &Command::SetStates { player: 0, units: vec![commander], fire: Some(0), movement: None });
    assert!(!lesson(4, 5).met(battle.progress(), &camera()));
    battle.used.record(&battle.sim, 0, &Command::SetStates { player: 0, units: vec![soldier], fire: Some(0), movement: None });
    assert!(lesson(4, 5).met(battle.progress(), &camera()));
    for id in [commander, enemy_factory] {
        battle.used.record(&battle.sim, 0, &Command::SetRally { player: 0, factory: id, pos: FxVec3::ZERO });
        battle.used.record(&battle.sim, 0, &Command::SetRepeat { player: 0, factories: vec![id], on: true });
    }
    assert!(!lesson(4, 2).met(battle.progress(), &camera()));
    assert!(!lesson(4, 3).met(battle.progress(), &camera()));
    battle.used.record(&battle.sim, 0, &Command::SetRally { player: 0, factory, pos: FxVec3::ZERO });
    battle.used.record(&battle.sim, 0, &Command::SetRepeat { player: 0, factories: vec![factory], on: true });
    assert!(lesson(4, 2).met(battle.progress(), &camera()));
    assert!(lesson(4, 3).met(battle.progress(), &camera()));
}

#[test]
fn factory_movement_orders_set_rally_routes_in_both_movement_modes() {
    for formation in [false, true] {
        for order in [Order::Move(FxVec3::ZERO), Order::Fight(FxVec3::ZERO), Order::Patrol(FxVec3::ZERO)] {
            let mut battle = Battle::new();
            let factory = battle.spawn("ARMLAB", 0, true);
            battle.order(&[factory], order, formation);
            assert!(lesson(4, 2).met(battle.progress(), &camera()), "factory order, formation={formation}");
            assert!(!battle.used.fight && !battle.used.patrol, "a factory is not the squad or the patrolling constructor");
        }
    }
}

#[test]
fn queue_edits_only_credit_an_existing_own_entry() {
    let mut battle = Battle::new();
    let builder = battle.spawn("ARMCOM", 0, true);
    let type_id = battle.sim.types.id("ARMSOLAR").unwrap();
    let pos = FxVec3::from_ints(700, 60, 700);
    let edit = Command::EditQueue { player: 0, unit: builder, index: 0, type_id, pos: Some(pos), to_front: true };
    battle.used.record(&battle.sim, 0, &edit);
    assert!(!lesson(2, 4).met(battle.progress(), &camera()), "there is no queued building to edit");
    battle.sim.give_orders(builder, [Order::Build { type_id, pos }]);
    battle.used.record(&battle.sim, 0, &edit);
    assert!(lesson(2, 4).met(battle.progress(), &camera()));
}

#[test]
fn removing_a_factory_queue_entry_also_completes_the_queue_lesson() {
    let mut battle = Battle::new();
    let factory = battle.spawn("ARMLAB", 0, true);
    let factory_type = battle.sim.unit(factory).unwrap().type_id;
    let type_id = battle.sim.types.id("ARMPW").unwrap();
    Arc::get_mut(&mut battle.sim.types).unwrap().types[factory_type].build_options.push(type_id);
    let remove = Command::Produce { player: 0, factory, type_id, count: -1 };
    battle.used.record(&battle.sim, 0, &remove);
    assert!(!lesson(2, 4).met(battle.progress(), &camera()), "an empty queue has nothing to remove");
    let produce = Command::Produce { player: 0, factory, type_id, count: 3 };
    battle.used.record(&battle.sim, 0, &produce);
    battle.sim.step(&[produce]);
    assert!(!lesson(2, 4).met(battle.progress(), &camera()), "adding production is not editing the queue");
    assert!(battle.sim.unit(factory).unwrap().factory_queue.contains(&type_id));
    battle.used.record(&battle.sim, 0, &remove);
    assert!(lesson(2, 4).met(battle.progress(), &camera()));
}

#[test]
fn combat_and_recovery_steps_wait_for_their_actual_targets() {
    let mut battle = Battle::new();
    let tower = battle.spawn("ARMLLT", 1, true);
    let radar = battle.spawn("ARMRAD", 1, true);
    let mut tutorial = lesson(1, 3);
    tutorial.outpost = vec![tower, radar];
    battle.sim.remove_unit(tower);
    assert!(!tutorial.met(battle.progress(), &camera()));
    battle.sim.remove_unit(radar);
    assert!(tutorial.met(battle.progress(), &camera()));
    let mut tutorial = lesson(3, 2);
    assert!(!tutorial.met(battle.progress(), &camera()), "no raid has arrived yet");
    let raider = battle.spawn("ARMPW", 1, true);
    tutorial.raid = vec![raider];
    assert!(!tutorial.met(battle.progress(), &camera()));
    battle.sim.remove_unit(raider);
    assert!(tutorial.met(battle.progress(), &camera()));
    let patient = battle.spawn("ARMLLT", 0, true);
    let mut tutorial = lesson(3, 3);
    tutorial.patient = Some(patient);
    battle.sim.set_health(patient, Fx::from_int(500));
    assert!(!tutorial.met(battle.progress(), &camera()));
    let max = battle.sim.unit_type(battle.sim.unit(patient).unwrap()).max_health;
    battle.sim.set_health(patient, max);
    assert!(tutorial.met(battle.progress(), &camera()));
    let commander = battle.spawn("ARMCOM", 0, true);
    battle.order(&[commander], Order::ReclaimUnit(patient), false);
    assert!(lesson(3, 4).met(battle.progress(), &camera()));
    for (n, spec) in LESSONS.iter().enumerate() {
        assert!(!lesson(n, spec.steps.len() - 1).met(battle.progress(), &camera()), "the final words wait for CONTINUE");
    }
}

#[cfg(windows)]
#[test]
#[ignore = "needs installed Total Annihilation data and a Windows graphics adapter"]
#[allow(deprecated)]
fn native_all_five_lessons_advance_and_attack_move_uses_real_input() {
    use crate::input::Input;
    use crate::setup::Assets;
    use glam::Vec2;
    use ta_render::Renderer;
    use winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
    use winit::keyboard::ModifiersState;
    use winit::platform::windows::EventLoopBuilderExtWindows;

    fn spawn(game: &mut Game, name: &str, at: FxVec3) -> UnitId {
        let t = game.sim.types.id(name).unwrap();
        let site = game.sim.find_build_site(t, at, 32).unwrap_or(at);
        game.sim.spawn_unit(t, game.local_player, site, 0, true).unwrap()
    }
    fn step(game: &mut Game) {
        let commands = std::mem::take(&mut game.commands);
        game.sim.step(&commands);
    }
    fn wait(game: &mut Game, tutorial: &mut Tutorial, camera: &Camera) {
        for _ in 0..18000 {
            if tutorial.met(Progress::from(&*game), camera) { return; }
            step(game);
        }
        let units: Vec<_> = game.sim.units().map(|u| (u.id, &game.sim.unit_type(u).def.name, u.owner, u.dead, u.pos.to_f32(), u.health.to_f32(), game.sim.unit_type(u).max_health.to_f32(), &u.orders, u.build)).collect();
        panic!("lesson {} step {} did not complete in ten simulated minutes; patient={:?}, units={units:?}", tutorial.lesson + 1, tutorial.step + 1, tutorial.patient);
    }
    fn button(input: &mut Input, state: ElementState) {
        input.on_event(&WindowEvent::MouseInput { device_id: DeviceId::dummy(), state, button: MouseButton::Left }, false, false);
    }
    fn frame(ctx: &egui::Context, tutorial: &mut Tutorial, game: &mut Game, camera: &mut Camera, events: Vec<egui::Event>) -> (egui::FullOutput, TutorialAction) {
        let view = Rect::from_min_size(Pos2::ZERO, vec2(1024.0, 768.0));
        let raw = egui::RawInput { screen_rect: Some(view), focused: true, max_texture_side: Some(8192), events, ..Default::default() };
        let mut action = TutorialAction::None;
        let output = ctx.run_ui(raw, |ui| action = tutorial.draw(ui.ctx(), game, camera, view, &Anchors::default()));
        (output, action)
    }
    fn text_center(shape: &egui::Shape, label: &str) -> Option<Pos2> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == label => Some(Rect::from_min_size(text.pos, text.galley.size()).center()),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| text_center(s, label)),
            _ => None,
        }
    }
    fn click(ctx: &egui::Context, tutorial: &mut Tutorial, game: &mut Game, camera: &mut Camera, label: &str) -> TutorialAction {
        // Settle the new step's Area size before using its hit rectangles.
        frame(ctx, tutorial, game, camera, Vec::new()).0.drop_without_applying_deltas();
        let (output, _) = frame(ctx, tutorial, game, camera, Vec::new());
        let at = output.shapes.iter().find_map(|s| text_center(&s.shape, label)).unwrap_or_else(|| panic!("tutorial button {label} is visible"));
        output.drop_without_applying_deltas();
        let mut action = TutorialAction::None;
        for pressed in [true, false] {
            let events = vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }];
            let (output, result) = frame(ctx, tutorial, game, camera, events);
            output.drop_without_applying_deltas();
            action = result;
        }
        action
    }

    let install = ta_vfs::install::discover(None).expect("Total Annihilation install");
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let event_loop = winit::event_loop::EventLoop::builder().with_any_thread(true).build().unwrap();
    let window = Arc::new(event_loop.create_window(winit::window::Window::default_attributes()
        .with_visible(false).with_inner_size(winit::dpi::PhysicalSize::new(1024, 768))).unwrap());
    let mut renderer = Renderer::new(window, 1024, 768).unwrap();
    let assets = Assets::load(Arc::new(vfs), Default::default(), &mut renderer, None, None, &Default::default()).unwrap();
    let mut checked = 0;
    for (n, spec) in LESSONS.iter().enumerate() {
        println!("Checking lesson {}: {}", n + 1, spec.title);
        let mut game = Game::new(&assets, &mut renderer, &setup(n)).unwrap();
        let mut tutorial = Tutorial::start(n, &mut game);
        let ctx = egui::Context::default();
        crate::theme::install_compact(&ctx);
        assert!(game.tutorial && !game.can_save());
        game.sim.players[game.local_player as usize].cheats.unlimited = true;
        let commander = game.local_commander().unwrap();
        let base = game.sim.unit(commander).unwrap().pos;
        let mut camera = camera();
        camera.focus = Vec3::from_array(base.to_f32());
        while !tutorial.finished {
            let previous = tutorial.step;
            let kind_name = |k| match k {
                Kind::Extractor => "ARMMEX", Kind::Generator => "ARMSOLAR", Kind::Factory => "ARMLAB",
                Kind::Tower => "ARMLLT", Kind::Radar => "ARMRAD", Kind::Builder => "ARMCK",
            };
            match tutorial.current().check {
                Check::CommanderSelected => game.selection = vec![commander],
                Check::CameraMoved => {
                    tutorial.update(&mut game, &camera, 0.1);
                    camera.yaw += 0.3;
                }
                Check::Owns(k) | Check::OwnsFinished(k) => {
                    let at = match k {
                        Kind::Tower => {
                            let a = tutorial.approach.unwrap();
                            FxVec3::new(Fx::from_f32(a.x), Fx::ZERO, Fx::from_f32(a.z))
                        }
                        Kind::Extractor => {
                            let &(x, z, _) = game.tutorial_markers.first().expect("metal deposit marked");
                            FxVec3::new(Fx::from_f32(x), Fx::ZERO, Fx::from_f32(z))
                        }
                        _ => base + FxVec3::from_ints(-120, 0, 150),
                    };
                    if k == Kind::Tower {
                        // Build the defence, placing the Commander by it as the next hint
                        // instructs. Spawning a distant tower left the fixture's Commander
                        // back at the start, outside the tower's build/repair approach.
                        let type_id = game.sim.types.id(kind_name(k)).unwrap();
                        let pos = game.sim.find_build_site(type_id, at, 32).unwrap();
                        game.issue(Command::Order { player: game.local_player, units: vec![commander], order: Order::Build { type_id, pos }, queue: false });
                        wait(&mut game, &mut tutorial, &camera);
                    } else {
                        spawn(&mut game, kind_name(k), at);
                    }
                }
                Check::CombatUnits(count) => {
                    for i in 0..count { spawn(&mut game, "ARMPW", base + FxVec3::from_ints(100 + i as i64 * 40, 0, 100)); }
                }
                Check::CombatSelected(_) => {
                    game.selection = game.sim.units().filter(|u| u.owner == game.local_player && !u.dead && is_combat(&game.sim, u)).map(|u| u.id).collect();
                }
                Check::Grouped => game.assign_group(1),
                Check::Fight => {
                    // Both movement rules, the default Alt+F and a rebound key, battlefield
                    // and minimap targets. Arming alone never checks off the step.
                    for modern in [false, true] {
                        for rebound in [false, true] {
                            game.sim.group_formations = modern;
                            game.used.fight = false;
                            let mut keys = ta_config::Keys::default();
                            if rebound { keys.set("fight", "Ctrl+G"); }
                            game.keys.sync(&keys);
                            let binding = crate::keys::Binding::parse(keys.get("fight")).unwrap();
                            let mut input = Input::default();
                            input.mouse = Vec2::new(800.0, 500.0);
                            input.modifiers = if rebound { ModifiersState::CONTROL } else { ModifiersState::ALT };
                            input.press(binding.key);
                            game.handle_input(&input, &camera, &renderer, false);
                            assert_eq!(game.command_mode, Some(CommandMode::Fight));
                            assert!(!game.used.fight);
                            input.end_frame();
                            input.modifiers = ModifiersState::empty();
                            let target = tutorial.outpost_centre(&game).unwrap();
                            game.cursor_override = Some(target);
                            button(&mut input, ElementState::Pressed);
                            game.handle_input(&input, &camera, &renderer, false);
                            input.end_frame();
                            button(&mut input, ElementState::Released);
                            game.handle_input(&input, &camera, &renderer, false);
                            assert!(tutorial.met(Progress::from(&game), &camera), "modern={modern}, rebound={rebound}");
                            assert_eq!(game.commands.iter().any(|c| matches!(c, Command::Formation { order: Order::Fight(_), .. })), modern);
                            game.commands.clear();
                            game.used.fight = false;
                            game.set_command_mode(Some(CommandMode::Fight));
                            game.order_selection_from_map(target, false, false);
                            assert!(tutorial.met(Progress::from(&game), &camera));
                            game.commands.clear();
                        }
                    }
                    game.sim.group_formations = true;
                    game.set_command_mode(Some(CommandMode::Fight));
                    let target = tutorial.outpost_centre(&game).unwrap();
                    game.order_selection_from_map(target, false, false);
                }
                Check::OutpostGone | Check::RaidBeaten => wait(&mut game, &mut tutorial, &camera),
                Check::Overlay(o) => game.toggle_overlay(o),
                Check::AreaMex => {
                    game.selection = vec![commander];
                    let &(x, z, _) = game.tutorial_markers.first().expect("metal deposit marked");
                    game.area_order(CommandMode::AreaMex, Vec3::new(x, 0.0, z), 200.0, false);
                    assert!(game.commands.iter().any(|c| matches!(c, Command::Order { order: Order::Build { .. }, .. })));
                }
                Check::AreaReclaim | Check::Reclaimed => {
                    game.selection = vec![commander];
                    step(&mut game);
                    let feature = game.sim.features.iter().find(|f| f.alive && game.sim.data.features[f.def].reclaimable && game.sim.explored(game.local_player, f.pos)).expect("a visible rock or wreck to reclaim").pos;
                    game.area_order(CommandMode::Reclaim, Vec3::from_array(feature.to_f32()), 100.0, false);
                    assert!(game.used.area_reclaim && game.used.reclaim);
                }
                Check::Grid => {
                    game.selection = vec![commander];
                    let solar = game.sim.types.id("ARMSOLAR").unwrap();
                    let site = game.sim.find_build_site(solar, base + FxVec3::from_ints(400, 0, 100), 32).unwrap();
                    let footprint = game.sim.types.get(solar).footprint;
                    for alt in [false, true] {
                        game.commands.clear();
                        let mut input = Input::default();
                        input.modifiers = if alt { ModifiersState::ALT } else { ModifiersState::empty() };
                        input.mouse = Vec2::new(700.0, 400.0);
                        game.begin_placement(solar);
                        game.cursor_override = Some(Vec3::from_array(site.to_f32()));
                        button(&mut input, ElementState::Pressed);
                        game.handle_input(&input, &camera, &renderer, false);
                        input.end_frame();
                        input.mouse += Vec2::splat(100.0);
                        game.cursor_override = Some(Vec3::from_array(site.to_f32()) + Vec3::new(footprint.0 as f32 * 64.0, 0.0, footprint.1 as f32 * 32.0));
                        game.handle_input(&input, &camera, &renderer, false);
                        input.end_frame();
                        button(&mut input, ElementState::Released);
                        game.handle_input(&input, &camera, &renderer, false);
                        assert!(game.commands.len() >= 4, "at least four buildable solar sites, Alt={alt}");
                        assert_eq!(game.used.grid, alt, "a plain row is not the Alt block the step teaches");
                    }
                }
                Check::QueueEdit => {
                    step(&mut game);
                    let queue = game.builder_queue(commander);
                    assert!(queue.len() >= 2, "the preceding block left buildings queued");
                    game.edit_builder_queue(commander, queue[1].0, true);
                }
                Check::Repaired => {
                    let patient = tutorial.patient.expect("the raid left a building to mend");
                    assert!(game.tutorial_markers.iter().any(|&(x, z, _)| {
                        let at = game.sim.unit(patient).unwrap().pos.to_f32();
                        x == at[0] && z == at[2]
                    }));
                    game.issue(Command::Order { player: game.local_player, units: vec![commander], order: Order::Repair(patient), queue: false });
                    wait(&mut game, &mut tutorial, &camera);
                }
                Check::Guarding => {
                    let builder = game.sim.units().find(|u| u.owner == game.local_player && is_kind(game.sim.unit_type(u), Kind::Builder)).unwrap().id;
                    let factory = game.sim.units().find(|u| u.owner == game.local_player && game.sim.unit_type(u).is_factory).unwrap().id;
                    game.issue(Command::Order { player: game.local_player, units: vec![builder], order: Order::Guard(factory), queue: false });
                }
                Check::Rally => {
                    let factory = game.sim.units().find(|u| u.owner == game.local_player && game.sim.unit_type(u).is_factory).unwrap().id;
                    game.selection = vec![factory];
                    game.order_selection_from_map(Vec3::from_array((base + FxVec3::from_ints(150, 0, 150)).to_f32()), false, false);
                    assert!(game.used.rally);
                    step(&mut game);
                    assert!(!game.sim.unit(factory).unwrap().rally.is_empty());
                }
                Check::Repeat => game.set_repeat(true),
                Check::Patrol => {
                    let builder = game.sim.units().find(|u| u.owner == game.local_player && is_kind(game.sim.unit_type(u), Kind::Builder)).unwrap().id;
                    game.selection = vec![builder];
                    game.set_command_mode(Some(CommandMode::Patrol));
                    game.order_selection_from_map(Vec3::from_array(base.to_f32()) + Vec3::X * 200.0, false, false);
                }
                Check::HoldFire => {
                    game.selection = game.sim.units().filter(|u| u.owner == game.local_player && is_combat(&game.sim, u)).map(|u| u.id).collect();
                    game.set_states(Some(0), None);
                }
                Check::Continue => {
                    assert!(!tutorial.met(Progress::from(&game), &camera));
                    assert!(matches!(click(&ctx, &mut tutorial, &mut game, &mut camera, "CONTINUE"), TutorialAction::None));
                    assert!(tutorial.finished, "CONTINUE completes the lesson");
                }
            }
            if !tutorial.finished {
                tutorial.update(&mut game, &camera, 1.3);
                assert_eq!(tutorial.step, previous + 1, "lesson {} step {}: {}", n + 1, previous + 1, tutorial.current().text);
            }
            checked += 1;
        }
        assert!(game.tutorial_markers.is_empty());
        if n + 1 < LESSONS.len() {
            assert!(matches!(click(&ctx, &mut tutorial, &mut game, &mut camera, "NEXT LESSON"), TutorialAction::Start(next) if next == n + 1));
        } else {
            assert!(matches!(click(&ctx, &mut tutorial, &mut game, &mut camera, "SET UP A SKIRMISH"), TutorialAction::Skirmish));
        }
        assert!(matches!(click(&ctx, &mut tutorial, &mut game, &mut camera, "MAIN MENU"), TutorialAction::Menu));
        if n == 2 {
            // Classic HUD deliberately omits the queue editor, so the lesson passes it over.
            tutorial.finished = false;
            tutorial.step = 4;
            tutorial.classic = true;
            game.used.queue_edit = false;
            tutorial.update(&mut game, &camera, 0.1);
            assert_eq!(tutorial.step, 5);
        }
        if n == 3 {
            // Losing the designated building must choose a surviving replacement and mark it.
            tutorial.finished = false;
            tutorial.step = 3;
            tutorial.done_for = 0.0;
            let old = tutorial.patient.unwrap();
            game.sim.remove_unit(old);
            tutorial.update(&mut game, &camera, 0.1);
            assert_ne!(tutorial.patient, Some(old));
            let replacement = tutorial.patient.expect("another building remains");
            assert!(game.sim.unit(replacement).unwrap().health < game.sim.unit_type(game.sim.unit(replacement).unwrap()).max_health);
            let buildings: Vec<_> = game.sim.units().filter(|u| u.owner == game.local_player && game.sim.unit_type(u).locomotion == Locomotion::Static).map(|u| u.id).collect();
            for id in buildings { game.sim.remove_unit(id); }
            tutorial.update(&mut game, &camera, 0.1);
            assert_eq!(tutorial.step, 4, "nothing remains to mend, so reclaim is next");
        }
    }
    assert_eq!(checked, LESSONS.iter().map(|l| l.steps.len()).sum::<usize>());
    println!("All {checked} tutorial steps progressed; default/rebound attack-move, formation rules, minimap orders, classic queue skip and destroyed repair targets passed");
}
