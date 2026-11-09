//! Short tips shown while a battle loads and at the start of a skirmish. Keys appear as
//! `{action}` and show whatever they are bound to (`keys::fill`). The tip comes a moment into
//! the battle, stays a few seconds, goes pale when the pointer comes near, can be put away, and
//! tips can be turned off for good (`ta_config::Gameplay::start_tips`).

use ta_config::{Gameplay, OrderButton, RadialOpens};
use ta_ui::i18n::{n, t};

use crate::hint::{self, Plate, Pressed};

/// The controls' settings a tip may only hold for.
#[derive(Clone, Copy, Debug)]
enum When {
    /// The right button gives orders, or the left one does (TA's original scheme).
    RightOrders,
    LeftOrders,
    /// The radial menu is on and opens on a right-click or a hold, or on a hold only.
    RadialClick,
    RadialHold,
}

impl When {
    fn holds(self, p: &Gameplay) -> bool {
        match self {
            When::RightOrders => p.order_button == OrderButton::Right,
            When::LeftOrders => p.order_button == OrderButton::Left,
            When::RadialClick => p.radial_menu && p.radial_opens == RadialOpens::ClickOrHold,
            When::RadialHold => p.radial_menu && p.radial_opens == RadialOpens::Hold,
        }
    }
}

/// Tips that only hold for some of the controls' settings: the button giving orders, how the
/// radial menu opens (none of its tips with the menu off).
const SETTING_TIPS: &[(When, &str)] = &[
    (When::RightOrders, n("Ctrl+right-drag draws a formation line: the selection spreads along it.")),
    (When::LeftOrders, n("Ctrl+drag draws a formation line: the selection spreads along it.")),
    (When::RightOrders, n("Used to TA's original controls? Options, Controls: give orders with the left button.")),
    (When::LeftOrders, n("A right-click clears the selection or cancels the order you were about to give.")),
    (When::RadialClick, n("Right-click one of your units for a ring of its orders; hold the button and flick towards a choice. A controller's B does the same.")),
    (When::RadialHold, n("Hold the right button on one of your units, or middle-click it, for a ring of its orders; flick towards a choice and let go.")),
];

const TIPS: &[&str] = &[
    n("Metal extractors must sit on metal: {overlay_metal} shows the metal map, and {area_mex} puts an extractor on every metal spot you circle."),
    n("Your Commander's D-gun ({dgun}) destroys almost anything it touches, including your own units in its path."),
    n("{perspective} climbs into the selected unit, or an ally's under the pointer, to see the battle through its eyes as it goes on with its orders. Escape climbs back out."),
    n("Keep your metal and energy incomes above what you spend: building slows down for everything when a store runs dry."),
    n("Right-clicking a factory's queued unit removes it; clicking it builds it next."),
    n("Fight ({fight}) is attack-move: units stop to fight whatever they meet on the way."),
    n("Ctrl+click is force fire: the selection attacks whatever is there, your own units too, or the ground."),
    n("{select_builders} selects your constructors, {select_factories} your factories, {select_armed} your armed units, {select_on_screen} everything on the screen."),
    n("Patrol ({patrol}) with constructors makes them repair, assist and reclaim along the route by themselves."),
    n("Put a constructor on duty ({duty}) and it works by itself: the economy, repairs, reclaiming or defence. Any order you give takes it back."),
    n("Drag with Reclaim ({reclaim}) to turn every wreck, rock and tree in a circle into metal and energy."),
    n("Ctrl+1 to Ctrl+9 make control groups; tap the number twice to look at the group."),
    n("Hold {players} to see every player's side, team, units and income (your allies' in full)."),
    n("Radar towers show enemy units long before they come into sight: build one early."),
    n("Wind generators pay off on windy maps: the skirmish page shows each map's wind range."),
    n("Metal makers turn spare energy into metal, and stop when your energy runs dry."),
    n("Solar collectors fold shut for a few seconds when hit: tougher, but making no energy."),
    n("Units gain veterancy with every kill: veterans aim better and shrug off damage."),
    n("Shift adds to the orders a unit already has instead of replacing them."),
    n("Alt+drag while placing a building fills a rectangle with it: handy for solar farms and walls."),
    n("Double-click a unit to select every unit of its type on screen; {select_type} selects them map-wide."),
    n("{idle_builder} selects the next idle constructor, {idle_factory} the next idle factory."),
    n("Wrecks can be resurrected by resurrection kbots, or reclaimed for most of their metal."),
    n("Hovering a unit shows its health, firepower and range; {overlay_path} shows where the selection can go."),
    n("{last_alert} (or Backspace) looks at the last place your units were attacked; press it again for the one before."),
    n("A notice or a piece of advice with LOOK on it takes you to the place, or the unit, when you click it."),
    n("{help} tells everything about the selected unit, or the one under the pointer: cost, speed, weapons, abilities."),
    n("{next_unit} looks at the next of your units off the screen; with units selected, the next of them."),
    n("Ctrl+F6 to Ctrl+F9 store camera positions; F6 to F9 bring them back."),
    n("Anti-air matters: bombers can wreck an undefended economy in one pass."),
    n("An advanced factory needs a lot of metal: build it once your extractors and power can feed it."),
    n("Guarding a factory with a constructor adds its build power to the factory's production."),
    n("Moho mines extract far more metal than basic extractors: advanced constructors build them."),
    n("Every game is kept as a replay: watch it from the main menu, and jump through it with the bar."),
    n("{overlay_sight} shows your sight, radar and sonar coverage; {overlay_height} shows the heights of the map."),
    n("The Commander can capture enemy units ({capture}): it takes a while, but the unit is yours."),
    n("{photo_mode} opens photo mode: the battle holds still, and you can light, frame and focus a shot at up to 4K."),
];

/// How long the tip at the start of a skirmish stays (ticks of game time), and the seconds it
/// leaves the battle's first moments alone.
pub const SHOWN_TICKS: u64 = 30 * 12;
const COMES: f32 = 2.5;

/// A tip picked by `seed` (e.g. the time), among those that hold for the player's controls (the
/// button giving orders, the radial menu).
pub fn pick(seed: u64, controls: &Gameplay) -> &'static str {
    let mouse: Vec<&str> = SETTING_TIPS.iter().filter(|(when, _)| when.holds(controls)).map(|(_, t)| *t).collect();
    let i = seed as usize % (TIPS.len() + mouse.len());
    TIPS.get(i).copied().unwrap_or_else(|| mouse[i - TIPS.len()])
}

/// The tip at the start of a skirmish (translated, its keys filled in), low in the middle of
/// `view`: it comes after a couple of seconds, fading in, and goes at the end; pale when the
/// pointer is near. What the player did to it comes back: put it away (`Away`), or no more tips
/// (`Never`).
pub fn strip(ctx: &egui::Context, tip: &str, tick: u64, view: egui::Rect) -> Option<Pressed> {
    use crate::theme::*;
    let secs = tick as f32 / 30.0;
    let end = SHOWN_TICKS as f32 / 30.0;
    let opacity = ((secs - COMES) / 0.6).min((end - secs) / 1.0).clamp(0.0, 1.0);
    if opacity <= 0.0 {
        return None;
    }
    // (It rises a little as it fades in.)
    let rise = (1.0 - ((secs - COMES) / 0.6).clamp(0.0, 1.0)).powi(2) * 10.0;
    let plate = Plate {
        id: "start_tip",
        lamp: AMBER,
        foot: AMBER_DIM,
        caption: n("TIP"),
        words: tip,
        linked: false,
        away: &t("Dismiss"),
        never: &t("Don't show tips again"),
        opacity,
        rise,
        left: ((end - secs) / (end - COMES)).clamp(0.0, 1.0),
    };
    let shown = hint::show(ctx, view, &plate);
    ctx.request_repaint();
    shown.pressed
}

#[cfg(test)]
mod tests {
    #[test]
    fn tips_are_short_enough_for_one_panel() {
        let keys = ta_config::Keys::default();
        for t in super::TIPS.iter().chain(super::SETTING_TIPS.iter().map(|(_, t)| t)) {
            assert!(crate::keys::fill(t, &keys, ta_config::OrderButton::Right).len() < 160, "{t}");
        }
    }

    /// Every `{placeholder}` names a key binding, so the tips follow the player's keys.
    #[test]
    fn tips_name_bound_keys() {
        let mut keys = ta_config::Keys::default();
        for t in super::TIPS.iter().chain(super::SETTING_TIPS.iter().map(|(_, t)| t)) {
            let mut rest = *t;
            while let Some(start) = rest.find('{') {
                let len = rest[start..].find('}').expect(t);
                let action = &rest[start + 1..start + len];
                assert!(ta_config::Keys::ACTIONS.iter().any(|&(id, _, _)| id == action), "{action} in {t}");
                rest = &rest[start + len + 1..];
            }
            assert!(!crate::keys::fill(t, &keys, ta_config::OrderButton::Right).contains('{'), "{t}");
        }
        keys.set("dgun", "Ctrl+G");
        let tip = super::TIPS.iter().find(|t| t.contains("{dgun}")).unwrap();
        assert!(crate::keys::fill(tip, &keys, ta_config::OrderButton::Right).contains("D-gun (Ctrl+G)"));
    }

    /// Every tip that can come up for these controls.
    fn all(controls: &ta_config::Gameplay) -> std::collections::BTreeSet<&'static str> {
        (0..200).map(|s| super::pick(s, controls)).collect()
    }

    #[test]
    fn mouse_tips_match_the_order_button() {
        use ta_config::OrderButton::{Left, Right};
        let with = |order_button| all(&ta_config::Gameplay { order_button, ..Default::default() });
        assert!(with(Right).iter().any(|t| t.contains("right-drag")) && !with(Right).iter().any(|t| t.starts_with("Ctrl+drag")));
        assert!(with(Left).iter().any(|t| t.starts_with("Ctrl+drag")) && !with(Left).iter().any(|t| t.contains("right-drag")));
    }

    /// The radial menu's tip says how the player's setting opens it, and none comes up with the
    /// menu off.
    #[test]
    fn the_radial_menus_tip_follows_its_setting() {
        use ta_config::RadialOpens::{ClickOrHold, Hold};
        let with = |radial_menu, radial_opens| -> Vec<&'static str> {
            all(&ta_config::Gameplay { radial_menu, radial_opens, ..Default::default() }).into_iter().filter(|t| t.contains("ring of its orders")).collect()
        };
        assert!(matches!(with(true, ClickOrHold)[..], [t] if t.starts_with("Right-click one of your units")));
        assert!(matches!(with(true, Hold)[..], [t] if t.starts_with("Hold the right button")));
        assert!(with(false, ClickOrHold).is_empty() && with(false, Hold).is_empty());
    }

    /// Battle after battle, a tip does not come again until all the others have: the seeds a
    /// session goes through (one more each battle) give every tip once in a round.
    #[test]
    fn tips_come_round_without_repeating() {
        use ta_config::OrderButton::{Left, Right};
        use ta_config::RadialOpens::{ClickOrHold, Hold};
        for order_button in [Left, Right] {
            for (radial_menu, radial_opens) in [(true, ClickOrHold), (true, Hold), (false, ClickOrHold)] {
                let controls = ta_config::Gameplay { order_button, radial_menu, radial_opens, ..Default::default() };
                let round = super::TIPS.len() + super::SETTING_TIPS.iter().filter(|(when, _)| when.holds(&controls)).count();
                for start in [0u64, 7, 1_700_000_123] {
                    let seen: std::collections::BTreeSet<_> = (start..start + round as u64).map(|s| super::pick(s, &controls)).collect();
                    assert_eq!(seen.len(), round, "a round of {round} tips from {start}");
                }
            }
        }
    }

    /// It comes a moment into the battle, not in its first seconds, and is gone by the end.
    #[test]
    fn the_tip_keeps_clear_of_the_first_moments() {
        let ctx = egui::Context::default();
        crate::theme::install_compact(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0));
        let drawn = |tick: u64| {
            let mut stood = false;
            let input = egui::RawInput { screen_rect: Some(screen), time: Some(tick as f64 / 30.0), ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| {
                super::strip(ui.ctx(), "A tip", tick, screen);
                stood = ui.ctx().memory(|m| m.area_rect(egui::Id::new("start_tip"))).is_some();
            });
            output.textures_delta.clear();
            output.drop_without_applying_deltas();
            stood
        };
        assert!(!drawn(30), "not in the first seconds");
        assert!(drawn(30 * 6), "on show in the middle of its time");
    }
}
