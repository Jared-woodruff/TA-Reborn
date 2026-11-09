//! The credits as a film. The names of the people who made Total Annihilation, its expansions, its
//! editor and TA Reborn are not rolled up a screen: they lie in the world of a battle (a scene of
//! `credits/*.toml`, in the stage format of `stage.rs`, on one of TA Reborn's own maps and at its
//! own time of day), laid on the ground as the camera flies over them, struck by blasts that set
//! them burning, projected as light, built by a nanolathe (`world_text.rs`), while the armies of
//! the scene fight across them. Hills hide them as they would anything else on the ground, and
//! shadows fall across them; what stands on a letter has it drawn over it again, lighter, so a
//! name reads through the unit on it (`ta_render::world_text`). The battle is staged off the
//! names: the armies stand clear of the road they lie on (an act's `road`, [`card_ground`]), and
//! the blasts that set a card burning fall beside it ([`card_blasts`]), so no scorch mark lies on
//! a name.
//!
//! The credits themselves (what `credits.rs` reads from the install and from TA Reborn) are packed
//! into cards ([`cards`]: a role and its names, up to seven lines, or a title, or a few lines of
//! prose), and the cards are dealt out to acts ([`plan`]): each act is a scene whose stations
//! (`[[station]]`) say where its cards go. The camera's path is made from the stations: for every
//! card a key that looks at it from over and behind, one of four rigs in turn, so each card is
//! framed to be read ([`build_stage`]). An act ends when its last card has been read; the next
//! one loads its world under a black chapter card. Each act's mood suits its own part of the
//! credits: its scene's `themes` say which roles it suits ("programming", "art", "music"), and an
//! act is dealt to the cards whose role asks for it first.
//!
//! The last act is the finale: TA Reborn's last words, then its emblem burnt into the ground
//! under the stars, the disclaimer over it, and TA's own last word.
//!
//! The film is played to its song, Hadal Static's "The War Reborn" (`credits_song.rs`), and laid
//! on its bars ([`plan`]): the song made as long as the film needs (the shortest of its
//! arrangements that holds it), every card read from a bar line for whole bars, the first card
//! burning in as the band comes in, a card or an act on each drop after a break, the finale's
//! ending the song's (the war's last blast on its loudest bar, the emblem on its flash, the cut
//! to black on its dead stop). The film keeps to the song's clock as the mixer plays it
//! ([`Show::follow`]), not to its frames.
//!
//! Captures: `--open credits` plays the whole film; `TA_CREDITS_ACT=<n>` starts at act n (from
//! 0; the song from its place), `TA_CREDITS_DIR=<dir>` reads the scenes and the song's cue sheet
//! (`song.toml`) from that folder's files (the embedded ones' names) rather than the game's, so
//! one being written needs no rebuild.

use std::fmt::Write as _;

use anyhow::{Context, Result, anyhow};
use glam::Vec2;
use ta_ui::i18n::n;

use crate::credits::Section;
use crate::credits_song::Score;
use crate::stage::{ActSpec, Stage, StationSpec, TextStyle};
use crate::world_text::{LineSpec, Look, TextSpec, Weight, WorldText};

// ---- The cards ------------------------------------------------------------------------------

/// What a card shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardKind {
    /// A section's title, and where it comes from.
    Title,
    /// Roles and the names in them.
    Names,
    /// A few lines of words.
    Prose,
}

/// A role and its names (a role may be empty: lines on their own).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Group {
    pub role: String,
    pub lines: Vec<String>,
    /// Its lines set as they are written, not in capitals (`Block::as_written`).
    pub as_written: bool,
}

/// One card of the roll: what is read at once, and for how long.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub kind: CardKind,
    pub groups: Vec<Group>,
    /// A title's second line.
    pub source: Option<String>,
    pub seconds: f32,
}

/// What fits on a card: its lines (a role counts for most of one, and a gap between two groups
/// for half).
const BUDGET: f32 = 8.0;
/// A line longer than this many characters is words, not a name.
const LONG: usize = 34;
/// Where words are broken into lines.
const WRAP: usize = 40;
/// The scale of words against names, and of a role.
pub const PROSE: f32 = 0.62;
pub const ROLE: f32 = 0.42;
const ROLE_COST: f32 = 0.7;
const GAP: f32 = 0.5;

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}

/// A section's title in lines: at its colon, else (when long) at the space nearest its middle.
fn title_lines(title: &str) -> Vec<String> {
    if let Some((a, b)) = title.split_once(':') {
        return vec![a.trim().to_string(), b.trim().to_string()];
    }
    if title.chars().count() > 24 {
        let middle = title.len() / 2;
        if let Some(at) = title.match_indices(' ').map(|(i, _)| i).min_by_key(|&i| i.abs_diff(middle)) {
            return vec![title[..at].to_string(), title[at + 1..].to_string()];
        }
    }
    vec![title.to_string()]
}

fn seconds_for_names(cost: f32) -> f32 {
    (2.4 + 0.42 * cost).clamp(3.6, 6.4)
}

fn seconds_for_prose(chars: usize) -> f32 {
    (2.6 + chars as f32 / 24.0).clamp(4.0, 7.0)
}

/// The sections of the credits packed into cards, each section opening with its title.
pub fn cards(sections: &[Section]) -> Vec<Card> {
    let mut cards = Vec::new();
    for section in sections {
        cards.push(Card { kind: CardKind::Title, groups: vec![Group { lines: title_lines(&section.title), ..Default::default() }], source: Some(section.source.clone()), seconds: 4.0 });
        let mut current: Option<(Card, f32)> = None;
        let flush = |current: &mut Option<(Card, f32)>, cards: &mut Vec<Card>| {
            if let Some((mut card, used)) = current.take() {
                card.seconds = seconds_for_names(used);
                cards.push(card);
            }
        };
        for block in &section.blocks {
            // (Names as written are names however long: never broken into lines as words are.)
            if !block.as_written && block.names.iter().any(|l| l.chars().count() > LONG) {
                // Words: a card of their own, broken into lines.
                flush(&mut current, &mut cards);
                let lines: Vec<String> = block.names.iter().flat_map(|l| wrap(l, WRAP)).collect();
                let chars = block.names.iter().map(|l| l.chars().count()).sum();
                cards.push(Card { kind: CardKind::Prose, groups: vec![Group { role: block.role.clone(), lines, as_written: false }], source: None, seconds: seconds_for_prose(chars) });
                continue;
            }
            let role = if block.role.is_empty() { 0.0 } else { ROLE_COST };
            let most = ((BUDGET - role).floor() as usize).max(1);
            for chunk in block.names.chunks(most) {
                let cost = role + chunk.len() as f32;
                if current.as_ref().is_some_and(|(_, used)| used + GAP + cost > BUDGET) {
                    flush(&mut current, &mut cards);
                }
                let group = Group { role: block.role.clone(), lines: chunk.to_vec(), as_written: block.as_written };
                match current.as_mut() {
                    Some((card, used)) => {
                        card.groups.push(group);
                        *used += GAP + cost;
                    }
                    None => current = Some((Card { kind: CardKind::Names, groups: vec![group], source: None, seconds: 0.0 }, cost)),
                }
            }
        }
        flush(&mut current, &mut cards);
    }
    cards
}

const CREAM: [f32; 3] = [0.96, 0.92, 0.8];
const AMBER: [f32; 3] = [0.95, 0.76, 0.3];
/// The light of a projected card's names.
const PROJECTED: [f32; 3] = [0.6, 0.92, 1.0];

/// What a card says, line by line as it is set in the world.
pub fn card_lines(card: &Card) -> Vec<LineSpec> {
    let mut lines = Vec::new();
    match card.kind {
        CardKind::Title => {
            for (i, l) in card.groups.iter().flat_map(|g| &g.lines).enumerate() {
                lines.push(LineSpec::new(l.to_uppercase(), if i == 0 { 1.0 } else { 0.86 }, Weight::Display, CREAM).gap(if i == 0 { 0.0 } else { 0.1 }));
            }
            if let Some(source) = &card.source {
                // (Broken into lines: a long one would make the title small to fit it.)
                for (i, l) in wrap(&source.to_uppercase(), 30).into_iter().enumerate() {
                    lines.push(LineSpec::new(l, 0.3, Weight::Strong, AMBER).tracking(0.14).gap(if i == 0 { 0.5 } else { 0.0 }));
                }
            }
        }
        CardKind::Names => {
            for (gi, group) in card.groups.iter().enumerate() {
                let mut gap = if gi == 0 { 0.0 } else { GAP };
                if !group.role.is_empty() {
                    lines.push(LineSpec::new(group.role.to_uppercase(), ROLE, Weight::Strong, AMBER).tracking(0.16).gap(gap));
                    gap = 0.05;
                }
                for l in &group.lines {
                    let text = if group.as_written { l.clone() } else { l.to_uppercase() };
                    lines.push(LineSpec::new(text, 1.0, Weight::Display, CREAM).gap(gap));
                    gap = 0.0;
                }
            }
        }
        CardKind::Prose => {
            for group in &card.groups {
                let mut gap = 0.0;
                if !group.role.is_empty() {
                    lines.push(LineSpec::new(group.role.to_uppercase(), ROLE, Weight::Strong, AMBER).tracking(0.16));
                    gap = 0.2;
                }
                for l in &group.lines {
                    lines.push(LineSpec::new(l.clone(), PROSE, Weight::Strong, CREAM).gap(gap));
                    gap = 0.0;
                }
            }
        }
    }
    lines
}

// ---- The acts -------------------------------------------------------------------------------

/// A scene of the film: its file in `credits/`, its text, and what it says of the moment (the
/// caption on the screen, translatable).
pub struct Template {
    pub id: &'static str,
    pub caption: &'static str,
    pub text: &'static str,
}

macro_rules! act {
    ($id:literal, $caption:expr) => {
        Template { id: $id, caption: $caption, text: include_str!(concat!("../credits/", $id, ".toml")) }
    };
}

/// Every scene, in the order the film plays them unless the roles ask for another; the finale
/// last.
pub const TEMPLATES: &[Template] = &[
    act!("highland_dawn", n("First light over the highlands")),
    act!("foundry_works", n("The shift begins at the foundry")),
    act!("cinder_night", n("Night falls on the basalt")),
    act!("sunscar_noon", n("High noon over the dunes")),
    act!("frostfall_night", n("Moonlight on the snow")),
    act!("starfall_finale", n("The last light, among the asteroids")),
];

/// A scene as read: its stage (the base, as authored), what it says of itself, and its places.
#[derive(Clone)]
pub struct Scene {
    pub id: &'static str,
    pub caption: &'static str,
    pub text: String,
    pub stage: Stage,
    pub act: ActSpec,
    pub stations: Vec<StationSpec>,
}

impl Scene {
    /// Whether it is the finale (it ends the film, and its last station is the emblem's).
    pub fn finale(&self) -> bool {
        self.act.themes.iter().any(|t| t == "finale")
    }
}

/// Reads the scenes (from `TA_CREDITS_DIR`'s files when it is set, else the embedded ones).
pub fn scenes() -> Vec<Scene> {
    let dir = std::env::var_os("TA_CREDITS_DIR").map(std::path::PathBuf::from);
    let mut scenes = Vec::new();
    for template in TEMPLATES {
        let text = match &dir {
            Some(dir) => std::fs::read_to_string(dir.join(format!("{}.toml", template.id))).unwrap_or_else(|_| template.text.to_string()),
            None => template.text.to_string(),
        };
        match Stage::parse(&text) {
            Ok(stage) => match stage.act.clone() {
                Some(act) if !stage.stations.is_empty() => scenes.push(Scene { id: template.id, caption: template.caption, text, act, stations: stage.stations.clone(), stage }),
                _ => log::warn!("credits: scene {} has no [act] or no stations", template.id),
            },
            Err(e) => log::warn!("credits: scene {}: {e:#}", template.id),
        }
    }
    scenes
}

/// The card the emblem stands for.
pub const EMBLEM: usize = usize::MAX;

/// A card in its act: which, when it is read from (its act's own seconds, from its picture
/// coming in) and for how long; and the song's hit its blast falls on, when it lands on one (the
/// band coming in, a drop), else its first beat.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub card: usize,
    pub start: f32,
    pub seconds: f32,
    pub hit: Option<f32>,
}

/// An act of the film: its scene, the cards it holds; when its picture comes in on the score's
/// clock (seconds of the song as the film plays it), and for how long it plays (to its going out,
/// the finale to the song's dead stop); and a drop it comes in on, in its own seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct ActPlan {
    pub scene: usize,
    pub slots: Vec<Slot>,
    pub start: f32,
    pub length: f32,
    pub drop: Option<f32>,
}

impl ActPlan {
    /// When its picture goes out (the score's clock).
    pub fn end(&self) -> f32 {
        self.start + self.length
    }
}

/// The finale's moments, in its own seconds: the war's last blast on the song's loudest bar, the
/// bar after it, the emblem burnt into the ground on the flash, the song's dead stop (the picture
/// cut to black).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ending {
    pub nuke: f32,
    pub after: f32,
    pub flash: f32,
    pub stop: f32,
}

impl Ending {
    /// The finale's ending, from its emblem's slot (which runs from the loudest bar to the stop).
    pub fn of(score: &Score, finale: &ActPlan) -> Option<Ending> {
        let emblem = finale.slots.iter().find(|s| s.card == EMBLEM)?;
        let after = (score.after - score.loudest) as f32;
        let flash = (score.flash - score.loudest) as f32;
        Some(Ending { nuke: emblem.start, after: emblem.start + after, flash: emblem.start + flash, stop: emblem.start + emblem.seconds })
    }
}

/// The words of the role a card asks about (what an act's themes are matched with).
fn role_of(card: &Card) -> String {
    card.groups.iter().map(|g| g.role.to_lowercase()).collect::<Vec<_>>().join(" ")
}

/// The cards for each act, as (scene, cards): the finale takes the last of them (its own stations
/// less the emblem's), the others the cards before in order, each deal as many as the scene has
/// stations, a section's title starting an act once the one before has some cards. Scenes are
/// dealt out in their order, those a role asks for first.
fn deal(cards: &[Card], scenes: &[Scene]) -> Vec<(usize, Vec<usize>)> {
    let finale = scenes.iter().rposition(Scene::finale);
    let normal: Vec<usize> = (0..scenes.len()).filter(|&i| Some(i) != finale && !scenes[i].stations.is_empty()).collect();
    let reserve = finale.map_or(0, |f| scenes[f].stations.len().saturating_sub(1).min(cards.len()));
    let body = cards.len() - if normal.is_empty() { 0 } else { reserve };
    let mut acts = Vec::new();
    let mut next = 0;
    let mut used: Vec<usize> = Vec::new();
    // (With no scene but the finale it holds what it can, and the rest is left out.)
    while next < body && !normal.is_empty() {
        let ask = cards[next..(next + 3).min(body)].iter().map(role_of).collect::<Vec<_>>().join(" ");
        if normal.iter().all(|s| used.contains(s)) {
            used.clear();
        }
        let free: Vec<usize> = normal.iter().copied().filter(|s| !used.contains(s)).collect();
        let scene = free.iter().copied().find(|&s| scenes[s].act.themes.iter().any(|t| ask.contains(t.as_str()))).unwrap_or(free[0]);
        used.push(scene);
        // Its share of what is left: the scenes still to come take as much (never more than they
        // have stations for), so no act is left with a few cards.
        let share = (body - next).div_ceil(free.len()).max(3);
        let capacity = scenes[scene].stations.len().min(share);
        let mut dealt = Vec::new();
        while next < body && dealt.len() < capacity {
            if dealt.len() >= 3 && dealt.len() * 2 >= capacity && cards[next].kind == CardKind::Title {
                break;
            }
            dealt.push(next);
            next += 1;
        }
        acts.push((scene, dealt));
    }
    if let Some(f) = finale {
        acts.push((f, (body..cards.len().min(body + reserve)).collect()));
    }
    acts
}

/// Bars an act's picture comes in over before its first card (the camera coming in over the
/// battle), and the black between two acts (the picture going out, the next one's chapter card,
/// its world made under it); a card's least (a card is read for at least two bars).
const LEAD_BARS: usize = 1;
const GAP_BARS: usize = 3;
/// (Two, the least the black between acts may be, to bring an act in on a drop.)
const GAP_LEAST: usize = 2;
const CARD_BARS: usize = 2;
/// A card built by a nanolathe takes a bar longer to read: it comes up a letter at a time.
const NANO_BARS: usize = 3;

/// A part of the film laid on the song's bars: an act's coming in, a card, the black between two
/// acts; how many bars it has, and how many it could.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Part {
    Lead(usize),
    Card(usize, usize),
    Gap(usize),
}

#[derive(Clone, Copy, Debug)]
struct Item {
    part: Part,
    bars: usize,
    least: usize,
    most: usize,
    /// The seconds it would like (a card's reading time).
    wants: f32,
}

impl Item {
    /// How much it wants another bar (the most first).
    fn want(&self, bar: f32) -> f32 {
        let over = (self.bars - self.least) as f32;
        match self.part {
            Part::Card(..) => self.wants - self.bars as f32 * bar - 1.5 * over,
            Part::Gap(_) => -0.4 - 2.0 * over,
            Part::Lead(_) => -0.8 - 2.0 * over,
        }
    }

    /// What its `e`th bar over its least is worth: a card's, the reading time it still wants; a
    /// gap's third, the time a world takes to make (the next act comes in late without it).
    fn worth(&self, e: usize, bar: f32) -> f32 {
        let at = self.least + e - 1;
        match self.part {
            Part::Card(..) => self.wants - at as f32 * bar - 1.5 * (e - 1) as f32,
            Part::Gap(_) if at < GAP_BARS => 3.0,
            Part::Gap(_) => -0.4 - 2.0 * (at - GAP_BARS) as f32,
            Part::Lead(_) => -0.8 - 2.0 * (e - 1) as f32,
        }
    }
}

/// What a card or an act coming in on one of the song's drops is worth against the bars' own.
const LANDED: f32 = 20.0;

/// The `spare` bars given out among `items` (each at most its most) so that as many cards and
/// acts as can come in on a drop (`drops`: bar indices) do, the rest where they are worth most
/// (dynamic programming over the bars given so far). The items landing on a drop, and the bars
/// none could take.
fn lay_out(items: &mut [Item], spare: usize, bar: f32, drops: &[usize]) -> (Vec<usize>, usize) {
    let n = items.len();
    let starts: Vec<usize> = items.iter().scan(0, |at, it| {
        let s = *at;
        *at += it.bars;
        Some(s)
    }).collect();
    let none = f32::NEG_INFINITY;
    // best[i][u]: the most worth with items 0..i laid and u spare bars given; given[i][u]: to i-1.
    let mut best = vec![vec![none; spare + 1]; n + 1];
    let mut given = vec![vec![0usize; spare + 1]; n + 1];
    best[0][0] = 0.0;
    for i in 0..n {
        let it = &items[i];
        let lands = matches!(it.part, Part::Card(..) | Part::Lead(_));
        for u in 0..=spare {
            if best[i][u] == none {
                continue;
            }
            let land = if lands && drops.contains(&(starts[i] + u)) { LANDED } else { 0.0 };
            let mut worth = 0.0;
            for e in 0..=(it.most - it.bars).min(spare - u) {
                if e > 0 {
                    worth += it.worth(e, bar);
                }
                let v = best[i][u] + land + worth;
                if v > best[i + 1][u + e] {
                    best[i + 1][u + e] = v;
                    given[i + 1][u + e] = e;
                }
            }
        }
    }
    // (All the spare bars given, else as many as could be.)
    let used = (0..=spare).rev().find(|&u| best[n][u] > none).unwrap_or(0);
    let mut u = used;
    for i in (0..n).rev() {
        let e = given[i + 1][u];
        items[i].bars += e;
        u -= e;
    }
    let mut landed = Vec::new();
    let mut at = 0;
    for (i, it) in items.iter().enumerate() {
        if matches!(it.part, Part::Card(..) | Part::Lead(_)) && drops.contains(&at) {
            landed.push(i);
        }
        at += it.bars;
    }
    (landed, spare - used)
}

/// `n` fewer bars among `items`' cards, from those that need them least (none under its least);
/// those none can give are kept.
fn shrink(items: &mut [Item], n: usize, bar: f32) -> usize {
    for taken in 0..n {
        let Some(best) = items.iter_mut().filter(|it| matches!(it.part, Part::Card(..)) && it.bars > CARD_BARS).min_by(|a, b| a.want(bar).total_cmp(&b.want(bar))) else { return n - taken };
        best.bars -= 1;
    }
    0
}

/// The film for these cards on the song: the cards dealt out to the acts, then laid on its bars.
/// Each card is read from a bar line for as many whole bars as its reading takes; each act comes
/// in a bar before its first card, the black between two acts three bars. The first act's picture
/// comes in with the song's first note and its first card on the band coming in; the finale's
/// last card is read up to the song's loudest bar, its ending (the war's last blast, the emblem
/// on the flash, the cut to black on the dead stop) the song's own. The song is the shortest of
/// its arrangements that holds the film; the bars it has over are given out so that on as many of
/// its drops after a break as they can a card's blast falls (or an act comes in, the black before
/// it the break), the rest to the cards that want them most.
pub fn plan(cards: &[Card], scenes: &[Scene], sheet: &crate::credits_song::Sheet) -> (Vec<ActPlan>, Score) {
    let dealt = deal(cards, scenes);
    let bar = sheet.bar() as f32;
    let finale = dealt.last().map(|(s, _)| *s).filter(|&s| scenes[s].finale());
    let band = sheet.bars.iter().position(|&b| b >= sheet.moments.band - 0.05).unwrap_or(3).max(1);
    let mut items = Vec::new();
    for (k, (scene, list)) in dealt.iter().enumerate() {
        let lead = if k == 0 { band } else { LEAD_BARS };
        items.push(Item { part: Part::Lead(k), bars: lead, least: lead, most: if k == 0 { lead } else { lead + 1 }, wants: 0.0 });
        let nano = scenes[*scene].act.style == TextStyle::Nano;
        for (j, &c) in list.iter().enumerate() {
            let least = ((cards[c].seconds / bar).round() as usize).max(if nano { NANO_BARS } else { CARD_BARS });
            items.push(Item { part: Part::Card(k, j), bars: least, least, most: least + 2, wants: cards[c].seconds });
        }
        if Some(*scene) != finale || k + 1 < dealt.len() {
            items.push(Item { part: Part::Gap(k), bars: GAP_LEAST, least: GAP_LEAST, most: GAP_BARS + 1, wants: 0.0 });
        }
    }
    let least: usize = items.iter().map(|it| it.bars).sum();
    let gaps = items.iter().filter(|it| matches!(it.part, Part::Gap(_))).count();
    // The shortest arrangement whose loudest bar comes after the film's cards (the black between
    // acts its full three bars), else the longest.
    let all = crate::credits_song::scores(sheet);
    let fits = |s: &Score| s.bar_near(s.loudest);
    let comfortable = least + gaps * (GAP_BARS - GAP_LEAST);
    let score = all.iter().find(|s| fits(s) >= comfortable).unwrap_or_else(|| all.last().expect("a score")).clone();
    let loudest = fits(&score);
    let mut spare = loudest as i64 - least as i64;
    if spare < 0 {
        // (Credits longer than any arrangement: read as quickly as they can be.)
        let left = shrink(&mut items, (-spare) as usize, bar);
        spare = -(left as i64);
    }
    // The bars spare given out, as many cards and acts as can be coming in on the drops.
    let drop_bars: Vec<usize> = score.drops.iter().map(|&d| score.bar_near(d)).collect();
    // (Credits far shorter than the song, even its shortest: the cards may take more bars, as many
    // as it takes.)
    let spare = spare.max(0) as usize;
    while items.iter().map(|it| it.most - it.bars).sum::<usize>() < spare && items.iter().any(|it| matches!(it.part, Part::Card(..))) {
        for it in items.iter_mut().filter(|it| matches!(it.part, Part::Card(..))) {
            it.most += 1;
        }
    }
    let (landed, left) = lay_out(&mut items, spare, bar, &drop_bars);
    if left > 0
        && let Some(last) = items.last_mut()
    {
        last.bars += left;
    }
    let mut on_drop: Vec<(usize, f64)> = Vec::new();
    let mut at = 0;
    for (i, it) in items.iter().enumerate() {
        if landed.contains(&i)
            && let Some(&d) = score.drops.iter().find(|&&d| score.bar_near(d) == at)
        {
            on_drop.push((i, d));
        }
        at += it.bars;
    }
    // The items on the bars: act by act.
    let mut acts: Vec<ActPlan> = dealt.iter().map(|(scene, _)| ActPlan { scene: *scene, slots: Vec::new(), start: 0.0, length: 0.0, drop: None }).collect();
    let mut at = 0usize;
    for (i, it) in items.iter().enumerate() {
        let (from, to) = (score.bar_time(at), score.bar_time(at + it.bars));
        let hit = on_drop.iter().find(|(j, _)| *j == i).map(|&(_, d)| d);
        match it.part {
            Part::Lead(k) => {
                // (The first act with the song's first note.)
                acts[k].start = if k == 0 { 0.0 } else { hit.map_or(from, |d| d.min(from)) as f32 };
                acts[k].drop = hit.map(|d| (d - acts[k].start as f64).max(0.0) as f32);
            }
            Part::Card(k, j) => {
                let start = acts[k].start as f64;
                let hit = hit.or((k == 0 && j == 0).then_some(score.band)).map(|h| (h - start) as f32);
                acts[k].slots.push(Slot { card: dealt[k].1[j], start: (from - start) as f32, seconds: (to - from) as f32, hit });
                acts[k].length = (to - start) as f32;
            }
            Part::Gap(_) => {}
        }
        at += it.bars;
    }
    if let Some(last) = acts.last_mut().filter(|a| Some(a.scene) == finale) {
        // The ending: from the loudest bar (or the last card, read past it) to the dead stop.
        let start = last.start as f64;
        let from = (score.loudest - start).max(last.length as f64) as f32;
        let stop = ((score.stop - start) as f32).max(from + 8.0);
        last.slots.push(Slot { card: EMBLEM, start: from, seconds: stop - from, hit: None });
        last.length = stop;
    }
    (acts, score)
}

// ---- The camera, the clearings, the time of day ----------------------------------------------

/// The way a station looks: forward (from the camera towards the card's far side) and to the right.
fn basis(heading: f32) -> (Vec2, Vec2) {
    let (s, c) = heading.to_radians().sin_cos();
    (Vec2::new(s, -c), Vec2::new(c, s))
}

/// How the camera stands to the card `i`: how far over it and behind it (px), how far to its side
/// (towards the right) and how far ahead of it it looks, as shares of the act's own.
#[derive(Clone, Copy, Debug)]
pub struct Rig {
    pub height: f32,
    pub back: f32,
    pub side: f32,
    pub ahead: f32,
}

/// The act's camera for its card `i`: trailing it, to the left of it, higher and nearer, lower and
/// to the right; and for the emblem (`last`), nearly overhead.
pub fn rig(act: &ActSpec, i: usize, emblem: bool) -> Rig {
    let (h, b) = (act.height.unwrap_or(250.0), act.back.unwrap_or(330.0));
    if emblem {
        return Rig { height: h * 2.3, back: b * 0.35, side: 0.0, ahead: 0.0 };
    }
    match i % 4 {
        0 => Rig { height: h, back: b, side: 0.0, ahead: 12.0 },
        1 => Rig { height: h * 0.85, back: b * 0.95, side: -0.5 * b, ahead: 12.0 },
        2 => Rig { height: h * 1.3, back: b * 0.6, side: 0.0, ahead: 12.0 },
        _ => Rig { height: h * 0.62, back: b * 0.66, side: 0.45 * b, ahead: 12.0 },
    }
}

/// How the camera sees a card from its rig: its distance and the angle it looks down at.
pub fn view_of(rig: &Rig) -> (f32, f32) {
    let ground = (rig.back * rig.back + rig.side * rig.side).sqrt();
    ((ground * ground + rig.height * rig.height).sqrt(), rig.height.atan2(ground))
}

fn camera_key(out: &mut String, t: f32, eye: Vec2, height: f32, look: Vec2, fov: f32) {
    let _ = write!(out, "\n[[camera.key]]\nt = {t:.3}\neye = [{:.1}, {:.1}]\nheight = {height:.1}\nlook = [{:.1}, {:.1}]\nlook_height = 0\nfov = {fov:.1}\n", eye.x, eye.y, look.x, look.y);
}

/// How far into a card its camera is where it reads it from, and when it sets off for the next
/// (shares of the card's seconds); how far it drifts between.
const ARRIVE: f32 = 0.22;
const LEAVE: f32 = 0.9;
const DRIFT: f32 = 30.0;
/// How far past the emblem the war's last blast goes off.
const FAR_BLAST: f32 = 520.0;

/// How long before the war's last blast the camera has come down to look at the horizon, and
/// how long the finale's last card is left before it.
const HORIZON_EARLY: f32 = 0.3;
const HORIZON_MOVE: f32 = 1.8;

/// The scene for an act, ready to play: the scene as authored, with the time of day running over
/// the act's length, the trees and rocks taken from each station and the camera's keys looking at
/// each card in its turn; the finale's (`ending`) down to the horizon for the war's last blast,
/// up over the emblem for the flash.
pub fn build_stage(scene: &Scene, act: &ActPlan, ending: Option<Ending>) -> Result<Stage> {
    let spec = &scene.act;
    let fov = spec.fov.unwrap_or(40.0);
    let mut head = String::new();
    if let Some([a, b]) = spec.hours {
        let _ = writeln!(head, "time = [[0.0, {a}], [{:.2}, {b}]]", act.length);
    }
    let mut tail = String::new();
    let place = |i: usize| -> Result<(Vec2, f32)> {
        let s = scene.stations.get(i).ok_or_else(|| anyhow!("scene {} has no station {i}", scene.id))?;
        let middle = (0.0, 0.0);
        let at = s.at.map(|[x, z]| (x, z)).or(s.rel.map(|[dx, dz]| (middle.0 + dx, middle.1 + dz))).ok_or_else(|| anyhow!("station {i} of {} has no place", scene.id))?;
        Ok((Vec2::new(at.0, at.1), s.heading))
    };
    for (i, _) in act.slots.iter().enumerate() {
        let (p, _) = place(i)?;
        let _ = write!(tail, "\n[[clear]]\nat = [{:.1}, {:.1}]\nradius = {:.0}\n", p.x, p.y, spec.clear.unwrap_or(300.0));
    }
    // The camera: coming in over the first card, a key at the middle of each, flying on past the last.
    let (p0, h0) = place(0)?;
    let (f0, r0) = basis(h0);
    let (h, b) = (spec.height.unwrap_or(250.0), spec.back.unwrap_or(330.0));
    camera_key(&mut tail, 0.0, p0 - f0 * (b * 2.3) + r0 * (b * 0.45), h * 1.9, p0 - f0 * 30.0, fov);
    for (i, slot) in act.slots.iter().enumerate() {
        let (p, heading) = place(i)?;
        let (f, r) = basis(heading);
        let g = rig(spec, i, slot.card == EMBLEM);
        // It comes to its place early in the card and drifts on a little (so the picture is never
        // still, and never moves while the card is read), then leaves for the next.
        let (eye, look) = (p - f * g.back + r * g.side, p + f * g.ahead);
        if slot.card == EMBLEM {
            // The finale's ending, on the song: down to the horizon for the war's last blast on its
            // loudest bar, held there a bar, then up over the emblem's ground for the flash, where
            // it burns in, and over it to the dead stop.
            let e = ending.unwrap_or(Ending { nuke: slot.start, after: slot.start + 1.7, flash: slot.start + 5.9, stop: slot.start + slot.seconds });
            let (horizon, horizon_look) = (p - f * (b * 0.8), p + f * FAR_BLAST);
            camera_key(&mut tail, e.nuke - HORIZON_EARLY, horizon, h * 0.5, horizon_look, fov);
            camera_key(&mut tail, e.after, horizon + f * 12.0, h * 0.54, horizon_look, fov);
            camera_key(&mut tail, e.flash - 0.25, eye, g.height, look, fov);
            camera_key(&mut tail, e.stop, eye + f * DRIFT, g.height * 0.97, look + f * DRIFT, fov);
            continue;
        }
        camera_key(&mut tail, slot.start + slot.seconds * ARRIVE, eye, g.height, look, fov);
        // (Before the ending, the last card is left in time to come down to the horizon.)
        let leave = match act.slots.get(i + 1) {
            Some(next) if next.card == EMBLEM => (slot.seconds * LEAVE).min(next.start - HORIZON_EARLY - HORIZON_MOVE - slot.start).max(slot.seconds * 0.5),
            _ => slot.seconds * LEAVE,
        };
        camera_key(&mut tail, slot.start + leave, eye + f * DRIFT, g.height * 0.97, look + f * DRIFT, fov);
    }
    let last = act.slots.len() - 1;
    let (pl, hl) = place(last)?;
    let (fl, rl) = basis(hl);
    let finale_end = act.slots.last().is_some_and(|s| s.card == EMBLEM);
    if !finale_end {
        camera_key(&mut tail, act.length, pl + fl * (b * 0.45) - rl * (b * 0.3), h * 1.7, pl + fl * (b * 1.6), fov);
    }
    let text = format!("{head}\n{}\n{tail}", scene.text);
    Stage::parse(&text).with_context(|| format!("the scene {}'s own time of day, clearings and camera", scene.id))
}

// ---- The cards in the world ------------------------------------------------------------------

/// A blast to set off at a time (the act's seconds on the game's clock): its size (px) and where.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blast {
    pub at: f32,
    pub size: f32,
    pub x: f32,
    pub z: f32,
    /// A weapon's own blast, by name (a nuclear one), else one of `size`.
    pub weapon: Option<&'static str>,
}

/// The most a card is wide (px) unless its act says (`[act] road`): the armies stand clear of it.
pub const ROAD: f32 = 400.0;

/// A blast's scorch mark reaches this far from it, against its size (`effects.rs` scorches the
/// ground 2.2 times a blast's radius across), and the room it leaves the letters at least.
const SCORCH: f32 = 1.1;
const SCORCH_ROOM: f32 = 14.0;
/// How big a burning card's blasts are: a title's, a card of names', and the two after either.
const TITLE_BLAST: f32 = 95.0;
const NAMES_BLAST: f32 = 62.0;
const SMALL_BLAST: f32 = 38.0;

/// The ground a card at `scene`'s station `i` may cover, round the station: how far it reaches to
/// either side of the road at most, and along it (short of the next card's ground, `place_texts`
/// keeps it to four fifths of its nearest neighbour's distance). Nothing of the battle should
/// stand or fall there while the card is read.
pub fn card_ground(scene: &Scene, i: usize) -> Option<(f32, f32)> {
    let at = Vec2::from(scene.stations.get(i)?.at?);
    let neighbour = [i.checked_sub(1), Some(i + 1)].into_iter().flatten().filter_map(|j| scene.stations.get(j)).filter_map(|n| n.at).map(|n| at.distance(Vec2::from(n))).fold(f32::MAX, f32::min);
    let road = scene.act.road.unwrap_or(ROAD);
    Some((road * 0.5, if neighbour < f32::MAX { neighbour * 0.4 } else { road * 0.5 }))
}

/// The blasts that set a burning card alight, `at` the act's seconds, for a card `wide` x `deep`
/// laid at `place`: the first (`big`, the one its letters catch from, rippling out across it)
/// beside the card's middle on `side` (1 its right, -1 its left), two small ones after it, on the
/// song's next two beats (`beat` seconds apart), beside its far corner on the other side and its
/// near corner on that one. Each so far out to the side that its scorch mark lies on none of its
/// letters, and so near its middle that it lies on no other card's ground (a blast beyond its far
/// edge left its mark over the next card's last names, read seconds later).
#[allow(clippy::too_many_arguments)]
pub fn card_blasts(at: f32, beat: f32, place: Vec2, forward: Vec2, right: Vec2, wide: f32, deep: f32, big: f32, side: f32) -> [Blast; 3] {
    let out = |size: f32| wide * 0.5 + size * SCORCH + SCORCH_ROOM;
    let blast = |when: f32, size: f32, p: Vec2| Blast { at: when, size, x: p.x, z: p.y, weapon: None };
    [
        blast(at, big, place + right * (side * out(big))),
        blast(at + beat, SMALL_BLAST, place - right * (side * out(SMALL_BLAST)) + forward * (deep * 0.3)),
        blast(at + 2.0 * beat, SMALL_BLAST, place + right * (side * out(SMALL_BLAST)) - forward * (deep * 0.35)),
    ]
}

/// The blasts on one of the song's hits (the band coming in, a drop after a break): the battle
/// going up either side of the road at `place`, in its lines, as far out as their scorch marks
/// keep off every card's ground (`road` wide).
pub fn hit_blasts(at: f32, place: Vec2, forward: Vec2, right: Vec2, road: f32) -> [Blast; 4] {
    let out = road * 0.5 + HIT_BLAST * SCORCH + SCORCH_ROOM + 10.0;
    let blast = |when: f32, size: f32, p: Vec2| Blast { at: when, size, x: p.x, z: p.y, weapon: None };
    // (A little ahead along the road: further into the picture, which widens there.)
    [
        blast(at, HIT_BLAST, place + right * out + forward * 100.0),
        blast(at, HIT_BLAST, place - right * out + forward * 100.0),
        blast(at + 0.12, HIT_BLAST * 0.7, place + right * (out + 60.0) + forward * 220.0),
        blast(at + 0.12, HIT_BLAST * 0.7, place - right * (out + 60.0) + forward * 160.0),
    ]
}

/// How big the battle's blasts on the song's hits are.
const HIT_BLAST: f32 = 140.0;

/// How long a card's letters stay after it is read (the end of its life goes out over
/// [`crate::world_text`]'s own time). They come on its first beat (or on the hit it lands on).
const AFTER: f32 = 1.2;

/// The cards of an act laid out in the world, `start` being the game's seconds at its start,
/// `aspect` the picture's and `beat` the song's: each at its station, as big as its rig's view
/// lets it be read, coming on its first beat, struck by its blasts; on the song's hits the battle
/// either side going up; the finale's ending (`ending`) the war's last blast on the loudest bar
/// and the emblem burnt in on the flash. The blasts to set off.
#[allow(clippy::too_many_arguments)]
pub fn place_texts(world_text: &mut WorldText, renderer: &mut ta_render::Renderer, cards: &[Card], scene: &Scene, act: &ActPlan, start: f32, aspect: f32, beat: f32, ending: Option<Ending>) -> Vec<Blast> {
    let spec = &scene.act;
    let fov = spec.fov.unwrap_or(40.0).to_radians();
    let road = spec.road.unwrap_or(ROAD);
    let mut blasts = Vec::new();
    // An act coming in on a drop: the battle goes up round its first card.
    if let (Some(drop), Some(first)) = (act.drop, scene.stations.first())
        && let Some([x, z]) = first.at
    {
        let (forward, right) = basis(first.heading);
        blasts.extend(hit_blasts(drop, Vec2::new(x, z), forward, right, road));
    }
    for (i, slot) in act.slots.iter().enumerate() {
        let Some(station) = scene.stations.get(i) else { break };
        let Some([x, z]) = station.at else { continue };
        let at = Vec2::new(x, z);
        let (forward, right) = basis(station.heading);
        let emblem = slot.card == EMBLEM;
        let g = rig(spec, i, emblem);
        let (distance, pitch) = view_of(&g);
        let frame = 2.0 * distance * (fov * 0.5).tan();
        let style = station.style.unwrap_or(spec.style);
        let comes = slot.hit.unwrap_or(slot.start);
        let born = start + comes;
        let life = slot.seconds + (slot.start - comes) + AFTER;
        if emblem {
            let e = ending.unwrap_or(Ending { nuke: slot.start, after: slot.start + 1.7, flash: slot.start + 5.9, stop: slot.start + slot.seconds });
            // The war's last: a nuclear blast on the horizon beyond the emblem's ground, on the
            // song's loudest bar, the camera come down to look at it.
            let far = at + forward * FAR_BLAST;
            blasts.push(Blast { at: e.nuke, size: 600.0, x: far.x, z: far.y, weapon: Some("ATOMIC_BLAST") });
            // The emblem burnt in on the flash, the camera over it: its ground a share of what the
            // picture shows of it.
            let side = (frame * 0.6 / pitch.sin().max(0.5)).min(300.0);
            let blast = at + forward * (side * 0.3);
            let look = Look::Burn { at: blast, after: 0.0 };
            if let Some(text) = world_text.make_emblem(renderer, at, station.heading, side, look, start + e.flash, e.stop - e.flash + AFTER + 6.0, 0.0) {
                world_text.add(text);
            }
            blasts.push(Blast { at: e.flash, size: 150.0, x: blast.x, z: blast.y, weapon: None });
            continue;
        }
        // On a hit (the band coming in, a drop): the battle goes up either side of the card.
        if let Some(hit) = slot.hit {
            blasts.extend(hit_blasts(hit, at, forward, right, road));
        }
        let Some(card) = cards.get(slot.card) else { continue };
        let mut lines = card_lines(card);
        if style == TextStyle::Light {
            // (Projected: its names in a light of their own.)
            for l in lines.iter_mut().filter(|l| l.color == CREAM) {
                l.color = PROJECTED;
            }
        }
        let block = world_text.lay_out(renderer, &lines);
        if block.is_empty() {
            continue;
        }
        // As wide as the picture lets it be and the road does, as deep as the picture lets it be
        // and its ground does: no card reaches the next one's ground.
        let Some((half_road, half_ground)) = card_ground(scene, i) else { continue };
        let width_budget = (frame * aspect * 0.8).min(half_road * 2.0);
        let depth_budget = (frame / pitch.sin().max(0.35) * 0.5).min(half_ground * 2.0);
        let size = (width_budget / block.width.max(0.1)).min(depth_budget / block.height.max(0.1)).min(42.0);
        let depth = block.height * size;
        let wide = block.width * size;
        // Burning: struck beside it, on one side and the other in turn.
        let big = if card.kind == CardKind::Title { TITLE_BLAST } else { NAMES_BLAST };
        let side = if i % 2 == 0 { 1.0 } else { -1.0 };
        let mut strikes = card_blasts(comes, beat, at, forward, right, wide, depth, big, side);
        if let Some([bx, bz]) = station.blast {
            (strikes[0].x, strikes[0].z) = (bx, bz);
        }
        let blast = Vec2::new(strikes[0].x, strikes[0].z);
        let look = match style {
            TextStyle::Burn => Look::Burn { at: blast, after: 0.0 },
            TextStyle::Paint => Look::Paint,
            TextStyle::Light => Look::Light,
            TextStyle::Nano => Look::Nano,
        };
        let text = world_text.make(renderer, &TextSpec { lines, at, heading: station.heading, size, look, born, life, floor: if style == TextStyle::Burn { 0.0 } else { spec.floor.unwrap_or(0.6) } });
        world_text.add(text);
        if style == TextStyle::Burn {
            blasts.extend(strikes);
        }
    }
    blasts
}

// ---- The show -------------------------------------------------------------------------------

/// Where the film is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Black, a chapter card over it, the next act's world being made.
    Chapter,
    /// An act playing.
    Play,
    /// An act's end: going out through black.
    Out,
    /// The last word, on black.
    End,
}

/// Seconds a chapter card is held once the act's world is made before the film's music begins
/// (the first act, or after a skip; between acts the song says when the picture comes in); how
/// long an act fades in and out; how long a chapter card goes out over before its act comes in.
pub const CHAPTER: f32 = 1.8;
/// Seconds the card is on the screen before the world is made (the screen does not move while it is).
pub const CARD_FIRST: f32 = 0.45;
pub const FADE_IN: f32 = 1.5;
/// An act coming in on a drop comes in with it: all but cut in (with reduced flashes, a quick fade).
const DROP_IN: f32 = 0.1;
const DROP_IN_CALM: f32 = 0.5;
pub const FADE_OUT: f32 = 1.3;
const CARD_OUT: f32 = 0.6;
/// The caption of an act: in from this second to that one.
const CAPTION_FROM: f32 = 2.0;
const CAPTION_UNTIL: f32 = 8.5;
/// The finale's disclaimer: in this long after the emblem burns in.
const DISCLAIMER_FROM: f32 = 1.2;
/// The last word's seconds before the buttons.
const LAST_WORD: f32 = 6.5;
/// How long the film waits for its music to be heard before it plays on without it.
const MUSIC_WAIT: f32 = 1.0;
/// How far behind the mixer the speakers are (seconds): what was mixed is heard a little later.
const HEARD_LATER: f64 = 0.015;
/// The most a film's battle is moved on at once (seconds): the game's own update takes 8 ticks at
/// most.
pub const CATCH_UP_STEP: f32 = 0.25;
/// How long after its time a blast is still heard when the battle catches up with the song, and
/// how long a blast's fire and smoke last (the war's last, nuclear, far longer): one longer past
/// is left out.
const STALE: f32 = 0.15;
const SPENT: f32 = 4.0;
const SPENT_NUCLEAR: f32 = 20.0;

/// The act's battle as [`Show::catch_up`] moves it on.
pub trait Battle {
    /// Moves the battle on by `dt` seconds, its own sounds of them not heard when `quiet` (long
    /// past by the time the step is shown); its clock after (the act's seconds).
    fn step(&mut self, dt: f32, quiet: bool) -> f32;
    /// Sets a blast off now, heard or not.
    fn blast(&mut self, blast: &Blast, heard: bool);
}

/// The film playing.
pub struct Show {
    pub cards: Vec<Card>,
    pub scenes: Vec<Scene>,
    pub acts: Vec<ActPlan>,
    /// The song as this film plays it.
    pub score: Score,
    pub act: usize,
    pub phase: Phase,
    /// Seconds in the phase (the chapter card's, the fade's...) and frames drawn in it.
    pub since: f32,
    pub frames: u32,
    /// The act's own clock while it plays: seconds since its start.
    pub clock: f32,
    /// Its world is made and its scene begun.
    pub ready: bool,
    pub blasts: Vec<Blast>,
    pub run: Option<crate::stage::StageRun>,
    /// The game's seconds when the act began.
    pub start: f32,
    /// The finished stage of the act playing.
    pub stage: Option<Stage>,
    /// The map whose world is loaded.
    pub world: Option<String>,
    /// The film's clock: seconds of the song as the film plays it (its music, as heard, when it
    /// plays; else the frames' own).
    pub song: f64,
    /// The music is the film's: its clock runs (with the music, or on its own when it cannot).
    pub scored: bool,
    /// The music asked for and not heard yet: the clock waits (seconds waited so far).
    pub waiting: Option<f32>,
    /// Seconds since the act's picture came in (its fade in, whatever its clock).
    pub shown: f32,
    /// Reduced flashes: an act coming in on a drop fades in, not all at once.
    pub calm: bool,
}

impl Show {
    /// The film for these sections of the credits. `first`: the act to begin with.
    pub fn new(sections: &[Section], first: usize) -> Option<Show> {
        let scenes = scenes();
        if scenes.is_empty() {
            return None;
        }
        let sheet = match crate::credits_song::Sheet::load() {
            Ok(sheet) => sheet,
            Err(e) => {
                log::warn!("credits: {e:#}");
                return None;
            }
        };
        let cards = cards(sections);
        let (acts, score) = plan(&cards, &scenes, &sheet);
        if acts.is_empty() {
            return None;
        }
        let act = first.min(acts.len() - 1);
        Some(Show {
            cards,
            scenes,
            acts,
            score,
            act,
            phase: Phase::Chapter,
            since: 0.0,
            frames: 0,
            clock: 0.0,
            ready: false,
            blasts: Vec::new(),
            run: None,
            start: 0.0,
            stage: None,
            world: None,
            song: 0.0,
            scored: false,
            waiting: None,
            shown: 0.0,
            calm: false,
        })
    }

    pub fn scene(&self) -> &Scene {
        &self.scenes[self.acts[self.act].scene]
    }

    pub fn plan(&self) -> &ActPlan {
        &self.acts[self.act]
    }

    /// The finale's ending (the act playing's, when it is the finale).
    pub fn ending(&self) -> Option<Ending> {
        Ending::of(&self.score, self.plan())
    }

    /// The film's length (seconds): the song to its dead stop, and the last word.
    pub fn length(&self) -> f32 {
        self.acts.last().map_or(0.0, ActPlan::end) + LAST_WORD
    }

    /// The whole film's progress at this moment (0..1).
    pub fn progress(&self) -> f32 {
        let at = match self.phase {
            Phase::End => self.length(),
            _ if self.scored => self.song as f32,
            _ => self.plan().start,
        };
        (at / self.length().max(1.0)).clamp(0.0, 1.0)
    }

    /// The music begins at the act's picture, from its place in the song (the film's start, or
    /// after a skip): the clock set there, waiting for it to be heard unless it runs on its own
    /// (`free`: a capture's fixed clock, or no sound).
    pub fn music_begins(&mut self, free: bool) {
        self.song = self.plan().start as f64;
        self.scored = true;
        self.waiting = (!free).then_some(0.0);
    }

    /// The film's clock moves on by `dt`: with the music when it is heard (`music`: its position,
    /// as far as it has been mixed), else on its own (`free`).
    pub fn follow(&mut self, music: Option<f64>, dt: f32, free: bool) {
        if !self.scored {
            return;
        }
        let music = music.filter(|_| !free).map(|m| m - HEARD_LATER);
        if let Some(waited) = self.waiting {
            match music {
                // (Heard: the clock with it.)
                Some(m) if m > self.song => {
                    self.waiting = None;
                    self.song = m;
                }
                _ if waited + dt > MUSIC_WAIT => {
                    log::warn!("credits: the music was not heard; the film plays on without it");
                    self.waiting = None;
                }
                _ => self.waiting = Some(waited + dt),
            }
            return;
        }
        let next = self.song + dt as f64;
        self.song = match music {
            // (Its position moves on in the mixer's steps: the clock eased onto it, or put back on
            // it when far off.)
            Some(m) if (m - next).abs() > 0.25 => m,
            Some(m) => next + (m - next) * 0.1,
            None => next,
        };
    }

    /// The act's own clock the song is at.
    pub fn act_clock(&self) -> f32 {
        (self.song - self.plan().start as f64) as f32
    }

    /// Moves on to the next act (or the last word); true when there is one.
    pub fn next_act(&mut self) -> bool {
        self.run = None;
        self.stage = None;
        self.blasts.clear();
        self.ready = false;
        self.since = 0.0;
        self.frames = 0;
        self.clock = 0.0;
        self.shown = 0.0;
        if self.act + 1 < self.acts.len() {
            self.act += 1;
            self.phase = Phase::Chapter;
            true
        } else {
            self.phase = Phase::End;
            false
        }
    }

    /// Skips ahead: an act playing goes out now (to the next, its music from its place in the
    /// song as it comes in), a chapter card is let go. True when the music is to be let go.
    pub fn skip(&mut self) -> bool {
        match self.phase {
            Phase::Play => {
                self.phase = Phase::Out;
                self.since = 0.0;
                self.scored = false;
                self.waiting = None;
                true
            }
            Phase::Chapter if self.ready && !self.scored => {
                self.since = self.since.max(CHAPTER - CARD_OUT);
                false
            }
            Phase::Chapter if self.ready => {
                // (Between acts, the song playing: the next act now, its music from its place.)
                self.scored = false;
                self.waiting = None;
                self.since = CHAPTER - CARD_OUT;
                true
            }
            _ => false,
        }
    }

    /// Starts the film over.
    pub fn restart(&mut self) {
        self.act = 0;
        self.phase = Phase::Chapter;
        self.since = 0.0;
        self.frames = 0;
        self.clock = 0.0;
        self.ready = false;
        self.run = None;
        self.stage = None;
        self.blasts.clear();
        self.song = 0.0;
        self.scored = false;
        self.waiting = None;
        self.shown = 0.0;
    }

    /// The chapter card held: whether the act's picture comes in now (its world made, and the card
    /// held its while, or the song come to it).
    pub fn picture_due(&self) -> bool {
        if !self.ready {
            return false;
        }
        if self.scored { self.waiting.is_none() && self.song >= self.plan().start as f64 - 1e-3 } else { self.since >= CHAPTER }
    }

    /// The act's clock for the camera held still (reduced motion): where the card being read has it
    /// come to its place, so the picture cuts from card to card and moves between none; in the
    /// finale's ending, the horizon for the war's last blast, then over the emblem from the flash.
    pub fn held_clock(&self, clock: f32) -> f32 {
        let plan = self.plan();
        if let Some(e) = self.ending().filter(|e| clock >= e.nuke - HORIZON_EARLY - HORIZON_MOVE) {
            return if clock < e.flash - 0.25 { e.nuke } else { e.flash };
        }
        let slot = plan.slots.iter().find(|s| clock < s.start + s.seconds).or(plan.slots.last());
        slot.map_or(clock, |s| s.start + s.seconds * ARRIVE)
    }

    /// The searchlights of the act at `clock`: pools of light running up and down its road, sweeping
    /// across it, that reveal the cards painted on it as they pass (`ground`: the height of the
    /// ground).
    pub fn searchlights(&self, clock: f32, ground: &dyn Fn(f32, f32) -> f32, out: &mut Vec<ta_render::PointLight>) {
        let scene = self.scene();
        let count = scene.act.searchlights;
        if count == 0 || !matches!(self.phase, Phase::Play | Phase::Out) || scene.stations.len() < 2 {
            return;
        }
        let last = scene.stations.len() - 1;
        for k in 0..count {
            // Up the road and back, each at its own pace, in step with none.
            let period = 24.0 + 6.0 * k as f32;
            let u = ((clock / period + k as f32 / count as f32) * 2.0).rem_euclid(2.0);
            let u = if u > 1.0 { 2.0 - u } else { u };
            let at = u * last as f32;
            let i = (at.floor() as usize).min(last - 1);
            let a = at - i as f32;
            let [x0, z0] = scene.stations[i].at.unwrap_or([0.0, 0.0]);
            let [x1, z1] = scene.stations[i + 1].at.unwrap_or([0.0, 0.0]);
            let (_, right) = basis(scene.stations[i].heading);
            let sweep = 230.0 * (clock * (0.45 + 0.11 * k as f32) + k as f32 * 2.1).sin();
            let (x, z) = (x0 + (x1 - x0) * a + right.x * sweep, z0 + (z1 - z0) * a + right.y * sweep);
            out.push(ta_render::PointLight::new([x, ground(x, z) + 90.0, z], [2.6, 2.7, 3.0], 330.0).without_shadows());
        }
    }

    /// The blasts due by `clock` (the act's seconds), taken off the list, in their order.
    pub fn blasts_due(&mut self, clock: f32) -> Vec<Blast> {
        let (mut due, later): (Vec<Blast>, Vec<Blast>) = std::mem::take(&mut self.blasts).into_iter().partition(|b| b.at <= clock);
        self.blasts = later;
        due.sort_by(|a, b| a.at.total_cmp(&b.at));
        due
    }

    /// Brings the act's battle to `to` (its own seconds: where the song is) from where it is, a
    /// step at a time (at most [`CATCH_UP_STEP`]), each of the act's blasts going off at its own
    /// time among the steps: a frame's worth of the battle as it plays, or all the seconds a world
    /// made late (or a stall) left it behind, which a frame plays at once. What is long past then
    /// (more than [`STALE`] before `to`) is seen as it would be by now but not heard, the battle's
    /// own sounds of those steps too, so nothing of it sounds at once on arrival; a blast whose fire
    /// and smoke would already be over ([`SPENT`]) is left out.
    pub fn catch_up(&mut self, to: f32, battle: &mut dyn Battle) {
        let mut now = self.clock;
        loop {
            for b in self.blasts_due(now) {
                let late = to - b.at;
                if late > if b.weapon.is_some() { SPENT_NUCLEAR } else { SPENT } {
                    continue;
                }
                battle.blast(&b, late <= STALE);
            }
            if to - now <= 1e-4 {
                break;
            }
            // (To the next blast's time, so it goes off on it.)
            let next = self.blasts.iter().map(|b| b.at).filter(|&t| t > now).fold(f32::INFINITY, f32::min);
            let dt = (to - now).min(CATCH_UP_STEP).min((next - now).max(1e-3));
            let reached = battle.step(dt, to - (now + dt) > STALE);
            if reached <= now + 1e-6 {
                // (The battle went no further: paused, or its ticks behind the clock.)
                break;
            }
            now = reached;
        }
        self.clock = now;
    }

    /// What the screen shows over the picture.
    pub fn showing(&self) -> Showing {
        let smooth = |x: f32| {
            let x = x.clamp(0.0, 1.0);
            x * x * (3.0 - 2.0 * x)
        };
        let mut s = Showing { progress: self.progress(), ..Default::default() };
        let scene = self.scene();
        let world = scene.stage.map.clone().unwrap_or_default();
        match self.phase {
            Phase::Chapter => {
                s.black = 1.0;
                // (In at once while the world is made, the card standing still; out as the act
                // comes in: when the song comes to it, else once held its while.)
                let out = if !self.ready {
                    0.0
                } else if self.scored {
                    smooth((self.song as f32 - (self.plan().start - CARD_OUT)) / CARD_OUT)
                } else {
                    smooth((self.since - (CHAPTER - CARD_OUT)) / CARD_OUT)
                };
                let held = if self.ready { 1.0 - out } else { smooth(self.since / 0.3) };
                s.chapter = Some(Chapter { number: self.act + 1, world, moment: scene.caption, alpha: held });
            }
            Phase::Play => {
                let fade_in = match self.plan().drop {
                    Some(_) if self.calm => DROP_IN_CALM,
                    Some(_) => DROP_IN,
                    None => FADE_IN,
                };
                s.black = 1.0 - smooth(self.shown / fade_in);
                let alpha = smooth((self.clock - CAPTION_FROM) / 0.8).min(1.0 - smooth((self.clock - CAPTION_UNTIL) / 0.8));
                if alpha > 0.0 {
                    s.caption = Some((world, scene.caption, alpha));
                }
                if let Some(e) = self.ending() {
                    s.disclaimer = smooth((self.clock - (e.flash + DISCLAIMER_FROM)) / 1.6);
                }
                s.hint = if self.act == 0 { smooth((self.clock - 0.8) / 0.8).min(1.0 - smooth((self.clock - 5.5) / 0.8)) } else { 0.0 };
            }
            Phase::Out => {
                s.black = smooth(self.since / FADE_OUT);
                if scene.finale() {
                    s.disclaimer = 1.0 - s.black;
                }
            }
            Phase::End => {
                // (The song stops dead: the picture is cut to black with it.)
                s.black = 1.0;
                s.last_word = smooth((self.since - 0.5) / 1.4);
                s.buttons = self.since > LAST_WORD;
            }
        }
        s
    }
}

/// A chapter card on the black between acts: which act, where and what moment.
#[derive(Clone, Debug)]
pub struct Chapter {
    pub number: usize,
    pub world: String,
    pub moment: &'static str,
    pub alpha: f32,
}

/// What the credits' page draws over the picture.
#[derive(Clone, Debug, Default)]
pub struct Showing {
    /// How black the screen is (0 none, 1 black).
    pub black: f32,
    pub chapter: Option<Chapter>,
    /// The world and the moment, and how far in the caption is.
    pub caption: Option<(String, &'static str, f32)>,
    /// How far the disclaimer is in (the finale).
    pub disclaimer: f32,
    /// How far the last word is in, and whether the buttons show (the end).
    pub last_word: f32,
    pub buttons: bool,
    /// How far the hint on leaving is in (at first).
    pub hint: f32,
    /// How far the film has got (0..1).
    pub progress: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credits::Block;

    fn block(role: &str, names: &[&str]) -> Block {
        Block::new(role, names.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn titles_break_at_their_colon_or_their_middle() {
        assert_eq!(title_lines("TOTAL ANNIHILATION: THE CORE CONTINGENCY"), vec!["TOTAL ANNIHILATION", "THE CORE CONTINGENCY"]);
        assert_eq!(title_lines("TA REBORN"), vec!["TA REBORN"]);
        let long = title_lines("THE TOTAL ANNIHILATION EDITOR");
        assert_eq!(long.len(), 2);
        assert!(long.iter().all(|l| l.chars().count() <= 20), "{long:?}");
    }

    #[test]
    fn words_wrap_and_names_pack_into_cards() {
        let section = Section {
            title: "THE EDITOR".into(),
            source: "its team".into(),
            blocks: vec![
                block("Project Leader", &["Chris Taylor"]),
                block("Lead Artist", &["Clayton Kauzlaric"]),
                block("Producer", &["Kellyn Beeck"]),
                block("Testing", &["A", "B", "C", "D", "E", "F", "G", "H", "I", "J"]),
                block("Thanks", &["The Total Annihilation community, whose decades of file-format research made this possible"]),
            ],
        };
        let cards = cards(std::slice::from_ref(&section));
        assert_eq!(cards[0].kind, CardKind::Title);
        // The three single names share a card; the ten testers need two.
        assert_eq!(cards[1].groups.len(), 3);
        let testers: Vec<&Card> = cards.iter().filter(|c| c.groups.iter().any(|g| g.role == "Testing")).collect();
        assert_eq!(testers.len(), 2);
        assert!(testers.iter().all(|c| c.groups.iter().map(|g| g.lines.len()).sum::<usize>() <= 7));
        // The prose is a card of its own, broken into lines.
        let prose = cards.last().unwrap();
        assert_eq!(prose.kind, CardKind::Prose);
        assert!(prose.groups[0].lines.len() >= 3 && prose.groups[0].lines.iter().all(|l| l.chars().count() <= WRAP));
        // Every card is read for a fair time.
        assert!(cards.iter().all(|c| (3.6..=7.0).contains(&c.seconds)), "{:?}", cards.iter().map(|c| c.seconds).collect::<Vec<_>>());
    }

    /// TA Reborn's team is one card of its five credits, each a role and a name, set as they are
    /// credited (not in capitals), with the website under them, and read for a fair time; in a
    /// language whose lines run long it is still names, never broken into lines as words are.
    #[test]
    fn reborns_team_is_one_card_of_its_credits_as_written() {
        let cards = cards(&[crate::credits::reborn()]);
        let team: Vec<&Card> = cards.iter().filter(|c| c.groups.iter().any(|g| g.as_written)).collect();
        assert_eq!(team.len(), 1, "the team on one card");
        let card = team[0];
        assert_eq!(card.kind, CardKind::Names);
        assert_eq!(cards.iter().position(|c| std::ptr::eq(c, card)), Some(1), "after TA Reborn's title");
        assert_eq!(card.groups[0].lines, ta_ui::credits::contributors());
        let lines: Vec<String> = card_lines(card).into_iter().map(|l| l.text).collect();
        assert_eq!(
            lines,
            ["Project Lead: Jared Woodruff", "DEV: Claude Opus 5.5 by Anthropic", "QA: GPT6 Astra by OpenAI", "VO: Kokoro-82M by Hexgrad", "VO: Chatterbox by Resemble AI", "WEBSITE", "ta-reborn.com"]
        );
        assert!((3.6..=6.4).contains(&card.seconds), "{}", card.seconds);
        let long = Section { title: "T".into(), source: "s".into(), blocks: vec![Block { role: String::new(), names: vec!["Desenvolvimento: Claude Opus 5.5 by Anthropic".into(); 5], as_written: true }] };
        let long = super::cards(&[long]);
        assert_eq!((long.len(), long[1].kind, long[1].groups[0].lines.len()), (2, CardKind::Names, 5));
    }

    #[test]
    fn the_embedded_scenes_read_and_have_their_places() {
        let scenes = scenes();
        assert_eq!(scenes.len(), TEMPLATES.len(), "every scene reads");
        for s in &scenes {
            assert!(!s.stations.is_empty(), "{}", s.id);
            assert!(s.stage.map.is_some(), "{}", s.id);
            assert!(s.stage.camera.keys.is_empty(), "{}: the camera is made from the stations", s.id);
        }
        assert!(scenes.last().is_some_and(Scene::finale), "the finale last");
    }

    fn sheet() -> crate::credits_song::Sheet {
        crate::credits_song::Sheet::load().expect("the song's cue sheet")
    }

    #[test]
    fn the_plan_deals_every_card_out_and_ends_on_the_emblem() {
        let scenes = scenes();
        let sections = vec![
            Section { title: "A".into(), source: "a".into(), blocks: (0..30).map(|i| block(&format!("Role {i}"), &["Name"])).collect() },
            Section { title: "B: C".into(), source: "b".into(), blocks: (0..12).map(|i| block(&format!("Part {i}"), &["N1", "N2", "N3"])).collect() },
        ];
        let cards = cards(&sections);
        let (acts, _) = plan(&cards, &scenes, &sheet());
        let dealt: Vec<usize> = acts.iter().flat_map(|a| a.slots.iter().map(|s| s.card)).filter(|&c| c != EMBLEM).collect();
        assert_eq!(dealt, (0..cards.len()).collect::<Vec<_>>(), "every card once, in order");
        let last = acts.last().unwrap();
        assert!(scenes[last.scene].finale());
        assert_eq!(last.slots.last().unwrap().card, EMBLEM);
        // An act has no more cards than its scene has stations, and each starts where the one before ended.
        for a in &acts {
            assert!(a.slots.len() <= scenes[a.scene].stations.len());
            for w in a.slots.windows(2) {
                assert!((w[0].start + w[0].seconds - w[1].start).abs() < 1e-3);
            }
            assert!(a.length > a.slots.last().unwrap().start);
        }
    }

    /// The film on its song: the first act's picture with its first note and its first card on
    /// the band coming in, every card from a bar line for whole bars, the acts three bars apart,
    /// a card (or an act) on each drop the bars allow, the finale's last card to the loudest bar,
    /// its ending the song's own, and the song the shortest arrangement that holds it all.
    #[test]
    fn the_film_lands_on_its_songs_bars_and_hits() {
        let scenes = scenes();
        let sheet = sheet();
        // Films of every size: a few names, the original game's alone, everyone's.
        for n in [6usize, 20, 40, 70] {
            let sections = vec![
                Section { title: "A".into(), source: "a".into(), blocks: (0..n).map(|i| block(&format!("Role {i}"), &["Name", "Other"])).collect() },
                Section { title: "B: C".into(), source: "b".into(), blocks: (0..n / 2).map(|i| block(&format!("Part {i}"), &["N1", "N2", "N3", "N4", "N5"])).collect() },
            ];
            let cards = cards(&sections);
            let (acts, score) = plan(&cards, &scenes, &sheet);
            let on_bar = |t: f64| score.bars.iter().any(|&b| (b - t).abs() < 1e-3);
            assert_eq!(acts[0].start, 0.0, "{n}: the first picture with the song's first note");
            let first = acts[0].slots[0];
            assert!((first.hit.unwrap() as f64 - score.band).abs() < 1e-3, "{n}: the first card on the band coming in");
            for (k, a) in acts.iter().enumerate() {
                for s in a.slots.iter().filter(|s| s.card != EMBLEM) {
                    let t = a.start as f64 + s.start as f64;
                    assert!(on_bar(t), "{n}: act {k} card {} at {t:.3} s is on no bar line", s.card);
                    let bars = (s.seconds as f64 / score.bar).round();
                    assert!(bars >= 2.0 && (s.seconds as f64 - bars * score.bar).abs() < 0.04 * bars * score.bar, "{n}: a card of {:.2} s", s.seconds);
                }
                if let Some(next) = acts.get(k + 1) {
                    let gap = next.start - a.end();
                    assert!(gap > 1.9 * score.bar as f32 && gap < 4.6 * score.bar as f32, "{n}: {gap:.2} s between act {k} and the next");
                }
            }
            // The drops the song has before its loudest bar have a card's (or an act's) blast on
            // them, as many as the bars spare allow.
            let hits: Vec<f64> = acts.iter().flat_map(|a| a.slots.iter().filter_map(|s| s.hit).chain(a.drop).map(move |h| a.start as f64 + h as f64)).collect();
            let landed = score.drops.iter().filter(|&&d| hits.iter().any(|&h| (h - d).abs() < 1e-3)).count();
            // (Half at least, however tight the bars: credits as long as the longest arrangement
            // leave no bars to move.)
            assert!(landed * 2 >= score.drops.len(), "{n}: {landed} of the {} drops landed on", score.drops.len());
            // The finale: its last card read to the loudest bar, the ending the song's.
            let finale = acts.last().unwrap();
            let ending = Ending::of(&score, finale).expect("the ending");
            let cards_end = finale.slots.iter().filter(|s| s.card != EMBLEM).map(|s| s.start + s.seconds).fold(0.0, f32::max);
            assert!((cards_end - ending.nuke).abs() < 1e-3, "{n}: the last card ends at {cards_end:.2}, the loudest bar at {:.2}", ending.nuke);
            assert!(((finale.start + ending.nuke) as f64 - score.loudest).abs() < 1e-3);
            assert!(((finale.start + ending.flash) as f64 - score.flash).abs() < 1e-3);
            assert!(((finale.end()) as f64 - score.stop).abs() < 1e-3, "{n}: the picture cut on the dead stop");
            // The shortest arrangement that holds it.
            let shorter = crate::credits_song::scores(&sheet).into_iter().rev().find(|s| s.length < score.length);
            if let Some(s) = shorter {
                let least: f32 = acts.iter().flat_map(|a| a.slots.iter().filter(|s| s.card != EMBLEM)).map(|s| (s.seconds as f64 / score.bar).round() as f32).sum();
                assert!(s.bar_near(s.loudest) as f32 <= least + 3.0 * acts.len() as f32 + 8.0, "{n}: a shorter arrangement would hold it");
            }
            println!("{n} names: {} cards, {} acts, the song arranged to {:.0} s (jumps {:?}), {landed} of {} drops", cards.len(), acts.len(), score.length, score.jumps, score.drops.len());
        }
    }

    #[test]
    fn every_act_builds_a_scene_with_a_camera_key_for_each_card() {
        let scenes = scenes();
        let sections = vec![Section { title: "A".into(), source: "a".into(), blocks: (0..12).map(|i| block(&format!("Role {i}"), &["Name"])).collect() }];
        let cards = cards(&sections);
        let (acts, score) = plan(&cards, &scenes, &sheet());
        for act in acts {
            let ending = Ending::of(&score, &act);
            let stage = build_stage(&scenes[act.scene], &act, ending).expect("the scene builds");
            let names = act.slots.iter().filter(|s| s.card != EMBLEM).count();
            let expected = if ending.is_some() { names * 2 + 4 + 1 } else { names * 2 + 2 };
            assert_eq!(stage.camera.keys.len(), expected, "two keys each, the ending four, and the coming in (and the flying on)");
            assert!(stage.camera.keys.windows(2).all(|w| w[0].t <= w[1].t), "in order");
            assert!(stage.time.is_some() == scenes[act.scene].act.hours.is_some());
        }
    }

    #[test]
    fn the_installs_credits_make_a_film_of_a_fair_length() {
        let Ok(install) = ta_vfs::install::discover(None) else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "base").unwrap();
        let mut sections = crate::credits::original(&vfs, Some(&install.root));
        sections.push(crate::credits::reborn());
        let disclaimer = crate::credits::disclaimer(&mut sections);
        assert!(disclaimer.is_some_and(|d| d.contains("unofficial")), "TA Reborn's closing words are kept");
        let cards = cards(&sections);
        let scenes = scenes();
        let (acts, score) = plan(&cards, &scenes, &sheet());
        for (i, c) in cards.iter().enumerate() {
            let first = c.groups.iter().flat_map(|g| g.lines.first()).next().cloned().unwrap_or_default();
            let roles: Vec<&str> = c.groups.iter().map(|g| g.role.as_str()).collect();
            println!("card {i:2} {:?} {:.1} s: {first} [{}]", c.kind, c.seconds, roles.join(" | "));
        }
        println!("the song: {:.1} s, jumps back {:?}; band {:.2}, drops {:?}, loudest {:.2}, flash {:.2}, stop {:.2}", score.length, score.jumps, score.band, score.drops, score.loudest, score.flash, score.stop);
        for (i, a) in acts.iter().enumerate() {
            println!("act {i}: {} from {:.2} s to {:.2} s ({} cards){}", scenes[a.scene].id, a.start, a.end(), a.slots.len(), a.drop.map_or(String::new(), |d| format!(", comes in on a drop at {:.2}", a.start + d)));
            for s in &a.slots {
                let what = if s.card == EMBLEM { "the ending".to_string() } else { format!("card {}", s.card) };
                println!("    {what} at {:.2} s for {:.2} s{}", a.start + s.start, s.seconds, s.hit.map_or(String::new(), |h| format!(", its blast on a hit at {:.2}", a.start + h)));
            }
        }
        let total = acts.last().unwrap().end();
        println!("{} cards, {} acts, {total:.0} s", cards.len(), acts.len());
        assert!(total < 330.0, "the film is {total:.0} s");
        assert!(scenes[acts.last().unwrap().scene].finale());
    }

    /// The blasts that set a card burning, and the battle's on the song's hits, leave their scorch
    /// marks on no letter: neither on the card's own (however wide and deep it is laid) nor on the
    /// ground any other card of its scene may cover (one beyond the card's far edge marked the next
    /// card's last names).
    #[test]
    fn a_burning_cards_blasts_mark_no_cards_ground() {
        // How far a circle round `p` reaches into a box `half` round station `j` (> 0: it does).
        let into = |scene: &Scene, j: usize, half: (f32, f32), p: Vec2, reach: f32| {
            let st = &scene.stations[j];
            let (forward, right) = basis(st.heading);
            let d = p - Vec2::from(st.at.unwrap());
            let (across, along) = (d.dot(right).abs(), d.dot(forward).abs());
            let outside = Vec2::new((across - half.0).max(0.0), (along - half.1).max(0.0)).length();
            reach - outside
        };
        let mut marked = Vec::new();
        for scene in scenes() {
            let road = scene.act.road.unwrap_or(ROAD);
            for i in 0..scene.stations.len() {
                let (half_road, half_ground) = card_ground(&scene, i).expect("a station's ground");
                let st = &scene.stations[i];
                let (forward, right) = basis(st.heading);
                let at = Vec2::from(st.at.unwrap());
                // A title as wide and deep as can be, a card of names as deep and half as wide, a
                // small one; the battle's blasts on a hit.
                let mut all = Vec::new();
                for (wide, deep, big) in [(half_road * 2.0, half_ground * 2.0, TITLE_BLAST), (half_road, half_ground * 2.0, NAMES_BLAST), (half_road * 0.5, half_ground, NAMES_BLAST)] {
                    for side in [1.0, -1.0] {
                        all.extend(card_blasts(0.0, 0.43, at, forward, right, wide, deep, big, side).map(|b| (b, (wide, deep))));
                    }
                }
                all.extend(hit_blasts(0.0, at, forward, right, road).map(|b| (b, (half_road * 2.0, half_ground * 2.0))));
                for (b, (wide, deep)) in all {
                    let p = Vec2::new(b.x, b.z);
                    let reach = b.size * SCORCH;
                    let own = into(&scene, i, (wide * 0.5, deep * 0.5), p, reach);
                    if own > 0.0 {
                        marked.push(format!("{} station {i}: a {:.0} px blast {own:.0} px into its own {wide:.0} x {deep:.0} card", scene.id, b.size));
                    }
                    for j in (0..scene.stations.len()).filter(|&j| j != i) {
                        let other = into(&scene, j, card_ground(&scene, j).unwrap(), p, reach);
                        if other > 0.0 {
                            marked.push(format!("{} station {i}: a {:.0} px blast {other:.0} px into station {j}'s ground", scene.id, b.size));
                        }
                    }
                }
            }
        }
        assert!(marked.is_empty(), "{marked:#?}");
    }

    #[test]
    fn a_rigs_view_looks_down_far_enough_to_read() {
        let act = ActSpec { title: "t".into(), themes: Vec::new(), hours: None, style: TextStyle::Burn, height: None, back: None, fov: None, clear: None, road: None, floor: None, searchlights: 0 };
        for i in 0..4 {
            let (distance, pitch) = view_of(&rig(&act, i, false));
            assert!(pitch.to_degrees() > 25.0, "rig {i}: {:.0} degrees", pitch.to_degrees());
            assert!((200.0..520.0).contains(&distance), "rig {i}: {distance}");
        }
        assert!(view_of(&rig(&act, 0, true)).1.to_degrees() > 60.0, "the emblem is seen from above");
    }

    fn show() -> Show {
        let sections = vec![Section { title: "A".into(), source: "a".into(), blocks: (0..40).map(|i| block(&format!("Role {i}"), &["Name", "Other", "Third"])).collect() }];
        Show::new(&sections, 0).expect("a film")
    }

    /// The film keeps to its song's clock as the mixer plays it: its position moves on in the
    /// mixer's steps (10 ms blocks here, frames at 60 a second), and the film's clock stays
    /// within a frame of what is heard; it waits for the song to be heard at first, and after a
    /// frame that took long (a world made, the frame's own time cut short) it is where the music
    /// is at once. With no music to follow (a capture's clock, no sound) it keeps its own time.
    #[test]
    fn the_film_follows_its_songs_clock() {
        let mut s = show();
        s.ready = true;
        assert!(!s.picture_due(), "the first act's card is held before the song begins");
        s.since = CHAPTER;
        assert!(s.picture_due());
        s.music_begins(false);
        // The mixer: 10 ms blocks; it starts (primed) 0.12 s after the song is asked for.
        let mut wall = 0.0f64;
        let mixer = |wall: f64| if wall < 0.12 { Some(0.0) } else { Some(((wall - 0.12) / 0.01).floor() * 0.01) };
        let dt = 1.0 / 60.0;
        let mut worst = 0.0f64;
        for frame in 0..600 {
            wall += dt as f64;
            s.follow(mixer(wall), dt, false);
            if wall < 0.12 {
                assert!(s.waiting.is_some() && s.song == 0.0, "frame {frame}: held until the song is heard");
            } else if wall > 0.5 {
                let heard = mixer(wall).unwrap() - HEARD_LATER;
                worst = worst.max((s.song - heard).abs());
            }
        }
        assert!(worst < 0.017, "the film is {worst:.4} s off the song");
        // A frame that took three seconds (its own time cut to a tenth): where the music is.
        wall += 3.0;
        s.follow(mixer(wall), 0.1, false);
        assert!((s.song - (mixer(wall).unwrap() - HEARD_LATER)).abs() < 1e-9, "{} against {:?}", s.song, mixer(wall));
        // The music gone (the jukebox took it): on its own time from there.
        let before = s.song;
        s.follow(None, dt, false);
        assert!((s.song - before - dt as f64).abs() < 1e-9);
        // Never heard: it plays on without it once it has waited long enough.
        let mut s = show();
        s.music_begins(false);
        for _ in 0..30 {
            s.follow(None, dt, false);
        }
        assert!(s.waiting.is_some() && s.song == 0.0);
        for _ in 0..40 {
            s.follow(None, dt, false);
        }
        assert!(s.waiting.is_none() && s.song > 0.0);
        // A capture's fixed clock: its frames' time, whatever the music says.
        let mut s = show();
        s.music_begins(true);
        for _ in 0..30 {
            s.follow(Some(99.0), 1.0 / 30.0, true);
        }
        assert!((s.song - 1.0).abs() < 1e-6);
    }

    /// An act whose world took longer to make than the black before it had: it comes in at once,
    /// where the song is (its battle moved on to it), its chapter card already gone; one on time
    /// waits for the song with its card held, and goes out over the song's last moments before it.
    #[test]
    fn a_late_act_comes_in_where_the_song_is() {
        let mut s = show();
        s.act = 1;
        let start = s.plan().start as f64;
        s.music_begins(true);
        s.ready = true;
        s.song = start - 2.0;
        assert!(!s.picture_due());
        let card = |s: &Show| s.showing().chapter.map_or(0.0, |c| c.alpha);
        assert!((card(&s) - 1.0).abs() < 1e-6, "held");
        s.song = start - CARD_OUT as f64 / 2.0;
        assert!(card(&s) > 0.0 && card(&s) < 1.0, "going out");
        s.song = start + 1.5;
        assert!(s.picture_due());
        assert!(card(&s) < 1e-6);
        assert!((s.act_clock() - 1.5).abs() < 1e-5, "its clock where the song is");
    }

    /// A battle that only keeps time: what went off when, heard or not; its steps.
    struct Kept {
        clock: f32,
        blasts: Vec<(f32, f32, bool)>,
        steps: Vec<(f32, bool)>,
    }

    impl Battle for Kept {
        fn step(&mut self, dt: f32, quiet: bool) -> f32 {
            self.clock += dt;
            self.steps.push((dt, quiet));
            self.clock
        }

        fn blast(&mut self, blast: &Blast, heard: bool) {
            self.blasts.push((self.clock, blast.at, heard));
        }
    }

    /// A stall of three seconds (a world made late, a frame that hung) with blasts due inside it:
    /// the battle is moved on to the song a step at a time, each blast going off on its own time
    /// among the steps, unheard when it is long past by then (the battle's own sounds of those
    /// steps too), so none sound together at the end; one whose fire would be over is left out,
    /// one still to come waits for its time.
    #[test]
    fn a_stall_sets_its_blasts_off_on_their_own_times() {
        let mut s = show();
        let blast = |at: f32, weapon: Option<&'static str>| Blast { at, size: 60.0, x: 0.0, z: 0.0, weapon };
        s.blasts = vec![blast(10.9, None), blast(10.2, None), blast(11.5, None), blast(12.3, None), blast(12.95, None), blast(13.1, None), blast(5.0, None), blast(6.0, Some("ATOMIC_BLAST"))];
        s.clock = 10.0;
        let mut battle = Kept { clock: 10.0, blasts: Vec::new(), steps: Vec::new() };
        s.catch_up(13.0, &mut battle);
        assert!((s.clock - 13.0).abs() < 1e-4, "where the song is");
        // Those due in the stall went off on their own times, in their order.
        for &at in &[10.2f32, 10.9, 11.5, 12.3, 12.95] {
            let &(when, _, _) = battle.blasts.iter().find(|b| (b.1 - at).abs() < 1e-6).unwrap_or_else(|| panic!("the blast at {at}"));
            assert!((when - at).abs() < 2e-3, "the blast due at {at} went off at {when}");
        }
        assert!(battle.blasts.windows(2).all(|w| w[0].0 <= w[1].0 && w[0].1 <= w[1].1), "in order: {:?}", battle.blasts);
        // Only the one not long past by the end is heard: nothing sounds at once on arrival.
        let heard: Vec<f32> = battle.blasts.iter().filter(|b| b.2).map(|b| b.1).collect();
        assert_eq!(heard, vec![12.95], "{:?}", battle.blasts);
        // The nuclear one, long past, goes off at once, unheard (its fire still burning); the one
        // whose fire is out is left out; the one still to come waits.
        assert!(battle.blasts.iter().any(|b| b.1 == 6.0 && !b.2 && (b.0 - 10.0).abs() < 1e-6));
        assert!(!battle.blasts.iter().any(|b| b.1 == 5.0));
        assert_eq!(s.blasts.len(), 1);
        assert_eq!(s.blasts[0].at, 13.1);
        // The steps: none over a quarter of a second, those long past quiet, the last ones heard.
        assert!(battle.steps.iter().all(|&(dt, _)| dt > 0.0 && dt <= CATCH_UP_STEP + 1e-6));
        let mut at = 10.0;
        for &(dt, quiet) in &battle.steps {
            at += dt;
            assert_eq!(quiet, 13.0 - at > STALE, "the step to {at}");
        }
        // A frame's worth as it plays: a blast inside it heard, on its time.
        let mut s = show();
        s.blasts = vec![blast(1.01, None)];
        s.clock = 1.0;
        let mut battle = Kept { clock: 1.0, blasts: Vec::new(), steps: Vec::new() };
        s.catch_up(1.0 + 1.0 / 60.0, &mut battle);
        assert_eq!(battle.blasts.len(), 1);
        assert!(battle.blasts[0].2 && (battle.blasts[0].0 - 1.01).abs() < 1e-3);
        assert!(battle.steps.iter().all(|s| !s.1));
    }

    /// Reduced motion: the camera held at each card's place, and in the finale's ending on the
    /// horizon for the war's last blast, then over the emblem from the flash.
    #[test]
    fn the_held_camera_holds_through_the_ending() {
        let mut s = show();
        s.act = s.acts.len() - 1;
        let e = s.ending().expect("the finale's ending");
        assert_eq!(s.held_clock(e.nuke + 0.5), e.nuke);
        assert_eq!(s.held_clock(e.after + 1.0), e.nuke);
        assert_eq!(s.held_clock(e.flash + 0.1), e.flash);
        assert_eq!(s.held_clock(e.stop - 0.1), e.flash);
        let first = s.plan().slots[0];
        assert_eq!(s.held_clock(first.start + 0.1), first.start + first.seconds * ARRIVE);
    }
}
