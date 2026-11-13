//! The radial context menu. Right-click one of your units (or hold the button on it, or press B
//! on a pad, which clicks as the right button does) and a ring of what it can do opens round the
//! pointer: a tower's attack, stop, perspective, firing mode and self-destruct; a factory's
//! build queue, repeat and stop; a builder's build, repair, reclaim and guard; a tank's move,
//! fight, patrol, stop and its fire and move orders. A choice that has more to it (the firing
//! modes, a building's kinds, the confirmation of a self-destruct) opens a ring of its own, and
//! what does not fit a ring goes on a page after it.
//!
//! This module is the menu itself: what it offers (`entries`), where on the ring a pointer or
//! a stick picks (`pick`, `sector`, `step`) and the state of an open menu (`Radial`). What a
//! choice does to the game is `Game::radial_choose`; the interface draws the menu and gives it
//! the pointer (`hud/radial.rs`).

use glam::Vec2;
use ta_ai::Duty;
use ta_sim::{Locomotion, Unit, UnitId};
use ta_ui::i18n::{n, t, tf};

use crate::game::{CommandMode, Game, UnitInfo};
use crate::input::Input;
use crate::hud::Purpose;
use crate::hud_icons::Glyph;
use crate::orders::{self, Kind};

/// The ring's inner and outer radius (points at the smallest window; the interface scales them),
/// and how far past the ring the pointer may stray before it counts as away from the menu.
pub const INNER: f32 = 52.0;
pub const OUTER: f32 = 132.0;
pub const REACH: f32 = 90.0;
/// How far past the outer edge the menu's parts (the rim, the keys) go at most: the room it keeps
/// from the edges of the battlefield (points at the smallest window).
pub const MARGIN: f32 = 26.0;
/// The share of the inner radius at the middle where nothing is picked.
const DEAD: f32 = 0.7;
/// Most places on a ring, "more" included.
pub const MOST: usize = 9;
/// How far the left stick goes before it picks.
pub const STICK: f32 = 0.5;

/// One of the menu's rings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    /// The unit's own ring: the first page of its choices.
    Main,
    /// The page of its choices after the one before (1 and on).
    More(usize),
    /// The fire orders: hold fire, return fire, fire at will.
    Fire,
    /// The move orders: hold position, maneuver, roam.
    Movement,
    /// A factory's units, a page of them.
    Units(usize),
    /// A constructor's kinds of building, by purpose.
    Purposes,
    /// The buildings of a purpose (its index in `Purpose::ALL`), a page of them.
    Buildings(usize, usize),
    /// Asked again before a self-destruct.
    Confirm,
    /// What a constructor does by itself: its duties.
    Duties,
}

/// What a choice does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Id {
    /// An order of the plate's (stop, a targeted order, Perspective).
    Order(Kind),
    /// Another ring.
    Open(Menu),
    /// A fire order set (0 hold fire, 1 return fire, 2 fire at will).
    Fire(u8),
    /// A move order set (0 hold position, 1 maneuver, 2 roam).
    Movement(u8),
    Cloak,
    Power,
    Repeat,
    /// A unit queued in the factory.
    Produce(usize),
    /// A building to place.
    Place(usize),
    /// The self-destruct itself (after the confirmation, or to cancel the count).
    Destruct,
    /// The information page.
    Info,
    /// A spectator's camera follows it.
    Follow,
    /// A duty for the constructors (none: back under the player's command).
    Duty(Option<Duty>),
}

/// How a choice is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Normal,
    /// What cannot be taken back: rusted.
    Danger,
}

/// One choice of a ring.
#[derive(Clone, Debug)]
pub struct Entry {
    pub id: Id,
    /// Its glyph, or the picture of a unit type (a building, a unit to queue).
    pub glyph: Option<Glyph>,
    pub picture: Option<usize>,
    pub label: String,
    /// A second line: what it costs.
    pub detail: Option<String>,
    /// The action its key is bound to, and the key as the player has it.
    pub action: Option<&'static str>,
    pub key: Option<String>,
    /// What it does, for the line under the ring.
    pub hint: String,
    pub tone: Tone,
    /// On, or the setting the unit has now.
    pub lit: bool,
    /// How many are queued (a factory's unit).
    pub count: usize,
    /// It leads to another ring.
    pub opens: bool,
    /// The menu stays open after it (queueing units one after another).
    pub stays: bool,
    /// What a unit or building costs, metal and energy (drawn as two figures on a badge; `detail`
    /// is the same in words).
    pub cost: Option<(f32, f32)>,
    /// It would do nothing now (`hint` says why): drawn dim, and it makes nothing happen.
    pub disabled: bool,
}

impl Entry {
    pub(crate) fn new(id: Id, glyph: Glyph, label: impl Into<String>, hint: impl Into<String>) -> Entry {
        Entry { id, glyph: Some(glyph), picture: None, label: label.into(), detail: None, action: None, key: None, hint: hint.into(), tone: Tone::Normal, lit: false, count: 0, opens: false, stays: false, cost: None, disabled: false }
    }

    /// It cannot be made now, for `why` (shown where its hint is).
    fn disabled(mut self, why: impl Into<String>) -> Entry {
        self.disabled = true;
        self.hint = why.into();
        self
    }

    /// Its key: the action it is bound to, as the player has it.
    fn keyed(mut self, game: &Game, action: &'static str) -> Entry {
        let key = game.keys.text(action).to_string();
        self.action = Some(action);
        self.key = (!key.is_empty()).then_some(key);
        self
    }

    fn opening(mut self) -> Entry {
        self.opens = true;
        self
    }

    fn lit(mut self, on: bool) -> Entry {
        self.lit = on;
        self
    }

    fn danger(mut self) -> Entry {
        self.tone = Tone::Danger;
        self
    }
}

/// The menu, open: which unit's, where (window pixels), how it was opened, which rings are
/// open and what is under the pointer or the stick.
#[derive(Clone, Debug)]
pub struct Radial {
    pub unit: UnitId,
    pub centre: Vec2,
    /// Opened by holding the button: letting go over a choice makes it.
    pub held: bool,
    /// The rings opened, the last shown.
    pub rings: Vec<Menu>,
    pub hover: Option<usize>,
    /// The choice under the hover came from a stick or the d-pad, which the pointer does not move.
    pub from_pad: bool,
    /// The interface's clock at the first frame that drew it (none until then), and the choice that
    /// closed it, when it has been told to close (the interface lets go of it at once: what it
    /// showed goes on folding away by itself).
    pub born: Option<f64>,
    pub leaving: Option<Leaving>,
    /// The choice made last, and the one refused last (what would do nothing): the interface reads
    /// each once, to press or to shake that plate.
    pub picked: Option<usize>,
    pub refused: Option<usize>,
    /// The window pixels the pointer was moved to for the menu to fit the screen, for the app to
    /// carry the cursor there.
    pub warp: Option<Vec2>,
    /// For each ring opened over the first, which choice of the ring before opened it (and how
    /// many it had), so that the middle can show where it came from and going back can return there.
    pub places: Vec<Place>,
}

/// Where a ring was opened from: the choice that did, and how many choices the ring it was on had.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub index: usize,
    pub of: usize,
}

/// A menu on its way out.
#[derive(Clone, Copy, Debug)]
pub struct Leaving {
    /// The choice that was made, to flare as it goes.
    pub chosen: Option<usize>,
}

impl Radial {
    pub fn open(unit: UnitId, centre: Vec2, held: bool) -> Radial {
        Radial { unit, centre, held, rings: vec![Menu::Main], hover: None, from_pad: false, born: None, leaving: None, picked: None, refused: None, warp: None, places: Vec::new() }
    }

    pub fn ring(&self) -> Menu {
        self.rings.last().copied().unwrap_or(Menu::Main)
    }

    /// Opens `menu` on top.
    pub fn push(&mut self, menu: Menu) {
        self.rings.push(menu);
        self.hover = None;
    }

    /// Opens `menu` as the choice of the ring on top that leads to it would (for captures): the
    /// place it came from is kept, as a click would keep it.
    pub fn enter(&mut self, game: &Game, menu: Menu) {
        let entries = entries(game, self);
        if let Some(index) = entries.iter().position(|e| e.id == Id::Open(menu)) {
            self.places.truncate(self.rings.len() - 1);
            self.places.push(Place { index, of: entries.len() });
        }
        self.push(menu);
    }

    /// Back to the ring before; false when it was the first.
    pub fn back(&mut self) -> bool {
        if self.rings.len() > 1 {
            self.rings.pop();
            self.places.truncate(self.rings.len() - 1);
            self.hover = None;
            true
        } else {
            false
        }
    }

    /// Goes to `menu` in the place of the last (the next page).
    pub fn turn_to(&mut self, menu: Menu) {
        if let Some(last) = self.rings.last_mut() {
            *last = menu;
        }
        self.hover = None;
    }

    /// Closes it, flaring the choice that was made.
    pub fn close(&mut self, chosen: Option<usize>) {
        if self.leaving.is_none() {
            self.leaving = Some(Leaving { chosen });
        }
    }
}

/// Which of `count` choices lies in the direction of `offset` from the middle (window pixels,
/// the ring drawn at `scale`), none while the pointer is at the middle or well away from the ring.
pub fn pick(offset: Vec2, count: usize, scale: f32) -> Option<usize> {
    let d = offset.length();
    if count == 0 || d < INNER * scale * DEAD || d > (OUTER + REACH) * scale {
        return None;
    }
    Some(sector(offset, count))
}

/// The sector of `count` that `offset` points into: the first is centred straight up, the rest
/// clockwise.
pub fn sector(offset: Vec2, count: usize) -> usize {
    let angle = offset.x.atan2(-offset.y).rem_euclid(std::f32::consts::TAU);
    let width = std::f32::consts::TAU / count.max(1) as f32;
    (((angle + width / 2.0) / width) as usize) % count.max(1)
}

/// The angle of the middle of sector `i` of `count`, clockwise from straight up.
pub fn angle(i: usize, count: usize) -> f32 {
    i as f32 * std::f32::consts::TAU / count.max(1) as f32
}

/// The next choice from `from` in `delta` steps round the ring (from none: the first, or with
/// a step back the last).
pub fn step(from: Option<usize>, count: usize, delta: i32) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match from {
        Some(i) => (i as i32 + delta).rem_euclid(count as i32) as usize,
        None if delta < 0 => count - 1,
        None => 0,
    })
}

/// A mouse button let go over the open menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

/// What letting go of a button does to the open menu (`Hud::radial_menu` carries it out).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    /// Makes this choice.
    Pick(usize),
    /// A queued unit: one fewer.
    Fewer(usize),
    /// Back to the ring before, or closed from the first.
    Back,
    /// Closed.
    Close,
    /// Opened by holding and let go at the middle: it stays up, for a click.
    Settle,
    Nothing,
}

/// What letting go of `button` does while choice `lit` is lit (by the pointer, or by a stick when
/// `from_pad`, which the pointer's place does not matter to), with the pointer at the `middle` of the
/// ring or `far` from it, in a menu that was `held` open (the button that opened it was still down) or
/// not; `fewer` when `lit` is a unit in a factory's queue ring.
///
/// A left click makes the choice, goes back at the middle and closes away from the menu. A right
/// click goes back (then closes), takes a queued unit off, or, in a menu held open, makes the choice
/// it is let go over: a flick towards it and a release.
pub fn released(button: Button, lit: Option<usize>, held: bool, from_pad: bool, middle: bool, far: bool, fewer: bool) -> Release {
    match button {
        Button::Left => match lit {
            Some(h) if !middle || from_pad => Release::Pick(h),
            _ if far => Release::Close,
            _ if middle => Release::Back,
            _ => Release::Nothing,
        },
        Button::Right => match lit {
            Some(h) if held && !middle => Release::Pick(h),
            // Held open and let go at the middle: it stays, for a click.
            _ if held && !far => Release::Settle,
            Some(h) if fewer && !middle => Release::Fewer(h),
            _ => Release::Back,
        },
    }
}

/// What the selection can be asked, as the ring sees it.
struct Facts {
    armed: bool,
    mobile: bool,
    /// Builds, repairs or reclaims (a nano turret included).
    works: bool,
    cloak: bool,
    on_off: bool,
}

impl Facts {
    fn of(game: &Game) -> Facts {
        let any = |f: &dyn Fn(&ta_sim::UnitType) -> bool| game.selection.iter().any(|&id| game.sim.unit(id).is_some_and(|u| f(game.sim.unit_type(u))));
        Facts {
            armed: any(&|t| t.weapons.iter().any(|w| w.is_some())),
            mobile: any(&|t| t.locomotion != Locomotion::Static),
            works: any(&|t| t.is_builder),
            cloak: any(&|t| t.def.cloak_cost > 0.0),
            on_off: any(&|t| t.def.on_offable),
        }
    }
}

/// What a unit is to its menu: which list of choices the ring is made from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    /// A factory (or a lab, a shipyard, a plant): its queue.
    Factory,
    /// A tower or another building that does not build units.
    Defence,
    /// A constructor that moves: a vehicle, a bot, a ship, a plane, the Commander.
    Builder,
    /// Everything else that moves.
    Fighter,
}

impl Role {
    fn of(ty: &ta_sim::UnitType) -> Role {
        // (A turret that only builds, repairs and reclaims is a building to its menu, not a factory.)
        if ty.is_factory && !ty.build_options.is_empty() {
            Role::Factory
        } else if ty.locomotion == Locomotion::Static {
            Role::Defence
        } else if ty.is_builder {
            Role::Builder
        } else {
            Role::Fighter
        }
    }
}

/// The choices of the ring the menu shows, for the unit it is open on (none when it has gone).
pub fn entries(game: &Game, radial: &Radial) -> Vec<Entry> {
    let Some(u) = game.sim.unit(radial.unit) else { return Vec::new() };
    if game.spectator {
        return watching(game, u);
    }
    match radial.ring() {
        Menu::Main => page(all(game, u), 0),
        Menu::More(k) => page(all(game, u), k),
        Menu::Fire => fire_ring(game),
        Menu::Movement => movement_ring(game),
        Menu::Units(k) => unit_ring(game, u, k),
        Menu::Purposes => purpose_ring(game, u),
        Menu::Buildings(p, k) => building_ring(game, u, p, k),
        Menu::Confirm => confirm_ring(game),
        Menu::Duties => duty_ring(game),
    }
}

/// What a spectator or a replay's watcher can do with a unit: see through its eyes, follow it,
/// read about it.
fn watching(game: &Game, u: &Unit) -> Vec<Entry> {
    let mut out = Vec::new();
    if crate::perspective::Perspective::can_enter(game, u.id) {
        let def = orders::Def { kind: Kind::Perspective(u.id), glyph: Glyph::Perspective, action: "perspective", group: orders::Group::Special };
        out.push(order_entry(game, &def));
    }
    out.push(Entry::new(Id::Follow, Glyph::Move, t("FOLLOW"), t("Follow the selected unit (scroll to let go)")).keyed(game, "follow").lit(game.follow == Some(u.id)));
    out.push(info_entry(game));
    out
}

/// How the choices stand on pages: all of them on one ring when they fit, else [`MOST`] places
/// to a page, the last of each but the last page the way on.
fn page(list: Vec<Entry>, k: usize) -> Vec<Entry> {
    if list.len() <= MOST {
        return if k == 0 { list } else { Vec::new() };
    }
    let per = MOST - 1;
    let from = (k * per).min(list.len());
    let rest = list.len() - from;
    let mut shown: Vec<Entry> = list.into_iter().skip(from).take(if rest > MOST { per } else { MOST }).collect();
    if rest > MOST {
        shown.push(Entry::new(Id::Open(Menu::More(k + 1)), Glyph::More, t("MORE"), t("The rest of the choices")).opening());
    }
    shown
}

/// Every choice the unit has, in the order of its kind: the first page is its ring (see
/// [`Choices`]).
fn all(game: &Game, u: &Unit) -> Vec<Entry> {
    Choices::of(game, u).list().into_iter().flatten().collect()
}

/// The choices a menu can offer the selection, one function for each, and the lists that stand
/// them on the ring for each kind of unit ([`Role`]): the first of a list is straight up, the rest
/// clockwise, and what does not fit goes on a page after it (`page`). A choice the selection
/// cannot make is `None` and leaves its place to the next.
///
/// To give a kind of unit another choice:
/// 1. make it a function here (an [`Entry`]: its [`Id`], its glyph, words, hint and `keyed` key;
///    `opening()` when it leads to a ring of its own);
/// 2. put it on the lists of the roles that have it (`factory`, `defence`, `builder`, `fighter`),
///    where it should stand on the ring;
/// 3. when it opens a ring: a [`Menu`] for it, its choices in `entries`, the words for the middle
///    in `title`; when it makes something happen: the [`Id`] it carries, done in
///    `Game::radial_pick`.
struct Choices<'a> {
    game: &'a Game,
    /// The unit the menu is open on.
    unit: &'a Unit,
    /// The orders the selection can give (`orders::available`).
    defs: Vec<orders::Def>,
    /// What the selection is and has.
    facts: Facts,
}

impl<'a> Choices<'a> {
    fn of(game: &'a Game, unit: &'a Unit) -> Choices<'a> {
        Choices { game, unit, defs: orders::available(game), facts: Facts::of(game) }
    }

    /// The list for the kind of unit the menu is on, in the order it stands on the ring.
    fn list(&self) -> Vec<Option<Entry>> {
        let mut choices = match Role::of(self.game.sim.unit_type(self.unit)) {
            Role::Factory => self.factory(),
            Role::Defence => self.defence(),
            Role::Builder => self.builder(),
            Role::Fighter => self.fighter(),
        };
        // Roles set the order, capabilities decide what is available. Include every
        // supported command even for unusual mod units and mixed selections.
        for def in &self.defs {
            if matches!(def.kind, Kind::Mode(_)) && !choices.iter().flatten().any(|e| e.id == Id::Order(def.kind)) {
                choices.push(Some(order_entry(self.game, def)));
            }
        }
        choices
    }

    // -- The lists ------------------------------------------------------------------------

    /// A factory: the build queue, repeat, stop; the routes the units it makes take.
    fn factory(&self) -> Vec<Option<Entry>> {
        vec![
            self.queue(),
            self.repeat(),
            self.stop(),
            self.mode(CommandMode::Move),
            self.mode(CommandMode::Patrol),
            self.mode(CommandMode::Fight),
            self.mode(CommandMode::Guard),
            self.mode(CommandMode::Attack),
            self.mode(CommandMode::DGun),
            self.power(),
            self.destruct(),
            self.info(),
        ]
    }

    /// A tower or another building: attack, stop, perspective, firing mode, self-destruct (and, for
    /// a turret that builds, its work).
    fn defence(&self) -> Vec<Option<Entry>> {
        vec![
            self.mode(CommandMode::Attack),
            // (Nothing to stop in a solar panel.)
            if self.facts.armed || self.facts.works { self.stop() } else { None },
            self.perspective(),
            self.fire(),
            self.mode(CommandMode::Repair),
            self.mode(CommandMode::Reclaim),
            self.mode(CommandMode::Guard),
            self.mode(CommandMode::Capture),
            self.mode(CommandMode::Resurrect),
            self.mode(CommandMode::DGun),
            self.power(),
            self.destruct(),
            self.info(),
        ]
    }

    /// A constructor: build, repair, reclaim, guard; then the rest.
    fn builder(&self) -> Vec<Option<Entry>> {
        vec![
            self.build(),
            self.mode(CommandMode::Repair),
            self.mode(CommandMode::Reclaim),
            self.mode(CommandMode::Guard),
            self.mode(CommandMode::Move),
            self.stop(),
            self.duty(),
            self.mode(CommandMode::Attack),
            self.mode(CommandMode::Patrol),
            self.mode(CommandMode::Fight),
            self.mode(CommandMode::DGun),
            self.mode(CommandMode::Capture),
            self.mode(CommandMode::Resurrect),
            self.mode(CommandMode::AreaMex),
            self.fire(),
            self.movement(),
            self.cloak(),
            self.perspective(),
            self.destruct(),
            self.info(),
        ]
    }

    /// A fighter or a transport: move, attack, fight, patrol, stop, the fire and move orders;
    /// then the rest.
    fn fighter(&self) -> Vec<Option<Entry>> {
        vec![
            self.mode(CommandMode::Move),
            self.mode(CommandMode::Attack),
            self.mode(CommandMode::Fight),
            self.mode(CommandMode::Patrol),
            self.stop(),
            self.fire(),
            self.movement(),
            self.mode(CommandMode::Guard),
            self.mode(CommandMode::Load),
            self.mode(CommandMode::Unload),
            self.mode(CommandMode::DGun),
            self.cloak(),
            // (A constructor without things to build, a rezzer or an assist kbot: its duties.)
            self.duty(),
            self.perspective(),
            self.destruct(),
            self.info(),
        ]
    }

    // -- The choices ----------------------------------------------------------------------

    /// An order of the plate's, when the selection can give it.
    fn order(&self, kind: Kind) -> Option<Entry> {
        self.defs.iter().find(|d| d.kind == kind).map(|d| order_entry(self.game, d))
    }

    fn mode(&self, mode: CommandMode) -> Option<Entry> {
        let entry = self.order(Kind::Mode(mode))?;
        // (A transport with nothing aboard has nothing to set down.)
        let empty = !self.game.selection.iter().filter_map(|&id| self.game.sim.unit(id)).any(|u| !u.cargo.is_empty());
        Some(if mode == CommandMode::Unload && empty { entry.disabled(t("Nothing loaded")) } else { entry })
    }

    fn stop(&self) -> Option<Entry> {
        let entry = self.order(Kind::Stop)?;
        let idle = self.game.selection.iter().filter_map(|&id| self.game.sim.unit(id)).all(nothing_to_stop);
        Some(if idle { entry.disabled(t("Nothing to stop")) } else { entry })
    }

    fn perspective(&self) -> Option<Entry> {
        self.defs.iter().find(|d| matches!(d.kind, Kind::Perspective(_))).map(|d| order_entry(self.game, d))
    }

    /// The fire order, when the selection has weapons: it opens the ring of them.
    fn fire(&self) -> Option<Entry> {
        self.facts.armed.then(|| fire_entry(self.game))
    }

    /// The move order, when the selection has weapons and moves: it opens the ring of them.
    fn movement(&self) -> Option<Entry> {
        (self.facts.armed && self.facts.mobile).then(|| movement_entry(self.game))
    }

    fn cloak(&self) -> Option<Entry> {
        self.facts.cloak.then(|| cloak_entry(self.game))
    }

    fn power(&self) -> Option<Entry> {
        self.facts.on_off.then(|| power_entry(self.game))
    }

    /// The constructors' duty, when the selection holds any that can take one: it opens the ring
    /// of them.
    fn duty(&self) -> Option<Entry> {
        duty_entry(self.game)
    }

    /// Self-destruct, which asks once more.
    fn destruct(&self) -> Option<Entry> {
        Some(destruct_entry(self.game))
    }

    fn info(&self) -> Option<Entry> {
        Some(info_entry(self.game))
    }

    /// A constructor's buildings, by purpose.
    fn build(&self) -> Option<Entry> {
        (!self.game.sim.unit_type(self.unit).build_options.is_empty()).then(|| build_entry(Menu::Purposes))
    }

    /// A factory's units.
    fn queue(&self) -> Option<Entry> {
        Some(build_entry(Menu::Units(0)))
    }

    fn repeat(&self) -> Option<Entry> {
        Some(repeat_entry(self.unit))
    }
}

/// Whether a unit has nothing for Stop to cancel: no orders, no way on its way, no building in
/// hand, no cargo being set down, no units queued and no route for the ones it makes (Stop clears
/// a factory's rally route and its queue, as `Sim::order_unit` has it).
fn nothing_to_stop(u: &Unit) -> bool {
    u.orders.is_empty() && u.path.is_empty() && u.build.is_none() && u.transport_job.is_none() && u.factory_queue.is_empty() && u.rally.is_empty()
}

/// An order of the plate's, as a choice.
fn order_entry(game: &Game, def: &orders::Def) -> Entry {
    let label = match def.kind {
        Kind::Stop => t("Stop").to_string(),
        Kind::Mode(mode) => t(mode.label_for(game)).to_string(),
        Kind::Perspective(_) => t("Perspective").to_string(),
    };
    Entry::new(Id::Order(def.kind), def.glyph, label, def.tip(game)).keyed(game, def.action)
}

/// What the first selected unit that can has set: fire and movement orders as the plate reads them.
fn settings(game: &Game) -> (u8, u8) {
    let first = |f: &dyn Fn(&ta_sim::UnitType) -> bool| game.selection.iter().filter_map(|&id| game.sim.unit(id)).find(|u| f(game.sim.unit_type(u)));
    let fire = first(&|t| t.weapons.iter().any(|w| w.is_some())).map_or(0, |u| u.fire_state.min(2));
    let movement = first(&|t| t.weapons.iter().any(|w| w.is_some()) && t.locomotion != Locomotion::Static).map_or(0, |u| u.move_state.min(2));
    (fire, movement)
}

const FIRE: [(Glyph, &str, &str); 3] = [
    (Glyph::HoldFire, n("HOLD FIRE"), n("Hold fire: never shoots")),
    (Glyph::ReturnFire, n("RETURN FIRE"), n("Return fire: shoots back when shot at")),
    (Glyph::FireAtWill, n("FIRE AT WILL"), n("Fire at will: shoots whatever comes in range")),
];

const MOVEMENT: [(Glyph, &str, &str); 3] = [
    (Glyph::HoldPosition, n("HOLD POSITION"), n("Hold position: stays where it is")),
    (Glyph::Maneuver, n("MANEUVER"), n("Maneuver: moves to answer fire nearby")),
    (Glyph::Roam, n("ROAM"), n("Roam: chases the enemy anywhere")),
];

/// The fire order as it stands, opening the ring of them.
fn fire_entry(game: &Game) -> Entry {
    let (fire, _) = settings(game);
    let (glyph, label, what) = FIRE[usize::from(fire)];
    Entry::new(Id::Open(Menu::Fire), glyph, t(label), t(what)).keyed(game, "fire_orders").opening()
}

/// The move order as it stands, opening the ring of them.
fn movement_entry(game: &Game) -> Entry {
    let (_, movement) = settings(game);
    let (glyph, label, what) = MOVEMENT[usize::from(movement)];
    Entry::new(Id::Open(Menu::Movement), glyph, t(label), t(what)).keyed(game, "move_orders").opening()
}

fn fire_ring(game: &Game) -> Vec<Entry> {
    let (fire, _) = settings(game);
    FIRE.iter().enumerate().map(|(k, &(glyph, label, what))| Entry::new(Id::Fire(k as u8), glyph, t(label), t(what)).lit(usize::from(fire) == k)).collect()
}

fn movement_ring(game: &Game) -> Vec<Entry> {
    let (_, movement) = settings(game);
    MOVEMENT.iter().enumerate().map(|(k, &(glyph, label, what))| Entry::new(Id::Movement(k as u8), glyph, t(label), t(what)).lit(usize::from(movement) == k)).collect()
}

fn cloak_entry(game: &Game) -> Entry {
    let can: Vec<&Unit> = game.selection.iter().filter_map(|&id| game.sim.unit(id)).filter(|u| game.sim.unit_type(u).def.cloak_cost > 0.0).collect();
    let on = !can.is_empty() && can.iter().all(|u| u.cloak_on);
    let (glyph, label) = if on { (Glyph::Cloaked, t("CLOAKED")) } else { (Glyph::Visible, t("VISIBLE")) };
    // (As the plate words it.)
    let (cost, moving) = can.first().map_or_else(Default::default, |u| {
        let ty = game.sim.unit_type(u);
        (format!("{:.0}", ty.def.cloak_cost), format!("{:.0}", ty.def.cloak_cost_moving.max(ty.def.cloak_cost)))
    });
    let key = game.keys.text("cloak");
    let args: [(&str, &dyn std::fmt::Display); 3] = [("key", &key), ("cost", &cost), ("moving", &moving)];
    let hint = if on { tf("Cloaked ({key}): click to show yourself. Cloaking costs {cost} energy/s ({moving} while moving)", &args) } else { tf("Visible ({key}): click to cloak, for {cost} energy/s ({moving} while moving)", &args) };
    Entry::new(Id::Cloak, glyph, label, hint).keyed(game, "cloak").lit(on)
}

fn power_entry(game: &Game) -> Entry {
    let active = game.selection.iter().filter_map(|&id| game.sim.unit(id)).find(|u| game.sim.unit_type(u).def.on_offable).is_some_and(|u| u.activated);
    let key = game.keys.text("on_off");
    let hint = if active { tf("On: click to turn off ({key})", &[("key", &key)]) } else { tf("Off: click to turn on ({key})", &[("key", &key)]) };
    Entry::new(Id::Power, Glyph::Power, if active { t("ON") } else { t("OFF") }, hint).keyed(game, "on_off").lit(active)
}

fn repeat_entry(u: &Unit) -> Entry {
    Entry::new(Id::Repeat, Glyph::Patrol, t("Repeat"), t("Finished units go back on the end of the queue")).lit(u.factory_repeat)
}

/// The duty control as the command plate words it, opening the ring of duties.
fn duty_entry(game: &Game) -> Option<Entry> {
    let control = game.duty_control()?;
    Some(Entry::new(Id::Open(Menu::Duties), Glyph::Duty, control.label, control.tip).keyed(game, "duty").opening().lit(control.lit))
}

/// The duties: each with what it does, the one the constructors are on lit, and "under my command".
fn duty_ring(game: &Game) -> Vec<Entry> {
    let now = game.selection_duty();
    crate::game::duty::CHOICES
        .iter()
        .map(|&(duty, glyph)| {
            let on = match duty {
                Some(_) => now.shared == duty,
                None => now.on_duty == 0,
            };
            Entry::new(Id::Duty(duty), glyph, t(crate::game::duty::choice_label(duty)), t(crate::game::duty::choice_blurb(duty))).lit(on)
        })
        .collect()
}

fn build_entry(menu: Menu) -> Entry {
    let (glyph, label, hint) = match menu {
        Menu::Units(_) => (Glyph::Queue, t("QUEUE"), t("Queue units to build")),
        _ => (Glyph::Build, t("BUILD"), t("Choose a building to place")),
    };
    Entry::new(Id::Open(menu), glyph, label, hint).opening()
}

/// Self-destruct: asks once more first; while the count runs it cancels at once.
fn destruct_entry(game: &Game) -> Entry {
    let counting = game.selection.iter().filter_map(|&id| game.sim.unit(id)).all(|u| u.self_destruct_at.is_some());
    if counting {
        Entry::new(Id::Destruct, Glyph::SelfDestruct, t("CANCEL"), t("Counting down: click to cancel")).keyed(game, "self_destruct").danger()
    } else {
        let hint = tf("Self-destruct: blow up the selection after a five-second countdown ({key}); again to cancel", &[("key", &game.keys.text("self_destruct"))]);
        Entry::new(Id::Open(Menu::Confirm), Glyph::SelfDestruct, t("SELF-DESTRUCT"), hint).keyed(game, "self_destruct").danger().opening()
    }
}

fn info_entry(game: &Game) -> Entry {
    Entry::new(Id::Info, Glyph::Info, t("INFO"), t("Information on the selected unit (with none, the controls reference)")).keyed(game, "help")
}

/// The ring that asks once more before the unit blows itself up.
fn confirm_ring(game: &Game) -> Vec<Entry> {
    let hint = tf("Self-destruct: blow up the selection after a five-second countdown ({key}); again to cancel", &[("key", &game.keys.text("self_destruct"))]);
    vec![
        Entry::new(Id::Open(Menu::Main), Glyph::Cancel, t("CANCEL"), t("Keep it")),
        Entry::new(Id::Destruct, Glyph::SelfDestruct, t("SELF-DESTRUCT"), hint).danger(),
    ]
}

/// What a unit type costs, as a second line.
fn cost_line(game: &Game, ty: usize) -> String {
    let k = game.sim.types.get(ty);
    format!("{:.0} · {:.0}", k.cost_metal.to_f32(), k.cost_energy.to_f32())
}

/// A unit type as a choice: its picture, name and cost, and what it is.
fn type_entry(game: &Game, id: Id, ty: usize, builder: &ta_sim::UnitType) -> Entry {
    let k = game.sim.types.get(ty);
    let label = crate::chat::unit_label(&game.sim, ty);
    let secs = if builder.worker_rate > ta_sim::Fx::ZERO { (k.build_time / builder.worker_rate).to_f32() / 30.0 } else { 0.0 };
    let mut hint = vec![label.clone()];
    if !k.def.description.trim().is_empty() {
        hint.push(k.def.description.trim().to_string());
    }
    hint.push(tf(
        "Metal {metal}  ·  Energy {energy}  ·  Build time {time} ({duration})",
        &[("metal", &format!("{:.0}", k.cost_metal.to_f32())), ("energy", &format!("{:.0}", k.cost_energy.to_f32())), ("time", &format!("{:.0}", k.build_time.to_f32())), ("duration", &crate::hud::duration(secs))],
    ));
    let mut e = Entry::new(id, Glyph::Build, label, hint.join("\n"));
    e.glyph = None;
    e.picture = Some(ty);
    e.detail = Some(cost_line(game, ty));
    e.cost = Some((k.cost_metal.to_f32(), k.cost_energy.to_f32()));
    e
}

/// A factory's units, a page of them: a click queues one, Shift five, right-click takes one off.
fn unit_ring(game: &Game, u: &Unit, k: usize) -> Vec<Entry> {
    let builder = game.sim.unit_type(u).clone();
    let options: Vec<usize> = builder.build_options.iter().copied().filter(|&o| game.sim.is_allowed(o)).collect();
    let queued = |ty: usize| u.factory_queue.iter().filter(|&&q| q == ty).count();
    let list: Vec<Entry> = options
        .iter()
        .map(|&o| {
            let mut e = type_entry(game, Id::Produce(o), o, &builder);
            e.hint = format!("{}\n{}", e.hint, t("Click: one more  ·  Shift+click: five  ·  Right-click: fewer"));
            e.count = queued(o);
            e.stays = true;
            e
        })
        .collect();
    let mut shown = page(list, k);
    // (The page's way on turns the ring, not opens another on top of it.)
    for e in &mut shown {
        if let Id::Open(Menu::More(p)) = e.id {
            e.id = Id::Open(Menu::Units(p));
        }
    }
    shown
}

/// A constructor's kinds of building by purpose: each with the picture of its first.
fn purpose_ring(game: &Game, u: &Unit) -> Vec<Entry> {
    let builder = game.sim.unit_type(u);
    let options: Vec<usize> = builder.build_options.iter().copied().filter(|&o| game.sim.is_allowed(o)).collect();
    Purpose::ALL
        .iter()
        .enumerate()
        .filter_map(|(i, &p)| {
            let of: Vec<usize> = options.iter().copied().filter(|&o| Purpose::of(&game.sim.types, game.sim.types.get(o)) == p).collect();
            let first = *of.first()?;
            let mut e = Entry::new(Id::Open(Menu::Buildings(i, 0)), Glyph::Build, t(p.short()), t(p.label())).opening();
            e.glyph = None;
            e.picture = Some(first);
            e.count = of.len();
            Some(e)
        })
        .collect()
}

/// The buildings of a purpose, a page of them: a click begins placing one.
fn building_ring(game: &Game, u: &Unit, purpose: usize, k: usize) -> Vec<Entry> {
    let builder = game.sim.unit_type(u).clone();
    let Some(&p) = Purpose::ALL.get(purpose) else { return Vec::new() };
    let list: Vec<Entry> = builder
        .build_options
        .iter()
        .copied()
        .filter(|&o| game.sim.is_allowed(o) && Purpose::of(&game.sim.types, game.sim.types.get(o)) == p)
        .map(|o| type_entry(game, Id::Place(o), o, &builder))
        .collect();
    let mut shown = page(list, k);
    for e in &mut shown {
        if let Id::Open(Menu::More(next)) = e.id {
            e.id = Id::Open(Menu::Buildings(purpose, next));
        }
    }
    shown
}

/// The words in the middle of the menu: what the ring is about.
pub fn title(game: &Game, radial: &Radial) -> String {
    let Some(u) = game.sim.unit(radial.unit) else { return String::new() };
    let name = crate::chat::unit_label(&game.sim, u.type_id);
    let many = game.selection.len();
    match radial.ring() {
        Menu::Confirm if many > 1 => tf("Blow up {n} units?", &[("n", &many)]),
        Menu::Confirm => tf("Blow up {name}?", &[("name", &name)]),
        _ if many > 1 && game.selection.contains(&radial.unit) => tf("{name} × {n}", &[("name", &name), ("n", &many)]),
        _ => name,
    }
}

/// What the ring on top is called, for the middle of the menu when it is not the first (none for
/// the unit's own ring).
pub fn ring_name(radial: &Radial) -> Option<String> {
    Some(
        match radial.ring() {
            Menu::Main => return None,
            Menu::Confirm => t("SELF-DESTRUCT"),
            Menu::More(_) => t("MORE"),
            Menu::Fire => t("FIRE ORDERS"),
            Menu::Movement => t("MOVE ORDERS"),
            Menu::Units(_) => t("QUEUE"),
            Menu::Purposes => t("BUILD"),
            Menu::Buildings(p, _) => t(Purpose::ALL.get(p)?.short()),
            Menu::Duties => t("DUTY"),
        }
        .to_string(),
    )
}

/// How many pages a list of `len` choices stands on (see `page`): one while they fit.
pub fn pages_for(len: usize) -> usize {
    if len <= MOST { 1 } else { 1 + (len - MOST).div_ceil(MOST - 1) }
}

/// Which page of how many the ring on top is, where its choices run to more than one.
pub fn paging(game: &Game, radial: &Radial) -> Option<(usize, usize)> {
    let u = game.sim.unit(radial.unit)?;
    let (len, at) = match radial.ring() {
        Menu::Main => (all(game, u).len(), 0),
        Menu::More(k) => (all(game, u).len(), k),
        Menu::Units(k) => (game.sim.unit_type(u).build_options.iter().filter(|&&o| game.sim.is_allowed(o)).count(), k),
        Menu::Buildings(p, k) => {
            let kind = *Purpose::ALL.get(p)?;
            (game.sim.unit_type(u).build_options.iter().filter(|&&o| game.sim.is_allowed(o) && Purpose::of(&game.sim.types, game.sim.types.get(o)) == kind).count(), k)
        }
        _ => return None,
    };
    let total = pages_for(len);
    (total > 1).then_some((at.min(total - 1), total))
}

/// How hurt the selection is, as the share of health it has left (the menu's unit when it is alone).
pub fn health(game: &Game, radial: &Radial) -> Option<f32> {
    let mut ids: Vec<UnitId> = game.selection.iter().copied().filter(|&id| game.sim.unit(id).is_some()).collect();
    if ids.is_empty() {
        ids.push(radial.unit);
    }
    let (have, most) = ids.iter().filter_map(|&id| game.sim.unit(id)).fold((0.0f32, 0.0f32), |(h, m), u| (h + u.health.to_f32(), m + game.sim.unit_type(u).max_health.to_f32()));
    (most > 0.0).then(|| (have / most).clamp(0.0, 1.0))
}

/// How long the right button is held on a unit of ours, the pointer still, before its menu opens
/// (seconds).
pub const HOLD: f32 = 0.32;

/// What a plain right click on one of our units finds: the rule that decides whether it opens the
/// unit's menu or does what the right button did before (`Game::radial_for_click`).
#[derive(Clone, Copy, Debug)]
pub struct Click {
    /// The left button gives orders (TA's original scheme), the right one only clears the selection.
    pub left_orders: bool,
    /// Watching: a spectator or a replay, with no orders to give.
    pub watching: bool,
    /// An order waits for its click or a building is being placed: the right click cancels that.
    pub waiting: bool,
    /// The unit is in the selection.
    pub selected: bool,
    pub nothing_selected: bool,
    /// The setting keeps the menu to a hold of the button (`RadialOpens::Hold`): a click is
    /// never the menu's.
    pub hold_only: bool,
}

impl Click {
    /// A right click opens the menu where it would not give an order: nothing is selected, or the
    /// unit is in the selection (it would only be told to move to where it stands), or the left
    /// button gives the orders and the right one clears the selection; never while an order or
    /// a placement waits for its click (the right click cancels it), nor when the player keeps the
    /// menu to a hold. Holding the button opens it in every case but a wait (`hold_opens`).
    pub fn opens_menu(self) -> bool {
        !self.hold_only && !self.waiting && (self.watching || self.left_orders || self.nothing_selected || self.selected)
    }
}

/// Whether the right button held `held` seconds with the pointer `moved` pixels from where it went
/// down opens the menu (on a unit that can have one): still, and long enough; with Ctrl it draws a
/// formation instead.
pub fn hold_opens(held: f32, moved: f32, ctrl: bool) -> bool {
    !ctrl && moved < 6.0 && held >= HOLD
}

impl Game {
    /// The unit under the pointer that a menu can be opened on: one of ours that is finished (a
    /// spectator's: any in sight).
    pub(crate) fn radial_target(&self) -> Option<UnitId> {
        if !self.radial_enabled {
            return None;
        }
        let id = self.hovered?;
        let u = self.sim.unit(id)?;
        if self.spectator {
            return Some(id);
        }
        (u.owner == self.local_player && u.is_complete() && u.carried_by.is_none()).then_some(id)
    }

    /// The unit a plain right click opens the menu on: one of ours (or, watching, any) when the
    /// click would not give an order: nothing is selected, or the unit is in the selection (a
    /// move to where it stands); with the left button giving orders, where the right one only
    /// clears the selection, always. (An order waiting for its click, a building being placed:
    /// the click cancels those.) None when the player keeps the menu to a hold. Holding the
    /// button does it in every case, and so does a middle click.
    pub(crate) fn radial_for_click(&self) -> Option<UnitId> {
        let id = self.radial_target()?;
        let left_orders = self.order_button == ta_config::OrderButton::Left;
        let click = Click {
            left_orders,
            watching: self.spectator,
            waiting: self.command_mode.is_some() || self.placement.is_some(),
            selected: self.selection.contains(&id),
            nothing_selected: self.selection.is_empty(),
            hold_only: self.radial_opens == ta_config::RadialOpens::Hold,
        };
        click.opens_menu().then_some(id)
    }

    /// Opens the menu on `id` at `at` (window pixels): the unit becomes the selection (or, in it,
    /// the selection's first), as the menu's choices are the selection's.
    pub fn open_radial(&mut self, id: UnitId, at: Vec2, held: bool) {
        match self.selection.iter().position(|&s| s == id) {
            Some(i) => self.selection.swap(0, i),
            None => {
                self.selection = vec![id];
                self.voices.push((id, ta_audio::SoundEvent::Select));
            }
        }
        self.command_mode = None;
        self.radial = Some(Radial::open(id, at, held));
    }

    /// The menu's keys and the pad's stick, for a frame it is up: Escape goes back, a choice's
    /// own key makes it, the left stick lights the choice it points at. (The pointer's part is the
    /// interface's, `Hud::radial_menu`.) The menu goes when its unit does.
    pub(crate) fn radial_input(&mut self, input: &Input) {
        let Some(mut radial) = self.radial.take() else { return };
        let gone = self.sim.unit(radial.unit).is_none_or(|u| u.dead || (!self.spectator && u.owner != self.local_player));
        if gone || !self.radial_enabled {
            return;
        }
        if radial.leaving.is_none() {
            let entries = entries(self, &radial);
            if input.pressed(winit::keyboard::KeyCode::Escape) {
                if !radial.back() {
                    radial.close(None);
                }
            } else if let Some(i) = entries.iter().position(|e| !e.disabled && e.action.is_some_and(|a| self.keys.pressed(input, a))) {
                self.radial_pick(&mut radial, &entries, i, input.modifiers.shift_key(), false);
            } else if input.pad_pan.length() >= STICK && !entries.is_empty() {
                radial.hover = Some(sector(input.pad_pan, entries.len()));
                radial.from_pad = true;
            }
        }
        self.radial = Some(radial);
    }

    /// The pad's d-pad for the menu: round the ring.
    pub fn radial_nav(&mut self, nav: &[crate::gamepad::Nav]) {
        use crate::gamepad::Nav;
        let Some(mut radial) = self.radial.take() else { return };
        if radial.leaving.is_none() {
            let count = entries(self, &radial).len();
            for n in nav {
                let delta = match n {
                    Nav::Left | Nav::Up => -1,
                    Nav::Right | Nav::Down => 1,
                    _ => continue,
                };
                radial.hover = step(radial.hover, count, delta);
                radial.from_pad = true;
            }
        }
        self.radial = Some(radial);
    }

    /// Makes choice `index` of `entries`: another ring opens, or what it asks is done and the menu
    /// closes (a unit queued, with `shift` five of them and with `secondary` one fewer, leaves it
    /// open). What it does it does to the selection, which holds the menu's unit.
    pub fn radial_pick(&mut self, radial: &mut Radial, entries: &[Entry], index: usize, shift: bool, secondary: bool) {
        let Some(entry) = entries.get(index) else { return };
        // (What would do nothing is left alone: the ring stays up, the choice dim, and shaken.)
        if entry.disabled {
            radial.refused = Some(index);
            return;
        }
        radial.picked = Some(index);
        if !self.selection.contains(&radial.unit) {
            self.selection = vec![radial.unit];
        }
        let mut stay = entry.stays;
        match entry.id {
            Id::Open(menu) => {
                if menu == Menu::Main {
                    if !radial.back() {
                        radial.close(None);
                    }
                } else if same_family(radial.ring(), menu) {
                    radial.turn_to(menu);
                } else {
                    radial.places.truncate(radial.rings.len() - 1);
                    radial.places.push(Place { index, of: entries.len() });
                    radial.push(menu);
                }
                return;
            }
            Id::Order(Kind::Stop) => {
                self.stop_selection();
                self.command_mode = None;
            }
            Id::Order(Kind::Mode(mode)) => {
                self.set_command_mode(Some(mode));
                self.ui_sounds.push("SPECIALORDERS");
            }
            Id::Order(Kind::Perspective(id)) => self.perspective_request = Some(id),
            Id::Fire(k) => self.set_states(Some(k), None),
            Id::Movement(k) => self.set_states(None, Some(k)),
            Id::Cloak => self.toggle_cloak(),
            Id::Power => {
                let on = self.selection.iter().filter_map(|&id| self.sim.unit(id)).find(|u| self.sim.unit_type(u).def.on_offable).is_some_and(|u| u.activated);
                self.set_active(!on);
            }
            Id::Repeat => {
                let on = self.sim.unit(radial.unit).is_some_and(|u| u.factory_repeat);
                self.set_repeat(!on);
            }
            Id::Produce(ty) => {
                self.produce(ty, if secondary { -1 } else if shift { 5 } else { 1 });
                stay = true;
            }
            Id::Place(ty) => self.begin_placement(ty),
            Id::Destruct => self.self_destruct_selection(),
            Id::Info => {
                if let Some(u) = self.sim.unit(radial.unit) {
                    self.unit_info = Some(UnitInfo { type_id: u.type_id, unit: Some(radial.unit) });
                }
            }
            Id::Follow => self.follow = if self.follow == Some(radial.unit) { None } else { Some(radial.unit) },
            Id::Duty(duty) => self.set_duty(duty),
        }
        if !stay {
            radial.close(Some(index));
        }
    }
}

/// Whether a choice that opens `to` turns the page of the ring `from` is on, instead of opening
/// another ring over it. (The rest of a unit's choices stack, so that the way back is the page
/// before; the pages of a list of pictures replace each other, and the way back is the ring
/// that led to them.)
fn same_family(from: Menu, to: Menu) -> bool {
    match (from, to) {
        (Menu::Units(_), Menu::Units(_)) => true,
        (Menu::Buildings(a, _), Menu::Buildings(b, _)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A right click opens a unit's menu where it would not have given an order, and never over
    /// an order or a building waiting for its click; holding the button is for the rest.
    #[test]
    fn a_right_click_opens_the_menu_where_it_would_not_order() {
        let click = |left_orders, nothing_selected, selected, waiting| Click { left_orders, watching: false, waiting, selected, nothing_selected, hold_only: false }.opens_menu();
        // The right button gives orders: nothing selected, or the unit itself selected.
        assert!(click(false, true, false, false), "nothing selected: its menu");
        assert!(click(false, false, true, false), "the unit in the selection: a move to itself would be no order");
        assert!(!click(false, false, false, false), "others selected: the click guards, repairs or loads");
        // Left-button orders: the right button only cleared the selection, so over a unit of ours
        // it opens the menu whatever is selected.
        assert!(click(true, false, false, false));
        assert!(click(true, true, false, false));
        // An order waiting for its click, a building being placed: the click cancels it.
        assert!(!click(false, true, false, true));
        assert!(!click(true, false, true, true));
        // Watching: any unit.
        assert!(Click { left_orders: false, watching: true, waiting: false, selected: false, nothing_selected: false, hold_only: false }.opens_menu());
        // Kept to a hold (`RadialOpens::Hold`): a click always does what the button did before,
        // whatever the scheme, the selection or the watching.
        for (left_orders, watching, selected, nothing_selected) in [(false, false, false, true), (false, false, true, false), (true, false, false, false), (false, true, false, false)] {
            assert!(!Click { left_orders, watching, waiting: false, selected, nothing_selected, hold_only: true }.opens_menu());
        }
    }

    /// Holding opens it when the pointer is still and the time is up, not with Ctrl (a formation).
    #[test]
    fn holding_the_button_opens_it_when_still() {
        assert!(hold_opens(HOLD, 0.0, false));
        assert!(hold_opens(2.0, 5.9, false));
        assert!(!hold_opens(HOLD - 0.05, 0.0, false), "not yet");
        assert!(!hold_opens(1.0, 6.0, false), "the pointer moved: an orbit or a drag");
        assert!(!hold_opens(1.0, 0.0, true), "Ctrl: a formation line");
    }

    /// The kinds of unit of the real game land in the roles their menus are made for: a lab is a
    /// factory, a tower and a turret that only builds are buildings, the Commander and the
    /// constructors build, the rest fight.
    #[test]
    fn the_units_of_the_game_have_the_menus_of_their_kind() {
        let Ok(install) = ta_vfs::install::discover(None) else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "base").unwrap();
        let data = ta_data::GameData::load(&vfs);
        let types = ta_sim::UnitTypes::compile(&data, &|p| vfs.read_opt(p));
        let role = |name: &str| Role::of(types.by_name(name).unwrap_or_else(|| panic!("no {name}")));
        for (name, want) in [
            ("ARMLAB", Role::Factory),
            ("CORVP", Role::Factory),
            ("ARMSY", Role::Factory),
            ("ARMAP", Role::Factory),
            ("ARMLLT", Role::Defence),
            ("ARMSOLAR", Role::Defence),
            ("ARMRAD", Role::Defence),
            ("ARMNANOTC", Role::Defence),
            ("ARMCOM", Role::Builder),
            ("CORCOM", Role::Builder),
            ("ARMCV", Role::Builder),
            ("ARMCK", Role::Builder),
            ("ARMCA", Role::Builder),
            ("ARMSTUMP", Role::Fighter),
            ("ARMFAST", Role::Fighter),
            ("ARMFIG", Role::Fighter),
            ("ARMATLAS", Role::Fighter),
        ] {
            if types.by_name(name).is_some() {
                assert_eq!(role(name), want, "{name}");
            }
        }
    }

    /// Letting go of a button over the menu: a left click makes the choice, goes back at the middle,
    /// closes away from it; a right click goes back, takes a queued unit off, or in a held menu
    /// makes the choice it is let go over (the flick), and at the middle leaves the menu up.
    #[test]
    fn letting_go_of_a_button_does_what_the_gesture_means() {
        use Button::{Left, Right};
        let go = |b, lit, held, pad, middle, far, fewer| released(b, lit, held, pad, middle, far, fewer);
        // Left: a choice, the middle (back), away (closed), nothing in between choices.
        assert_eq!(go(Left, Some(3), false, false, false, false, false), Release::Pick(3));
        assert_eq!(go(Left, None, false, false, true, false, false), Release::Back);
        assert_eq!(go(Left, None, false, false, false, true, false), Release::Close);
        assert_eq!(go(Left, None, false, false, false, false, false), Release::Nothing);
        // A stick lit a choice with the pointer at the middle: A makes it.
        assert_eq!(go(Left, Some(2), false, true, true, false, false), Release::Pick(2));
        // A left click at the middle with the pointer's own choice lit there is a way back.
        assert_eq!(go(Left, Some(2), false, false, true, false, false), Release::Back);
        // Right, not held: back; over a queued unit one fewer; at the middle back.
        assert_eq!(go(Right, Some(1), false, false, false, false, false), Release::Back);
        assert_eq!(go(Right, Some(1), false, false, false, false, true), Release::Fewer(1));
        assert_eq!(go(Right, None, false, false, true, false, true), Release::Back);
        assert_eq!(go(Right, None, false, false, false, true, false), Release::Back);
        // Right, held open: letting go over a choice makes it (queued or not), at the middle the menu
        // stays up, far away it goes back.
        assert_eq!(go(Right, Some(5), true, false, false, false, false), Release::Pick(5));
        assert_eq!(go(Right, Some(5), true, false, false, false, true), Release::Pick(5));
        assert_eq!(go(Right, None, true, false, true, false, false), Release::Settle);
        assert_eq!(go(Right, Some(4), true, false, true, false, false), Release::Settle);
        assert_eq!(go(Right, None, true, false, false, true, false), Release::Back);
    }

    fn choices(n: usize) -> Vec<Entry> {
        (0..n).map(|i| Entry::new(Id::Produce(i), Glyph::Info, format!("choice {i}"), "")).collect()
    }

    /// Up to nine choices stand on one ring; more go on pages of eight, each but the last with its
    /// way on, and every choice is on exactly one page.
    #[test]
    fn the_choices_stand_on_pages_of_a_ring() {
        let ids = |list: &[Entry]| list.iter().map(|e| e.id).collect::<Vec<_>>();
        assert_eq!(page(choices(9), 0).len(), 9, "nine fit");
        assert!(page(choices(9), 1).is_empty(), "and leave no second page");
        assert!(page(choices(0), 0).is_empty());
        // Ten: eight and the way on, then the other two.
        let first = page(choices(10), 0);
        assert_eq!(first.len(), 9);
        assert_eq!(first[8].id, Id::Open(Menu::More(1)));
        assert_eq!(page(choices(10), 1).len(), 2);
        // However many, nothing is lost or shown twice.
        for n in [10usize, 17, 18, 25, 40] {
            let mut seen = Vec::new();
            let mut k = 0;
            loop {
                let ring = page(choices(n), k);
                assert!(ring.len() <= MOST, "{n} choices, page {k}: {}", ring.len());
                let more = ring.last().is_some_and(|e| matches!(e.id, Id::Open(Menu::More(_))));
                seen.extend(ids(&ring).into_iter().filter(|i| !matches!(i, Id::Open(_))));
                if !more {
                    break;
                }
                k += 1;
            }
            assert_eq!(seen, ids(&choices(n)), "{n} choices over their pages");
        }
    }

    /// Stop is dull only where there is nothing for it to cancel: a unit with an order, a way to
    /// go, a building in hand, a load to set down, a queue or a rally route (a factory's Stop
    /// clears both) is still something to stop.
    #[test]
    fn stop_is_dull_only_where_it_has_nothing_to_cancel() {
        let fresh = || Unit::new(UnitId { slot: 0, generation: 0 }, 0, 0, ta_sim::FxVec3::ZERO);
        assert!(nothing_to_stop(&fresh()));
        let mut busy = fresh();
        busy.orders.push_back(ta_sim::Order::Stop);
        assert!(!nothing_to_stop(&busy), "an order");
        let mut walking = fresh();
        walking.path.push((1, 2));
        assert!(!nothing_to_stop(&walking), "a way to go");
        let mut making = fresh();
        making.factory_queue.push_back(3);
        assert!(!nothing_to_stop(&making), "a unit queued");
        let mut routed = fresh();
        routed.rally.push(ta_sim::Order::Stop);
        assert!(!nothing_to_stop(&routed), "a rally route: Stop clears it");
        let mut carrying = fresh();
        carrying.transport_job = Some((UnitId { slot: 1, generation: 0 }, 0));
        assert!(!nothing_to_stop(&carrying), "cargo on its way");
    }

    /// The count of pages (for the dots in the middle of the menu) is the number the choices are
    /// laid on.
    #[test]
    fn the_pages_are_counted_as_they_are_laid() {
        for n in 0..70usize {
            let (mut pages, mut k) = (0, 0);
            loop {
                let ring = page(choices(n), k);
                pages += 1;
                if !ring.last().is_some_and(|e| matches!(e.id, Id::Open(Menu::More(_)))) {
                    break;
                }
                k += 1;
            }
            assert_eq!(pages_for(n), pages, "{n} choices");
        }
    }

    /// The first choice is straight up and the rest follow clockwise; the middle and far away
    /// pick nothing.
    #[test]
    fn a_direction_picks_a_choice_clockwise_from_the_top() {
        assert_eq!(pick(Vec2::new(0.0, -90.0), 8, 1.0), Some(0));
        assert_eq!(pick(Vec2::new(90.0, 0.0), 8, 1.0), Some(2));
        assert_eq!(pick(Vec2::new(0.0, 90.0), 8, 1.0), Some(4));
        assert_eq!(pick(Vec2::new(-90.0, 0.0), 8, 1.0), Some(6));
        assert_eq!(pick(Vec2::new(64.0, -64.0), 8, 1.0), Some(1));
        assert_eq!(pick(Vec2::new(3.0, 4.0), 8, 1.0), None, "the middle");
        assert_eq!(pick(Vec2::new(0.0, -600.0), 8, 1.0), None, "far away");
        assert_eq!(pick(Vec2::new(0.0, -200.0), 8, 1.0), Some(0), "a little past the ring still picks");
        // Two choices: the top half and the bottom half.
        assert_eq!(pick(Vec2::new(60.0, -30.0), 2, 1.0), Some(0));
        assert_eq!(pick(Vec2::new(-60.0, 30.0), 2, 1.0), Some(1));
        assert_eq!(pick(Vec2::new(0.0, -90.0), 0, 1.0), None);
    }

    /// The middle of a sector is where `angle` says, and a point there picks it, whatever the count.
    #[test]
    fn the_sectors_are_where_they_are_drawn() {
        for count in 1..=12 {
            for i in 0..count {
                let a = angle(i, count);
                let at = Vec2::new(a.sin(), -a.cos()) * 90.0;
                assert_eq!(sector(at, count), i, "{i} of {count}");
            }
        }
    }

    /// The d-pad walks round the ring, wrapping; from nothing the first step lands on the first
    /// (the last going back).
    #[test]
    fn stepping_wraps_round_the_ring() {
        assert_eq!(step(None, 5, 1), Some(0));
        assert_eq!(step(None, 5, -1), Some(4));
        assert_eq!(step(Some(4), 5, 1), Some(0));
        assert_eq!(step(Some(0), 5, -1), Some(4));
        assert_eq!(step(Some(2), 5, 1), Some(3));
        assert_eq!(step(None, 0, 1), None);
    }
}
