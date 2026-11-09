//! Local commander identity editor, shared by first-launch setup and the launcher.
use crate::tooltip::TooltipExt as _;
use egui::{Color32, Rect, Response, RichText, Sense, TextureHandle, Ui, pos2, vec2};
use ta_config::profiles::{Allegiance, Career, Profile, Rank};
use crate::i18n::{grouped, n, t, tf};
use crate::theme::*;
use crate::widgets::{self, ActionButton};

mod achievements;
mod career;
mod hero;
mod overview;
mod pictures;
mod record;
mod toast;
pub use achievements::{AchievementsView, Sort as AchievementSort, Status as AchievementStatus};
pub use career::{CampaignSummary, CareerAction, CareerData, CareerTab, CareerView, CareerWindow};
pub use hero::{HeroSize, hero};
pub use overview::xp_shares;
pub use record::{BattleRecord, battle_record, battle_record_line};
pub use toast::CareerNotices;

pub struct ProfileDraft {
    pub name: String,
    /// The line under the name (trimmed; empty for none).
    pub motto: String,
    pub allegiance: Allegiance,
    /// A normalized PNG when the picture was changed; None keeps an existing picture.
    pub avatar_png: Option<Vec<u8>>,
    pub clear_avatar: bool,
}

pub struct ProfileEditor {
    pub name: String,
    pub motto: String,
    pub allegiance: Option<Allegiance>,
    pub error: Option<String>,
    editing: bool,
    avatar_png: Option<Vec<u8>>,
    avatar: Option<TextureHandle>,
    clear_avatar: bool,
    focus_name: bool,
    initial: (String, String, Option<Allegiance>),
}

impl ProfileEditor {
    pub fn new(existing: Option<&Profile>) -> Self {
        let name = existing.map_or(String::new(), |p| p.name.clone());
        let motto = existing.map_or(String::new(), |p| p.motto.clone());
        let allegiance = existing.map(|p| p.allegiance);
        Self {
            initial: (name.clone(), motto.clone(), allegiance), name, motto, allegiance,
            error: None, editing: existing.is_some(), avatar_png: None, avatar: None,
            clear_avatar: false, focus_name: true,
        }
    }

    /// Whether closing this editor would discard a draft, including a changed picture.
    pub fn changed(&self) -> bool {
        self.name != self.initial.0 || self.motto != self.initial.1 || self.allegiance != self.initial.2
            || self.avatar_png.is_some() || self.clear_avatar
    }

    pub fn show(&mut self, ui: &mut Ui) -> Option<ProfileDraft> {
        ui.spacing_mut().item_spacing = vec2(10.0, 8.0);
        if let Some(error) = &self.error {
            ui.add(egui::Label::new(RichText::new(error).color(RED)).wrap());
        }
        // Panel supplies a bounded body. Only the form scrolls on short windows;
        // the primary action keeps its own space below it and remains reachable.
        let form_height = (ui.available_height() - 60.0).max(64.0);
        let area = egui::ScrollArea::vertical().id_salt("profile_identity_form")
            .max_height(form_height).min_scrolled_height(64.0).auto_shrink([false, true]);
        crate::scroll::show(ui, area, crate::scroll::Behind::Plate, |ui| {
            ui.add(egui::Label::new(RichText::new(t("Your profile keeps your settings, saves and career separate from other players on this device."))
                .color(STEEL).size(13.0)).wrap());
            ui.add_space(8.0);
            let texture = self.avatar.clone().or_else(|| {
                (self.editing && !self.clear_avatar).then(|| crate::portrait::texture(ui.ctx())).flatten()
            });
            if ui.available_width() >= 340.0 {
                ui.horizontal_top(|ui| {
                    avatar_picture(ui, texture.as_ref(), self.allegiance, 96.0)
                        .hover_tip(t("Your picture"));
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        self.name_field(ui);
                        self.picture_actions(ui, texture.is_some());
                    });
                });
            } else {
                self.name_field(ui);
                ui.horizontal(|ui| {
                    ui.add_space(((ui.available_width() - 72.0) / 2.0).max(0.0));
                    avatar_picture(ui, texture.as_ref(), self.allegiance, 72.0)
                        .hover_tip(t("Your picture"));
                });
                self.picture_actions(ui, texture.is_some());
            }
            ui.add(egui::Label::new(RichText::new(t("Picture is optional. Your emblem is used by default."))
                .color(STEEL).size(12.0)).wrap());
            ui.add_space(8.0);
            ui.label(widgets::caption("MOTTO"));
            let motto = ui.add(egui::TextEdit::singleline(&mut self.motto)
                .char_limit(ta_config::profiles::MAX_MOTTO_CHARS).hint_text(t("A line of your own under your name (optional)"))
                .desired_width(ui.available_width()).min_size(vec2(0.0, 32.0))
                .margin(egui::Margin::symmetric(10, 6)).vertical_align(egui::Align::Center));
            ui.ctx().accesskit_node_builder(motto.id, |node| node.set_label(t("MOTTO")));
            if motto.changed() { self.error = None; }
            ui.add_space(8.0);
            ui.label(widgets::caption("ALLEGIANCE"));
            let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
            ui.horizontal(|ui| {
                for side in [Allegiance::Arm, Allegiance::Core] {
                    if faction_choice(ui, side, self.allegiance == Some(side), width).clicked() {
                        self.allegiance = Some(side);
                        self.error = None;
                    }
                }
            });
            ui.add(egui::Label::new(RichText::new(t("Allegiance adds a faction badge to your avatar. You can play either army."))
                .color(STEEL).size(12.0)).wrap());
            ui.add_space(4.0);
        });
        ui.add(egui::Separator::default().spacing(1.0));
        // What is still needed, said under the button rather than left to a grey button.
        let missing = if ta_config::profiles::clean_name(&self.name).is_err() {
            Some(t("Type your commander's name (1 to 24 characters)"))
        } else if ta_config::profiles::clean_motto(&self.motto).is_err() {
            Some(t("Shorten the motto"))
        } else if self.allegiance.is_none() {
            Some(t("Choose Arm or Core"))
        } else {
            None
        };
        let valid = missing.is_none();
        let label = if self.editing { n("SAVE PROFILE") } else { n("CREATE PROFILE") };
        let button = ui.add_enabled(valid, ActionButton::primary(label).size(ui.available_width(), 40.0));
        if let Some(missing) = missing {
            ui.label(RichText::new(missing).color(AMBER).size(13.0));
        }
        if button.clicked() {
            return Some(ProfileDraft { name: self.name.trim().to_owned(), motto: self.motto.trim().to_owned(), allegiance: self.allegiance.unwrap(),
                avatar_png: self.avatar_png.clone(), clear_avatar: self.clear_avatar });
        }
        None
    }

    fn name_field(&mut self, ui: &mut Ui) -> Response {
        ui.label(widgets::caption("COMMANDER NAME"));
        let name_field = ui.add(egui::TextEdit::singleline(&mut self.name)
            .char_limit(ta_config::limits::PLAYER_NAME_CHARS).hint_text(t("Commander"))
            .desired_width(ui.available_width()).min_size(vec2(0.0, 36.0))
            .margin(egui::Margin::symmetric(10, 6)).vertical_align(egui::Align::Center));
        ui.ctx().accesskit_node_builder(name_field.id, |node| node.set_label(t("COMMANDER NAME")));
        if self.focus_name && !ui.is_sizing_pass() {
            name_field.request_focus();
            self.focus_name = false;
        }
        if name_field.changed() { self.error = None; }
        name_field
    }

    fn picture_actions(&mut self, ui: &mut Ui, has_picture: bool) {
        ui.horizontal_wrapped(|ui| {
            // Wrap translations as a pair of complete controls; never squeeze their labels.
            if ui.add(ActionButton::secondary("CHOOSE PICTURE…").height(30.0)).clicked()
                && let Some(file) = rfd::FileDialog::new().set_title(t("Your picture"))
                    .add_filter(t("Pictures"), &["png", "jpg", "jpeg"]).pick_file()
            {
                match crate::portrait::read_from(&file) {
                    Ok(bytes) => {
                        if let Ok(image) = image::load_from_memory(&bytes) {
                            let rgba = image.to_rgba8();
                            self.avatar = Some(ui.ctx().load_texture("profile-editor-avatar",
                                egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()),
                                egui::TextureOptions::LINEAR));
                            self.avatar_png = Some(bytes);
                            self.clear_avatar = false;
                            self.error = None;
                        }
                    }
                    Err(error) => self.error = Some(tf("Could not open picture: {error}", &[("error", &error)])),
                }
            }
            if ui.add_enabled(has_picture, ActionButton::secondary("USE EMBLEM").height(30.0)).clicked() {
                self.avatar = None;
                self.avatar_png = None;
                self.clear_avatar = true;
                self.error = None;
            }
        });
    }

}

/// A whole-card radio target: the insignia, label, outline and radio dot all agree.
fn faction_choice(ui: &mut Ui, side: Allegiance, selected: bool, width: f32) -> Response {
    let narrow = width < 130.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, if narrow { 88.0 } else { 66.0 }), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, ui.is_enabled(), selected, side.label()));
    widgets::reveal_focus(ui, &response);
    if response.clicked() && !selected { crate::sound::play(ui.ctx(), crate::sound::UiSound::Tab); }
    if ui.is_rect_visible(rect) {
        let (accent, tint) = match side {
            Allegiance::Arm => (SKY, Color32::from_rgb(23, 39, 49)),
            Allegiance::Core => (RED, Color32::from_rgb(48, 29, 26)),
        };
        let hovered = response.hovered() && ui.is_enabled();
        let painter = ui.painter();
        painter.rect_filled(rect, 5.0, if selected { tint } else if hovered { PANEL_HI } else { BG_DEEP });
        painter.rect_stroke(rect, 5.0, egui::Stroke::new(if selected { 1.5 } else { 1.0 },
            if selected { accent } else if hovered { LINE_HI } else { LINE }), egui::StrokeKind::Inside);
        let symbol = if narrow {
            Rect::from_center_size(pos2(rect.center().x, rect.top() + 27.0), vec2(34.0, 34.0))
        } else {
            Rect::from_center_size(pos2(rect.left() + 31.0, rect.center().y), vec2(36.0, 36.0))
        };
        painter.rect_filled(symbol.expand(3.0), 4.0, if side == Allegiance::Arm {
            Color32::from_rgb(31, 75, 110)
        } else { Color32::from_rgb(114, 37, 41) });
        crate::faction::paint(painter, symbol, side);
        let text_pos = if narrow { pos2(rect.center().x, rect.bottom() - 21.0) }
            else { pos2(rect.left() + 60.0, rect.center().y) };
        painter.text(text_pos, if narrow { egui::Align2::CENTER_CENTER } else { egui::Align2::LEFT_CENTER },
            side.label(), strong(17.0), TEXT_BRIGHT);
        let radio = if narrow { rect.right_top() + vec2(-12.0, 12.0) }
            else { pos2(rect.right() - 18.0, rect.center().y) };
        painter.circle_stroke(radio, 5.5, egui::Stroke::new(1.5, if selected { accent } else { STEEL_DIM }));
        if selected { painter.circle_filled(radio, 2.5, accent); }
        widgets::focus_ring(ui, &response, rect.shrink(3.0));
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Read identity metadata at most once a second per profile, even on animated screens.
pub fn current(ctx: &egui::Context) -> Option<Profile> {
    let id = egui::Id::new(("profile_identity", ta_config::player_data_dir()));
    let now = ctx.input(|i| i.time);
    if let Some((checked, profile)) = ctx.data(|d| d.get_temp::<(f64, Option<Profile>)>(id))
        && now - checked < 1.0 { return profile; }
    let profile = ta_config::profiles::current_profile().ok().flatten();
    ctx.data_mut(|d| d.insert_temp(id, (now, profile.clone())));
    profile
}

/// The pinned player's portrait. The faction mark remains legible on any photograph.
pub fn avatar(ui: &mut Ui, profile: &Profile, size: f32) -> Response {
    let texture = crate::portrait::texture(ui.ctx());
    avatar_picture(ui, texture.as_ref(), Some(profile.allegiance), size)
        .hover_tip(tf("{name} · {faction}", &[("name", &profile.name), ("faction", &profile.allegiance.label())]))
}

pub fn avatar_picture(ui: &mut Ui, texture: Option<&TextureHandle>, side: Option<Allegiance>, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 6.0, BG_DEEP);
    let inside = rect.shrink(size * 0.05);
    if let Some(texture) = texture {
        painter.image(texture.id(), inside, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    } else {
        crate::widgets::paint_emblem(painter, inside);
    }
    if let Some(side) = side { badge(painter, rect, side); }
    response
}

/// The same faction stamp is used on profile, chat and player-list portraits.
pub fn badge(painter: &egui::Painter, rect: Rect, side: Allegiance) {
    let size = rect.width().min(rect.height());
    let diameter = (size * 0.39).max(15.0).min(size * 0.6);
    let badge = Rect::from_min_size(rect.right_bottom() - vec2(diameter, diameter), vec2(diameter, diameter));
    painter.circle_filled(badge.center(), diameter * 0.52, BG_DEEP);
    painter.circle_filled(badge.center(), diameter * 0.44, if side == Allegiance::Arm { Color32::from_rgb(31, 95, 165) } else { Color32::from_rgb(158, 39, 48) });
    crate::faction::paint(painter, badge.shrink(diameter * 0.10), side);
}

pub fn rank_text(career: &Career) -> String { rank_line(&career.rank()) }

/// "Staff Sergeant · Level 32", with the service stars past the top level.
pub fn rank_line(rank: &Rank) -> String {
    if rank.stars > 0 {
        tf("{rank} ★{stars} · Level {level}", &[("rank", &t(rank.title)), ("stars", &rank.stars), ("level", &rank.level)])
    } else {
        tf("{rank} · Level {level}", &[("rank", &t(rank.title)), ("level", &rank.level)])
    }
}

/// "Next rank: Master Sergeant at level 35", or how far to the next service star.
pub fn next_rank_text(rank: &Rank) -> String {
    match rank.next {
        Some((title, level)) => tf("Next rank: {rank} at level {level}", &[("rank", &t(title)), ("level", &level)]),
        None if rank.stars >= ta_config::profiles::MAX_STARS => t("Every service star earned").to_string(),
        None => tf("A service star for every {xp} XP past the top rank", &[("xp", &grouped(ta_config::profiles::STAR_XP))]),
    }
}

/// The insignia of `career`'s rank, `size` points across, its rank and the next on hover.
pub fn rank_insignia(ui: &mut Ui, career: &Career, size: f32) -> Response {
    let rank = career.rank();
    let response = crate::badges::insignia(ui, rank.grade, size);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Image, true, rank_line(&rank)));
    response.hover_tip_ui(|ui| {
        ui.label(RichText::new(rank_line(&rank)).font(strong(13.0)).color(AMBER));
        ui.label(RichText::new(next_rank_text(&rank)).color(STEEL));
    })
}

/// A recessed brass meter, with the readable value outside the changing fill.
/// The same treatment is used in the launcher, career dossier and game results.
pub fn xp_progress(ui: &mut Ui, career: &Career) -> Response {
    let rank = career.rank();
    let value = progress_text(&rank);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(t(if rank.xp_for_next_level.is_some() { "RANK PROGRESS" } else { "SERVICE STARS" })).font(strong(11.0)).color(STEEL));
        ui.label(RichText::new(&value).font(strong(13.0)).color(TEXT_BRIGHT));
    });
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 15.0), Sense::hover());
    paint_xp_track(ui, rect, rank.progress);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ProgressIndicator, ui.is_enabled(), value.clone()));
    response
}

/// "14000 / 16000 XP" into the level; at the top level, toward the next service star.
pub fn progress_text(rank: &Rank) -> String {
    use ta_config::profiles::{MAX_STARS, STAR_XP};
    match rank.xp_for_next_level {
        Some(next) => tf("{xp} / {next} XP", &[("xp", &grouped(rank.xp_into_level)), ("next", &grouped(next))]),
        None if rank.stars >= MAX_STARS => t("Maximum rank").to_string(),
        None => tf("{xp} / {next} XP to the next star", &[("xp", &grouped(rank.xp_into_level % STAR_XP)), ("next", &grouped(STAR_XP))]),
    }
}

pub fn paint_xp_track(ui: &Ui, rect: Rect, fraction: f32) {
    if !ui.is_rect_visible(rect) { return; }
    let target = crate::progress::fraction(fraction);
    // The allocation's stable auto-ID follows the widget, not its screen position
    // while scrolling. Existing ranks appear immediately; earned progress assembles.
    let duration = if crate::motion::still(ui.ctx()) { 0.0 } else { 0.45 };
    let shown = ui.ctx().animate_value_with_time(ui.next_auto_id().with("career_progress"), target, duration);
    crate::progress::paint(ui, rect, Some(shown), (shown - target).abs() > 0.0001);
}

/// A whole row target, including portrait and faction. Selected state is named as
/// well as coloured, and the whole row participates in keyboard navigation.
pub fn profile_choice(ui: &mut Ui, profile: &Profile, portrait: Option<&TextureHandle>, selected: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 54.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, ui.is_enabled(), selected,
        format!("{} · {}", profile.name, profile.allegiance.label())));
    widgets::reveal_focus(ui, &response);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rect, 4.0, if selected { AMBER_DEEP } else if response.hovered() { PANEL_HI } else { BG_DEEP });
        p.rect_stroke(rect, 4.0, egui::Stroke::new(1.0, if selected { AMBER_DIM } else { LINE }), egui::StrokeKind::Inside);
        let avatar = Rect::from_min_size(rect.min + vec2(7.0, 7.0), vec2(40.0, 40.0));
        let mut picture = ui.new_child(egui::UiBuilder::new().max_rect(avatar));
        avatar_picture(&mut picture, portrait, Some(profile.allegiance), 40.0);
        let text = Rect::from_min_max(rect.min + vec2(58.0, 7.0), rect.max - vec2(10.0, 7.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text));
        child.set_clip_rect(child.clip_rect().intersect(text));
        child.spacing_mut().item_spacing.y = 3.0;
        child.add(egui::Label::new(RichText::new(&profile.name).font(strong(13.0)).color(TEXT_BRIGHT)).truncate());
        child.add(egui::Label::new(RichText::new(if selected {
            tf("{faction} · ACTIVE PROFILE", &[("faction", &profile.allegiance.label())])
        } else { profile.allegiance.label().to_string() }).font(body(11.0)).color(if selected { AMBER } else { STEEL })).truncate());
        widgets::focus_ring(ui, &response, rect.shrink(2.0));
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Mark rank names for extraction even though the progression model belongs to ta_config
/// (`ta_config::profiles::RANKS`; a test keeps the two lists together).
#[cfg_attr(not(test), allow(dead_code))]
const RANK_TITLES: [&str; 21] = [n("Recruit"), n("Private"), n("Private First Class"), n("Lance Corporal"), n("Corporal"),
    n("Sergeant"), n("Staff Sergeant"), n("Master Sergeant"), n("Sergeant Major"), n("Warrant Officer"), n("Second Lieutenant"),
    n("Lieutenant"), n("Captain"), n("Major"), n("Lieutenant Colonel"), n("Colonel"), n("Brigadier"), n("Major General"),
    n("Lieutenant General"), n("General"), n("Commander")];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_rank_title_is_marked_for_translation() {
        assert_eq!(RANK_TITLES.to_vec(), ta_config::profiles::RANKS.iter().map(|(_, title)| *title).collect::<Vec<_>>());
    }

    #[test]
    fn rank_lines_show_service_stars_past_the_top() {
        use ta_config::profiles::{STAR_XP, rank, rank_threshold};
        assert_eq!(rank_line(&rank(rank_threshold(32))), "Staff Sergeant · Level 32");
        assert_eq!(next_rank_text(&rank(rank_threshold(32))), "Next rank: Master Sergeant at level 35");
        let top = rank(rank_threshold(100) + 3 * STAR_XP + 250);
        assert_eq!(rank_line(&top), "Commander ★3 · Level 100");
        assert_eq!(progress_text(&top), "250 / 100,000 XP to the next star");
    }

    #[test]
    fn earned_progress_animates_then_rests_and_respects_reduced_motion() {
        let ctx = egui::Context::default();
        let draw = |time, value, reduced| {
            crate::motion::set_reduced(&ctx, reduced);
            let mut output = ctx.run_ui(egui::RawInput { time: Some(time), ..Default::default() }, |ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(220.0, 15.0), Sense::hover());
                paint_xp_track(ui, rect, value);
            });
            output.textures_delta.clear();
            output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
        };
        for i in 0..5 { draw(i as f64, 0.25, false); }
        assert!(draw(5.0, 0.25, false) > std::time::Duration::from_secs(1), "unchanged rank stays quiet");
        assert!(draw(6.0, 0.75, false) < std::time::Duration::from_millis(34), "earned XP assembles visibly");
        // egui carries an immediate repaint request into the following frame.
        // Let that already-requested frame drain before checking idle behavior.
        draw(7.0, 0.75, false);
        assert!(draw(7.1, 0.75, false) > std::time::Duration::from_secs(1), "completed animation stops");
        assert!(draw(8.0, 0.9, true) > std::time::Duration::from_secs(1), "reduced motion applies immediately");
    }

    #[test]
    fn commander_name_and_placeholder_are_centered_in_the_input() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        for name in ["", "Jared Woodruff", "Commander gjpq"] {
            let mut editor = ProfileEditor::new(None);
            editor.name = name.to_string();
            editor.focus_name = false;
            let mut rect = Rect::NOTHING;
            let output = ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(320.0, 200.0))), ..Default::default() }, |ui| {
                rect = editor.name_field(ui).rect;
            });
            fn text_rect(shape: &egui::Shape, wanted: &str) -> Option<Rect> {
                match shape {
                    egui::Shape::Text(text) if text.galley.text() == wanted => Some(Rect::from_min_size(text.pos, text.galley.size())),
                    egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_rect(shape, wanted)),
                    _ => None,
                }
            }
            let text = output.shapes.iter().find_map(|s| text_rect(&s.shape, if name.is_empty() { "Commander" } else { name })).expect("name or placeholder is painted");
            assert!(rect.height() >= 36.0);
            assert!((text.center().y - rect.center().y).abs() <= 1.0, "{name:?}: text {text:?} is not centered in {rect:?}");
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn profile_rows_activate_with_keyboard_but_not_when_switching_is_blocked() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let profile = Profile { id: "test".into(), name: "Commander".into(), allegiance: Allegiance::Core, has_avatar: false, created_at: 0, ..Default::default() };
        let mut id = egui::Id::NULL;
        let input = |key: Option<egui::Key>| egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(320.0, 200.0))),
            events: key.into_iter().map(|key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }).collect(),
            ..Default::default()
        };
        ctx.run_ui(input(None), |ui| { id = profile_choice(ui, &profile, None, false).id; }).drop_without_applying_deltas();
        ctx.memory_mut(|m| m.request_focus(id));
        let mut clicked = false;
        ctx.run_ui(input(Some(egui::Key::Enter)), |ui| { clicked = profile_choice(ui, &profile, None, false).clicked(); }).drop_without_applying_deltas();
        assert!(clicked);
        ctx.run_ui(input(None), |ui| { ui.disable(); id = profile_choice(ui, &profile, None, false).id; }).drop_without_applying_deltas();
        ctx.memory_mut(|m| m.request_focus(id));
        ctx.run_ui(input(Some(egui::Key::Enter)), |ui| { ui.disable(); clicked = profile_choice(ui, &profile, None, false).clicked(); }).drop_without_applying_deltas();
        assert!(!clicked);
    }
}
