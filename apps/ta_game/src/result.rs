//! The battle's result. The interface steps aside and the battle is graded down behind its
//! title (VICTORY, DEFEAT, a mission's, a siege's), with what decided it under the title; under
//! that the debrief: a summary (the player's figures, their best unit, the battle's key moments
//! and what it did for the career), every player's totals, and their graphs; then what next.
//! It comes in after a beat (the interface gone, the camera on its way to where the battle was
//! decided): the title settles, the debrief rises, the figures count up (all at once with
//! reduced motion).

use std::f32::consts::TAU;

use egui::emath::TSTransform;
use egui::{Align2, Color32, CornerRadius, Id, Order, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use ta_ui::i18n::{n, t, tf, upper};
use ta_ui::panel::{self, Panel};
use ta_ui::widgets::{self as w, ActionButton};

use crate::chronicle::Best;
use crate::game::{Game, Outcome};
use crate::hud::UnitPic;
use crate::stats::{big, clock};
use crate::theme::*;

/// The debrief's pages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Summary,
    Players,
    Graphs,
}

/// What the player chose to do next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    /// The main menu (or the editor that waited for this battle).
    Menu,
    /// The same battle again: a skirmish's rematch, a scenario's or a lost mission's retry.
    Again,
    /// The campaign's page at this mission: the next after a victory, this one's briefing after
    /// a defeat.
    Campaign { campaign: String, mission: usize },
    Scenarios,
    Conquest,
    Highlights,
    /// A defeat in a battle that goes on: watched through everyone's eyes.
    KeepWatching,
    /// Multiplayer: back to the lobby with the same people (the host takes everyone; a player
    /// follows the host).
    Lobby,
}

/// The ways on from the result, as the battle allows them (worked out before the panel is
/// drawn: its footer has the game to itself only so).
struct Ways {
    highlights: bool,
    highlights_available: bool,
    /// A skirmish of the player's own (a rematch).
    skirmish: bool,
    scenario: bool,
    conquest: bool,
    mission: Option<(String, usize)>,
    /// Others fight on after a defeat.
    watch: bool,
    from_editor: bool,
    /// Multiplayer: back to the lobby now (true), or once the host has gone back (false).
    lobby: Option<bool>,
    /// The way home's name (see [`way_home`]).
    home: &'static str,
}

/// The narrowest window (points) whose result has room for a page's name on its way home beside
/// WATCH HIGHLIGHTS, KEEP WATCHING and REMATCH, in the longer translations too.
const ROOM_FOR_A_NAME: f32 = 1000.0;

/// The name of a result's way home, `back` the battle menu's way out (named after the page the
/// battle came from, where leaving goes: the skirmish setup, the saved games, the title menu). A
/// skirmish's result goes back there too, and says so as the battle menu does (it said MAIN MENU
/// and went back to the skirmish setup); in a window too narrow for the page's name (`roomy`
/// false), BACK, as the pages' own way out says. The other battles' say MAIN MENU beside their
/// own ways on (the campaign's, the scenarios', Conquest's).
fn way_home(skirmish: bool, back: &'static str, roomy: bool) -> &'static str {
    match (skirmish, roomy) {
        (false, _) => n("MAIN MENU"),
        (true, false) if back != n("MAIN MENU") => n("BACK"),
        (true, _) => back,
    }
}

/// Whether a result's MAIN MENU takes the menu to its title page, as it says. The menu keeps the
/// page a battle came from, so MAIN MENU went back there: to the campaign page after a mission,
/// the galaxy map after a Conquest battle, the scenarios after one, where the result's own ways
/// on already go (CONTINUE CAMPAIGN or BACK TO BRIEFING, CONTINUE CONQUEST, SCENARIOS). So for a
/// battle with a way back to its page (`own_way_back`) played live (not a replay, multiplayer or
/// the editor's test: `live`), it goes to the title menu. A skirmish's names its page instead
/// ([`way_home`]).
pub fn main_menu_to_title(own_way_back: bool, live: bool) -> bool {
    own_way_back && live
}

/// [`main_menu_to_title`] for a battle.
pub fn main_menu_to_title_after(game: &Game) -> bool {
    let own_way_back = game.setup.mission.is_some() || game.setup.conquest.is_some() || game.scenario.is_some();
    main_menu_to_title(own_way_back, game.playback.is_none() && game.net.is_none() && !game.from_editor)
}

/// The menu page a result's way home leads to, `back_to` the page the battle came from: the title
/// menu for a battle whose MAIN MENU says so (`to_title`, [`main_menu_to_title`]: a mission's,
/// Conquest's, a scenario's, beside their own ways back to their pages), else where the battle
/// came from, as its way home says (see [`way_home`]).
pub fn home_page(to_title: bool, back_to: crate::menu::Page) -> crate::menu::Page {
    if to_title { crate::menu::Page::Title } else { back_to }
}

/// The result screen's state: when it came up, the page shown, and the summary, worked out
/// once (the battle is over).
#[derive(Default)]
pub struct Debrief {
    since: Option<f64>,
    /// The battle's tick when it was decided (the battle goes on behind its result).
    tick: u64,
    tab: Tab,
    summary: Option<Summary>,
}

impl Debrief {
    /// The type of the player's best unit, once the summary is worked out (its picture is the
    /// interface's to draw).
    pub fn best_type(&self) -> Option<usize> {
        self.summary.as_ref()?.best.map(|b| b.type_id)
    }
}

/// The summary's figures and words, and every player's totals and readings as they were when
/// the battle was decided.
struct Summary {
    players: Vec<ta_sim::Player>,
    finals: Vec<Option<ta_sim::StatSample>>,
    figures: Vec<Figure>,
    best: Option<Best>,
    moments: Vec<Moment>,
    /// What decided the battle (under the title).
    verdict: Option<String>,
}

/// One of the player's figures: its caption, its value, how it is written, and whether it is
/// the best of every player's.
struct Figure {
    caption: &'static str,
    value: f64,
    write: fn(f64) -> String,
    lead: bool,
}

/// A key moment: when, what, and its colour (whose it was).
struct Moment {
    tick: u64,
    text: String,
    colour: Color32,
}

/// The title's size, a mission's smaller (its words are longer), on a short screen smaller
/// still.
fn title_size(compact: bool, long: bool) -> f32 {
    (if compact { 48.0 } else { 84.0 }) * if long { 0.75 } else { 1.0 }
}

/// `x` (0 to 1, clamped) eased out.
fn ease(x: f32) -> f32 {
    1.0 - (1.0 - x.clamp(0.0, 1.0)).powi(3)
}

/// The result over the battle, `best_pic` the picture of the player's best unit (see
/// [`Debrief::best_type`]), `back` the battle menu's way out (see [`way_home`]): what the player
/// chose to do next, if anything (watching on is seen to here).
pub fn draw(ctx: &egui::Context, game: &mut Game, best_pic: Option<UnitPic>, back: &'static str) -> Option<Choice> {
    let outcome = game.outcome?;
    let now = ctx.input(|i| i.time);
    if game.debrief.since.is_none() {
        (game.debrief.since, game.debrief.tick) = (Some(now), game.sim.tick);
    }
    let since = game.debrief.since.unwrap_or(now);
    let t = if ta_ui::motion::still(ctx) { 10.0 } else { (now - since) as f32 };
    if t < 3.5 {
        ctx.request_repaint();
    }
    if game.debrief.summary.is_none() {
        game.debrief.summary = Some(summarise(game, outcome));
    }
    let endless = game.scenario.as_ref().is_some_and(|s| s.kind == crate::scenario::Kind::Endless);
    let mission = game.setup.mission.clone();
    let won = outcome == Outcome::Victory && !endless;
    let (title, colour) = match (outcome, mission.is_some()) {
        // (An endless siege always ends in defeat: how long it lasted is the thing.)
        _ if endless => (n("OVERRUN"), RED),
        (Outcome::Victory, false) => (n("VICTORY"), AMBER),
        (Outcome::Defeat, false) => (n("DEFEAT"), RED),
        (Outcome::Victory, true) => (n("MISSION ACCOMPLISHED"), AMBER),
        (Outcome::Defeat, true) => (n("MISSION FAILED"), RED),
    };
    let screen = ctx.content_rect();
    scrim(ctx, screen, won, ease(t / 1.2));

    let compact = screen.height() < 640.0;
    let width = (screen.width() - 32.0).min(1040.0);
    // (Placed from its height as last drawn: centred, or from the top when it fills the screen.)
    let height_id = Id::new("result_height");
    let last = ctx.data(|d| d.get_temp::<f32>(height_id)).unwrap_or(screen.height());
    let top = ((screen.height() - last) / 2.0).max(if compact { 8.0 } else { 16.0 });
    let skirmish = mission.is_none() && game.scenario.is_none() && game.setup.conquest.is_none() && game.net.is_none() && !game.from_editor && game.playback.is_none();
    let ways = Ways {
        // (Not in multiplayer: watching them leaves the battle, the host's for everyone.)
        highlights: game.tally.has_moments() && game.net.is_none(),
        highlights_available: game.highlights_available(),
        skirmish,
        home: way_home(skirmish, back, screen.width() >= ROOM_FOR_A_NAME),
        scenario: game.scenario.is_some(),
        conquest: game.setup.conquest.is_some(),
        mission: mission.as_ref().map(|m| (m.campaign.clone(), m.index)),
        // (In multiplayer, while the battle goes on for others.)
        // (Sides, not players: two computer players of one team are no battle left to watch.)
        watch: !crate::lobby::battle_over(game) && !endless,
        from_editor: game.from_editor,
        lobby: game.net.as_ref().filter(|n| !n.host_lost).and_then(|n| match n.is_host() {
            // (The host, once the battle is over for everyone: others may still be fighting.)
            true => crate::lobby::battle_over(game).then_some(true),
            false => Some(n.rematch),
        }),
    };
    let mut choice = None;
    let shown = egui::Area::new(Id::new("result")).order(Order::Foreground).anchor(Align2::CENTER_TOP, [0.0, top]).show(ctx, |ui| {
        ui.set_width(width);
        ui.vertical_centered(|ui| {
            banner(ui, game, title, colour, mission.is_some(), compact, won, t, now as f32);
        });
        ui.add_space(if compact { 6.0 } else { 14.0 });
        // The debrief rises into place under the title.
        let rise = ease((t - 1.3) / 0.5);
        ui.multiply_opacity(rise);
        let lift = TSTransform::from_translation(vec2(0.0, 24.0 * (1.0 - rise)));
        ui.with_visual_transform(lift, |ui| {
            let room = screen.bottom() - (if compact { 8.0 } else { 16.0 }) - ui.cursor().top();
            let page = Panel::new("").width(width - 48.0).max_height(room.max(220.0)).accent(colour);
            let response = page.show_with_footer(ui, 40.0, |ui| debrief(ui, game, best_pic, t, compact), |ui| footer(ui, &ways, outcome));
            choice = response.inner.1;
        });
    });
    let height = shown.response.rect.height();
    if (height - last).abs() > 0.5 {
        ctx.data_mut(|d| d.insert_temp(height_id, height));
    }
    if choice == Some(Choice::KeepWatching) {
        // (Our team fighting on: its victory will be ours.)
        let me = game.local_player;
        game.watching_team = game.sim.players.iter().any(|p| p.index != me && game.sim.allied(p.index, me) && !p.defeated);
        game.spectator = true;
        game.view_as = None;
        game.outcome = None;
        game.debrief = Debrief::default();
        return None;
    }
    choice
}

/// The battle dimmed behind the result, more for a defeat, and the input held off it.
fn scrim(ctx: &egui::Context, screen: Rect, won: bool, a: f32) {
    egui::Area::new(Id::new("result_scrim")).order(Order::Middle).fixed_pos(screen.min).show(ctx, |ui| {
        let (rect, _) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
        let p = ui.painter();
        let dim = |alpha: f32| Color32::from_black_alpha((alpha * a) as u8);
        p.rect_filled(rect, 0.0, dim(if won { 70.0 } else { 110.0 }));
        w::gradient_v(p, Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + rect.height() * 0.35)), dim(150.0), dim(0.0));
        w::gradient_v(p, Rect::from_min_max(pos2(rect.min.x, rect.max.y - rect.height() * 0.45), rect.max), dim(0.0), dim(170.0));
    });
}

/// A soft glow of `colour` round `centre`, `radii` across, fading to nothing at its edge.
fn glow(painter: &egui::Painter, centre: Pos2, radii: Vec2, colour: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(centre, colour);
    const SIDES: u32 = 64;
    // (A ring at half the radius, half as bright: the light falls off softly.)
    for (ring, shade) in [(0.5, colour.gamma_multiply(0.45)), (1.0, Color32::TRANSPARENT)] {
        for i in 0..SIDES {
            let a = i as f32 / SIDES as f32 * TAU;
            mesh.colored_vertex(centre + vec2(a.cos() * radii.x, a.sin() * radii.y) * ring, shade);
        }
    }
    for i in 0..SIDES {
        let j = (i + 1) % SIDES;
        mesh.add_triangle(0, 1 + i, 1 + j);
        mesh.add_triangle(1 + i, 1 + SIDES + i, 1 + j);
        mesh.add_triangle(1 + j, 1 + SIDES + i, 1 + SIDES + j);
    }
    painter.add(mesh);
}

/// Rays of light turning slowly round `centre` (a victory's), `reach` long.
fn rays(painter: &egui::Painter, centre: Pos2, reach: Vec2, colour: Color32, turn: f32) {
    const RAYS: usize = 14;
    let mut mesh = egui::Mesh::default();
    for k in 0..RAYS {
        let a = turn + k as f32 / RAYS as f32 * TAU;
        let half = TAU / RAYS as f32 * 0.18;
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(centre, colour);
        for edge in [a - half, a + half] {
            mesh.colored_vertex(centre + vec2(edge.cos() * reach.x, edge.sin() * reach.y), Color32::TRANSPARENT);
        }
        mesh.add_triangle(base, base + 1, base + 2);
    }
    painter.add(mesh);
}

/// The title coming in over its glow (rays of light behind a victory), what decided the battle,
/// where and how long it was, and whether cheats were used.
#[allow(clippy::too_many_arguments)]
fn banner(ui: &mut egui::Ui, game: &Game, title: &'static str, colour: Color32, long: bool, compact: bool, won: bool, t: f32, now: f32) {
    let size = title_size(compact, long);
    let come = ease((t - 0.6) / 0.6);
    let centre = pos2(ui.max_rect().center().x, ui.cursor().top() + size * 0.55);
    let painter = ui.painter().clone();
    let reach = vec2(size * 5.5, size * 1.6);
    if won {
        rays(&painter, centre, reach * 1.1, colour.gamma_multiply(0.08 * come), if ta_ui::motion::still(ui.ctx()) { 0.0 } else { now * 0.04 });
        if !ta_ui::motion::still(ui.ctx()) {
            ui.ctx().request_repaint();
        }
    }
    glow(&painter, centre, reach, colour.gamma_multiply(0.22 * come));
    // (Settling from a little larger.)
    let scale = 1.0 + 0.12 * (1.0 - come);
    let settle = TSTransform::new((centre.to_vec2()) * (1.0 - scale), scale);
    ui.scope(|ui| {
        ui.multiply_opacity(come);
        ui.with_visual_transform(settle, |ui| panel::glow_title(ui, title, size, colour));
    });
    let words = ease((t - 0.9) / 0.5);
    ui.scope(|ui| {
        ui.multiply_opacity(words);
        ui.spacing_mut().item_spacing.y = 3.0;
        // The words on a soft dark pool of their own: the small captions in steel were muddy
        // over a bright battlefield (a snowfield, a desert at noon).
        let pool = ui.painter().add(egui::Shape::Noop);
        let mut said = Rect::NOTHING;
        if let Some(verdict) = game.debrief.summary.as_ref().and_then(|s| s.verdict.clone()) {
            said = said.union(shadowed(ui, RichText::new(verdict).font(strong(if compact { 15.0 } else { 18.0 })).color(CREAM)));
        }
        for (line, tint) in captions(game) {
            said = said.union(shadowed(ui, w::caption(&line).color(tint)));
        }
        if game.cheated() {
            said = said.union(shadowed(ui, w::caption("CHEATS USED").color(RED)));
        }
        if said.is_positive() {
            ui.painter().set(pool, panel::scrim(said.expand2(vec2(14.0, 2.0))));
        }
    });
}

/// A line over the battle, its words shadowed so that they read over any ground (centred, and
/// wrapping where it runs past the width): where they are.
fn shadowed(ui: &mut egui::Ui, text: RichText) -> Rect {
    let mut job = std::sync::Arc::unwrap_or_clone(egui::WidgetText::from(text).into_layout_job(ui.style(), egui::FontSelection::Default, egui::Align::Center));
    job.wrap.max_width = ui.available_width();
    job.halign = egui::Align::Center;
    let galley = ui.painter().layout_job(job);
    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
    // (A centred galley's lines are laid out round its middle.)
    let at = pos2(rect.center().x, rect.min.y);
    let p = ui.painter();
    for (dx, dy) in [(1.5, 2.0), (0.0, 1.0), (-1.0, 0.5), (1.0, 0.5)] {
        p.galley_with_override_text_color(at + vec2(dx, dy), galley.clone(), Color32::from_black_alpha(170));
    }
    p.galley(at, galley, CREAM);
    rect
}

/// The lines under the verdict: where and how long, a scenario's or a conquest's own.
fn captions(game: &Game) -> Vec<(String, Color32)> {
    let time = clock(game.debrief.tick);
    let endless = game.scenario.as_ref().is_some_and(|s| s.kind == crate::scenario::Kind::Endless);
    let won = game.outcome == Some(Outcome::Victory);
    match &game.scenario {
        Some(s) if endless => vec![(tf("{title} · HELD OUT FOR {clock}", &[("title", &upper(t(&s.title))), ("clock", &time)]), AMBER_BRIGHT)],
        Some(s) => vec![(tf("{title} · BATTLE TIME {clock}", &[("title", &upper(t(&s.title))), ("clock", &time)]), STEEL)],
        // A Conquest battle: the system taken or holding out, and where its report is.
        None if let Some(cs) = &game.setup.conquest => {
            let name = upper(&game.map_title);
            let args: &[(&str, &dyn std::fmt::Display)] = &[("name", &name), ("clock", &time)];
            let caption = match (cs.defence, won) {
                (true, true) => tf("{name} HELD · BATTLE TIME {clock}", args),
                (true, false) => tf("{name} LOST · BATTLE TIME {clock}", args),
                (false, true) => tf("{name} TAKEN · BATTLE TIME {clock}", args),
                (false, false) => tf("{name} HOLDS · BATTLE TIME {clock}", args),
            };
            vec![(caption, if won { AMBER_BRIGHT } else { TEXT }), (t("Continue the conquest for the battle's report.").to_string(), STEEL)]
        }
        None if game.setup.mission.is_some() => vec![(tf("BATTLE TIME {clock}", &[("clock", &time)]), STEEL)],
        None => vec![(tf("{title} · BATTLE TIME {clock}", &[("title", &upper(&game.map_title)), ("clock", &time)]), STEEL)],
    }
}

/// The battle's summary for the player: their figures, their best unit, its key moments and
/// what decided it.
fn summarise(game: &Game, outcome: Outcome) -> Summary {
    let sim = &game.sim;
    let me = game.local_player;
    let players = &sim.players;
    let mine = |p: &ta_sim::Player| -> [f64; 5] {
        let s = &p.stats;
        [s.built as f64, s.lost as f64, s.kills as f64, s.metal_produced.to_f32() as f64, s.energy_produced.to_f32() as f64]
    };
    let figures_of: Vec<[f64; 5]> = players.iter().map(mine).collect();
    let own = figures_of.get(me as usize).copied().unwrap_or_default();
    // (The best of every player's: the most, but losing the fewest; shared, no one's.)
    let lead = |k: usize| {
        let values: Vec<f64> = figures_of.iter().map(|f| f[k]).collect();
        let pick = if k == 1 { values.iter().copied().fold(f64::MAX, f64::min) } else { values.iter().copied().fold(f64::MIN, f64::max) };
        players.len() > 1 && own[k] == pick && values.iter().filter(|&&v| v == pick).count() == 1 && (k == 1 || pick > 0.0)
    };
    let count = |v: f64| format!("{:.0}", v);
    let amount = |v: f64| big(ta_sim::Fx::from_f32(v as f32));
    let figures = vec![
        Figure { caption: n("BATTLE TIME"), value: sim.tick as f64, write: |v| clock(v as u64), lead: false },
        Figure { caption: n("BUILT"), value: own[0], write: count, lead: lead(0) },
        Figure { caption: n("LOST"), value: own[1], write: count, lead: lead(1) },
        Figure { caption: n("KILLS"), value: own[2], write: count, lead: lead(2) },
        Figure { caption: n("METAL MADE"), value: own[3], write: amount, lead: lead(3) },
        Figure { caption: n("ENERGY MADE"), value: own[4], write: amount, lead: lead(4) },
    ];
    let chronicle = &game.tally.chronicle;
    let best = chronicle.best(me, sim);
    // The key moments, in the order they happened.
    let colour_of = |player: u8| players.get(player as usize).map_or(STEEL, |p| crate::palette::team_color(p.color));
    let mut moments = Vec::new();
    if let Some(tick) = chronicle.first_blood {
        moments.push(Moment { tick, text: t("First blood").to_string(), colour: AMBER });
    }
    for m in &chronicle.commanders {
        let text = if m.player == me { t("You lost your Commander").to_string() } else { tf("{player} lost their Commander", &[("player", &game.player_label(m.player))]) };
        moments.push(Moment { tick: m.tick, text, colour: colour_of(m.player) });
    }
    for m in chronicle.launches.iter().take(3) {
        let text = if m.player == me { t("You launched a nuclear missile").to_string() } else { tf("{player} launched a nuclear missile", &[("player", &game.player_label(m.player))]) };
        moments.push(Moment { tick: m.tick, text, colour: colour_of(m.player) });
    }
    if game.tally.has_moments()
        && let Some(fight) = game.tally.moments().into_iter().filter(|m| m.kind == crate::highlights::MomentKind::Battle).max_by(|a, b| a.worth.total_cmp(&b.worth))
    {
        moments.push(Moment { tick: fight.start, text: tf("The largest battle: {n} units destroyed", &[("n", &fight.units)]), colour: STEEL });
    }
    moments.sort_by_key(|m| m.tick);
    // (The end, last.)
    let endless = game.scenario.as_ref().is_some_and(|s| s.kind == crate::scenario::Kind::Endless);
    let end = match (outcome, game.setup.mission.is_some()) {
        _ if endless => n("OVERRUN"),
        (Outcome::Victory, false) => n("VICTORY"),
        (Outcome::Defeat, false) => n("DEFEAT"),
        (Outcome::Victory, true) => n("MISSION ACCOMPLISHED"),
        (Outcome::Defeat, true) => n("MISSION FAILED"),
    };
    moments.push(Moment { tick: sim.tick, text: t(end).to_string(), colour: if outcome == Outcome::Victory && !endless { AMBER_BRIGHT } else { RED } });
    Summary { players: players.clone(), finals: crate::stats::finals(game), figures, best, moments, verdict: verdict(game, outcome) }
}

/// What decided the battle, in a sentence: a skirmish's fallen Commanders (or forces), a
/// mission's name. None where the captions say it (scenarios, Conquest).
fn verdict(game: &Game, outcome: Outcome) -> Option<String> {
    if game.scenario.is_some() || game.setup.conquest.is_some() {
        return None;
    }
    if let Some(m) = game.mission.as_ref().filter(|m| !m.name.is_empty()) {
        return Some(crate::campaign::strip_number(&m.name).to_string());
    }
    let sim = &game.sim;
    let me = game.local_player;
    let fallen = |player: u8| game.tally.chronicle.commanders.iter().find(|m| m.player == player);
    match outcome {
        Outcome::Victory if game.team_won => Some(t("Your team won the battle").to_string()),
        Outcome::Victory => {
            let enemies: Vec<u8> = sim.players.iter().filter(|p| sim.is_enemy(p.index, me)).map(|p| p.index).collect();
            let every = !enemies.is_empty() && enemies.iter().all(|&e| fallen(e).is_some());
            Some(match (every, enemies.len()) {
                (true, 1) => t("The enemy Commander has fallen").to_string(),
                (true, _) => t("Every enemy Commander has fallen").to_string(),
                (false, _) => t("The enemy has been defeated").to_string(),
            })
        }
        Outcome::Defeat => Some(match fallen(me) {
            Some(m) => tf("Your Commander fell at {clock}", &[("clock", &clock(m.tick))]),
            None => t("Your forces have been defeated").to_string(),
        }),
    }
}

/// The debrief: its pages, the one chosen shown.
fn debrief(ui: &mut egui::Ui, game: &mut Game, best_pic: Option<UnitPic>, t: f32, compact: bool) {
    let mut tab = game.debrief.tab;
    let tabs = [(Tab::Summary, n("SUMMARY")), (Tab::Players, n("PLAYERS")), (Tab::Graphs, n("GRAPHS"))];
    ui.horizontal(|ui| {
        w::segmented(ui, &mut tab, &tabs);
    });
    game.debrief.tab = tab;
    ui.add_space(10.0);
    let Some(summary) = game.debrief.summary.take() else { return };
    match tab {
        Tab::Summary => summary_page(ui, game, &summary, best_pic, t, compact),
        Tab::Players => crate::stats::table(ui, game, &summary.players, 36.0),
        Tab::Graphs => {
            let mut series = game.stats_series;
            crate::stats::series_picker(ui, &mut series);
            game.stats_series = series;
            ui.add_space(6.0);
            crate::stats::chart(ui, game, &summary.players, Some(&summary.finals), series, if compact { 170.0 } else { 260.0 });
        }
    }
    game.debrief.summary = Some(summary);
}

/// The summary: the player's figures (counting up as it comes in), their best unit beside the
/// battle's key moments (one above the other where narrow), and the service record.
fn summary_page(ui: &mut egui::Ui, game: &Game, s: &Summary, best_pic: Option<UnitPic>, t: f32, compact: bool) {
    figures(ui, &s.figures, ease((t - 1.6) / 1.1), compact);
    let gap = if compact { 8.0 } else { 12.0 };
    ui.add_space(gap - 8.0);
    let width = ui.available_width();
    if width >= 620.0 {
        let left = (width * 0.38).floor();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.allocate_ui_with_layout(vec2(left, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(left);
                best_card(ui, game, s.best, best_pic, compact);
            });
            ui.allocate_ui_with_layout(vec2(width - left - gap, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(width - left - gap);
                moments_card(ui, &s.moments, compact);
            });
        });
    } else {
        best_card(ui, game, s.best, best_pic, compact);
        ui.add_space(gap);
        moments_card(ui, &s.moments, compact);
    }
    // What the battle did for the commander's career (in a line on a short screen).
    if let Some(record) = &game.career_record {
        ui.add_space(if compact { 6.0 } else { 10.0 });
        panel::rule(ui);
        ui.add_space(6.0);
        if compact {
            ta_ui::profiles::battle_record_line(ui, record);
        } else {
            ta_ui::profiles::battle_record(ui, record);
        }
    }
}

/// A card's plate: a sunken panel with its caption.
fn card<R>(ui: &mut egui::Ui, caption: &str, compact: bool, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(INSET)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(3)
        .inner_margin(if compact { egui::Margin::symmetric(10, 6) } else { egui::Margin::symmetric(12, 10) })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(w::caption(caption));
            ui.add_space(if compact { 3.0 } else { 6.0 });
            add(ui)
        })
        .inner
}

/// The player's figures, a card each in a row (two rows where narrow), `k` of the way to their
/// values; the best of every player's in brass.
fn figures(ui: &mut egui::Ui, figures: &[Figure], k: f32, compact: bool) {
    let gap = 8.0;
    let width = ui.available_width();
    let across = if width >= 640.0 { figures.len() } else { figures.len().div_ceil(2) };
    let each = ((width - gap * (across as f32 - 1.0)) / across as f32).floor();
    let height = if compact { 46.0 } else { 64.0 };
    for row in figures.chunks(across) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (i, f) in row.iter().enumerate() {
                // (The last of a full row takes what the rounding left: the row ends where the
                // cards under it do.)
                let w = if i + 1 == across { width - (across as f32 - 1.0) * (each + gap) } else { each };
                let (rect, response) = ui.allocate_exact_size(vec2(w, height), Sense::hover());
                let p = ui.painter();
                p.rect(rect, CornerRadius::same(3), INSET, Stroke::new(1.0, LINE), StrokeKind::Inside);
                // (One line, cut short where a translation runs over, whole on hover: a table's
                // caption may break in two.)
                let words = t(f.caption).replace('\n', " ");
                let mut job = egui::text::LayoutJob::single_section(words.clone(), egui::TextFormat { font_id: strong(11.5), color: STEEL, extra_letter_spacing: ta_ui::i18n::tracking(1.2), ..Default::default() });
                job.wrap = egui::text::TextWrapping::truncate_at_width(w - 20.0);
                let caption = ui.painter().layout_job(job);
                let cut = caption.elided;
                p.galley(rect.min + vec2(10.0, if compact { 5.0 } else { 8.0 }), caption, STEEL);
                let response = if cut { response.on_hover_text(words) } else { response };
                let value = (f.write)(f.value * k as f64);
                let colour = if f.lead { AMBER_BRIGHT } else { TEXT_BRIGHT };
                let size = if compact { 20.0 } else { 28.0 };
                let galley = ui.painter().layout_no_wrap(value, display(size), colour);
                let at = pos2(rect.min.x + 10.0, rect.max.y - if compact { 4.0 } else { 6.0 } - galley.size().y);
                ta_ui::metal::embossed(ui.painter(), at, galley, colour);
                if f.lead {
                    response.on_hover_text(t("The best of every player's"));
                }
            }
        });
        ui.add_space(gap);
    }
}

/// The player's best unit: its picture, its name, how many it destroyed and whether it
/// survived; or that none of theirs destroyed anything.
fn best_card(ui: &mut egui::Ui, game: &Game, best: Option<Best>, pic: Option<UnitPic>, compact: bool) {
    card(ui, n("BEST UNIT"), compact, |ui| {
        let Some(best) = best else {
            ui.add(egui::Label::new(RichText::new(t("None of your units destroyed an enemy")).color(STEEL)).wrap());
            return;
        };
        let ty = game.sim.types.get(best.type_id);
        ui.horizontal_top(|ui| {
            let side = if compact { 48.0 } else { 64.0 };
            let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
            match pic {
                Some(p) => {
                    p.paint(ui.painter(), rect, Color32::WHITE);
                    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, LINE), StrokeKind::Outside);
                }
                None => crate::encyclopaedia::no_picture(ui.painter(), rect, &ty.def.display_name),
            }
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = if compact { 1.0 } else { 3.0 };
                ui.add(egui::Label::new(RichText::new(&ty.def.display_name).font(strong(if compact { 15.0 } else { 17.0 })).color(TEXT_BRIGHT)).wrap());
                let kills = if best.kills == 1 { t("1 kill").to_string() } else { tf("{n} kills", &[("n", &best.kills)]) };
                ui.label(RichText::new(kills).font(strong(if compact { 13.5 } else { 15.0 })).color(AMBER_BRIGHT));
                let fate = if best.alive { t("Still standing") } else { t("Destroyed in the battle") };
                ui.label(RichText::new(fate).font(body(12.5)).color(if best.alive { GREEN } else { STEEL }));
            });
        });
    });
}

/// The battle's key moments: each its time and what happened, in the colour of whose it was.
fn moments_card(ui: &mut egui::Ui, moments: &[Moment], compact: bool) {
    card(ui, n("KEY MOMENTS"), compact, |ui| {
        ui.spacing_mut().item_spacing.y = if compact { 2.0 } else { 4.0 };
        for m in moments {
            ui.horizontal(|ui| {
                let time = w::galley(ui, RichText::new(clock(m.tick)).font(strong(12.0)).color(CREAM));
                let (rect, _) = ui.allocate_exact_size(vec2(time.size().x + 12.0, 20.0), Sense::hover());
                ui.painter().rect(rect, CornerRadius::same(2), Color32::from_black_alpha(110), Stroke::new(1.0, m.colour.gamma_multiply(0.6)), StrokeKind::Inside);
                ui.painter().galley(rect.center() - time.size() / 2.0, time, CREAM);
                ui.add(egui::Label::new(RichText::new(&m.text).font(body(13.5)).color(TEXT_BRIGHT)).wrap());
            });
        }
    });
}

/// What next: the battle's highlights on the left; the way home, and the next step in brass,
/// on the right.
fn footer(ui: &mut egui::Ui, ways: &Ways, outcome: Outcome) -> Option<Choice> {
    let mut choice = None;
    ui.horizontal(|ui| {
        // The battle's decisive moments, replayed.
        if ways.highlights && ui.add_enabled(ways.highlights_available, ActionButton::secondary("WATCH HIGHLIGHTS").height(40.0)).on_disabled_hover_text(t(crate::replay::RECORDING_STOPPED)).clicked() {
            choice = Some(Choice::Highlights);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let primary = |ui: &mut egui::Ui, label: &str| ui.add(ActionButton::primary(label).height(40.0).min_width(170.0)).clicked();
            let secondary = |ui: &mut egui::Ui, label: &str| ui.add(ActionButton::secondary(label).height(40.0).min_width(150.0)).clicked();
            let mut next = true;
            match (&ways.mission, outcome) {
                _ if ways.lobby.is_some() => {
                    // Multiplayer: the same people again (a player waits for the host to go
                    // back), or, defeated with the battle going on, watching it.
                    let open = ways.lobby == Some(true);
                    if ui.add_enabled(open, ActionButton::primary("BACK TO THE LOBBY").height(40.0).min_width(170.0)).on_disabled_hover_text(t("Once the host goes back to the lobby, you can follow")).clicked() {
                        choice = Some(Choice::Lobby);
                    }
                    if outcome == Outcome::Defeat && ways.watch && secondary(ui, "KEEP WATCHING") {
                        choice = Some(Choice::KeepWatching);
                    }
                }
                _ if ways.scenario => {
                    if primary(ui, "PLAY AGAIN") {
                        choice = Some(Choice::Again);
                    }
                    if secondary(ui, "SCENARIOS") {
                        choice = Some(Choice::Scenarios);
                    }
                }
                _ if ways.conquest => {
                    if primary(ui, "CONTINUE CONQUEST") {
                        choice = Some(Choice::Conquest);
                    }
                }
                (Some((campaign, index)), Outcome::Victory) => {
                    if primary(ui, "CONTINUE CAMPAIGN") {
                        choice = Some(Choice::Campaign { campaign: campaign.clone(), mission: index + 1 });
                    }
                }
                (Some((campaign, index)), Outcome::Defeat) => {
                    if primary(ui, "RETRY MISSION") {
                        choice = Some(Choice::Again);
                    }
                    if secondary(ui, "BACK TO BRIEFING") {
                        choice = Some(Choice::Campaign { campaign: campaign.clone(), mission: *index });
                    }
                }
                (None, Outcome::Defeat) if ways.watch => {
                    // The battle goes on without us: watch it through everyone's eyes.
                    if primary(ui, "KEEP WATCHING") {
                        choice = Some(Choice::KeepWatching);
                    }
                    if ways.skirmish && secondary(ui, "REMATCH") {
                        choice = Some(Choice::Again);
                    }
                }
                _ if ways.skirmish => {
                    if primary(ui, "REMATCH") {
                        choice = Some(Choice::Again);
                    }
                }
                _ => next = false,
            }
            let home = match (ways.from_editor, next) {
                (true, _) => ActionButton::primary("BACK TO THE EDITOR"),
                (false, true) => ActionButton::secondary(ways.home),
                (false, false) => ActionButton::primary("RETURN TO MAIN MENU"),
            };
            if ui.add(home.height(40.0).min_width(if next { 150.0 } else { 240.0 })).clicked() {
                choice = Some(Choice::Menu);
            }
        });
    });
    choice
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A skirmish's result offered MAIN MENU and went back to the page the battle came from (the
    /// skirmish setup, the saved games), as the battle menu's way out says: it says so now.
    #[test]
    fn a_skirmish_result_names_the_page_it_goes_back_to() {
        assert_eq!(way_home(true, "BACK TO THE SKIRMISH SETUP", true), "BACK TO THE SKIRMISH SETUP");
        assert_eq!(way_home(true, "BACK TO SAVED GAMES", true), "BACK TO SAVED GAMES", "a skirmish loaded from the saved games");
        assert_eq!(way_home(true, "MAIN MENU", true), "MAIN MENU", "begun from the title menu (Continue, --map)");
        // (A narrow window: no room for the page's name beside three other ways on.)
        assert_eq!(way_home(true, "BACK TO THE SKIRMISH SETUP", false), "BACK");
        assert_eq!(way_home(true, "MAIN MENU", false), "MAIN MENU");
        assert_eq!(way_home(false, "BACK TO THE CAMPAIGN", true), "MAIN MENU", "other battles keep theirs beside their own ways on");
    }

    /// A mission's, Conquest's or a scenario's MAIN MENU went back to the campaign page, the
    /// galaxy map or the scenarios (the menu kept the page), where their own buttons go: it goes
    /// to the title menu now. Replays, multiplayer and the editor's tests keep their ways.
    #[test]
    fn main_menu_after_a_mission_is_the_title_menu() {
        assert!(main_menu_to_title(true, true), "a mission, a Conquest battle, a scenario");
        assert!(!main_menu_to_title(true, false), "watched as a replay, or played with others");
        assert!(!main_menu_to_title(false, true), "a skirmish names its page instead");
    }

    /// A scenario's result offered MAIN MENU and went back to the scenarios' page, as SCENARIOS
    /// beside it does: it goes to the title menu now. A skirmish's way home still goes back to
    /// where the battle came from, as it says.
    #[test]
    fn a_scenarios_main_menu_goes_to_the_title_menu() {
        use crate::menu::Page;
        assert_eq!(home_page(true, Page::Scenarios), Page::Title);
        assert_eq!(home_page(true, Page::Records), Page::Title, "a scenario loaded from the saved games");
        assert_eq!(home_page(false, Page::Skirmish), Page::Skirmish);
    }

    #[test]
    fn the_title_eases_in_and_holds() {
        assert_eq!(ease(-1.0), 0.0);
        assert_eq!(ease(1.0), 1.0);
        assert_eq!(ease(5.0), 1.0);
        assert!(ease(0.5) > 0.5, "out quickly, in slowly");
        assert!(title_size(true, true) < title_size(false, false));
    }
}
