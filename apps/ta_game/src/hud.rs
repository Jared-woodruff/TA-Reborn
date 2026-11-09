//! In-game HUD (egui): resources, selection info, build menus, unit overlays.

use std::collections::HashMap;

use egui::{Color32, CornerRadius, Pos2, Rect, Stroke, TextureHandle, Vec2 as EVec2};
use glam::{Vec2, Vec3, Vec4Swizzles};
use ta_formats::pcx::PcxImage;
use ta_render::Camera;
use ta_sim::{Fx, UnitId};
use ta_vfs::Vfs;

use crate::game::Game;
use crate::hud_skin;
use ta_ui::i18n::n;

pub(crate) mod anchors;
mod cast;
mod cheats;
mod comms;
mod companion;
mod conquest;
mod economy;
mod eta;
mod idle;
pub(crate) mod info;
mod radial;
pub(crate) mod ranges;
mod scenario;
mod deck;

pub(crate) use deck::{Purpose, activity};

const ACCENT: Color32 = crate::theme::AMBER;
const METAL: Color32 = Color32::from_rgb(170, 184, 196);
const ENERGY: Color32 = Color32::from_rgb(242, 212, 60);

/// Seconds the pointer rests on a unit before its card comes (not as it passes over units on its
/// way across the battlefield).
const HOVER_CARD_AFTER: f32 = 0.45;

#[derive(Default)]
pub struct Hud {
    history: crate::battle_history::HistoryWindow,
    pub accessibility: crate::accessibility::BattleAccess,
    /// TA's own pictures of the units (`unitpics/*.pcx`), by type.
    pics: HashMap<usize, Option<TextureHandle>>,
    /// Units' portraits (`ta_render::portrait`): by type, player's colour, size and skin side,
    /// as registered with the interface's renderer (none: the model could not be drawn); and
    /// those asked for this frame and not drawn yet (the app draws them before the next).
    portraits: HashMap<PortraitKey, Option<egui::TextureId>>,
    portraits_wanted: Vec<PortraitKey>,
    /// The idle-builder counter was clicked this frame.
    select_idle: bool,
    /// The idle-factory counter was clicked this frame.
    select_idle_factory: bool,
    /// The top bar's MENU was clicked (the app opens the battle menu).
    open_menu: bool,
    /// A map overlay chip was clicked this frame.
    toggle_overlay: Option<crate::overlay::Overlay>,
    /// TA's mouse cursors.
    cursors: crate::cursor::Cursors,
    /// A Share button was clicked: (ally, metal, energy).
    share: Option<(u8, i64, i64)>,
    /// Give the selected units to this ally.
    give: Option<u8>,
    /// TA's own 1997 panels in place of ours (the setting, or `--classic-ui`).
    pub classic: bool,
    pub force_classic: bool,
    classic_hud: crate::hud_classic::ClassicHud,
    /// Where the deck and the top bar drew what a lesson may point at, this frame.
    anchors: anchors::Anchors,
    /// The panel metal of the side shown.
    skin: hud_skin::Skin,
    /// The cheat panel's spawn picker.
    cheats: cheats::CheatPanel,
    /// The economy's last minutes, for the deck's charts.
    trend: deck::EconomyTrend,
    /// The enemy Commanders' transmissions.
    comms: comms::Comms,
    /// The cast's kill feed, and how far down the screen its scoreboard reaches (a share of the
    /// screen's height, as last drawn: the director frames fights under it).
    feed: cast::Feed,
    board_share: Option<f32>,
    /// The replay's EXPORT VIDEO dialog while open, its choices (kept from one film to the next)
    /// and a film asked for.
    film_dialog: Option<crate::film::FilmDialog>,
    film_settings: crate::film::FilmSettings,
    film_request: Option<crate::film::FilmSettings>,
    /// The replay bar's EXPORT VIDEO button, as wide as its lettering (last frame's).
    film_button: f32,
    /// The replay's EXPORT CLIP dialog while open (`clip_dialog.rs`), its choices (kept from one
    /// clip to the next), and its button's width (last frame's).
    clip_dialog: Option<crate::clip_dialog::ClipDialog>,
    clip_choices: crate::clip_dialog::ClipChoices,
    clip_button: f32,
    /// The dialog closed this frame: its pictures are let go with the next (this frame's
    /// shapes still draw them).
    clip_dialog_closed: Option<crate::clip_dialog::ClipDialog>,
    /// The companion screen's own close button was pressed (its HUD's).
    companion_close: bool,
    /// When the pointer, a key or a click last touched the game (the interface's clock): the
    /// cast's controls leave the picture a few seconds after.
    controls_touched: f64,
    /// The build menu's tab last shown, per kind of constructor.
    build_tab: HashMap<usize, deck::Purpose>,
    /// The building being placed whose tab the build menu last turned to: it turns there once,
    /// when the placing starts, and the player may look at other tabs meanwhile.
    build_tab_placing: Option<usize>,
    /// The build menu's page (when its pictures are on pages), for which builder and tab; the
    /// building being placed whose page it last turned to (as the tab); the wheel's turning
    /// over it, towards a page.
    build_page: ((usize, Option<deck::Purpose>), usize),
    build_page_placing: Option<usize>,
    build_wheel: f32,
    /// When each selected unit was picked (interface clock), for its ring's pop; and the
    /// selection those were taken from.
    picked_at: HashMap<UnitId, f32>,
    /// The unit under the pointer and since when (interface clock): its card waits for the
    /// pointer to rest, and says more the longer it does.
    hover_since: Option<(UnitId, f32)>,
    /// The replay bar's row of controls, as wide as it was last frame (the timeline spans it).
    replay_width: f32,
    /// The top bar's right end (the MAP menu, the pace), as wide as it was last frame: the left
    /// end narrows to leave it room.
    top_right_width: f32,
    /// The top bar's lower edge, as laid out last frame (the minimap and the economy's panel hang
    /// under it).
    top_bar_bottom: f32,
    /// Where the minimap's plate stands, at the top right under the top bar (nothing in TA's
    /// own interface): the panels that hang at the right edge keep below it.
    minimap_rect: Option<Rect>,
    /// The economy's panel under the top bar is open (a click on the economy readout), and where
    /// that readout stands (the panel hangs under it).
    economy_open: bool,
    economy_rect: Option<Rect>,
    /// Our idle constructors and factories, and since when they have been idle (interface
    /// clock), for their alerts.
    idle_alerts: idle::IdleAlerts,
    /// An idle alert was clicked: select that unit and look at it.
    look_at: Option<UnitId>,
    /// A notice that points somewhere was clicked: go there.
    follow: Option<crate::game::Link>,
    /// The units on radar but out of sight, as of a tick and viewer.
    radar_contacts: (Option<(u64, Option<u8>)>, Vec<UnitId>),
    /// The forces' groups as the deck last drew them, and for which player and tick (see
    /// `Hud::forces_now`).
    forces_cache: Option<((u8, u64), Vec<deck::Force>)>,
    /// The deck's sums over the selection, as of a frame (see `deck::SelectionSummary`).
    selection_summary: Option<(u64, deck::SelectionSummary)>,
    /// The column of messages under the top bar, as laid out last frame (none when empty): the
    /// idle alerts' arrows keep out of it.
    messages_rect: Option<Rect>,
    /// The selected units' rings on the ground this frame, for the scene to draw (see
    /// `ta_render::rings`).
    rings: Vec<ta_render::RingInstance>,
    /// Health bars shown lately (by unit): the health just lost lingering on them, and their
    /// stun's (see `BarTrail`).
    bar_trails: HashMap<UnitId, BarTrail>,
    /// How the builds in view have gone lately, for the time left over them (see `eta`).
    etas: eta::Etas,
    /// The radial menu's motion and what it last drew (see `radial`).
    radial_view: radial::View,
}

/// A health bar's memory (interface clock): what it showed (the health just lost lingers as a
/// pale chunk, then drains), its health last frame, when it was last hit and last drawn; the
/// paralysis it shows, until when and how long it was (ticks).
#[derive(Clone, Copy)]
struct BarTrail {
    shown: f32,
    last: f32,
    hit_at: f32,
    seen: f32,
    stun: (u64, u64),
}

/// How long the health just lost stays on a bar before it drains (seconds), and how fast it
/// drains (bars a second); how long a hit lights the bar's frame.
const TRAIL_HOLD: f32 = 0.45;
const TRAIL_DRAIN: f32 = 0.9;
const HIT_LIGHT: f32 = 0.22;

/// A unit's portrait: its type, the player's colour it wears and its size in pixels.
/// Unit type, team colour, resolution and whether it uses the enemy skin.
pub type PortraitKey = (usize, u32, u32, bool);

/// The sizes portraits are drawn at (pixels): each picture of a unit is drawn from the smallest
/// at least as large as it is shown (a little smaller will do).
pub(crate) const PORTRAIT_SIZES: [u32; 5] = [48, 72, 104, 152, 216];

/// Whether the modern interface pictures units by their portraits (`TA_PORTRAITS=0`: TA's
/// pictures, for comparisons).
pub(crate) fn portraits_wanted() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("TA_PORTRAITS").map_or(true, |v| v != "0"))
}

/// A picture of a unit: a portrait drawn from its model, on a clear ground (the interface sets
/// it on a backdrop), or else TA's own picture, with its own.
#[derive(Clone, Copy)]
pub(crate) struct UnitPic {
    pub id: egui::TextureId,
    pub portrait: bool,
    pub allegiance: Option<ta_config::profiles::Allegiance>,
}

impl UnitPic {
    /// Paints the picture over `rect` (a portrait on its backdrop), tinted.
    pub fn paint(&self, painter: &egui::Painter, rect: Rect, tint: Color32) {
        if self.portrait {
            portrait_backdrop(painter, rect, tint);
        }
        painter.image(self.id, rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint);
        if let Some(side) = self.allegiance { ta_ui::profiles::badge(painter, rect, side); }
    }

    /// Paints the picture in a frame of another shape, `plate`: a portrait over `rect` in it,
    /// on its ground filling the frame; TA's own picture, a scene of its own, over the whole
    /// frame, its edges cut to the frame's shape.
    pub fn paint_framed(&self, painter: &egui::Painter, plate: Rect, rect: Rect, tint: Color32) {
        if self.portrait {
            portrait_backdrop(painter, plate, tint);
            painter.image(self.id, rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint);
        } else {
            let (w, h) = (plate.width(), plate.height().max(1.0));
            let uv = if w < h {
                Rect::from_min_max(Pos2::new(0.5 - w / h / 2.0, 0.0), Pos2::new(0.5 + w / h / 2.0, 1.0))
            } else {
                Rect::from_min_max(Pos2::new(0.0, 0.5 - h / w / 2.0), Pos2::new(1.0, 0.5 + h / w / 2.0))
            };
            painter.image(self.id, plate, uv, tint);
        }
        if let Some(side) = self.allegiance { ta_ui::profiles::badge(painter, plate, side); }
    }
}

fn profile_emblem(ctx: &egui::Context) -> TextureHandle {
    let key = egui::Id::new("ta_game_profile_emblem");
    if let Some(texture) = ctx.data_mut(|data| data.get_temp::<TextureHandle>(key)) { return texture; }
    let rgba = ta_ui::icon::emblem_rgba(128);
    let texture = ctx.load_texture("profile-emblem", egui::ColorImage::from_rgba_unmultiplied([128, 128], &rgba), egui::TextureOptions::LINEAR);
    ctx.data_mut(|data| data.insert_temp(key, texture.clone()));
    texture
}

/// The ground a portrait stands on: gunmetal lit from above, darker towards the foot, with a
/// pool of light behind the unit.
fn portrait_backdrop(painter: &egui::Painter, rect: Rect, tint: Color32) {
    let tone = |c: Color32| c.gamma_multiply(tint.r() as f32 / 255.0);
    painter.add(ta_ui::metal::vgrad(rect, tone(Color32::from_rgb(66, 72, 78)), tone(Color32::from_rgb(24, 26, 28))));
    // The light behind it, a soft disc fading out.
    let c = rect.center() - EVec2::new(0.0, rect.height() * 0.08);
    let r = rect.width().min(rect.height()) * 0.5;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(c, tone(Color32::from_rgba_premultiplied(30, 34, 38, 0)));
    let n = 32;
    for k in 0..=n {
        let a = k as f32 / n as f32 * std::f32::consts::TAU;
        mesh.colored_vertex(c + EVec2::new(a.cos(), a.sin()) * r, Color32::TRANSPARENT);
        if k > 0 {
            mesh.add_triangle(0, k, k + 1);
        }
    }
    painter.with_clip_rect(rect.intersect(painter.clip_rect())).add(egui::Shape::mesh(mesh));
}

/// The portrait of a unit type in `player`'s colours to show `px` pixels across: drawn at the
/// smallest size at least as large (a little smaller will do).
pub(crate) fn portrait_key(game: &Game, type_id: usize, px: f32, player: u8) -> PortraitKey {
    let size = PORTRAIT_SIZES.iter().copied().find(|&s| s as f32 >= px * 0.92).unwrap_or(PORTRAIT_SIZES[PORTRAIT_SIZES.len() - 1]);
    let colour = game.sim.players.get(player as usize).map_or(0, |p| p.color as u32);
    (type_id, colour, size, game.sim.is_enemy(player, game.hud_player()))
}

/// Seconds a constructor or factory stands idle before its alert shows (a constructor that has
/// just finished a building is not forgotten yet), and until it fades (the top bar keeps
/// counting them).
const IDLE_ALERT: f32 = 6.0;
const IDLE_ALERT_END: f32 = 20.0;

/// How far overlays keep from the screen's edges to stay clear of the panels, and where
/// hover cards must stay.
#[derive(Clone, Copy)]
struct Clearance {
    left: f32,
    top: f32,
    bottom: f32,
    /// Offset of the free view's centre from the screen's.
    center: f32,
    view: Rect,
    /// How far under `top` the minimap's plate reaches at the right edge: what hangs there (the
    /// ally strip, the share and cheat panels, the cast's kill feed) starts below it.
    right_top: f32,
}

/// A Share menu choice: resources for an ally, or the selection.
pub(crate) enum Gift {
    Resources(u8, i64, i64),
    Units(u8),
}

impl Hud {
    pub fn new(force_classic: bool) -> Hud {
        Hud { force_classic, ..Default::default() }
    }

    /// Where this frame's interface drew what the tutorial's guide may point at.
    pub fn anchors(&self) -> &anchors::Anchors {
        if self.classic { &self.classic_hud.anchors } else { &self.anchors }
    }

    /// The transmissions waiting and shown go (a new battle, a replay sought back), and what the
    /// deck kept of the battle before: the forces' groups, the economy's panel.
    pub fn clear_transmissions(&mut self) {
        self.comms = Default::default();
        self.forces_cache = None;
        self.economy_open = false;
    }

    /// The top bar's MENU, or the classic interface's OPTIONS, was clicked (the app opens the
    /// game menu).
    pub fn take_open_menu(&mut self) -> bool {
        std::mem::take(&mut self.open_menu) | std::mem::take(&mut self.classic_hud.open_menu)
    }

    /// The classic interface: TA's shortcut keys pressed this frame, and the keys its page
    /// answers to (none with the modern HUD).
    pub fn set_classic_keys(&mut self, pressed: Vec<winit::keyboard::KeyCode>) {
        self.classic_hud.set_keys(if self.classic { pressed } else { Vec::new() });
    }

    pub fn classic_claims(&self) -> Vec<winit::keyboard::KeyCode> {
        if self.classic { self.classic_hud.claimed_keys() } else { Vec::new() }
    }

    /// How the classic interface's art and lettering are enlarged: smooth, or TA's own pixels
    /// (the setting `gameplay.classic_pixels`).
    pub fn set_classic_look(&mut self, look: crate::hud_classic_scale::Look) {
        self.classic_hud.look = look;
    }

    /// The free view with the minimap's plate at the top right taken off its top (where the
    /// notices that come in at the top right of the view stand: the achievements').
    pub fn notices_room(&self, ctx: &egui::Context) -> Rect {
        let mut room = self.free_view(ctx);
        if !self.classic && let Some(m) = self.minimap_rect {
            room.min.y = room.min.y.max(m.bottom());
        }
        room
    }

    /// The part of the screen the HUD leaves free for the battle (and panels over it).
    pub fn free_view(&self, ctx: &egui::Context) -> Rect {
        let screen = ctx.content_rect();
        let c = self.clearance(ctx);
        Rect::from_min_max(egui::pos2(screen.left() + c.left, screen.top() + c.top), egui::pos2(screen.right(), screen.bottom() - c.bottom))
    }

    fn clearance(&self, ctx: &egui::Context) -> Clearance {
        let screen = ctx.content_rect();
        if self.classic {
            let view = crate::hud_classic::view_rect(screen);
            Clearance { left: view.left() - screen.left(), top: view.top() - screen.top(), bottom: screen.bottom() - view.bottom(), center: view.center().x - screen.center().x, view, right_top: 0.0 }
        } else {
            // Our resource bar and the command deck along the foot, the minimap at the top right;
            // messages centre on the screen.
            let right_top = self.minimap_rect.map_or(0.0, |m| (m.bottom() - (screen.top() + 34.0) + deck::GAP).max(0.0));
            Clearance { left: 0.0, top: 34.0, bottom: deck::deck_height(screen) + 2.0 * deck::GAP, center: 0.0, view: screen, right_top }
        }
    }

    /// A player's picture, `size` points across: the player's own (`portrait.png` in the settings
    /// folder) for this computer's player, else their side's Commander in their colours, as the
    /// deck pictures units (its portrait, or TA's picture with TA's panels).
    pub(crate) fn portrait(&mut self, ctx: &egui::Context, vfs: &Vfs, game: &Game, player: u8, size: f32) -> Option<UnitPic> {
        if player == game.local_player && !game.spectator {
            if let Some(allegiance) = game.profile_allegiance {
                let own = ta_ui::portrait::texture(ctx).unwrap_or_else(|| profile_emblem(ctx));
                return Some(UnitPic { id: own.id(), portrait: false, allegiance: Some(allegiance) });
            }
            if let Some(own) = ta_ui::portrait::texture(ctx) {
                return Some(UnitPic { id: own.id(), portrait: false, allegiance: None });
            }
        }
        let side = &game.sim.players.get(player as usize)?.side;
        let commander = crate::setup::resolve_side(&game.sim.data, side).1.map(|s| s.commander.clone())?;
        let type_id = game.sim.types.id(&commander)?;
        self.unit_pic_of(ctx, vfs, game, type_id, size, player)
    }

    /// TA's own picture of a unit type.
    fn cameo(&mut self, ctx: &egui::Context, vfs: &Vfs, game: &Game, type_id: usize) -> Option<TextureHandle> {
        self.cameo_for_type(ctx, vfs, type_id, &game.sim.types.get(type_id).name)
    }

    fn cameo_for_type(&mut self, ctx: &egui::Context, vfs: &Vfs, type_id: usize, name: &str) -> Option<TextureHandle> {
        self.pics
            .entry(type_id)
            .or_insert_with(|| {
                let name = name.to_ascii_lowercase();
                let pcx = PcxImage::parse(&vfs.read_opt(&format!("unitpics/{name}.pcx"))?).ok()?;
                let img = egui::ColorImage::from_rgba_unmultiplied([pcx.width as usize, pcx.height as usize], &pcx.to_rgba());
                Some(ctx.load_texture(format!("unitpic-{name}"), img, egui::TextureOptions::LINEAR))
            })
            .clone()
    }

    /// A unit type's picture `size` points across, as the player watched sees their units: its
    /// portrait drawn from its model (asked for the next frame if not drawn yet; TA's picture
    /// meanwhile); TA's picture with TA's own panels.
    fn unit_pic(&mut self, ctx: &egui::Context, vfs: &Vfs, game: &Game, type_id: usize, size: f32) -> Option<UnitPic> {
        self.unit_pic_of(ctx, vfs, game, type_id, size, game.hud_player())
    }

    /// [`Hud::unit_pic`] in `player`'s colours.
    pub(crate) fn unit_pic_of(&mut self, ctx: &egui::Context, vfs: &Vfs, game: &Game, type_id: usize, size: f32, player: u8) -> Option<UnitPic> {
        if !self.classic
            && let Some(id) = self.model_portrait(ctx, game, type_id, size, player)
        {
            return Some(UnitPic { id, portrait: true, allegiance: None });
        }
        self.cameo(ctx, vfs, game, type_id).map(|t| UnitPic { id: t.id(), portrait: false, allegiance: None })
    }

    /// A unit type's portrait drawn from its model, `size` points across, in `player`'s colours
    /// (on a clear ground): asked for the next frame if it is not drawn yet. Also for pictures
    /// outside the deck, such as a cockpit's screen.
    pub(crate) fn model_portrait(&mut self, ctx: &egui::Context, game: &Game, type_id: usize, size: f32, player: u8) -> Option<egui::TextureId> {
        if !portraits_wanted() {
            return None;
        }
        let key = portrait_key(game, type_id, size * ctx.pixels_per_point(), player);
        match self.portraits.get(&key) {
            Some(id) => *id,
            None => {
                if !self.portraits_wanted.contains(&key) {
                    self.portraits_wanted.push(key);
                }
                ctx.request_repaint();
                None
            }
        }
    }

    /// A portrait asked for elsewhere (a cockpit's screen), to draw with the HUD's own (once).
    pub fn request_portrait(&mut self, key: PortraitKey) {
        if !self.portraits.contains_key(&key) && !self.portraits_wanted.contains(&key) {
            self.portraits_wanted.push(key);
        }
    }

    /// The portraits asked for since this was last called, to draw (the app does, see
    /// [`Hud::set_portrait`]).
    pub fn take_portrait_requests(&mut self) -> Vec<PortraitKey> {
        std::mem::take(&mut self.portraits_wanted)
    }

    /// A portrait drawn and registered with the interface's renderer (none: it could not be).
    pub fn set_portrait(&mut self, key: PortraitKey, id: Option<egui::TextureId>) {
        self.portraits.insert(key, id);
    }

    /// TA's cursor stands in for the system's this frame (the app keeps the system's hidden
    /// to the frame's end).
    pub fn ta_cursor_drawn(&self) -> bool {
        self.cursors.drawn()
    }

    /// Before the frame's interface: TA's cursor not drawn yet.
    pub fn begin_frame(&mut self) {
        self.cursors.draw_none();
        self.rings.clear();
    }

    /// The selected units' rings on the ground, as this frame's interface found them (the scene
    /// draws them, under what stands on them).
    pub fn rings(&self) -> &[ta_render::RingInstance] {
        &self.rings
    }

    pub fn draw(&mut self, ui: &mut egui::Ui, vfs: &Vfs, game: &mut Game, camera: &mut Camera, minimap: &mut crate::minimap::Minimap) {
        let ctx = ui.ctx().clone();
        self.anchors.clear(false);
        // Register the modal before other HUD areas so their controls cannot act behind it.
        let summary = if game.history_open { crate::accessibility::battle_summary(game) } else { String::new() };
        self.history.show(&ctx, &game.event_history, &mut game.history_open, &summary);
        if !self.classic {
            self.skin.load(&ctx, vfs, &game.sim.players[game.hud_player() as usize].side);
            self.trend.sample(game);
            self.top_bar(ui, game);
            self.lay_out_minimap(&ctx, game);
        }
        if std::mem::take(&mut self.select_idle) {
            game.select_next_idle_builder();
        }
        if std::mem::take(&mut self.select_idle_factory) {
            game.select_next_idle_factory();
        }
        if let Some((to, m, e)) = self.share.take() {
            game.share(to, m, e);
        }
        if let Some(to) = self.give.take() {
            game.give_selection(to);
        }
        let clear = self.clearance(&ctx);
        if let Some(o) = self.toggle_overlay.take() {
            game.toggle_overlay(o);
        }
        self.overlays(&ctx, vfs, game, camera, clear.view);
        if let Some(id) = self.look_at.take() {
            game.select_and_look(id);
        }
        if !self.classic {
            self.group_bar(&ctx, vfs, game, clear);
        }
        if self.classic {
            self.classic_hud.draw(ui, vfs, game, camera, minimap);
        } else {
            self.deck(ui, vfs, game, camera, minimap);
            self.economy_panel(&ctx, vfs, game);
        }
        self.messages(&ctx, game, clear);
        if let Some(link) = self.follow.take() {
            game.follow(link);
        }
        self.cast(&ctx, vfs, game, clear);
        self.comms(&ctx, vfs, game);
        self.scenario_panel(&ctx, game, clear);
        self.conquest_panel(&ctx, game, clear);
        self.ceasefire_panel(&ctx, game, clear);
        self.waiting_banner(&ctx, game, clear);
        if !self.classic {
            self.ally_strip(&ctx, game, clear);
        }
        self.chat(&ctx, vfs, game, clear);
        self.cheat_panel(&ctx, vfs, game, clear);
        self.share_panel(&ctx, game, clear);
        self.unit_info_panel(&ctx, vfs, game, clear);
        self.replay_bar(&ctx, game, clear);
        self.film_dialog(&ctx, game);
        self.clip_dialog(&ctx, game);
        self.spectator_bar(&ctx, game, clear);
        if game.show_players {
            let pictures: Vec<(u8, Option<UnitPic>)> = (0..game.sim.players.len() as u8).map(|p| (p, self.portrait(&ctx, vfs, game, p, 28.0))).collect();
            players_panel(&ctx, game, [clear.center, clear.top + 36.0], &pictures);
        }
        // The radial menu over everything else, while one is open (and a moment after).
        self.radial_menu(&ctx, vfs, game);
        if game.ta_cursors {
            let over_ui = ctx.is_pointer_over_egui();
            let kind = game.cursor_kind(over_ui);
            self.cursors.draw(&ctx, vfs, &game.sim.data.palette, kind, game.cursor_scale, game.high_contrast_cursor);
        } else {
            self.cursors.draw_none();
        }
    }

    /// Replays: the transport (back, play or pause, ahead), the timeline (a click or a drag
    /// jumps there), the speed, and whose eyes to watch through.
    fn replay_bar(&mut self, ctx: &egui::Context, game: &mut Game, clear: Clearance) {
        use ta_ui::i18n::{t, tf};
        let Some(end) = game.playback.as_ref().filter(|p| !p.resume).map(|p| p.end_tick.max(1)) else { return };
        let clock = |t: u64| format!("{:02}:{:02}", t / 30 / 60, t / 30 % 60);
        let shown = self.controls_shown(ctx, game, "replay_bar");
        if shown <= 0.0 {
            return;
        }
        ta_ui::panel::strip_shown(ctx, "replay_bar", egui::Align2::CENTER_BOTTOM, EVec2::new(clear.left / 2.0, -clear.bottom - 14.0), shown, |ui| {
            // Two rows: the timeline over the controls, as wide as they are.
            let controls = ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = EVec2::new(6.0, 8.0);
                let top = ui.horizontal(|ui| {
                    ui.label(ta_ui::widgets::caption("REPLAY").color(ACCENT));
                    let width = (self.replay_width - 160.0 - self.film_button - self.clip_button - 14.0).max(240.0);
                    let scan = game.replay_scan.as_ref().and_then(|s| s.state().lock().ok().map(|s| s.clone()));
                    if let Some(to) = timeline(ui, game.sim.tick, end, width, scan.as_ref()) {
                        game.seek_replay(to);
                    }
                    ui.label(egui::RichText::new(format!("{} / {}", clock(game.sim.tick), clock(end))).font(crate::theme::strong(12.5)).color(crate::theme::TEXT_BRIGHT));
                    ui.add_space(4.0);
                    let clip = ui.add(ta_ui::widgets::ActionButton::secondary(ta_ui::i18n::n("EXPORT CLIP")).height(26.0)).on_hover_text(t("A 30 or 60 second clip for social media, vertical or horizontal, its best moment chosen for you"));
                    if clip.clicked() {
                        self.clip_dialog = Some(crate::clip_dialog::ClipDialog::new(self.clip_choices));
                    }
                    if (clip.rect.width() - self.clip_button).abs() > 0.5 {
                        self.clip_button = clip.rect.width();
                        ctx.request_repaint();
                    }
                    let film = ui.add(ta_ui::widgets::ActionButton::secondary(ta_ui::i18n::n("EXPORT VIDEO")).height(26.0)).on_hover_text(t("Make the replay into a video to share: all of it, a stretch or its big moments"));
                    if film.clicked() {
                        self.film_dialog = Some(crate::film::FilmDialog { settings: self.film_settings.clone() });
                    }
                    if (film.rect.width() - self.film_button).abs() > 0.5 {
                        self.film_button = film.rect.width();
                        ctx.request_repaint();
                    }
                });
                let _ = top;
                if game.replay_approximate {
                    ui.label(egui::RichText::new(t("Compatibility playback")).font(crate::theme::strong(12.0)).color(crate::theme::AMBER))
                        .on_hover_text(t("Recorded with different game data or an older engine. AI, physics and outcomes may differ."));
                }
                let row = ui.horizontal(|ui| {
                    if transport(ui, Transport::Back, t("-2 min").to_string()).on_hover_text(t("Back two minutes")).clicked() {
                        game.seek_replay(game.sim.tick.saturating_sub(30 * 120));
                    }
                    if transport(ui, Transport::Back, t("-30 s").to_string()).on_hover_text(t("Back half a minute")).clicked() {
                        game.seek_replay(game.sim.tick.saturating_sub(30 * 30));
                    }
                    let pause = tf("Pause ({key})", &[("key", &game.keys.text("pause"))]);
                    if transport(ui, if game.paused { Transport::Play } else { Transport::Pause }, String::new()).on_hover_text(pause).clicked() {
                        game.paused = !game.paused;
                    }
                    if transport(ui, Transport::Ahead, t("+30 s").to_string()).on_hover_text(t("Ahead half a minute")).clicked() {
                        game.skip_replay(30 * 30);
                    }
                    if transport(ui, Transport::Ahead, t("+2 min").to_string()).on_hover_text(t("Ahead two minutes")).clicked() {
                        game.skip_replay(30 * 120);
                    }
                    ui.add_space(10.0);
                    let options: [(f32, &str); 5] = [(0.5, "½×"), (1.0, "1×"), (2.0, "2×"), (4.0, "4×"), (8.0, "8×")];
                    // Lit only at one of these speeds: at another (set with the keys) none is, so a
                    // click on any of them takes.
                    let mut speed = options.iter().map(|o| o.0).find(|s| (s - game.speed).abs() < 0.01).unwrap_or(game.speed);
                    // (Tight: the segments as wide as their lettering.)
                    let changed = ui.allocate_ui(EVec2::new(200.0, 30.0), |ui| ta_ui::widgets::segmented(ui, &mut speed, &options).changed()).inner;
                    if changed {
                        game.speed = speed;
                    }
                    ui.add_space(10.0);
                    vision_picker(ui, game);
                    ui.add_space(4.0);
                    ta_ui::widgets::check_box(ui, &mut game.action_camera);
                    ui.label(egui::RichText::new(t("Follow the action")).font(crate::theme::body(13.0)).color(crate::theme::TEXT))
                        .on_hover_text(t("The camera goes where the fighting is; scrolling takes back control"));
                    ui.add_space(4.0);
                    cast_switch(ui, game);
                });
                row.response.rect.width()
            });
            let width = controls.inner;
            if (width - self.replay_width).abs() > 0.5 {
                self.replay_width = width;
                ctx.request_repaint();
            }
        });
    }

    /// How far down the screen the cast's scoreboard reaches, as a share of its height (none
    /// while it is not shown).
    pub fn board_share(&self) -> Option<f32> {
        self.board_share
    }

    /// The companion screen's close button was pressed (asked of its HUD).
    pub fn take_companion_close(&mut self) -> bool {
        std::mem::take(&mut self.companion_close)
    }

    /// A film asked for on the replay's EXPORT VIDEO dialog.
    pub fn take_film_request(&mut self) -> Option<crate::film::FilmSettings> {
        self.film_request.take()
    }

    /// A film's interface, drawn over each of its pictures (in a pass of the interface at the
    /// picture's size, `film.rs`): the cast's scoreboard and kill feed when `cast`, on the film's
    /// clock (`now`), at the top of a picture with nothing else over it.
    pub fn film_overlay(&mut self, ctx: &egui::Context, vfs: &Vfs, game: &mut Game, cast: bool, now: f64) {
        let clear = Clearance { left: 0.0, top: 0.0, bottom: 0.0, center: 0.0, view: ctx.content_rect(), right_top: 0.0 };
        let was = std::mem::replace(&mut game.cast, cast);
        self.cast_at(ctx, vfs, game, clear, now);
        game.cast = was;
    }

    /// How far into view a replay's or a spectator's controls (the strip `id`) are, 0 to 1: in
    /// the cast they leave the picture after a few seconds without the pointer moving, a key
    /// or a click (not while over them or paused), and come back with the next.
    fn controls_shown(&mut self, ctx: &egui::Context, game: &Game, id: &str) -> f32 {
        const IDLE: f64 = 3.0;
        let now = ctx.input(|i| i.time);
        let touched = ctx.input(|i| i.pointer.delta() != EVec2::ZERO || i.pointer.any_down() || i.events.iter().any(|e| matches!(e, egui::Event::MouseWheel { .. } | egui::Event::Key { pressed: true, .. })));
        let over = ctx.pointer_hover_pos().zip(ctx.memory(|m| m.area_rect(egui::Id::new(id)))).is_some_and(|(p, r)| r.expand(24.0).contains(p));
        if touched || over || game.paused || !game.cast {
            self.controls_touched = now;
        }
        let shown = ctx.animate_bool_with_time(egui::Id::new(id).with("shown"), now - self.controls_touched < IDLE, 0.35);
        if game.cast && shown > 0.0 && shown < 1.0 {
            ctx.request_repaint();
        }
        shown
    }

    /// The replay's EXPORT VIDEO dialog, while open: EXPORT VIDEO asks the app for the film.
    fn film_dialog(&mut self, ctx: &egui::Context, game: &Game) {
        let Some(dialog) = self.film_dialog.as_mut() else { return };
        let Some(end) = game.playback.as_ref().filter(|p| !p.resume).map(|p| p.end_tick.max(1)) else {
            self.film_dialog = None;
            return;
        };
        let scan = game.replay_scan.as_ref().and_then(|s| s.state().lock().ok().map(|s| s.clone()));
        match crate::film::dialog(ctx, dialog, end, game.sim.tick, scan.as_ref()) {
            crate::film::Answer::Stay => self.film_settings = dialog.settings.clone(),
            crate::film::Answer::Close => self.film_dialog = None,
            crate::film::Answer::Export(settings) => {
                self.film_settings = settings.clone();
                self.film_request = Some(settings);
                self.film_dialog = None;
            }
        }
    }

    /// The replay's EXPORT CLIP dialog, while open (`clip_dialog.rs`): EXPORT CLIP asks the app
    /// for the clip, as a film.
    fn clip_dialog(&mut self, ctx: &egui::Context, game: &Game) {
        self.clip_dialog_closed = None;
        let Some(dialog) = self.clip_dialog.as_mut() else { return };
        let Some(end) = game.playback.as_ref().filter(|p| !p.resume).map(|p| p.end_tick.max(1)) else {
            self.clip_dialog_closed = self.clip_dialog.take();
            return;
        };
        let scan = game.replay_scan.as_ref().and_then(|s| s.state().lock().ok().map(|s| s.clone()));
        let answer = crate::clip_dialog::dialog(ctx, dialog, end, scan.as_ref());
        self.clip_choices = dialog.choices;
        match answer {
            crate::clip_dialog::Answer::Stay => {}
            crate::clip_dialog::Answer::Close => self.clip_dialog_closed = self.clip_dialog.take(),
            crate::clip_dialog::Answer::Export(settings) => {
                self.film_request = Some(settings);
                self.clip_dialog_closed = self.clip_dialog.take();
            }
        }
    }

    /// Opens the EXPORT CLIP dialog on `spec` (captures: `TA_CLIP` with `TA_CLIP_DIALOG=1`).
    pub fn open_clip_dialog(&mut self, spec: crate::clips::ClipSpec) {
        self.clip_choices.spec = spec;
        self.clip_dialog = Some(crate::clip_dialog::ClipDialog::new(self.clip_choices));
    }

    /// Whether the EXPORT CLIP dialog is open.
    pub fn clip_dialog_open(&self) -> bool {
        self.clip_dialog.is_some()
    }

    /// Whether the replay's EXPORT VIDEO or EXPORT CLIP dialog is open.
    pub fn export_dialog_open(&self) -> bool {
        self.film_dialog.is_some() || self.clip_dialog.is_some()
    }

    /// The pictures of climaxes the EXPORT CLIP dialog asks the app to take.
    pub fn take_clip_shots(&mut self) -> Vec<crate::clip_dialog::PreviewShot> {
        self.clip_dialog.as_mut().map_or_else(Vec::new, |d| d.take_wanted())
    }

    /// A picture taken for the EXPORT CLIP dialog (see [`crate::clip_dialog::ClipDialog::put_picture`]).
    pub fn put_clip_picture(&mut self, ctx: &egui::Context, tick: u64, shape: crate::clips::Shape, rgba: &[u8], width: usize, height: usize) {
        if let Some(d) = self.clip_dialog.as_mut() {
            d.put_picture(ctx, tick, shape, rgba, width, height);
        }
    }

    /// Multiplayer spectators: whose eyes to watch through.
    fn spectator_bar(&mut self, ctx: &egui::Context, game: &mut Game, clear: Clearance) {
        if !game.spectator || game.net.is_none() {
            return;
        }
        let shown = self.controls_shown(ctx, game, "spectator_bar");
        if shown <= 0.0 {
            return;
        }
        ta_ui::panel::strip_shown(ctx, "spectator_bar", egui::Align2::CENTER_BOTTOM, EVec2::new(clear.left / 2.0, -clear.bottom - 14.0), shown, |ui| {
            ui.horizontal(|ui| {
                ui.label(ta_ui::widgets::caption("SPECTATING").color(ACCENT));
                vision_picker(ui, game);
                ui.checkbox(&mut game.action_camera, ta_ui::i18n::t("Follow the action")).on_hover_text(ta_ui::i18n::t("The camera goes where the fighting is; scrolling takes back control"));
                cast_switch(ui, game);
            });
        });
    }

    fn top_bar(&mut self, root: &mut egui::Ui, game: &Game) {
        use ta_ui::i18n::{t, tf};
        let p = &game.sim.players[game.hud_player() as usize];
        let frame = egui::Frame::new().inner_margin(egui::Margin { left: 10, right: 10, top: 5, bottom: 6 });
        let time = root.ctx().input(|i| i.time);
        let bar = egui::Panel::top("resources").frame(frame).show(root, |ui| {
            // (The side's panel metal goes here, behind, once the bar's size is known.)
            let background = ui.painter().add(egui::Shape::Noop);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                // Narrower windows (or a larger interface) get narrower economy gauges, and then
                // leave out what matters least: the sparklines, then the wind, then the clock and
                // the FULL tags; the idle counters merge into one (`economy::Tier`). (The
                // thresholds count the MAP menu on the right, about 80 wide; more there, a long
                // overlay's name or the pace, takes it off.)
                let width = ui.available_width() - (self.top_right_width - 80.0).max(0.0);
                let tier = economy::Tier::of(width);
                let (narrow, tight) = (tier <= economy::Tier::Narrow, tier == economy::Tier::Tight);
                ta_ui::metal::embossed_label(ui, egui::RichText::new(p.side.to_uppercase()).font(crate::theme::display(20.0)).extra_letter_spacing(ta_ui::i18n::tracking(1.5)), ACCENT);
                ui.add_space(10.0);
                let gap = if tight { 12.0 } else { 22.0 };
                // The economy: metal and energy, what is stored, what comes in and goes out.
                self.economy_readout(ui, game, tier, time);
                ui.add_space(gap);
                let units = p.unit_count;
                if game.sim.unit_limit < u32::MAX {
                    let full = units >= game.sim.unit_limit;
                    let value = egui::RichText::new(format!("{units} / {}", game.sim.unit_limit)).color(if full { crate::theme::RED } else { crate::theme::TEXT });
                    stat(ui, n("UNITS"), value).on_hover_text(t("Factories and builders wait while you are at the unit limit."));
                } else {
                    stat(ui, n("UNITS"), egui::RichText::new(units.to_string()).color(crate::theme::TEXT));
                }
                let secs = game.sim.tick / 30;
                if !tight {
                    ui.add_space(10.0);
                    let clock = if secs >= 3600 { format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60) } else { format!("{:02}:{:02}", secs / 60, secs % 60) };
                    stat(ui, n("TIME"), egui::RichText::new(clock).color(crate::theme::TEXT));
                }
                if !narrow {
                    ui.add_space(10.0);
                    stat(ui, n("WIND"), egui::RichText::new(format!("{:.0}", game.sim.wind.to_f32())).color(crate::theme::TEXT))
                        .on_hover_text(t("How hard the wind blows now: what each wind generator makes"));
                }
                ui.add_space(10.0);
                let idle_factories = game.idle_factories().len();
                let idle = game.idle_builders().len();
                let factory_tip = tf("Select the next factory with nothing to build ({key})", &[("key", &game.keys.text("idle_factory"))]);
                let builder_tip = tf("Select the next idle constructor ({key})", &[("key", &game.keys.text("idle_builder"))]);
                if narrow {
                    // One counter for both: the constructors first.
                    let all = idle + idle_factories;
                    if all > 0 && idle_chip(ui, &tf("{n} idle", &[("n", &all)])).on_hover_text(if idle > 0 { builder_tip } else { factory_tip }).clicked() {
                        if idle > 0 {
                            self.select_idle = true;
                        } else {
                            self.select_idle_factory = true;
                        }
                    }
                } else {
                    if idle_factories > 0 && idle_chip(ui, &tf("IDLE FACTORIES {n}", &[("n", &idle_factories)])).on_hover_text(factory_tip).clicked() {
                        self.select_idle_factory = true;
                    }
                    if idle > 0 && idle_chip(ui, &tf("IDLE BUILDERS {n}", &[("n", &idle)])).on_hover_text(builder_tip).clicked() {
                        self.select_idle = true;
                    }
                }
                // Allies: send them resources.
                let allies = game.allies();
                if !allies.is_empty() {
                    ui.add_space(6.0);
                    ui.menu_button(format!("{} ▾", t("Share")), |ui| match share_menu(ui, game) {
                        Some(Gift::Resources(a, m, e)) => self.share = Some((a, m, e)),
                        Some(Gift::Units(a)) => {
                            self.give = Some(a);
                            ui.close();
                        }
                        None => {}
                    });
                }
                // On the right: the pace when it is not normal, and the map overlays in one menu
                // (the one shown named on it).
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let menu = ui.add(egui::Button::new(egui::RichText::new(t("MENU")).font(crate::theme::strong(11.0)).color(crate::theme::STEEL)).frame(false))
                        .on_hover_text(t("Pause, save, options and the way out (Esc)"));
                    if menu.clicked() {
                        self.open_menu = true;
                    }
                    let shown = game.shown_overlay();
                    let label = match shown {
                        Some(o) => format!("{} · {} ▾", t("MAP"), t(o.label())),
                        None => format!("{} ▾", t("MAP")),
                    };
                    let map = ui.menu_button(egui::RichText::new(label).font(crate::theme::strong(11.0)).color(if shown.is_some() { ACCENT } else { crate::theme::STEEL }), |ui| {
                        for o in crate::overlay::Overlay::ALL {
                            let key = game.keys.text(o.action());
                            if ui.selectable_label(shown == Some(o), format!("{}  ({key})", t(o.label()))).on_hover_text(t(o.legend())).clicked() {
                                self.toggle_overlay = Some(o);
                                ui.close();
                            }
                        }
                    })
                    .response
                    .on_hover_text(t("Map overlays: where units can go, the heights, the metal, what your units see"));
                    self.anchors.map_menu = Some(map.rect);
                    if game.paused {
                        ta_ui::widgets::tag(ui, n("PAUSED"), ACCENT);
                    } else if (game.speed - 1.0).abs() > 0.01 {
                        ta_ui::widgets::tag(ui, &tf("SPEED ×{speed}", &[("speed", &format!("{:.2}", game.speed))]), ACCENT);
                    }
                    net_signal(ui, game);
                    let used = ui.min_rect().width();
                    if (used - self.top_right_width).abs() > 0.5 {
                        self.top_right_width = used;
                        ui.ctx().request_repaint();
                    }
                });
            });
            background
        });
        root.painter().set(bar.inner, self.skin.top(root.ctx(), bar.response.rect));
        self.top_bar_bottom = bar.response.rect.bottom();
    }

    /// Where the minimap's plate stands this frame (see [`deck::minimap_layout`]), for what hangs
    /// at the right edge to keep below it and for the deck to draw it.
    fn lay_out_minimap(&mut self, ctx: &egui::Context, game: &Game) {
        let aspect = game.map.height_px() as f32 / game.map.width_px().max(1) as f32;
        self.minimap_rect = Some(deck::minimap_layout(ctx.content_rect(), self.top_bar_bottom, aspect));
    }

    fn overlays(&mut self, ctx: &egui::Context, vfs: &Vfs, game: &Game, camera: &Camera, view: Rect) {
        // What is drawn over the battlefield keeps to the part of it the panels leave free (the
        // battle goes on under the resource bar and TA's panels); the deck's plates cover it
        // themselves, the battlefield showing between them.
        let free = self.free_view(ctx);
        let free = if self.classic { free } else { Rect::from_min_max(free.min, egui::pos2(free.max.x, ctx.content_rect().bottom())) };
        let painter = ctx.layer_painter(egui::LayerId::background()).with_clip_rect(free.intersect(view));
        // Newly picked units (for their rings' pop); the ones no longer selected forgotten.
        {
            let now = ctx.input(|i| i.time) as f32;
            let first: rustc_hash::FxHashSet<UnitId> = game.selection.iter().take(200).copied().collect();
            self.picked_at.retain(|id, _| first.contains(id));
            for &id in &first {
                self.picked_at.entry(id).or_insert(now);
            }
        }
        let ppp = ctx.pixels_per_point();
        let alpha = game.alpha();
        let project = |p: Vec3| -> Option<Pos2> {
            let c = camera.view_proj() * p.extend(1.0);
            if c.w <= 0.0 {
                return None;
            }
            let ndc = c.xy() / c.w;
            let s = Vec2::new((ndc.x + 1.0) * 0.5 * camera.viewport.x, (1.0 - ndc.y) * 0.5 * camera.viewport.y) / ppp;
            Some(Pos2::new(s.x, s.y))
        };
        // Orders of the selected units: a line through each queued point, coloured by kind.
        // They show for a moment after an order is given, while Shift is held (to plan more),
        // and for the unit under the pointer; otherwise the battlefield stays clear of them.
        let ground_at = |x: f32, z: f32| game.sim.world.height_at(ta_sim::Fx::from_f32(x), ta_sim::Fx::from_f32(z)).to_f32().max(game.sim.world.sea_level.to_f32());
        let clock = ctx.input(|i| i.time) as f32;
        let planning = ctx.input(|i| i.modifiers.shift);
        let since_order = game.last_order.map_or(f32::MAX, |t| t.elapsed().as_secs_f32());
        let routes = if planning { 1.0 } else { 1.0 - ((since_order - 1.6) / 1.2).clamp(0.0, 1.0) };
        if routes < 1.0 && routes > 0.0 {
            ctx.request_repaint();
        }
        let hovered_own = game.hovered.filter(|h| game.sim.unit(*h).is_some_and(|u| u.owner == game.local_player));
        let shown_routes: Vec<(ta_sim::UnitId, f32)> = game
            .selection
            .iter()
            .take(80)
            .map(|&id| (id, routes))
            .chain(hovered_own.map(|h| (h, 1.0)))
            .filter(|&(_, k)| k > 0.0)
            .collect();
        let dashes = |a: Pos2, b: Pos2, color: Color32, fade: f32| {
            let stroke = Stroke::new(1.6, color.gamma_multiply(0.75 * fade));
            egui::Shape::dashed_line_with_offset(&[a, b], stroke, &[7.0], &[5.0], (clock * 22.0) % 12.0)
        };
        for &id in game.selection.iter().take(80) {
            let Some(u) = game.sim.unit(id) else { continue };
            // Factories: the route their units take, with a flag at its first stop.
            if game.sim.unit_type(u).is_factory && !u.rally.is_empty() {
                let mut prev = project(Vec3::from_array(u.pos.to_f32()) + Vec3::Y * 8.0);
                for (k, o) in u.rally.iter().enumerate() {
                    let (p, c) = match o {
                        ta_sim::Order::Move(p) => (Some(p.to_f32()), Color32::from_rgb(120, 200, 255)),
                        ta_sim::Order::Fight(p) => (Some(p.to_f32()), Color32::from_rgb(235, 180, 60)),
                        ta_sim::Order::Patrol(p) => (Some(p.to_f32()), Color32::from_rgb(90, 150, 255)),
                        ta_sim::Order::Guard(g) => (game.sim.unit(*g).map(|x| x.pos.to_f32()), Color32::from_rgb(90, 220, 220)),
                        _ => (None, Color32::WHITE),
                    };
                    let Some(r) = p else { continue };
                    let here = project(Vec3::new(r[0], ground_at(r[0], r[2]) + 3.0, r[2]));
                    if let (Some(a), Some(b)) = (prev, here) {
                        painter.line_segment([a, b], Stroke::new(1.2, c.gamma_multiply(0.6)));
                        if k == 0 {
                            painter.line_segment([b, b - EVec2::new(0.0, 16.0)], Stroke::new(1.5, c));
                            painter.add(egui::Shape::convex_polygon(vec![b - EVec2::new(0.0, 16.0), b - EVec2::new(-9.0, 12.5), b - EVec2::new(0.0, 9.0)], c, Stroke::NONE));
                        } else {
                            painter.circle_filled(b, 3.0, c);
                        }
                    }
                    prev = here;
                }
            }
        }
        for &(id, fade) in &shown_routes {
            let Some(u) = game.sim.unit(id) else { continue };
            if u.orders.is_empty() {
                continue;
            }
            let Some(start) = game.scene.unit_position(id, alpha) else { continue };
            // (The first leg from the edge of the unit's ring, towards its first point: from its
            // middle the line ran over the unit's body.)
            let reach = game.sim.unit_type(u).radius.to_f32() + 3.0;
            let mut prev = None;
            let mut first = true;
            for o in u.orders.iter().take(16) {
                let (point, color) = match o {
                    ta_sim::Order::Move(p) => (Some(p.to_f32()), Color32::from_rgb(90, 220, 90)),
                    ta_sim::Order::Fight(p) => (Some(p.to_f32()), Color32::from_rgb(235, 180, 60)),
                    ta_sim::Order::Patrol(p) => (Some(p.to_f32()), Color32::from_rgb(90, 150, 255)),
                    ta_sim::Order::Unload(p) => (Some(p.to_f32()), Color32::from_rgb(200, 150, 255)),
                    ta_sim::Order::Build { pos, .. } => (Some(pos.to_f32()), Color32::from_rgb(90, 220, 220)),
                    ta_sim::Order::Attack(ta_sim::Target::Ground(p)) | ta_sim::Order::ManualFire(ta_sim::Target::Ground(p)) => (Some(p.to_f32()), Color32::from_rgb(235, 70, 60)),
                    ta_sim::Order::Attack(ta_sim::Target::Unit(t)) | ta_sim::Order::ManualFire(ta_sim::Target::Unit(t)) | ta_sim::Order::Capture(t) => {
                        (game.sim.unit(*t).filter(|x| game.seen(x) != ta_sim::Seen::Hidden).map(|x| x.pos.to_f32()), Color32::from_rgb(235, 70, 60))
                    }
                    ta_sim::Order::Repair(t) | ta_sim::Order::Guard(t) | ta_sim::Order::Load(t) => (game.sim.unit(*t).map(|x| x.pos.to_f32()), Color32::from_rgb(90, 220, 220)),
                    ta_sim::Order::Reclaim(f) => (game.sim.features.get(*f).map(|x| x.pos.to_f32()), Color32::from_rgb(200, 200, 90)),
                    _ => (None, Color32::WHITE),
                };
                let Some(p) = point else { continue };
                if std::mem::take(&mut first) {
                    let away = Vec2::new(p[0] - start.x, p[2] - start.z);
                    prev = if away.length() > reach * 1.5 {
                        let edge = Vec2::new(start.x, start.z) + away.normalize() * reach;
                        project(Vec3::new(edge.x, ground_at(edge.x, edge.y).max(start.y - 2.0) + 3.0, edge.y))
                    } else {
                        project(start + Vec3::Y * 4.0)
                    };
                }
                let here = project(Vec3::new(p[0], ground_at(p[0], p[2]) + 3.0, p[2]));
                if let (Some(a), Some(b)) = (prev, here) {
                    painter.extend(dashes(a, b, color, fade));
                    painter.circle_filled(b, 3.2, color.gamma_multiply(fade));
                    painter.circle_stroke(b, 3.2, Stroke::new(1.0, Color32::from_black_alpha((140.0 * fade) as u8)));
                }
                prev = here;
            }
        }
        // Orders just given: a ring closing in on the spot, with chevrons, or a bracket pulsing
        // round the unit it was given against.
        for m in &game.order_marks {
            let age = m.born.elapsed().as_secs_f32() / crate::game::MARK_LIFE;
            if age >= 1.0 {
                continue;
            }
            ctx.request_repaint();
            let color = mark_color(m.kind);
            let fade = 1.0 - age * age;
            let centre = match m.unit {
                Some(id) => game.scene.unit_position(id, alpha).unwrap_or(m.pos),
                None => m.pos,
            };
            if let Some(id) = m.unit {
                // Corners of a square round the unit, tightening and pulsing twice.
                let r_world = game.sim.unit(id).map_or(16.0, |u| game.sim.unit_type(u).radius.to_f32()) + 8.0;
                let Some(c) = project(centre + Vec3::Y * 6.0) else { continue };
                let r = (r_world * camera.zoom / ppp).clamp(12.0, 70.0) * (1.35 - 0.35 * (age * 2.0).min(1.0));
                let pulse = 0.6 + 0.4 * (age * std::f32::consts::TAU * 2.0).cos().abs();
                let stroke = Stroke::new(2.2, color.gamma_multiply(fade * pulse));
                let arm = r * 0.45;
                for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                    let corner = c + EVec2::new(sx * r, sy * r * 0.75);
                    painter.line_segment([corner, corner - EVec2::new(sx * arm, 0.0)], stroke);
                    painter.line_segment([corner, corner - EVec2::new(0.0, sy * arm * 0.75)], stroke);
                }
            } else {
                // A ring on the ground shrinking into the spot, and four chevrons closing in.
                let size = if m.small { 0.55 } else { 1.0 };
                let ease = 1.0 - (1.0 - age).powi(3);
                let r = (30.0 - 22.0 * ease) * size;
                let n = 40;
                let pts: Vec<Pos2> = (0..=n)
                    .filter_map(|k| {
                        let a = k as f32 / n as f32 * std::f32::consts::TAU;
                        let (x, z) = (centre.x + a.cos() * r, centre.z + a.sin() * r);
                        project(Vec3::new(x, ground_at(x, z) + 1.5, z))
                    })
                    .collect();
                painter.add(egui::Shape::line(pts, Stroke::new(2.0, color.gamma_multiply(fade))));
                if !m.small {
                    for k in 0..4 {
                        let a = k as f32 * std::f32::consts::FRAC_PI_2 + std::f32::consts::FRAC_PI_4;
                        let (dx, dz) = (a.cos(), a.sin());
                        let d = r + 9.0 - 4.0 * ease;
                        let tip = Vec3::new(centre.x + dx * (d - 5.0), 0.0, centre.z + dz * (d - 5.0));
                        let wing = |s: f32| Vec3::new(centre.x + dx * d - dz * 4.0 * s, 0.0, centre.z + dz * d + dx * 4.0 * s);
                        let on_ground = |v: Vec3| project(Vec3::new(v.x, ground_at(v.x, v.z) + 1.5, v.z));
                        if let (Some(t), Some(l), Some(rw)) = (on_ground(tip), on_ground(wing(1.0)), on_ground(wing(-1.0))) {
                            painter.add(egui::Shape::convex_polygon(vec![t, l, rw], color.gamma_multiply(fade), Stroke::NONE));
                        }
                    }
                }
            }
        }
        // Mission objective areas: a pulsing ring on the ground.
        let t = game.sim.tick as f32 / 30.0;
        for (x, z, r) in game.objective_areas() {
            let pulse = 0.55 + 0.45 * (t * 3.0).sin();
            let pts: Vec<Pos2> = (0..=48)
                .filter_map(|i| {
                    let a = i as f32 / 48.0 * std::f32::consts::TAU;
                    let (px, pz) = (x + a.cos() * r, z + a.sin() * r);
                    let y = game.sim.world.height_at(ta_sim::Fx::from_f32(px), ta_sim::Fx::from_f32(pz)).to_f32().max(game.sim.world.sea_level.to_f32());
                    project(Vec3::new(px, y + 2.0, pz))
                })
                .collect();
            let c = Color32::from_rgba_unmultiplied(120, 255, 140, (140.0 + 115.0 * pulse) as u8);
            painter.add(egui::Shape::line(pts, Stroke::new(2.5, c)));
            let y = game.sim.world.height_at(ta_sim::Fx::from_f32(x), ta_sim::Fx::from_f32(z)).to_f32();
            if let Some(s) = project(Vec3::new(x, y + 30.0, z)) {
                painter.text(s, egui::Align2::CENTER_BOTTOM, ta_ui::i18n::t("OBJECTIVE"), egui::FontId::proportional(14.0), c);
            }
        }
        // Only our side knows its intended impact points. Radar detects a missile's
        // flight, not its destination; each warning lasts exactly as long as its missile.
        for marker in crate::game::nuclear::impact_markers(&game.sim, game.viewer()) {
            let p = marker.at;
            let pulse = 0.5 + 0.5 * (marker.age * 6.0).sin();
            let r = marker.radius;
            let pts: Vec<Pos2> = (0..=40)
                .filter_map(|i| {
                    let a = i as f32 / 40.0 * std::f32::consts::TAU;
                    let (px, pz) = (p.x + a.cos() * r, p.z + a.sin() * r);
                    let y = game.sim.world.height_at(ta_sim::Fx::from_f32(px), ta_sim::Fx::from_f32(pz)).to_f32().max(game.sim.world.sea_level.to_f32());
                    project(Vec3::new(px, y + 2.0, pz))
                })
                .collect();
            let c = Color32::from_rgba_unmultiplied(255, 70, 50, (120.0 + 135.0 * pulse) as u8);
            painter.add(egui::Shape::line(pts, Stroke::new(3.0, c)));
            // (Its name, and under it how long until it comes down.)
            if let Some(s) = project(p + Vec3::Y * 40.0) {
                let shadow = Color32::from_black_alpha(200);
                let label = ta_ui::i18n::t("NUCLEAR IMPACT").to_string();
                let when = crate::hud::duration(marker.seconds.max(0.0));
                for (offset, colour) in [(EVec2::new(1.0, 1.0), shadow), (EVec2::ZERO, c)] {
                    painter.text(s + offset, egui::Align2::CENTER_BOTTOM, &label, egui::FontId::proportional(15.0), colour);
                    painter.text(s + offset, egui::Align2::CENTER_TOP, &when, egui::FontId::proportional(14.0), colour);
                }
            }
        }
        // Shift over a unit that cloaks: the white circle any enemy inside of uncovers it (TA
        // 3.1's minimum cloaking distance).
        if (planning || game.shift_held)
            && let Some(u) = game.hovered.and_then(|h| game.sim.unit(h)).filter(|u| game.sim.unit_type(u).def.cloak_cost > 0.0)
            && let Some(centre) = game.scene.unit_position(u.id, alpha)
        {
            let r = game.sim.unit_type(u).def.min_cloak_distance.max(8.0) as f32;
            let pts: Vec<Pos2> = (0..=48)
                .filter_map(|i| {
                    let a = i as f32 / 48.0 * std::f32::consts::TAU;
                    let (px, pz) = (centre.x + a.cos() * r, centre.z + a.sin() * r);
                    project(Vec3::new(px, ground_at(px, pz) + 2.0, pz))
                })
                .collect();
            painter.add(egui::Shape::line(pts, Stroke::new(1.6, Color32::from_white_alpha(210))));
        }
        // Shift over a constructor: TA's squares round the planned buildings, green for the
        // selected constructors', blue for the others'.
        if game.show_all_plans {
            let me = game.local_player;
            let chosen: rustc_hash::FxHashSet<UnitId> = game.selection.iter().copied().collect();
            for u in game.sim.units().filter(|u| u.owner == me && !u.dead) {
                let selected = chosen.contains(&u.id);
                let colour = if selected { Color32::from_rgb(80, 230, 90) } else { Color32::from_rgb(80, 150, 255) };
                for (k, o) in u.orders.iter().enumerate().take(40) {
                    // (The job under way shows its own frame.)
                    let ta_sim::Order::Build { type_id, pos } = o else { continue };
                    if k == 0 && u.build.is_some() {
                        continue;
                    }
                    let t = game.sim.types.get(*type_id);
                    let (hx, hz) = (t.footprint.0 as f32 * 8.0, t.footprint.1 as f32 * 8.0);
                    let c = Vec3::from_array(pos.to_f32());
                    let corners: Vec<Pos2> = [(-hx, -hz), (hx, -hz), (hx, hz), (-hx, hz), (-hx, -hz)]
                        .iter()
                        .filter_map(|&(dx, dz)| project(Vec3::new(c.x + dx, ground_at(c.x + dx, c.z + dz) + 1.5, c.z + dz)))
                        .collect();
                    if corners.len() == 5 {
                        painter.add(egui::Shape::line(corners, Stroke::new(1.5, colour)));
                    }
                }
            }
        }
        // Shift over a mine (The Core Contingency's): its trigger circle, steady red, and its
        // blast's, a red ring pulsing out to it.
        if (planning || game.shift_held)
            && let Some(u) = game.hovered.and_then(|h| game.sim.unit(h)).filter(|u| {
                let t = game.sim.unit_type(u);
                t.def.kamikaze && t.locomotion == ta_sim::Locomotion::Static
            })
            && let Some(centre) = game.scene.unit_position(u.id, alpha)
        {
            let t = game.sim.unit_type(u);
            let ring = |r: f32, stroke: Stroke| {
                let pts: Vec<Pos2> = (0..=48)
                    .filter_map(|i| {
                        let a = i as f32 / 48.0 * std::f32::consts::TAU;
                        let (px, pz) = (centre.x + a.cos() * r, centre.z + a.sin() * r);
                        project(Vec3::new(px, ground_at(px, pz) + 2.0, pz))
                    })
                    .collect();
                painter.add(egui::Shape::line(pts, stroke));
            };
            ring(t.def.kamikaze_distance.max(8.0) as f32, Stroke::new(1.8, Color32::from_rgb(230, 60, 50)));
            let boom = if t.def.self_destruct_as.is_empty() { &t.def.explode_as } else { &t.def.self_destruct_as };
            if let Some(w) = game.sim.weapon_types.types.iter().find(|w| w.name.eq_ignore_ascii_case(boom)) {
                let blast = w.aoe_radius.to_f32().max(8.0);
                let pulse = (ctx.input(|i| i.time) as f32 * 0.8).fract();
                ring(blast * pulse, Stroke::new(2.0, Color32::from_rgba_unmultiplied(230, 60, 50, (220.0 * (1.0 - pulse)) as u8)));
                ring(blast, Stroke::new(1.0, Color32::from_rgba_unmultiplied(230, 60, 50, 90)));
                ctx.request_repaint();
            }
        }
        // Radar contacts: pulsing rings where sensors report something unseen. (Who they are
        // changes with the ticks, not the frames: found once a tick.)
        let pulse = 0.6 + 0.4 * (game.sim.tick as f32 * 0.21).sin();
        let key = (game.sim.tick, game.viewer());
        if self.radar_contacts.0 != Some(key) {
            self.radar_contacts = (Some(key), game.sim.units().filter(|u| game.seen(u) == ta_sim::Seen::Radar).map(|u| u.id).collect());
        }
        for u in self.radar_contacts.1.iter().filter_map(|&id| game.sim.unit(id)) {
            let Some(pos) = game.scene.unit_position(u.id, alpha) else { continue };
            let Some(s) = project(pos + Vec3::Y * 6.0) else { continue };
            let c = if game.team_markers { Color32::WHITE } else { crate::palette::team_color(game.sim.players.get(u.owner as usize).map_or(0, |p| p.color)) };
            let r = (6.0 * camera.zoom / ppp).clamp(4.0, 9.0);
            painter.circle_filled(s, r * 0.45, c.gamma_multiply(pulse));
            painter.circle_stroke(s, r, Stroke::new(1.2, c.gamma_multiply(0.5 + 0.5 * pulse)));
            if game.team_markers {
                painter.circle_filled(s, 6.5, Color32::BLACK);
                painter.text(s, egui::Align2::CENTER_CENTER, "?", crate::theme::strong(12.0), Color32::WHITE);
            }
        }
        crate::team_markers::world(game, camera, ppp, &painter, project);
        // Health / build bars over selected, hovered and damaged units.
        let selection: rustc_hash::FxHashSet<ta_sim::UnitId> = game.selection.iter().copied().collect();
        // (Far out the strategic icons' rings mark the selection.)
        let strategic = crate::strategic::shown(game, camera, ppp) > 0.5;
        // Control group digits (the lowest group a unit is in).
        let mut group_of: rustc_hash::FxHashMap<ta_sim::UnitId, usize> = Default::default();
        for (g, ids) in game.groups.iter().enumerate().rev() {
            for &id in ids {
                group_of.insert(id, g);
            }
        }
        let close = camera.zoom >= 0.8;
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        let steady = game.scene.reduced_flashes;
        // (Square corners on a big selection's bars: rounded ones cost several times as many
        // vertices, thousands of them a frame.)
        let round = if selection.len() > 300 { 0.0 } else { 1.0 };
        self.bar_trails.retain(|_, t| clock - t.seen < 2.0);
        // The time left over what is being built fades out as the strategic icons take over.
        self.etas.forget_old(clock);
        let eta_strength = (1.0 - crate::strategic::amount(camera, ppp) * 2.0).clamp(0.0, 1.0);
        // (How fast each build goes at full pace: found once a frame, when something shows one.)
        let mut paces: Option<HashMap<UnitId, f32>> = None;
        for u in game.sim.units() {
            let selected = !selection.is_empty() && selection.contains(&u.id);
            let t = game.sim.unit_type(u);
            let damaged = u.health < t.max_health;
            let countdown = u.self_destruct_manual && u.self_destruct_at.is_some();
            let group = if close || selected { group_of.get(&u.id).copied() } else { None };
            let capturing = u.capture_progress > ta_sim::Fx::ZERO;
            let shown = match game.health_bars {
                ta_config::HealthBars::Auto => close && (!u.is_complete() || damaged && u.owner == game.local_player),
                ta_config::HealthBars::Damaged => !u.is_complete() || damaged,
                ta_config::HealthBars::Selected => false,
            };
            let bar = selected || capturing || game.hovered == Some(u.id) || shown;
            // (The viewer's sight is asked only of those with something to show.)
            if !(countdown || group.is_some() || bar) || game.seen(u) != ta_sim::Seen::Visible {
                continue;
            }
            // (Where it is on screen first: nothing is drawn of one out of sight.)
            let Some(pos) = game.scene.unit_position(u.id, alpha) else { continue };
            let top = pos + Vec3::Y * (t.model.height.to_f32() + 6.0);
            let Some(s) = project(top) else { continue };
            if s.x < -50.0 || s.y < -50.0 || s.x > camera.viewport.x / ppp + 50.0 || s.y > camera.viewport.y / ppp + 50.0 {
                continue;
            }
            // Self-destruct countdown: big red seconds over the unit.
            if let (Some(at), true) = (u.self_destruct_at, u.self_destruct_manual) && let Some(s) = project(pos + Vec3::Y * (t.model.height.to_f32() + 18.0)) {
                let secs = at.saturating_sub(game.sim.tick).div_ceil(30);
                painter.text(s, egui::Align2::CENTER_BOTTOM, secs.to_string(), egui::FontId::proportional(20.0), Color32::from_rgb(255, 70, 50));
            }
            let w = (t.radius.to_f32() * camera.zoom / ppp).clamp(18.0, 60.0);
            if let Some(g) = group {
                // Its group on a small dark plate at the end of the health bar, as on the group bar.
                const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
                let badge = Rect::from_center_size(Pos2::new(s.x + w / 2.0 + 8.0, s.y - 4.0), EVec2::new(11.0, 13.0));
                painter.rect_filled(badge, 2.0 * round, Color32::from_black_alpha(190));
                painter.rect_stroke(badge, 2.0 * round, Stroke::new(1.0, crate::theme::AMBER_DIM), egui::StrokeKind::Inside);
                painter.text(badge.center() + EVec2::new(0.0, 0.5), egui::Align2::CENTER_CENTER, DIGITS.get(g).copied().unwrap_or("?"), crate::theme::display(11.0), crate::theme::CREAM);
            }
            // (Far out a selected unit at full health shows none: the strategic icons' rings
            // mark the selection.)
            if !bar || strategic && selected && !damaged && u.is_complete() && !capturing && u.paralyzed_until <= game.sim.tick && game.hovered != Some(u.id) {
                continue;
            }
            if u.capture_progress > ta_sim::Fx::ZERO {
                let c = u.capture_progress.to_f32().clamp(0.0, 1.0);
                let cb = Rect::from_min_size(Pos2::new(s.x - w / 2.0, s.y - 12.0), EVec2::new(w, 3.0));
                painter.rect_filled(cb, 1.0, Color32::from_black_alpha(180));
                painter.rect_filled(Rect::from_min_size(cb.min, EVec2::new(w * c, 3.0)), 1.0, Color32::from_rgb(200, 120, 255));
            }
            let frac = (u.health / t.max_health).to_f32().clamp(0.0, 1.0);
            let bar = Rect::from_min_size(Pos2::new(s.x - w / 2.0, s.y - 6.0), EVec2::new(w, 4.0));
            // The health just lost lingers as a pale chunk, then drains away (how hard the blow
            // was, at a glance); healing shows at once.
            let now_tick = game.sim.tick;
            let trail = self.bar_trails.entry(u.id).or_insert(BarTrail { shown: frac, last: frac, hit_at: f32::MIN, seen: clock, stun: (0, 0) });
            if frac < trail.last - 1e-4 {
                trail.hit_at = clock;
            }
            if frac >= trail.shown {
                trail.shown = frac;
            } else if clock - trail.hit_at > TRAIL_HOLD {
                trail.shown = (trail.shown - dt * TRAIL_DRAIN).max(frac);
            }
            trail.last = frac;
            trail.seen = clock;
            if u.paralyzed_until > trail.stun.0 {
                trail.stun = (u.paralyzed_until, u.paralyzed_until.saturating_sub(now_tick).max(1));
            }
            let trail = *trail;
            if trail.shown > frac + 1e-3 || clock - trail.hit_at < HIT_LIGHT {
                ctx.request_repaint();
            }
            // (In a dark frame: full, a bar was a bare slab of green over the ground.)
            painter.rect_filled(bar.expand(1.0), 1.5 * round, Color32::from_black_alpha(200));
            if trail.shown > frac {
                let chunk = Rect::from_min_max(Pos2::new(bar.min.x + w * frac, bar.min.y), Pos2::new(bar.min.x + w * trail.shown, bar.max.y));
                painter.rect_filled(chunk, round, Color32::from_rgb(255, 226, 150));
            }
            // Nearly gone, the red beats (steady with reduced flashes).
            let mut colour = crate::palette::health_color(frac);
            if frac < 0.25 && !steady {
                colour = colour.gamma_multiply(0.62 + 0.38 * (clock * 5.0).sin().abs());
                ctx.request_repaint();
            }
            painter.rect_filled(Rect::from_min_size(bar.min, EVec2::new(w * frac, 4.0)), round, colour);
            // Its quarters marked on a long bar.
            if w >= 34.0 {
                for q in [0.25, 0.5, 0.75] {
                    painter.vline(bar.min.x + w * q, bar.y_range(), Stroke::new(1.0, Color32::from_black_alpha(120)));
                }
            }
            // A hit lights its frame for a moment.
            let since = clock - trail.hit_at;
            if since < HIT_LIGHT && !steady {
                let a = ((1.0 - since / HIT_LIGHT) * 210.0) as u8;
                painter.rect_stroke(bar.expand(1.0), 1.5, Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 240, 210, a)), egui::StrokeKind::Outside);
            }
            // Paralysed: the time it has left, in electric blue over the bar.
            if u.paralyzed_until > now_tick {
                let left = (u.paralyzed_until - now_tick) as f32 / trail.stun.1 as f32;
                let sb = Rect::from_min_size(Pos2::new(bar.min.x, bar.min.y - 4.0), EVec2::new(w, 2.0));
                painter.rect_filled(sb.expand(1.0), 1.0, Color32::from_black_alpha(180));
                painter.rect_filled(Rect::from_min_size(sb.min, EVec2::new(w * left.clamp(0.0, 1.0), 2.0)), 1.0, Color32::from_rgb(110, 175, 255));
            }
            // Veterans: a gold chevron per rank at the bar's left end.
            for k in 0..u.rank() {
                let x = bar.min.x - 7.0 - k as f32 * 5.0;
                let (y, h) = (bar.center().y, 3.5);
                let chevron = vec![Pos2::new(x - 2.5, y - h), Pos2::new(x + 1.5, y), Pos2::new(x - 2.5, y + h)];
                painter.add(egui::Shape::line(chevron, Stroke::new(1.8, ACCENT)));
            }
            if !u.is_complete() {
                let b = u.build_progress.to_f32().clamp(0.0, 1.0);
                let bb = Rect::from_min_size(Pos2::new(bar.min.x, bar.min.y + 6.0), EVec2::new(w, 3.0));
                painter.rect_filled(bb.expand(1.0), 1.5, Color32::from_black_alpha(200));
                painter.rect_filled(Rect::from_min_size(bb.min, EVec2::new(w * b, 3.0)), 1.0, Color32::from_rgb(80, 200, 255));
                // The time left, small and dim over the bars, for what we and our allies are
                // building (not far out, where the strategic icons are, and not in photo mode:
                // the interface is not drawn there).
                if close && eta_strength > 0.0 && game.may_inspect(u.owner) {
                    let paces = paces.get_or_insert_with(|| eta::builder_paces(&game.sim));
                    let done = (u.build_work as f64 / game.sim.unit_type(u).build_work.max(1) as f64) as f32;
                    if let Some(left) = self.etas.of(u.id, game.sim.tick, done, paces.get(&u.id).copied().unwrap_or(0.0), clock) {
                        eta::paint(&painter, Pos2::new(s.x, bar.min.y - 5.0), left, eta_strength);
                    }
                }
            }
            if selected && !strategic {
                // On the ground round the unit, as a decal lies (drawn with the scene, so that the
                // unit standing in it and its neighbours hide it: painted over the picture, rings
                // crossed hulls and turrets), in health's green, a little larger for a moment as it
                // is picked; an aircraft's on the ground under it, a ship's on the water.
                let picked = self.picked_at.get(&u.id).map_or(1.0, |t| (clock - *t) / 0.25);
                let grow = if picked < 1.0 { 1.0 + 0.35 * (1.0 - picked).powi(2) } else { 1.0 };
                let r = (t.radius.to_f32() + 3.0) * grow;
                let sea = game.sim.world.sea_level.to_f32();
                let lowest = if t.locomotion == ta_sim::Locomotion::Air { sea } else { (pos.y - 2.0).max(sea) };
                // ((120, 255, 140) in linear light.)
                self.rings.push(ta_render::RingInstance::new([pos.x, pos.z], r, lowest, [0.19, 1.0, 0.26], 1.0, 1.6 * ppp, 3.0 * ppp));
                if picked < 1.0 {
                    ctx.request_repaint();
                }
            }
        }
        // Team signals on the ground: a tall beacon in the meaning's colour, with who gave it
        // and, for a call, what they mean.
        let now_s = game.sim.tick as f32 / 30.0;
        for ping in &game.pings {
            let age = ping.age(now_s);
            if !(0.0..1.0).contains(&age) {
                continue;
            }
            let c = crate::signals::colour(ping.kind, ((1.0 - age) * 255.0) as u8);
            if let (Some(base), Some(top)) = (project(ping.at), project(ping.at + Vec3::Y * 90.0)) {
                painter.line_segment([base, top], Stroke::new(2.0, c));
                let pulse = ((now_s - ping.time) * 1.5).fract();
                painter.circle_stroke(base, 6.0 + pulse * 26.0, Stroke::new(2.0, c));
                let words = if ping.kind == ta_sim::SignalKind::Look { ping.from.clone() } else { format!("{} · {}", ping.from, ta_ui::i18n::t(crate::signals::label(ping.kind))) };
                // (A shadow under the words: legible over bright ground.)
                let font = crate::theme::strong(13.0);
                painter.text(top + EVec2::new(1.0, 1.0), egui::Align2::CENTER_BOTTOM, &words, font.clone(), Color32::from_black_alpha((210.0 * (1.0 - age)) as u8));
                painter.text(top, egui::Align2::CENTER_BOTTOM, words, font, c);
            }
        }
        if let Some(wheel) = &game.signal_wheel {
            crate::signals::draw_wheel(ctx, wheel);
        }
        // Formation being drawn: the line and where each unit will stand.
        if let Some(path) = &game.formation {
            let pts: Vec<Pos2> = path.iter().filter_map(|&p| project(p + Vec3::Y * 3.0)).collect();
            if pts.len() >= 2 {
                painter.add(egui::Shape::line(pts, Stroke::new(2.0, Color32::from_rgb(120, 255, 120))));
            }
            let movers = game.selection.iter().filter(|&&id| game.sim.unit(id).is_some_and(|u| game.sim.unit_type(u).locomotion != ta_sim::Locomotion::Static)).count();
            for spot in crate::game::spread_along(path, movers) {
                if let Some(s) = project(spot + Vec3::Y * 3.0) {
                    painter.circle_filled(s, 3.5, Color32::from_rgb(120, 255, 120));
                }
            }
        }
        // Hovered unit: a card by the cursor once the pointer rests on it (not while an order
        // is being targeted): its name, whose it is and its health at first; what it is and
        // what it is good for if the pointer stays.
        let now = ctx.input(|i| i.time) as f32;
        self.hover_since = match (game.hovered, self.hover_since) {
            (Some(id), Some((was, t))) if was == id => Some((id, t)),
            (Some(id), _) => Some((id, now)),
            (None, _) => None,
        };
        let rest = self.hover_since.map_or(0.0, |(_, t)| now - t);
        if rest < 1.1 && self.hover_since.is_some() {
            ctx.request_repaint();
        }
        let pointer = ctx.pointer_hover_pos().or_else(|| game.cursor_override.and_then(project));
        if let (Some(id), None, Some(mouse), true) = (game.hovered, game.command_mode, pointer, rest >= HOVER_CARD_AFTER) {
            if let Some(u) = game.sim.unit(id).filter(|u| game.seen(u) == ta_sim::Seen::Visible) {
                use ta_ui::i18n::tf;
                let t = game.sim.unit_type(u);
                let owner = game.sim.players.get(u.owner as usize);
                let more = rest >= 1.0;
                let mut lines = vec![(t.def.display_name.clone(), Color32::WHITE, 14.0)];
                if more && !t.def.description.trim().is_empty() && !t.def.description.trim().eq_ignore_ascii_case(t.def.display_name.trim()) {
                    lines.push((t.def.description.clone(), Color32::from_gray(170), 11.0));
                }
                if game.team_markers {
                    let marker = crate::team_markers::player(game, u.owner);
                    lines.push((format!("{} {} · {}", marker.label(), game.player_label(u.owner), marker.relation.name()), Color32::WHITE, 12.0));
                } else if game.spectator || u.owner != game.local_player {
                    let ally = !game.spectator && game.sim.allied(u.owner, game.local_player);
                    let who = if ally { tf("Ally: {name}", &[("name", &game.player_label(u.owner))]) } else { game.player_label(u.owner).to_string() };
                    lines.push((format!("{who} · {}", owner.map_or("", |p| p.side.as_str())), crate::palette::team_color(owner.map_or(0, |p| p.color)), 11.0));
                }
                let mut facts = Vec::new();
                if !u.is_complete() {
                    facts.push(tf("{pct}% built", &[("pct", &format!("{:.0}", (u.build_progress.to_f32() * 100.0).floor()))]));
                }
                if u.kills > 0 {
                    let rank = [n("Recruit"), n("Veteran"), n("Hardened"), n("Elite")][usize::from(u.rank().min(3))];
                    facts.push(if u.kills == 1 { tf("{rank} · 1 kill", &[("rank", &ta_ui::i18n::t(rank))]) } else { tf("{rank} · {n} kills", &[("rank", &ta_ui::i18n::t(rank)), ("n", &u.kills)]) });
                }
                if u.cloaked {
                    facts.push(ta_ui::i18n::t("Cloaked").to_string());
                }
                if u.paralyzed_until > game.sim.tick {
                    facts.push(tf("Paralyzed {secs} s", &[("secs", &(u.paralyzed_until - game.sim.tick).div_ceil(30))]));
                }
                if u.self_destruct_manual && u.self_destruct_at.is_some() {
                    facts.push(ta_ui::i18n::t("Self-destructing").to_string());
                }
                // A constructor of ours on duty: which, and what it is about when its orders do not say.
                let duty = if u.owner == game.local_player { game.duty_of(id) } else { None };
                if let Some(duty) = duty {
                    facts.push(tf("ON DUTY · {duty}", &[("duty", &ta_ui::i18n::t(crate::game::duty::label(duty)))]));
                }
                if !facts.is_empty() {
                    lines.push((facts.join("  ·  "), Color32::from_gray(200), 11.0));
                }
                if duty.is_some() && let Some(line) = game.duty_line(id) {
                    lines.push((line, crate::theme::SKY, 11.0));
                }
                // What a finished unit of ours (or an ally's) makes and uses of metal and energy
                // now, under its health; the type's storage and sensors in the lines below.
                let figures = if game.may_inspect(u.owner) && u.is_complete() { figure_rows(&game.sim.unit_flows(u)) } else { Vec::new() };
                if more {
                    for line in unit_fact_lines(&game.sim, t, if figures.is_empty() { Economy::Type } else { Economy::Storage }) {
                        lines.push((line, Color32::from_gray(165), 11.0));
                    }
                }
                let health = (u.health / t.max_health).to_f32().clamp(0.0, 1.0);
                hover_card(view, &painter, mouse, lines, Some((health, format!("{:.0} / {:.0}", u.health.to_f32().max(0.0), t.max_health.to_f32()))), figures);
            }
        } else if let (None, Some(mouse), Some(f)) = (game.hovered, ctx.pointer_hover_pos().or_else(|| game.cursor_override.and_then(project)), game.hovered_feature()) {
            // A wreck, rock or tree: what reclaiming it would give (what is left of it), and
            // what a wreck could be raised as.
            if !ctx.is_pointer_over_egui() && matches!(game.command_mode, None | Some(crate::game::CommandMode::Reclaim | crate::game::CommandMode::Resurrect)) {
                let feature = &game.sim.features[f];
                let def = &game.sim.data.features[feature.def];
                let left = (1.0 - feature.reclaimed.to_f32()).clamp(0.0, 1.0);
                let name = if def.description.is_empty() { def.name.clone() } else { def.description.clone() };
                let mut lines = vec![(name, Color32::WHITE, 13.0)];
                let (metal, energy) = (format!("{:.0}", def.metal * left as f64), format!("{:.0}", def.energy * left as f64));
                let worth = match (def.metal > 0.0, def.energy > 0.0) {
                    (true, true) => Some(ta_ui::i18n::tf("Reclaim for {metal} metal and {energy} energy", &[("metal", &metal), ("energy", &energy)])),
                    (true, false) => Some(ta_ui::i18n::tf("Reclaim for {metal} metal", &[("metal", &metal)])),
                    (false, true) => Some(ta_ui::i18n::tf("Reclaim for {energy} energy", &[("energy", &energy)])),
                    (false, false) => None,
                };
                if let Some(worth) = worth {
                    lines.push((worth, METAL, 11.0));
                }
                if let Some(t) = game.sim.wreck_type(f) {
                    lines.push((ta_ui::i18n::tf("Can be resurrected as {unit}", &[("unit", &game.sim.types.get(t).def.display_name)]), Color32::from_rgb(150, 220, 150), 11.0));
                }
                hover_card(view, &painter, mouse, lines, None, Vec::new());
            }
        }
        // Placing an extractor: the deposits around, and what this site would yield.
        if let Some((type_id, pos)) = game.placement.as_ref().and_then(|pl| pl.pos.map(|p| (pl.type_id, p))) {
            let t = game.sim.types.get(type_id);
            if t.extracts_metal > Fx::ZERO {
                let world = &game.sim.world;
                let (cx, cz) = ((pos.x / 32.0) as i32, (pos.z / 32.0) as i32);
                // One dot per 2x2 cells of deposit, sized by how rich. Traces of metal that many
                // maps have everywhere are left out, and so is a metal world's plain ground (all
                // of it is metal; its deposits are what is richer).
                let plain = i64::from(world.surface_metal) * 4;
                for bz in cz - 20..=cz + 20 {
                    for bx in cx - 20..=cx + 20 {
                        let sum = world.metal_under((bx * 2, bz * 2), (2, 2));
                        if sum < 64 || sum <= plain {
                            continue;
                        }
                        let (x, z) = (bx as f32 * 32.0 + 16.0, bz as f32 * 32.0 + 16.0);
                        // Not over the ghost itself.
                        let (hx, hz) = (t.footprint.0 as f32 * 8.0, t.footprint.1 as f32 * 8.0);
                        if (x - pos.x).abs() < hx && (z - pos.z).abs() < hz {
                            continue;
                        }
                        if let Some(p) = project(Vec3::new(x, ground_at(x, z) + 1.0, z)) {
                            painter.circle(p, ((sum as f32).sqrt() * 0.35).clamp(2.5, 7.0), Color32::from_rgba_unmultiplied(150, 195, 255, 200), Stroke::new(1.0, Color32::from_black_alpha(160)));
                        }
                    }
                }
                let rate = game.sim.extraction_at(type_id, crate::game::fx_vec(pos)).to_f32();
                let off_ore = game.sim.placement_error(type_id, crate::game::fx_vec(pos)) == Some(ta_sim::world::PlacementError::NeedsMetal);
                if let Some(p) = project(pos + Vec3::Y * (t.model.height.to_f32() + 14.0)) {
                    let bad = Color32::from_rgb(240, 120, 100);
                    let (text, color) = if off_ore {
                        (ta_ui::i18n::t("Needs a metal deposit").to_string(), bad)
                    } else if rate > 0.0 {
                        (ghost_figures(&game.sim.type_flows(type_id, crate::game::fx_vec(pos))), Color32::from_rgb(205, 220, 240))
                    } else {
                        (ta_ui::i18n::t("No metal here").to_string(), bad)
                    };
                    let font = egui::FontId::proportional(15.0);
                    painter.text(p + EVec2::new(1.0, 1.0), egui::Align2::CENTER_BOTTOM, &text, font.clone(), Color32::from_black_alpha(200));
                    painter.text(p, egui::Align2::CENTER_BOTTOM, text, font, color);
                }
            } else {
                // Any other building: what it would make and use there.
                let flows = game.sim.type_flows(type_id, crate::game::fx_vec(pos));
                if flows.any() && let Some(p) = project(pos + Vec3::Y * (t.model.height.to_f32() + 14.0)) {
                    let (text, font) = (ghost_figures(&flows), egui::FontId::proportional(15.0));
                    painter.text(p + EVec2::new(1.0, 1.0), egui::Align2::CENTER_BOTTOM, &text, font.clone(), Color32::from_black_alpha(200));
                    painter.text(p, egui::Align2::CENTER_BOTTOM, text, font, Color32::from_rgb(205, 220, 240));
                }
            }
        }
        // Pending targeted order: label the cursor like TA's order cursors.
        if let (Some(mode), Some(mouse)) = (game.command_mode, ctx.pointer_hover_pos()) {
            let at = mouse + EVec2::new(14.0, 10.0);
            let galley = painter.layout_no_wrap(ta_ui::i18n::upper(ta_ui::i18n::t(mode.label_for(game))), egui::FontId::proportional(13.0), Color32::BLACK);
            let r = Rect::from_min_size(at, galley.size()).expand2(EVec2::new(5.0, 2.0));
            painter.rect_filled(r, 2.0, ACCENT);
            painter.galley(at, galley, Color32::BLACK);
        }
        // A circle on the ground, following the terrain: a side every few points round it however
        // large it is on the screen (sides by its size in the world showed as corners on a wide
        // one), its line edged dark to read on pale ground.
        let ring = |c: Vec3, r: f32, stroke: Stroke| {
            let across = [Vec3::X, Vec3::Z].iter().filter_map(|d| Some(project(c + *d * r)?.distance(project(c)?))).fold(0.0f32, f32::max);
            let n = (std::f32::consts::TAU * across / 4.0).clamp(48.0, 720.0) as usize;
            let pts: Vec<Pos2> = (0..=n)
                .filter_map(|k| {
                    let a = k as f32 / n as f32 * std::f32::consts::TAU;
                    let (x, z) = (c.x + a.cos() * r, c.z + a.sin() * r);
                    project(Vec3::new(x, ground_at(x, z) + 2.0, z))
                })
                .collect();
            painter.add(egui::Shape::line(pts.clone(), Stroke::new(stroke.width + 2.0, Color32::from_black_alpha((stroke.color.a() as f32 * 0.3) as u8))));
            painter.add(egui::Shape::line(pts, stroke));
        };
        // An area order being dragged.
        if let (Some((c, r)), Some(mode)) = (game.area, game.command_mode) {
            let color = match mode {
                crate::game::CommandMode::Attack => Color32::from_rgb(255, 90, 70),
                crate::game::CommandMode::Capture => Color32::from_rgb(200, 130, 255),
                crate::game::CommandMode::Resurrect => Color32::from_rgb(120, 200, 255),
                _ => Color32::from_rgb(230, 200, 90),
            };
            ring(c, r, Stroke::new(2.0, color));
            if let Some(p) = project(c + Vec3::Y * 2.0) {
                painter.circle_filled(p, 3.0, color);
            }
        }
        // Ranges of the building being placed, of the selected and of what the pointer rests on:
        // the weapons of buildings (red; nuclear missiles reach too far to show) and what the
        // sensors reach (`ranges`: radar, sonar and jammers, with the unit's own sight to set them
        // against). Of the selected, those of buildings, and of units that only sense (a radar
        // vehicle, a jammer); the Commander's and the fighters' show when the pointer rests on one.
        let ranged = ranges::collect(game, alpha, (rest >= HOVER_CARD_AFTER).then_some(game.hovered).flatten());
        let still = ta_ui::motion::still(ctx);
        let canvas = ranges::Canvas { painter: &painter, project: &project, ground: &ground_at, free: free.intersect(view), clock, still, placed: std::cell::RefCell::new(self.minimap_rect.filter(|_| !self.classic).map(|m| vec![m.expand(6.0)]).unwrap_or_default()) };
        for r in ranged {
            let t = game.sim.types.get(r.type_id);
            let weapon = t.weapons.iter().flatten().map(|&w| game.sim.weapon_types.get(w)).filter(|w| !w.def.stockpile && !w.def.interceptor).map(|w| w.range.to_f32()).fold(0.0, f32::max);
            if r.weapons && weapon > 0.0 {
                ring(r.at, weapon, Stroke::new(1.5, Color32::from_rgba_unmultiplied(255, 90, 70, 170)));
            }
            for range in ranges::display_ranges(t, &game.sim.weapon_types) {
                ranges::draw(&canvas, r.at, range, r.power);
                if !still && r.power == ranges::Power::On {
                    ctx.request_repaint();
                }
            }
        }
        self.duty_tags(ctx, game, camera, &painter);
        self.idle_alerts(ctx, vfs, game, camera, view, &painter);
        if let Some((lo, hi)) = game.drag_rect {
            let r = Rect::from_min_max(Pos2::new(lo.x / ppp, lo.y / ppp), Pos2::new(hi.x / ppp, hi.y / ppp));
            painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(120, 255, 120, 20));
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_rgb(120, 255, 120)), egui::StrokeKind::Inside);
        }
    }

    /// A small tag over each constructor on duty that is in view: a cog and the name of its duty, in
    /// the colour of information, a little over its health and build bars (where the idle lamp
    /// stands, which they never share: a unit on duty is not idle). One with nothing to do wears
    /// "IDLE" and a duller tag; those of one duty (and state) close together share one tag with their
    /// number, and tags of different duties that meet are stacked.
    fn duty_tags(&self, ctx: &egui::Context, game: &Game, camera: &Camera, painter: &egui::Painter) {
        use crate::theme::{SKY, STEEL, strong};
        let (ppp, alpha, free) = (ctx.pixels_per_point(), game.alpha(), painter.clip_rect());
        struct Tag {
            at: Pos2,
            duty: ta_ai::Duty,
            idle: bool,
            count: usize,
        }
        let mut tags: Vec<Tag> = Vec::new();
        for (id, duty) in game.units_on_duty().take(60) {
            let Some(u) = game.sim.unit(id) else { continue };
            let Some(pos) = game.scene.unit_position(id, alpha) else { continue };
            let clip = camera.view_proj() * (pos + Vec3::Y * (game.sim.unit_type(u).model.height.to_f32() + 6.0)).extend(1.0);
            if clip.w <= 0.0 {
                continue;
            }
            let ndc = clip.xy() / clip.w;
            let at = Pos2::new((ndc.x + 1.0) * 0.5 * camera.viewport.x / ppp, (1.0 - ndc.y) * 0.5 * camera.viewport.y / ppp) - EVec2::new(0.0, 18.0);
            if !free.contains(at) {
                continue;
            }
            let idle = matches!(game.duty_status(id), Some(ta_ai::DutyStatus::Idle(_)));
            match tags.iter_mut().find(|t| t.duty == duty && t.idle == idle && (t.at.x - at.x).abs() < 44.0 && (t.at.y - at.y).abs() < 12.0) {
                Some(t) => t.count += 1,
                None => tags.push(Tag { at, duty, idle, count: 1 }),
            }
        }
        let mut placed: Vec<Rect> = Vec::new();
        for tag in tags {
            let color = if tag.idle { STEEL } else { SKY };
            let mut text = ta_ui::i18n::t(crate::game::duty::label(tag.duty)).to_string();
            if tag.count > 1 {
                text = format!("{text} \u{d7}{}", tag.count);
            }
            if tag.idle {
                text = format!("{text} \u{b7} {}", ta_ui::i18n::t("IDLE"));
            }
            let galley = painter.layout_no_wrap(text, strong(9.5), color);
            let size = galley.size();
            let mut rect = Rect::from_center_size(tag.at, EVec2::new(size.x + 18.0, size.y + 3.0));
            // (Tags that would lie on one another are stacked, upwards.)
            for _ in 0..8 {
                match placed.iter().find(|r| r.intersects(rect)) {
                    Some(r) => rect = rect.translate(EVec2::new(0.0, r.top() - rect.bottom() - 1.0)),
                    None => break,
                }
            }
            placed.push(rect);
            painter.rect_filled(rect, 2.5, Color32::from_black_alpha(155));
            crate::hud_icons::paint(painter, Rect::from_center_size(Pos2::new(rect.left() + 9.0, rect.center().y), EVec2::splat(11.0)), crate::hud_icons::Glyph::Duty, color);
            painter.galley(Pos2::new(rect.left() + 15.0, rect.center().y - size.y / 2.0), galley, color);
        }
    }

    /// Idle constructors and factories, a few seconds after they run out of work: a lamp over
    /// those in view, and for the others an arrow at the edge of the view pointing to where
    /// they stand (a click selects the unit and looks at it). They fade after a while; the top
    /// bar keeps counting them.
    fn idle_alerts(&mut self, ctx: &egui::Context, vfs: &Vfs, game: &Game, camera: &Camera, view: Rect, painter: &egui::Painter) {
        let now = ctx.input(|i| i.time) as f32;
        let due = self.idle_alerts.update(game.idle_builders().into_iter().chain(game.idle_factories()), now);
        if due[0].is_none() {
            return;
        }
        ctx.request_repaint();
        let ppp = ctx.pixels_per_point();
        let alpha = game.alpha();
        // (Inside the part of the screen the panels leave free.)
        let free = self.free_view(ctx).intersect(view);
        let inset = free.shrink2(EVec2::new(34.0, 30.0));
        let mut arrows = 0;
        for (id, age) in due.into_iter().flatten() {
            let Some(u) = game.sim.unit(id) else { continue };
            let t = game.sim.unit_type(u);
            let Some(pos) = game.scene.unit_position(id, alpha) else { continue };
            // Fading in, and out towards the end.
            let fade = ((age - IDLE_ALERT) / 0.4).min((IDLE_ALERT_END - age) / 3.0).clamp(0.0, 1.0);
            let pulse = 0.6 + 0.4 * (now * 4.0).sin().abs();
            // (Where its health bar is anchored: the lamp goes a fixed height over the bars.)
            let clip = camera.view_proj() * (pos + Vec3::Y * (t.model.height.to_f32() + 6.0)).extend(1.0);
            let mut ndc = clip.xy() / clip.w;
            if clip.w < 0.0 {
                ndc = -ndc;
            }
            let s = Pos2::new((ndc.x + 1.0) * 0.5 * camera.viewport.x / ppp, (1.0 - ndc.y) * 0.5 * camera.viewport.y / ppp);
            if clip.w > 0.0 && inset.contains(s) {
                // In view: a lamp and a word over it, clear of its health and build bars, the
                // whole lamp fading in and out (its dark glass alone read as a black ball on the
                // bar).
                let s = s - EVec2::new(0.0, 18.0);
                let color = crate::theme::AMBER.gamma_multiply(fade);
                painter.add(ta_ui::metal::faded_lamp(s, 3.5, crate::theme::AMBER, fade * pulse, fade));
                let text = ta_ui::i18n::t("IDLE");
                let galley = painter.layout_no_wrap(text.to_string(), crate::theme::strong(10.0), color);
                let at = s + EVec2::new(7.0, -galley.size().y / 2.0);
                painter.rect_filled(Rect::from_min_size(at - EVec2::new(3.0, 1.0), galley.size() + EVec2::new(6.0, 2.0)), 2.0, Color32::from_black_alpha((150.0 * fade) as u8));
                painter.galley(at, galley, color);
                continue;
            }
            if arrows >= 6 {
                continue;
            }
            arrows += 1;
            // Out of view: where the line from the view's middle to it crosses the inset edge;
            // short of the messages under the top bar if it would land on them (an arrow there
            // covered their words).
            let c = inset.center();
            let d = s - c;
            let mut k = (inset.width() / 2.0 / d.x.abs().max(1e-3)).min(inset.height() / 2.0 / d.y.abs().max(1e-3));
            if let Some(column) = self.messages_rect.map(|r| r.expand(26.0))
                && column.contains(c + d * k)
                && d.y < 0.0
            {
                k = k.min((column.bottom() - c.y) / d.y).max(0.0);
            }
            let mut at = c + d * k;
            // (Nor on the minimap's plate at the top right: along the top edge, left of it, or
            // down the right edge, below it.)
            if !self.classic && let Some(plate) = self.minimap_rect.map(|m| m.expand(26.0)) && plate.contains(at) {
                if (at.y - inset.top()).abs() < 2.0 {
                    at.x = plate.left() - 6.0;
                } else {
                    at.y = plate.bottom() + 6.0;
                }
            }
            let dir = d.normalized();
            let area = egui::Area::new(egui::Id::new(("idle_alert", id.slot))).fixed_pos(at - EVec2::splat(17.0)).order(egui::Order::Middle).show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(EVec2::splat(34.0), egui::Sense::click());
                let p = ui.painter();
                let hot = resp.hovered();
                let centre = rect.center();
                // The pointer outside the disc, towards the unit.
                let tip = centre + dir * 24.0;
                let side = EVec2::new(-dir.y, dir.x);
                p.add(egui::Shape::convex_polygon(vec![tip, centre + dir * 15.0 + side * 6.0, centre + dir * 15.0 - side * 6.0], crate::theme::AMBER_BRIGHT.gamma_multiply(fade * pulse), Stroke::NONE));
                p.circle_filled(centre, 16.0, Color32::from_black_alpha((200.0 * fade) as u8));
                if let Some(pic) = self.unit_pic(ctx, vfs, game, u.type_id, 24.0) {
                    let r = Rect::from_center_size(centre, EVec2::splat(24.0));
                    pic.paint(p, r, Color32::WHITE.gamma_multiply(fade));
                }
                p.circle_stroke(centre, 16.0, Stroke::new(if hot { 2.5 } else { 1.8 }, crate::theme::AMBER_BRIGHT.gamma_multiply(fade * if hot { 1.0 } else { pulse })));
                resp
            });
            let name = crate::chat::unit_label(&game.sim, u.type_id);
            let resp = area.inner.on_hover_text(ta_ui::i18n::tf("Idle: {name}. Click to select it and look at it", &[("name", &name)])).on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.clicked() {
                self.look_at = Some(id);
            }
        }
    }

    /// The control groups along the foot of the battlefield: each with its number, its
    /// commonest unit's picture, its size and its health. A click selects it (Shift adds it; a
    /// second click, or a double-click, looks at it); Ctrl+click sets it to the selection.
    fn group_bar(&mut self, ctx: &egui::Context, vfs: &Vfs, game: &mut Game, clear: Clearance) {
        use ta_ui::i18n::tf;
        if game.spectator || game.playback.as_ref().is_some_and(|p| !p.resume) {
            return;
        }
        let groups: Vec<(usize, Vec<UnitId>)> = (1..=10).map(|k| k % 10).map(|g| (g, game.group_units(g))).filter(|(_, units)| !units.is_empty()).collect();
        if groups.is_empty() {
            return;
        }
        let selection: rustc_hash::FxHashSet<UnitId> = game.selection.iter().copied().collect();
        let mut clicked: Option<(usize, bool, bool)> = None;
        egui::Area::new(egui::Id::new("group_bar")).anchor(egui::Align2::CENTER_BOTTOM, [clear.left / 2.0, -clear.bottom - 8.0]).order(egui::Order::Middle).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (g, units) in &groups {
                    // Its commonest type, and its health together.
                    let mut kinds: Vec<(usize, usize)> = Vec::new();
                    let (mut health, mut max) = (0.0f32, 0.0f32);
                    for &id in units {
                        let Some(u) = game.sim.unit(id) else { continue };
                        match kinds.iter_mut().find(|(k, _)| *k == u.type_id) {
                            Some((_, n)) => *n += 1,
                            None => kinds.push((u.type_id, 1)),
                        }
                        health += u.health.to_f32();
                        max += game.sim.unit_type(u).max_health.to_f32();
                    }
                    let picture = kinds.iter().max_by_key(|&&(k, n)| (n, std::cmp::Reverse(k))).map(|&(k, _)| k);
                    let chosen = units.len() == selection.len() && units.iter().all(|id| selection.contains(id));
                    let (rect, resp) = ui.allocate_exact_size(EVec2::new(64.0, 44.0), egui::Sense::click());
                    let hot = if resp.hovered() { 1.0 } else { 0.0 };
                    let p = ui.painter();
                    p.add(ta_ui::metal::slab(ctx, rect, ta_ui::metal::Slab::Slot, hot, resp.is_pointer_button_down_on()));
                    if let Some(pic) = picture.and_then(|k| self.unit_pic(ctx, vfs, game, k, 30.0)) {
                        let r = Rect::from_min_size(rect.min + EVec2::new(5.0, 5.0), EVec2::splat(30.0));
                        pic.paint(p, r, Color32::WHITE);
                        p.add(pic_frame(r, false));
                    }
                    let digit = if chosen { crate::theme::AMBER_BRIGHT } else { crate::theme::CREAM };
                    p.text(Pos2::new(rect.right() - 6.0, rect.top() + 3.0), egui::Align2::RIGHT_TOP, g.to_string(), crate::theme::display(17.0), digit);
                    p.text(Pos2::new(rect.right() - 6.0, rect.bottom() - 7.0), egui::Align2::RIGHT_BOTTOM, units.len().to_string(), crate::theme::strong(11.5), crate::theme::TEXT_BRIGHT);
                    let fraction = if max > 0.0 { (health / max).clamp(0.0, 1.0) } else { 1.0 };
                    let bar = Rect::from_min_max(Pos2::new(rect.left() + 5.0, rect.bottom() - 5.0), Pos2::new(rect.right() - 5.0, rect.bottom() - 3.0));
                    p.rect_filled(bar, CornerRadius::ZERO, Color32::from_black_alpha(160));
                    p.rect_filled(Rect::from_min_size(bar.min, EVec2::new(bar.width() * fraction, bar.height())), CornerRadius::ZERO, hud_skin::health_color(fraction));
                    if chosen {
                        p.rect_stroke(rect.shrink(1.0), CornerRadius::same(2), Stroke::new(1.5, crate::theme::AMBER_BRIGHT), egui::StrokeKind::Inside);
                    }
                    let tip = tf("Group {g}: {n} units. Click to select it (Shift adds it), double-click to look at it; Ctrl+click makes it the selection", &[("g", g), ("n", &units.len())]);
                    let resp = resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand);
                    if resp.clicked() {
                        let m = ta_ui::widgets::click_modifiers(&resp, egui::PointerButton::Primary);
                        clicked = Some((*g, m.shift, m.ctrl || m.command));
                        ta_ui::sound::play(ctx, ta_ui::sound::UiSound::Button);
                    }
                }
            });
        });
        match clicked {
            Some((g, _, true)) => game.assign_group(g),
            Some((g, add, false)) => game.recall_group(g, add, false),
            None => {}
        }
    }

    /// Multiplayer chat: recent lines at the bottom left, and the input line while typing
    /// (Enter sends, Escape cancels).
    fn chat(&mut self, ctx: &egui::Context, vfs: &Vfs, game: &mut Game, clear: Clearance) {
        use ta_ui::i18n::{t, tf};
        // (While a line is being typed, the chat so far is on its panel, over it.)
        let typing_open = game.chat_input.is_some();
        let above = 52.0;
        // Long lines wrap (an Area wider than the screen would be pushed off its left edge).
        let width = (ctx.content_rect().width() - clear.left - 48.0).clamp(240.0, 640.0);
        // Who is typing (the computer players think before they answer).
        let now = game.game_secs();
        let typing: Vec<(String, u8)> = game.chatter.as_ref().map(|c| c.typing(now)).unwrap_or_default().into_iter().map(|t| (t.name, t.player)).collect();
        if !typing.is_empty() {
            ctx.request_repaint();
        }
        // The pictures of those on the chat.
        let mut speakers: Vec<u8> = game.chat_log.iter().chain(game.chat_history.iter().rev().take(40)).filter_map(|l| l.player).chain(typing.iter().map(|t| t.1)).collect();
        speakers.sort_unstable();
        speakers.dedup();
        let pictures: Vec<(u8, Option<UnitPic>)> = speakers.into_iter().map(|p| (p, self.portrait(ctx, vfs, game, p, 20.0))).collect();
        let picture = |p: Option<u8>| p.and_then(|p| pictures.iter().find(|(q, _)| *q == p)).and_then(|(_, pic)| *pic);
        // A speaker's colour, lit enough to read on the dark plates (a deep blue would not be).
        let readable = |c: Color32| c.lerp_to_gamma(Color32::WHITE, 0.38);
        // A line: on a plate of its own (`plated`; in the history they share one), with a bar
        // of the speaker's colour down its left edge and their picture.
        let line_ui = |ui: &mut egui::Ui, line: &crate::chat::ChatLine, fade: f32, plated: bool| {
            let color = line.player.and_then(|p| game.sim.players.get(p as usize)).map_or(if line.from.is_empty() { crate::theme::GREEN } else { ACCENT }, |p| crate::palette::team_color(p.color));
            let frame = if plated { egui::Frame::new().fill(Color32::from_black_alpha((165.0 * fade) as u8)).corner_radius(2) } else { egui::Frame::new() };
            let shown = frame.inner_margin(egui::Margin { left: 10, right: 8, top: 3, bottom: 3 }).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if line.from.is_empty() {
                        // The game's own answers (cheat codes).
                        ui.label(egui::RichText::new(&line.text).color(crate::theme::GREEN.gamma_multiply(fade)).italics());
                    } else {
                        // Speakers with their pictures, in their colours.
                        if let Some(pic) = picture(line.player) {
                            let (rect, _) = ui.allocate_exact_size(EVec2::splat(20.0), egui::Sense::hover());
                            pic.paint(ui.painter(), rect, Color32::WHITE.gamma_multiply(fade));
                            ui.painter().rect_stroke(rect, 1.0, Stroke::new(1.0, Color32::from_black_alpha((200.0 * fade) as u8)), egui::StrokeKind::Outside);
                        }
                        ui.label(egui::RichText::new(&line.from).font(crate::theme::strong(13.0)).color(readable(color).gamma_multiply(fade)));
                        ui.label(egui::RichText::new(&line.text).font(crate::theme::body(13.5)).color(crate::theme::TEXT_BRIGHT.gamma_multiply(fade)));
                    }
                });
            });
            let r = shown.response.rect;
            ui.painter().rect_filled(Rect::from_min_size(r.min + EVec2::new(0.0, 2.0), EVec2::new(3.0, r.height() - 4.0)), 1.0, color.gamma_multiply(0.9 * fade));
        };
        // Someone typing (the computer players think before they answer).
        let typing_ui = |ui: &mut egui::Ui, name: &str, player: u8| {
            let color = game.sim.players.get(player as usize).map_or(ACCENT, |p| crate::palette::team_color(p.color));
            let dots = ".".repeat(1 + (ctx.input(|i| i.time) * 3.0) as usize % 3);
            let shown = egui::Frame::new().fill(Color32::from_black_alpha(140)).inner_margin(egui::Margin { left: 10, right: 8, top: 3, bottom: 3 }).corner_radius(2).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if let Some(pic) = picture(Some(player)) {
                        let (rect, _) = ui.allocate_exact_size(EVec2::splat(20.0), egui::Sense::hover());
                        pic.paint(ui.painter(), rect, Color32::from_gray(200));
                        ui.painter().rect_stroke(rect, 1.0, Stroke::new(1.0, Color32::from_black_alpha(200)), egui::StrokeKind::Outside);
                    }
                    ui.label(egui::RichText::new(format!("{}{dots}", tf("{name} is typing", &[("name", &name)]))).font(crate::theme::body(13.0)).color(crate::theme::STEEL).italics());
                });
            });
            let r = shown.response.rect;
            ui.painter().rect_filled(Rect::from_min_size(r.min + EVec2::new(0.0, 2.0), EVec2::new(3.0, r.height() - 4.0)), 1.0, color.gamma_multiply(0.5));
        };
        if !typing_open {
            egui::Area::new(egui::Id::new("chat_log")).anchor(egui::Align2::LEFT_BOTTOM, [clear.left + 16.0, -clear.bottom - above]).interactable(false).show(ctx, |ui| {
                ui.set_max_width(width);
                ui.spacing_mut().item_spacing.y = 3.0;
                // (The computer players' lines come in on their transmission card.)
                for line in game.chat_log.iter().filter(|l| l.title.is_none()) {
                    line_ui(ui, line, line.secs.min(1.0), true);
                }
            });
        }
        let spectator = game.spectator;
        let multiplayer = game.net.is_some();
        // (The key that opened the line this frame must not also send it.)
        let fresh = std::mem::take(&mut game.chat_fresh);
        let Game { chat_input, chat_allies: allies, chat_history, .. } = game;
        let Some(line) = chat_input.as_mut() else { return };
        let mut send = false;
        let mut close = false;
        // One panel as wide as the chat's lines over the battle: the chat so far in a well (to
        // scroll back through), who is typing, and the line being typed across the foot. It
        // comes up as a dialog does (at once with reduced motion).
        let (shown, _) = ta_ui::panel::entrance(ctx, egui::Id::new("chat_input"), ta_ui::motion::Pace::FULL.dialog_in);
        ta_ui::panel::strip_shown(ctx, "chat_input", egui::Align2::LEFT_BOTTOM, EVec2::new(clear.left + 16.0, -clear.bottom - 14.0), shown, |ui| {
            let inner = width - ta_ui::panel::strip_margin().sum().x;
            ui.set_width(inner);
            if !chat_history.is_empty() {
                egui::Frame::new().fill(crate::theme::INSET).corner_radius(2).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(260.0).stick_to_bottom(true).show(ui, |ui| {
                        ui.set_width(inner - 12.0);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for line in chat_history.iter() {
                            line_ui(ui, line, 1.0, false);
                        }
                    });
                });
                ui.add_space(4.0);
            }
            for (name, player) in &typing {
                typing_ui(ui, name, *player);
            }
            {
                ui.horizontal(|ui| {
                    if multiplayer {
                        // Who hears it: click to switch (Shift+Enter opens the chat to allies).
                        let to = t(if *allies { "To allies:" } else { "To all:" });
                        if ui.add(egui::Label::new(egui::RichText::new(to).color(if *allies { Color32::from_rgb(120, 220, 140) } else { ACCENT })).sense(egui::Sense::click())).on_hover_text(t("Click to switch between everyone and your allies")).clicked() && !spectator {
                            *allies = !*allies;
                        }
                    } else {
                        ui.label(egui::RichText::new(t("Message:")).color(ACCENT));
                    }
                    let hint = if multiplayer { t("A line starting with + is a command (+help)") } else { t("A line starting with + is a command or cheat code (+help)") };
                    let resp = ui.add(egui::TextEdit::singleline(line).desired_width(ui.available_width()).char_limit(200).hint_text(hint));
                    resp.request_focus();
                    if !fresh && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        send = true;
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        close = true;
                    }
                });
            }
        });
        if send {
            let text = game.chat_input.take().unwrap_or_default();
            if !text.trim().is_empty() {
                let allies = game.chat_allies;
                game.say(&text, allies);
            }
        } else if close {
            game.chat_input = None;
        }
    }

    /// Our allies at a glance (the modern interface; not watching): a card each under the top
    /// bar's right end, in their colour, with their army and their Commander, an alarm while
    /// they are under attack; a click looks where they were attacked, or at their Commander.
    fn ally_strip(&mut self, ctx: &egui::Context, game: &mut Game, clear: Clearance) {
        use ta_ui::i18n::{t, tf};
        if game.spectator || game.playback.is_some() || game.outcome.is_some() || game.show_players {
            return;
        }
        let allies = game.allies();
        if allies.is_empty() {
            return;
        }
        // What each has: the worth of its army, and its Commander's health and place.
        let n = game.sim.players.len();
        let mut army = vec![0.0f32; n];
        let mut commander: Vec<Option<(f32, Vec3)>> = vec![None; n];
        for u in game.sim.units() {
            if u.dead || !u.is_complete() || !allies.contains(&u.owner) {
                continue;
            }
            let ty = game.sim.unit_type(u);
            if ty.def.is_commander {
                commander[u.owner as usize] = Some(((u.health / ty.max_health).to_f32().clamp(0.0, 1.0), Vec3::from_array(u.pos.to_f32())));
            } else if ty.locomotion != ta_sim::Locomotion::Static && ty.weapons.iter().any(|w| w.is_some()) {
                army[u.owner as usize] += ty.cost_metal.to_f32() + ty.cost_energy.to_f32() / 60.0;
            }
        }
        let now = game.sim.tick as f32 / 30.0;
        let clock = ctx.input(|i| i.time) as f32;
        let mut look = None;
        let mut alarm = false;
        egui::Area::new(egui::Id::new("ally_strip")).anchor(egui::Align2::RIGHT_TOP, [-12.0, clear.top + clear.right_top + 10.0]).interactable(true).show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            for &a in &allies {
                let colour = crate::palette::team_color(game.sim.players[a as usize].color);
                let attacked = game.attacks.iter().rev().find(|(_, t, who)| *who == Some(a) && now - *t < 8.0).map(|(at, ..)| *at);
                alarm |= attacked.is_some();
                let pulse = if attacked.is_some() { 0.5 + 0.5 * (clock * 6.0).sin() } else { 0.0 };
                let edge = if attacked.is_some() { Color32::from_rgb(255, 70, 50).gamma_multiply(0.4 + 0.6 * pulse) } else { Color32::from_white_alpha(30) };
                let frame = egui::Frame::new().fill(Color32::from_black_alpha(170)).stroke(Stroke::new(if attacked.is_some() { 1.5 } else { 1.0 }, edge)).corner_radius(CornerRadius::same(2)).inner_margin(egui::Margin { left: 10, right: 10, top: 5, bottom: 5 });
                let card = frame.show(ui, |ui| {
                    ui.set_width(196.0);
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.horizontal(|ui| {
                        let (chip, _) = ui.allocate_exact_size(EVec2::new(10.0, 10.0), egui::Sense::hover());
                        ui.painter().rect_filled(chip, 2.0, colour);
                        ui.add(egui::Label::new(egui::RichText::new(game.player_label(a)).font(crate::theme::strong(13.0)).color(Color32::WHITE)).truncate());
                        if attacked.is_some() {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ta_ui::widgets::tag(ui, "UNDER ATTACK", crate::theme::RED);
                            });
                        }
                    });
                    let worth = army[a as usize];
                    let worth = if worth >= 1000.0 { format!("{:.1}k", worth / 1000.0) } else { format!("{worth:.0}") };
                    let line = match commander[a as usize] {
                        Some((health, _)) => tf("ARMY {army} · COMMANDER {health}%", &[("army", &worth), ("health", &((health * 100.0).round() as i32))]),
                        None => tf("ARMY {army}", &[("army", &worth)]),
                    };
                    ui.label(egui::RichText::new(line).font(crate::theme::strong(10.5)).color(crate::theme::STEEL).extra_letter_spacing(ta_ui::i18n::tracking(0.8)));
                    if let Some((health, _)) = commander[a as usize] {
                        let (bar, _) = ui.allocate_exact_size(EVec2::new(ui.available_width(), 3.0), egui::Sense::hover());
                        ui.painter().rect_filled(bar, 1.0, Color32::from_white_alpha(25));
                        let tone = if health > 0.5 { crate::theme::GREEN } else if health > 0.25 { crate::theme::AMBER } else { crate::theme::RED };
                        ui.painter().rect_filled(egui::Rect::from_min_size(bar.min, EVec2::new(bar.width() * health, bar.height())), 1.0, tone);
                    }
                });
                let response = card.response.interact(egui::Sense::click()).on_hover_text(if attacked.is_some() { t("Look where they are under attack") } else { t("Look at their Commander") });
                if response.clicked() {
                    look = attacked.or(commander[a as usize].map(|c| c.1));
                }
            }
        });
        if alarm {
            ctx.request_repaint();
        }
        if let Some(at) = look {
            game.focus_request = Some(at);
        }
    }

    /// Multiplayer banners under the messages: who paused the game (RESUME for anyone), and,
    /// after a long wait for a slow player, the host's choice to go on without them (the others
    /// told it is theirs).
    fn waiting_banner(&mut self, ctx: &egui::Context, game: &mut Game, clear: Clearance) {
        use ta_ui::i18n::{t, tf};
        if game.net.is_none() && game.paused && game.outcome.is_none() && !game.menu_open && game.playback.is_none() {
            let key = game.keys.text("pause");
            // (In the classic interface, under TA's own PAUSED title.)
            let top = if self.classic { crate::hud_classic::paused_title_bottom(ctx.content_rect()) + 6.0 } else { clear.top + 76.0 };
            ta_ui::panel::strip(ctx, "paused", egui::Align2::CENTER_TOP, EVec2::new(clear.center, top), |ui| {
                ui.label(egui::RichText::new(tf("PAUSED · {key} TO RESUME", &[("key", &key.to_uppercase())])).font(crate::theme::strong(15.0)).color(ACCENT).extra_letter_spacing(ta_ui::i18n::tracking(1.4)));
            });
            return;
        }
        if game.net.is_none() || game.outcome.is_some() {
            return;
        }
        let mut top = clear.top + 76.0;
        if game.paused {
            let by = game.net.as_ref().and_then(|n| n.paused_by.clone());
            // (The host takes the pace from its players, not from those who watch.)
            let may_resume = !game.spectator || game.net.as_ref().is_some_and(|n| n.is_host());
            let mut resume = false;
            let shown = ta_ui::panel::strip(ctx, "net_paused", egui::Align2::CENTER_TOP, EVec2::new(clear.center, top), |ui| {
                ui.horizontal(|ui| {
                    let title = by.as_ref().map_or_else(|| t("PAUSED").to_string(), |name| tf("PAUSED BY {name}", &[("name", &name.to_uppercase())]));
                    ui.label(egui::RichText::new(title).font(crate::theme::strong(15.0)).color(ACCENT).extra_letter_spacing(ta_ui::i18n::tracking(1.4)));
                    if may_resume {
                        ui.add_space(8.0);
                        resume = ui.add(ta_ui::widgets::ActionButton::secondary("RESUME")).on_hover_text(t("Resume the game for everyone")).clicked();
                    }
                });
                ui.min_rect()
            });
            top += shown.height() + 40.0;
            if resume {
                game.set_pace(Some(false), None);
            }
        }
        let Some(net) = game.net.as_mut() else { return };
        if net.waiting_for.is_empty() || game.net_wait < 20.0 {
            return;
        }
        let players = net.waiting_for.join(", ");
        let host = net.is_host();
        let mut drop = false;
        ta_ui::panel::strip(ctx, "waiting_drop", egui::Align2::CENTER_TOP, EVec2::new(clear.center, top), |ui| {
            ui.horizontal(|ui| {
                if host {
                    ui.label(egui::RichText::new(tf("{players} still behind.", &[("players", &players)])).color(crate::theme::TEXT));
                    drop = ui.add(ta_ui::widgets::ActionButton::secondary("GO ON WITHOUT THEM")).on_hover_text(t("Close their connection; their units stay on the map, idle")).clicked();
                } else {
                    ui.label(egui::RichText::new(tf("{players} still behind: the host can go on without them.", &[("players", &players)])).color(crate::theme::TEXT));
                }
            });
        });
        if drop {
            net.drop_waiting();
        }
    }

    fn messages(&mut self, ctx: &egui::Context, game: &Game, clear: Clearance) {
        // Multiplayer trouble stays up for as long as it lasts.
        let mut status = Vec::new();
        if let Some(net) = &game.net {
            if let Some(tick) = net.desync {
                let since = format!("{:02}:{:02}", tick / 30 / 60, tick / 30 % 60);
                status.push((ta_ui::i18n::tf("OUT OF SYNC since {time}: the players' games no longer match", &[("time", &since)]), Color32::from_rgb(255, 90, 70)));
            }
            if !net.waiting_for.is_empty() {
                let secs = game.net_wait as u32;
                let time = format!("{}:{:02}", secs / 60, secs % 60);
                status.push((ta_ui::i18n::tf("Waiting for {players} · {time}", &[("players", &net.waiting_for.join(", ")), ("time", &time)]), ACCENT));
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
        }
        let legend = game.shown_overlay();
        self.messages_rect = None;
        if game.messages.is_empty() && status.is_empty() && legend.is_none() {
            return;
        }
        // One column under the top bar (the map overlay's legend, multiplayer trouble, then the
        // messages), centred on the free battlefield, right of the goals or lesson panel when
        // one is up in its corner (as laid out last frame), so nothing covers anything.
        let free = self.free_view(ctx);
        let left = ["scenario_goals", "conquest_goals", "tutorial", "transmission"]
            .iter()
            .filter_map(|id| ctx.memory(|m| m.area_rect(egui::Id::new(*id))))
            .filter(|r| r.min.x < free.center().x && r.max.y > free.top())
            .map(|r| r.max.x + 12.0)
            .fold(free.left(), f32::max)
            .min(free.right() - 160.0);
        let centre = (left + free.right()) / 2.0 - ctx.content_rect().center().x;
        // Measure every notice before laying out the column, so a short first notice does
        // not force the next one to wrap to its width (nor does a previous frame pin it there).
        let painter = ctx.layer_painter(egui::LayerId::background());
        // (One that points somewhere has the LOOK chip at its right: room for it.)
        let chip = crate::hint::chip_size(&painter);
        let measured = game.messages.iter().map(|m| painter.layout_no_wrap(m.text.clone(), crate::theme::strong(14.0), Color32::WHITE).size().x + 30.0 + if m.link.is_some() { chip.x + 8.0 } else { 0.0 })
            .chain(status.iter().map(|(m, _)| painter.layout_no_wrap(m.clone(), crate::theme::strong(15.0), Color32::WHITE).size().x + 32.0))
            .fold(120.0, f32::max);
        let legend_width = legend.map_or(0.0, |o| {
            let label = painter.layout_no_wrap(ta_ui::i18n::t(o.label()).to_string(), crate::theme::strong(12.0), Color32::WHITE);
            let text = painter.layout_no_wrap(ta_ui::i18n::t(o.legend()).to_string(), crate::theme::body(12.5), Color32::WHITE);
            label.size().x + text.size().x + 42.0
        });
        let width = measured.max(legend_width).min((free.right() - left - 24.0).max(120.0));
        // (Under the cast's scoreboard, as laid out last frame.)
        let below = if game.cast { ctx.memory(|m| m.area_rect(egui::Id::new("cast_scoreboard"))).map_or(0.0, |r| r.bottom() - ctx.content_rect().top() - clear.top) } else { 0.0 };
        // (The notices that point somewhere, as they were laid out: for their buttons, below.)
        let mut pointing: Vec<(usize, Rect, crate::game::Link, f32)> = Vec::new();
        let column = egui::Area::new(egui::Id::new("messages")).anchor(egui::Align2::CENTER_TOP, [centre, clear.top + 10.0 + below]).interactable(false).show(ctx, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.vertical_centered(|ui| {
                if let Some(o) = legend {
                    egui::Frame::new().fill(Color32::from_black_alpha(180)).stroke(Stroke::new(1.0, ta_ui::metal::STEEL_SHADE)).inner_margin(egui::Margin::symmetric(12, 5)).corner_radius(2).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add(egui::Label::new(ta_ui::widgets::caption(o.label()).color(ACCENT)).extend());
                            ui.add(egui::Label::new(egui::RichText::new(ta_ui::i18n::t(o.legend())).font(crate::theme::body(12.5)).color(crate::theme::TEXT)).wrap());
                        });
                    });
                }
                for (m, c) in &status {
                    egui::Frame::new().fill(Color32::from_black_alpha(170)).stroke(Stroke::new(1.0, c.gamma_multiply(0.5))).inner_margin(egui::Margin::symmetric(14, 5)).corner_radius(2).show(ui, |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(m).color(*c).font(crate::theme::strong(15.0))).wrap());
                    });
                }
                // Long notices wrap within the available battlefield, including at larger UI
                // scales. Framed as the legend is, with their tone's colour along the left edge:
                // amber to know, green for what was done, red for what went wrong.
                for (i, notice) in game.messages.iter().enumerate() {
                    use crate::game::Tone;
                    let fade = notice.secs.min(1.0);
                    let (words, edge, rim) = match notice.tone {
                        Tone::Note => (crate::theme::AMBER_BRIGHT, crate::theme::AMBER, ta_ui::metal::STEEL_SHADE),
                        Tone::Done => (crate::theme::GREEN, crate::theme::GREEN, ta_ui::metal::STEEL_SHADE),
                        Tone::Trouble => (Color32::from_rgb(255, 132, 112), crate::theme::RED, crate::theme::RED.gamma_multiply(0.55)),
                    };
                    let shown = egui::Frame::new()
                        .fill(Color32::from_black_alpha((170.0 * fade) as u8))
                        .stroke(Stroke::new(1.0, rim.gamma_multiply(fade)))
                        .inner_margin(egui::Margin { left: 14, right: if notice.link.is_some() { 20 + chip.x as i8 } else { 12 }, top: 4, bottom: 4 })
                        .corner_radius(2)
                        .show(ui, |ui| {
                            ui.add(egui::Label::new(egui::RichText::new(&notice.text).color(words.gamma_multiply(fade)).font(crate::theme::strong(14.0))).wrap());
                        });
                    let r = shown.response.rect;
                    ui.painter().rect_filled(Rect::from_min_size(r.min + EVec2::new(1.0, 1.0), EVec2::new(3.0, r.height() - 2.0)), 1.0, edge.gamma_multiply(0.9 * fade));
                    if let Some(link) = notice.link {
                        crate::hint::paint_chip(ui.painter(), Rect::from_center_size(Pos2::new(r.right() - 8.0 - chip.x / 2.0, r.center().y), chip), false, fade);
                        pointing.push((i, r, link, fade));
                    }
                }
            });
        });
        self.messages_rect = Some(column.response.rect);
        // A notice that points somewhere is a button: a click goes there. (Each has an area of its
        // own over it, as the column is no area to click, so that the rest of the column lets the
        // battlefield's clicks through.)
        for (i, r, link, fade) in pointing {
            let button = egui::Area::new(egui::Id::new(("notice_link", i))).fixed_pos(r.min).order(egui::Order::Middle).show(ctx, |ui| {
                let (rect, response) = ui.allocate_exact_size(r.size(), egui::Sense::click());
                if response.hovered() {
                    ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, crate::theme::AMBER.gamma_multiply(0.7 * fade)), egui::StrokeKind::Inside);
                    crate::hint::paint_chip(ui.painter(), Rect::from_center_size(Pos2::new(rect.right() - 8.0 - chip.x / 2.0, rect.center().y), chip), true, fade);
                }
                response
            });
            if button.inner.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.follow = Some(link);
            }
        }
    }
}

/// Multiplayer, in the top bar: the slowest connection's bars and milliseconds (the battle goes
/// at the pace of its slowest player); on hover, everyone's: their round trips, who is behind
/// and who is away, with the time left to come back.
fn net_signal(ui: &mut egui::Ui, game: &Game) {
    use ta_ui::i18n::{t, tf};
    let Some(net) = &game.net else { return };
    let people: Vec<u8> = (0..game.sim.players.len() as u8).filter(|&p| net.player_name(p).is_some()).collect();
    if people.len() < 2 {
        return;
    }
    let host = net.host_player();
    let pings = net.player_pings();
    // (The host's own round trip is nothing: not the slowest.)
    let worst = pings.iter().filter(|(p, _)| Some(*p) != host).map(|(_, ms)| *ms).max();
    // (Someone away says so in place of the milliseconds.)
    let (text, colour) = if !net.away.is_empty() {
        (tf("{n} AWAY", &[("n", &net.away.len())]), crate::theme::AMBER)
    } else {
        match worst {
            Some(ms) => (ms.to_string(), if net.waiting_for.is_empty() { crate::theme::TEXT } else { crate::theme::AMBER }),
            None => ("—".to_string(), crate::theme::STEEL_DIM),
        }
    };
    let galley = ui.painter().layout_no_wrap(text, crate::theme::strong(11.5), colour);
    let (rect, response) = ui.allocate_exact_size(EVec2::new(19.0 + galley.size().x + 4.0, 20.0), egui::Sense::hover());
    crate::lobby::signal::bars(ui.painter(), rect, worst);
    ui.painter().galley(egui::pos2(rect.min.x + 19.0, rect.center().y - galley.size().y / 2.0 + 1.0), galley, colour);
    response.on_hover_ui(|ui| {
        ui.label(ta_ui::widgets::caption(n("CONNECTIONS")));
        for p in people {
            let name = game.player_label(p);
            let line = if let Some((_, secs)) = net.away.iter().find(|(q, _)| *q == p) {
                tf("{name}: away, {time} to come back", &[("name", &name), ("time", &format!("{}:{:02}", secs / 60, secs % 60))])
            } else if net.player_name(p).is_some_and(|n| net.waiting_for.iter().any(|w| w == n)) {
                tf("{name}: behind", &[("name", &name)])
            } else if host == Some(p) {
                tf("{name}: hosting", &[("name", &name)])
            } else {
                match pings.iter().find(|(q, _)| *q == p) {
                    Some((_, ms)) => tf("{name}: {ms} ms", &[("name", &name), ("ms", ms)]),
                    None => tf("{name}: {state}", &[("name", &name), ("state", &t("Measuring…"))]),
                }
            };
            ui.label(line);
        }
    });
}

/// Every player at a glance (hold Tab): side, team, whether still in, and — for ourselves,
/// allies, spectators and after the game — units, income and kills.
/// The players panel (Tab): each player's picture (`pictures`, by player; a colour square
/// without one), name, side, team, and what we may see of their army and economy.
pub(crate) fn players_panel(ctx: &egui::Context, game: &Game, offset: [f32; 2], pictures: &[(u8, Option<UnitPic>)]) {
    let players = &game.sim.players;
    let mut order: Vec<u8> = (0..players.len() as u8).collect();
    order.sort_by_key(|&i| (players[i as usize].team, i));
    // Multiplayer: everyone's round trip to the host, and who is away or behind.
    let net = game.net.as_ref();
    let pings = net.map(|n| n.player_pings()).unwrap_or_default();
    ta_ui::panel::strip(ctx, "players_panel", egui::Align2::CENTER_TOP, EVec2::from(offset), |ui| {
        {
            ta_ui::panel::section(ui, "PLAYERS");
            if game.team_markers {
                ui.label(egui::RichText::new(crate::team_markers::legend(game)).color(Color32::WHITE));
                ui.add_space(5.0);
            }
            let columns = if net.is_some() { 10 } else { 9 };
            egui::Grid::new("players_grid").num_columns(columns).spacing([16.0, 6.0]).show(ui, |ui| {
                for h in ["", n("PLAYER"), n("SIDE"), n("TEAM"), n("UNITS"), n("METAL"), n("ENERGY"), n("KILLS")] {
                    ui.label(ta_ui::widgets::caption(h));
                }
                if net.is_some() {
                    ui.label(ta_ui::widgets::caption(n("PING")));
                }
                ui.label(ta_ui::widgets::caption(n("STATUS")));
                ui.end_row();
                for i in order {
                    let p = &players[i as usize];
                    let colour = crate::palette::team_color(p.color);
                    match pictures.iter().find(|(q, _)| *q == i).and_then(|(_, pic)| *pic) {
                        Some(pic) => {
                            let (r, _) = ui.allocate_exact_size(EVec2::new(28.0, 28.0), egui::Sense::hover());
                            pic.paint(ui.painter(), r, Color32::WHITE);
                            ui.painter().rect_stroke(r.expand(0.5), 1.0, Stroke::new(1.5, colour), egui::StrokeKind::Outside);
                        }
                        None => {
                            let (r, _) = ui.allocate_exact_size(EVec2::new(14.0, 14.0), egui::Sense::hover());
                            ui.painter().rect_filled(r, 2.0, colour);
                        }
                    }
                    let me = !game.spectator && i == game.local_player;
                    let name = if game.team_markers { format!("{} {}", crate::team_markers::player(game, i).label(), game.player_label(i)) } else { game.player_label(i) };
                    let name = egui::RichText::new(name);
                    ui.label(if me { name.strong().color(ACCENT) } else { name.color(Color32::WHITE) });
                    ui.label(p.side.as_str());
                    ui.label(format!("{}", p.team + 1));
                    if game.may_inspect(i) {
                        let e = &p.economy;
                        ui.label(p.unit_count.to_string());
                        ui.label(egui::RichText::new(format!("+{:.1}", e.metal_income.to_f32())).color(METAL));
                        ui.label(egui::RichText::new(format!("+{:.0}", e.energy_income.to_f32())).color(ENERGY));
                        ui.label(p.stats.kills.to_string());
                    } else {
                        for _ in 0..4 {
                            ui.label(egui::RichText::new("—").color(Color32::from_gray(110)));
                        }
                    }
                    // (A person's: the host's own and the computers' have none.)
                    let link = net.and_then(|net| {
                        let name = net.player_name(i)?;
                        let away = net.away.iter().find(|(p, _)| *p == i).map(|(_, secs)| *secs);
                        Some((away, net.waiting_for.iter().any(|w| w == name)))
                    });
                    if let Some(net) = net {
                        let ms = pings.iter().find(|(p, _)| *p == i).map(|(_, ms)| *ms).filter(|_| net.host_player() != Some(i) && link.is_some_and(|(away, _)| away.is_none()));
                        match ms {
                            Some(ms) => ui.label(egui::RichText::new(ta_ui::i18n::tf("{ms} ms", &[("ms", &ms)])).color(crate::lobby::signal::tone(ms))),
                            None => ui.label(egui::RichText::new("—").color(Color32::from_gray(110))),
                        };
                    }
                    match link {
                        _ if p.defeated => ta_ui::widgets::tag(ui, "OUT", crate::theme::RED),
                        Some((Some(secs), _)) => ta_ui::widgets::tag(ui, &ta_ui::i18n::tf("AWAY {time}", &[("time", &format!("{}:{:02}", secs / 60, secs % 60))]), crate::theme::AMBER),
                        Some((None, true)) => ta_ui::widgets::tag(ui, "BEHIND", crate::theme::AMBER),
                        _ => ta_ui::widgets::tag(ui, "IN", crate::theme::GREEN),
                    };
                    ui.end_row();
                }
            });
        }
    });
}

/// The Share menu's entries: resources for each ally, and the selected units.
pub(crate) fn share_menu(ui: &mut egui::Ui, game: &Game) -> Option<Gift> {
    use ta_ui::i18n::{t, tf};
    let mut gift = None;
    for a in game.allies() {
        let side = game.sim.players.get(a as usize).map_or("", |p| p.side.as_str()).to_string();
        ui.label(egui::RichText::new(format!("{} · {side}", game.player_label(a))).strong());
        ui.horizontal(|ui| {
            for (m, e) in [(100, 0), (500, 0), (0, 1000), (0, 5000)] {
                let label = if m > 0 { tf("{n} metal", &[("n", &m)]) } else { tf("{n} energy", &[("n", &e)]) };
                if ui.button(label).clicked() {
                    gift = Some(Gift::Resources(a, m, e));
                }
            }
        });
        let n = game.selection.len();
        if n > 0 {
            let label = if n == 1 { t("Give the selected unit").to_string() } else { tf("Give the {n} selected units", &[("n", &n)]) };
            if ui.button(label).on_hover_text(t("They change sides with their orders dropped; Commanders stay with you")).clicked() {
                gift = Some(Gift::Units(a));
            }
        }
    }
    gift
}

impl Hud {
    /// The share panel (TA's H): resources and the selected units for an ally.
    fn share_panel(&mut self, ctx: &egui::Context, game: &mut Game, clear: Clearance) {
        if !game.share_panel {
            return;
        }
        if game.allies().is_empty() || game.spectator {
            game.share_panel = false;
            return;
        }
        let mut close = false;
        egui::Area::new(egui::Id::new("share_panel")).anchor(egui::Align2::RIGHT_TOP, [-14.0, clear.top + clear.right_top + 12.0]).order(egui::Order::Foreground).show(ctx, |ui| {
            let key = game.keys.text("share").to_string();
            let subtitle = ta_ui::i18n::tf("WITH YOUR ALLIES · {key} CLOSES", &[("key", &key)]);
            let page = ta_ui::panel::Panel::new("SHARE").subtitle(&subtitle).width(330.0).exit(ta_ui::panel::Exit::Close);
            let response = page.show(ui, |ui| {
                ui.set_width(330.0);
                // What goes to them on its own (TA's +sharemetal and +shareenergy).
                let mut s = game.sim.players.get(game.local_player as usize).map(|p| p.sharing).unwrap_or_default();
                let before = s;
                ui.label(egui::RichText::new(ta_ui::i18n::t("SPARE RESOURCES GO TO YOUR ALLIES")).font(crate::theme::strong(11.0)).color(crate::theme::STEEL));
                ui.horizontal(|ui| {
                    let keep = |k: Option<ta_sim::Fx>| k.map(|k| ta_ui::i18n::tf(" (above {n})", &[("n", &format!("{:.0}", k.to_f32()))])).unwrap_or_default();
                    ui.checkbox(&mut s.metal, format!("{}{}", ta_ui::i18n::t("Metal"), keep(s.keep_metal)))
                        .on_hover_text(ta_ui::i18n::t("What your storage cannot hold goes to allies with room (+setsharemetal N shares everything above N)"));
                    ui.checkbox(&mut s.energy, format!("{}{}", ta_ui::i18n::t("Energy"), keep(s.keep_energy)))
                        .on_hover_text(ta_ui::i18n::t("What your storage cannot hold goes to allies with room (+setshareenergy N shares everything above N)"));
                });
                if s != before {
                    game.issue(ta_sim::Command::SetSharing { player: game.local_player, sharing: s });
                }
                ui.add_space(6.0);
                match share_menu(ui, game) {
                    Some(Gift::Resources(a, m, e)) => self.share = Some((a, m, e)),
                    Some(Gift::Units(a)) => self.give = Some(a),
                    None => {}
                }
            });
            close = response.exit;
        });
        if close {
            game.share_panel = false;
        }
    }
}

/// A missile silo's or anti-missile system's stockpile: the line telling the missiles ready
/// and how far the next one is, and that fraction (None for other units); at the most a silo
/// holds (`ta_sim::combat::MAX_STOCKPILE`), full: it builds no more until one is fired (it said
/// "next 0%", the bar empty, as if building one).
pub(crate) fn stockpile(sim: &ta_sim::Sim, u: &ta_sim::Unit) -> Option<(String, f32)> {
    use ta_ui::i18n::tf;
    let interceptor = sim.unit_type(u).weapons.iter().flatten().map(|&w| sim.weapon_types.get(w)).find(|w| w.def.stockpile)?.def.interceptor;
    if u.stockpile >= ta_sim::combat::MAX_STOCKPILE {
        let line = if interceptor { tf("Interceptors {n}  ·  full", &[("n", &u.stockpile)]) } else { tf("Missiles {n}  ·  full", &[("n", &u.stockpile)]) };
        return Some((line, 1.0));
    }
    let next = u.stockpile_progress.to_f32().clamp(0.0, 1.0);
    let pct = format!("{:.0}", (next * 100.0).floor());
    let line = if interceptor {
        tf("Interceptors {n}  ·  next {pct}%", &[("n", &u.stockpile), ("pct", &pct)])
    } else {
        tf("Missiles {n}  ·  next {pct}%", &[("n", &u.stockpile), ("pct", &pct)])
    };
    Some((line, next))
}

/// "45 s", "2 min 5 s".
pub(crate) fn duration(secs: f32) -> String {
    use ta_ui::i18n::tf;
    let s = secs.round() as u32;
    if s >= 60 { tf("{min} min {s} s", &[("min", &(s / 60)), ("s", &(s % 60))]) } else { tf("{s} s", &[("s", &s)]) }
}

/// What a unit type is good for, in a line or two: firepower and reach, sensors, economy.
pub(crate) fn unit_facts(sim: &ta_sim::Sim, t: &ta_sim::UnitType) -> Vec<String> {
    unit_fact_lines(sim, t, Economy::Type)
}

/// How much of a type's economy [`unit_fact_lines`] tells: what the type makes and uses, or only
/// what it stores (a hover card says the rest, for the unit itself, in its figures).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Economy {
    Type,
    Storage,
}

/// [`unit_facts`], telling as much of the economy as `economy` says.
pub(crate) fn unit_fact_lines(sim: &ta_sim::Sim, t: &ta_sim::UnitType, economy_part: Economy) -> Vec<String> {
    use ta_ui::i18n::tf;
    let mut out = Vec::new();
    let mut combat = Vec::new();
    // Sustained damage of the ordinary weapons (not D-guns, nukes or paralysers).
    let (mut dps, mut range) = (0.0f64, 0.0f64);
    for w in t.weapons.iter().flatten().map(|&w| sim.weapon_types.get(w)) {
        if w.command_fire || w.def.stockpile || w.paralyzer {
            continue;
        }
        dps += w.default_damage.to_f64() * w.burst.max(1) as f64 * 30.0 / w.reload_ticks.max(1) as f64;
        range = range.max(w.range.to_f64());
    }
    let whole = |v: f64| format!("{v:.0}");
    if dps > 0.0 {
        combat.push(tf("Damage {dps}/s", &[("dps", &whole(dps))]));
        combat.push(tf("Range {range}", &[("range", &whole(range))]));
    }
    if t.def.radar_distance > 0.0 {
        combat.push(tf("Radar {range}", &[("range", &whole(t.def.radar_distance))]));
    }
    if t.def.sonar_distance > 0.0 {
        combat.push(tf("Sonar {range}", &[("range", &whole(t.def.sonar_distance))]));
    }
    if t.def.radar_jam.max(t.def.sonar_jam) > 0.0 {
        combat.push(tf("Jams {range}", &[("range", &whole(t.def.radar_jam.max(t.def.sonar_jam)))]));
    }
    if !combat.is_empty() {
        out.push(combat.join("  ·  "));
    }
    let mut economy = Vec::new();
    // Rates per second: small ones keep a decimal.
    let rate = |v: f64| if v < 10.0 { format!("{v:.1}") } else { format!("{v:.0}") };
    // Net energy (solar plants "use" negative energy; many units make what they use).
    let energy = t.def.energy_make - t.def.energy_use;
    let whole_economy = economy_part == Economy::Type;
    if energy >= 0.05 && whole_economy {
        economy.push(tf("+{rate} energy", &[("rate", &rate(energy))]));
    }
    if t.def.wind_generator > 0.0 && whole_economy {
        economy.push(ta_ui::i18n::t("Energy from the wind").to_string());
    }
    if t.def.tidal_generator > 0.0 && whole_economy {
        economy.push(ta_ui::i18n::t("Energy from the tide").to_string());
    }
    let metal_in = t.def.metal_make + t.def.makes_metal;
    if metal_in >= 0.05 && whole_economy {
        economy.push(tf("+{rate} metal", &[("rate", &rate(metal_in))]));
    }
    if t.def.extracts_metal > 0.0 && whole_economy {
        economy.push(ta_ui::i18n::t("Extracts metal").to_string());
    }
    if energy <= -0.05 && whole_economy {
        economy.push(tf("Uses {rate} energy", &[("rate", &rate(-energy))]));
    }
    if t.def.metal_storage > 0.0 || t.def.energy_storage > 0.0 {
        economy.push(tf("Stores {metal} metal, {energy} energy", &[("metal", &whole(t.def.metal_storage)), ("energy", &whole(t.def.energy_storage))]));
    }
    if !economy.is_empty() {
        out.push(economy.join("  ·  "));
    }
    out
}

/// Picks whose vision a spectator (or replay) shows.
fn vision_picker(ui: &mut egui::Ui, game: &mut Game) {
    let names: Vec<String> = (0..game.sim.players.len() as u8).map(|i| format!("{} · {}", game.player_label(i), game.sim.players[i as usize].side)).collect();
    let current = game.view_as.and_then(|i| names.get(i as usize)).map_or(ta_ui::i18n::t("Vision: everything").to_string(), |n| ta_ui::i18n::tf("Vision: {player}", &[("player", n)]));
    egui::ComboBox::from_id_salt("replay_vision").selected_text(current).width(190.0).show_ui(ui, |ui| {
        if ui.selectable_label(game.view_as.is_none(), ta_ui::i18n::t("Everything")).clicked() {
            game.view_as = None;
        }
        for (i, n) in names.iter().enumerate() {
            if ui.selectable_label(game.view_as == Some(i as u8), n).clicked() {
                game.view_as = Some(i as u8);
            }
        }
    });
}

/// A store of metal or energy as the top bar reads it.
struct Store {
    amount: f32,
    capacity: f32,
    income: f32,
    expense: f32,
    state: StoreState,
}

/// What a store is doing, in the one word that matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StoreState {
    /// Filling or holding: nothing to do.
    Steady,
    /// Draining: empty in this many seconds (shown when under a minute).
    Draining(u32),
    /// Empty while builders want more: everything builds slower.
    Stalling,
    /// Full and still coming in: the surplus is wasted.
    Full,
}

impl Store {
    fn of(amount: Fx, capacity: Fx, income: Fx, expense: Fx, rationed: bool) -> Store {
        let (amount, capacity, income, expense) = (amount.to_f32(), capacity.to_f32(), income.to_f32(), expense.to_f32());
        let net = income - expense;
        let state = if amount <= (capacity * 0.02).max(1.0) && (rationed || net < -0.05) {
            StoreState::Stalling
        } else if capacity > 0.0 && amount >= capacity * 0.99 && net > 0.05 {
            StoreState::Full
        } else if net < -0.05 && amount / -net < 60.0 {
            StoreState::Draining((amount / -net).ceil() as u32)
        } else {
            StoreState::Steady
        };
        Store { amount, capacity, income, expense, state }
    }

    /// What its tip says: the figures, and what to do about a store in trouble.
    fn tip(&self) -> String {
        use ta_ui::i18n::{t, tf};
        let mut tip = tf(
            "{amount} of {capacity} stored. Income {income}/s, spending {spending}/s.",
            &[
                ("amount", &format!("{:.0}", self.amount)),
                ("capacity", &format!("{:.0}", self.capacity)),
                ("income", &format!("{:.1}", self.income)),
                ("spending", &format!("{:.1}", self.expense)),
            ],
        );
        let advice = match self.state {
            StoreState::Stalling => Some(t("Empty: everything builds slower until more comes in. Build more, or build less at once.")),
            StoreState::Full => Some(t("Full: what comes in now is wasted. Spend it, or build storage.")),
            StoreState::Draining(_) => Some(t("Spending more than comes in: it will run dry.")),
            StoreState::Steady => None,
        };
        if let Some(a) = advice {
            tip = format!("{tip}\n{a}");
        }
        tip
    }
}

/// A resource: its store as a bar with the amount in it, how fast it changes, and a warning
/// when it runs dry or overflows; the details on the pointer.
fn resource_bar(ui: &mut egui::Ui, name: &str, s: Store, color: Color32, bar_width: f32, time: f64, tight: bool) {
    use crate::theme::*;
    use ta_ui::i18n::tf;
    let response = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(ta_ui::widgets::caption(name).color(color));
            let frac = if s.capacity > 0.0 { (s.amount / s.capacity).clamp(0.0, 1.0) } else { 0.0 };
            let (rect, _) = ui.allocate_exact_size(EVec2::new(bar_width, 12.0), egui::Sense::hover());
            // Recessed, marked in quarters; a stalling store's bar glows red.
            ui.painter().add(hud_skin::gauge(rect, frac, color, 4));
            if s.state == StoreState::Stalling {
                let pulse = (0.55 + 0.45 * (time * 5.0).sin()) as f32;
                ui.painter().rect_stroke(rect.expand(1.5), CornerRadius::same(2), Stroke::new(1.5, RED.gamma_multiply(pulse)), egui::StrokeKind::Outside);
                ui.ctx().request_repaint();
            }
            ui.label(egui::RichText::new(format!("{:.0}", s.amount)).font(strong(13.0)).color(TEXT_BRIGHT));
            let net = s.income - s.expense;
            let rate = if net >= 0.0 { format!("+{net:.1}") } else { format!("{net:.1}") };
            let rate_color = match s.state {
                StoreState::Stalling => RED,
                StoreState::Draining(secs) if secs < 20 => RED,
                StoreState::Draining(_) => AMBER_BRIGHT,
                _ if net > 0.05 => GREEN,
                _ => STEEL,
            };
            ui.label(egui::RichText::new(rate).font(strong(12.0)).color(rate_color));
            match s.state {
                StoreState::Stalling => {
                    ta_ui::widgets::tag(ui, n("STALLING"), RED);
                }
                StoreState::Full if !tight => {
                    ta_ui::widgets::tag(ui, n("FULL"), STEEL);
                }
                StoreState::Draining(secs) if secs < 20 => {
                    ta_ui::widgets::tag(ui, &tf("EMPTY IN {secs} S", &[("secs", &secs)]), RED);
                }
                _ => {}
            }
        })
        .response;
    let tip = s.tip();
    response.on_hover_text(tip);
}

/// A caption and its value, side by side ("UNITS 12 / 1500").
fn stat(ui: &mut egui::Ui, label: &str, value: egui::RichText) -> egui::Response {
    // (The caption translates it.)
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        ui.label(ta_ui::widgets::caption(label));
        ui.label(value.font(crate::theme::strong(12.5)));
    })
    .response
}

/// What a button of the replay's transport does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Transport {
    Back,
    Play,
    Pause,
    Ahead,
}

/// A transport button: gunmetal with a glyph and a short label ("-30 s"); play and pause
/// brass, bigger, alone.
fn transport(ui: &mut egui::Ui, kind: Transport, label: String) -> egui::Response {
    use ta_ui::metal::{self, Slab};
    let main = matches!(kind, Transport::Play | Transport::Pause);
    let galley = (!label.is_empty()).then(|| ui.painter().layout_no_wrap(label, crate::theme::strong(11.0), Color32::WHITE));
    let width = if main { 38.0 } else { galley.as_ref().map_or(0.0, |g| g.size().x) + 30.0 };
    let (rect, response) = ui.allocate_exact_size(EVec2::new(width, if main { 30.0 } else { 26.0 }), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let hot = if response.hovered() { 1.0 } else { 0.0 };
        let down = response.is_pointer_button_down_on();
        let slab = if main { Slab::Brass } else { Slab::Steel };
        let painter = ui.painter();
        painter.add(metal::slab(ui.ctx(), rect, slab, hot, down));
        let color = slab.text(hot);
        let dy = if down { 1.0 } else { 0.0 };
        let c = if main { rect.center() } else { Pos2::new(rect.left() + 12.0, rect.center().y) } + EVec2::new(0.0, dy);
        let s = if main { 6.0 } else { 4.0 };
        let tri = |c: Pos2, dir: f32| egui::Shape::convex_polygon(vec![Pos2::new(c.x + s * dir, c.y), Pos2::new(c.x - s * 0.8 * dir, c.y - s), Pos2::new(c.x - s * 0.8 * dir, c.y + s)], color, Stroke::NONE);
        let edge = |shape: egui::Shape| {
            let mut shadow = shape.clone();
            shadow.translate(EVec2::new(1.0, 1.0));
            if let egui::Shape::Path(p) = &mut shadow {
                p.fill = Color32::from_black_alpha(if main { 60 } else { 170 });
            } else if let egui::Shape::Rect(r) = &mut shadow {
                r.fill = Color32::from_black_alpha(if main { 60 } else { 170 });
            }
            painter.add(shadow);
            painter.add(shape);
        };
        match kind {
            Transport::Play => edge(tri(c + EVec2::new(1.0, 0.0), 1.0)),
            Transport::Pause => {
                for dx in [-3.5, 3.5] {
                    edge(egui::Shape::rect_filled(Rect::from_center_size(c + EVec2::new(dx, 0.0), EVec2::new(4.0, 13.0)), 1.0, color));
                }
            }
            Transport::Back => {
                edge(tri(c + EVec2::new(-2.5, 0.0), -1.0));
                edge(tri(c + EVec2::new(3.5, 0.0), -1.0));
            }
            Transport::Ahead => {
                edge(tri(c + EVec2::new(-3.5, 0.0), 1.0));
                edge(tri(c + EVec2::new(2.5, 0.0), 1.0));
            }
        }
        if let Some(g) = galley {
            ta_ui::metal::slab_text(painter, Pos2::new(rect.right() - 7.0 - g.size().x / 2.0, rect.center().y), g, true, color, down);
        }
    }
    if response.clicked() {
        ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Button);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The replay's timeline, `width` wide: a recessed track filled in brass as far as the battle
/// has played, a mark every minute (every five on a long battle), and a handle. A click or a
/// drag returns the tick to go to (a drag when it is let go: going back replays from the start).
/// The cast's switch: the players head to head, the kills and a directing camera (which it
/// turns on with it).
fn cast_switch(ui: &mut egui::Ui, game: &mut Game) {
    use ta_ui::i18n::t;
    let was = game.cast;
    ta_ui::widgets::check_box(ui, &mut game.cast);
    ui.label(egui::RichText::new(t("Cast view")).font(crate::theme::body(13.0)).color(crate::theme::TEXT))
        .on_hover_text(t("A broadcast's view: the players head to head, the notable kills, and a camera that directs itself"));
    if game.cast && !was {
        game.action_camera = true;
    }
}

/// The replay's timeline: where it is, a click or a drag to jump; the replay's big moments found
/// ahead (`scan`) marked over it, a click on one jumping to a few seconds before it, and while
/// the scan runs, how far it has got along its foot.
fn timeline(ui: &mut egui::Ui, tick: u64, end: u64, width: f32, scan: Option<&crate::replay_scan::ScanState>) -> Option<u64> {
    use crate::theme::{AMBER, AMBER_BRIGHT, AMBER_DIM, INSET};
    let (rect, response) = ui.allocate_exact_size(EVec2::new(width, 22.0), egui::Sense::click_and_drag());
    let track = Rect::from_min_max(Pos2::new(rect.left() + 6.0, rect.center().y - 4.0), Pos2::new(rect.right() - 6.0, rect.center().y + 4.0));
    let at = |x: f32| ((x - track.left()) / track.width()).clamp(0.0, 1.0);
    let dragging = response.dragged();
    let shown = match response.interact_pointer_pos() {
        Some(p) if dragging => at(p.x),
        _ => (tick as f32 / end as f32).clamp(0.0, 1.0),
    };
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.add(ta_ui::metal::well(track.expand(2.0), INSET));
        let filled = Rect::from_min_max(track.min, Pos2::new(track.left() + track.width() * shown, track.bottom()));
        painter.add(ta_ui::metal::vgrad(filled, AMBER_BRIGHT, AMBER_DIM));
        let minutes = end / (30 * 60);
        let every = if minutes > 40 { 10 } else if minutes > 12 { 5 } else { 1 };
        for m in (every..=minutes).step_by(every as usize) {
            let x = track.left() + track.width() * (m * 30 * 60) as f32 / end as f32;
            painter.vline(x.round() + 0.5, (track.top() - 1.0)..=(track.bottom() + 1.0), Stroke::new(1.0, Color32::from_black_alpha(150)));
        }
        // Where the pointer would jump to.
        if let Some(h) = response.hover_pos().filter(|_| !dragging) {
            painter.vline(h.x, track.y_range().expand(3.0), Stroke::new(1.0, Color32::from_white_alpha(70)));
        }
        // How far the scan has got (while it runs).
        if let Some(s) = scan.filter(|s| !s.finished) {
            let to = track.left() + track.width() * (s.done as f32 / end as f32).clamp(0.0, 1.0);
            painter.hline(track.left()..=to, track.bottom() + 3.0, Stroke::new(1.0, Color32::from_white_alpha(60)));
        }
        let x = track.left() + track.width() * shown;
        let handle = Rect::from_center_size(Pos2::new(x, rect.center().y), EVec2::new(8.0, 18.0));
        painter.add(ta_ui::metal::slab(ui.ctx(), handle, ta_ui::metal::Slab::Steel, if response.hovered() || dragging { 1.0 } else { 0.0 }, dragging));
        painter.hline(handle.x_range().shrink(2.0), handle.center().y, Stroke::new(1.0, AMBER));
    }
    // The big moments: a mark each over the track, its colour its kind, its size its worth;
    // the one under the pointer named.
    let marks = scan.map_or(&[][..], |s| &s.marks[..]);
    let mark_x = |m: &crate::highlights::Moment| track.left() + track.width() * (m.start as f32 / end as f32).clamp(0.0, 1.0);
    let hovered = response.hover_pos().filter(|_| !dragging).and_then(|h| marks.iter().filter(|m| (mark_x(m) - h.x).abs() <= 6.0).min_by(|a, b| (mark_x(a) - h.x).abs().total_cmp(&(mark_x(b) - h.x).abs())));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        for m in marks {
            let x = mark_x(m);
            let colour = match m.kind {
                crate::highlights::MomentKind::Nuke => Color32::from_rgb(255, 96, 70),
                crate::highlights::MomentKind::Commander => Color32::from_rgb(240, 236, 220),
                crate::highlights::MomentKind::Battle => AMBER_BRIGHT,
            };
            let size = (2.5 + (m.worth.max(1.0).log10() - 2.5).clamp(0.0, 2.5) * 1.2) * if hovered.is_some_and(|h| std::ptr::eq(h, m)) { 1.4 } else { 1.0 };
            painter.vline(x, track.y_range(), Stroke::new(1.0, colour.gamma_multiply(0.45)));
            let tip = Pos2::new(x, track.top() - 1.0);
            painter.add(egui::Shape::convex_polygon(vec![Pos2::new(x - size, tip.y - size * 1.6), Pos2::new(x + size, tip.y - size * 1.6), tip], colour, Stroke::new(1.0, Color32::from_black_alpha(160))));
        }
    }
    let hover = match hovered {
        Some(m) => {
            let (title, detail) = crate::highlights::describe(m);
            let s = m.start / 30;
            format!("{:02}:{:02} · {}\n{}", s / 60, s % 60, ta_ui::i18n::t(title), detail)
        }
        None if scan.is_some_and(|s| !s.finished) => format!("{}\n{}", ta_ui::i18n::t("Click or drag to jump there"), ta_ui::i18n::t("Finding the replay's big moments…")),
        None => ta_ui::i18n::t("Click or drag to jump there").to_string(),
    };
    let response = response.on_hover_text(hover).on_hover_cursor(egui::CursorIcon::PointingHand);
    // (A click on a mark: a few seconds before its moment.)
    if response.clicked() && let Some(m) = hovered {
        return Some(m.start.saturating_sub(90));
    }
    let to = |f: f32| (f * end as f32) as u64;
    if response.drag_stopped() {
        return ui.ctx().pointer_latest_pos().map(|p| to(at(p.x)));
    }
    if response.clicked() {
        return response.interact_pointer_pos().map(|p| to(at(p.x)));
    }
    None
}

/// A counter of idle units on the top bar ("IDLE BUILDERS 2"): a gunmetal button with an
/// amber lamp, as the deck marks idle units.
fn idle_chip(ui: &mut egui::Ui, text: &str) -> egui::Response {
    use crate::theme::{AMBER, AMBER_BRIGHT, CREAM};
    let galley = ta_ui::widgets::galley(ui, egui::RichText::new(text).font(crate::theme::strong(11.0)).extra_letter_spacing(ta_ui::i18n::tracking(0.8)));
    let (rect, response) = ui.allocate_exact_size(EVec2::new(galley.size().x + 28.0, 22.0), egui::Sense::click());
    let hot = if response.hovered() { 1.0 } else { 0.0 };
    let down = response.is_pointer_button_down_on();
    let painter = ui.painter();
    painter.add(ta_ui::metal::slab(ui.ctx(), rect, ta_ui::metal::Slab::Steel, hot, down));
    // The lamp breathes, as an alert does, slowly.
    let pulse = 0.65 + 0.35 * (ui.input(|i| i.time) * 2.4).sin() as f32;
    painter.add(ta_ui::metal::lamp(Pos2::new(rect.left() + 10.0, rect.center().y + if down { 1.0 } else { 0.0 }), 3.0, AMBER, pulse));
    let text = Pos2::new(rect.center().x + 6.0, rect.center().y);
    ta_ui::metal::slab_text(painter, text, galley, true, if hot > 0.0 { AMBER_BRIGHT } else { CREAM }, down);
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A unit's picture as a button: set in the panel with a dark rim, lit brass under the
/// pointer, pressed in when clicked.
fn pic_button(ui: &mut egui::Ui, picture: &UnitPic, size: EVec2) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let down = response.is_pointer_button_down_on();
        let shown = if down { rect.shrink(1.0).translate(EVec2::new(0.0, 1.0)) } else { rect };
        let tint = if response.hovered() { Color32::WHITE } else { Color32::from_gray(228) };
        picture.paint(ui.painter(), shown, tint);
        ui.painter().add(pic_frame(rect, response.hovered()));
    }
    if response.clicked() || response.secondary_clicked() {
        ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Button);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The rim of a picture in the panel: a dark edge, a light lip below, brass when `hot`.
fn pic_frame(rect: Rect, hot: bool) -> egui::Shape {
    let mut shapes = vec![
        egui::Shape::rect_stroke(rect, CornerRadius::same(1), Stroke::new(1.0, Color32::from_black_alpha(220)), egui::StrokeKind::Inside),
        egui::Shape::hline(rect.x_range(), rect.max.y + 0.5, Stroke::new(1.0, Color32::from_white_alpha(28))),
    ];
    if hot {
        shapes.push(egui::Shape::rect_stroke(rect.shrink(1.0), CornerRadius::same(1), Stroke::new(1.5, crate::theme::AMBER_BRIGHT), egui::StrokeKind::Inside));
    }
    egui::Shape::Vec(shapes)
}

/// A count over a picture (units queued), in brass lettering outlined in black.
fn count(ui: &egui::Ui, at: Pos2, align: egui::Align2, n: usize, size: f32) {
    let galley = ui.painter().layout_no_wrap(n.to_string(), crate::theme::display(size), crate::theme::AMBER_BRIGHT);
    let rect = align.anchor_size(at, galley.size());
    ta_ui::metal::embossed(ui.painter(), rect.min, galley, crate::theme::AMBER_BRIGHT);
}

/// Focuses the camera on a unit (used at game start).
pub fn focus_on(camera: &mut Camera, game: &Game, id: UnitId) {
    if let Some(u) = game.sim.unit(id) {
        let p = u.pos.to_f32();
        camera.focus = Vec3::new(p[0], p[1], p[2]);
    }
}

/// The colour of an order's acknowledgement (as its route is drawn).
fn mark_color(kind: crate::game::MarkKind) -> Color32 {
    use crate::game::MarkKind as K;
    match kind {
        K::Move => Color32::from_rgb(110, 235, 110),
        K::Fight => Color32::from_rgb(240, 186, 64),
        K::Patrol => Color32::from_rgb(100, 160, 255),
        K::Attack => Color32::from_rgb(245, 80, 64),
        K::Help => Color32::from_rgb(90, 225, 225),
        K::Reclaim => Color32::from_rgb(214, 214, 96),
        K::Unload => Color32::from_rgb(200, 150, 255),
    }
}

/// A resource's row on a hover card: its name and colour, and what the unit makes and uses of it a
/// second.
struct Figure {
    label: &'static str,
    colour: Color32,
    make: f32,
    uses: f32,
}

/// The card's rows for a unit's flows: metal, then energy, those it makes or uses.
fn figure_rows(f: &ta_sim::flows::UnitFlows) -> Vec<Figure> {
    [(n("METAL"), METAL, f.metal_make, f.metal_use), (n("ENERGY"), ENERGY, f.energy_make, f.energy_use)]
        .into_iter()
        .map(|(label, colour, make, uses)| Figure { label, colour, make: make.to_f32(), uses: uses.to_f32() })
        .filter(|r| r.make >= 0.05 || r.uses >= 0.05)
        .collect()
}

/// What a building put down would make and use, for its ghost's label: "+1.8 metal  ·  Uses 3.0
/// energy".
fn ghost_figures(f: &ta_sim::flows::UnitFlows) -> String {
    use ta_ui::i18n::tf;
    let rate = |v: f32| if v < 10.0 { format!("{v:.1}") } else { format!("{v:.0}") };
    let (metal, energy, uses) = (f.metal_make.to_f32(), f.energy_make.to_f32(), f.energy_use.to_f32());
    let mut parts = Vec::new();
    if metal >= 0.05 {
        parts.push(tf("+{rate} metal", &[("rate", &rate(metal))]));
    }
    if energy >= 0.05 {
        parts.push(tf("+{rate} energy", &[("rate", &rate(energy))]));
    }
    if uses >= 0.05 {
        parts.push(tf("Uses {rate} energy", &[("rate", &rate(uses))]));
    }
    parts.join("  ·  ")
}

/// A rate a second as the cards give it: a decimal while small.
fn per_second(v: f32) -> String {
    ta_ui::i18n::tf("{rate}/s", &[("rate", &if v < 10.0 { format!("{v:.1}") } else { format!("{v:.0}") })])
}

/// One figure's row: the resource, what is made (green) and used (red) a second, and the net
/// where it does both.
fn figure_job(r: &Figure) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    let (caption, figure) = (crate::theme::strong(10.5), crate::theme::strong(12.0));
    let mut job = LayoutJob::default();
    let mut add = |text: &str, font: egui::FontId, color: Color32| job.append(text, 0.0, TextFormat { font_id: font, color, valign: egui::Align::Center, ..Default::default() });
    add(&format!("{}   ", ta_ui::i18n::upper(ta_ui::i18n::t(r.label))), caption, r.colour);
    let (makes, uses) = (r.make >= 0.05, r.uses >= 0.05);
    if makes {
        add(&format!("+{}", per_second(r.make)), figure.clone(), crate::theme::GREEN);
    }
    if uses {
        add(&format!("{}-{}", if makes { "   " } else { "" }, per_second(r.uses)), figure.clone(), Color32::from_rgb(240, 130, 105));
    }
    if makes && uses {
        // (Many of TA's buildings make what they use: a radar tower's five energy cancel.)
        let net = r.make - r.uses;
        let words = if net.abs() < 0.05 { per_second(0.0) } else { format!("{}{}", if net > 0.0 { "+" } else { "-" }, per_second(net.abs())) };
        add(&format!("   = {words}"), figure, crate::theme::TEXT);
    }
    job
}

/// A small card of text lines (text, colour, size) by the cursor, kept within `bounds`: with a
/// health gauge under the name, and under that the unit's figures (what it makes and uses).
fn hover_card(bounds: Rect, painter: &egui::Painter, mouse: Pos2, lines: Vec<(String, Color32, f32)>, health: Option<(f32, String)>, figures: Vec<Figure>) {
    let galleys: Vec<_> = lines.into_iter().map(|(text, color, size)| painter.layout_no_wrap(text, egui::FontId::proportional(size), color)).collect();
    let figure_galleys: Vec<_> = figures.iter().map(|r| painter.layout_job(figure_job(r))).collect();
    // A health gauge under the name, its numbers on it.
    const GAUGE: f32 = 13.0;
    let gauge_text = health.as_ref().map(|(_, s)| painter.layout_no_wrap(s.clone(), crate::theme::strong(10.5), crate::theme::TEXT_BRIGHT));
    let w = galleys
        .iter()
        .chain(&figure_galleys)
        .map(|g| g.size().x)
        .fold(gauge_text.as_ref().map_or(0.0, |g| g.size().x + 40.0), f32::max)
        .max(if health.is_some() { 150.0 } else { 0.0 });
    let rows: f32 = figure_galleys.iter().map(|g| g.size().y + 2.0).sum::<f32>();
    let h: f32 = galleys.iter().map(|g| g.size().y + 1.0).sum::<f32>() + if health.is_some() { GAUGE + 5.0 } else { 0.0 } + if rows > 0.0 { rows + 4.0 } else { 0.0 };
    let mut at = mouse + EVec2::new(18.0, 18.0);
    at.x = at.x.min(bounds.right() - w - 12.0);
    at.y = at.y.min(bounds.bottom() - h - 12.0);
    let card = Rect::from_min_size(at, EVec2::new(w, h)).expand2(EVec2::new(9.0, 7.0));
    painter.add(egui::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(120) }.as_shape(card, 2.0));
    painter.rect_filled(card.expand(1.0), 3.0, ta_ui::metal::STEEL_EDGE);
    painter.add(ta_ui::metal::plate(painter.ctx(), card, crate::theme::PANEL));
    painter.hline(card.x_range().shrink(1.0), card.min.y + 1.0, Stroke::new(2.0, crate::theme::AMBER_DIM));
    let mut y = at.y;
    for (i, g) in galleys.into_iter().enumerate() {
        let gh = g.size().y;
        painter.galley(Pos2::new(at.x, y), g, Color32::WHITE);
        y += gh + 1.0;
        if i == 0
            && let Some((fraction, _)) = &health
        {
            y += 2.0;
            let bar = Rect::from_min_size(Pos2::new(at.x, y), EVec2::new(w, GAUGE));
            painter.add(hud_skin::gauge(bar, *fraction, hud_skin::health_color(*fraction), 1));
            if let Some(text) = gauge_text.clone() {
                ta_ui::metal::slab_text(painter, bar.center(), text, true, crate::theme::TEXT_BRIGHT, false);
            }
            y += GAUGE + 3.0;
        }
        // The figures right under the name and the gauge: what hovering a building is for.
        if i == 0 && !figure_galleys.is_empty() {
            y += 2.0;
            for g in &figure_galleys {
                let gh = g.size().y;
                painter.galley(Pos2::new(at.x, y), g.clone(), Color32::WHITE);
                y += gh + 2.0;
            }
            y += 2.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_sim::flows::UnitFlows;

    /// A silo's line says how far its next missile is; at the most it holds, that it is full
    /// (it builds no more: "next 0%" and an empty bar read as one just begun).
    #[test]
    fn a_full_silo_says_so() {
        // (Loaded as a mod's folder is, so that the silo finds its weapon by name.)
        let dir = std::env::temp_dir().join(format!("ta-full-silo-{}", std::process::id()));
        for (path, text) in [
            ("weapons/nuke.tdf", "[NUKE] { stockpile=1; commandfire=1; targetable=1; range=32000; reloadtime=180; }"),
            ("units/silo.fbi", "[UNITINFO] { UnitName=SILO; Objectname=SILO; FootprintX=4; FootprintZ=4; MaxDamage=2300; BuildTime=100; Weapon1=NUKE; }"),
        ] {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_mod_dir(&dir, "test").unwrap();
        let data = ta_data::GameData::load(&vfs);
        std::fs::remove_dir_all(&dir).ok();
        let types = std::sync::Arc::new(ta_sim::UnitTypes::compile(&data, &|_| None));
        let side = 64;
        let map = ta_formats::tnt::TntMap { width: side, height: side, sea_level: 0, heights: vec![60; (side * side) as usize], features: vec![ta_formats::tnt::FEATURE_NONE; (side * side) as usize], tiles: Vec::new(), tile_map: Vec::new(), feature_names: Vec::new(), minimap: Default::default() };
        let players = [ta_sim::PlayerSetup { side: "ARM".into(), color: 0, start: ta_sim::FxVec3::ZERO, metal: ta_sim::Fx::ZERO, energy: ta_sim::Fx::ZERO, ai: false }];
        let mut sim = ta_sim::Sim::new(types, std::sync::Arc::new(data), &map, &ta_data::MapInfo::default(), 0, &players, 7);
        let id = sim.spawn_unit(sim.types.id("SILO").unwrap(), 0, ta_sim::FxVec3::from_ints(400, 60, 400), 0, true).unwrap();
        sim.grant_stockpile(id, 3);
        assert_eq!(stockpile(&sim, sim.unit(id).unwrap()), Some(("Missiles 3  ·  next 0%".to_string(), 0.0)));
        sim.grant_stockpile(id, ta_sim::combat::MAX_STOCKPILE);
        assert_eq!(stockpile(&sim, sim.unit(id).unwrap()), Some(("Missiles 10  ·  full".to_string(), 1.0)));
    }

    /// Real managed/native texture registration, without a window or original game data.
    #[test]
    #[ignore = "requires a GPU adapter; run with --ignored"]
    fn film_hud_uses_its_own_cameos_and_native_portrait_registrations() {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("GPU adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let ctx = egui::Context::default();
        crate::theme::install_compact(&ctx);
        let mut window = Hud::default();
        let mut renderer = crate::loading::ui_renderer(&device, format, egui_wgpu::RendererOptions::default()).unwrap();
        let dir = std::env::temp_dir().join(format!("ta-film-hud-{}", ta_config::profiles::new_id()));
        std::fs::create_dir_all(dir.join("unitpics")).unwrap();
        for (name, width, colour) in [("warm", 2u16, [255, 0, 0]), ("film_only", 3, [0, 0, 255])] {
            let mut pcx = vec![0u8; 128];
            pcx[0] = 0x0A;
            pcx[3] = 8;
            pcx[8..10].copy_from_slice(&(width - 1).to_le_bytes());
            pcx[65] = 3;
            pcx[66..68].copy_from_slice(&width.to_le_bytes());
            for channel in colour { pcx.extend(std::iter::repeat_n(channel, width as usize)); }
            std::fs::write(dir.join(format!("unitpics/{name}.pcx")), pcx).unwrap();
        }
        let mut vfs = Vfs::new();
        vfs.mount_mod_dir(&dir, "test").unwrap();
        let draw = |ctx: &egui::Context, hud: &mut Hud, renderer: &mut egui_wgpu::Renderer, kinds: &[(usize, &str)]| {
            let mut ids = Vec::new();
            let output = ctx.run_ui(egui::RawInput { max_texture_side: Some(8192), ..Default::default() }, |ui| {
                for &(kind, name) in kinds {
                    let picture = hud.cameo_for_type(ui.ctx(), &vfs, kind, name).expect("synthetic PCX");
                    ui.image((picture.id(), egui::vec2(24.0, 24.0)));
                    ids.push(picture.id());
                }
            });
            let mut textures = output.textures_delta;
            for (id, deltas) in textures.set.drain() {
                for delta in deltas { renderer.update_texture(&device, &queue, id, &delta); }
            }
            for id in textures.free.drain() { renderer.free_texture(&id); }
            ids
        };
        let warm = draw(&ctx, &mut window, &mut renderer, &[(0, "warm")])[0];
        let mut film = crate::film::PictureUi::new(&ctx, &device, format, window.classic).unwrap();
        let film_ids = draw(&film.ctx, &mut film.hud, &mut film.renderer, &[(0, "warm"), (1, "film_only")]);
        for (&id, width) in film_ids.iter().zip([2, 3]) {
            let texture = film.renderer.texture(&id).and_then(|t| t.texture.as_ref()).expect("the film uploads its own cameos");
            assert_eq!(texture.width(), width);
        }
        assert_eq!(window.pics.len(), 1, "a picture loaded by export cannot enter the window's cache");

        // Native IDs are local to a renderer too. Warm the window's namespace before
        // registering the same portrait view independently with the film's renderer.
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("film portrait regression"), size: wgpu::Extent3d { width: 2, height: 2, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        renderer.register_native_texture(&device, &view, wgpu::FilterMode::Linear);
        let window_id = renderer.register_native_texture(&device, &view, wgpu::FilterMode::Linear);
        let key = (0, 0, 48, false);
        window.set_portrait(key, Some(window_id));
        film.hud.request_portrait(key);
        assert_eq!(film.hud.take_portrait_requests(), [key], "a window portrait does not satisfy the film's request");
        film.set_portrait(&device, key, Some(&view));
        let film_id = film.hud.portraits[&key].unwrap();
        assert_ne!(film_id, window_id, "the film must register, rather than copy, a window texture ID");
        assert!(film.renderer.texture(&film_id).is_some());
        assert_eq!(window.portraits[&key], Some(window_id));
        assert!(renderer.texture(&window_id).is_some());
        film.set_portrait(&device, (1, 0, 48, false), None);
        film.hud.request_portrait((1, 0, 48, false));
        assert!(film.hud.take_portrait_requests().is_empty(), "missing models retain the cameo fallback without retrying every frame");
        drop(film);

        let after = draw(&ctx, &mut window, &mut renderer, &[(0, "warm"), (1, "film_only")]);
        assert_eq!(after[0], warm, "export must not replace the window's existing cameo");
        assert_eq!(renderer.texture(&after[1]).unwrap().texture.as_ref().unwrap().width(), 3,
            "a cameo first loaded during export must still be uploaded to the window after export ends");
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn flows(metal_make: f64, metal_use: f64, energy_make: f64, energy_use: f64) -> UnitFlows {
        let fx = Fx::from_f64;
        UnitFlows { metal_make: fx(metal_make), metal_use: fx(metal_use), energy_make: fx(energy_make), energy_use: fx(energy_use) }
    }

    #[test]
    fn a_card_shows_the_resources_a_unit_makes_or_uses_and_no_others() {
        let rows = figure_rows(&flows(1.7, 0.0, 0.0, 3.0));
        assert_eq!(rows.iter().map(|r| r.label).collect::<Vec<_>>(), ["METAL", "ENERGY"]);
        assert_eq!(figure_rows(&flows(0.0, 0.0, 20.0, 0.0)).len(), 1);
        assert!(figure_rows(&flows(0.0, 0.0, 0.01, 0.0)).is_empty(), "a hundredth of a unit a second is nothing");
    }

    #[test]
    fn figures_read_as_made_used_and_net() {
        let text = |f: UnitFlows| figure_rows(&f).iter().map(|r| figure_job(r).text).collect::<Vec<_>>();
        assert_eq!(text(flows(1.7, 0.0, 0.0, 3.0)), ["METAL   +1.7/s", "ENERGY   -3.0/s"]);
        assert_eq!(text(flows(0.0, 0.0, 25.0, 48.0)), ["ENERGY   +25/s   -48/s   = -23/s"]);
        // (Many of TA's buildings make what they use: a radar tower's five energy cancel.)
        assert_eq!(text(flows(0.0, 0.0, 5.0, 5.0)), ["ENERGY   +5.0/s   -5.0/s   = 0.0/s"]);
    }

    #[test]
    fn a_ghost_says_what_the_building_would_make_and_use() {
        assert_eq!(ghost_figures(&flows(1.7, 0.0, 0.0, 3.0)), "+1.7 metal  ·  Uses 3.0 energy");
        assert_eq!(ghost_figures(&flows(0.0, 0.0, 20.0, 0.0)), "+20 energy");
        assert_eq!(ghost_figures(&flows(0.0, 0.0, 0.0, 0.0)), "");
    }
}
