//! The Conquest pages: choosing a new conquest ([`setup`]), and the galaxy where systems are
//! picked ([`galaxy`]), looked into ([`panels`]) and briefed for battle ([`briefing`]), with
//! each battle reported after it and the campaign's end ([`report`]). The rules are in
//! `conquest.rs`.

mod briefing;
mod galaxy;
mod panels;
mod perks;
mod report;
mod setup;

pub(crate) use galaxy::{paint_perk, paint_planet, paint_space, planet_color, strength_text};

use std::collections::HashMap;

use egui::{Align2, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2, pos2, vec2};
use ta_ui::i18n::{n, t, tf};
use ta_ui::panel::{self, Exit, Panel};
use ta_ui::widgets::ActionButton;

use crate::conquest::{Conquest, Level, Size};
use crate::palette::team_color;
use crate::setup::{Assets, GameSetup, MapEntry};
use crate::theme::*;

/// What the Conquest pages ask the menu to do.
// (One action a frame at most: the size difference between its variants costs nothing.)
#[allow(clippy::large_enum_variant)]
pub enum ConquestAction {
    None,
    Back,
    /// Start the battle for a system.
    Start(GameSetup),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum EmpireTab {
    #[default]
    Progress,
    Veterans,
    Journal,
}

pub struct ConquestPage {
    /// The conquest in progress (read from disk when the page opens).
    pub state: Option<Conquest>,
    selected: Option<usize>,
    /// New conquest choices, and the galaxy they make (drawn from `seed`: the one shown is
    /// the one started).
    side: String,
    level: Level,
    size: Size,
    seed: u64,
    galaxy: Option<((Size, u64), Option<Conquest>)>,
    confirm_abandon: bool,
    load_error: Option<String>,
    error: Option<String>,
    empire_tab: EmpireTab,
    /// The galaxy conquered: its record seen, the galaxy shown again.
    victory_reviewed: bool,
    /// A failed roster write remains retryable and blocks starting a different army.
    pending_deployment: Option<Option<Vec<usize>>>,
    /// The system whose battle is being briefed (ATTACK opens it, LAUNCH starts the battle).
    briefing: Option<usize>,
    /// The battle just fought (the conquest's battle count when it began): its report opens
    /// once the conquest has taken its result in.
    debrief_after: Option<u32>,
    /// The last battle's report is open.
    debrief: bool,
    /// A system just taken, its capture played on the map (since when, on the page's clock).
    capture: Option<(usize, f64)>,
    /// The route to the enemy capital is drawn on the map.
    show_route: bool,
    /// Narrow screens: the details are open over the map.
    sheet: bool,
    /// A promotion's perk put off until later (asked again after the next battle, or from the
    /// empire's panel).
    perk_later: bool,
    pics: HashMap<String, Option<egui::TextureHandle>>,
    /// The sides' emblems from TA's art, by side and size in pixels.
    emblems: HashMap<(String, u32), Option<egui::TextureHandle>>,
    /// The galaxies' planets, each its map's world.
    planets: crate::planet::Planets,
}

impl Default for ConquestPage {
    fn default() -> Self {
        ConquestPage {
            state: None,
            selected: None,
            side: "ARM".into(),
            level: Level::Standard,
            size: Size::Medium,
            seed: new_seed(),
            galaxy: None,
            confirm_abandon: false,
            load_error: None,
            error: None,
            empire_tab: EmpireTab::default(),
            victory_reviewed: false,
            pending_deployment: None,
            briefing: None,
            debrief_after: None,
            debrief: false,
            capture: None,
            show_route: false,
            sheet: false,
            perk_later: false,
            pics: HashMap::new(),
            emblems: HashMap::new(),
            planets: crate::planet::Planets::default(),
        }
    }
}

/// A galaxy's seed: from the clock, or `TA_CONQUEST_SEED` (captures).
fn new_seed() -> u64 {
    std::env::var("TA_CONQUEST_SEED").ok().and_then(|s| s.trim().parse().ok()).unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64))
}

fn animation_time(ctx: &egui::Context) -> f32 {
    if ta_ui::motion::still(ctx) { 0.0 } else { ctx.input(|i| i.time) as f32 }
}

/// Match `Panel::show`'s rounded pipe inset and inner margins.
fn panel_chrome() -> f32 {
    2.0 * (ta_ui::metal::pipe_inset(5.0) + ta_ui::theme::space::L).round()
}

/// Keep panel chrome and a screen margin outside its readable body.
fn page_width(screen_width: f32, preferred: f32) -> f32 {
    (screen_width - panel_chrome() - 32.0).clamp(100.0, preferred)
}

/// A screen too small for the galaxy beside its panel: the map scrolls, the details open
/// over it.
fn compact_galaxy(screen: Vec2) -> bool {
    screen.x < 1100.0 || screen.y < 620.0
}

/// Where the wide page puts its panel on `screen`, and how the galaxy beside it is shown.
fn wide_layout(screen: Rect, time: f32) -> (Rect, galaxy::View) {
    let panel_width = (screen.width() * 0.27).clamp(390.0, 540.0);
    let top = screen.min.y + 112.0;
    let panel = Rect::from_min_max(pos2(screen.max.x - 32.0 - panel_width, top), pos2(screen.max.x - 32.0, screen.max.y - 32.0));
    let bounds = Rect::from_min_max(pos2(screen.min.x + 8.0, screen.min.y + 100.0), pos2(panel.min.x - 16.0, screen.max.y - 74.0));
    let k = galaxy::scale_for(bounds.shrink2(vec2(64.0, 44.0)));
    (panel, galaxy::View { area: bounds.shrink2(vec2(60.0 * k, 42.0 * k)), bounds, k, time })
}

/// The compact map's room within its scrolled canvas: the map, and what it may paint in.
fn compact_map_rects(canvas: Rect) -> (Rect, Rect) {
    let bounds = canvas.shrink(8.0);
    (bounds.shrink2(vec2(42.0, 30.0)), bounds)
}

/// The enemy's strength at system `i`: how many computer players and how good (no more than
/// its map has room for, when the maps are given).
fn enemy_line(c: &Conquest, i: usize, maps: Option<&[MapEntry]>) -> String {
    let (count, difficulty) = c.enemies(i);
    let count = maps.and_then(|maps| maps.iter().find(|m| m.name == c.systems[i].map)).map_or(count, |m| count.min(m.start_positions.saturating_sub(1)).max(1));
    galaxy::strength_text(count, difficulty)
}

impl ConquestPage {
    /// The page with the conquest in progress read from disk.
    pub fn loaded() -> ConquestPage {
        let mut page = ConquestPage::default();
        page.reload();
        page
    }

    /// Reload without replacing an unreadable save with a new galaxy.
    pub fn reload(&mut self) { self.reload_at(&Conquest::path()); }

    fn reload_at(&mut self, path: &std::path::Path) {
        match Conquest::load_at(path) {
            Ok(state) => {
                self.state = state;
                self.load_error = None;
                self.error = None;
                self.pending_deployment = None;
                let n = self.state.as_ref().map_or(0, |c| c.systems.len());
                self.selected = self.selected.filter(|&i| i < n);
                self.briefing = self.briefing.filter(|&i| i < n);
            }
            Err(error) => self.load_error = Some(error.to_string()),
        }
    }

    fn start_at(&mut self, conquest: Conquest, path: &std::path::Path) {
        match conquest.create_at(path) {
            Ok(()) => {
                self.state = Some(conquest);
                self.selected = None;
                self.briefing = None;
                self.debrief = false;
                self.debrief_after = None;
                self.error = None;
                self.load_error = None;
                self.pending_deployment = None;
                self.victory_reviewed = false;
                self.seed = new_seed();
            }
            Err(error) => self.error = Some(tf("Could not save: {error}", &[("error", &error)])),
        }
    }

    fn abandon_at(&mut self, path: &std::path::Path) {
        let result = if self.load_error.is_some() {
            Conquest::delete_unreadable_at(path)
        } else {
            match self.state.as_ref() {
                Some(current) => Conquest::delete_matching_at(path, current),
                None => Conquest::delete_at(path),
            }
        };
        match result {
            Ok(()) => {
                self.state = None;
                self.selected = None;
                self.briefing = None;
                self.debrief = false;
                self.debrief_after = None;
                self.confirm_abandon = false;
                self.load_error = None;
                self.error = None;
                self.galaxy = None;
                self.pending_deployment = None;
                self.seed = new_seed();
            }
            Err(error) => self.error = Some(tf("Could not abandon Conquest: {error}", &[("error", &error)])),
        }
    }

    /// Shows a system's details (captures).
    pub fn select_target(&mut self) {
        self.selected = self.state.as_ref().and_then(|c| (0..c.systems.len()).find(|&i| c.attackable(i)));
    }

    /// Opens the briefing for the first system within reach (captures).
    pub fn brief_target(&mut self) {
        self.select_target();
        self.briefing = self.selected;
    }

    /// Opens the briefing for the defence of the system under attack, if one is (captures).
    pub fn brief_defence(&mut self) {
        self.selected = self.state.as_ref().and_then(|c| c.counter.map(|counter| counter.system));
        self.briefing = self.selected;
    }

    /// Opens the last battle's report (captures).
    pub fn show_report(&mut self) {
        self.debrief = self.state.as_ref().is_some_and(|c| c.last.is_some());
    }

    /// Reads the conquest in progress (the page shows what is on disk).
    pub fn open(&mut self) {
        self.reload();
        self.selected = None;
        self.briefing = None;
        self.debrief = false;
        self.debrief_after = None;
        self.capture = None;
        self.confirm_abandon = false;
        self.sheet = false;
        self.seed = new_seed();
        self.galaxy = None;
        self.empire_tab = EmpireTab::Progress;
        self.victory_reviewed = false;
    }

    /// Opens the page after battle number `battle` of the conquest: its report shows once the
    /// result is in.
    pub fn open_after_battle(&mut self, battle: u32) {
        self.open();
        self.debrief_after = Some(battle);
        self.perk_later = false;
    }

    /// Opens a promotion's choice of perks, if one is due (captures).
    pub fn show_perks(&mut self) {
        self.debrief = false;
        self.perk_later = false;
    }

    fn set_deployment_at(&mut self, expected: &Conquest, deployment: Option<Vec<usize>>, path: &std::path::Path) {
        self.pending_deployment = Some(deployment.clone());
        match Conquest::update_deployment_at(path, expected, deployment) {
            Ok(current) => {
                self.state = Some(current);
                self.pending_deployment = None;
                self.error = None;
            }
            Err(error) => self.error = Some(tf("Could not save veteran selection: {error}", &[("error", &error)])),
        }
    }

    /// The galaxy the choices make (the side and the opposition leave it as it is).
    fn new_galaxy(&mut self, assets: &Assets) -> Option<&Conquest> {
        let key = (self.size, self.seed);
        if self.galaxy.as_ref().is_none_or(|(k, _)| *k != key) {
            self.galaxy = Some((key, Conquest::new(&assets.selectable_maps, &self.side, self.level, self.size, self.seed)));
        }
        self.galaxy.as_ref().and_then(|(_, c)| c.as_ref())
    }

    /// A side's emblem from TA's art (`bitmaps/armbkg.pcx`, `corebkg.pcx`: a shape in one
    /// colour on another), as a white mask `px` pixels across, smoothed down from TA's 256.
    fn emblem(&mut self, ctx: &egui::Context, assets: &Assets, side: &str, px: u32) -> Option<egui::TextureHandle> {
        self.emblems
            .entry((side.to_string(), px))
            .or_insert_with(|| {
                let file = if side.eq_ignore_ascii_case("CORE") { "bitmaps/corebkg.pcx" } else { "bitmaps/armbkg.pcx" };
                let pcx = ta_formats::pcx::PcxImage::parse(&assets.vfs.read_opt(file)?).ok()?;
                let (w, h) = (pcx.width as usize, pcx.height as usize);
                let rgba = pcx.to_rgba();
                let at = |x: usize, y: usize| &rgba[(y * w + x) * 4..(y * w + x) * 4 + 3];
                let ground = at(0, 0).to_vec();
                let on = |x: usize, y: usize| at(x, y) != ground.as_slice();
                // The shape's box (square, round its middle).
                let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
                for y in 0..h {
                    for x in 0..w {
                        if on(x, y) {
                            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
                        }
                    }
                }
                if x1 <= x0 || y1 <= y0 {
                    return None;
                }
                let side_len = (x1 - x0).max(y1 - y0);
                let (cx, cy) = ((x0 + x1) as f32 / 2.0, (y0 + y1) as f32 / 2.0);
                let px = px.clamp(8, 256) as usize;
                let step = side_len as f32 / px as f32;
                // Each pixel the share of the shape under it (a box filter: no mipmaps here).
                let mut pixels = Vec::with_capacity(px * px);
                for j in 0..px {
                    for i in 0..px {
                        let (fx, fy) = (cx - side_len as f32 / 2.0 + i as f32 * step, cy - side_len as f32 / 2.0 + j as f32 * step);
                        let (mut hits, mut total) = (0u32, 0u32);
                        for sy in 0..4 {
                            for sx in 0..4 {
                                let (x, y) = ((fx + (sx as f32 + 0.5) * step / 4.0) as isize, (fy + (sy as f32 + 0.5) * step / 4.0) as isize);
                                total += 1;
                                if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && on(x as usize, y as usize) {
                                    hits += 1;
                                }
                            }
                        }
                        let a = (255 * hits / total) as u8;
                        pixels.push(Color32::from_rgba_premultiplied(a, a, a, a));
                    }
                }
                let image = egui::ColorImage::new([px, px], pixels);
                Some(ctx.load_texture(format!("conquest-emblem-{side}-{px}"), image, egui::TextureOptions::LINEAR))
            })
            .clone()
    }

    /// Paints a side's emblem in `rect`, raised, in `body`'s colour.
    fn paint_emblem(&mut self, ui: &egui::Ui, assets: &Assets, side: &str, rect: Rect, body: Color32, edge: f32) {
        let ppp = ui.ctx().pixels_per_point();
        if let Some(tex) = self.emblem(ui.ctx(), assets, side, (rect.width() * ppp).round() as u32) {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            let painter = ui.painter();
            painter.image(tex.id(), rect.translate(vec2(1.5, 2.0)), uv, Color32::from_black_alpha(200));
            painter.image(tex.id(), rect.translate(vec2(-0.8, -0.8)), uv, Color32::from_white_alpha((60.0 + 60.0 * edge) as u8));
            painter.image(tex.id(), rect, uv, body);
        }
    }

    fn unit_pic(&mut self, ctx: &egui::Context, assets: &Assets, unit: &str) -> Option<egui::TextureHandle> {
        self.pics
            .entry(unit.to_string())
            .or_insert_with(|| {
                let name = unit.to_ascii_lowercase();
                let pcx = ta_formats::pcx::PcxImage::parse(&assets.vfs.read_opt(&format!("unitpics/{name}.pcx"))?).ok()?;
                let img = egui::ColorImage::from_rgba_unmultiplied([pcx.width as usize, pcx.height as usize], &pcx.to_rgba());
                Some(ctx.load_texture(format!("conquest-pic-{name}"), img, egui::TextureOptions::LINEAR))
            })
            .clone()
    }

    pub fn draw(&mut self, ui: &mut egui::Ui, assets: &Assets, portraits: &mut crate::encyclopaedia::Portraits) -> ConquestAction {
        if let Some(error) = self.load_error.clone() {
            return self.load_error_page(ui, &error);
        }
        // The battle just fought is in: its report opens.
        if let (Some(after), Some(c)) = (self.debrief_after, self.state.as_ref())
            && c.battles > after
        {
            self.debrief_after = None;
            self.debrief = c.last.is_some();
        }
        match self.state.clone() {
            None => self.new_conquest(ui, assets),
            Some(c) if c.won && !self.victory_reviewed && !self.debrief => self.campaign_complete(ui, assets, portraits, &c),
            Some(c) => self.galaxy_page(ui, assets, portraits, &c),
        }
    }

    /// An unreadable save: what went wrong, with Retry and explicit abandonment.
    fn load_error_page(&mut self, ui: &mut egui::Ui, error: &str) -> ConquestAction {
        let room = ui.ctx().content_rect();
        let page = Panel::new("CONQUEST").width(page_width(room.width(), 600.0)).max_height(room.height() - 24.0).exit(Exit::Back);
        let response = panel::page(ui.ctx(), "conquest_load_error", page, |ui| {
            ui.add(egui::Label::new(RichText::new(tf("Could not load Conquest: {error}", &[("error", &error)])).color(RED)).wrap());
            if let Some(error) = &self.error { ui.add(egui::Label::new(RichText::new(error).color(RED)).wrap()); }
            ui.horizontal_wrapped(|ui| {
                if ui.add(ActionButton::primary("Retry")).clicked() { self.reload(); }
                if self.confirm_abandon {
                    ui.label(t("Abandon this conquest for good?"));
                    if ui.add(ActionButton::danger("ABANDON")).clicked() { self.abandon_at(&Conquest::path()); }
                    if ui.add(ActionButton::secondary("KEEP IT")).clicked() { self.confirm_abandon = false; }
                } else if ui.add(ActionButton::secondary("ABANDON CONQUEST")).clicked() { self.confirm_abandon = true; }
            });
        });
        if response.exit { ConquestAction::Back } else { ConquestAction::None }
    }

    fn galaxy_page(&mut self, ui: &mut egui::Ui, assets: &Assets, portraits: &mut crate::encyclopaedia::Portraits, c: &Conquest) -> ConquestAction {
        let mut action = ConquestAction::None;
        let ctx = ui.ctx().clone();
        let screen = ui.max_rect();
        let time = animation_time(&ctx);
        self.selected = self.selected.filter(|&i| i < c.systems.len());
        galaxy::paint_space(ui.painter(), screen, time);
        ta_ui::motion::next_frame(&ctx, std::time::Duration::from_millis(33));
        if compact_galaxy(screen.size()) {
            self.compact_page(ui, assets, portraits, c, time, &mut action);
        } else {
            self.wide_page(ui, assets, portraits, c, screen, time, &mut action);
        }
        if let Some(i) = self.briefing.filter(|&i| i < c.systems.len())
            && let Some(start) = self.briefing_modal(&ctx, assets, portraits, c, i)
        {
            action = start;
        }
        if self.debrief {
            self.debrief_modal(&ctx, assets, portraits, c);
        } else if self.briefing.is_none() && !self.perk_later && !c.won && c.perks_due() > 0 {
            // A promotion's perk, after the battle's report.
            self.perk_modal(&ctx, c);
        }
        action
    }

    /// The galaxy beside its panel: the header over it, the panel at the right, the legend and
    /// the campaign's controls under the map.
    #[allow(clippy::too_many_arguments)]
    fn wide_page(&mut self, ui: &mut egui::Ui, assets: &Assets, portraits: &mut crate::encyclopaedia::Portraits, c: &Conquest, screen: Rect, time: f32, action: &mut ConquestAction) {
        let ctx = ui.ctx().clone();
        let (panel_rect, view) = wide_layout(screen, time);
        let (top, bounds) = (panel_rect.min.y, view.bounds);
        let input = self.draw_galaxy(ui, c, view, true);
        self.take_input(input);

        // Header: the title and the campaign's tally; the last battle; MAIN MENU.
        let painter = ui.painter().clone();
        let title = panel::shadowed_title(&painter, pos2(screen.min.x + 64.0, screen.min.y + 34.0), "CONQUEST", 44.0, TEXT_BRIGHT);
        let tally = tf("{side} · {held} OF {total} SYSTEMS · {battles} BATTLES, {wins} WON", &[("side", &c.side), ("held", &c.held()), ("total", &c.systems.len()), ("battles", &c.battles), ("wins", &c.wins)]);
        let tally = painter.text(pos2(title.min.x + 2.0, title.max.y + 4.0), Align2::LEFT_TOP, tally, strong(12.5), AMBER);
        let back = egui::Area::new(egui::Id::new("conquest_back")).anchor(Align2::RIGHT_TOP, [-32.0, 34.0]).show(&ctx, |ui| {
            if panel::back_button(ui, "MAIN MENU").clicked() {
                *action = ConquestAction::Back;
            }
        });
        if let Some(r) = &c.last {
            let header = title.union(tally);
            let slot = Rect::from_min_max(pos2(header.max.x + 32.0, screen.min.y + 30.0), pos2(back.response.rect.min.x - 32.0, top - 14.0));
            panel::strip_in(&ctx, "conquest_report", slot, view.area.center().x, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let (head, color) = report_head(c, r);
                    ui.label(RichText::new(t("LAST BATTLE")).font(strong(10.5)).color(STEEL).extra_letter_spacing(ta_ui::i18n::tracking(1.2)));
                    ui.label(RichText::new(head).font(strong(14.0)).color(color).extra_letter_spacing(ta_ui::i18n::tracking(1.2)));
                    if ui.add(ActionButton::secondary("REPORT").height(28.0)).clicked() {
                        self.debrief = true;
                    }
                });
            });
        }

        // The panel.
        egui::Area::new(egui::Id::new("conquest_side")).fixed_pos(panel_rect.min).show(&ctx, |ui| {
            ui.set_width(panel_rect.width());
            let height = panel_rect.height() - self.bar_height(c);
            self.details(ui, assets, portraits, c, panel_rect.width() - panel_chrome(), Some(height));
            self.action_bar(ui, assets, c, panel_rect.width());
        });

        // Under the map: what its marks mean, and the campaign's own controls.
        egui::Area::new(egui::Id::new("conquest_footer")).anchor(Align2::LEFT_BOTTOM, [64.0, -30.0]).show(&ctx, |ui| {
            ui.set_max_width(bounds.width() - 64.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 18.0;
                self.campaign_controls(ui, c);
                legend(ui, c);
            });
        });
    }

    /// Narrow screens: the header, then the map (scrolling sideways when it needs the room);
    /// the details open over its foot when a system is chosen or the empire is asked for.
    fn compact_page(&mut self, ui: &mut egui::Ui, assets: &Assets, portraits: &mut crate::encyclopaedia::Portraits, c: &Conquest, time: f32, action: &mut ConquestAction) {
        let ctx = ui.ctx().clone();
        let screen = ui.max_rect();
        egui::Frame::new().inner_margin(16.0).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(t("CONQUEST")).font(display(32.0)).color(TEXT_BRIGHT));
                if panel::back_button(ui, "MAIN MENU").clicked() { *action = ConquestAction::Back; }
                if ui.add(ActionButton::secondary("YOUR EMPIRE")).clicked() {
                    self.selected = None;
                    self.sheet = true;
                }
            });
            ui.add(egui::Label::new(RichText::new(tf("{side} · {held} OF {total} SYSTEMS · {battles} BATTLES, {wins} WON", &[("side", &c.side), ("held", &c.held()), ("total", &c.systems.len()), ("battles", &c.battles), ("wins", &c.wins)])).font(strong(12.5)).color(AMBER)).wrap());
            if let Some(r) = &c.last {
                ui.horizontal_wrapped(|ui| {
                    let (head, color) = report_head(c, r);
                    ui.label(RichText::new(head).font(strong(13.0)).color(color));
                    if ui.add(ActionButton::secondary("REPORT").height(26.0)).clicked() { self.debrief = true; }
                });
            }
            ui.add_space(6.0);
            let map_width = ui.available_width().max(840.0);
            let map_height = (ui.available_height() - 40.0).max(520.0);
            let scroll = egui::ScrollArea::both().id_salt("conquest_map").auto_shrink([false, false]).max_height(ui.available_height() - 40.0);
            ta_ui::scroll::show(ui, scroll, ta_ui::scroll::Behind::Picture, |ui| {
                let (canvas, _) = ui.allocate_exact_size(vec2(map_width, map_height), Sense::hover());
                let (area, bounds) = compact_map_rects(canvas);
                let input = self.draw_galaxy(ui, c, galaxy::View { area, bounds, k: 1.0, time }, true);
                self.take_input(input);
            });
            // (Under the sheet while it is up.)
            if !(self.sheet || self.selected.is_some()) {
                ui.horizontal_wrapped(|ui| {
                    self.campaign_controls(ui, c);
                });
            }
        });
        // The details, over the map's foot (the map's head still in sight above them).
        if self.sheet || self.selected.is_some() {
            let width = (screen.width() - 24.0).min(620.0);
            let height = (screen.height() - 110.0).min(560.0).max(screen.height() * 0.62).max(260.0);
            egui::Area::new(egui::Id::new("conquest_sheet")).anchor(Align2::CENTER_BOTTOM, [0.0, -10.0]).order(egui::Order::Foreground).show(&ctx, |ui| {
                ui.set_width(width);
                self.details(ui, assets, portraits, c, width - panel_chrome(), Some(height - self.bar_height(c)));
                self.action_bar(ui, assets, c, width);
            });
        }
    }

    /// What the player did on the map: a system chosen (the empty space, the empire again), or
    /// one within reach to be briefed.
    fn take_input(&mut self, input: galaxy::MapInput) {
        if let Some(select) = input.select {
            self.selected = select;
            self.confirm_abandon = false;
            self.sheet = select.is_some();
        }
        if let Some(i) = input.brief {
            self.selected = Some(i);
            self.briefing = Some(i);
        }
    }

    /// ABANDON CONQUEST, or for a conquered galaxy NEW CONQUEST, each asked twice.
    fn campaign_controls(&mut self, ui: &mut egui::Ui, c: &Conquest) {
        let (ask, question, confirm) = if c.won {
            (n("NEW CONQUEST"), t("Start a new conquest? This campaign's record will be cleared."), n("START ANEW"))
        } else {
            (n("ABANDON CONQUEST"), t("Abandon this conquest for good?"), n("ABANDON"))
        };
        if self.confirm_abandon {
            ui.label(RichText::new(question).color(TEXT));
            if ui.add(ActionButton::danger(confirm)).clicked() { self.abandon_at(&Conquest::path()); }
            if ui.add(ActionButton::secondary("KEEP IT")).clicked() { self.confirm_abandon = false; }
        } else if ui.add(ActionButton::secondary(ask)).clicked() {
            self.confirm_abandon = true;
        }
        if let Some(error) = &self.error {
            ui.label(RichText::new(error).color(RED));
        }
    }
}

/// A battle report's headline: the system taken, or the defeat there, in its colour.
fn report_head(c: &Conquest, r: &crate::conquest::Report) -> (String, Color32) {
    let name = c.systems.get(r.system).map_or(String::new(), |s| s.name.to_uppercase());
    match (r.defence, r.won) {
        (true, true) => (tf("{name} HELD", &[("name", &name)]), GREEN),
        (true, false) => (tf("{name} LOST", &[("name", &name)]), RED),
        (false, true) => (tf("{name} TAKEN", &[("name", &name)]), GREEN),
        (false, false) => (tf("DEFEAT AT {name}", &[("name", &name)]), RED),
    }
}

/// What the map's marks mean: ours and theirs, those within reach, the capital, our home.
fn legend(ui: &mut egui::Ui, c: &Conquest) {
    // (Each item one widget of its measured width, so that the row wraps between them.)
    let item = |ui: &mut egui::Ui, label: &str, paint: &dyn Fn(&egui::Painter, Pos2)| {
        let text = ta_ui::widgets::galley(ui, RichText::new(t(label)).font(strong(10.5)).color(STEEL).extra_letter_spacing(ta_ui::i18n::tracking(1.0)));
        let (rect, _) = ui.allocate_exact_size(vec2(28.0 + text.size().x, text.size().y.max(18.0)), Sense::hover());
        paint(ui.painter(), pos2(rect.min.x + 11.0, rect.center().y));
        ui.painter().galley(pos2(rect.min.x + 28.0, rect.center().y - text.size().y / 2.0), text, STEEL);
    };
    item(ui, n("YOUR SYSTEMS"), &|p, c| {
        p.circle_filled(c, 5.0, team_color(0).gamma_multiply(0.5));
        p.circle_stroke(c, 6.0, Stroke::new(1.6, team_color(0)));
    });
    item(ui, n("ENEMY SYSTEMS"), &|p, c| {
        p.circle_filled(c, 5.0, team_color(1).gamma_multiply(0.5));
        p.circle_stroke(c, 6.0, Stroke::new(1.6, team_color(1)));
    });
    item(ui, n("WITHIN REACH"), &|p, c| {
        p.circle_stroke(c, 5.0, Stroke::new(1.6, team_color(1)));
        p.circle_stroke(c, 8.5, Stroke::new(1.4, AMBER));
    });
    item(ui, n("THE FRONT"), &|p, c| {
        p.line_segment([c + vec2(-10.0, 5.0), c + vec2(10.0, -5.0)], Stroke::new(1.8, AMBER.gamma_multiply(0.7)));
    });
    item(ui, n("CAPITAL"), &|p, c| galaxy::paint_crown(p, c + vec2(0.0, 9.0), 4.0, 0.8));
    item(ui, n("REWARD"), &|p, c| {
        p.circle(c, 7.5, Color32::from_rgba_unmultiplied(8, 10, 14, 225), Stroke::new(1.0, galaxy::reward_color(crate::conquest::Reward::Mines).gamma_multiply(0.7)));
        galaxy::paint_reward(p, c, 9.4, crate::conquest::Reward::Mines);
    });
    // (What only some wars show.)
    if c.counter.is_some() {
        item(ui, n("UNDER ATTACK"), &|p, c| {
            p.circle_stroke(c, 5.0, Stroke::new(1.6, team_color(0)));
            p.circle_stroke(c, 8.5, Stroke::new(1.8, RED));
        });
    }
    if c.charted().contains(&false) {
        item(ui, n("UNCHARTED"), &|p, c| {
            p.circle_filled(c, 6.5, Color32::from_rgb(52, 56, 66));
            p.text(c, egui::Align2::CENTER_CENTER, "?", strong(10.0), Color32::from_rgb(170, 178, 192));
        });
    }
}

/// A rank's name, as the simulation ranks experience.
fn rank_name(rank: u8) -> &'static str {
    match rank {
        0 => t("RECRUIT"),
        1 => t("VETERAN"),
        2 => t("HARDENED"),
        _ => t("ELITE"),
    }
}

/// The steel of an emblem not ours.
fn metal_steel() -> Color32 {
    Color32::from_rgb(112, 114, 112)
}

/// The title menu's line for the Conquest entry.
pub fn menu_detail(state: Option<&Conquest>) -> String {
    match state {
        Some(c) if c.won => t("Galaxy conquered").to_string(),
        Some(c) => tf("{side} · {held} of {total} systems", &[("side", &c.side), ("held", &c.held()), ("total", &c.systems.len())]),
        None => t("Take the galaxy system by system").to_string(),
    }
}

#[cfg(test)]
mod tests;
