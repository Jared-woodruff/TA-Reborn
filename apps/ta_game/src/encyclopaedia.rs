//! The unit encyclopaedia (Archives → Units): every unit of the game data, mods' too, what it
//! is and what it can do, standing in a TA world of the player's choosing in its side's colours
//! (the 3D showroom is the app's, `showroom.rs`), at rest, on the move or firing; and a
//! wallpaper of it, framed in a preview of the very picture before it is saved at the size of
//! the player's screen, 1080p, 1440p, 4K or 8K.

use std::path::PathBuf;
use std::sync::mpsc;

use egui::{Align2, Color32, Key, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, pos2, vec2};
use ta_formats::pcx::PcxImage;
use ta_sim::{Locomotion, UnitType};
use ta_ui::i18n::{n, t, tf};
use ta_ui::panel::{self, Exit, Panel};
use ta_ui::scroll::{self, Behind};
use ta_ui::widgets::{self as w, ActionButton};

use crate::game::Game;
use crate::hud::{PortraitKey, UnitPic};
use crate::setup::Assets;
pub use crate::showroom::Act;
use crate::showroom::side_colour;
use crate::theme::*;

/// What kind of unit (the list's filter).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    All,
    Kbots,
    Vehicles,
    Aircraft,
    Ships,
    Hovercraft,
    Buildings,
}

impl Kind {
    const ALL: [Kind; 7] = [Kind::All, Kind::Kbots, Kind::Vehicles, Kind::Aircraft, Kind::Ships, Kind::Hovercraft, Kind::Buildings];

    pub fn label(self) -> &'static str {
        match self {
            Kind::All => n("ALL KINDS"),
            Kind::Kbots => n("KBOTS"),
            Kind::Vehicles => n("VEHICLES"),
            Kind::Aircraft => n("AIRCRAFT"),
            Kind::Ships => n("SHIPS"),
            Kind::Hovercraft => n("HOVER"),
            Kind::Buildings => n("BUILDINGS"),
        }
    }

    /// TA's categories say which walk (KBOT) and which drive; the Commander walks.
    pub fn of(ty: &UnitType) -> Kind {
        match ty.locomotion {
            Locomotion::Static => Kind::Buildings,
            Locomotion::Air => Kind::Aircraft,
            Locomotion::Naval => Kind::Ships,
            Locomotion::Hover => Kind::Hovercraft,
            Locomotion::Ground if ty.def.is_commander || ty.def.category.split_whitespace().any(|c| c.eq_ignore_ascii_case("KBOT")) => Kind::Kbots,
            Locomotion::Ground => Kind::Vehicles,
        }
    }
}

/// A world to show units in: a skirmish map of one of TA's planets.
#[derive(Clone, Debug)]
pub struct World {
    pub planet: String,
    /// The map's file stem.
    pub map: String,
    pub title: String,
}

impl World {
    /// How the world is named: its planet, then its map.
    fn name(&self) -> String {
        format!("{} · {}", self.planet.to_uppercase(), self.title)
    }
}

/// One map of each planet the maps have (open ones first, where the install has them).
pub fn worlds(assets: &Assets) -> Vec<World> {
    const PREFERRED: &[&str] = &[
        "painted desert",
        "sherwood",
        "greenhaven",
        "lava run",
        "metal heck",
        "coast to coast",
        "the pass",
        "rock alley",
        "crystal maze",
        "acid trip",
        "anteer strait",
        "seven islands",
        "brain coral",
        "biggie biggs",
    ];
    let mut planets: Vec<String> = assets.maps.iter().map(|m| m.planet.trim().to_string()).filter(|p| !p.is_empty()).collect();
    planets.sort_by_key(|p| p.to_ascii_lowercase());
    planets.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    planets
        .into_iter()
        .filter_map(|planet| {
            let of_planet: Vec<&crate::setup::MapEntry> = assets.maps.iter().filter(|m| m.planet.trim().eq_ignore_ascii_case(&planet)).collect();
            let pick = PREFERRED.iter().find_map(|p| of_planet.iter().find(|m| m.name.eq_ignore_ascii_case(p))).or_else(|| of_planet.first())?;
            Some(World { planet, map: pick.name.clone(), title: pick.title.clone() })
        })
        .collect()
}

/// The wallpaper sizes: (name, width, height).
pub const SIZES: [(&str, u32, u32); 4] = [("1080p", 1920, 1080), ("1440p", 2560, 1440), ("4K", 3840, 2160), ("8K", 7680, 4320)];

/// What the page asks of the app.
pub enum Action {
    None,
    Back,
    /// Show this unit (its type's name) in this world (map) at this time of day, doing this.
    Show { unit: String, map: String, hours: f32, act: Act },
    /// Draw a wallpaper of the unit on show, this size, with its words on it or not.
    Wallpaper { width: u32, height: u32, words: bool },
}

/// The times of day offered: (name, hours).
const TIMES: [(&str, f32); 4] = [(n("DAWN"), 6.6), (n("DAY"), 12.0), (n("DUSK"), 17.6), (n("NIGHT"), 0.5)];

/// The deck along the showroom's foot: its height before it is first drawn (a row of
/// controls and a line of help in a strip's frame).
pub(crate) const DECK: f32 = 88.0;

/// The deck's height as it was last drawn: the stage stands on it.
pub(crate) fn deck_height(ctx: &egui::Context) -> f32 {
    ctx.data(|d| d.get_temp::<f32>(deck_id())).unwrap_or(DECK)
}

fn deck_id() -> egui::Id {
    egui::Id::new("enc_deck_height")
}

/// The wallpaper's bar, under its preview: its height before it is first drawn, and as it was
/// last drawn (the preview stands on it).
pub(crate) const BAR: f32 = 110.0;

pub(crate) fn bar_height(ctx: &egui::Context) -> f32 {
    ctx.data(|d| d.get_temp::<f32>(bar_id())).unwrap_or(BAR)
}

fn bar_id() -> egui::Id {
    egui::Id::new("enc_wallpaper_bar_height")
}

/// The room round the page's panels, by the screen's width.
fn margin(screen: Rect) -> f32 {
    (screen.width() * 0.02).clamp(12.0, 24.0)
}

/// Logical-point layout shared by the panels and the camera. On small/high-DPI windows
/// details open over the stage on request, leaving room to inspect the model.
pub(crate) struct ShowroomLayout {
    pub margin: f32,
    pub list_width: f32,
    pub info_width: f32,
    pub list_inline: bool,
    pub info_inline: bool,
    pub stage: egui::Rect,
}

pub(crate) fn showroom_layout(screen: egui::Rect, deck: f32) -> ShowroomLayout {
    let margin = margin(screen);
    let list_width = (screen.width() * 0.19).clamp(200.0, 300.0);
    let info_width = (screen.width() * 0.24).clamp(240.0, 400.0);
    let list_inline = screen.width() >= 700.0;
    let info_inline = screen.width() >= 1100.0;
    let chrome = (ta_ui::metal::pipe_inset(5.0) + space::L).round() * 2.0;
    let left = if list_inline { list_width + chrome + 16.0 } else { 0.0 };
    let right = if info_inline { info_width + chrome + 16.0 } else { 0.0 };
    // (The deck along the foot, and a gap over it.)
    let bottom = deck + 16.0;
    let stage = egui::Rect::from_min_max(
        screen.min + vec2(margin + left, margin + 32.0),
        screen.max - vec2(margin + right, margin + bottom),
    );
    ShowroomLayout { margin, list_width, info_width, list_inline, info_inline, stage }
}

/// The wallpaper's preview: its picture's place on the screen (points), `aspect` (width over
/// height) as large as it goes over the bar (`bar` high) along the foot.
pub(crate) fn preview_rect(screen: Rect, bar: f32, aspect: f32) -> Rect {
    let margin = margin(screen);
    let room = Rect::from_min_max(screen.min + vec2(margin, margin), screen.max - vec2(margin, margin + bar + 12.0));
    let aspect = aspect.max(0.1);
    let size = if room.width() / room.height().max(1.0) > aspect { vec2(room.height() * aspect, room.height()) } else { vec2(room.width(), room.width() / aspect) };
    Rect::from_center_size(room.center(), size.max(vec2(1.0, 1.0)))
}

/// Frames the list keeps scrolling the unit on show into sight for, even once it seems to be.
const SCROLL_FRAMES: u8 = 3;

/// A move along the list (the keys').
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    By(isize),
    First,
    Last,
}

pub struct Encyclopaedia {
    search: String,
    /// The side shown (None: every side).
    side: Option<String>,
    kind: Kind,
    /// The unit on show, by its type's name.
    pub selected: Option<String>,
    worlds: Vec<World>,
    pub world: usize,
    time: usize,
    /// The camera turns round the unit on its own (not while a wallpaper is framed).
    turning: bool,
    /// What the unit on show does.
    act: Act,
    /// The view asked back to the one the showroom starts from (the app takes it).
    reset: bool,
    list_open: bool,
    info_open: bool,
    /// The showroom last asked for (asked again when anything changes).
    asked: Option<(String, String, usize, Act)>,
    /// Unit pictures by type id: TA's build pictures, or for a unit with none, one drawn of
    /// it in the showroom (kept on disk).
    pics: std::collections::HashMap<usize, Option<TextureHandle>>,
    /// Units with no picture yet (by type name, lower case): the showroom draws one of the
    /// unit on show.
    unpictured: std::collections::HashSet<String>,
    /// The wallpaper's preview: open, the size picked (none yet: the screen's, or 1080p),
    /// whether its words are on it, saves under way and the last word.
    wallpaper_open: bool,
    size: Option<(u32, u32)>,
    words: bool,
    saving: Vec<mpsc::Receiver<Result<PathBuf, String>>>,
    notice: Option<(String, Option<PathBuf>, f64)>,
    /// The size of the screen the game is on (pixels; the app's to say): a wallpaper's size.
    pub screen: Option<(u32, u32)>,
    /// The list scrolls the unit on show into sight (with this alignment), until it is and at
    /// least this many frames have been drawn.
    scroll_to: Option<(Option<egui::Align>, u8)>,
    /// Captures: the wallpaper's preview opened at once, and this size saved once the page has
    /// drawn this many frames (`TA_WALLPAPER=1080p|1440p|4K|8K|screen`; `TA_WALLPAPER_SAVE=0`
    /// only opens the preview, `TA_WALLPAPER_WORDS=0` leaves the words off).
    auto_wallpaper: Option<(String, u32, bool)>,
}

impl Encyclopaedia {
    pub fn new(assets: &Assets) -> Encyclopaedia {
        let worlds = worlds(assets);
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        // (Captures: `TA_UNIT=CORKROG`, `TA_WORLD=<planet>`, `TA_WORLD_TIME=0..3`.)
        let world = env("TA_WORLD")
            .and_then(|p| worlds.iter().position(|w| w.planet.eq_ignore_ascii_case(&p) || w.map.eq_ignore_ascii_case(&p)))
            .or_else(|| worlds.iter().position(|w| w.map.eq_ignore_ascii_case("painted desert")))
            .unwrap_or(0);
        let auto_wallpaper = env("TA_WALLPAPER").map(|v| (v, 0, env("TA_WALLPAPER_SAVE").is_none_or(|s| s != "0")));
        Encyclopaedia {
            search: String::new(),
            side: None,
            kind: Kind::All,
            selected: env("TA_UNIT").map(|u| u.to_ascii_uppercase()),
            worlds,
            world,
            time: env("TA_WORLD_TIME").and_then(|v| v.parse::<usize>().ok()).filter(|&t| t < TIMES.len()).unwrap_or(1),
            turning: true,
            act: match env("TA_UNIT_ACT").as_deref() {
                Some("move") => Act::Move,
                Some("fire") => Act::Fire,
                _ => Act::Stand,
            },
            reset: false,
            list_open: false,
            info_open: false,
            asked: None,
            pics: Default::default(),
            unpictured: Default::default(),
            wallpaper_open: false,
            size: None,
            words: env("TA_WALLPAPER_WORDS").is_none_or(|v| v != "0"),
            saving: Vec::new(),
            notice: None,
            screen: None,
            scroll_to: Some((Some(egui::Align::Center), SCROLL_FRAMES)),
            auto_wallpaper,
        }
    }

    /// The page opens again: ask for the showroom afresh.
    pub fn opened(&mut self) {
        self.asked = None;
        self.scroll_to = Some((Some(egui::Align::Center), SCROLL_FRAMES));
        self.wallpaper_open = false;
    }

    /// The world the app moved the showroom to (a ship needs the sea): the page follows.
    pub fn moved_to(&mut self, map: &str) {
        if let Some(i) = self.worlds.iter().position(|w| w.map.eq_ignore_ascii_case(map)) {
            self.world = i;
            if let Some(a) = self.asked.as_mut() {
                a.1 = map.to_string();
            }
        }
    }

    /// A world with a sea for a ship (TA's sea worlds first), other than `not`; `skip` passes
    /// over that many (when the first had no sea after all).
    pub fn sea_world(&self, not: &str, skip: u32) -> Option<String> {
        const SEAS: [&str; 5] = ["archipelago", "water world", "lush", "ice", "wet desert"];
        SEAS.iter()
            .filter_map(|p| self.worlds.iter().find(|w| w.planet.eq_ignore_ascii_case(p)))
            .filter(|w| !w.map.eq_ignore_ascii_case(not))
            .nth(skip as usize)
            .map(|w| w.map.clone())
    }

    pub fn hours(&self) -> f32 {
        TIMES[self.time].1
    }

    /// Whether the camera turns round the unit on its own (it holds still while a wallpaper is
    /// framed).
    pub fn turning(&self) -> bool {
        self.turning && !self.wallpaper_open
    }

    /// The wallpaper's preview, while it is open: its picture's shape (width over height), and
    /// whether its words are on it.
    pub fn preview(&self) -> Option<(f32, bool)> {
        let (width, height) = self.wallpaper_size();
        self.wallpaper_open.then_some((width as f32 / height.max(1) as f32, self.words))
    }

    /// Whether the view was asked back to the one the showroom starts from (once).
    pub fn take_reset(&mut self) -> bool {
        std::mem::take(&mut self.reset)
    }

    /// The sizes a wallpaper comes in: (name, width, height), the screen's own first where it
    /// is none of the usual ones.
    fn sizes(&self) -> Vec<(String, u32, u32)> {
        let mut sizes: Vec<(String, u32, u32)> = SIZES.iter().map(|s| (s.0.to_string(), s.1, s.2)).collect();
        if let Some((w, h)) = self.screen.filter(|&(w, h)| w >= 800 && h >= 600 && !SIZES.iter().any(|s| (s.1, s.2) == (w, h))) {
            sizes.insert(0, (t("YOUR SCREEN").to_string(), w, h));
        }
        sizes
    }

    /// The wallpaper's size: as picked, else the screen's (one of the usual ones or its own),
    /// else 1080p.
    fn wallpaper_size(&self) -> (u32, u32) {
        self.size.or(self.screen.filter(|&(w, h)| w >= 800 && h >= 600)).unwrap_or((SIZES[0].1, SIZES[0].2))
    }

    /// A wallpaper is being written (`receive` says how it went), or could not be drawn.
    pub fn wallpaper_saving(&mut self, receive: Option<mpsc::Receiver<Result<PathBuf, String>>>, now: f64) {
        match receive {
            Some(r) => {
                self.saving.push(r);
                self.notice = Some((t("Saving the wallpaper…").to_string(), None, now));
            }
            None => self.notice = Some((t("The wallpaper could not be drawn at this size: this graphics card has too little memory for it. Try a smaller one.").to_string(), None, now)),
        }
    }

    /// A unit's picture `size` points across: its portrait, in its side's colours; its build
    /// picture until the portrait is drawn, and where its model cannot be.
    fn unit_pic(&mut self, ctx: &egui::Context, assets: &Assets, portraits: &mut Portraits, ty: &UnitType, size: f32) -> Option<UnitPic> {
        portraits.get(ctx, ty.id, side_colour(&ty.def.side) as u32, size).or_else(|| self.build_pic(ctx, assets, ty).map(|t| UnitPic { id: t.id(), portrait: false, allegiance: None }))
    }

    /// A unit's build picture: TA's, or for a unit with none, one drawn of it in the showroom.
    fn build_pic(&mut self, ctx: &egui::Context, assets: &Assets, ty: &UnitType) -> Option<TextureHandle> {
        let unpictured = &mut self.unpictured;
        self.pics
            .entry(ty.id)
            .or_insert_with(|| {
                let name = ty.name.to_ascii_lowercase();
                let built = assets.vfs.read_opt(&format!("unitpics/{name}.pcx")).and_then(|b| PcxImage::parse(&b).ok()).map(|pcx| egui::ColorImage::from_rgba_unmultiplied([pcx.width as usize, pcx.height as usize], &pcx.to_rgba()));
                // None of TA's: one drawn of it before, if any.
                let image = built.or_else(|| {
                    let drawn = image::open(drawn_picture_path(assets, &name)).ok().map(|i| i.to_rgba8()).map(|i| egui::ColorImage::from_rgba_unmultiplied([i.width() as usize, i.height() as usize], i.as_raw()));
                    if drawn.is_none() {
                        unpictured.insert(name.clone());
                    }
                    drawn
                })?;
                Some(ctx.load_texture(format!("enc-{name}"), image, egui::TextureOptions::LINEAR))
            })
            .clone()
    }

    /// Whether the unit of type `name` still has no picture (the showroom draws one).
    pub fn wants_picture(&self, name: &str) -> bool {
        self.unpictured.contains(&name.to_ascii_lowercase())
    }

    /// A picture drawn of the unit of type `name` (`type_id`) on show, which had none: shown
    /// from now on, and kept for the next time.
    #[allow(clippy::too_many_arguments)]
    pub fn picture_drawn(&mut self, ctx: &egui::Context, assets: &Assets, type_id: usize, name: &str, width: u32, height: u32, rgba: &[u8]) {
        let name = name.to_ascii_lowercase();
        self.unpictured.remove(&name);
        let path = drawn_picture_path(assets, &name);
        if let Err(e) = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| image::save_buffer(&path, rgba, width, height, image::ColorType::Rgba8).map_err(std::io::Error::other)) {
            log::warn!("could not keep the picture of {name}: {e}");
        }
        let image = egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], rgba);
        self.pics.insert(type_id, Some(ctx.load_texture(format!("enc-{name}"), image, egui::TextureOptions::LINEAR)));
    }

    /// Whether a unit type belongs in the archive at all: one with a side and a name.
    fn archived(ty: &UnitType) -> bool {
        !ty.def.side.trim().is_empty() && !ty.def.display_name.trim().is_empty()
    }

    /// Whether a unit type is one the search finds.
    fn found(&self, ty: &UnitType) -> bool {
        let search = self.search.trim().to_ascii_lowercase();
        search.is_empty()
            || ty.def.display_name.to_ascii_lowercase().contains(&search)
            || ty.def.description.to_ascii_lowercase().contains(&search)
            || ty.name.to_ascii_lowercase().contains(&search)
    }

    fn of_side(&self, ty: &UnitType) -> bool {
        self.side.as_ref().is_none_or(|s| ty.def.side.trim().eq_ignore_ascii_case(s))
    }

    fn of_kind(&self, ty: &UnitType) -> bool {
        self.kind == Kind::All || Kind::of(ty) == self.kind
    }

    /// The units the list holds: those with a side and a name, by side then name, as the
    /// filters have it.
    fn listed<'a>(&self, game: &'a Game) -> Vec<&'a UnitType> {
        let mut list: Vec<&UnitType> = game.sim.types.types.iter().filter(|ty| Self::archived(ty) && self.of_side(ty) && self.of_kind(ty) && self.found(ty)).collect();
        list.sort_by_key(|ty| (ty.def.side.trim().to_ascii_uppercase(), ty.def.display_name.to_ascii_lowercase()));
        list
    }

    /// Whether the list is narrowed by a search or a filter.
    fn filtered(&self) -> bool {
        !self.search.trim().is_empty() || self.side.is_some() || self.kind != Kind::All
    }

    fn clear_filters(&mut self) {
        self.search.clear();
        self.side = None;
        self.kind = Kind::All;
    }

    /// Puts the unit of type `name` on show (a link followed: the list shows it, its filters let
    /// go if they hide it).
    fn select(&mut self, game: &Game, name: &str) {
        if let Some(ty) = game.sim.types.id(name).map(|id| game.sim.types.get(id))
            && !(self.of_side(ty) && self.of_kind(ty) && self.found(ty))
        {
            self.clear_filters();
        }
        self.selected = Some(name.to_string());
        self.scroll_to = Some((Some(egui::Align::Center), SCROLL_FRAMES));
    }

    pub fn draw(&mut self, ui: &mut egui::Ui, assets: &Assets, game: &Game, portraits: &mut Portraits) -> Action {
        let now = ui.input(|i| i.time);
        self.poll(now);
        let mut action = Action::None;
        // The first unit there is, to start with (the Arm Commander, if there is one).
        if self.selected.as_ref().is_none_or(|s| game.sim.types.id(s).is_none()) {
            self.selected = game.sim.types.id("ARMCOM").or_else(|| self.listed(game).first().map(|ty| ty.id)).map(|id| game.sim.types.get(id).name.clone());
        }
        let ty = self.selected.as_ref().and_then(|s| game.sim.types.id(s)).map(|id| game.sim.types.get(id));
        // (What it cannot do, it does not.)
        if let Some(ty) = ty
            && !self.act.possible(game, ty)
        {
            self.act = Act::Stand;
        }
        let ctx = ui.ctx().clone();
        let mut scrolled = false;
        if self.wallpaper_open {
            self.draw_preview(ui, &ctx, game, ty, &mut action);
        } else {
            scrolled = self.draw_browser(ui, &ctx, assets, game, ty, portraits, &mut action);
        }
        self.draw_notice(&ctx, now);
        // (A pass thrown away is drawn again: the list scrolls again in it. The page's first
        // frames are laid out before the window's size is known, every row in sight.)
        if !ctx.will_discard()
            && let Some((_, frames)) = self.scroll_to.as_mut()
        {
            if scrolled && *frames == 0 {
                self.scroll_to = None;
            } else {
                *frames = frames.saturating_sub(1);
            }
        }

        let auto = self.auto_wallpaper.as_mut().map(|(size, frames, save)| {
            *frames += 1;
            (size.clone(), *frames, *save)
        });
        if let Some((wanted, frames, save)) = auto {
            if frames == 2 {
                let screen = wanted.eq_ignore_ascii_case("screen");
                self.size = self.sizes().into_iter().find(|s| s.0.eq_ignore_ascii_case(&wanted) || screen && Some((s.1, s.2)) == self.screen).map(|s| (s.1, s.2)).or(self.size);
                self.open_wallpaper();
            } else if frames == 150 && save {
                let (width, height) = self.wallpaper_size();
                action = Action::Wallpaper { width, height, words: self.words };
            }
        }
        // Ask for the showroom whenever the unit, the world, the light or what it does changes.
        if let (Some(unit), Some(world)) = (self.selected.clone(), self.worlds.get(self.world).cloned()) {
            let want = (unit.clone(), world.map.clone(), self.time, self.act);
            if self.asked.as_ref() != Some(&want) && matches!(action, Action::None) {
                self.asked = Some(want);
                action = Action::Show { unit, map: world.map, hours: self.hours(), act: self.act };
            }
        }
        action
    }

    /// The wallpaper's preview opens: the camera holds still, and comes round to the front if it
    /// was turning on its own (one the player turned and stopped stays as it is).
    fn open_wallpaper(&mut self) {
        self.wallpaper_open = true;
        if self.turning {
            self.reset = true;
        }
    }

    /// The archive as it is browsed: the list, the unit's details, its stage's bar and the deck.
    /// Whether the list scrolled the unit on show into sight.
    #[allow(clippy::too_many_arguments)]
    fn draw_browser(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, assets: &Assets, game: &Game, ty: Option<&UnitType>, portraits: &mut Portraits, action: &mut Action) -> bool {
        let screen = ui.max_rect();
        let layout = showroom_layout(screen, deck_height(ctx));
        // Shade behind the panels so they read over any world.
        w::gradient_h(ui.painter(), egui::Rect::from_min_max(screen.min, egui::pos2(screen.min.x + layout.list_width + 100.0, screen.max.y)), Color32::from_black_alpha(150), Color32::from_black_alpha(0));
        if layout.info_inline || self.info_open {
            w::gradient_h(ui.painter(), egui::Rect::from_min_max(egui::pos2(screen.max.x - layout.info_width - 100.0, screen.min.y), screen.max), Color32::from_black_alpha(0), Color32::from_black_alpha(150));
        }

        if !layout.list_inline {
            egui::Area::new(egui::Id::new("enc_list_toggle")).anchor(Align2::LEFT_TOP, [layout.margin, layout.margin]).show(ctx, |ui| {
                if ui.add(ActionButton::secondary("UNITS")).clicked() {
                    self.list_open = !self.list_open;
                }
            });
        }
        if !layout.info_inline {
            egui::Area::new(egui::Id::new("enc_info_toggle")).anchor(Align2::RIGHT_TOP, [-layout.margin, layout.margin]).show(ctx, |ui| {
                if ui.add(ActionButton::secondary("DETAILS")).clicked() {
                    self.info_open = !self.info_open;
                }
            });
        }

        let mut scrolled = false;
        // (The bar over the stage keeps clear of the panels as they are drawn.)
        let mut between = (screen.min.x, screen.max.x);
        if layout.list_inline || self.list_open {
            let list;
            (scrolled, list) = self.draw_list(ctx, &layout, assets, game, portraits, action);
            if layout.list_inline {
                between.0 = list.max.x + 12.0;
            }
        }
        if (layout.info_inline || self.info_open)
            && let Some(ty) = ty
        {
            let info = self.draw_info(ctx, &layout, assets, game, ty, portraits);
            if layout.info_inline {
                between.1 = info.min.x - 12.0;
            }
        }
        if let Some(ty) = ty {
            self.draw_stage_bar(ctx, screen, &layout, between, game, ty);
        }
        self.draw_deck(ctx, screen, &layout);
        scrolled
    }

    /// The list, on the left: the search, the filters, and the units by side. Up and down (a
    /// page, to either end) move along it. Whether it scrolled the unit on show into sight, and
    /// where it was drawn.
    fn draw_list(&mut self, ctx: &egui::Context, layout: &ShowroomLayout, assets: &Assets, game: &Game, portraits: &mut Portraits, action: &mut Action) -> (bool, Rect) {
        let mut scrolled = false;
        let shown = egui::Area::new(egui::Id::new("enc_list")).anchor(Align2::LEFT_TOP, [layout.margin, layout.margin + if layout.list_inline { 0.0 } else { 40.0 }]).show(ctx, |ui| {
            let units = self.listed(game);
            let total = game.sim.types.types.iter().filter(|ty| Self::archived(ty)).count();
            let subtitle = if self.filtered() { tf("{n} OF {total} UNITS", &[("n", &units.len()), ("total", &total)]) } else { tf("{n} UNITS", &[("n", &units.len())]) };
            let page = Panel::new("UNITS").subtitle(&subtitle).width(layout.list_width).exit(Exit::Back);
            let response = page.show(ui, |ui| {
                ui.set_width(layout.list_width);
                let search = w::search_field(ui, &mut self.search, t("Search units…"), layout.list_width, Some(egui::Id::new("enc_search")));
                // Enter, or down, from the search: on to its first find.
                let (enter, down) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::ArrowDown)));
                if (search.lost_focus() && enter || search.has_focus() && down)
                    && let Some(first) = units.first()
                {
                    if !self.selected.as_ref().is_some_and(|s| units.iter().any(|ty| ty.name.eq_ignore_ascii_case(s))) {
                        self.selected = Some(first.name.clone());
                    }
                    self.scroll_to = Some((None, SCROLL_FRAMES));
                    search.surrender_focus();
                }
                self.filters(ui, game, layout.list_width);
                ui.add_space(4.0);
                panel::well(ui, |ui| {
                    let list_height = (layout.stage.bottom() - ui.cursor().top() - 34.0).max(60.0);
                    // The list's keys, while nothing else has them.
                    let free = ui.memory(|m| m.focused().is_none()) && !egui::Popup::is_any_open(ui.ctx());
                    if free {
                        let page = ((list_height / (ROW + 2.0)) as isize - 1).max(1);
                        let step = ui.input(|i| {
                            [(Key::ArrowUp, Step::By(-1)), (Key::ArrowDown, Step::By(1)), (Key::PageUp, Step::By(-page)), (Key::PageDown, Step::By(page)), (Key::Home, Step::First), (Key::End, Step::Last)]
                                .into_iter()
                                .find(|(key, _)| i.key_pressed(*key))
                                .map(|(_, step)| step)
                        });
                        if let Some(step) = step {
                            self.step(&units, step);
                        }
                    }
                    let area = egui::ScrollArea::vertical().max_height(list_height).min_scrolled_height(list_height).auto_shrink([false, false]);
                    scroll::show(ui, area, Behind::Well, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        if units.is_empty() {
                            ui.add_space(16.0);
                            ui.vertical_centered(|ui| {
                                ui.label(RichText::new(t("No unit matches")).font(strong(13.0)).color(TEXT_BRIGHT));
                                ui.add_space(8.0);
                                if ui.add(ActionButton::secondary("CLEAR FILTERS").height(28.0)).clicked() {
                                    self.clear_filters();
                                    self.scroll_to = Some((Some(egui::Align::Center), SCROLL_FRAMES));
                                }
                            });
                            return;
                        }
                        let mut side = String::new();
                        let on_list = self.selected.as_ref().is_some_and(|s| units.iter().any(|ty| ty.name.eq_ignore_ascii_case(s)));
                        if !on_list {
                            // (Nothing to scroll to: the unit on show is filtered out.)
                            scrolled = true;
                        }
                        for ty in units {
                            if !ty.def.side.trim().eq_ignore_ascii_case(&side) {
                                side = ty.def.side.trim().to_ascii_uppercase();
                                ui.add_space(4.0);
                                let colour = crate::palette::team_color(side_colour(&side));
                                ui.horizontal(|ui| {
                                    let (dot, _) = ui.allocate_exact_size(vec2(8.0, 12.0), Sense::hover());
                                    ui.painter().circle(dot.center(), 3.5, colour, Stroke::new(1.0, Color32::from_black_alpha(200)));
                                    ui.label(RichText::new(&side).font(strong(11.5)).color(AMBER).extra_letter_spacing(ta_ui::i18n::tracking(1.5)));
                                });
                            }
                            let on = self.selected.as_ref().is_some_and(|s| s.eq_ignore_ascii_case(&ty.name));
                            // (Pictures asked for only for the rows in sight.)
                            let seen = ui.is_rect_visible(egui::Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), ROW)));
                            let pic = if seen { self.unit_pic(ctx, assets, portraits, ty, ROW_PICTURE) } else { None };
                            let row = unit_row(ui, ty, pic, on);
                            if on && let Some((align, _)) = self.scroll_to {
                                if ui.is_visible() && !ui.is_sizing_pass() && ui.clip_rect().contains_rect(row.rect) {
                                    scrolled = true;
                                } else {
                                    // (A little of the rows round it in sight too: a side's name over the first.)
                                    ui.scroll_to_rect(row.rect.expand2(vec2(0.0, ROW * 0.5)), align);
                                }
                            }
                            if row.clicked() {
                                self.selected = Some(ty.name.clone());
                                self.list_open = false;
                            }
                        }
                    });
                });
            });
            if response.exit {
                *action = Action::Back;
            }
        });
        (scrolled, shown.response.rect)
    }

    /// The side and kind filters, side by side where every choice fits half the width, else one
    /// above the other: each choice with how many units it would list.
    fn filters(&mut self, ui: &mut egui::Ui, game: &Game, width: f32) {
        let mut sides: Vec<String> = game.sim.types.types.iter().filter(|ty| Self::archived(ty)).map(|ty| ty.def.side.trim().to_ascii_uppercase()).collect();
        sides.sort();
        sides.dedup();
        // (A drop-down's button: its choice's lettering, the arrow and the padding.)
        let spacing = ui.spacing().clone();
        let needs = |text: &str| {
            egui::WidgetText::from(text).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button).size().x + spacing.icon_spacing + spacing.icon_width + 2.0 * spacing.button_padding.x
        };
        let half = (width - 6.0) / 2.0;
        let widest = sides.iter().map(String::as_str).chain([t("ALL SIDES")]).chain(Kind::ALL.iter().map(|k| t(k.label()))).map(needs).fold(0.0, f32::max);
        let across = widest <= half;
        let each = if across { half } else { width };
        let side_text = self.side.clone().unwrap_or_else(|| t("ALL SIDES").to_string());
        let side_combo = |ui: &mut egui::Ui, this: &mut Encyclopaedia| {
            egui::ComboBox::from_id_salt("enc_side").width(each).selected_text(side_text.clone()).truncate().show_ui(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 2.0);
                let count = |side: Option<&str>| game.sim.types.types.iter().filter(|ty| Self::archived(ty) && side.is_none_or(|s| ty.def.side.trim().eq_ignore_ascii_case(s)) && this.of_kind(ty) && this.found(ty)).count();
                let all = count(None);
                let counts: Vec<usize> = sides.iter().map(|s| count(Some(s))).collect();
                if choice(ui, this.side.is_none(), t("ALL SIDES"), None, all) {
                    this.side = None;
                }
                for (s, count) in sides.iter().zip(counts) {
                    let on = this.side.as_ref().is_some_and(|x| x.eq_ignore_ascii_case(s));
                    if choice(ui, on, s, Some(crate::palette::team_color(side_colour(s))), count) {
                        this.side = Some(s.clone());
                    }
                }
            });
        };
        let kind_combo = |ui: &mut egui::Ui, this: &mut Encyclopaedia| {
            egui::ComboBox::from_id_salt("enc_kind").width(each).selected_text(t(this.kind.label())).truncate().show_ui(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 2.0);
                for k in Kind::ALL {
                    let count = game.sim.types.types.iter().filter(|ty| Self::archived(ty) && this.of_side(ty) && (k == Kind::All || Kind::of(ty) == k) && this.found(ty)).count();
                    if choice(ui, this.kind == k, t(k.label()), None, count) {
                        this.kind = k;
                    }
                }
            });
        };
        if across {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                // (Each in its half: a drop-down's width is only its least.)
                for side in [true, false] {
                    ui.scope(|ui| {
                        ui.set_max_width(each);
                        if side { side_combo(ui, self) } else { kind_combo(ui, self) }
                    });
                }
            });
        } else {
            side_combo(ui, self);
            kind_combo(ui, self);
        }
    }

    /// The unit on show moved along the list by a key.
    fn step(&mut self, units: &[&UnitType], step: Step) {
        let Some(last) = units.len().checked_sub(1) else { return };
        let at = self.selected.as_ref().and_then(|s| units.iter().position(|ty| ty.name.eq_ignore_ascii_case(s)));
        let to = match (step, at) {
            (Step::First, _) => 0,
            (Step::Last, _) => last,
            (Step::By(by), Some(at)) => at.saturating_add_signed(by).min(last),
            (Step::By(by), None) => if by < 0 { last } else { 0 },
        };
        if Some(to) != at {
            self.selected = Some(units[to].name.clone());
            self.scroll_to = Some((None, SCROLL_FRAMES));
        }
    }

    /// What the unit is, on the right: its picture and figures, where it comes from, who builds
    /// it and what it builds (each a link to that unit), and its weapons and abilities. Where it
    /// was drawn.
    fn draw_info(&mut self, ctx: &egui::Context, layout: &ShowroomLayout, assets: &Assets, game: &Game, ty: &UnitType, portraits: &mut Portraits) -> Rect {
        let mut follow = None;
        let shown = egui::Area::new(egui::Id::new("enc_info")).anchor(Align2::RIGHT_TOP, [-layout.margin, layout.margin + if layout.info_inline { 0.0 } else { 40.0 }]).show(ctx, |ui| {
            let title = ty.def.display_name.to_uppercase();
            let subtitle = format!("{} · {}", ty.def.side.trim().to_uppercase(), t(Kind::of(ty).label()));
            Panel::new(&title).subtitle(&subtitle).width(layout.info_width).show(ui, |ui| {
                // (An area offers only the room it took last frame: the height is asked for
                // outright, all of it down to the stage's foot when the facts need it.)
                let info_height = (layout.stage.bottom() - ui.cursor().top() - 28.0).max(60.0);
                let area = egui::ScrollArea::vertical().max_height(info_height).min_scrolled_height(info_height).auto_shrink([false, true]);
                scroll::show(ui, area, Behind::Plate, |ui| {
                    ui.set_width(ui.available_width());
                    let pic = self.unit_pic(ctx, assets, portraits, ty, crate::hud::info::PICTURE);
                    crate::hud::info::head(ui, game, ty, None, pic);
                    if let Some(from) = provenance(assets, ty) {
                        ui.add_space(4.0);
                        ui.label(RichText::new(tf("From {source}", &[("source", &from)])).font(body(12.0)).color(STEEL));
                    }
                    // Who builds it, and what it builds: links along the chain.
                    let mut builders: Vec<&UnitType> = game.sim.types.types.iter().filter(|b| Self::archived(b) && b.build_options.contains(&ty.id)).collect();
                    builders.sort_by_key(|b| b.def.display_name.to_ascii_lowercase());
                    builders.dedup_by(|a, b| a.def.display_name == b.def.display_name);
                    if !builders.is_empty() {
                        panel::section(ui, n("BUILT BY"));
                        ui.horizontal_wrapped(|ui| {
                            // (Two to a row where they fit, each as wide.)
                            let gap = 6.0;
                            let row = ui.available_width();
                            let width = if row >= 240.0 { ((row - gap) / 2.0 - 0.5).floor() } else { row };
                            ui.spacing_mut().item_spacing = vec2(gap, gap);
                            for b in builders {
                                let pic = self.unit_pic(ctx, assets, portraits, b, LINK_PICTURE);
                                if unit_chip(ui, b, pic, width).clicked() {
                                    follow = Some(b.name.clone());
                                }
                            }
                        });
                    }
                    crate::hud::info::facts(ui, game, ty);
                    let builds: Vec<&UnitType> = ty.build_options.iter().map(|&o| game.sim.types.get(o)).filter(|o| Self::archived(o)).collect();
                    if !builds.is_empty() {
                        panel::section(ui, n("BUILDS"));
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                            for o in builds {
                                let pic = self.unit_pic(ctx, assets, portraits, o, BUILD_PICTURE);
                                if unit_tile(ui, o, pic).clicked() {
                                    follow = Some(o.name.clone());
                                }
                            }
                        });
                    }
                });
            });
        });
        if let Some(name) = follow {
            self.select(game, &name);
        }
        shown.response.rect
    }

    /// Over the stage, `between` the panels (x): what the unit does (where it can do more than
    /// stand), whether the camera turns round it, and the view it starts from again.
    fn draw_stage_bar(&mut self, ctx: &egui::Context, screen: Rect, layout: &ShowroomLayout, between: (f32, f32), game: &Game, ty: &UnitType) {
        // Between the panels, and the corners' buttons where the panels open on request.
        let left = if layout.list_inline { between.0.max(layout.stage.min.x) } else { screen.min.x + layout.margin + 110.0 };
        let right = if layout.info_inline { between.1.min(layout.stage.max.x) } else { screen.max.x - layout.margin - 120.0 };
        let slot = Rect::from_min_max(pos2(left, screen.min.y + layout.margin), pos2(right.max(left + 120.0), layout.stage.min.y + 60.0));
        panel::strip_in(ctx, "enc_stage_bar", slot, (left + right) / 2.0, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                let acts: Vec<(Act, &str)> = Act::ALL.iter().filter(|a| a.possible(game, ty)).map(|&a| (a, a.label())).collect();
                if acts.len() > 1 {
                    w::segmented(ui, &mut self.act, &acts).on_hover_text(t("What the unit does on show"));
                    ui.add_space(6.0);
                }
                w::toggle(ui, &mut self.turning).on_hover_text(t("The camera turns round the unit on its own"));
                ui.label(RichText::new(t("Turn")).font(strong(12.5)).color(if self.turning { TEXT_BRIGHT } else { STEEL }));
                ui.add_space(6.0);
                if ui.add(ActionButton::secondary("RESET VIEW").height(28.0)).on_hover_text(t("Back to the view from in front of the unit")).clicked() {
                    self.reset = true;
                }
            });
        });
    }

    /// The world and the light, and the wallpaper: a deck along the foot, framed as the panels
    /// are, from the list's left edge to the details' right edge.
    fn draw_deck(&mut self, ctx: &egui::Context, screen: Rect, layout: &ShowroomLayout) {
        let narrow = screen.width() < 900.0;
        let deck = screen.width() - 2.0 * layout.margin - panel::strip_margin().sum().x;
        let hint = RichText::new(t("Drag to look round the unit, the wheel to come closer")).font(body(11.5)).color(STEEL_DIM);
        let mut hint_below = true;
        let height = panel::strip(ctx, "enc_world", Align2::LEFT_BOTTOM, vec2(layout.margin, -layout.margin), |ui| {
            ui.set_width(deck);
            ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
            let controls = |ui: &mut egui::Ui| {
                // (One height from the start: everything in the row sits on one line.)
                ui.set_min_height(32.0);
                self.world_picker(ui, narrow);
                ui.add_space(14.0);
                self.time_picker(ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(ActionButton::primary("WALLPAPER")).on_hover_text(t("Frame a picture of this unit for your desktop")).clicked() {
                        self.open_wallpaper();
                    }
                    // How to look round, in the row where there is room for it (else under it).
                    if ui.available_width() >= w::galley(ui, hint.clone()).size().x + 24.0 {
                        ui.add_space(12.0);
                        ui.label(hint.clone());
                        hint_below = false;
                    }
                });
            };
            if narrow {
                ui.horizontal_wrapped(controls);
            } else {
                ui.horizontal(controls);
            }
            if hint_below {
                ui.label(hint.clone());
            }
            ui.min_rect().height() + panel::strip_margin().sum().y
        });
        // (The stage stands on the deck: it follows the deck's height.)
        if (height - deck_height(ctx)).abs() > 0.5 {
            ctx.data_mut(|d| d.insert_temp(deck_id(), height));
            ctx.request_discard("the showroom's stage stands on its deck");
        }
    }

    /// The world, by the arrows either side or picked from the list between them.
    fn world_picker(&mut self, ui: &mut egui::Ui, narrow: bool) {
        ui.label(w::caption("WORLD"));
        let count = self.worlds.len().max(1);
        if w::icon_button(ui, w::Glyph::Left, count > 1).on_hover_text(t("The world before")).clicked() {
            self.world = (self.world + count - 1) % count;
        }
        let shown = self.worlds.get(self.world).map_or(String::new(), World::name);
        let width = if narrow { 170.0 } else { 240.0 };
        egui::ComboBox::from_id_salt("enc_world_pick").width(width).selected_text(RichText::new(shown).font(strong(13.0)).color(TEXT_BRIGHT)).truncate().show_ui(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 2.0);
            for (i, world) in self.worlds.iter().enumerate() {
                if ui.selectable_label(i == self.world, world.name()).clicked() {
                    self.world = i;
                }
            }
        });
        if w::icon_button(ui, w::Glyph::Right, count > 1).on_hover_text(t("The next world")).clicked() {
            self.world = (self.world + 1) % count;
        }
    }

    fn time_picker(&mut self, ui: &mut egui::Ui) {
        let times: Vec<(usize, &str)> = TIMES.iter().enumerate().map(|(i, (name, _))| (i, *name)).collect();
        w::segmented(ui, &mut self.time, &times);
    }

    /// The wallpaper's preview: the picture as it will be saved (the unit framed for it, its
    /// words laid out over it), the rest of the screen darkened round it, and its bar along the
    /// foot: the size, the words, the world and the light, CLOSE and SAVE WALLPAPER. Escape
    /// closes it, Enter saves.
    fn draw_preview(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, game: &Game, ty: Option<&UnitType>, action: &mut Action) {
        let screen = ui.max_rect();
        let (width, height) = self.wallpaper_size();
        let rect = preview_rect(screen, bar_height(ctx), width as f32 / height.max(1) as f32);
        let painter = ui.painter();
        // Outside the picture, the screen darkened; round it, a fine line and brass corners.
        let dim = Color32::from_black_alpha(200);
        for band in [
            Rect::from_min_max(screen.min, pos2(screen.max.x, rect.min.y)),
            Rect::from_min_max(pos2(screen.min.x, rect.max.y), screen.max),
            Rect::from_min_max(pos2(screen.min.x, rect.min.y), pos2(rect.min.x, rect.max.y)),
            Rect::from_min_max(pos2(rect.max.x, rect.min.y), pos2(screen.max.x, rect.max.y)),
        ] {
            painter.rect_filled(band, 0.0, dim);
        }
        painter.rect_stroke(rect, 0.0, Stroke::new(1.0, Color32::from_white_alpha(40)), StrokeKind::Outside);
        let arm = (rect.height() * 0.05).clamp(10.0, 28.0);
        let brass = Stroke::new(2.0, AMBER);
        for (corner, dx, dy) in [(rect.left_top(), 1.0, 1.0), (rect.right_top(), -1.0, 1.0), (rect.left_bottom(), 1.0, -1.0), (rect.right_bottom(), -1.0, -1.0)] {
            let c = corner + vec2(-dx * 3.0, -dy * 3.0);
            painter.line_segment([c, c + vec2(dx * arm, 0.0)], brass);
            painter.line_segment([c, c + vec2(0.0, dy * arm)], brass);
        }
        let picture = painter.with_clip_rect(rect);
        match ty {
            Some(ty) if self.words => crate::showroom::wallpaper_words(&picture, rect, game, ty),
            _ => crate::showroom::wallpaper_vignette(&picture, rect),
        }

        // The bar.
        let margin = margin(screen);
        let deck = screen.width() - 2.0 * margin - panel::strip_margin().sum().x;
        let sizes = self.sizes();
        let mut close = false;
        let mut save = false;
        let bar = panel::strip(ctx, "enc_wallpaper_bar", Align2::LEFT_BOTTOM, vec2(margin, -margin), |ui| {
            ui.set_width(deck);
            ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
            // Wide, two rows: the picture (its size, its words, the view) and CLOSE and SAVE, then
            // the world, the light and the size in pixels. Narrow, three: CLOSE and SAVE go to
            // the third, with the size in pixels.
            let wide = deck >= 1000.0;
            let note = if Some((width, height)) == self.screen { tf("{width} × {height} pixels, your screen's size", &[("width", &width), ("height", &height)]) } else { tf("{width} × {height} pixels", &[("width", &width), ("height", &height)]) };
            let note = RichText::new(note).font(strong(12.0)).color(CREAM);
            let hint = RichText::new(t("Drag to turn the unit, the wheel to come closer")).font(body(11.5)).color(STEEL_DIM);
            let mut buttons = |ui: &mut egui::Ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    save = ui.add(ActionButton::primary("SAVE WALLPAPER")).on_hover_text(t("Save this picture in your Pictures folder")).clicked();
                    close = ui.add(ActionButton::secondary("CLOSE")).clicked();
                });
            };
            ui.horizontal(|ui| {
                ui.set_min_height(32.0);
                ui.label(w::caption("SIZE"));
                let mut picked = sizes.iter().position(|s| (s.1, s.2) == (width, height)).unwrap_or(0);
                let options: Vec<(usize, &str)> = sizes.iter().enumerate().map(|(i, s)| (i, s.0.as_str())).collect();
                if w::segmented_raw(ui, &mut picked, &options).changed() {
                    self.size = sizes.get(picked).map(|s| (s.1, s.2));
                }
                ui.add_space(10.0);
                w::toggle(ui, &mut self.words).on_hover_text(t("The unit's name and numbers on the picture, or the picture alone"));
                ui.label(RichText::new(t("Name and numbers")).font(strong(12.5)).color(if self.words { TEXT_BRIGHT } else { STEEL }));
                if wide {
                    ui.add_space(10.0);
                    if ui.add(ActionButton::secondary("RESET VIEW").height(28.0)).on_hover_text(t("Back to the view from in front of the unit")).clicked() {
                        self.reset = true;
                    }
                    buttons(ui);
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.set_min_height(32.0);
                self.world_picker(ui, !wide);
                ui.add_space(14.0);
                self.time_picker(ui);
                if wide {
                    ui.add_space(14.0);
                    ui.label(note.clone());
                    // (How to turn it, where there is room.)
                    if ui.available_width() >= w::galley(ui, hint.clone()).size().x + 8.0 {
                        ui.label(hint.clone());
                    }
                } else {
                    ui.add_space(10.0);
                    if ui.add(ActionButton::secondary("RESET VIEW").height(28.0)).on_hover_text(t("Back to the view from in front of the unit")).clicked() {
                        self.reset = true;
                    }
                }
            });
            if !wide {
                ui.horizontal(|ui| {
                    ui.set_min_height(32.0);
                    ui.label(note.clone());
                    // (How to turn it, where the buttons leave room: each as wide as its lettering
                    // and its padding.)
                    let button = |ui: &egui::Ui, label: &str| (w::galley(ui, RichText::new(t(label)).font(strong(13.5)).extra_letter_spacing(ta_ui::i18n::tracking(0.8))).size().x + 30.0).max(72.0);
                    let buttons_width = button(ui, "SAVE WALLPAPER") + button(ui, "CLOSE") + 24.0;
                    if ui.available_width() >= w::galley(ui, hint.clone()).size().x + buttons_width {
                        ui.label(hint.clone());
                    }
                    buttons(ui);
                });
            }
            ui.min_rect().height() + panel::strip_margin().sum().y
        });
        // (The preview stands on the bar: it follows the bar's height.)
        if (bar - bar_height(ctx)).abs() > 0.5 {
            ctx.data_mut(|d| d.insert_temp(bar_id(), bar));
            ctx.request_discard("the wallpaper's preview stands on its bar");
        }
        let typing = ctx.text_edit_focused();
        if !typing && !egui::Popup::is_any_open(ctx) {
            close |= ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
            save |= ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
        }
        if save {
            *action = Action::Wallpaper { width, height, words: self.words };
        } else if close {
            self.wallpaper_open = false;
            ta_ui::sound::play(ctx, ta_ui::sound::UiSound::Back);
        }
    }

    /// Collects the wallpapers written since the last frame.
    fn poll(&mut self, now: f64) {
        let mut done = Vec::new();
        self.saving.retain(|r| match r.try_recv() {
            Ok(result) => {
                done.push(result);
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => false,
        });
        for result in done {
            self.notice = Some(match result {
                Ok(path) => {
                    log::info!("saved wallpaper {}", path.display());
                    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    (tf("Saved {file} in your Pictures folder", &[("file", &name)]), Some(path), now)
                }
                Err(e) => (tf("The wallpaper could not be saved: {error}", &[("error", &e)]), None, now),
            });
        }
    }

    fn draw_notice(&mut self, ctx: &egui::Context, now: f64) {
        let Some((text, path, at)) = self.notice.clone() else { return };
        if now - at > 10.0 && self.saving.is_empty() {
            self.notice = None;
            return;
        }
        ta_ui::panel::strip(ctx, "enc_notice", Align2::CENTER_TOP, vec2(0.0, 24.0), |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&text).color(TEXT_BRIGHT));
                if let Some(path) = &path {
                    // (What opens a folder opens a picture too, in the viewer the system has for it.)
                    if ui.add(ActionButton::secondary("OPEN").height(28.0)).on_hover_text(t("Open the picture")).clicked() {
                        crate::photo::open_folder(path);
                    }
                    if ui.add(ActionButton::secondary("OPEN FOLDER").height(28.0)).clicked()
                        && let Some(dir) = path.parent()
                    {
                        crate::photo::open_folder(dir);
                    }
                }
            });
        });
    }
}

/// A choice in a filter's drop-down: its name (a side's colour beside it), and how many units
/// it lists.
fn choice(ui: &mut egui::Ui, on: bool, label: &str, colour: Option<Color32>, count: usize) -> bool {
    let width = ui.available_width().max(150.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click());
    let painter = ui.painter();
    if on {
        painter.rect_filled(rect, 2.0, Color32::from_rgba_unmultiplied(196, 160, 72, 40));
    } else if response.hovered() {
        painter.rect_filled(rect, 2.0, Color32::from_white_alpha(12));
    }
    let mut x = rect.min.x + 8.0;
    if let Some(colour) = colour {
        painter.circle(pos2(x + 4.0, rect.center().y), 4.0, colour, Stroke::new(1.0, Color32::from_black_alpha(200)));
        x += 14.0;
    }
    painter.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, label, strong(12.5), if on { AMBER_BRIGHT } else { TEXT_BRIGHT });
    painter.text(pos2(rect.max.x - 8.0, rect.center().y), Align2::RIGHT_CENTER, count.to_string(), body(12.0), if count == 0 { STEEL_DIM } else { STEEL });
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, on, label));
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// Units' portraits for the menus' pages, drawn from their models in the art style on show
/// (`ta_render::portrait`, using the player's own skin): by type, colour and size (none:
/// the model could not be drawn); and those asked for and not drawn yet (the app draws them
/// before the next frame).
#[derive(Default)]
pub struct Portraits {
    drawn: std::collections::HashMap<PortraitKey, Option<egui::TextureId>>,
    wanted: Vec<PortraitKey>,
}

impl Portraits {
    /// The portrait of unit type `type_id` in player colour `colour`, `size` points across:
    /// asked for when it is not drawn yet (none until it is, and when it cannot be).
    pub fn get(&mut self, ctx: &egui::Context, type_id: usize, colour: u32, size: f32) -> Option<UnitPic> {
        if !crate::hud::portraits_wanted() {
            return None;
        }
        let px = size * ctx.pixels_per_point();
        let sizes = crate::hud::PORTRAIT_SIZES;
        let size = sizes.iter().copied().find(|&s| s as f32 >= px * 0.92).unwrap_or(sizes[sizes.len() - 1]);
        let key = (type_id, colour, size, false);
        match self.drawn.get(&key) {
            Some(drawn) => drawn.map(|id| UnitPic { id, portrait: true, allegiance: None }),
            None => {
                if !self.wanted.contains(&key) {
                    self.wanted.push(key);
                }
                ctx.request_repaint();
                None
            }
        }
    }

    /// The portraits asked for since this was last called, to draw (the app does, see
    /// [`Portraits::set`]).
    pub fn take_requests(&mut self) -> Vec<PortraitKey> {
        std::mem::take(&mut self.wanted)
    }

    /// A portrait drawn and registered with the interface's renderer (none: it could not be).
    pub fn set(&mut self, key: PortraitKey, id: Option<egui::TextureId>) {
        self.drawn.insert(key, id);
    }
}

/// Where the picture drawn of a unit with none of its own is kept (by the game data's hash: a
/// mod may change the model; `unitpics2`: drawn from the front left, as TA's own are).
fn drawn_picture_path(assets: &Assets, name: &str) -> PathBuf {
    ta_config::config_dir().join("cache").join("unitpics2").join(format!("{:016x}", assets.data_hash)).join(format!("{name}.png"))
}

/// In place of a unit's picture until it has one: a gunmetal plate lettered with the first
/// letters of its name, as TA lettered a build button whose picture was missing.
pub(crate) fn no_picture(painter: &egui::Painter, rect: egui::Rect, name: &str) {
    painter.add(ta_ui::metal::slab(painter.ctx(), rect, ta_ui::metal::Slab::Steel, 0.0, true));
    let letters: String = name.split_whitespace().filter_map(|w| w.chars().find(|c| c.is_alphanumeric())).take(2).collect::<String>().to_uppercase();
    let galley = painter.layout_no_wrap(letters, display((rect.height() * 0.44).round()), CREAM);
    ta_ui::metal::slab_text(painter, rect.center(), galley, true, CREAM, false);
}

/// The height of a row of the list, and of its picture.
const ROW: f32 = 44.0;
const ROW_PICTURE: f32 = 36.0;
/// The pictures of the details' links: a builder's chip, and a tile of what the unit builds.
const LINK_PICTURE: f32 = 22.0;
const BUILD_PICTURE: f32 = 38.0;

/// One line of `text` cut to `width` with an ellipsis where it runs over.
fn one_line(painter: &egui::Painter, text: String, font: egui::FontId, colour: Color32, width: f32, tracking: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(text, egui::TextFormat { font_id: font, color: colour, extra_letter_spacing: tracking, ..Default::default() });
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(10.0));
    painter.layout_job(job)
}

/// A unit in the list: its picture, its name and what it is (each cut short with an ellipsis
/// where the row is too narrow, whole on hover).
fn unit_row(ui: &mut egui::Ui, ty: &UnitType, pic: Option<UnitPic>, on: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    panel::row_state(&painter, rect, on, response.hovered());
    let pic_rect = egui::Rect::from_min_size(rect.min + vec2(7.0, (ROW - ROW_PICTURE) / 2.0), vec2(ROW_PICTURE, ROW_PICTURE));
    match pic {
        Some(p) => {
            p.paint(&painter, pic_rect, Color32::WHITE);
            painter.rect_stroke(pic_rect, 0.0, egui::Stroke::new(1.0, LINE), egui::StrokeKind::Outside);
        }
        None => no_picture(&painter, pic_rect, &ty.def.display_name),
    }
    let x = pic_rect.max.x + 8.0;
    let width = rect.max.x - x - 6.0;
    let name = one_line(&painter, ty.def.display_name.clone(), strong(13.0), if on { AMBER_BRIGHT } else { TEXT_BRIGHT }, width, 0.0);
    let cut = name.elided;
    let compact = rect.width() < 240.0;
    painter.galley(egui::pos2(x, rect.min.y + if compact { 14.0 } else { 5.0 }), name, TEXT_BRIGHT);
    // What it is, in TA's words; what kind of unit it is where they only say its name again.
    let about = Some(ty.def.description.trim()).filter(|d| !d.is_empty() && !d.eq_ignore_ascii_case(ty.def.display_name.trim()));
    if !compact {
        let line = match about {
            Some(about) => one_line(&painter, about.to_owned(), body(11.5), STEEL, width, 0.0),
            None => one_line(&painter, t(Kind::of(ty).label()).to_owned(), strong(10.5), STEEL_DIM, width, ta_ui::i18n::tracking(1.2)),
        };
        painter.galley(egui::pos2(x, rect.min.y + 23.0), line, STEEL);
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, on, &ty.def.display_name));
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    match (about, cut) {
        (Some(about), true) => response.on_hover_text(format!("{}\n{about}", ty.def.display_name)),
        (Some(about), false) => response.on_hover_text(about),
        (None, true) => response.on_hover_text(&ty.def.display_name),
        (None, false) => response,
    }
}

/// A link to a unit in the details: its picture and name on a chip `width` wide, the name cut
/// short with an ellipsis where it runs over (a builder of the one on show).
fn unit_chip(ui: &mut egui::Ui, ty: &UnitType, pic: Option<UnitPic>, width: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width.min(ui.available_width().max(60.0)), LINK_PICTURE + 6.0), Sense::click());
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let hot = response.hovered();
    painter.rect(rect, 3.0, if hot { Color32::from_white_alpha(16) } else { Color32::from_black_alpha(70) }, Stroke::new(1.0, if hot { AMBER_DIM } else { LINE }), StrokeKind::Inside);
    let pic_rect = Rect::from_min_size(rect.min + vec2(3.0, 3.0), vec2(LINK_PICTURE, LINK_PICTURE));
    match pic {
        Some(p) => p.paint(&painter, pic_rect, Color32::WHITE),
        None => no_picture(&painter, pic_rect, &ty.def.display_name),
    }
    let x = pic_rect.max.x + 7.0;
    let text = one_line(&painter, ty.def.display_name.clone(), strong(12.0), if hot { AMBER_BRIGHT } else { TEXT_BRIGHT }, rect.max.x - x - 6.0, 0.0);
    painter.galley(pos2(x, rect.center().y - text.size().y / 2.0), text, TEXT_BRIGHT);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Link, true, &ty.def.display_name));
    let about = ty.def.description.trim();
    let tip = if about.is_empty() || about.eq_ignore_ascii_case(ty.def.display_name.trim()) { ty.def.display_name.clone() } else { format!("{}
{about}", ty.def.display_name) };
    response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip)
}

/// A link to a unit in the details: its picture as a tile, its name on hover (one the unit on
/// show builds).
fn unit_tile(ui: &mut egui::Ui, ty: &UnitType, pic: Option<UnitPic>) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(BUILD_PICTURE, BUILD_PICTURE), Sense::click());
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    match pic {
        Some(p) => p.paint(&painter, rect, Color32::WHITE),
        None => no_picture(&painter, rect, &ty.def.display_name),
    }
    let hot = response.hovered();
    painter.rect_stroke(rect, 0.0, Stroke::new(if hot { 2.0 } else { 1.0 }, if hot { AMBER } else { LINE }), StrokeKind::Inside);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Link, true, &ty.def.display_name));
    let about = ty.def.description.trim();
    let tip = if about.is_empty() || about.eq_ignore_ascii_case(ty.def.display_name.trim()) { ty.def.display_name.clone() } else { format!("{}\n{about}", ty.def.display_name) };
    response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip)
}

/// Where the unit comes from: the game, an expansion, a mod.
fn provenance(assets: &Assets, ty: &UnitType) -> Option<String> {
    let file = format!("units/{}.fbi", ty.name.to_ascii_lowercase());
    assets.vfs.origin(&file).map(|o| {
        let archive = o.path.file_name().map(|f| f.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        match archive.as_str() {
            a if a.starts_with("cc") => t("Total Annihilation: The Core Contingency").to_string(),
            a if a.starts_with("bt") || a.starts_with("tactics") => t("Total Annihilation: Battle Tactics").to_string(),
            a if a.starts_with("totala") || a.starts_with("rev31") || a.ends_with(".ufo") => t("Total Annihilation").to_string(),
            _ if o.layer != "base" => tf("The mod {name}", &[("name", &o.layer)]),
            _ => t("Total Annihilation").to_string(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preview_keeps_its_shape_over_the_bar() {
        for (w, h) in [(1280.0, 720.0), (853.0, 480.0), (2560.0, 1080.0), (1920.0, 1200.0)] {
            let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(w, h));
            for aspect in [16.0 / 9.0, 21.0 / 9.0, 16.0 / 10.0] {
                let r = preview_rect(screen, BAR, aspect);
                assert!((r.width() / r.height() - aspect).abs() < 0.01, "{w}x{h}: {aspect}");
                assert!(screen.contains_rect(r), "inside the screen");
                assert!(r.max.y <= h - BAR - 12.0 + 0.5, "clear of the bar");
                // As large as it goes: it meets the room's sides or its top and foot.
                assert!(r.width() >= w - 2.0 * 24.0 - 0.5 || r.height() >= h - BAR - 12.0 - 2.0 * 24.0 - 0.5);
            }
        }
    }

    #[test]
    fn keys_step_along_the_list() {
        // (A list of nothing moves nowhere.)
        let mut enc = Encyclopaedia {
            search: String::new(),
            side: None,
            kind: Kind::All,
            selected: None,
            worlds: Vec::new(),
            world: 0,
            time: 1,
            turning: true,
            act: Act::Stand,
            reset: false,
            list_open: false,
            info_open: false,
            asked: None,
            pics: Default::default(),
            unpictured: Default::default(),
            wallpaper_open: false,
            size: None,
            words: true,
            saving: Vec::new(),
            notice: None,
            screen: Some((2560, 1440)),
            scroll_to: None,
            auto_wallpaper: None,
        };
        enc.step(&[], Step::By(1));
        assert_eq!(enc.selected, None);
        // The screen's size is one of the usual ones: no size of its own, and picked at first.
        assert_eq!(enc.sizes().len(), SIZES.len());
        assert_eq!(enc.wallpaper_size(), (2560, 1440));
        enc.screen = Some((3440, 1440));
        assert_eq!(enc.sizes()[0].1, 3440);
        assert_eq!(enc.wallpaper_size(), (3440, 1440));
        enc.size = Some((3840, 2160));
        assert_eq!(enc.wallpaper_size(), (3840, 2160));
        // The preview holds the camera still, and only while it is open.
        assert!(enc.turning() && enc.preview().is_none());
        enc.open_wallpaper();
        assert!(!enc.turning() && enc.take_reset() && !enc.take_reset());
        assert_eq!(enc.preview(), Some((3840.0 / 2160.0, true)));
    }
}
