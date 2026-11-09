//! Guided lessons for new players: a game on Painted Desert with a list of steps, each an
//! instruction and a check against what the player has done (units owned or selected,
//! orders given, map views shown). The driver runs on the client: it reads the game, sets
//! the scene at the start (lesson 2's squad and enemy outpost) and marks places on the
//! ground, but never plays for the player. The enemy is a player without a computer
//! behind it; lessons end when their steps do, and keep no replay or saves.
//!
//! The guide shows the way a click at a time: each step names what it is about (`Aim`), and
//! the interface's anchors (`hud::anchors`) say where the next click goes: the unit to select
//! on the map, the build menu's tab, page and picture, an order's button. A step counts what it
//! asks for as it goes, and says more when the player seems stuck.

use egui::{Color32, CornerRadius, Margin, Pos2, Rect, RichText, Stroke, StrokeKind, pos2, vec2};
use glam::Vec3;
use ta_render::camera::Camera;
use ta_sim::{Fx, FxVec3, Locomotion, UnitId};
use ta_ui::i18n::{n, t, tf};
use ta_ui::panel::{self, Exit, Panel};
use ta_ui::widgets::{self as w, ActionButton};

use crate::game::{CommandMode, Game};
use crate::hud::anchors::Anchors;
use crate::overlay::Overlay;
use crate::setup::{Controller, GameSetup};
use crate::theme::*;

/// Something the player has done, noted by the game for the tutorial's checks.
#[derive(Clone, Copy, Debug, Default)]
pub struct Used {
    pub fight: bool,
    pub area_mex: bool,
    pub area_reclaim: bool,
    pub queue_edit: bool,
    /// A block of four or more buildings placed at once with Alt.
    pub grid: bool,
    /// A unit told to guard (a builder helping a factory).
    pub guard: bool,
    /// A builder sent on patrol.
    pub patrol: bool,
    pub repair: bool,
    /// Something reclaimed (a wreck, a rock; one or an area's).
    pub reclaim: bool,
    /// A factory's rally point set.
    pub rally: bool,
    /// A factory's repeat turned on.
    pub repeat: bool,
    /// Units told to hold fire.
    pub hold_fire: bool,
}

impl Used {
    /// Note the player's actions at their shared dispatch point, including formation orders.
    /// Merely arming an order, or sending it with no eligible units, does not do the lesson.
    pub fn record(&mut self, sim: &ta_sim::Sim, player: u8, command: &ta_sim::Command) {
        use ta_sim::{Command, Order};
        let own = |id: UnitId| sim.unit(id).filter(|u| u.owner == player && !u.dead && u.is_complete());
        let factory = |id| own(id).is_some_and(|u| sim.unit_type(u).is_factory);
        let builder = |u: &ta_sim::Unit| {
            let t = sim.unit_type(u);
            t.is_builder && !t.is_factory && t.locomotion != Locomotion::Static
        };
        match command {
            Command::Order { player: p, units, order, .. } | Command::Formation { player: p, units, order, .. } if *p == player => {
                // Factories take movement/guard orders as rally routes in the simulation.
                if matches!(order, Order::Move(_) | Order::Fight(_) | Order::Patrol(_) | Order::Guard(_)) {
                    self.rally |= units.iter().any(|&id| factory(id));
                }
                let mut ours = units.iter().filter_map(|&id| own(id));
                match order {
                    Order::Fight(_) => self.fight |= ours.any(|u| is_combat(sim, u)),
                    Order::Guard(target) if factory(*target) => {
                        self.guard |= ours.any(|u| builder(u) && !sim.unit_type(u).def.is_commander && u.id != *target);
                    }
                    Order::Repair(_) => self.repair |= ours.any(builder),
                    Order::Reclaim(_) | Order::ReclaimUnit(_) => self.reclaim |= ours.any(|u| sim.unit_type(u).def.can_reclaim),
                    Order::Patrol(_) => self.patrol |= ours.any(|u| builder(u) && !sim.unit_type(u).def.is_commander),
                    _ => {}
                }
            }
            Command::QueueFront { player: p, factory: id, index } if *p == player && factory(*id) => {
                self.queue_edit |= own(*id).is_some_and(|u| (*index as usize) < u.factory_queue.len());
            }
            // The factory HUD removes an entry with Produce(-1), not EditQueue.
            Command::Produce { player: p, factory: id, type_id, count } if *p == player && *count < 0 && factory(*id) => {
                self.queue_edit |= own(*id).is_some_and(|u| u.factory_queue.contains(type_id));
            }
            Command::EditQueue { player: p, unit, type_id, pos, .. } if *p == player => {
                self.queue_edit |= own(*unit).is_some_and(|u| match pos {
                    Some(pos) => u.orders.iter().any(|o| matches!(o, Order::Build { type_id: t, pos: at } if t == type_id && at == pos)),
                    None => sim.unit_type(u).is_factory && u.factory_queue.contains(type_id),
                });
            }
            Command::SetRally { player: p, factory: id, .. } if *p == player => self.rally |= factory(*id),
            Command::SetRepeat { player: p, factories, on: true } if *p == player => self.repeat |= factories.iter().any(|&id| factory(id)),
            Command::SetStates { player: p, units, fire: Some(0), .. } if *p == player => {
                self.hold_fire |= units.iter().filter_map(|&id| own(id)).any(|u| is_combat(sim, u));
            }
            _ => {}
        }
    }
}

/// The parts of the battle that lesson checks observe, independent of rendering and input.
struct Progress<'a> {
    sim: &'a ta_sim::Sim,
    me: u8,
    selection: &'a [UnitId],
    groups: &'a [Vec<UnitId>],
    used: &'a Used,
    overlay: Option<Overlay>,
}

impl<'a> From<&'a Game> for Progress<'a> {
    fn from(game: &'a Game) -> Self {
        Self { sim: &game.sim, me: game.local_player, selection: &game.selection, groups: &game.groups, used: &game.used, overlay: game.overlay }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Extractor,
    Generator,
    Factory,
    /// A building with a gun.
    Tower,
    Radar,
    /// A builder that walks (not the Commander).
    Builder,
}

#[derive(Clone, Copy)]
enum Check {
    CommanderSelected,
    CameraMoved,
    /// Started (or finished) building one.
    Owns(Kind),
    OwnsFinished(Kind),
    CombatUnits(usize),
    CombatSelected(usize),
    Grouped,
    Fight,
    Overlay(Overlay),
    AreaMex,
    AreaReclaim,
    QueueEdit,
    Grid,
    OutpostGone,
    /// Lesson 4's raiders all destroyed.
    RaidBeaten,
    /// The damaged building mended to full health.
    Repaired,
    Reclaimed,
    Guarding,
    Rally,
    Repeat,
    Patrol,
    HoldFire,
    /// The player clicks CONTINUE.
    Continue,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Marker {
    None,
    /// The metal deposit nearest the Commander.
    Metal,
    /// Lesson 2's enemy outpost.
    Outpost,
    /// Lesson 4's way in for the raiders, where its tower goes.
    Approach,
    /// Lesson 4's raiders.
    Raid,
    /// Where the raiders fell, and their wrecks lie.
    Wrecks,
    /// The building to mend.
    Patient,
}

/// What a step is about: the guide leads the player to it a click at a time.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Aim {
    None,
    /// Our Commander, to select.
    Commander,
    /// Our combat units, to select together.
    Squad,
    /// Building this (a unit's name): a builder of it selected, then the build menu's tab,
    /// page and picture.
    Build(&'static str),
    /// A factory making this (a unit's name): the factory selected, then the picture.
    Produce(&'static str),
    /// An order's button (its key binding's action), with units that can give it selected.
    Order(&'static str),
    /// The top bar's MAP menu.
    MapMenu,
    /// The Commander's build queue.
    Queue,
    /// Our factory, to select (its rally point is then a click on the ground).
    Factory,
    /// The factory's Repeat switch.
    Repeat,
    /// The fire order button, with our combat units selected.
    FireState,
}

/// What the driver does as a step begins.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Enter {
    None,
    /// Lesson 4's raiders set out for the base.
    Raid,
    /// One of our buildings to mend: the most damaged, else one wounded for the lesson.
    Damage,
}

/// How a step reads on TA's classic interface, whose build menu has pages but no tabs, and
/// which has no MAP menu or queue to reorder.
#[derive(Clone, Copy)]
enum Classic {
    Same,
    Text(&'static str),
    /// It cannot be done there: passed over.
    Skip,
}

struct Step {
    text: &'static str,
    check: Check,
    marker: Marker,
    aim: Aim,
    /// Said when the player seems stuck (empty: nothing more to say).
    hint: &'static str,
    classic: Classic,
    enter: Enter,
}

impl Step {
    const fn new(text: &'static str, check: Check) -> Step {
        Step { text, check, marker: Marker::None, aim: Aim::None, hint: "", classic: Classic::Same, enter: Enter::None }
    }

    const fn enter(self, enter: Enter) -> Step {
        Step { enter, ..self }
    }

    const fn marker(self, marker: Marker) -> Step {
        Step { marker, ..self }
    }

    const fn aim(self, aim: Aim) -> Step {
        Step { aim, ..self }
    }

    const fn hint(self, hint: &'static str) -> Step {
        Step { hint, ..self }
    }

    const fn classic(self, classic: Classic) -> Step {
        Step { classic, ..self }
    }
}

pub struct Lesson {
    pub title: &'static str,
    pub summary: &'static str,
    /// What it teaches, a few words each (the lessons page).
    teaches: &'static [&'static str],
    /// About how long it takes.
    minutes: u8,
    steps: &'static [Step],
}

/// Every lesson plays here: open desert with clear metal deposits and rocks to reclaim.
pub const MAP: &str = "Painted Desert";

pub const LESSONS: [Lesson; 5] = [
    Lesson {
        title: n("THE BASICS"),
        summary: n("Your Commander, the camera, metal, energy and a first factory."),
        teaches: &[n("Selecting units"), n("The camera"), n("Metal and energy"), n("Factories")],
        minutes: 6,
        steps: &[
            Step::new(
                n("Welcome, Commander. The walking machine in the middle of the screen is your Commander: it builds, it fights, and losing it usually loses the battle. Click it to select it."),
                Check::CommanderSelected,
            )
            .aim(Aim::Commander)
            .hint(n("Lost sight of it? {select_commander} selects the Commander from anywhere, and SHOW ME brings the camera to it.")),
            Step::new(
                n("Look around. The arrow keys, the screen edges or dragging with the middle button scroll; the mouse wheel zooms towards the cursor, and dragging with the right button turns the view."),
                Check::CameraMoved,
            )
            .hint(n("Push the pointer against an edge of the screen to scroll, or turn the mouse wheel to zoom.")),
            Step::new(
                n("Everything is made of metal and energy. Metal comes from extractors on metal deposits. Select the Commander, pick the Metal Extractor from the ECONOMY tab of the build menu at the bottom right and place it on the marked deposit."),
                Check::Owns(Kind::Extractor),
            )
            .marker(Marker::Metal)
            .aim(Aim::Build("ARMMEX"))
            .classic(Classic::Text(n("Everything is made of metal and energy. Metal comes from extractors on metal deposits. Select the Commander, pick the Metal Extractor from the build menu on the left and place it on the marked deposit.")))
            .hint(n("The outline turns green where the extractor can stand. Bring it onto the marked ring and click to place it.")),
            Step::new(
                n("Energy comes from solar collectors and wind generators. Build a Solar Collector next to your Commander. The bars at the top show your stores and how fast they fill or drain; they warn you when one runs dry."),
                Check::Owns(Kind::Generator),
            )
            .aim(Aim::Build("ARMSOLAR"))
            .hint(n("The Solar Collector is a flat, square panel. Pick it, then click on open ground near the Commander.")),
            Step::new(
                n("Factories build your army. Build a Kbot Lab (on the FACTORY tab) on a flat spot near the Commander, and wait for it to finish."),
                Check::OwnsFinished(Kind::Factory),
            )
            .aim(Aim::Build("ARMLAB"))
            .classic(Classic::Text(n("Factories build your army. Build a Kbot Lab from the build menu on a flat spot near the Commander, and wait for it to finish.")))
            .hint(n("Building takes metal and energy as it goes: while either bar at the top is empty, everything builds more slowly.")),
            Step::new(
                n("Select the Kbot Lab and click the Peewee three times to queue three of them (right-click one to take it off the queue). New units wait in front of the lab."),
                Check::CombatUnits(3),
            )
            .aim(Aim::Produce("ARMPW"))
            .hint(n("Click the lab itself on the map to select it: its build menu shows the units it makes.")),
            Step::new(
                n("That is the whole cycle: extract, generate, build, fight. Keep both incomes above what you spend and keep your builders busy. Lesson complete!"),
                Check::Continue,
            ),
        ],
    },
    Lesson {
        title: n("FIRST BLOOD"),
        summary: n("Select and group an army, attack-move, and destroy an enemy outpost."),
        teaches: &[n("Box selection"), n("Control groups"), n("Attack-move"), n("Destroying a base")],
        minutes: 5,
        steps: &[
            Step::new(n("Your Kbot Lab has built a squad of Peewees. Drag a box around them with the left mouse button to select them all."), Check::CombatSelected(4))
                .aim(Aim::Squad)
                .hint(n("Press the left button on open ground beside the squad, hold it, drag the box over them all and let go.")),
            Step::new(n("Press Ctrl+1 to make them group 1. Pressing 1 selects them again from anywhere; pressing it twice brings the camera to them."), Check::Grouped)
                .hint(n("Hold Ctrl and press 1 while the squad is selected.")),
            Step::new(
                n("The enemy has an outpost at the marked place. Fight is attack-move: units walk there and take on whatever they meet on the way. Press {fight}, then click near the outpost."),
                Check::Fight,
            )
            .marker(Marker::Outpost)
            .aim(Aim::Order("fight"))
            .hint(n("Select the squad first (press 1). Then press {fight}, or the Fight button among the orders, and click on the ground by the marked ring.")),
            Step::new(
                n("Destroy every building of the outpost. Its laser tower hits hard: keep the squad together, and queue reinforcements at the lab if you need them."),
                Check::OutpostGone,
            )
            .marker(Marker::Outpost)
            .hint(n("Losing the fight? Pull the squad back, queue more Peewees at the lab and go in again with all of them together.")),
            Step::new(n("The outpost is gone. The wrecks it left are worth metal: builders reclaim them (next lesson). Lesson complete!"), Check::Continue),
        ],
    },
    Lesson {
        title: n("ORDERS LIKE A VETERAN"),
        summary: n("Map views, area orders, building blocks and build queues."),
        teaches: &[n("Map overlays"), n("Area orders"), n("Building blocks"), n("Build queues")],
        minutes: 6,
        steps: &[
            Step::new(
                n("The map shows what the ground hides. Press {overlay_metal} for the metal map ({overlay_path} shows where units can go, {overlay_height} heights, {overlay_sight} sight and radar); the same key hides it again. The MAP menu at the top right does the same."),
                Check::Overlay(Overlay::Metal),
            )
            .aim(Aim::MapMenu)
            .classic(Classic::Text(n("The map shows what the ground hides. Press {overlay_metal} for the metal map ({overlay_path} shows where units can go, {overlay_height} heights, {overlay_sight} sight and radar); the same key hides it again.")))
            .hint(n("Press {overlay_metal}: the metal deposits light up all over the map.")),
            Step::new(
                n("Select the Commander, press {area_mex} and drag a circle over the marked deposits: every metal spot inside gets an extractor, nearest first. A plain click takes the metal around the cursor."),
                Check::AreaMex,
            )
            .marker(Marker::Metal)
            .aim(Aim::Order("area_mex"))
            .hint(n("With the Commander selected, press {area_mex}, then press on the ground over the deposits and drag outwards: the circle grows as you drag.")),
            Step::new(n("Many orders work on areas. Press {reclaim} (reclaim) and drag a circle over the rocks near your base: the Commander turns every one into metal."), Check::AreaReclaim)
                .aim(Aim::Order("reclaim"))
                .hint(n("Rocks and wrecks are worth metal. With the Commander selected, press {reclaim}, then press on the ground and drag the circle over a few rocks.")),
            Step::new(n("Hold Alt while you drag out a building to fill a whole block with it. Pick the Solar Collector, hold Alt and drag out a block of four or more."), Check::Grid)
                .aim(Aim::Build("ARMSOLAR"))
                .hint(n("Pick the Solar Collector, then hold Alt, press on the ground and drag: a block of outlines follows the pointer. Let go to place them all.")),
            Step::new(
                n("The Commander's queued buildings show under BUILD QUEUE, beside its picture at the bottom of the screen. Click one to build it next (right-click drops it). A factory's queue works the same way."),
                Check::QueueEdit,
            )
            .aim(Aim::Queue)
            .classic(Classic::Skip)
            .hint(n("The BUILD QUEUE shows while the Commander alone is selected and has buildings waiting. Click any but the first: that one is being built.")),
            Step::new(n("Area orders, building blocks and queues save you hundreds of clicks in a battle. Lesson complete!"), Check::Continue),
        ],
    },
    Lesson {
        title: n("HOLD THE LINE"),
        summary: n("Defences, radar, repairs, and the metal in wrecks."),
        teaches: &[n("Defences"), n("Radar"), n("Repairs"), n("Reclaiming wrecks")],
        minutes: 6,
        steps: &[
            Step::new(
                n("Raiders will come from the marked direction. Build a Light Laser Tower (on the DEFENCE tab) in the marked ring: towers shoot whatever comes in range."),
                Check::OwnsFinished(Kind::Tower),
            )
            .marker(Marker::Approach)
            .aim(Aim::Build("ARMLLT"))
            .classic(Classic::Text(n("Raiders will come from the marked direction. Build a Light Laser Tower from the build menu in the marked ring: towers shoot whatever comes in range.")))
            .hint(n("The Light Laser Tower is a small gun on a post. Pick it, put it in the ring and wait for it to finish.")),
            Step::new(
                n("Build a Radar Tower (on the SENSORS tab) near your base. Radar shows enemies as blips on the map and the minimap long before your units see them."),
                Check::OwnsFinished(Kind::Radar),
            )
            .aim(Aim::Build("ARMRAD"))
            .classic(Classic::Text(n("Build a Radar Tower from the build menu near your base. Radar shows enemies as blips on the map and the minimap long before your units see them.")))
            .hint(n("The Radar Tower is a tall mast. Pick it, place it near the Commander and wait for it to finish.")),
            Step::new(n("Here they come: raiders on your radar. Keep the Commander by the tower and let them come to you: together they hold the line."), Check::RaidBeaten)
                .marker(Marker::Raid)
                .enter(Enter::Raid)
                .hint(n("Do not chase them: the tower only fights what comes in range. Keep the Commander beside it; while it fights, the Commander can mend it ({repair}, then click the tower).")),
            Step::new(n("The fight left a building damaged. Select the Commander, press {repair} and click it: builders mend what is damaged."), Check::Repaired)
                .aim(Aim::Order("repair"))
                .marker(Marker::Patient)
                .enter(Enter::Damage)
                .hint(n("With the Commander selected, press {repair}, then click the damaged building. Its health bar fills as it is mended.")),
            Step::new(n("The raiders left wrecks, and wrecks are worth metal. Select the Commander, press {reclaim} and click a wreck, or drag a circle over them all."), Check::Reclaimed)
                .marker(Marker::Wrecks)
                .aim(Aim::Order("reclaim"))
                .hint(n("Wrecks lie where the raiders fell. With the Commander selected, press {reclaim} and click one: the metal bar rises as it is taken apart.")),
            Step::new(n("Towers hold ground while your army is elsewhere, radar sees what is coming, and builders mend and recycle. Lesson complete!"), Check::Continue),
        ],
    },
    Lesson {
        title: n("BUILDERS AND FACTORIES"),
        summary: n("Builders that help, factories that keep building, patrols and standing orders."),
        teaches: &[n("Construction units"), n("Helping a factory"), n("Rally points"), n("Standing orders")],
        minutes: 7,
        steps: &[
            Step::new(n("One builder is slow. Select the Kbot Lab and build a Construction Kbot: it builds what the Commander builds, and more besides."), Check::OwnsFinished(Kind::Builder))
                .aim(Aim::Produce("ARMCK"))
                .hint(n("Click the lab on the map to select it; the Construction Kbot is in its build menu. Wait for it to walk out.")),
            Step::new(n("Builders help each other. Select the Construction Kbot, press {guard} and click the Kbot Lab: it lends the lab its build power, and the lab builds faster."), Check::Guarding)
                .aim(Aim::Order("guard"))
                .hint(n("With the Construction Kbot selected, press {guard}, then click the lab. It stays by the lab and helps with everything it makes.")),
            Step::new(n("Factories send their units where you say. Select the Kbot Lab. {order_click} on open ground nearby: new units gather there."), Check::Rally)
                .aim(Aim::Factory)
                .hint(n("Select the lab first. With it selected, {order_click} on the ground where its units should gather.")),
            Step::new(n("Turn on Repeat for the lab: whatever it finishes goes back on the end of its queue, so it never stops building."), Check::Repeat)
                .aim(Aim::Repeat)
                .classic(Classic::Text(n("Turn on REPEAT BUILD for the lab: whatever it finishes goes back on the end of its queue, so it never stops building.")))
                .hint(n("Select the lab, then turn Repeat on: the pointer shows where it is.")),
            Step::new(n("Builders on patrol mend and reclaim whatever they pass. Select the Construction Kbot, press {patrol} and click on the far side of your base."), Check::Patrol)
                .aim(Aim::Order("patrol"))
                .hint(n("With the Construction Kbot selected, press {patrol}, then click on the ground: it walks back and forth between there and where it stood.")),
            Step::new(
                n("Units keep standing orders. Select your Peewees and set their fire order to Hold fire with the button beside the orders: they will not shoot until you say."),
                Check::HoldFire,
            )
            .aim(Aim::FireState)
            .classic(Classic::Text(n("Units keep standing orders. Select your Peewees and set their FIRE ORDERS to Hold fire: they will not shoot until you say.")))
            .hint(n("Select the Peewees, then click the fire order button until it shows HOLD FIRE: each click is the next order.")),
            Step::new(n("You are ready for a skirmish: start against an Easy computer player, then Normal. {help} lists every key. Good hunting, Commander!"), Check::Continue),
        ],
    },
];

/// The game a lesson plays: ARM against a CORE player that does nothing.
pub fn setup(lesson: usize) -> GameSetup {
    let mut s = GameSetup::duel(MAP);
    s.players[1].controller = Controller::Human;
    s.seed = 0x7E4C_0000 + lesson as u64;
    // Lesson 2 takes the enemy Commander away and leaves its outpost standing, in plain
    // sight.
    // (Lesson 4's raid does not end the lesson with a lost Commander either.)
    s.rules.commander_ends_game = !matches!(lesson, 1 | 3);
    s.rules.fog_of_war = lesson != 1;
    s
}

/// What the tutorial asks the app to do.
pub enum TutorialAction {
    None,
    /// Start this lesson (the next one).
    Start(usize),
    Menu,
    /// The skirmish setup, against an Easy computer player (after the last lesson).
    Skirmish,
}

/// Seconds on a step without it done before its hint shows.
const HINT_AFTER: f32 = 20.0;

pub struct Tutorial {
    pub lesson: usize,
    step: usize,
    /// Camera position, zoom and orientation when the camera step began.
    camera_at: Option<Camera>,
    /// The enemy outpost (lesson 2).
    outpost: Vec<UnitId>,
    /// Lesson 4: the raiders, where they were last seen together, where they come in.
    raid: Vec<UnitId>,
    raid_last: Option<Vec3>,
    approach: Option<Vec3>,
    /// The building to mend (lesson 4).
    patient: Option<UnitId>,
    /// Seconds the current step has been done for (a moment before the next one).
    done_for: f32,
    /// Seconds on the current step without it done (its hint shows after a while).
    stuck_for: f32,
    /// The step's tick-off, or the lesson's end, has been heard.
    chimed: bool,
    /// Said aloud: the step's words (as it began) and whether its hint too.
    spoken: Option<(usize, bool)>,
    /// The panel folded down to its heading (a new step opens it again).
    folded: bool,
    /// LEAVE was clicked once (a second click confirms).
    leave_armed: bool,
    /// TA's classic interface is in use: its words, and the steps it cannot show passed over.
    classic: bool,
    pub finished: bool,
}

/// Where the guide points: a place on the interface, one of our units, a place on the map, or
/// the step's marked place (its ring shows it: pointed at only from the edge, out of view).
#[derive(Clone, Copy)]
enum Pointer {
    Ui(Rect),
    Unit(UnitId),
    Place(Vec3),
    Mark(Vec3),
}

impl Tutorial {
    /// Sets the scene for a lesson (its game has just started) and begins it.
    pub fn start(lesson: usize, game: &mut Game) -> Tutorial {
        let me = game.local_player;
        game.tutorial = true;
        game.used = Used::default();
        let mut outpost = Vec::new();
        let sim = &mut game.sim;
        let base = sim.units().find(|u| u.owner == me && sim.unit_type(u).def.is_commander).map(|u| u.pos);
        if let (1, Some(base)) = (lesson, base) {
            let id = |sim: &ta_sim::Sim, n: &str| sim.types.id(n);
            let theirs: Vec<UnitId> = sim.units().filter(|u| u.owner != me).map(|u| u.id).collect();
            for u in theirs {
                sim.remove_unit(u);
            }
            // The outpost: towards the middle of the map, 1200 to 1800 px out.
            let (w, h) = sim.world.size_px();
            let dir = (FxVec3::new(w / 2, Fx::ZERO, h / 2) - base).xz();
            let len = dir.length_xz().max(Fx::ONE);
            let reach = (len * 2 / 3).max(Fx::from_int(1200)).min(Fx::from_int(1800));
            let ahead = |d: i64, side: i64| {
                let (x, z) = (dir.x * Fx::from_int(d) / len, dir.z * Fx::from_int(d) / len);
                let (sx, sz) = (-dir.z * Fx::from_int(side) / len, dir.x * Fx::from_int(side) / len);
                base + FxVec3::new(x + sx, Fx::ZERO, z + sz)
            };
            let at = ahead(0, 0) + FxVec3::new(dir.x * reach / len, Fx::ZERO, dir.z * reach / len);
            // Our squad on the outpost's side of the Commander, and the lab that "built" it
            // behind.
            if let Some(lab) = id(sim, "ARMLAB") && let Some(site) = sim.find_build_site(lab, ahead(-170, 0), 12) {
                sim.spawn_unit(lab, me, site, 0, true);
            }
            if let Some(pw) = id(sim, "ARMPW") {
                let heading = ta_sim::fixed::heading_of(dir);
                for i in 0..8 {
                    let p = ahead(130 + (i / 4) * 50, -90 + (i % 4) * 60);
                    sim.spawn_unit(pw, me, p, heading, true);
                }
            }
            log::info!("tutorial: enemy outpost at ({}, {})", at.x.to_int(), at.z.to_int());
            for (name, dx, dz) in [("CORLLT", 0, 0), ("CORSOLAR", 120, -70), ("CORSOLAR", 120, 70), ("CORRAD", -100, 80), ("CORWIN", -100, -80)] {
                let Some(t) = id(sim, name) else { continue };
                if let Some(site) = sim.find_build_site(t, at + FxVec3::from_ints(dx, 0, dz), 10) {
                    outpost.extend(sim.spawn_unit(t, 1, site, 0, true));
                }
            }
        }
        // Lesson 4: the raiders' way in, towards the middle of the map (the tower's ring).
        let mut approach = None;
        if let (3, Some(base)) = (lesson, base) {
            let (w, h) = sim.world.size_px();
            let dir = (FxVec3::new(w / 2, Fx::ZERO, h / 2) - base).xz();
            let len = dir.length_xz().max(Fx::ONE);
            let at = base + FxVec3::new(dir.x * Fx::from_int(340) / len, Fx::ZERO, dir.z * Fx::from_int(340) / len);
            approach = Some(Vec3::from_array(at.to_f32()));
        }
        // Lesson 5: a base to run: the Kbot Lab, two solar collectors and three Peewees.
        if let (4, Some(base)) = (lesson, base) {
            for (name, dx, dz) in [("ARMLAB", -270, 110), ("ARMSOLAR", 170, 30), ("ARMSOLAR", 170, 120)] {
                let Some(t) = sim.types.id(name) else { continue };
                if let Some(site) = sim.find_build_site(t, base + FxVec3::from_ints(dx, 0, dz), 12) {
                    sim.spawn_unit(t, me, site, 0, true);
                }
            }
            if let Some(pw) = sim.types.id("ARMPW") {
                for i in 0..3 {
                    sim.spawn_unit(pw, me, base + FxVec3::from_ints(-100 + i * 50, 0, 130), 0, true);
                }
            }
        }
        let t = Tutorial {
            lesson,
            step: 0,
            camera_at: None,
            outpost,
            raid: Vec::new(),
            raid_last: None,
            approach,
            patient: None,
            done_for: 0.0,
            stuck_for: 0.0,
            chimed: false,
            spoken: None,
            folded: false,
            leave_armed: false,
            classic: false,
            finished: false,
        };
        t.update_markers(game);
        t
    }

    /// Goes straight to a step (captures: `TA_TUTORIAL_STEP`).
    pub fn jump_to(&mut self, step: usize, game: &mut Game) {
        self.step = step.min(self.lesson().steps.len() - 1);
        self.done_for = 0.0;
        self.stuck_for = 0.0;
        self.enter_step(game);
        self.update_markers(game);
    }

    /// What the driver does as a step begins: the raiders set out, a building to mend.
    fn enter_step(&mut self, game: &mut Game) {
        match self.current().enter {
            Enter::None => {}
            Enter::Raid => self.send_raid(game),
            Enter::Damage => {
                let me = game.local_player;
                let sim = &mut game.sim;
                let buildings: Vec<(UnitId, Fx, Fx, bool)> = sim
                    .units()
                    .filter(|u| u.owner == me && !u.dead && u.is_complete() && sim.unit_type(u).locomotion == Locomotion::Static)
                    .map(|u| (u.id, u.health, sim.unit_type(u).max_health, is_kind(sim.unit_type(u), Kind::Tower)))
                    .collect();
                let share = |b: &(UnitId, Fx, Fx, bool)| b.1.to_f32() / b.2.to_f32().max(1.0);
                // The most damaged; none damaged, the tower (else another) takes a beating.
                self.patient = match buildings.iter().filter(|b| share(b) < 0.99).min_by(|a, b| share(a).total_cmp(&share(b))) {
                    Some(b) => Some(b.0),
                    None => buildings.iter().find(|b| b.3).or(buildings.first()).map(|b| {
                        sim.set_health(b.0, b.2 * 2 / 5);
                        b.0
                    }),
                };
            }
        }
    }

    /// Lesson 4's raiders: four AK kbots well out along the way in, attack-moving on it.
    fn send_raid(&mut self, game: &mut Game) {
        let me = game.local_player;
        let Some(approach) = self.approach else { return };
        let sim = &mut game.sim;
        let Some(base) = sim.units().find(|u| u.owner == me && sim.unit_type(u).def.is_commander).map(|u| Vec3::from_array(u.pos.to_f32())) else { return };
        let Some(ak) = sim.types.id("CORAK") else { return };
        let dir = (approach - base).normalize_or_zero();
        let side = Vec3::new(-dir.z, 0.0, dir.x);
        let (w, h) = sim.world.size_px();
        let (w, h) = (w.to_f32() - 64.0, h.to_f32() - 64.0);
        let start = base + dir * 1500.0;
        let heading = ta_sim::fixed::heading_of(FxVec3::new(Fx::from_f32(-dir.x), Fx::ZERO, Fx::from_f32(-dir.z)));
        for i in 0..4 {
            let at = start + side * (-60.0 + 40.0 * i as f32);
            let (x, z) = (Fx::from_f32(at.x.clamp(64.0, w)), Fx::from_f32(at.z.clamp(64.0, h)));
            self.raid.extend(sim.spawn_unit(ak, 1, FxVec3::new(x, sim.world.height_at(x, z), z), heading, true));
        }
        // To the way in, then on to the base (wherever the tower went up).
        let ground = |at: Vec3| FxVec3::new(Fx::from_f32(at.x), Fx::ZERO, Fx::from_f32(at.z));
        game.direct(ta_sim::Command::Order { player: 1, units: self.raid.clone(), order: ta_sim::Order::Fight(ground(approach)), queue: false });
        game.direct(ta_sim::Command::Order { player: 1, units: self.raid.clone(), order: ta_sim::Order::Fight(ground(base)), queue: true });
    }

    /// The middle of the raiders still standing (lesson 4).
    fn raid_centre(&self, game: &Game) -> Option<Vec3> {
        let sim = &game.sim;
        let pts: Vec<Vec3> = self.raid.iter().filter_map(|&id| sim.unit(id)).filter(|u| !u.dead).map(|u| Vec3::from_array(u.pos.to_f32())).collect();
        (!pts.is_empty()).then(|| pts.iter().sum::<Vec3>() / pts.len() as f32)
    }

    /// The words to say aloud now, each once: the step's as it begins (its classic words with
    /// TA's own interface), then its hint when that shows; nothing once the lesson is over.
    pub fn take_line(&mut self) -> Option<&'static str> {
        if self.finished {
            return None;
        }
        let step = self.current();
        if self.spoken.is_none_or(|(s, _)| s != self.step) {
            self.spoken = Some((self.step, false));
            return Some(match step.classic {
                Classic::Text(classic) if self.classic => classic,
                _ => step.text,
            });
        }
        // (As `draw` shows it.)
        let hint_shown = self.done_for == 0.0 && !self.folded && !step.hint.is_empty() && self.stuck_for >= HINT_AFTER;
        if hint_shown && self.spoken == Some((self.step, false)) {
            self.spoken = Some((self.step, true));
            return Some(step.hint);
        }
        None
    }

    /// As if the player had been stuck on the step a while (captures: its hint).
    pub fn stuck(&mut self) {
        self.stuck_for = HINT_AFTER;
    }

    fn lesson(&self) -> &'static Lesson {
        &LESSONS[self.lesson.min(LESSONS.len() - 1)]
    }

    fn current(&self) -> &'static Step {
        let steps = self.lesson().steps;
        &steps[self.step.min(steps.len() - 1)]
    }

    /// Whether the current step is done.
    fn met(&mut self, game: Progress<'_>, camera: &Camera) -> bool {
        let me = game.me;
        let sim = game.sim;
        let mine = || sim.units().filter(move |u| u.owner == me && !u.dead);
        let selected = || game.selection.iter().filter_map(|&id| sim.unit(id)).filter(|u| u.owner == me && !u.dead);
        match self.current().check {
            Check::CommanderSelected => selected().any(|u| sim.unit_type(u).def.is_commander),
            Check::CameraMoved => {
                let start = self.camera_at.get_or_insert_with(|| camera.clone());
                let turned = |a: f32, b: f32| (a - b).sin().atan2((a - b).cos()).abs() > 0.15;
                camera.focus.distance(start.focus) > 240.0 || (camera.zoom / start.zoom.max(0.01) - 1.0).abs() > 0.3
                    || turned(camera.yaw, start.yaw) || turned(camera.pitch, start.pitch)
            }
            Check::Owns(k) => mine().any(|u| is_kind(sim.unit_type(u), k)),
            Check::OwnsFinished(k) => mine().any(|u| u.is_complete() && is_kind(sim.unit_type(u), k)),
            Check::CombatUnits(n) => mine().filter(|u| is_combat(sim, u)).count() >= n,
            Check::CombatSelected(n) => selected().filter(|u| is_combat(sim, u)).count() >= n,
            Check::Grouped => game.groups.get(1).is_some_and(|g| {
                g.iter().filter_map(|&id| sim.unit(id)).filter(|u| u.owner == me && !u.dead && is_combat(sim, u)).count() >= 4
            }),
            Check::Fight => game.used.fight,
            Check::Overlay(o) => game.overlay == Some(o),
            Check::AreaMex => game.used.area_mex,
            Check::AreaReclaim => game.used.area_reclaim,
            Check::QueueEdit => game.used.queue_edit,
            Check::Grid => game.used.grid,
            Check::OutpostGone => self.outpost.iter().all(|&id| sim.unit(id).is_none_or(|u| u.dead)),
            Check::RaidBeaten => !self.raid.is_empty() && self.raid.iter().all(|&id| sim.unit(id).is_none_or(|u| u.dead)),
            Check::Repaired => self.patient.and_then(|id| sim.unit(id)).is_some_and(|u| !u.dead && u.health >= sim.unit_type(u).max_health),
            Check::Reclaimed => game.used.reclaim,
            Check::Guarding => game.used.guard,
            Check::Rally => game.used.rally,
            Check::Repeat => game.used.repeat,
            Check::Patrol => game.used.patrol,
            Check::HoldFire => game.used.hold_fire,
            Check::Continue => false,
        }
    }

    /// How far along the step is, when it counts something.
    fn count(&self, game: &Game) -> Option<String> {
        let me = game.local_player;
        let sim = &game.sim;
        let mine = || sim.units().filter(move |u| u.owner == me && !u.dead);
        match self.current().check {
            Check::CombatUnits(n) => {
                let ready = mine().filter(|u| is_combat(sim, u)).count().min(n);
                Some(tf("Ready: {done} of {total}", &[("done", &ready), ("total", &n)]))
            }
            Check::CombatSelected(n) => {
                let chosen = game.selection.iter().filter_map(|&id| sim.unit(id)).filter(|u| u.owner == me && !u.dead && is_combat(sim, u)).count().min(n);
                Some(tf("Selected: {done} of {total}", &[("done", &chosen), ("total", &n)]))
            }
            // The one going up (the furthest along).
            Check::OwnsFinished(k) => {
                let built = mine().filter(|u| !u.is_complete() && is_kind(sim.unit_type(u), k)).map(|u| u.build_progress.to_f32()).reduce(f32::max)?;
                Some(tf("Built: {percent}%", &[("percent", &((built.clamp(0.0, 1.0) * 100.0) as u32))]))
            }
            Check::OutpostGone => {
                let left = self.outpost.iter().filter(|&&id| sim.unit(id).is_some_and(|u| !u.dead)).count();
                Some(tf("Still standing: {left} of {total}", &[("left", &left), ("total", &self.outpost.len())]))
            }
            Check::RaidBeaten if !self.raid.is_empty() => {
                let left = self.raid.iter().filter(|&&id| sim.unit(id).is_some_and(|u| !u.dead)).count();
                Some(tf("Raiders left: {left} of {total}", &[("left", &left), ("total", &self.raid.len())]))
            }
            Check::Repaired => {
                let u = self.patient.and_then(|id| sim.unit(id)).filter(|u| !u.dead)?;
                let health = (u.health.to_f32() / sim.unit_type(u).max_health.to_f32().max(1.0) * 100.0) as u32;
                Some(tf("Health: {percent}%", &[("percent", &health.min(100))]))
            }
            _ => None,
        }
    }

    /// Moves on once the step is done (after a moment, so the player sees it tick off).
    pub fn update(&mut self, game: &mut Game, camera: &Camera, dt: f32) {
        if self.finished {
            return;
        }
        // A repair target can be reclaimed or lost while the player is reading. Pick another
        // surviving building (or pass the step when none remain), rather than wait forever.
        if self.done_for == 0.0 && matches!(self.current().check, Check::Repaired)
            && self.patient.is_none_or(|id| game.sim.unit(id).is_none_or(|u| u.dead || u.owner != game.local_player))
        {
            self.enter_step(game);
            self.update_markers(game);
        }
        // (Passed over: what the classic interface cannot show; mending with nothing to mend,
        // every building lost.)
        let nothing_to_mend = matches!(self.current().check, Check::Repaired) && self.patient.is_none();
        if self.classic && matches!(self.current().classic, Classic::Skip) || nothing_to_mend {
            self.advance(game);
            return;
        }
        if self.done_for > 0.0 || self.met(Progress::from(&*game), camera) {
            self.done_for += dt.max(0.001);
            if self.done_for > 1.2 {
                self.advance(game);
            }
        } else {
            self.stuck_for += dt;
        }
        // (The raiders seen together last: their wrecks lie about there.)
        if let Some(c) = self.raid_centre(game) {
            self.raid_last = Some(c);
        }
        if matches!(self.current().marker, Marker::Outpost | Marker::Raid) {
            self.update_markers(game);
        }
    }

    fn advance(&mut self, game: &mut Game) {
        self.done_for = 0.0;
        self.stuck_for = 0.0;
        self.chimed = false;
        self.folded = false;
        self.camera_at = None;
        // (What the player has used stays noted: doing a step early counts.)
        if self.step + 1 >= self.lesson().steps.len() {
            self.finished = true;
        } else {
            self.step += 1;
            self.enter_step(game);
        }
        self.update_markers(game);
    }

    /// The middle of what is left of the enemy outpost (lesson 2).
    pub fn outpost_centre(&self, game: &Game) -> Option<Vec3> {
        let sim = &game.sim;
        let pts: Vec<Vec3> = self.outpost.iter().filter_map(|&id| sim.unit(id)).filter(|u| !u.dead).map(|u| Vec3::from_array(u.pos.to_f32())).collect();
        (!pts.is_empty()).then(|| pts.iter().sum::<Vec3>() / pts.len() as f32)
    }

    /// Rings on the ground (and the minimap) where the step points.
    fn update_markers(&self, game: &mut Game) {
        game.tutorial_markers.clear();
        if self.finished {
            return;
        }
        let sim = &game.sim;
        match self.current().marker {
            Marker::None => {}
            Marker::Metal => {
                let me = game.local_player;
                let Some(base) = sim.units().find(|u| u.owner == me && sim.unit_type(u).def.is_commander).map(|u| u.pos.to_f32()) else { return };
                let w = &sim.world;
                let top = w.metal.iter().copied().max().unwrap_or(0);
                if top == 0 {
                    return;
                }
                // The nearest rich cell (a third of the map's best or more) not built on.
                let (bx, bz) = (base[0] / 16.0, base[2] / 16.0);
                let nearest = (0..w.height as i32)
                    .flat_map(|z| (0..w.width as i32).map(move |x| (x, z)))
                    .filter(|&(x, z)| w.metal[(z * w.width as i32 + x) as usize] >= top.div_ceil(3) && matches!(w.occupant(x, z), ta_sim::world::Occupant::Free))
                    .min_by(|a, b| {
                        let d = |&(x, z): &(i32, i32)| (x as f32 - bx).powi(2) + (z as f32 - bz).powi(2);
                        d(a).total_cmp(&d(b))
                    });
                if let Some((x, z)) = nearest {
                    game.tutorial_markers.push((x as f32 * 16.0 + 8.0, z as f32 * 16.0 + 8.0, 48.0));
                }
            }
            Marker::Outpost => {
                if let Some(c) = self.outpost_centre(game) {
                    game.tutorial_markers.push((c.x, c.z, 200.0));
                }
            }
            Marker::Approach => {
                if let Some(a) = self.approach {
                    game.tutorial_markers.push((a.x, a.z, 80.0));
                }
            }
            Marker::Raid => {
                if let Some(c) = self.raid_centre(game) {
                    game.tutorial_markers.push((c.x, c.z, 140.0));
                }
            }
            Marker::Wrecks => {
                if let Some(c) = self.raid_last {
                    game.tutorial_markers.push((c.x, c.z, 160.0));
                }
            }
            Marker::Patient => {
                if let Some(u) = self.patient.and_then(|id| sim.unit(id)).filter(|u| !u.dead) {
                    let p = u.pos.to_f32();
                    game.tutorial_markers.push((p[0], p[2], 64.0));
                }
            }
        }
    }

    /// Where the guide points now: the next click for the step, else (placing the building,
    /// giving the order) its marked place; nothing once it is done.
    fn pointer(&self, game: &Game, anchors: &Anchors) -> Option<Pointer> {
        if self.finished || self.done_for > 0.0 {
            return None;
        }
        self.next_click(game, anchors).or_else(|| {
            let &(x, z, _) = game.tutorial_markers.first()?;
            Some(Pointer::Mark(Vec3::new(x, game.sim.world.height_at(Fx::from_f32(x), Fx::from_f32(z)).to_f32(), z)))
        })
    }

    /// The next click for the step: a unit to select, a place on the interface.
    fn next_click(&self, game: &Game, anchors: &Anchors) -> Option<Pointer> {
        let me = game.local_player;
        let sim = &game.sim;
        let selected = |id: UnitId| game.selection.contains(&id);
        let ours = || sim.units().filter(move |u| u.owner == me && !u.dead);
        let commander = || ours().find(|u| sim.unit_type(u).def.is_commander);
        let builder = || {
            ours().find(|u| {
                let t = sim.unit_type(u);
                u.is_complete() && t.is_builder && !t.def.is_commander && !t.is_factory && t.locomotion != Locomotion::Static
            })
        };
        let factory = || ours().find(|u| u.is_complete() && sim.unit_type(u).is_factory);
        // Our combat units not selected yet, where they stand together.
        let squad = || {
            let at: Vec<Vec3> = ours().filter(|u| is_combat(sim, u) && !selected(u.id)).map(|u| Vec3::from_array(u.pos.to_f32())).collect();
            (!at.is_empty()).then(|| Pointer::Place(at.iter().sum::<Vec3>() / at.len() as f32))
        };
        let step = self.current();
        match step.aim {
            Aim::None => None,
            Aim::Commander => commander().filter(|c| !selected(c.id)).map(|c| Pointer::Unit(c.id)),
            Aim::Squad => squad(),
            Aim::Build(name) => {
                let t = sim.types.id(name)?;
                if game.placement.as_ref().is_some_and(|p| p.type_id == t) {
                    return None;
                }
                // (One going up: the step waits for it.)
                if matches!(step.check, Check::OwnsFinished(_)) && ours().any(|u| u.type_id == t) {
                    return None;
                }
                let can = |u: &ta_sim::Unit| sim.unit_type(u).build_options.contains(&t);
                if game.selection.iter().filter_map(|&id| sim.unit(id)).any(can)
                    && let Some(r) = anchors.towards_build(t)
                {
                    return Some(Pointer::Ui(r));
                }
                // Select a builder of it first (alone, for its build menu).
                commander().filter(|c| can(c)).or_else(|| ours().find(|u| u.is_complete() && can(u))).map(|u| Pointer::Unit(u.id))
            }
            Aim::Produce(name) => {
                let t = sim.types.id(name)?;
                let factories: Vec<&ta_sim::Unit> = ours().filter(|u| u.is_complete() && sim.unit_type(u).is_factory && sim.unit_type(u).build_options.contains(&t)).collect();
                // Enough ready and on the way: nothing more to click.
                if let Check::CombatUnits(n) = step.check {
                    let coming: usize = factories.iter().map(|f| f.factory_queue.iter().filter(|&&q| q == t).count()).sum();
                    if ours().filter(|u| is_combat(sim, u)).count() + coming >= n {
                        return None;
                    }
                }
                // One here or on its way: the step waits for it.
                if matches!(step.check, Check::OwnsFinished(_)) && (ours().any(|u| u.type_id == t) || factories.iter().any(|f| f.factory_queue.contains(&t))) {
                    return None;
                }
                let factory = factories.first()?;
                if selected(factory.id)
                    && let Some(r) = anchors.towards_build(t)
                {
                    return Some(Pointer::Ui(r));
                }
                Some(Pointer::Unit(factory.id))
            }
            Aim::Order(action) => {
                let mode = mode_of(action)?;
                if game.command_mode == Some(mode) {
                    return None;
                }
                if game.mode_available(mode) {
                    return anchors.order_button(action).map(Pointer::Ui);
                }
                // Who gives it: the squad fights, a builder guards and patrols, the Commander
                // mends, reclaims and builds extractors.
                match mode {
                    CommandMode::Fight => squad(),
                    CommandMode::Guard | CommandMode::Patrol => builder().map(|u| Pointer::Unit(u.id)),
                    _ => commander().map(|c| Pointer::Unit(c.id)),
                }
            }
            // (The factory alone: its own panel, and a click on the ground its rally point.)
            Aim::Factory => factory().filter(|f| !selected(f.id) || game.selection.len() > 1).map(|f| Pointer::Unit(f.id)),
            Aim::Repeat => {
                let f = factory()?;
                if !selected(f.id) || game.selection.len() > 1 {
                    return Some(Pointer::Unit(f.id));
                }
                anchors.repeat.map(Pointer::Ui)
            }
            Aim::FireState => {
                if game.selection.iter().filter_map(|&id| sim.unit(id)).any(|u| is_combat(sim, u)) {
                    return anchors.order_button("fire_state").map(Pointer::Ui);
                }
                squad()
            }
            Aim::MapMenu => anchors.map_menu.map(Pointer::Ui),
            Aim::Queue => {
                let c = commander()?;
                if !selected(c.id) || game.selection.len() > 1 {
                    return Some(Pointer::Unit(c.id));
                }
                anchors.queue.map(Pointer::Ui)
            }
        }
    }

    /// The instructions panel in the top left of `view` (the battlefield clear of the HUD),
    /// and the guide's pointer.
    pub fn draw(&mut self, ctx: &egui::Context, game: &mut Game, camera: &mut Camera, view: Rect, anchors: &Anchors) -> TutorialAction {
        let mut action = TutorialAction::None;
        self.classic = anchors.classic;
        // Heard: each step as it ticks off, and the lesson's end.
        if !self.chimed && (self.finished || self.done_for > 0.0) {
            self.chimed = true;
            ta_ui::sound::play(ctx, if self.finished { ta_ui::sound::UiSound::Achievement } else { ta_ui::sound::UiSound::Done });
        }
        let pointer = self.pointer(game, anchors);
        if let Some(p) = pointer {
            draw_pointer(ctx, game, camera, view, anchors.side_panel, p);
        }
        // SHOW ME: the marked place, else what the guide points at on the map.
        let look = if self.finished || self.done_for > 0.0 {
            None
        } else if let Some(&(x, z, _)) = game.tutorial_markers.first() {
            Some(Vec3::new(x, game.sim.world.height_at(Fx::from_f32(x), Fx::from_f32(z)).to_f32(), z))
        } else {
            pointer.and_then(|p| ground_of(game, p))
        };
        let lesson = self.lesson();
        let total = lesson.steps.len();
        egui::Area::new(egui::Id::new("tutorial")).fixed_pos(view.min + vec2(16.0, 16.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            let background = ui.painter().add(egui::Shape::Noop);
            let inner = egui::Frame::new().inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
                ui.set_width(420.0);
                ui.horizontal(|ui| {
                    ui.label(w::caption(&tf("LESSON {n} · {title}", &[("n", &(self.lesson + 1)), ("title", &t(lesson.title))])).color(AMBER));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !self.finished {
                            let (glyph, tip) = if self.folded { (w::Glyph::Down, n("Show the instructions")) } else { (w::Glyph::Up, n("Fold the instructions away")) };
                            if w::icon_button(ui, glyph, true).on_hover_text(t(tip)).clicked() {
                                self.folded = !self.folded;
                            }
                        }
                        ui.label(w::caption(&format!("{} / {total}", (self.step + 1).min(total))));
                    });
                });
                ui.add_space(2.0);
                // A segment per step.
                let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), egui::Sense::hover());
                let seg = bar.width() / total as f32;
                for k in 0..total {
                    let r = egui::Rect::from_min_size(egui::pos2(bar.min.x + seg * k as f32 + 1.0, bar.min.y), vec2(seg - 2.0, 4.0));
                    let done = self.finished || k < self.step || k == self.step && self.done_for > 0.0;
                    let color = if done {
                        AMBER
                    } else if k == self.step {
                        AMBER_DIM
                    } else {
                        LINE_HI
                    };
                    ui.painter().rect_filled(r, 1.0, color);
                }
                if self.finished {
                    ui.add_space(10.0);
                    ui.label(RichText::new(t("LESSON COMPLETE")).font(display(24.0)).color(AMBER).extra_letter_spacing(ta_ui::i18n::tracking(2.0)));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if self.lesson + 1 < LESSONS.len() && ui.add(ActionButton::primary("NEXT LESSON")).clicked() {
                            action = TutorialAction::Start(self.lesson + 1);
                        }
                        // (The last lesson: on to a first battle of one's own.)
                        if self.lesson + 1 >= LESSONS.len()
                            && ui.add(ActionButton::primary("SET UP A SKIRMISH")).on_hover_text(t("Choose a map: your opponents start as Easy computer players")).clicked()
                        {
                            action = TutorialAction::Skirmish;
                        }
                        if ui.add(ActionButton::secondary("MAIN MENU")).clicked() {
                            action = TutorialAction::Menu;
                        }
                    });
                    return;
                }
                if self.folded {
                    return;
                }
                ui.add_space(10.0);
                let step = self.current();
                let done = self.done_for > 0.0;
                let words = match step.classic {
                    Classic::Text(classic) if self.classic => classic,
                    _ => step.text,
                };
                let text = crate::keys::fill(words, game.keys.config(), game.order_button);
                ui.add(egui::Label::new(RichText::new(text).font(body(14.5)).color(if done { GREEN } else { TEXT_BRIGHT })).wrap());
                if !done && let Some(count) = self.count(game) {
                    ui.add_space(6.0);
                    ui.label(RichText::new(count).font(strong(12.5)).color(AMBER).extra_letter_spacing(ta_ui::i18n::tracking(0.6)));
                }
                if !done && !step.hint.is_empty() && self.stuck_for >= HINT_AFTER {
                    ui.add_space(8.0);
                    ui.label(w::caption(n("HINT")).color(AMBER_DIM));
                    let hint = crate::keys::fill(step.hint, game.keys.config(), game.order_button);
                    ui.add(egui::Label::new(RichText::new(hint).font(body(13.5)).color(TEXT)).wrap());
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if matches!(step.check, Check::Continue) {
                        if ui.add(ActionButton::primary("CONTINUE")).clicked() {
                            self.advance(game);
                        }
                    } else if done {
                        ui.label(RichText::new(t("DONE")).font(strong(13.0)).color(GREEN).extra_letter_spacing(ta_ui::i18n::tracking(1.5)));
                    } else if ui.add(ActionButton::secondary("SKIP")).on_hover_text(ta_ui::i18n::t("Go on without doing this step")).clicked() {
                        self.advance(game);
                    }
                    if let Some(at) = look
                        && ui.add(ActionButton::secondary("SHOW ME")).on_hover_text(t("Bring the camera to it")).clicked()
                    {
                        camera.focus = at;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = if self.leave_armed { ta_ui::i18n::n("REALLY LEAVE?") } else { ta_ui::i18n::n("LEAVE") };
                        if ui.add(ActionButton::secondary(label)).on_hover_text(ta_ui::i18n::t("Back to the main menu: this lesson starts over next time")).clicked() {
                            if self.leave_armed {
                                action = TutorialAction::Menu;
                            }
                            self.leave_armed = !self.leave_armed;
                        }
                    });
                });
            });
            let r = inner.response.rect;
            let shadow = egui::epaint::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(150) };
            ui.painter().set(
                background,
                egui::Shape::Vec(vec![
                    shadow.as_shape(r, CornerRadius::same(2)).into(),
                    egui::Shape::rect_filled(r.expand(1.0), CornerRadius::same(3), ta_ui::metal::STEEL_EDGE),
                    ta_ui::metal::riveted_plate(ui.ctx(), r, PANEL, 0.0),
                    egui::Shape::rect_filled(egui::Rect::from_min_size(r.min + vec2(14.0, 0.0), vec2(72.0, 2.0)), CornerRadius::ZERO, AMBER),
                ]),
            );
        });
        action
    }
}

/// The clip of a line said in the language in use, else in English (`tools/voice/tutorial.py`
/// makes them): named by its English words with the keys and mouse buttons filled in, so a line
/// reworded, or one with a key the player rebound, has none and is not heard.
pub fn voice_path(vfs: &ta_vfs::Vfs, words: &str, keys: &ta_config::Keys, order_button: ta_config::OrderButton) -> Option<String> {
    let id = clip_id(words, keys, order_button);
    let language = ta_ui::i18n::language();
    let language = if language.is_empty() { "en" } else { language.as_str() };
    [language, "en"].into_iter().map(|code| format!("voice/tutorial/{code}/{id}.ogg")).find(|path| vfs.exists(path))
}

/// A line's clip's name: the start of the SHA-256 of its English words with the keys filled in.
fn clip_id(words: &str, keys: &ta_config::Keys, order_button: ta_config::OrderButton) -> String {
    use sha2::Digest;
    let english = crate::keys::fill_english(words, keys, order_button);
    sha2::Sha256::digest(english.as_bytes()).iter().take(6).map(|b| format!("{b:02x}")).collect()
}

/// Whether a unit type is of a kind a step asks for.
fn is_kind(t: &ta_sim::UnitType, k: Kind) -> bool {
    match k {
        Kind::Extractor => t.extracts_metal > Fx::ZERO,
        Kind::Generator => t.locomotion == Locomotion::Static && (t.energy_make > t.energy_use || t.wind_generator > Fx::ZERO || t.tidal_generator > Fx::ZERO),
        Kind::Factory => t.is_factory,
        Kind::Tower => t.locomotion == Locomotion::Static && !t.is_factory && t.weapons.iter().any(|w| w.is_some()),
        Kind::Radar => t.locomotion == Locomotion::Static && t.def.radar_distance > 0.0,
        Kind::Builder => t.is_builder && !t.def.is_commander && !t.is_factory && t.locomotion != Locomotion::Static,
    }
}

/// A built unit that moves and fights (not a builder or the Commander).
fn is_combat(sim: &ta_sim::Sim, u: &ta_sim::Unit) -> bool {
    let t = sim.unit_type(u);
    u.is_complete() && t.max_speed > Fx::ZERO && !t.is_builder && !t.def.is_commander && t.weapons.iter().any(|w| w.is_some())
}

/// The order a step's button gives.
fn mode_of(action: &str) -> Option<CommandMode> {
    Some(match action {
        "fight" => CommandMode::Fight,
        "reclaim" => CommandMode::Reclaim,
        "area_mex" => CommandMode::AreaMex,
        "guard" => CommandMode::Guard,
        "patrol" => CommandMode::Patrol,
        "repair" => CommandMode::Repair,
        _ => return None,
    })
}

/// Where on the ground the guide points (for SHOW ME): the unit's place, or the place.
fn ground_of(game: &Game, pointer: Pointer) -> Option<Vec3> {
    match pointer {
        Pointer::Ui(_) => None,
        Pointer::Unit(id) => game.scene.unit_position(id, game.alpha()).or_else(|| game.sim.unit(id).map(|u| Vec3::from_array(u.pos.to_f32()))),
        Pointer::Place(at) | Pointer::Mark(at) => Some(at),
    }
}

/// The guide's pointer: a ring breathing round a place on the interface with an arrow at it,
/// or an arrow over a unit or place on the map (at the edge of the view, towards it, when it
/// is out of sight).
fn draw_pointer(ctx: &egui::Context, game: &Game, camera: &Camera, view: Rect, side_panel: Option<Rect>, pointer: Pointer) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("tutorial_guide")));
    let still = ta_ui::motion::reduced(ctx);
    let time = ctx.input(|i| i.time) as f32;
    let wave = if still { 1.0 } else { 0.5 + 0.5 * (time * 4.5).sin() };
    let bob = 5.0 * wave;
    if !still {
        ctx.request_repaint();
    }
    if let Pointer::Ui(rect) = pointer {
        painter.rect_stroke(rect.expand(2.0 + 2.0 * wave), CornerRadius::same(3), Stroke::new(2.5, AMBER.gamma_multiply(0.55 + 0.45 * wave)), StrokeKind::Outside);
        // The arrow from just past TA's side panel for what is on it, else from above, else from
        // below (the top of the screen).
        let screen = ctx.content_rect();
        if let Some(panel) = side_panel.filter(|p| p.contains(rect.center())) {
            arrow(&painter, pos2(panel.right() + 4.0 + bob, rect.center().y), vec2(-1.0, 0.0));
        } else if rect.top() - screen.top() > 50.0 {
            arrow(&painter, pos2(rect.center().x, rect.top() - 6.0 - bob), vec2(0.0, 1.0));
        } else {
            arrow(&painter, pos2(rect.center().x, rect.bottom() + 6.0 + bob), vec2(0.0, -1.0));
        }
        return;
    }
    let Some(ground) = ground_of(game, pointer) else { return };
    // Over a unit: over its top.
    let height = match pointer {
        Pointer::Unit(id) => game.sim.unit(id).map_or(0.0, |u| game.sim.unit_type(u).model.height.to_f32()),
        _ => 0.0,
    };
    let at = ground + Vec3::Y * (height + 8.0);
    let ppp = ctx.pixels_per_point();
    let clip = camera.view_proj() * at.extend(1.0);
    let mut ndc = glam::Vec2::new(clip.x, clip.y) / clip.w;
    if clip.w < 0.0 {
        ndc = -ndc;
    }
    let s = pos2((ndc.x + 1.0) * 0.5 * camera.viewport.x / ppp, (1.0 - ndc.y) * 0.5 * camera.viewport.y / ppp);
    let inset = view.shrink(40.0);
    if clip.w > 0.0 && inset.contains(s) {
        // (A marked place in view: its ring is enough.)
        if !matches!(pointer, Pointer::Mark(_)) {
            arrow(&painter, s - vec2(0.0, 4.0 + bob), vec2(0.0, 1.0));
        }
    } else {
        // At the view's edge, on the line from its middle towards it, pointing out.
        let c = inset.center();
        let d = (s - c).normalized();
        let d = if d.is_finite() && d.length() > 0.5 { d } else { vec2(0.0, -1.0) };
        let reach = (inset.width() / 2.0 / d.x.abs().max(1e-3)).min(inset.height() / 2.0 / d.y.abs().max(1e-3));
        arrow(&painter, c + d * (reach + bob), d);
    }
}

/// An arrow with its tip at `tip`, pointing along `dir` (a unit vector): brass, outlined.
fn arrow(painter: &egui::Painter, tip: Pos2, dir: egui::Vec2) {
    let side = vec2(-dir.y, dir.x);
    let base = tip - dir * 16.0;
    let outline = Stroke::new(1.5, Color32::from_black_alpha(210));
    let shaft = vec![base + side * 4.5, base - side * 4.5, base - side * 4.5 - dir * 14.0, base + side * 4.5 - dir * 14.0];
    painter.add(egui::Shape::convex_polygon(shaft, AMBER, outline));
    painter.add(egui::Shape::convex_polygon(vec![tip, base + side * 11.0, base - side * 11.0], AMBER, outline));
}

/// What the lessons page asks the menu to do.
pub enum PageAction {
    None,
    Back,
    Start(usize),
}

/// Between the lessons page's cards.
const CARD_GAP: f32 = 6.0;

/// The menu page listing the lessons (`done`: the ones completed).
pub fn page(ctx: &egui::Context, done: &[u8]) -> PageAction {
    let mut action = PageAction::None;
    let subtitle = tf("{done} OF {total} LESSONS COMPLETE", &[("done", &done.iter().filter(|&&d| (d as usize) < LESSONS.len()).count()), ("total", &LESSONS.len())]);
    // The lessons scroll under the words that stay when they do not all fit (as many whole
    // cards from the top as there is room for), by the page's head and where each card ends
    // below the first's top, as last drawn. (Each card's own: in a language whose lessons' lines
    // wrap, they differ, and a stride taken from the first two left room for most of another,
    // which the scroll area then left blank.)
    let sizes = egui::Id::new("tutorial_page_sizes");
    let (head, bottoms) = ctx.data(|d| d.get_temp::<(f32, Vec<f32>)>(sizes)).unwrap_or_else(|| (220.0, (1..=LESSONS.len()).map(|k| k as f32 * 110.0 - CARD_GAP).collect()));
    let free = ctx.content_rect().height() - 16.0 - head;
    let (scroll, room) = lessons_room(&bottoms, free, ctx.content_rect().height() - 16.0);
    let mut list = 0.0;
    let mut cards: Vec<egui::Rect> = Vec::new();
    let response = panel::page(ctx, "tutorial", Panel::new("TUTORIAL").subtitle(&subtitle).width(760.0).exit(Exit::Back), |ui| {
        ui.label(RichText::new(t("Short guided games on Painted Desert against an enemy that stays out of your way. Follow the panel at the top left; each step ticks off as you do it.")).color(TEXT));
        ui.add_space(6.0);
        // The first lesson not done yet is the one to play.
        let next = (0..LESSONS.len()).find(|i| !done.contains(&(*i as u8)));
        let mut lessons = |ui: &mut egui::Ui| {
            ui.spacing_mut().item_spacing.y = CARD_GAP;
            for (i, lesson) in LESSONS.iter().enumerate() {
                let complete = done.contains(&(i as u8));
                // (The cards come in one after another as the page does.)
                let drawn = panel::list_entry(ui, i, |ui| egui::Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, if next == Some(i) { AMBER_DIM } else { LINE }))
                    .corner_radius(CornerRadius::same(2))
                    .inner_margin(Margin { left: 14, right: 12, top: 6, bottom: 6 })
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            // (The words wrap short of the button.)
                            let words = (ui.available_width() - 176.0).max(220.0);
                            w::figures(ui, i + 1, 2, true, display(30.0), if complete { STEEL } else { AMBER });
                            ui.add_space(8.0);
                            ui.vertical(|ui| {
                                ui.set_width(words);
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(t(lesson.title)).font(strong(16.0)).color(TEXT_BRIGHT).extra_letter_spacing(ta_ui::i18n::tracking(1.0)));
                                    if complete {
                                        w::tag(ui, "DONE", GREEN);
                                    }
                                });
                                let facts = tf("{summary} · {n} steps · about {minutes} min", &[("summary", &t(lesson.summary)), ("n", &lesson.steps.len()), ("minutes", &lesson.minutes)]);
                                // (Its figures whole where it wraps: no "min" alone on a line.)
                                let facts = w::keep_facts_whole(&facts, t(lesson.summary));
                                ui.add(egui::Label::new(RichText::new(facts).font(body(13.0)).color(STEEL)).wrap());
                                // What it teaches.
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                                    for skill in lesson.teaches {
                                        w::tag(ui, skill, if complete { STEEL } else { AMBER_DIM });
                                    }
                                });
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let button = if next == Some(i) { ActionButton::primary("START") } else { ActionButton::secondary(if complete { "PLAY AGAIN" } else { "START" }) };
                                if ui.add(button.size(118.0, 34.0)).clicked() {
                                    action = PageAction::Start(i);
                                }
                            });
                        });
                    }));
                cards.push(drawn.response.rect);
            }
        };
        if scroll {
            // (Asked for outright: an area offers only the room it took last frame.)
            let area = egui::ScrollArea::vertical().id_salt("tutorial_lessons").max_height(room).min_scrolled_height(room).auto_shrink([false, true]);
            list = ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Plate, lessons).inner_rect.height();
        } else {
            lessons(ui);
            list = cards.last().zip(cards.first()).map_or(0.0, |(last, first)| last.bottom() - first.top());
        }
    });
    let measured = match cards.first() {
        Some(first) if cards.len() == LESSONS.len() => cards.iter().map(|r| r.bottom() - first.top()).collect(),
        _ => bottoms,
    };
    ctx.data_mut(|d| d.insert_temp(sizes, (response.rect.height() - list, measured)));
    if response.exit {
        action = PageAction::Back;
    }
    action
}

/// Whether the lesson cards (each ending `bottoms` below the first's top) scroll in `free` room,
/// on a page that has `page` room in all, and the view's height when they do: down to the last
/// whole card from the top that fits (the first at least). A page a little too tall for its room
/// is shown a little smaller rather (`panel::page` fits it to the screen): a list cut to one card
/// fewer, to scroll, for the want of a few points was the worse of the two.
fn lessons_room(bottoms: &[f32], free: f32, page: f32) -> (bool, f32) {
    const SHRINK: f32 = 0.04;
    let total = bottoms.last().copied().unwrap_or(0.0);
    let room = bottoms.iter().copied().filter(|&b| b <= free).fold(bottoms.first().copied().unwrap_or(0.0), f32::max);
    (total > free + page * SHRINK, room)
}

#[cfg(test)]
#[path = "tutorial/tests.rs"]
mod progression_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// The lessons scroll only when they do not all fit; then the view holds the whole cards from
    /// the top that fit, however their heights differ (and the first however little room).
    #[test]
    fn the_lessons_view_holds_whole_cards() {
        // Cards 100 high, 10 apart: each ends 110 further down.
        let even = [100.0, 210.0, 320.0, 430.0, 540.0];
        assert_eq!(lessons_room(&even, 540.0, 800.0), (false, 540.0), "all of them fit");
        assert!(!lessons_room(&even, 530.0, 800.0).0, "a few points short: the page is shown a little smaller");
        assert_eq!(lessons_room(&even, 480.0, 800.0), (true, 430.0), "far short: four whole cards, and a scroll");
        // A tall card among short ones: the view ends above it, not across it.
        let mixed = [100.0, 210.0, 380.0, 490.0, 600.0];
        assert_eq!(lessons_room(&mixed, 300.0, 500.0), (true, 210.0));
        assert_eq!(lessons_room(&mixed, 50.0, 250.0), (true, 100.0), "the first card at least");
    }

    /// Steps name keys as `{action}`, so they follow the player's bindings: their words, their
    /// classic interface's and their hints alike.
    #[test]
    fn steps_name_bound_keys() {
        let mut keys = ta_config::Keys::default();
        for step in LESSONS.iter().flat_map(|l| l.steps) {
            let classic = match step.classic {
                Classic::Text(c) => c,
                _ => "",
            };
            for words in [step.text, step.hint, classic] {
                assert!(!crate::keys::fill(words, &keys, ta_config::OrderButton::Right).contains('{'), "{words}");
            }
        }
        keys.set("fight", "Ctrl+G");
        let step = LESSONS.iter().flat_map(|l| l.steps).find(|s| s.text.contains("{fight}")).unwrap();
        assert!(crate::keys::fill(step.text, &keys, ta_config::OrderButton::Right).contains("Press Ctrl+G, then click"));
    }

    /// What the guide points at is named as the game knows it, and only steps that can be done
    /// on the classic interface stay there.
    #[test]
    fn the_guide_names_what_exists() {
        for step in LESSONS.iter().flat_map(|l| l.steps) {
            match step.aim {
                Aim::Build(name) | Aim::Produce(name) => assert!(name.starts_with("ARM") && name == name.to_ascii_uppercase(), "{name}"),
                Aim::Order(action) => {
                    assert!(mode_of(action).is_some(), "{action}");
                    assert!(!ta_config::Keys::default().get(action).is_empty(), "{action} is a key binding's action");
                }
                _ => {}
            }
            // A step passed over on the classic interface is never the one that ends a lesson.
            assert!(!(matches!(step.classic, Classic::Skip) && matches!(step.check, Check::Continue)));
        }
        // Each lesson says what it teaches and how long it takes.
        assert!(LESSONS.iter().all(|l| !l.teaches.is_empty() && l.minutes > 0));
    }

    /// A lesson as `start` leaves it, without a battle.
    pub(super) fn lesson(lesson: usize) -> Tutorial {
        Tutorial {
            lesson,
            step: 0,
            camera_at: None,
            outpost: Vec::new(),
            raid: Vec::new(),
            raid_last: None,
            approach: None,
            patient: None,
            done_for: 0.0,
            stuck_for: 0.0,
            chimed: false,
            spoken: None,
            folded: false,
            leave_armed: false,
            classic: false,
            finished: false,
        }
    }

    /// Each step is said once as it begins, its hint once when that shows, and nothing after
    /// the lesson's end; with TA's own interface, the classic words.
    #[test]
    fn the_lessons_words_are_said_once_each() {
        let mut t = lesson(0);
        let steps = LESSONS[0].steps;
        assert_eq!(t.take_line(), Some(steps[0].text));
        assert_eq!(t.take_line(), None);
        t.stuck_for = HINT_AFTER - 1.0;
        assert_eq!(t.take_line(), None, "the hint not shown yet");
        t.stuck_for = HINT_AFTER;
        assert_eq!(t.take_line(), Some(steps[0].hint));
        assert_eq!(t.take_line(), None);
        // The next step: its words at once (the hint's time starts again).
        t.step = 1;
        t.stuck_for = 0.0;
        assert_eq!(t.take_line(), Some(steps[1].text));
        // Folded away, the hint is not shown, so not said.
        t.folded = true;
        t.stuck_for = HINT_AFTER;
        assert_eq!(t.take_line(), None);
        // The extractor's step on TA's own interface: its classic words.
        t.classic = true;
        t.step = 2;
        let Classic::Text(classic) = steps[2].classic else { panic!("the extractor's step has classic words") };
        assert_eq!(t.take_line(), Some(classic));
        t.finished = true;
        t.step = 3;
        assert_eq!(t.take_line(), None);
    }

    /// A language voiced at all is voiced throughout (never a lesson half in its words and half
    /// in English's), for either mouse button; and only languages the game has are voiced.
    #[test]
    fn every_voiced_language_has_every_line() {
        let Some(builtin) = ta_vfs::builtin_dir() else { return };
        let Ok(dirs) = std::fs::read_dir(builtin.join("voice").join("tutorial")) else { return };
        let keys = ta_config::Keys::default();
        let lang = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lang");
        for dir in dirs.flatten().filter(|d| d.path().is_dir()) {
            let code = dir.file_name().to_string_lossy().into_owned();
            assert!(code == "en" || lang.join(format!("{code}.toml")).exists(), "voice/tutorial/{code}: no such language");
            let mut missing = Vec::new();
            for step in LESSONS.iter().flat_map(|l| l.steps) {
                let classic = match step.classic {
                    Classic::Text(c) => c,
                    _ => "",
                };
                for words in [step.text, step.hint, classic].into_iter().filter(|w| !w.is_empty()) {
                    for button in [ta_config::OrderButton::Right, ta_config::OrderButton::Left] {
                        let id = clip_id(words, &keys, button);
                        if !dir.path().join(format!("{id}.ogg")).exists() {
                            missing.push(crate::keys::fill_english(words, &keys, button));
                        }
                    }
                }
            }
            missing.dedup();
            assert!(missing.is_empty(), "voice/tutorial/{code} lacks: {missing:#?}
(python tools/voice/tutorial.py --only {code} makes them)");
        }
    }

    /// Every clip ends with its last word. Of the 150 ms kept after it, the word's own release
    /// fades in the first 30 ms; the last 120 ms are quiet, at least 35 dB under the clip's
    /// loudest 50 ms (no breath, mumble, click or hiss after the words, as the voice often adds),
    /// and the last 10 ms 50 dB under (faded out, never cut off mid-sound).
    /// `tools/voice/tutorial.py` cuts each sentence to its words as it makes it, and measures a
    /// clip's end the same way (`end_test`).
    #[test]
    fn every_clip_ends_with_its_words() {
        let Some(builtin) = ta_vfs::builtin_dir() else { return };
        let Ok(dirs) = std::fs::read_dir(builtin.join("voice").join("tutorial")) else { return };
        // The RMS level (dB under full scale) of each `win`-sample window, `hop` samples apart.
        fn loudest(x: &[f32], win: usize, hop: usize) -> f32 {
            (0..=x.len().saturating_sub(win))
                .step_by(hop.max(1))
                .map(|i| {
                    let w = &x[i..(i + win).min(x.len())];
                    let mean = w.iter().map(|&s| s as f64 * s as f64).sum::<f64>() / w.len().max(1) as f64;
                    (10.0 * (mean + 1e-18).log10()) as f32
                })
                .fold(f32::MIN, f32::max)
        }
        let dirs: Vec<_> = dirs.flatten().map(|d| d.path()).filter(|p| p.is_dir()).collect();
        let (clips, loud) = std::thread::scope(|scope| {
            let jobs: Vec<_> = dirs
                .iter()
                .map(|dir| {
                    scope.spawn(move || {
                        let (mut clips, mut loud) = (0, Vec::new());
                        for path in std::fs::read_dir(dir).unwrap().flatten().map(|f| f.path()) {
                            if path.extension().is_none_or(|e| e != "ogg") {
                                continue;
                            }
                            let decoded = ta_audio::decode::decode(&std::fs::read(&path).unwrap()).unwrap();
                            let (rate, channels) = (decoded.sound.sample_rate() as usize, decoded.sound.channels() as usize);
                            let x: Vec<f32> = decoded.sound.samples().chunks_exact(channels).map(|f| f[0]).collect();
                            let speech = loudest(&x, rate / 20, rate / 100);
                            let tail = loudest(&x[x.len().saturating_sub(rate * 12 / 100)..], rate / 100, rate / 100) - speech;
                            let last = loudest(&x[x.len().saturating_sub(rate / 100)..], rate / 100, rate / 100) - speech;
                            if tail > -35.0 || last > -50.0 {
                                loud.push(format!("{}: its last 120 ms {tail:.0} dB, its last 10 ms {last:.0} dB under its loudest", path.display()));
                            }
                            clips += 1;
                        }
                        (clips, loud)
                    })
                })
                .collect();
            jobs.into_iter().map(|j| j.join().unwrap()).fold((0, Vec::new()), |(n, mut all), (c, l)| {
                all.extend(l);
                (n + c, all)
            })
        });
        assert!(clips > 0 || dirs.is_empty());
        assert!(loud.is_empty(), "{} of {clips} clips do not end with their words:\n{}\n(python tools/voice/tutorial.py --clean --ids <id> cuts them)", loud.len(), loud.join("\n"));
    }

    /// Every line the tutorial says has its English clip, named as `tools/voice/tutorial.py`
    /// names it (made again with it when a line's words change), for either mouse button.
    #[test]
    fn every_line_has_its_english_clip() {
        let mut vfs = ta_vfs::Vfs::new();
        let Some(builtin) = vfs.mount_builtin() else { return };
        let keys = ta_config::Keys::default();
        let mut missing = Vec::new();
        for step in LESSONS.iter().flat_map(|l| l.steps) {
            let classic = match step.classic {
                Classic::Text(c) => c,
                _ => "",
            };
            for words in [step.text, step.hint, classic].into_iter().filter(|w| !w.is_empty()) {
                for button in [ta_config::OrderButton::Right, ta_config::OrderButton::Left] {
                    if voice_path(&vfs, words, &keys, button).is_none() {
                        missing.push(crate::keys::fill_english(words, &keys, button));
                    }
                }
            }
        }
        missing.dedup();
        assert!(missing.is_empty(), "no clip in {}/voice/tutorial/en for: {missing:#?}\n(python tools/voice/tutorial.py makes them)", builtin.display());
        // As the sound engine hears them: Ogg Vorbis, the first step's some seconds long.
        let first = voice_path(&vfs, LESSONS[0].steps[0].text, &keys, ta_config::OrderButton::Right).unwrap();
        let decoded = ta_audio::decode::decode(&vfs.read(&first).unwrap()).unwrap();
        assert_eq!(decoded.format, ta_audio::decode::SourceFormat::OggVorbis);
        let seconds = decoded.sound.frames() as f32 / decoded.sound.sample_rate() as f32;
        assert!((4.0..30.0).contains(&seconds), "{first}: {seconds} s");
        // A rebound key: that line has none (not heard saying the wrong key).
        let mut rebound = keys.clone();
        rebound.set("fight", "Ctrl+G");
        let fight = LESSONS.iter().flat_map(|l| l.steps).find(|s| s.text.contains("{fight}")).unwrap();
        assert!(voice_path(&vfs, fight.text, &rebound, ta_config::OrderButton::Right).is_none());
    }
}
