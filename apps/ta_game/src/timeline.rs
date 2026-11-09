//! The Galactic War (Extras → THE GALACTIC WAR): the TA universe's story, in three parts.
//! The CHRONICLE tells it in TA Reborn's own words (`lore::CHAPTERS`), its eras and chapters
//! along a ribbon, each over a painting of the galaxy as it stood then (`galaxy`), which eases
//! from one chapter's galaxy to the next while the chapter is told a line at a time, beside
//! TA's own picture of it where the install has one; PLAY THE CHRONICLE reads it all through.
//! The ARCHIVE holds the war's documents (`archive`). The CAMPAIGNS tell the war mission by
//! mission across their worlds, each with the picture, the briefing and the goals the install
//! holds (read when shown), and the way into it. A chapter leads to the campaigns that fight
//! it, and a campaign back to its chapter.

mod archive;
mod galaxy;

use std::collections::HashMap;

use egui::text::{CCursor, LayoutJob, TextFormat};
use egui::{Align, Color32, FontId, Layout, Rect, RichText, Sense, Stroke, pos2, vec2};
use ta_data::mission::{BriefColor, Campaign, Mission};
use ta_ui::i18n::{n, t, tf, tracking};
use ta_ui::panel::{self, Exit, Panel};
use ta_ui::widgets::{self as w, ActionButton};

use crate::lore::{self, CHAPTERS, DOCUMENTS, ERAS};
use crate::setup::Assets;
use crate::theme::*;

/// How long the galaxy takes to ease from one chapter's to the next (seconds).
const EASE: f64 = 1.6;
/// The widest the chronicle's text runs (its lines kept to a reading length).
const TEXT_WIDTH: f32 = 820.0;
/// The chapter's picture's width.
const PICTURE_WIDTH: f32 = 300.0;

/// The colour a world is marked with on the timeline (TA's planets, by their kind).
fn world_colour(planet: &str) -> Color32 {
    let p = planet.to_ascii_lowercase();
    let has = |k: &str| p.contains(k);
    if has("green") {
        Color32::from_rgb(96, 160, 72)
    } else if has("lush") {
        Color32::from_rgb(70, 130, 60)
    } else if has("archipelago") {
        Color32::from_rgb(60, 150, 160)
    } else if has("water") {
        Color32::from_rgb(56, 104, 190)
    } else if has("lava") {
        Color32::from_rgb(214, 92, 40)
    } else if has("red") {
        Color32::from_rgb(170, 70, 50)
    } else if has("metal") {
        Color32::from_rgb(130, 140, 160)
    } else if has("ice") {
        Color32::from_rgb(170, 210, 230)
    } else if has("slate") {
        Color32::from_rgb(110, 116, 130)
    } else if has("lunar") || has("moon") {
        Color32::from_rgb(190, 190, 180)
    } else if has("wet desert") {
        Color32::from_rgb(190, 170, 110)
    } else if has("desert") {
        Color32::from_rgb(214, 180, 110)
    } else if has("crystal") {
        Color32::from_rgb(150, 100, 200)
    } else if has("acid") {
        Color32::from_rgb(150, 200, 60)
    } else {
        STEEL
    }
}

/// The colour of each era's band on the chronicle's ribbon.
const ERA_COLOURS: [Color32; 5] = [
    Color32::from_rgb(214, 186, 112),
    Color32::from_rgb(196, 132, 96),
    galaxy::CORE_RED,
    Color32::from_rgb(255, 150, 70),
    AMBER,
];

/// The chronicle's chapter a campaign fights (TA's own campaigns, by their ids).
fn chapter_of(campaign: &str) -> Option<usize> {
    let id = campaign.to_ascii_lowercase();
    let title = if id.contains("krogoth") {
        "THE KROGOTH"
    } else if id.contains("contingency") {
        "THE CORE CONTINGENCY"
    } else if id == "arm campaign" || id == "core campaign" {
        "TOTAL ANNIHILATION"
    } else {
        return None;
    };
    CHAPTERS.iter().position(|c| c.title == title)
}

/// How long PLAY THE CHRONICLE stays on a chapter: its telling, and the time to read it.
fn dwell(text: &str) -> f64 {
    let chars = text.chars().count() as f64;
    chars / lore::TELLING + 4.0 + chars / 22.0
}

/// The page's parts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Part {
    #[default]
    Chronicle,
    Archive,
    Campaigns,
}

/// What the timeline asks of the menu.
pub enum TimelineAction {
    None,
    Back,
    /// Open this campaign (its id) at this mission.
    Play { campaign: String, mission: usize },
}

#[derive(Default)]
pub struct Timeline {
    part: Part,
    /// The chronicle's chapter shown; the one its galaxy eases from, and since when (seconds
    /// of the interface's clock).
    chapter: usize,
    from: usize,
    since: f64,
    /// Since when the chapter has been told (`None`: shown whole).
    telling: Option<f64>,
    /// PLAY THE CHRONICLE: since when the chapter shown has been up.
    playing: Option<f64>,
    /// The archive's document open, and since when it has been told.
    document: usize,
    document_telling: Option<f64>,
    /// The campaign shown (of those with a story) and its mission.
    track: usize,
    selected: usize,
    missions: HashMap<String, Option<Mission>>,
    briefings: HashMap<String, Vec<(BriefColor, String)>>,
    glamour: HashMap<String, Option<egui::TextureHandle>>,
    planets: crate::planet::Planets,
    /// The interface's pass when the page was last drawn (opened again, it is told again).
    drawn: Option<u64>,
    /// Captures: `TA_TIMELINE` read once; `TA_TIMELINE_NEXT`'s step to the next chapter, when.
    primed: bool,
    next_at: Option<f64>,
}

impl Timeline {
    fn mission(&mut self, assets: &Assets, file: &str) -> Option<Mission> {
        self.missions.entry(file.to_string()).or_insert_with(|| Mission::load(&assets.vfs, file)).clone()
    }

    fn glamour(&mut self, ctx: &egui::Context, assets: &Assets, name: &str) -> Option<egui::TextureHandle> {
        if name.is_empty() {
            return None;
        }
        self.glamour
            .entry(name.to_string())
            .or_insert_with(|| {
                // (Cleaned of its palette's dither.)
                let color = crate::picture::read(&assets.vfs, &format!("bitmaps/glamour/{}.pcx", name.to_ascii_lowercase()))?;
                Some(ctx.load_texture(format!("timeline-{name}"), color, egui::TextureOptions::LINEAR))
            })
            .clone()
    }

    /// Shows chapter `k`, its galaxy easing to it from the one shown, its text told.
    fn show_chapter(&mut self, k: usize, now: f64) {
        let k = k.min(CHAPTERS.len() - 1);
        if k != self.chapter {
            self.from = self.chapter;
            self.chapter = k;
            self.since = now;
        }
        self.telling = Some(now);
    }

    /// Captures: `TA_TIMELINE=chronicle|archive|<campaign>,<n>` (a campaign by its number on
    /// the CAMPAIGNS tabs, from 1; 0 is the chronicle) shows that chapter, document or
    /// mission, told whole; `TA_TIMELINE_NEXT=<seconds>` steps the chronicle on to the next
    /// chapter after that many (its galaxy easing to it, its text told).
    fn prime(&mut self, now: f64) -> bool {
        let Ok(value) = std::env::var("TA_TIMELINE") else { return false };
        self.next_at = std::env::var("TA_TIMELINE_NEXT").ok().and_then(|v| v.trim().parse::<f64>().ok()).map(|s| now + s);
        let (part, node) = value.split_once(',').unwrap_or((value.as_str(), "0"));
        let node: usize = node.trim().parse().unwrap_or(0);
        match part.trim() {
            "archive" => (self.part, self.document) = (Part::Archive, node.min(DOCUMENTS.len() - 1)),
            "chronicle" | "0" => {
                (self.part, self.chapter) = (Part::Chronicle, node.min(CHAPTERS.len() - 1));
                self.from = self.chapter;
            }
            number => {
                let Ok(track) = number.parse::<usize>() else { return false };
                (self.part, self.track, self.selected) = (Part::Campaigns, track.saturating_sub(1), node);
            }
        }
        true
    }

    pub fn draw(&mut self, ui: &mut egui::Ui, assets: &Assets, progress: &ta_config::CampaignProgress) -> TimelineAction {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        let primed = !std::mem::replace(&mut self.primed, true) && self.prime(now);
        // Opened (again): its chapter and document told from the start, its galaxy there already.
        let pass = ctx.cumulative_pass_nr();
        let opened = self.drawn.is_none_or(|drawn| pass > drawn + 1);
        self.drawn = Some(pass);
        if opened && !primed {
            self.telling = Some(now);
            self.document_telling = Some(now);
            self.from = self.chapter;
            self.playing = None;
        }
        if opened {
            self.since = now - EASE;
        }
        let mut action = TimelineAction::None;
        // The campaigns with a story (Battle Tactics' sets of battles have none).
        let campaigns: Vec<&Campaign> = assets.campaigns.iter().filter(|c| !c.title.to_ascii_lowercase().starts_with("battle tactics")).collect();
        self.track = self.track.min(campaigns.len().saturating_sub(1));
        let page = Panel::new("THE GALACTIC WAR").subtitle(t("THE STORY OF THE TA UNIVERSE")).width(1240.0).exit(Exit::Back);
        let response = panel::page(&ctx, "timeline", page, |ui| {
            ui.horizontal(|ui| {
                let before = self.part;
                // (ARCHIVES, as "ARCHIVE" alone is a mod's file.)
                let mut parts = vec![(Part::Chronicle, n("CHRONICLE")), (Part::Archive, n("ARCHIVES"))];
                if !campaigns.is_empty() {
                    parts.push((Part::Campaigns, n("CAMPAIGNS")));
                }
                w::segmented(ui, &mut self.part, &parts);
                if self.part != before {
                    self.playing = None;
                    match self.part {
                        Part::Chronicle => {
                            self.telling = Some(now);
                            (self.from, self.since) = (self.chapter, now - EASE);
                        }
                        Part::Archive => self.document_telling = Some(now),
                        Part::Campaigns => {}
                    }
                }
                if self.part == Part::Chronicle {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| self.chronicle_controls(ui, now));
                }
            });
            ui.add_space(10.0);
            match self.part {
                Part::Chronicle => {
                    if let Some(track) = self.chronicle(ui, assets, &campaigns, progress, now) {
                        (self.part, self.track, self.selected) = (Part::Campaigns, track, progress.unlocked(&campaigns[track].id));
                    }
                }
                Part::Archive => {
                    let height = panel::fit_height(&ctx, 330.0, 420.0, 640.0);
                    archive::draw(ui, &mut self.document, &mut self.document_telling, now, height);
                }
                Part::Campaigns => {
                    if let Some(chapter) = self.campaign(ui, assets, progress, &campaigns, &mut action) {
                        self.part = Part::Chronicle;
                        self.playing = None;
                        self.show_chapter(chapter, now);
                        (self.from, self.since) = (self.chapter, now - EASE);
                    }
                }
            }
        });
        if response.exit {
            action = TimelineAction::Back;
        }
        action
    }

    /// The chronicle's way through: the chapter before and after, and PLAY THE CHRONICLE (or
    /// STOP), right to left.
    fn chronicle_controls(&mut self, ui: &mut egui::Ui, now: f64) {
        ui.spacing_mut().item_spacing.x = 8.0;
        let playing = self.playing.is_some();
        let play = if playing { n("STOP") } else { n("PLAY THE CHRONICLE") };
        if ui.add(ActionButton::primary(play).height(32.0)).clicked() {
            if playing {
                self.playing = None;
            } else {
                // (From the start when at its end.)
                if self.chapter + 1 == CHAPTERS.len() {
                    self.show_chapter(0, now);
                } else {
                    self.telling = Some(now);
                }
                self.playing = Some(now);
            }
        }
        let size = vec2(32.0, 32.0);
        if w::glyph_button(ui, w::Glyph::Right, size, w::Tone::Secondary, self.chapter + 1 < CHAPTERS.len()).on_hover_text(t("The next chapter")).clicked() {
            self.playing = None;
            self.show_chapter(self.chapter + 1, now);
        }
        if w::glyph_button(ui, w::Glyph::Left, size, w::Tone::Secondary, self.chapter > 0).on_hover_text(t("The chapter before")).clicked() {
            self.playing = None;
            self.show_chapter(self.chapter.saturating_sub(1), now);
        }
    }

    /// The chronicle: its ribbon, the galaxy, the chapter told. Returns a campaign to go to
    /// (by its place among `campaigns`).
    fn chronicle(&mut self, ui: &mut egui::Ui, assets: &Assets, campaigns: &[&Campaign], progress: &ta_config::CampaignProgress, now: f64) -> Option<usize> {
        let ctx = ui.ctx().clone();
        ctx.request_repaint();
        // PLAY THE CHRONICLE: on to the next chapter once this one has been read.
        let mut read = None;
        if let Some(start) = self.playing {
            let long = dwell(t(CHAPTERS[self.chapter].text));
            read = Some(((now - start) / long).clamp(0.0, 1.0) as f32);
            if now - start >= long {
                if self.chapter + 1 < CHAPTERS.len() {
                    self.show_chapter(self.chapter + 1, now);
                    self.playing = Some(now);
                    read = Some(0.0);
                } else {
                    (self.playing, read) = (None, None);
                }
            }
        }
        if self.next_at.is_some_and(|at| now >= at) {
            self.next_at = None;
            self.show_chapter(self.chapter + 1, now);
        }
        // Left and right step through it.
        let step = ui.input(|i| i.key_pressed(egui::Key::ArrowRight) as isize - i.key_pressed(egui::Key::ArrowLeft) as isize);
        if step != 0 {
            let to = (self.chapter as isize + step).clamp(0, CHAPTERS.len() as isize - 1) as usize;
            if to != self.chapter {
                self.playing = None;
                self.show_chapter(to, now);
            }
        }
        if let Some(k) = ribbon(ui, self.chapter, read) {
            self.playing = None;
            self.show_chapter(k, now);
        }
        ui.add_space(10.0);
        let chapter = &CHAPTERS[self.chapter];
        let eased = {
            let x = ((now - self.since) / EASE).clamp(0.0, 1.0) as f32;
            x * x * (3.0 - 2.0 * x)
        };
        // The galaxy as it stood, its date at the left and its key at the right.
        let width = ui.available_width();
        let (banner, _) = ui.allocate_exact_size(vec2(width, panel::fit_height(&ctx, 600.0, 240.0, 330.0)), Sense::hover());
        galaxy::paint(ui.painter(), banner, &CHAPTERS[self.from].sky, &chapter.sky, eased, now as f32, &mut self.planets);
        {
            let mut painter = ui.painter_at(banner);
            painter.set_opacity(eased.max(0.15));
            let era = painter.layout_job(LayoutJob::single_section(t(ERAS[chapter.era]).to_string(), TextFormat { font_id: strong(12.5), color: ERA_COLOURS[chapter.era], extra_letter_spacing: tracking(2.0), ..Default::default() }));
            painter.galley(banner.min + vec2(22.0, 20.0), era, AMBER);
            let date = painter.layout(t(chapter.dateline).to_string(), display(36.0), Color32::WHITE, banner.width() * 0.24);
            ta_ui::metal::chrome(&painter, banner.min + vec2(22.0, 40.0), date);
        }
        ui.painter().add(ta_ui::metal::bezel(banner, 5.0, true));
        ui.add_space(14.0);
        // The chapter: TA's picture of it, its title and its telling; the campaigns that fight it.
        let mut go = None;
        let picture = chapter.art.iter().find_map(|stem| self.glamour(&ctx, assets, stem));
        // As tall as the tallest chapter: the page keeps its place from one to the next.
        let text_width = if picture.is_some() { (width - PICTURE_WIDTH - 24.0).min(TEXT_WIDTH) } else { width.min(TEXT_WIDTH) };
        let tallest = CHAPTERS
            .iter()
            .enumerate()
            .map(|(k, c)| {
                let title = ui.painter().layout(t(c.title).to_string(), display(34.0), Color32::WHITE, text_width).size().y;
                let text = ui.painter().layout_job(told_job(t(c.text), usize::MAX, &body(19.0), TEXT_BRIGHT, 28.0, text_width)).size().y;
                let links = if campaigns.iter().any(|campaign| chapter_of(&campaign.id) == Some(k)) { 46.0 } else { 0.0 };
                22.0 + title + 8.0 + text + links
            })
            .fold(0.0, f32::max);
        ui.horizontal_top(|ui| {
            ui.set_min_height(tallest);
            ui.spacing_mut().item_spacing.x = 24.0;
            match &picture {
                Some(tex) => {
                    let s = tex.size_vec2();
                    let (rect, _) = ui.allocate_exact_size(s * (PICTURE_WIDTH / s.x.max(1.0)), Sense::hover());
                    ui.painter().image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE.gamma_multiply(eased.max(0.05)));
                    ui.painter().add(ta_ui::metal::bezel(rect, 5.0, true));
                }
                None => ui.add_space(((ui.available_width() - TEXT_WIDTH) / 2.0 - 24.0).max(0.0)),
            }
            ui.vertical(|ui| {
                ui.set_width(ui.available_width().min(TEXT_WIDTH));
                ui.label(w::caption(&tf("CHAPTER {n} OF {count}", &[("n", &(self.chapter + 1)), ("count", &CHAPTERS.len())])).color(AMBER));
                let title = ui.painter().layout(t(chapter.title).to_string(), display(34.0), Color32::WHITE, ui.available_width());
                let (r, _) = ui.allocate_exact_size(title.size(), Sense::hover());
                ta_ui::metal::chrome(ui.painter(), r.min, title);
                ui.add_space(8.0);
                told_text(ui, t(chapter.text), &mut self.telling, now, body(19.0), TEXT_BRIGHT, 28.0);
                let fought: Vec<usize> = (0..campaigns.len()).filter(|&i| chapter_of(&campaigns[i].id) == Some(self.chapter)).collect();
                if !fought.is_empty() {
                    ui.add_space(16.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(10.0, 8.0);
                        ui.label(w::caption("FIGHT IT").color(AMBER));
                        for i in fought {
                            let c = campaigns[i];
                            let done = progress.unlocked(&c.id).min(c.missions.len());
                            let label = format!("{}  ·  {}/{}", crate::campaign::dashes(&c.title).to_uppercase(), done, c.missions.len());
                            if ui.add(ActionButton::secondary(&label).height(30.0)).on_hover_text(t("Its missions, on the CAMPAIGNS tab")).clicked() {
                                go = Some(i);
                            }
                        }
                    });
                }
            });
        });
        go
    }

    /// A campaign's missions on their worlds. Returns a chapter of the chronicle to go to.
    fn campaign(&mut self, ui: &mut egui::Ui, assets: &Assets, progress: &ta_config::CampaignProgress, campaigns: &[&Campaign], action: &mut TimelineAction) -> Option<usize> {
        let ctx = ui.ctx().clone();
        let mut go = None;
        // The campaigns, by their titles as the game data has them.
        let titles: Vec<String> = campaigns.iter().map(|c| crate::campaign::dashes(&c.title).to_uppercase()).collect();
        let tracks: Vec<(usize, &str)> = titles.iter().enumerate().map(|(i, title)| (i, title.as_str())).collect();
        if w::segmented_raw(ui, &mut self.track, &tracks).changed() {
            self.selected = 0;
        }
        ui.add_space(10.0);
        let &c = campaigns.get(self.track)?;
        // Its missions by their numbers (too many for their names), on their worlds.
        let nodes: Vec<Node> = c
            .missions
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let planet = self.mission(assets, &m.file).map(|m| m.planet).unwrap_or_default();
                let colour = world_colour(&planet);
                Node { label: (i + 1).to_string(), world: planet, colour }
            })
            .collect();
        self.selected = self.selected.min(nodes.len().saturating_sub(1));
        // Left and right step along it.
        if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
            self.selected = (self.selected + 1).min(nodes.len().saturating_sub(1));
        }
        if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
            self.selected = self.selected.saturating_sub(1);
        }
        if let Some(clicked) = strip(ui, &nodes, self.selected, &mut self.planets) {
            self.selected = clicked;
        }
        ui.add_space(14.0);
        // What happened there: TA's picture of it, beside its title and its story.
        let index = self.selected;
        let entry = c.missions.get(index)?;
        let mission = self.mission(assets, &entry.file);
        let unlocked = index <= progress.unlocked(&c.id);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 22.0;
            ui.vertical(|ui| {
                ui.set_width(340.0);
                if let Some(tex) = mission.as_ref().and_then(|m| self.glamour(&ctx, assets, &m.glamour)) {
                    let s = tex.size_vec2();
                    let (rect, _) = ui.allocate_exact_size(s * (340.0 / s.x.max(1.0)), Sense::hover());
                    ui.painter().image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    ui.painter().add(ta_ui::metal::bezel(rect, 5.0, true));
                    ui.add_space(10.0);
                }
                if let Some(m) = &mission {
                    panel::section(ui, "OBJECTIVES");
                    crate::campaign::objectives(ui, &m.victory, &m.defeat, assets);
                }
            });
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                let side = if c.side.to_ascii_uppercase().starts_with("COR") { "CORE" } else { "ARM" };
                ui.horizontal(|ui| {
                    ui.label(w::caption(&tf("{side} · MISSION {n} OF {count}", &[("side", &side), ("n", &(index + 1)), ("count", &c.missions.len())])).color(AMBER));
                    // Back to the chapter of the chronicle it fights.
                    if let Some(chapter) = chapter_of(&c.id) {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let text = tf("THE CHRONICLE: {chapter}", &[("chapter", &t(CHAPTERS[chapter].title))]);
                            let link = ui.add(egui::Label::new(RichText::new(format!("{text}  ›")).font(strong(11.5)).color(AMBER_BRIGHT).extra_letter_spacing(tracking(1.2))).sense(Sense::click()));
                            if link.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t("This campaign's place in the war's story")).clicked() {
                                go = Some(chapter);
                            }
                        });
                    }
                });
                let title = if entry.name.is_empty() { mission.as_ref().map(|m| m.name.clone()).unwrap_or_default() } else { entry.name.clone() };
                w::fitted_title(ui, &crate::campaign::strip_number(&title), 30.0, TEXT_BRIGHT);
                if let Some(m) = &mission
                    && !m.planet.is_empty()
                {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let (orb, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                        let painter = ui.painter().clone();
                        let planet = m.planet.clone();
                        self.planets.paint(&painter, orb.center(), 7.0, &planet, crate::planet::seed_of(&planet), || crate::conquest_ui::paint_planet(&painter, orb.center(), 7.0, crate::conquest_ui::planet_color(&planet)));
                        ui.label(RichText::new(m.planet.to_uppercase()).font(strong(12.0)).color(world_colour(&m.planet)).extra_letter_spacing(tracking(1.2)));
                    });
                }
                ui.add_space(8.0);
                let brief = mission.as_ref().map(|m| self.briefings.entry(m.brief.clone()).or_insert_with(|| ta_data::mission::briefing(&assets.vfs, &m.brief).unwrap_or_default()).clone()).unwrap_or_default();
                panel::well(ui, |ui| {
                    let height = ta_ui::scroll::whole_lines(panel::fit_height(&ctx, 560.0, 140.0, 300.0), crate::campaign::BRIEF_LINE, 8.0);
                    let area = egui::ScrollArea::vertical().id_salt("timeline_brief").max_height(height).auto_shrink([false, true]);
                    ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Well, |ui| {
                        egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            if brief.is_empty() {
                                ui.label(RichText::new(t("No briefing for this mission.")).color(STEEL));
                            } else {
                                crate::campaign::briefing_text(ui, &brief);
                            }
                        });
                    });
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    // (Big and brass, as the campaign's START MISSION is: it starts a battle.)
                    let play = panel::hero_button(ui, "PLAY THIS MISSION", vec2(300.0, 52.0), unlocked);
                    let play = if unlocked { play } else { play.on_hover_text(t("Win the missions before it first")) };
                    if play.clicked() {
                        *action = TimelineAction::Play { campaign: c.id.clone(), mission: index };
                    }
                    if !unlocked {
                        ui.label(RichText::new(t("Locked: the campaign gets here in its own time")).font(body(12.0)).color(STEEL));
                    }
                });
            });
        });
        go
    }
}

/// The characters of `text` shown, its `[[blacked out]]` marks taken out.
fn shown_chars(text: &str) -> usize {
    lore::runs(text).iter().map(|(run, _)| run.chars().count()).sum()
}

/// `text` laid out for `width`, its first `shown` characters seen, the rest laid out unseen
/// (so it keeps its size as it is told); its blacked-out words barred.
fn told_job(text: &str, shown: usize, font: &FontId, color: Color32, line_height: f32, width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    let format = |color: Color32, background: Color32| TextFormat { font_id: font.clone(), color, background, line_height: Some(line_height), ..Default::default() };
    let mut left = shown;
    for (run, barred) in lore::runs(text) {
        let chars = run.chars().count();
        let seen = left.min(chars);
        left -= seen;
        let split = run.char_indices().nth(seen).map_or(run.len(), |(at, _)| at);
        let (told, untold) = run.split_at(split);
        if !told.is_empty() {
            if barred {
                job.append(told, 0.0, format(Color32::from_rgb(8, 8, 8), Color32::from_rgb(8, 8, 8)));
            } else {
                job.append(told, 0.0, format(color, Color32::TRANSPARENT));
            }
        }
        if !untold.is_empty() {
            job.append(untold, 0.0, format(Color32::TRANSPARENT, Color32::TRANSPARENT));
        }
    }
    job
}

/// Shows `text` told a character at a time since `telling` (`None`: whole), a cursor at the
/// end of what is told; a click tells the rest at once. It takes its whole room from the start.
fn told_text(ui: &mut egui::Ui, text: &str, telling: &mut Option<f64>, now: f64, font: FontId, color: Color32, line_height: f32) {
    let total = shown_chars(text);
    let shown = telling.map_or(total, |since| lore::told(total, now - since));
    let galley = ui.painter().layout_job(told_job(text, shown, &font, color, line_height, ui.available_width()));
    let sense = if shown < total { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(galley.size(), sense);
    ui.painter().galley(rect.min, galley.clone(), color);
    if shown < total {
        ui.ctx().request_repaint();
        let at = galley.pos_from_cursor(CCursor::new(shown)).translate(rect.min.to_vec2());
        let blink = ((now * 3.0).fract() < 0.6) as u8 as f32;
        let block = Rect::from_min_size(pos2(at.min.x + 1.0, at.center().y - font.size * 0.55), vec2(font.size * 0.5, font.size * 1.1));
        ui.painter().rect_filled(block, 1.0, AMBER.gamma_multiply(0.4 + 0.6 * blink));
        ui.scroll_to_rect(block, None);
        if response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t("Click to read it all at once")).clicked() {
            *telling = None;
        }
    } else if telling.is_some() {
        *telling = None;
    }
}

/// The chronicle's ribbon: its eras in bands over its chapters' line, each chapter a node
/// with its date over it and its title under it; the one shown in brass, ringed by how far
/// PLAY THE CHRONICLE has read it (`read`). Returns the node clicked.
fn ribbon(ui: &mut egui::Ui, chapter: usize, read: Option<f32>) -> Option<usize> {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 104.0), Sense::hover());
    let painter = ui.painter().clone();
    painter.add(ta_ui::metal::well(rect, INSET));
    let count = CHAPTERS.len();
    let gap = (rect.width() - 40.0) / count as f32;
    let x_of = |i: usize| rect.min.x + 20.0 + gap * (i as f32 + 0.5);
    let line_y = rect.min.y + 56.0;
    // Lettering centred on at most `rows` lines in `width`: a size smaller, and tighter, before
    // it is cut short.
    let fitted = |text: &str, size: f32, color: Color32, width: f32, spacing: f32, rows: usize| {
        let mut galley = None;
        for (size, spacing) in [(size, spacing), (size - 1.0, spacing * 0.3)] {
            let mut job = LayoutJob::single_section(text.to_string(), TextFormat { font_id: strong(size), color, extra_letter_spacing: tracking(spacing), ..Default::default() });
            job.wrap = egui::text::TextWrapping { max_width: width, max_rows: rows, break_anywhere: rows == 1, overflow_character: Some('…') };
            job.halign = Align::Center;
            let laid = painter.layout_job(job);
            let cut = laid.elided;
            galley = Some(laid);
            if !cut {
                break;
            }
        }
        galley.expect("laid out at least once")
    };
    // The eras: a band over each run of their chapters, named.
    for (era, name) in ERAS.iter().enumerate() {
        let Some(first) = CHAPTERS.iter().position(|c| c.era == era) else { continue };
        let last = CHAPTERS.iter().rposition(|c| c.era == era).unwrap_or(first);
        let colour = ERA_COLOURS[era];
        let band = Rect::from_min_max(pos2(x_of(first) - gap * 0.47, rect.min.y + 8.0), pos2(x_of(last) + gap * 0.47, rect.min.y + 26.0));
        let on = CHAPTERS[chapter].era == era;
        painter.rect_filled(band, 2.0, colour.gamma_multiply(if on { 0.45 } else { 0.22 }));
        painter.rect_stroke(band, 2.0, Stroke::new(1.0, colour.gamma_multiply(if on { 1.0 } else { 0.6 })), egui::StrokeKind::Inside);
        let label = fitted(t(name), 9.5, if on { TEXT_BRIGHT } else { TEXT }, band.width() - 6.0, 0.8, 1);
        painter.galley(pos2(band.center().x, band.center().y - label.size().y / 2.0), label, TEXT);
    }
    painter.line_segment([pos2(rect.min.x + 14.0, line_y), pos2(rect.max.x - 14.0, line_y)], Stroke::new(2.0, LINE_HI));
    let mut clicked = None;
    for (k, c) in CHAPTERS.iter().enumerate() {
        let p = pos2(x_of(k), line_y);
        let hit = Rect::from_min_max(pos2(p.x - gap / 2.0, rect.min.y + 28.0), pos2(p.x + gap / 2.0, rect.max.y - 2.0));
        let response = ui.interact(hit, ui.id().with(("chronicle_chapter", k)), Sense::click());
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, k == chapter, t(c.title)));
        let on = k == chapter;
        let colour = ERA_COLOURS[c.era];
        let r = if on { 8.5 } else if response.hovered() { 7.0 } else { 5.5 };
        painter.circle_filled(p, if on { r + 4.5 } else { r + 2.0 }, INSET);
        painter.circle_filled(p, r + 2.0, Color32::from_black_alpha(200));
        painter.circle_filled(p, r, if on { AMBER } else { colour });
        if on {
            painter.circle_stroke(p, r + 4.0, Stroke::new(1.5, AMBER_BRIGHT.gamma_multiply(0.5)));
            if let Some(read) = read {
                // How far it has been read: an arc round from the top.
                let points: Vec<egui::Pos2> = (0..=48).map(|s| {
                    let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * read * s as f32 / 48.0;
                    p + vec2(a.cos(), a.sin()) * (r + 4.0)
                }).collect();
                painter.add(egui::Shape::line(points, Stroke::new(2.2, AMBER_BRIGHT)));
            } else {
                painter.circle_stroke(p, r + 4.0, Stroke::new(1.5, AMBER_BRIGHT));
            }
        }
        let text = if on {
            AMBER_BRIGHT
        } else if response.hovered() {
            TEXT
        } else {
            STEEL
        };
        let date = fitted(t(c.dateline), 9.5, if on { AMBER } else { STEEL_DIM }, gap - 6.0, 0.4, 1);
        painter.galley(pos2(p.x, line_y - 26.0), date, STEEL);
        // Its title centred under it on at most two lines.
        painter.galley(pos2(p.x, line_y + 16.0), fitted(t(c.title), 10.5, text, gap - 6.0, 0.8, 2), text);
        if response.on_hover_text(t(c.title)).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            clicked = Some(k);
        }
    }
    clicked
}

/// A step of a campaign: its label under it, its world (none: no band) and its colour.
struct Node {
    label: String,
    world: String,
    colour: Color32,
}

/// A campaign's track: a line with a node for each mission, its worlds as bands over it, each
/// with the planet; the chosen one in brass. Returns the node clicked.
fn strip(ui: &mut egui::Ui, nodes: &[Node], selected: usize, planets: &mut crate::planet::Planets) -> Option<usize> {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 90.0), Sense::hover());
    let painter = ui.painter().clone();
    painter.add(ta_ui::metal::well(rect, INSET));
    let n = nodes.len().max(1);
    let gap = (rect.width() - 60.0) / n as f32;
    let x_of = |i: usize| rect.min.x + 30.0 + gap * (i as f32 + 0.5);
    let line_y = rect.min.y + 54.0;
    painter.line_segment([pos2(rect.min.x + 18.0, line_y), pos2(rect.max.x - 18.0, line_y)], Stroke::new(2.0, LINE_HI));
    // The worlds: a band over each run of missions on the same kind of world, its planet at
    // its start and its name on it.
    let mut i = 0;
    while i < nodes.len() {
        let world = &nodes[i].world;
        let mut j = i;
        while j + 1 < nodes.len() && nodes[j + 1].world.eq_ignore_ascii_case(world) {
            j += 1;
        }
        if !world.is_empty() {
            let band = Rect::from_min_max(pos2(x_of(i) - gap * 0.45, rect.min.y + 8.0), pos2(x_of(j) + gap * 0.45, rect.min.y + 30.0));
            painter.rect_filled(band, 2.0, nodes[i].colour.gamma_multiply(0.35));
            painter.rect_stroke(band, 2.0, Stroke::new(1.0, nodes[i].colour.gamma_multiply(0.8)), egui::StrokeKind::Inside);
            let orb = pos2(band.min.x + 11.0, band.center().y);
            let room = band.width() - 26.0;
            if room > 10.0 {
                planets.paint(&painter, orb, 7.5, world, crate::planet::seed_of(world), || crate::conquest_ui::paint_planet(&painter, orb, 7.5, crate::conquest_ui::planet_color(world)));
            }
            let label = painter.layout_no_wrap(world.to_uppercase(), strong(10.0), TEXT_BRIGHT);
            if label.size().x <= room - 2.0 {
                let x = band.min.x + 22.0 + (room - label.size().x) / 2.0;
                painter.galley(pos2(x, band.center().y - label.size().y / 2.0), label, TEXT_BRIGHT);
            }
        }
        i = j + 1;
    }
    let mut clicked = None;
    for (k, node) in nodes.iter().enumerate() {
        let c = pos2(x_of(k), line_y);
        // (Its whole column answers, its label too.)
        let hit = Rect::from_min_max(pos2(c.x - gap.max(16.0) / 2.0, line_y - 16.0), pos2(c.x + gap.max(16.0) / 2.0, rect.max.y - 4.0));
        let response = ui.interact(hit, ui.id().with(("timeline_node", k)), Sense::click());
        let on = k == selected;
        let r = if on { 9.0 } else if response.hovered() { 7.5 } else { 6.0 };
        // The well's own face under the node: the line stops at its socket, and at the halo
        // round the one chosen.
        painter.circle_filled(c, if on { r + 4.5 } else { r + 2.0 }, INSET);
        painter.circle_filled(c, r + 2.0, Color32::from_black_alpha(200));
        painter.circle_filled(c, r, if on { AMBER } else { node.colour });
        if on {
            painter.circle_stroke(c, r + 4.0, Stroke::new(1.5, AMBER_BRIGHT));
        }
        let color = if on {
            AMBER_BRIGHT
        } else if response.hovered() {
            TEXT
        } else {
            STEEL
        };
        painter.text(pos2(c.x, line_y + 22.0), egui::Align2::CENTER_CENTER, &node.label, strong(10.5), color);
        let response = if node.world.is_empty() { response.on_hover_text(&node.label) } else { response.on_hover_text(node.world.to_uppercase()) };
        if response.clicked() {
            clicked = Some(k);
        }
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ta_campaigns_lead_to_their_chapters() {
        for id in ["arm campaign", "core campaign", "Arm Campaign - Core Contingency", "core campaign - core contingency", "krogoth encounter"] {
            assert!(chapter_of(id).is_some(), "{id}");
        }
        assert_eq!(chapter_of("arm campaign"), chapter_of("core campaign"));
        assert_ne!(chapter_of("arm campaign"), chapter_of("arm campaign - core contingency"));
        assert_eq!(chapter_of("a mod's own campaign"), None);
    }

    #[test]
    fn a_told_text_keeps_its_size_and_hides_its_barred_words() {
        let text = "The gates are to be held at [[any cost]]. End.";
        assert_eq!(shown_chars(text), text.len() - 4);
        let font = FontId::proportional(16.0);
        let half = told_job(text, 10, &font, Color32::WHITE, 20.0, 400.0);
        let whole = told_job(text, usize::MAX, &font, Color32::WHITE, 20.0, 400.0);
        // The same characters laid out either way, only their colours told apart.
        assert_eq!(half.text, whole.text);
        assert_eq!(whole.text, "The gates are to be held at any cost. End.");
        assert!(half.sections.iter().any(|s| s.format.color == Color32::TRANSPARENT));
        let barred = whole.sections.iter().find(|s| &whole.text[s.byte_range.start.0..s.byte_range.end.0] == "any cost").expect("the barred words are a section");
        assert_eq!(barred.format.color, barred.format.background);
    }

    #[test]
    fn telling_splits_on_characters_not_bytes() {
        let font = FontId::proportional(16.0);
        let job = told_job("Ça va – très bien", 4, &font, Color32::WHITE, 20.0, 400.0);
        assert_eq!(job.text, "Ça va – très bien");
        let first = &job.sections[0].byte_range;
        assert_eq!(&job.text[first.start.0..first.end.0], "Ça v");
    }

    #[test]
    fn the_chronicle_plays_long_enough_to_read() {
        for c in &CHAPTERS {
            let chars = c.text.chars().count() as f64;
            assert!(dwell(c.text) > chars / lore::TELLING + 3.0, "{}", c.title);
        }
    }
}
