//! A coach for a player's first battles: it watches for what new players get stuck on (no
//! factory yet, a store running dry or overflowing, factories and builders left idle, the
//! Commander badly hurt) and says once each, briefly, what to do and the key that does it. Advice
//! that is about a place or a unit has a LOOK chip that goes there (a click on the words too);
//! every advice has a cross that puts it away and a line that stops it for good (`Coach::muted`,
//! kept in the settings). It stands still while the pointer is on it, and goes pale when the
//! pointer comes near. It only reads the battle. On for a profile's first battles, always, or
//! never (`ta_config::Coach`); `TA_COACH=1` turns it on in any battle and `TA_COACH_SAY=<advice>`
//! says one at once (captures).

use ta_sim::Fx;
use ta_ui::i18n::{n, t};

use crate::game::{Game, Link};
use crate::hint::{self, Plate, Pressed};

/// Battles a profile finishes before the coach (`Coach::FirstBattles`) says no more.
pub const FIRST_BATTLES: u64 = 5;

/// How long advice stays up, the quiet before the next, and the battle's opening the coach
/// leaves to the start tip (seconds of battle).
const SHOWN: f32 = 14.0;
const QUIET: f32 = 40.0;
const OPENING: f32 = 30.0;

/// Something worth a word, most pressing first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Advice {
    CommanderHurt,
    EnergyDry,
    MetalDry,
    NoFactory,
    FactoryIdle,
    BuildersIdle,
    MetalFull,
}

impl Advice {
    const ALL: [Advice; 7] = [Advice::CommanderHurt, Advice::EnergyDry, Advice::MetalDry, Advice::NoFactory, Advice::FactoryIdle, Advice::BuildersIdle, Advice::MetalFull];

    fn index(self) -> usize {
        Advice::ALL.iter().position(|&a| a == self).unwrap_or(0)
    }

    fn text(self) -> &'static str {
        match self {
            Advice::CommanderHurt => n("Your Commander is badly hurt! Pull it back out of the fight: losing it usually loses the battle."),
            Advice::EnergyDry => n("Energy is running out, and everything builds more slowly. Build Solar Collectors or Wind Generators, and build less at once until it recovers."),
            Advice::MetalDry => n("Metal is running out, and everything builds more slowly. Put Metal Extractors on more deposits ({area_mex} takes a whole area), or reclaim wrecks and rocks ({reclaim})."),
            Advice::NoFactory => n("No factory yet: factories make your army. Your Commander can build a Kbot Lab or a Vehicle Plant."),
            Advice::FactoryIdle => n("A factory has nothing to build. {idle_factory} selects it: queue some units, and turn on Repeat to keep it busy."),
            Advice::BuildersIdle => n("Builders stand idle. {idle_builder} selects the next one: give it something to build, or send it to help a factory ({guard})."),
            Advice::MetalFull => n("Your metal store is full, so new metal goes to waste. Spend it: another factory, more builders, more units."),
        }
    }

    /// Seconds of battle the situation lasts before the coach speaks.
    fn after(self) -> f32 {
        match self {
            Advice::CommanderHurt | Advice::NoFactory => 0.0,
            Advice::EnergyDry | Advice::MetalDry => 6.0,
            Advice::MetalFull => 25.0,
            Advice::FactoryIdle | Advice::BuildersIdle => 30.0,
        }
    }

    /// Its name for `TA_COACH_SAY`, and for the settings' list of what to say no more.
    fn name(self) -> &'static str {
        match self {
            Advice::CommanderHurt => "commander_hurt",
            Advice::EnergyDry => "energy_dry",
            Advice::MetalDry => "metal_dry",
            Advice::NoFactory => "no_factory",
            Advice::FactoryIdle => "factory_idle",
            Advice::BuildersIdle => "builders_idle",
            Advice::MetalFull => "metal_full",
        }
    }

    pub fn named(name: &str) -> Option<Advice> {
        Advice::ALL.into_iter().find(|a| a.name() == name)
    }

    /// Whether the situation holds now, for the local player.
    fn holds(self, game: &Game) -> bool {
        let me = game.local_player;
        let sim = &game.sim;
        let Some(p) = sim.players.get(me as usize) else { return false };
        let e = &p.economy;
        // A store under a twentieth full and spending more than it takes in.
        let dry = |stock: Fx, capacity: Fx, income: Fx, expense: Fx| capacity > Fx::ZERO && stock * 20 < capacity && expense > income;
        let ours = || sim.units().filter(move |u| u.owner == me && !u.dead);
        // (Advice to build is for a side that can: a scenario's army without a Commander or a
        // builder, Armageddon's, has nothing to build with.)
        let builds = |wanted: &dyn Fn(&ta_sim::UnitType) -> bool| can_build(sim, me, wanted);
        match self {
            Advice::CommanderHurt => ours().any(|u| sim.unit_type(u).def.is_commander && u.health * 2 < sim.unit_type(u).max_health),
            Advice::EnergyDry => dry(e.energy, e.energy_capacity, e.energy_income, e.energy_expense) && builds(&|_| true),
            Advice::MetalDry => dry(e.metal, e.metal_capacity, e.metal_income, e.metal_expense) && builds(&|_| true),
            Advice::NoFactory => sim.tick > 30 * 120 && !ours().any(|u| sim.unit_type(u).is_factory) && builds(&|t| t.is_factory),
            Advice::FactoryIdle => !game.idle_factories().is_empty(),
            // (Not the Commander alone: no factory yet says more.)
            Advice::BuildersIdle => game.idle_builders().into_iter().any(|id| sim.unit(id).is_some_and(|u| !sim.unit_type(u).def.is_commander)),
            Advice::MetalFull => e.metal_capacity > Fx::ZERO && e.metal * 20 >= e.metal_capacity * 19 && e.metal_income > e.metal_expense && builds(&|_| true),
        }
    }

    /// Where the advice points, if it does: the Commander (to pull back, or to build the factory
    /// with), the idle factory or builder, a metal deposit near the Commander with nothing on
    /// it.
    fn link(self, game: &Game) -> Option<Link> {
        let sim = &game.sim;
        match self {
            Advice::CommanderHurt | Advice::NoFactory => game.local_commander().map(Link::Unit),
            Advice::FactoryIdle => game.idle_factories().first().map(|&id| Link::Unit(id)),
            Advice::BuildersIdle => game.idle_builders().into_iter().find(|&id| sim.unit(id).is_some_and(|u| !sim.unit_type(u).def.is_commander)).map(Link::Unit),
            Advice::MetalDry => game.free_metal_near_commander().map(Link::Place),
            Advice::EnergyDry | Advice::MetalFull => None,
        }
    }
}

/// Whether `player` has a finished unit that can build what `wanted` takes (a factory, or
/// anything at all), the battle's restrictions allowing.
pub(crate) fn can_build(sim: &ta_sim::Sim, player: u8, wanted: impl Fn(&ta_sim::UnitType) -> bool) -> bool {
    sim.units()
        .filter(|u| u.owner == player && !u.dead && u.is_complete())
        .any(|u| sim.unit_type(u).build_options.iter().any(|&b| sim.is_allowed(b) && wanted(sim.types.get(b))))
}

#[derive(Default)]
pub struct Coach {
    /// How long each situation has held (seconds of battle).
    held: [f32; 7],
    /// What has been said: each once a battle.
    said: [bool; 7],
    /// The advice up, and since when (seconds of battle).
    shown: Option<(Advice, f32)>,
    /// The battle's clock at the last look, when the last advice went, and whether the one up
    /// is still to be heard.
    last: Option<f32>,
    quiet_from: f32,
    chime: bool,
    /// What the player asked never to hear again (advice names, as the settings keep them).
    muted: Vec<String>,
    /// Where the advice up points (found as it comes up, and looked for again each second while
    /// there is nowhere yet), which advice it was found for, and when it was last looked for.
    link: Option<Link>,
    link_for: Option<Advice>,
    link_tried: f32,
    /// The battle's clock when it was last drawn (its time stands still under the pointer).
    drawn: Option<f32>,
}

impl Coach {
    /// A coach that says nothing of what `muted` names.
    pub fn new(muted: &[String]) -> Coach {
        Coach { muted: muted.to_vec(), ..Coach::default() }
    }

    fn is_muted(&self, advice: Advice) -> bool {
        self.muted.iter().any(|m| m == advice.name())
    }

    /// Says `advice` from the battle's next look (captures).
    pub fn say(&mut self, advice: Advice) {
        self.show(advice, f32::NEG_INFINITY);
    }

    fn show(&mut self, advice: Advice, now: f32) {
        self.said[advice.index()] = true;
        self.shown = Some((advice, now));
        self.chime = true;
    }

    /// Watches the battle: its clock is the simulation's, so a paused battle waits.
    pub fn update(&mut self, game: &Game) {
        if game.playback.is_some() || game.spectator || game.tutorial {
            return;
        }
        let now = game.sim.tick as f32 / 30.0;
        // (What is muted is never held: it does not wait its turn, nor take anyone's.)
        let holding = Advice::ALL.map(|a| !self.is_muted(a) && a.holds(game));
        self.step(now, holding);
        // Where the advice up points is found as it comes up (and again each second while there
        // is nowhere: an idle factory may only just have stood idle).
        let up = self.shown.map(|(a, _)| a);
        if up != self.link_for || up.is_some() && self.link.is_none() && now - self.link_tried >= 1.0 {
            self.link_for = up;
            self.link_tried = now;
            self.link = up.and_then(|a| a.link(game));
        }
    }

    /// One look at the battle at `now`: which situations hold.
    fn step(&mut self, now: f32, holding: [bool; 7]) {
        let dt = self.last.map_or(0.0, |l| (now - l).max(0.0));
        self.last = Some(now);
        // (Said before the battle's clock was read: from now.)
        if let Some((a, since)) = self.shown
            && since == f32::NEG_INFINITY
        {
            self.shown = Some((a, now));
        }
        for (held, &on) in self.held.iter_mut().zip(&holding) {
            *held = if on { *held + dt } else { 0.0 };
        }
        // The advice up goes when its time is up, or a moment after it is put right.
        if let Some((a, since)) = self.shown
            && (now - since > SHOWN || !holding[a.index()] && now - since > 3.0)
        {
            self.shown = None;
            self.quiet_from = now;
        }
        // (The Commander hurt does not wait its turn: it takes the place of other advice, and
        // does not wait out the quiet after one.)
        let hurt = Advice::CommanderHurt.index();
        if now >= OPENING && !self.said[hurt] && holding[hurt] && self.held[hurt] >= Advice::CommanderHurt.after()
            && self.shown.is_none_or(|(a, _)| a != Advice::CommanderHurt)
        {
            self.show(Advice::CommanderHurt, now);
            return;
        }
        if self.shown.is_some() || now < OPENING || now - self.quiet_from < QUIET && self.quiet_from > 0.0 {
            return;
        }
        if let Some(a) = Advice::ALL.into_iter().find(|a| !self.said[a.index()] && holding[a.index()] && self.held[a.index()] >= a.after()) {
            self.show(a, now);
        }
    }

    /// The advice up, low in the battlefield `view`: a plate of the HUD's metal with the coach's
    /// lamp and the words (keys filled in), a LOOK chip when it points somewhere, a cross to put it
    /// away, and a line to hear no more of it; along its foot, the time it has left (which stands
    /// still while the pointer is on it). Whether the player asked never to hear it again comes
    /// back (to be kept in the settings).
    pub fn draw(&mut self, ctx: &egui::Context, game: &mut Game, view: egui::Rect) -> Option<Advice> {
        let (advice, since) = self.shown?;
        if std::mem::take(&mut self.chime) {
            ta_ui::sound::play(ctx, ta_ui::sound::UiSound::Message);
        }
        let now = game.sim.tick as f32 / 30.0;
        let age = now - since;
        let still = ta_ui::motion::reduced(ctx);
        let opacity = if still { 1.0 } else { (age / 0.5).min((SHOWN - age) / 1.0).clamp(0.0, 1.0) };
        let rise = if still { 0.0 } else { (1.0 - (age / 0.5).clamp(0.0, 1.0)).powi(2) * 10.0 };
        let words = crate::keys::fill(advice.text(), game.keys.config(), game.order_button);
        let plate = Plate {
            id: "coach",
            lamp: crate::theme::GREEN,
            foot: crate::theme::GREEN.gamma_multiply(0.7),
            caption: n("COACH"),
            words: &words,
            linked: self.link.is_some(),
            away: &t("Put it away (the coach can be turned off in Options, Gameplay)"),
            never: &t("Don't show this advice again"),
            opacity,
            rise,
            left: ((SHOWN - age) / SHOWN).clamp(0.0, 1.0),
        };
        let shown = hint::show(ctx, view, &plate);
        // (Its time stands still while the pointer is on it: there is time to read, and to click.)
        let dt = self.drawn.map_or(0.0, |d| (now - d).max(0.0));
        self.drawn = Some(now);
        if shown.on && dt > 0.0 {
            self.shown = Some((advice, since + dt));
        }
        let mut muted = None;
        match shown.pressed {
            Some(Pressed::Look) => {
                if let Some(link) = self.link {
                    game.follow(link);
                }
            }
            Some(Pressed::Away) => {
                self.shown = None;
                self.quiet_from = now;
            }
            Some(Pressed::Never) => {
                self.muted.push(advice.name().to_string());
                self.shown = None;
                self.quiet_from = now;
                muted = Some(advice);
            }
            None => {}
        }
        if !still {
            ctx.request_repaint();
        }
        muted
    }

    /// The names of the advice the player asked never to hear again.
    pub fn muted(&self) -> &[String] {
        &self.muted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holding(a: &[Advice]) -> [bool; 7] {
        Advice::ALL.map(|x| a.contains(&x))
    }

    /// A situation must last its while, after the opening; each is said once; one at a time,
    /// with a quiet spell between two; advice put right goes soon after.
    #[test]
    fn the_coach_speaks_once_and_in_its_time() {
        let (open, quiet) = (OPENING as i32, QUIET as i32);
        let mut c = Coach::new(&[]);
        // Energy runs dry in the opening: not yet.
        for s in 0..open {
            c.step(s as f32, holding(&[Advice::EnergyDry]));
        }
        assert!(c.shown.is_none(), "the opening is the start tip's");
        // Past the opening, having held six seconds and more.
        c.step(open as f32, holding(&[Advice::EnergyDry]));
        assert_eq!(c.shown.map(|s| s.0), Some(Advice::EnergyDry));
        // Put right at once: gone a few seconds later.
        let put_right = open + 1;
        for s in put_right..=put_right + 3 {
            c.step(s as f32, holding(&[]));
        }
        assert!(c.shown.is_none());
        let quiet_from = put_right + 3;
        // Metal runs dry, and energy again: a quiet spell first, then the metal (energy was said
        // already).
        for s in quiet_from + 1..quiet_from + quiet {
            c.step(s as f32, holding(&[Advice::EnergyDry, Advice::MetalDry]));
        }
        assert!(c.shown.is_none(), "a quiet spell between two");
        c.step((quiet_from + quiet) as f32, holding(&[Advice::EnergyDry, Advice::MetalDry]));
        assert_eq!(c.shown.map(|s| s.0), Some(Advice::MetalDry));
        // The most pressing first: a hurt Commander, then (its time up, a quiet spell) an idle
        // factory.
        let mut c = Coach::new(&[]);
        for s in 0..=open {
            c.step(s as f32, holding(&[Advice::FactoryIdle, Advice::CommanderHurt]));
        }
        assert_eq!(c.shown.map(|s| s.0), Some(Advice::CommanderHurt));
        let mut s = open + 1;
        while c.shown.map(|a| a.0) != Some(Advice::FactoryIdle) && s < open + 150 {
            c.step(s as f32, holding(&[Advice::FactoryIdle, Advice::CommanderHurt]));
            s += 1;
        }
        assert_eq!(c.shown.map(|a| a.0), Some(Advice::FactoryIdle));
        // (Its predecessor's time up, then a quiet spell.)
        assert!(s as f32 - 1.0 >= open as f32 + SHOWN + QUIET, "the quiet spell between two: said at {}", s - 1);
        // A paused battle's clock stands still: nothing more held.
        let held = c.held;
        c.step((s - 1) as f32, holding(&[Advice::FactoryIdle, Advice::CommanderHurt]));
        assert_eq!(c.held, held);
    }

    #[test]
    fn the_commander_hurt_does_not_wait_its_turn() {
        let open = OPENING as i32;
        let mut c = Coach::new(&[]);
        for s in 0..=open + 4 {
            c.step(s as f32, holding(&[Advice::EnergyDry]));
        }
        assert_eq!(c.shown.map(|s| s.0), Some(Advice::EnergyDry));
        // The Commander hurt while other advice is up: it takes its place.
        let from = open + 5;
        let mut s = from;
        while c.shown.map(|s| s.0) != Some(Advice::CommanderHurt) && s < from + 40 {
            c.step(s as f32, holding(&[Advice::EnergyDry, Advice::CommanderHurt]));
            s += 1;
        }
        assert_eq!(c.shown.map(|s| s.0), Some(Advice::CommanderHurt), "said at once, not after the energy");
        assert!(s - from <= Advice::CommanderHurt.after().ceil() as i32 + 1);
    }

    #[test]
    fn advice_is_named_for_captures_and_names_bound_keys() {
        let keys = ta_config::Keys::default();
        for a in Advice::ALL {
            assert_eq!(Advice::named(a.name()), Some(a));
            assert!(!crate::keys::fill(a.text(), &keys, ta_config::OrderButton::Right).contains('{'), "{}", a.text());
        }
    }

    /// What the player asked never to hear again is not said, and does not hold up what is:
    /// the next advice speaks in its place.
    #[test]
    fn muted_advice_is_never_said_and_the_next_speaks() {
        let open = OPENING as i32;
        let muted = vec![Advice::EnergyDry.name().to_string()];
        let mut c = Coach::new(&muted);
        assert!(c.is_muted(Advice::EnergyDry) && !c.is_muted(Advice::MetalDry));
        // (`update` never holds what is muted: the same here.)
        let holds = |c: &Coach| Advice::ALL.map(|a| !c.is_muted(a) && [Advice::EnergyDry, Advice::MetalDry].contains(&a));
        for s in 0..=open + 10 {
            let now = holds(&c);
            c.step(s as f32, now);
        }
        assert_eq!(c.shown.map(|s| s.0), Some(Advice::MetalDry), "the energy is muted, the metal is said");
        assert_eq!(c.muted(), &muted[..]);
    }
}
