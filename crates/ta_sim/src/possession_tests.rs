//! Driving a unit from the first-person view (`Command::DirectControl`, `sim/possession.rs`): its
//! way under it, its weapons under the sight, and its being taken over and let go. Made-up units on
//! a flat map.

use super::*;
use crate::DirectInput;
use crate::unit::DirectControl;

/// No throttle, no turn, the sight level and due south (heading 0), the trigger let go.
const STILL: DirectInput = DirectInput { throttle: 0, turn: 0, course: None, heading: 0, pitch: 0, fire: false };

fn drive(sim: &mut Sim, unit: UnitId, input: DirectInput, ticks: usize) {
    for _ in 0..ticks {
        sim.step(&[Command::DirectControl { player: 0, unit, input: Some(input) }]);
    }
}

fn let_go(sim: &mut Sim, unit: UnitId) {
    sim.step(&[Command::DirectControl { player: 0, unit, input: None }]);
}

/// The shots `unit` fired since the events were last taken: each weapon's and where at.
fn shots(sim: &mut Sim, unit: UnitId) -> Vec<(usize, FxVec3)> {
    sim.take_events().into_iter().filter_map(|e| match e {
        SimEvent::WeaponFired { unit: shooter, weapon, target, .. } if shooter == unit => Some((weapon, target)),
        _ => None,
    }).collect()
}

fn weapon_id(sim: &Sim, name: &str) -> usize {
    sim.weapon_types.by_name(name).unwrap().id
}

fn put(sim: &mut Sim, name: &str, owner: u8, x: i64, z: i64) -> UnitId {
    sim.spawn_unit(sim.types.id(name).unwrap(), owner, FxVec3::from_ints(x, 60, z), 0, true).unwrap()
}

/// From a stop a driven unit gathers way at its acceleration (braking from a stop, it set off at
/// its brake rate, with a lurch), and sheds it at its brake rate; put astern from ahead it brakes to
/// a stop and gathers way astern at its acceleration again, not past the stop at its brake rate.
#[test]
fn a_driven_unit_gathers_way_at_its_acceleration_and_sheds_it_at_its_brake_rate() {
    let mut sim = sim_of(data_of(&[unit("DRIVER", "CanMove=1; MaxVelocity=2; Acceleration=0.1; BrakeRate=0.5; TurnRate=1000;")]), 1);
    let id = put(&mut sim, "DRIVER", 0, 500, 300);
    let t = sim.unit_type(sim.unit(id).unwrap()).clone();
    assert!(t.acceleration < t.brake);
    let ahead = DirectInput { throttle: 100, ..STILL };
    drive(&mut sim, id, ahead, 1);
    assert_eq!(sim.unit(id).unwrap().speed, t.acceleration, "from a stop, at its acceleration");
    drive(&mut sim, id, ahead, 60);
    assert_eq!(sim.unit(id).unwrap().speed, t.max_speed);
    drive(&mut sim, id, STILL, 1);
    assert_eq!(sim.unit(id).unwrap().speed, t.max_speed - t.brake, "throttle off: it brakes");
    let mut first_astern = None;
    for _ in 0..30 {
        drive(&mut sim, id, DirectInput { throttle: -100, ..STILL }, 1);
        let speed = sim.unit(id).unwrap().speed;
        if speed < Fx::ZERO && first_astern.is_none() {
            first_astern = Some(speed);
        }
    }
    assert_eq!(first_astern, Some(Fx::ZERO - t.acceleration), "past the stop, astern at its acceleration");
    assert_eq!(sim.unit(id).unwrap().speed, Fx::ZERO - t.max_speed / 2, "astern at half its top speed");
}

/// Let go going astern, or taken by another player, a unit comes to a stop: its orders only move
/// it ahead (from astern it gathered way slowly, counting itself stuck meanwhile), and the hands
/// on it are its old owner's no more.
#[test]
fn a_unit_let_go_or_captured_astern_stops() {
    let mut sim = sim_of(data_of(&[unit("DRIVER", "CanMove=1; MaxVelocity=2; Acceleration=0.1; BrakeRate=0.1; TurnRate=1000;")]), 2);
    let astern = DirectInput { throttle: -100, ..STILL };
    for capture in [false, true] {
        let id = put(&mut sim, "DRIVER", 0, 500, 300 + capture as i64 * 300);
        sim.step(&[Command::Order { player: 0, units: vec![id], order: Order::Move(FxVec3::from_ints(900, 60, 300 + capture as i64 * 300)), queue: false }]);
        drive(&mut sim, id, astern, 20);
        assert!(sim.unit(id).unwrap().speed < Fx::ZERO, "astern");
        if capture {
            sim.transfer_unit(id, 1);
            let u = sim.unit(id).unwrap();
            assert!(u.direct_control.is_none(), "captured, it is driven no more");
            assert!(u.speed >= Fx::ZERO);
        } else {
            let_go(&mut sim, id);
            let u = sim.unit(id).unwrap();
            assert!(u.direct_control.is_none());
            assert!(u.speed >= Fx::ZERO, "let go, it stops (and its orders take it on from there): {:?}", u.speed);
            assert!(u.path_goal.is_some(), "its order taken up again");
        }
    }
}

/// Driven square into a wall (its heading straight along an axis, so that sliding along the
/// wall goes nowhere), a unit stops: it does not run on the spot at full speed.
#[test]
fn a_unit_driven_square_into_a_wall_stops() {
    let mut sim = sim_of(data_of(&[unit("DRIVER", "CanMove=1; MaxVelocity=2; Acceleration=1; BrakeRate=1; TurnRate=1000; FootprintX=1; FootprintZ=1;")]), 1);
    for x in 0..64 {
        sim.world.occupy(Occupant::Feature(900), (x, 30), (1, 1), &[]);
    }
    let id = put(&mut sim, "DRIVER", 0, 500, 400);
    drive(&mut sim, id, DirectInput { throttle: 100, ..STILL }, 90);
    let u = sim.unit(id).unwrap();
    assert!(u.pos.z > Fx::from_int(440) && u.pos.z < Fx::from_int(496), "at the wall: {:?}", u.pos);
    assert!(u.speed.is_zero() && !u.moving, "stopped against it: speed {:?}", u.speed);
}

/// With nothing under the sight, the trigger fires at the place on the ground the sight looks at,
/// within the weapon's range (where it meets the ground, else the ground at the range ahead), not
/// at a point in the air; and only the weapons that would fire on that place by order: a laser,
/// and TA's anti-air too (told to attack the ground it fires on it), but not a torpedo on dry land,
/// nor a weapon naming its only targets.
#[test]
fn the_free_aim_fires_on_the_ground_the_sight_looks_at_with_the_weapons_that_take_it() {
    let data = data_of(&[
        unit("DRIVER", "CanMove=1; MaxVelocity=1; Weapon1=LASER; Weapon2=TORP; Weapon3=PICKY; OnlyTargetCategory3=VTOL;"),
        unit("FLAKKER", "CanMove=1; MaxVelocity=1; Weapon1=AAGUN;"),
        weapon_file("LASER", "range=300; turret=1;"),
        weapon_file("TORP", "range=300; turret=1; waterweapon=1;"),
        weapon_file("PICKY", "range=300; turret=1;"),
        weapon_file("AAGUN", "range=300; turret=1; toairweapon=1;"),
    ]);
    let mut sim = sim_of(data, 1);
    for name in ["DRIVER", "FLAKKER"] {
        let kind = sim.types.id(name).unwrap();
        // (A gun 10 px over the ground: it aims from half its model's height.)
        Arc::make_mut(&mut Arc::get_mut(&mut sim.types).unwrap().types[kind].model).height = Fx::from_int(20);
    }
    let (laser, aa) = (weapon_id(&sim, "LASER"), weapon_id(&sim, "AAGUN"));
    let driver = put(&mut sim, "DRIVER", 0, 300, 300);
    sim.take_events();
    // Looking down a little (about 11 degrees): the ground some 50 px ahead.
    drive(&mut sim, driver, DirectInput { pitch: -2000, fire: true, ..STILL }, 10);
    let fired = shots(&mut sim, driver);
    assert!(!fired.is_empty() && fired.iter().all(|&(w, _)| w == laser), "the laser alone: {fired:?}");
    for &(_, at) in &fired {
        assert_eq!(at.y, Fx::from_int(60), "on the ground: {at:?}");
        assert!(at.x == Fx::from_int(300) && at.z > Fx::from_int(340) && at.z < Fx::from_int(365), "where the sight meets it: {at:?}");
    }
    // Level: the ground at the range ahead.
    drive(&mut sim, driver, DirectInput { fire: true, ..STILL }, 10);
    let fired = shots(&mut sim, driver);
    assert!(!fired.is_empty() && fired.iter().all(|&(w, _)| w == laser), "the laser alone: {fired:?}");
    for &(_, at) in &fired {
        assert_eq!(at, FxVec3::from_ints(300, 60, 600), "on the ground at its range ahead");
    }
    // TA's anti-air fires on the ground by order, and so by the trigger.
    let flakker = put(&mut sim, "FLAKKER", 0, 600, 300);
    sim.step(&[Command::Order { player: 0, units: vec![flakker], order: Order::Attack(Target::Ground(FxVec3::from_ints(600, 60, 450))), queue: false }]);
    for _ in 0..10 { sim.step(&[]); }
    assert!(shots(&mut sim, flakker).iter().any(|&(w, _)| w == aa), "told to attack the ground, it fires on it");
    sim.step(&[Command::Order { player: 0, units: vec![flakker], order: Order::Stop, queue: false }]);
    drive(&mut sim, flakker, DirectInput { pitch: -2000, fire: true, ..STILL }, 10);
    assert!(shots(&mut sim, flakker).iter().any(|&(w, _)| w == aa), "and by the trigger");
}

/// A click holds the trigger for a tick: the burst it begins fires whole, and the next pull waits
/// out the reload (the rest of the burst waited for the next pull, which then fired with no
/// reload between).
#[test]
fn a_click_fires_a_whole_burst_and_the_next_pull_waits_for_the_reload() {
    let data = data_of(&[unit("DRIVER", "CanMove=1; MaxVelocity=1; Weapon1=BURST;"), weapon_file("BURST", "range=300; turret=1; burst=3; burstrate=0.1; reloadtime=2;")]);
    let mut sim = sim_of(data, 1);
    let id = put(&mut sim, "DRIVER", 0, 300, 300);
    sim.take_events();
    drive(&mut sim, id, DirectInput { fire: true, ..STILL }, 1);
    drive(&mut sim, id, STILL, 15);
    assert_eq!(shots(&mut sim, id).len(), 3, "the whole burst");
    drive(&mut sim, id, DirectInput { fire: true, ..STILL }, 30);
    assert!(shots(&mut sim, id).is_empty(), "reloading");
}

/// Held still by its script and told to go (`IMMOBILE`, packing up), a driven unit fires nothing,
/// as one on orders fires nothing then; standing, it fires.
#[test]
fn a_driven_unit_packing_up_to_go_fires_nothing() {
    let data = data_of(&[unit("DRIVER", "CanMove=1; MaxVelocity=1; Weapon1=LASER;"), weapon_file("LASER", "range=300; turret=1;")]);
    let mut sim = sim_of(data, 1);
    let id = put(&mut sim, "DRIVER", 0, 300, 300);
    sim.unit_mut(id).unwrap().immobile = true;
    sim.take_events();
    drive(&mut sim, id, DirectInput { fire: true, ..STILL }, 10);
    assert!(!shots(&mut sim, id).is_empty(), "held still and standing, it fires");
    drive(&mut sim, id, DirectInput { throttle: 100, fire: true, ..STILL }, 20);
    assert!(sim.unit(id).unwrap().packing_to_go());
    assert!(shots(&mut sim, id).is_empty(), "packing up, nothing");
}

/// A constructor taken over at work stops working (its build job, its nano stream and its build
/// stance end with it); its order waits, and let go it goes back to the job.
#[test]
fn a_constructor_taken_over_at_work_stops_and_goes_back_to_it_let_go() {
    let data = data_of(&[
        unit("BUILDER", "Builder=1; WorkerTime=30; CanMove=1; MaxVelocity=1; BuildDistance=128; FootprintX=1; FootprintZ=1;"),
        unit("FRAME", "BuildTime=3000; BuildCostMetal=100; BuildCostEnergy=100; FootprintX=2; FootprintZ=2;"),
    ]);
    let mut sim = sim_of(with_menu(data, "BUILDER", &["FRAME"]), 1);
    sim.players[0].endless = true;
    let builder = put(&mut sim, "BUILDER", 0, 200, 300);
    let order = Order::Build { type_id: sim.types.id("FRAME").unwrap(), pos: FxVec3::from_ints(280, 60, 300) };
    sim.step(&[Command::Order { player: 0, units: vec![builder], order: order.clone(), queue: false }]);
    let frame = (0..120).find_map(|_| {
        sim.step(&[]);
        sim.unit(builder).unwrap().build.map(|job| job.target)
    }).expect("at work");
    drive(&mut sim, builder, STILL, 1);
    let u = sim.unit(builder).unwrap();
    assert!(u.build.is_none() && !u.in_build_stance, "its job ends with the take-over");
    assert_eq!(u.orders.front(), Some(&order), "its order waits");
    let work = sim.unit(frame).unwrap().build_work;
    drive(&mut sim, builder, STILL, 30);
    assert_eq!(sim.unit(frame).unwrap().build_work, work, "no work done driven");
    let_go(&mut sim, builder);
    for _ in 0..120 { sim.step(&[]); }
    assert!(sim.unit(frame).unwrap().build_work > work, "back at work");
}

/// Taken over, a unit leaves the formation it was in: the rest do not wait for it to form up.
#[test]
fn a_unit_taken_over_leaves_its_formation() {
    let data = data_of(&[unit("TANK", "CanMove=1; MaxVelocity=4; Acceleration=1; BrakeRate=1; TurnRate=4096; SteeringMode=2; FootprintX=1; FootprintZ=1;")]);
    let mut sim = sim_of(data, 1);
    sim.group_formations = true;
    let (a, b) = (put(&mut sim, "TANK", 0, 200, 300), put(&mut sim, "TANK", 0, 200, 350));
    sim.step(&[Command::Formation { player: 0, units: vec![a, b], order: Order::Move(FxVec3::from_ints(900, 60, 300)), queue: false, shape: crate::formation::Shape::Keep }]);
    assert!(sim.formations.iter().any(|f| f.mates.iter().any(|m| m.id == a)), "in the formation");
    drive(&mut sim, a, STILL, 1);
    let u = sim.unit(a).unwrap();
    assert!(u.formations.is_empty() && u.face.is_none() && !u.orders.is_empty(), "out of it, its order kept");
    assert!(sim.formations.iter().all(|f| f.mates.iter().all(|m| m.id != a)), "the formation goes without it");
    assert!(sim.formations.iter().any(|f| f.mates.iter().any(|m| m.id == b)), "the other still in it");
}

/// Its owner lets a driven unit go in whatever state it is: one being taken apart too (built up
/// again, it carried on with its last input until that lapsed).
#[test]
fn a_unit_being_taken_apart_is_let_go() {
    let mut sim = sim_of(data_of(&[unit("DRIVER", "CanMove=1; MaxVelocity=2; TurnRate=1000;")]), 1);
    let id = put(&mut sim, "DRIVER", 0, 500, 300);
    drive(&mut sim, id, DirectInput { throttle: 100, ..STILL }, 5);
    let total = sim.unit_type(sim.unit(id).unwrap()).build_work;
    sim.unit_mut(id).unwrap().set_build_work(total / 2, total);
    let_go(&mut sim, id);
    assert!(sim.unit(id).unwrap().direct_control.is_none());
}

/// Let go, each weapon that had something under the sight lets it go as a lost target does: its
/// script's `TargetCleared` (the weapon's number) puts its turret back to rest.
#[test]
fn letting_go_puts_the_turrets_back_to_rest() {
    use crate::cob::CobProgram;
    use ta_formats::cob::{CobFunction, CobScript, op};
    let data = data_of(&[unit("DRIVER", "CanMove=1; MaxVelocity=1; Weapon1=LASER; Weapon2=LASER;"), weapon_file("LASER", "range=300; turret=1;")]);
    let mut sim = sim_of(data, 1);
    let kind = sim.types.id("DRIVER").unwrap();
    let ty = &mut Arc::get_mut(&mut sim.types).unwrap().types[kind];
    // TargetCleared(weapon): static 0 += weapon.
    let script = CobScript {
        functions: vec![CobFunction { name: "TargetCleared".into(), offset: 0 }],
        code: vec![op::CREATE_LOCAL, op::PUSH_STATIC, 0, op::PUSH_LOCAL, 0, op::ADD, op::POP_STATIC, 0, op::PUSH_CONSTANT, 0, op::RETURN],
        static_var_count: 1,
        ..Default::default()
    };
    ty.script = Some(Arc::new(CobProgram::new(script, &[])));
    ty.callins.target_cleared = true;
    let id = put(&mut sim, "DRIVER", 0, 300, 300);
    drive(&mut sim, id, STILL, 3);
    assert!(sim.unit(id).unwrap().weapons[..2].iter().all(|w| w.target.is_some()), "both on the sight's place");
    assert_eq!(sim.unit(id).unwrap().cob.as_ref().unwrap().statics[0], 0);
    let_go(&mut sim, id);
    assert_eq!(sim.unit(id).unwrap().cob.as_ref().unwrap().statics[0], 1 + 2, "TargetCleared(1) and TargetCleared(2)");
}

/// In the crowd a driven unit with way on is moving, giving more of the room to those standing
/// (counted as standing, it shoved them aside), and one standing, its sight on the ground, is not
/// holding a fight: its own side's units held up against it slip past it, as past any standing
/// idle in their way.
#[test]
fn in_the_crowd_a_driven_unit_moves_with_its_way_and_fights_only_what_it_aims_at() {
    let data = data_of(&[unit("TANK", "CanMove=1; MaxVelocity=2; FootprintX=1; FootprintZ=1;")]);
    let mut sim = sim_of(data, 2);
    let driven = |input| DirectControl { aim_piece: None, player: 0, input, expires: u64::MAX, pull: None };
    let (a, b) = (put(&mut sim, "TANK", 0, 500, 300), put(&mut sim, "TANK", 0, 500, 300));
    let body = sim.unit_type(sim.unit(a).unwrap()).radius * 4 / 5;
    // Overlapping by 2 px: `a` driven ahead, `b` standing idle.
    sim.unit_mut(b).unwrap().pos.x = Fx::from_int(500) + body * 2 - Fx::from_int(2);
    {
        let u = sim.unit_mut(a).unwrap();
        u.direct_control = Some(driven(DirectInput { throttle: 100, ..STILL }));
        u.speed = Fx::ONE;
    }
    let (a0, b0) = (sim.unit(a).unwrap().pos.x, sim.unit(b).unwrap().pos.x);
    sim.resolve_collisions();
    let (pushed_a, pushed_b) = ((a0 - sim.unit(a).unwrap().pos.x).abs(), (sim.unit(b).unwrap().pos.x - b0).abs());
    assert!(pushed_a > pushed_b * 2, "the mover gives way: {pushed_a:?} against {pushed_b:?}");
    // `b` drives on into `a`, standing now with its sight on the ground; then on an enemy.
    let enemy = put(&mut sim, "TANK", 1, 900, 900);
    for (aim, slips) in [(Target::Ground(FxVec3::from_ints(500, 60, 600)), true), (Target::Unit(enemy), false)] {
        sim.unit_mut(b).unwrap().pos.x = Fx::from_int(500) + body * 2 - Fx::from_int(2);
        {
            let u = sim.unit_mut(a).unwrap();
            u.pos.x = Fx::from_int(500);
            u.speed = Fx::ZERO;
            u.direct_control = Some(driven(STILL));
            u.weapons[0].target = Some(aim);
        }
        {
            let u = sim.unit_mut(b).unwrap();
            u.path_goal = Some(FxVec3::from_ints(100, 60, 300));
            u.heading = -16384;
            u.head_on_tick = 0;
        }
        sim.tick += 100;
        sim.resolve_collisions();
        assert_eq!(sim.unit(b).unwrap().head_on_tick == sim.tick, slips, "held up by one standing with {aim:?}: slips past it {slips}");
    }
}

/// A cloaked unit going astern is on the move: it pays its cloak's cost on the move, not standing.
#[test]
fn a_cloaked_unit_going_astern_pays_for_its_cloak_on_the_move() {
    let mut sim = sim_of(data_of(&[unit("SPY", "CanMove=1; MaxVelocity=2; Acceleration=1; BrakeRate=1; TurnRate=1000; CloakCost=30; CloakCostMoving=300;")]), 1);
    let id = put(&mut sim, "SPY", 0, 500, 300);
    sim.unit_mut(id).unwrap().cloak_on = true;
    drive(&mut sim, id, DirectInput { throttle: -100, ..STILL }, 5);
    assert!(sim.unit(id).unwrap().speed < Fx::ZERO && sim.unit(id).unwrap().cloaked);
    let before = sim.players[0].economy.energy;
    drive(&mut sim, id, DirectInput { throttle: -100, ..STILL }, 1);
    let paid = before - sim.players[0].economy.energy;
    assert_eq!(paid, Fx::from_int(300) / TICKS_PER_SECOND, "on the move's cost: {paid:?}");
}

/// Steered by a course, a driven unit comes round to it at its turn rate, on the spot too, and no
/// further (no swinging past it and back); the shorter way round; the course steers it, not `turn`
/// beside it; and a course off the circle is refused, as the rest of the input out of its bounds.
#[test]
fn a_unit_steered_by_a_course_comes_round_to_it_at_its_turn_rate_and_no_further() {
    let mut sim = sim_of(data_of(&[unit("DRIVER", "CanMove=1; MaxVelocity=2; Acceleration=0.1; BrakeRate=0.5; TurnRate=1000;")]), 1);
    let id = put(&mut sim, "DRIVER", 0, 500, 300);
    let rate = sim.unit_type(sim.unit(id).unwrap()).turn_rate;
    let start = sim.unit(id).unwrap().pos;
    let course = rate * 5 + rate / 2;
    let face = DirectInput { course: Some(course), turn: -1, ..STILL };
    drive(&mut sim, id, face, 1);
    assert_eq!(sim.unit(id).unwrap().heading, rate, "at its turn rate, the course's way (not the turn's)");
    drive(&mut sim, id, face, 10);
    let u = sim.unit(id).unwrap();
    assert_eq!(u.heading, course, "come round to it, and no further");
    assert!(u.speed.is_zero() && u.pos == start, "no throttle: it turned on the spot");
    // Just short of half a turn round to the right (past half a turn to the left): it turns right.
    let round = crate::fixed::wrap_angle(course - 32768 + rate * 3);
    drive(&mut sim, id, DirectInput { course: Some(round), ..STILL }, 1);
    assert_eq!(sim.unit(id).unwrap().heading, crate::fixed::wrap_angle(course - rate), "the shorter way round");
    let held = |sim: &Sim| sim.unit(id).unwrap().direct_control.map(|control| control.input.course);
    let mut off = DirectInput { course: Some(40_000), ..STILL };
    drive(&mut sim, id, off, 1);
    assert_eq!(held(&sim), Some(Some(round)), "a course off the circle is refused (the last input holds)");
    off.course = Some(-32_768);
    drive(&mut sim, id, off, 1);
    assert_eq!(held(&sim), Some(Some(-32_768)), "the circle's end is on it");
}

/// Off its course a driven unit goes slower the further it faces from it (by the angle's cosine:
/// sixty degrees off, half its speed), and from a right angle off or more it turns on the spot
/// first: sent the way it does not face, it goes that way rather than round in an arc.
#[test]
fn off_its_course_a_driven_unit_slows_and_from_a_right_angle_off_turns_on_the_spot() {
    let mut sim = sim_of(data_of(&[unit("DRIVER", "CanMove=1; MaxVelocity=2; Acceleration=2; BrakeRate=2; TurnRate=1000;")]), 1);
    let id = put(&mut sim, "DRIVER", 0, 500, 300);
    let (top, rate) = {
        let t = sim.unit_type(sim.unit(id).unwrap());
        (t.max_speed, t.turn_rate)
    };
    let start = sim.unit(id).unwrap().pos;
    // Sent straight back: on the spot until it is within a right angle of its course.
    let back = DirectInput { throttle: DirectInput::FULL, course: Some(-32_768), ..STILL };
    let mut ticks = 0;
    loop {
        drive(&mut sim, id, back, 1);
        let u = sim.unit(id).unwrap();
        let off = crate::fixed::wrap_angle(-32_768 - u.heading);
        if off.abs() < crate::fixed::QUARTER_TURN {
            assert_eq!(u.speed, top * crate::fixed::cos(off), "within a right angle it sets off, slower the further off");
            break;
        }
        assert!(u.speed.is_zero() && u.pos.x == start.x && u.pos.z == start.z, "a right angle off or more: on the spot (tick {ticks})");
        ticks += 1;
        assert!(ticks <= 32_768 / rate as usize, "it comes round");
    }
    drive(&mut sim, id, back, 30);
    let u = sim.unit(id).unwrap();
    assert_eq!((u.heading, u.speed), (-32_768, top), "on its course, at its top speed");
    assert!(u.pos.z < start.z, "and on its way back: {:?}", u.pos);
    // Held sixty degrees off (a unit that barely turns), it goes at half its top speed.
    let mut sim = sim_of(data_of(&[unit("SLOW", "CanMove=1; MaxVelocity=2; Acceleration=2; BrakeRate=2; TurnRate=1;")]), 1);
    let id = put(&mut sim, "SLOW", 0, 500, 300);
    let top = sim.unit_type(sim.unit(id).unwrap()).max_speed;
    let sixty = crate::fixed::FULL_TURN / 6;
    drive(&mut sim, id, DirectInput { throttle: DirectInput::FULL, course: Some(sixty), ..STILL }, 1);
    let u = sim.unit(id).unwrap();
    assert_eq!(u.speed, top * crate::fixed::cos(sixty - 1));
    assert!((u.speed.to_f32() - top.to_f32() / 2.0).abs() < 0.001, "half its top speed: {:?}", u.speed);
}

/// The throttle is a share of the top speed in hundredths (a pad's stick pushed part way): half of
/// it ahead, half its top speed; half of it astern, half of its speed astern (a quarter).
#[test]
fn the_throttle_is_a_share_of_the_top_speed() {
    let mut sim = sim_of(data_of(&[unit("DRIVER", "CanMove=1; MaxVelocity=2; Acceleration=0.25; BrakeRate=0.25; TurnRate=1000;")]), 1);
    let id = put(&mut sim, "DRIVER", 0, 500, 300);
    let top = sim.unit_type(sim.unit(id).unwrap()).max_speed;
    drive(&mut sim, id, DirectInput { throttle: 50, ..STILL }, 30);
    assert_eq!(sim.unit(id).unwrap().speed, top / 2);
    drive(&mut sim, id, DirectInput { throttle: -50, ..STILL }, 60);
    assert_eq!(sim.unit(id).unwrap().speed, Fx::ZERO - top / 4);
}

/// Held still by its script (a weapon unpacked), a driven unit with a course alone (a walker
/// facing where its driver looks) is not packed up: only a throttle, or a turn by hand, wants it
/// on the move.
#[test]
fn a_course_alone_does_not_pack_up_a_held_unit() {
    let mut sim = sim_of(data_of(&[unit("DRIVER", "CanMove=1; MaxVelocity=2; Acceleration=1; BrakeRate=1; TurnRate=1000;")]), 1);
    let id = put(&mut sim, "DRIVER", 0, 500, 300);
    sim.unit_mut(id).unwrap().immobile = true;
    drive(&mut sim, id, DirectInput { course: Some(8_000), turn: 1, ..STILL }, 5);
    let u = sim.unit(id).unwrap();
    assert!(!u.moving && u.heading == 0, "held, facing where it faced, not wanted on the move");
    drive(&mut sim, id, DirectInput { course: Some(8_000), throttle: DirectInput::FULL, ..STILL }, 1);
    assert!(sim.unit(id).unwrap().moving, "with a throttle, wanted on the move");
}

/// A click on a gun not yet on the sight (a gun fixed in its hull, the hull coming round to its
/// course) holds the trigger until the gun comes on, and fires then, once; too long coming round
/// (past [`crate::unit::PULL_TICKS`]), the click lapses and nothing fires.
#[test]
fn a_click_waits_for_the_gun_to_come_round_and_fires_once() {
    let data = data_of(&[unit("FIXED", "CanMove=1; MaxVelocity=1; TurnRate=1000; Weapon1=GUN;"), weapon_file("GUN", "range=300; reloadtime=0.1; tolerance=500;")]);
    let mut sim = sim_of(data, 1);
    let id = put(&mut sim, "FIXED", 0, 300, 300);
    sim.take_events();
    // The sight and the course a quarter turn round: some sixteen ticks coming round.
    let round = DirectInput { course: Some(16_384), heading: 16_384, ..STILL };
    drive(&mut sim, id, DirectInput { fire: true, ..round }, 1);
    drive(&mut sim, id, round, 3);
    assert!(shots(&mut sim, id).is_empty(), "not on the sight yet");
    drive(&mut sim, id, round, 15);
    assert_eq!(shots(&mut sim, id).len(), 1, "fired as it came on, once");
    drive(&mut sim, id, round, 20);
    assert!(shots(&mut sim, id).is_empty(), "the click spent");
    // Half a turn round: longer than a click holds.
    let back = DirectInput { course: Some(-16_384), heading: -16_384, ..STILL };
    drive(&mut sim, id, DirectInput { fire: true, ..back }, 1);
    drive(&mut sim, id, back, 40);
    assert!(shots(&mut sim, id).is_empty(), "lapsed before it came round");
    assert_eq!(sim.unit(id).unwrap().heading, -16_384);
    assert!(sim.unit(id).unwrap().direct_control.unwrap().pull.is_none(), "and forgotten");
}

/// Driven square on into a post (a tree, a corner), a unit slips round it rather than stop dead,
/// still facing its way, and goes on past it; along a wall met at a slant it slides as before.
#[test]
fn a_unit_driven_square_into_a_post_slips_round_it() {
    let mut sim = sim_of(data_of(&[unit("DRIVER", "CanMove=1; MaxVelocity=2; Acceleration=1; BrakeRate=1; TurnRate=1000; FootprintX=1; FootprintZ=1;")]), 1);
    sim.world.occupy(Occupant::Feature(900), (31, 30), (1, 1), &[]);
    let id = put(&mut sim, "DRIVER", 0, 504, 400);
    drive(&mut sim, id, DirectInput { throttle: 100, ..STILL }, 90);
    let u = sim.unit(id).unwrap();
    assert!(u.pos.z > Fx::from_int(530), "past the post: {:?}", u.pos);
    assert_eq!(u.heading, 0, "still facing its way");
    // A wall met at a slant: along it, at its x's share of the way.
    let mut sim = sim_of(data_of(&[unit("DRIVER", "CanMove=1; MaxVelocity=2; Acceleration=1; BrakeRate=1; TurnRate=1000; FootprintX=1; FootprintZ=1;")]), 1);
    for x in 0..64 {
        sim.world.occupy(Occupant::Feature(900), (x, 30), (1, 1), &[]);
    }
    let id = put(&mut sim, "DRIVER", 0, 300, 440);
    let slant = DirectInput { throttle: 100, course: Some(12_000), ..STILL };
    drive(&mut sim, id, slant, 60);
    let u = sim.unit(id).unwrap();
    assert!(u.pos.z < Fx::from_int(488) && u.pos.x > Fx::from_int(380), "slid along the wall, short of its cells: {:?}", u.pos);
}
