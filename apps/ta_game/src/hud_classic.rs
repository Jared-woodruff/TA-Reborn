//! The classic interface: TA's own 1997 in-game panels rebuilt from its interface data
//! (`hud_classic_art.rs`) and scaled from TA's 640×480 layout to the window height.
//!
//! The side panel holds the minimap and a GUI page for the selection: the builder's build
//! menu (`guis/<unit><page>.gui`), the orders page (`guis/<side>gen.gui`) or, with nothing
//! selected, the main page (`guis/<side>main.gui`). The top bar shows the resources and the
//! bottom bar the selected and the hovered unit, where `gamedata/sidedata.tdf` puts them.
//! Buttons call the same `Game` methods as the modern HUD (`hud.rs`); commands TA had no
//! button for (fight, resurrect, factory repeat, self-destruct) get plain buttons in the
//! places the page leaves free.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, Pos2, Rect, Sense, Vec2, pos2, vec2};
use ta_render::Camera;
use ta_sim::{Fx, UnitId};
use ta_ui::i18n::{n, t, tf};
use ta_vfs::Vfs;
use winit::keyboard::KeyCode;

use crate::game::{CommandMode, Game};
use crate::hud_classic_art::{Art, Frames, Gadget, Gui, SideLayout, Sprite, TaFont};
use crate::hud_classic_scale::Look;
use crate::minimap::Minimap;

/// TA's screen, which every GUI and sidedata position is given in.
const TA_WIDTH: f32 = 640.0;
const TA_HEIGHT: f32 = 480.0;
/// Side panel width and the heights of the top and bottom bars, in TA pixels.
const SIDE_W: f32 = 128.0;
const TOP_H: f32 = 32.0;
const BOTTOM_H: f32 = 33.0;
/// The lettering colour of TA's order buttons, for the buttons drawn without art.
const BUTTON_TEXT: Color32 = Color32::from_rgb(223, 223, 183);
const TEXT: Color32 = Color32::from_rgb(236, 236, 236);
const DIM: Color32 = Color32::from_rgb(170, 170, 160);

/// TA's layout on the window: `s` points per TA pixel from `origin`, `ppp` screen pixels to a
/// point, the art enlarged as `look` has it.
#[derive(Clone, Copy)]
struct Ta {
    origin: Pos2,
    s: f32,
    ppp: f32,
    look: Look,
}

impl Ta {
    fn pos(self, x: f32, y: f32) -> Pos2 {
        self.origin + vec2(x, y) * self.s
    }

    fn rect(self, r: Rect) -> Rect {
        Rect::from_min_max(self.pos(r.min.x, r.min.y), self.pos(r.max.x, r.max.y))
    }

    /// Screen pixels to one of TA's.
    fn px(self) -> f32 {
        self.s * self.ppp
    }

    /// `r` (TA pixels) on the screen, its edges on whole screen pixels: art drawn there is
    /// drawn pixel for pixel, and pieces that meet in TA's layout meet on the screen.
    fn snap(self, r: Rect) -> Rect {
        let on = |v: f32| (v * self.ppp).round() / self.ppp;
        let r = self.rect(r);
        Rect::from_min_max(pos2(on(r.min.x), on(r.min.y)), pos2(on(r.max.x), on(r.max.y)))
    }

    /// The screen pixels `size` TA pixels take.
    fn pixels(self, size: Vec2) -> [usize; 2] {
        [(size.x * self.px()).round().max(1.0) as usize, (size.y * self.px()).round().max(1.0) as usize]
    }
}

/// Points per TA pixel: the window height over the layout's (`height`: TA's 480, or the taller
/// screen a mod's pages were made for), narrower than 4:3 the width.
fn scale(screen: Rect, height: f32) -> f32 {
    (screen.height() / height).min(screen.width() / (height * TA_WIDTH / TA_HEIGHT)).max(0.1)
}

/// TA's PAUSED title: how far under the top bar it stands, and how tall it is (TA pixels).
const PAUSED_AT: f32 = 24.0;
const PAUSED_HEIGHT: f32 = 29.0;

/// Where TA's PAUSED title ends, in points from the screen's top, the layout `height` TA pixels
/// tall: what else says the game is paused goes under it.
fn paused_title_bottom(screen: Rect, height: f32) -> f32 {
    (TOP_H + PAUSED_AT + PAUSED_HEIGHT) * scale(screen, height)
}

/// The part of the screen the classic panels leave to the battlefield, the layout `height` TA
/// pixels tall.
fn view_rect(screen: Rect, height: f32) -> Rect {
    let s = scale(screen, height);
    Rect::from_min_max(screen.min + vec2(SIDE_W * s, TOP_H * s), screen.max - vec2(0.0, BOTTOM_H * s))
}

/// What the bottom bar tells about the button under the pointer.
enum Tip {
    /// A build button: the unit it builds.
    Unit(usize),
    /// Anything else: its name and what it does.
    Text(String, String),
}

/// What a gadget of TA's in-game GUIs does, by its name without the side prefix.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    OrdersTab,
    BuildTab,
    Prev,
    Next,
    Stop,
    Mode(CommandMode),
    Fire,
    Movement,
    OnOff,
    Cloak,
}

fn action_of(name: &str) -> Option<Action> {
    Some(match name {
        "ORDERS" => Action::OrdersTab,
        "BUILD" => Action::BuildTab,
        "PREV" => Action::Prev,
        "NEXT" => Action::Next,
        "STOP" => Action::Stop,
        "MOVE" => Action::Mode(CommandMode::Move),
        "ATTACK" => Action::Mode(CommandMode::Attack),
        "PATROL" => Action::Mode(CommandMode::Patrol),
        "DEFEND" => Action::Mode(CommandMode::Guard),
        "BLAST" => Action::Mode(CommandMode::DGun),
        "RECLAIM" => Action::Mode(CommandMode::Reclaim),
        "REPAIR" => Action::Mode(CommandMode::Repair),
        "CAPTURE" => Action::Mode(CommandMode::Capture),
        "LOAD" => Action::Mode(CommandMode::Load),
        "UNLOAD" => Action::Mode(CommandMode::Unload),
        "FIREORD" => Action::Fire,
        "MOVEORD" => Action::Movement,
        "ONOFF" => Action::OnOff,
        "CLOAK" => Action::Cloak,
        _ => return None,
    })
}

/// An order's key binding action (the tutorial's guide finds its button by it).
fn mode_action(m: CommandMode) -> Option<&'static str> {
    Some(match m {
        CommandMode::Move => "move",
        CommandMode::Attack => "attack",
        CommandMode::Patrol => "patrol",
        CommandMode::Guard => "guard",
        CommandMode::DGun => "dgun",
        CommandMode::Reclaim => "reclaim",
        CommandMode::Repair => "repair",
        CommandMode::Capture => "capture",
        CommandMode::Load => "load",
        CommandMode::Unload => "unload",
        _ => return None,
    })
}

/// The viewed player's side: art, colours, lettering and bar layout.
struct Style {
    /// Gadget and GUI name prefix (`ARM`: `ARMATTACK`, `armgen.gui`).
    prefix: String,
    /// Every side's prefix: captured builders bring their own side's GUIs.
    prefixes: Vec<String>,
    /// `anims/<intgaf>.gaf`: the bars and the side panel.
    intgaf: String,
    metal: u8,
    energy: u8,
    layout: Option<Arc<SideLayout>>,
    font: Option<Arc<TaFont>>,
    font_gui: Option<Arc<TaFont>>,
    /// Lettering for the buttons drawn without art: the condensed bold face of TA's order
    /// buttons (`hatt12`).
    font_button: Option<Arc<TaFont>>,
    /// The commander's first build page, the layout of pages that have no GUI.
    template: String,
    /// How tall a screen the side panel is laid out for (TA pixels): TA's 480, or as tall as
    /// the side's pages reach (TA: Escalation's and Total Mayhem's, 768).
    height: f32,
}

/// What a frame's drawing needs besides the HUD's own state.
struct Env<'a> {
    ctx: egui::Context,
    vfs: &'a Vfs,
    data: Arc<ta_data::GameData>,
    st: Style,
    /// TA's layout from the top-left corner, and from the bottom-left one (the bottom bar).
    ta: Ta,
    tb: Ta,
}

/// What the selected units can do, and their standing orders: the first capable unit's,
/// and whether the others agree.
struct Selection {
    fire: Option<(u8, bool)>,
    movement: Option<(u8, bool)>,
    active: Option<(bool, bool)>,
    cloak: Option<(bool, bool)>,
    transport: bool,
    /// The first unit is a factory: whether it repeats its queue.
    repeat: Option<bool>,
    /// The first unit's self-destruct countdown is running.
    destructing: bool,
}

fn agree<T: PartialEq + Copy>(mut values: impl Iterator<Item = T>) -> Option<(T, bool)> {
    let first = values.next()?;
    Some((first, values.all(|v| v == first)))
}

impl Selection {
    fn of(game: &Game) -> Selection {
        let sim = &game.sim;
        let units: Vec<(&ta_sim::Unit, &ta_sim::UnitType)> = game.selection.iter().filter_map(|&id| sim.unit(id)).map(|u| (u, sim.unit_type(u))).collect();
        let armed = |t: &ta_sim::UnitType| t.weapons.iter().any(|w| w.is_some());
        let first = units.first();
        Selection {
            fire: agree(units.iter().filter(|(_, t)| armed(t)).map(|(u, _)| u.fire_state)),
            movement: agree(units.iter().filter(|(_, t)| armed(t) && t.locomotion != ta_sim::Locomotion::Static).map(|(u, _)| u.move_state)),
            active: agree(units.iter().filter(|(_, t)| t.def.on_offable).map(|(u, _)| u.activated)),
            cloak: agree(units.iter().filter(|(_, t)| t.def.cloak_cost > 0.0).map(|(u, _)| u.cloak_on)),
            transport: units.iter().any(|(_, t)| t.transport_capacity > 0),
            repeat: first.filter(|(_, t)| t.is_factory).map(|(u, _)| u.factory_repeat),
            destructing: first.is_some_and(|(u, _)| u.self_destruct_at.is_some() && u.self_destruct_manual),
        }
    }
}

/// The first selected factory's queue: how many of each type, and the one in production.
#[derive(Default)]
struct Queue {
    factory: bool,
    counts: HashMap<usize, usize>,
    front: Option<(usize, f32)>,
}

/// A build menu on show: the builder type, the page and how many pages it has.
#[derive(Clone, Copy)]
struct MenuAt {
    builder: usize,
    page: usize,
    pages: usize,
}

#[derive(Default)]
pub struct ClassicHud {
    art: Art,
    /// The side panel shows the build menu rather than the orders page.
    building: bool,
    /// The unit the page was chosen for: a new first selected unit chooses again.
    page_for: Option<UnitId>,
    /// Build menu page shown per builder type.
    build_page: HashMap<usize, usize>,
    /// What the pointer is over in the panels, for the bottom bar.
    tip: Option<Tip>,
    /// The main page's ALLIES button keeps the players panel up.
    allies: bool,
    /// The SHARE button's menu is open.
    share: bool,
    /// The OPTIONS button was clicked: the app opens the game menu.
    pub open_menu: bool,
    /// TA's shortcut keys pressed this frame, the keys the page being drawn answers to, and
    /// those the page drawn last answered to.
    keys_pressed: Vec<KeyCode>,
    claiming: Vec<KeyCode>,
    claimed: Vec<KeyCode>,
    /// Where the panel drew what a lesson may point at, this frame.
    pub anchors: crate::hud::anchors::Anchors,
    /// How TA's art and lettering are enlarged (the setting `gameplay.classic_pixels`).
    pub look: Look,
    /// The layout's height last drawn ([`Style::height`]; 0 before the first frame: TA's).
    height: f32,
}

/// The selection's build menu: all of one type that builds something.
fn builder(game: &Game) -> Option<usize> {
    let u = game.sim.unit(*game.selection.first()?)?;
    let t = game.sim.unit_type(u);
    let same = game.selection.iter().all(|&id| game.sim.unit(id).is_some_and(|o| o.type_id == u.type_id));
    (same && !t.build_options.is_empty()).then_some(u.type_id)
}

fn full_uv() -> Rect {
    Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0))
}

/// A box of sidedata.tdf with its far edge included (`x2`/`y2` are inclusive).
fn inclusive(r: Rect) -> Rect {
    Rect::from_min_max(r.min, r.max + vec2(1.0, 1.0))
}

/// Art over `r` (TA pixels), made for the screen pixels it covers there.
fn fill(painter: &egui::Painter, sprite: &Sprite, ta: Ta, r: Rect, tint: Color32) {
    let texture = sprite.texture(painter.ctx(), ta.pixels(r.size()), ta.look);
    painter.image(texture, ta.snap(r), full_uv(), tint);
}

/// The part `uv` of art at its own size, over `r` (TA pixels).
fn part(painter: &egui::Painter, sprite: &Sprite, ta: Ta, r: Rect, uv: Rect, tint: Color32) {
    let texture = sprite.texture(painter.ctx(), ta.pixels(sprite.size), ta.look);
    painter.image(texture, ta.snap(r), uv, tint);
}

/// Art at its own size with its top-left corner at `at` (TA pixels).
fn blit(painter: &egui::Painter, sprite: &Sprite, ta: Ta, at: Pos2, tint: Color32) {
    part(painter, sprite, ta, Rect::from_min_size(at, sprite.size), full_uv(), tint);
}

/// Bar art at `at`, then its last `tile` columns repeated out to TA x `end`: windows wider
/// than 4:3 have more bar than TA's 640 pixels.
fn strip(painter: &egui::Painter, sprite: &Sprite, ta: Ta, at: Pos2, end: f32, tile: f32) {
    blit(painter, sprite, ta, at, Color32::WHITE);
    let w = sprite.size.x;
    let uv = Rect::from_min_max(pos2((w - tile) / w, 0.0), pos2(1.0, 1.0));
    let mut x = at.x + w;
    while x < end {
        part(painter, sprite, ta, Rect::from_min_size(pos2(x, at.y), vec2(tile, sprite.size.y)), uv, Color32::WHITE);
        x += tile;
    }
}

#[derive(Clone, Copy)]
enum Align {
    Left,
    Center,
    Right,
}

/// Text at TA position (`x` per `align`, `y` the top), whole TA pixels apart like TA's.
#[allow(clippy::too_many_arguments)]
fn text(painter: &egui::Painter, font: &TaFont, ta: Ta, x: f32, y: f32, align: Align, text: &str, color: Color32) {
    let w = font.width(text);
    let x = match align {
        Align::Left => x,
        Align::Center => x - w / 2.0,
        Align::Right => x - w,
    };
    font.draw_shadowed(painter, ta.pos(x.round(), y.round()), ta.s, text, color, ta.look);
}

/// A bar of `frac` in rows of the palette ramp starting at `color` (TA's shaded bars): with
/// the original pixels a band of the screen to each row; smooth, the ramp's colours shaded from
/// one row's middle to the next. Its end on the nearest screen pixel, not TA's.
fn bar(painter: &egui::Painter, ta: Ta, r: Rect, frac: f32, palette: &ta_formats::image::Palette, color: u8) {
    let w = r.width() * frac.clamp(0.0, 1.0);
    if w <= 0.0 {
        return;
    }
    let rows = r.height().round().max(1.0) as usize;
    let shade = |row: usize| {
        let [red, g, b] = palette.rgb(color.saturating_add(row.min(255) as u8));
        Color32::from_rgb(red, g, b)
    };
    match ta.look {
        Look::Pixels => {
            for row in 0..rows {
                painter.rect_filled(ta.snap(Rect::from_min_size(pos2(r.min.x, r.min.y + row as f32), vec2(w, 1.0))), 0.0, shade(row));
            }
        }
        Look::Smooth => {
            let outer = ta.snap(Rect::from_min_size(r.min, vec2(w, rows as f32)));
            let mut stops = vec![(outer.top(), shade(0))];
            stops.extend((0..rows).map(|row| (ta.pos(0.0, r.min.y + row as f32 + 0.5).y, shade(row))));
            stops.push((outer.bottom(), shade(rows - 1)));
            let mut mesh = egui::Mesh::default();
            for (i, &(y, c)) in stops.iter().enumerate() {
                mesh.colored_vertex(pos2(outer.left(), y), c);
                mesh.colored_vertex(pos2(outer.right(), y), c);
                if i > 0 {
                    let k = 2 * i as u32;
                    mesh.add_triangle(k - 2, k - 1, k);
                    mesh.add_triangle(k - 1, k + 1, k);
                }
            }
            painter.add(egui::Shape::mesh(mesh));
        }
    }
}

/// Health in TA's colours: green, yellow when hurt, red when nearly gone.
fn health_color(frac: f32) -> u8 {
    if frac > 0.66 {
        232
    } else if frac > 0.33 {
        208
    } else {
        211
    }
}

/// logos.gaf entry of a side's emblem (ten frames, one per player colour).
fn logo_entry(side: &str) -> &'static str {
    let side = side.to_ascii_uppercase();
    if side.starts_with("ARM") {
        "Arm32Lt"
    } else if side.starts_with("COR") {
        "Core32Lt"
    } else {
        "32xlogos"
    }
}

/// "12.5", "250": rates keep a decimal while small.
fn rate(v: f32) -> String {
    if v.abs() < 100.0 { format!("{v:.1}") } else { format!("{v:.0}") }
}

fn cycle(v: u8, n: u8, back: bool) -> u8 {
    if back { (v + n - 1) % n } else { (v + 1) % n }
}

/// A unit's resource flows per second as the unit bar shows them (metal made, metal used,
/// energy made, energy used): see [`ta_sim::Sim::unit_flows`].
fn flows(sim: &ta_sim::Sim, u: &ta_sim::Unit) -> [f32; 4] {
    let f = sim.unit_flows(u);
    [f.metal_make.to_f32(), f.metal_use.to_f32(), f.energy_make.to_f32(), f.energy_use.to_f32()]
}

impl ClassicHud {
    fn art(&mut self, env: &Env, gaf: &str, entry: &str) -> Option<Frames> {
        self.art.frames(env.vfs, &env.data.palette, gaf, entry)
    }

    fn style(&mut self, ctx: &egui::Context, vfs: &Vfs, game: &Game) -> Style {
        let player = game.sim.players.get(game.hud_player() as usize);
        let side_name = player.map_or("ARM", |p| p.side.as_str());
        let sides = &game.sim.data.sides;
        let side = sides.iter().find(|s| s.name.eq_ignore_ascii_case(side_name)).or(sides.first());
        let prefix = side.map_or("ARM".to_string(), |s| s.name_prefix.to_ascii_uppercase());
        let intgaf = side.map(|s| s.interface_gaf.to_ascii_lowercase()).filter(|g| !g.is_empty()).unwrap_or_else(|| format!("{}int", prefix.to_ascii_lowercase()));
        let layout = self.art.side_layout(vfs, side_name);
        let font = layout.as_ref().and_then(|l| self.art.font(ctx, vfs, &l.font));
        let font_gui = layout.as_ref().and_then(|l| self.art.font(ctx, vfs, &l.font_gui)).or_else(|| font.clone());
        let font_button = self.art.font(ctx, vfs, "hatt12").or_else(|| font_gui.clone());
        let template = side.map_or("armcom1".to_string(), |s| format!("{}1", s.commander.to_ascii_lowercase()));
        let height = self.art.layout_height(vfs, &prefix, &template);
        Style {
            prefixes: sides.iter().map(|s| s.name_prefix.to_ascii_uppercase()).filter(|p| !p.is_empty()).collect(),
            intgaf,
            metal: side.map_or(224, |s| s.metal_color),
            energy: side.map_or(208, |s| s.energy_color),
            layout,
            font,
            font_gui,
            font_button,
            template,
            height,
            prefix,
        }
    }

    /// TA's shortcut keys pressed this frame (letters, no Ctrl or Alt; none while a menu or
    /// the chat line has the keyboard), for the page's gadgets.
    pub fn set_keys(&mut self, pressed: Vec<KeyCode>) {
        self.keys_pressed = pressed;
    }

    /// The keys the page shown last frame answers to (the rebindable keys stand down for them).
    pub fn claimed_keys(&self) -> Vec<KeyCode> {
        self.claimed.clone()
    }

    /// Whether a gadget's key was pressed, noting that the page answers to it.
    fn key_hit(&mut self, key: Option<char>) -> bool {
        let Some(k) = key.and_then(crate::keys::letter_key) else { return false };
        if !self.claiming.contains(&k) {
            self.claiming.push(k);
            self.keys_pressed.contains(&k)
        } else {
            // (A second gadget on the page with the same key: the first has it.)
            false
        }
    }

    /// The layout's height (TA pixels): TA's 480 until a frame has been drawn.
    fn layout_height(&self) -> f32 {
        self.height.max(TA_HEIGHT)
    }

    /// The part of the screen the panels leave to the battlefield.
    pub fn view_rect(&self, screen: Rect) -> Rect {
        view_rect(screen, self.layout_height())
    }

    /// Where TA's PAUSED title ends, in points from the screen's top.
    pub fn paused_title_bottom(&self, screen: Rect) -> f32 {
        paused_title_bottom(screen, self.layout_height())
    }

    /// Draws the three panels, the pause title and the popups of the main page.
    pub fn draw(&mut self, root: &mut egui::Ui, vfs: &Vfs, game: &mut Game, camera: &mut Camera, minimap: &mut Minimap) {
        self.claiming.clear();
        self.anchors.clear(true);
        let ctx = root.ctx().clone();
        let screen = ctx.content_rect();
        let st = self.style(&ctx, vfs, game);
        self.height = st.height;
        let s = scale(screen, st.height);
        let (ppp, look) = (ctx.pixels_per_point(), self.look);
        let env = Env {
            ctx: ctx.clone(),
            vfs,
            data: game.sim.data.clone(),
            st,
            ta: Ta { origin: screen.min, s, ppp, look },
            tb: Ta { origin: pos2(screen.min.x, screen.max.y - TA_HEIGHT * s), s, ppp, look },
        };
        self.anchors.side_panel = Some(Rect::from_min_max(screen.min, pos2(screen.min.x + SIDE_W * s, screen.max.y)));
        // A newly selected unit opens its build menu if it has one.
        let first = game.selection.first().copied();
        if first != self.page_for {
            self.page_for = first;
            self.building = builder(game).is_some();
        }
        self.tip = None;
        let panel = |p: egui::Panel, size: f32| p.exact_size(size).resizable(false).show_separator_line(false).frame(egui::Frame::new());
        panel(egui::Panel::left("classic_side"), SIDE_W * s).show(root, |ui| self.side_panel(ui, &env, game, camera, minimap));
        panel(egui::Panel::top("classic_top"), TOP_H * s).show(root, |ui| self.top_bar(ui, &env, game));
        panel(egui::Panel::bottom("classic_bottom"), BOTTOM_H * s).show(root, |ui| self.bottom_bar(ui, &env, game));
        if game.paused {
            self.paused(&env, self.view_rect(screen));
        }
        if self.allies && !game.show_players {
            crate::hud::players_panel(&ctx, game, [self.view_rect(screen).center().x - screen.center().x, TOP_H * s + 36.0], &[]);
        }
        if self.share {
            self.share_menu(&env, game);
        }
        self.claimed = std::mem::take(&mut self.claiming);
        self.keys_pressed.clear();
    }

    fn paused(&mut self, env: &Env, view: Rect) {
        let painter = env.ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("classic_paused")));
        let at = |w: f32| pos2(view.center().x - w * env.ta.s / 2.0, view.top() + PAUSED_AT * env.ta.s);
        match self.art(env, "igtitles", "igpaused") {
            Some(f) => {
                // (In TA pixels from the layout's corner, where the title's middle falls.)
                let corner = (at(f[0].size.x) - env.ta.origin) / env.ta.s;
                blit(&painter, &f[0], env.ta, corner.to_pos2(), Color32::WHITE);
            }
            None => {
                if let Some(font) = &env.st.font {
                    font.draw_shadowed(&painter, at(font.width("PAUSED")), env.ta.s, "PAUSED", TEXT, env.ta.look);
                }
            }
        }
    }

    /// The side's `gen` (orders) or `main` page; sides without their own use Arm's.
    fn side_gui(&mut self, env: &Env, page: &str) -> Option<Arc<Gui>> {
        let own = format!("{}{page}", env.st.prefix.to_ascii_lowercase());
        self.art.gui(env.vfs, &own).or_else(|| self.art.gui(env.vfs, &format!("arm{page}")))
    }

    fn share_menu(&mut self, env: &Env, game: &mut Game) {
        let mut gift = None;
        let mut close = false;
        let at = env.ta.pos(SIDE_W + 4.0, 128.0 + 228.0);
        egui::Area::new(egui::Id::new("classic_share")).fixed_pos(at).order(egui::Order::Foreground).show(&env.ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                gift = crate::hud::share_menu(ui, game);
                close = ui.button(ta_ui::i18n::t("Close")).clicked();
            });
        });
        match gift {
            Some(crate::hud::Gift::Resources(to, metal, energy)) => game.share(to, metal, energy),
            Some(crate::hud::Gift::Units(to)) => {
                game.give_selection(to);
                close = true;
            }
            None => {}
        }
        if close || game.allies().is_empty() {
            self.share = false;
        }
    }

    // ---- Side panel -------------------------------------------------------------------

    fn side_panel(&mut self, ui: &mut egui::Ui, env: &Env, game: &mut Game, camera: &mut Camera, minimap: &mut Minimap) {
        let painter = ui.painter().clone();
        match self.art(env, &env.st.intgaf, "PANELSIDE") {
            Some(f) => {
                // (As tall as the panel: TA's 480 pixels, or more in a window narrower than 4:3.)
                fill(&painter, &f[0], env.ta, Rect::from_min_size(Pos2::ZERO, vec2(f[0].size.x, ui.max_rect().height() / env.ta.s)), Color32::WHITE);
            }
            None => {
                painter.rect_filled(ui.max_rect(), 0.0, Color32::from_rgb(12, 13, 17));
            }
        }
        // The minimap in TA's 128×128 corner, the map's shape kept.
        let map = vec2(game.map.width_px() as f32, game.map.height_px() as f32).max(vec2(1.0, 1.0));
        let fit = map * ((SIDE_W - 2.0) / map.x.max(map.y));
        let r = Rect::from_min_size(env.ta.pos((SIDE_W - fit.x) / 2.0, (SIDE_W - fit.y) / 2.0), fit * env.ta.s);
        let mut mini = ui.new_child(egui::UiBuilder::new().max_rect(r));
        minimap.draw(&mut mini, game, camera, r.width(), f32::INFINITY);
        if game.selection.is_empty() {
            self.main_page(ui, env, game);
            return;
        }
        let menu = builder(game).filter(|_| self.building);
        let (gui_name, menu_at, page) = match menu {
            Some(b) => {
                let pages = self.art.menu(env.vfs, &game.sim.types, b, &env.st.template);
                let p = self.build_page.get(&b).copied().unwrap_or(0).min(pages.len().saturating_sub(1));
                self.anchors.pages(pages.iter().enumerate().flat_map(|(k, pg)| pg.slots.iter().flatten().map(move |&o| (o, k))), p);
                let page = pages.get(p).map(|pg| (pg.gui.clone(), pg.slots.clone()));
                match page {
                    Some((gui, slots)) => (gui, Some(MenuAt { builder: b, page: p, pages: pages.len() }), Some(slots)),
                    None => (format!("{}gen", env.st.prefix.to_ascii_lowercase()), None, None),
                }
            }
            None => (format!("{}gen", env.st.prefix.to_ascii_lowercase()), None, None),
        };
        let Some(gui) = self.art.gui(env.vfs, &gui_name).or_else(|| self.side_gui(env, "gen")) else { return };
        self.page(ui, env, game, &gui, &gui_name, menu_at.zip(page.as_deref()));
    }

    /// A GUI page: its panel, its gadgets, and extra buttons in the places left free.
    fn page(&mut self, ui: &egui::Ui, env: &Env, game: &mut Game, gui: &Gui, gui_name: &str, menu: Option<(MenuAt, &[Option<usize>])>) {
        let painter = ui.painter().clone();
        if let Some(f) = self.art(env, "commongui", &gui.panel) {
            blit(&painter, &f[0], env.ta, gui.origin, Color32::WHITE);
        }
        let sel = Selection::of(game);
        let queue = self.queue(game);
        let (mut shown, mut small, mut wide) = (Vec::new(), Vec::new(), Vec::new());
        for (i, g) in gui.gadgets.iter().enumerate() {
            if g.kind != 1 {
                continue;
            }
            let rect = g.rect.translate(gui.origin.to_vec2());
            let id = ui.id().with(("gadget", i));
            if g.attribs & 32 != 0 {
                if let Some((_, slots)) = menu {
                    self.build_button(ui, env, game, rect, id, slots.get(gui.grid.place_of(g.rect)).copied().flatten(), gui_name, &queue);
                }
                continue;
            }
            let name = env.st.prefixes.iter().find_map(|p| g.name.strip_prefix(p.as_str())).unwrap_or(g.name.as_str());
            let Some(action) = action_of(name) else { continue };
            if self.gadget(ui, env, game, &sel, g, rect, id, action, menu.map(|m| m.0)) {
                shown.push(rect);
            } else if g.rect.width() >= 100.0 {
                wide.push(rect);
            } else {
                small.push(rect);
            }
        }
        // Places of hidden gadgets another gadget doesn't cover.
        small.retain(|r| !shown.iter().any(|s| s.intersects(r.shrink(1.0))));
        wide.retain(|r| !shown.iter().any(|s| s.intersects(r.shrink(1.0))));
        self.extras(ui, env, game, &sel, small, wide);
    }

    fn queue(&self, game: &Game) -> Queue {
        let Some(u) = game.selection.first().and_then(|&id| game.sim.unit(id)) else { return Queue::default() };
        if !game.sim.unit_type(u).is_factory {
            return Queue::default();
        }
        let mut counts = HashMap::new();
        for &t in &u.factory_queue {
            *counts.entry(t).or_default() += 1;
        }
        let progress = u.build.and_then(|b| game.sim.unit(b.target)).map_or(0.0, |b| b.build_progress.to_f32().clamp(0.0, 1.0));
        Queue { factory: true, counts, front: u.factory_queue.front().map(|&t| (t, progress)) }
    }

    /// One of the GUIs' standard gadgets, doing what it says when clicked. Returns whether
    /// it shows: orders the selection can't carry out leave their place free.
    #[allow(clippy::too_many_arguments)]
    fn gadget(&mut self, ui: &egui::Ui, env: &Env, game: &mut Game, sel: &Selection, g: &Gadget, rect: Rect, id: egui::Id, action: Action, menu: Option<MenuAt>) -> bool {
        let (shown, enabled) = match action {
            Action::OrdersTab | Action::Stop => (true, true),
            Action::BuildTab => (true, builder(game).is_some()),
            Action::Prev | Action::Next => (menu.is_some(), menu.is_some_and(|m| m.pages > 1)),
            // Transports keep UNLOAD, greyed while empty.
            Action::Mode(CommandMode::Unload) => (sel.transport || game.mode_available(CommandMode::Unload), game.mode_available(CommandMode::Unload)),
            Action::Mode(m) => (game.mode_available(m), true),
            Action::Fire => (sel.fire.is_some(), true),
            Action::Movement => (sel.movement.is_some(), true),
            Action::OnOff => (sel.active.is_some(), true),
            Action::Cloak => (sel.cloak.is_some(), true),
        };
        if !shown {
            return false;
        }
        let Some(frames) = self.art(env, "commongui", &g.name) else { return false };
        let resp = ui.interact(env.ta.rect(rect), id, Sense::click());
        // (For the tutorial's guide: the way to the build menu, its pages, the orders.)
        match action {
            Action::BuildTab if enabled && !self.building => self.anchors.build_page(resp.rect),
            Action::Prev if enabled => self.anchors.pager(Some(resp.rect), None),
            Action::Next if enabled => self.anchors.pager(None, Some(resp.rect)),
            Action::Fire => self.anchors.order("fire_state", resp.rect),
            Action::Mode(m) if enabled => {
                if let Some(a) = mode_action(m) {
                    self.anchors.order(a, resp.rect);
                }
            }
            _ => {}
        }
        let down = enabled && resp.is_pointer_button_down_on();
        let n = frames.len();
        let frame = match action {
            // State buttons: a frame per state, then "mixed", pressed and greyed.
            Action::Fire | Action::Movement | Action::OnOff | Action::Cloak => {
                let states = n.saturating_sub(3).max(1);
                let value = match action {
                    Action::Fire => sel.fire.map(|(v, same)| (v as usize, same)),
                    Action::Movement => sel.movement.map(|(v, same)| (v as usize, same)),
                    Action::OnOff => sel.active.map(|(v, same)| (v as usize, same)),
                    _ => sel.cloak.map(|(v, same)| (v as usize, same)),
                };
                if down {
                    states + 1
                } else {
                    match value {
                        Some((v, true)) => v.min(states - 1),
                        _ => states,
                    }
                }
            }
            // Buttons: up, down, greyed. Tabs and pending orders stay down.
            _ => {
                let on = match action {
                    Action::OrdersTab => !self.building || builder(game).is_none(),
                    Action::BuildTab => self.building && builder(game).is_some(),
                    Action::Mode(m) => game.command_mode == Some(m),
                    _ => false,
                };
                if !enabled {
                    2
                } else if down || on {
                    1
                } else {
                    0
                }
            }
        };
        blit(ui.painter(), &frames[frame.min(n - 1)], env.ta, rect.min, Color32::WHITE);
        if resp.hovered() {
            self.tip = Some(action_tip(action, game, menu));
        }
        if !enabled {
            return true;
        }
        // Its key (TA's quickkey) presses it too; state buttons step back on a right-click.
        let key = self.key_hit(g.quickkey);
        let back = resp.secondary_clicked();
        if !resp.clicked() && !key && !(back && matches!(action, Action::Fire | Action::Movement | Action::OnOff | Action::Cloak)) {
            return true;
        }
        match action {
            Action::OrdersTab => self.building = false,
            Action::BuildTab => self.building = true,
            Action::Prev | Action::Next => {
                if let Some(m) = menu {
                    let page = if action == Action::Next { (m.page + 1) % m.pages } else { (m.page + m.pages - 1) % m.pages };
                    self.build_page.insert(m.builder, page);
                }
            }
            Action::Stop => {
                game.stop_selection();
                game.command_mode = None;
            }
            Action::Mode(m) => game.set_command_mode(if game.command_mode == Some(m) { None } else { Some(m) }),
            Action::Fire => {
                if let Some((v, _)) = sel.fire {
                    game.set_states(Some(cycle(v, 3, back)), None);
                }
            }
            Action::Movement => {
                if let Some((v, _)) = sel.movement {
                    game.set_states(None, Some(cycle(v, 3, back)));
                }
            }
            Action::OnOff => {
                if let Some((on, _)) = sel.active {
                    game.set_active(!on);
                }
            }
            Action::Cloak => game.toggle_cloak(),
        }
        true
    }

    /// A build menu button: the unit's art, or the empty-place patch. Builders pick what to
    /// place; factories queue one (Shift: five) and a right-click takes one off.
    #[allow(clippy::too_many_arguments)]
    fn build_button(&mut self, ui: &egui::Ui, env: &Env, game: &mut Game, rect: Rect, id: egui::Id, unit: Option<usize>, gui_name: &str, queue: &Queue) {
        let painter = ui.painter().clone();
        let ta = env.ta;
        let Some(unit) = unit else {
            if let Some(f) = self.art(env, "commongui", "IGPATCH") {
                blit(&painter, &f[0], ta, rect.min, Color32::WHITE);
            }
            return;
        };
        let name = game.sim.types.get(unit).name.clone();
        let frames = self.art.unit_button(env.vfs, &env.data.palette, gui_name, &name);
        let allowed = game.sim.is_allowed(unit);
        let resp = ui.interact(ta.rect(rect), id, Sense::click());
        self.anchors.picture(unit, resp.rect);
        let placing = game.placement.as_ref().is_some_and(|p| p.type_id == unit);
        let down = allowed && (resp.is_pointer_button_down_on() || placing);
        let square = Rect::from_min_size(rect.min, vec2(64.0, 64.0));
        let place = ta.snap(square);
        match &frames {
            Some(f) if f.len() >= 3 => blit(&painter, &f[if !allowed { 2 } else if down { 1 } else { 0 }], ta, rect.min, Color32::WHITE),
            Some(f) => {
                fill(&painter, &f[0], ta, square, if allowed { Color32::WHITE } else { Color32::from_gray(90) });
                if down {
                    painter.rect_filled(place, 0.0, Color32::from_white_alpha(36));
                }
            }
            None => {
                painter.rect_filled(place, 0.0, Color32::from_gray(34));
                if let Some(font) = &env.st.font_gui {
                    text(&painter, font, ta, rect.min.x + 32.0, rect.min.y + 26.0, Align::Center, &name, DIM);
                }
            }
        }
        // Factories: how many are queued, and how far the one in production is.
        if let (Some(&n), Some(font)) = (queue.counts.get(&unit), &env.st.font_gui) {
            text(&painter, font, ta, rect.min.x + 4.0, rect.min.y + 3.0, Align::Left, &n.to_string(), Color32::WHITE);
        }
        if let Some((_, done)) = queue.front.filter(|&(q, _)| q == unit) {
            let r = Rect::from_min_size(pos2(rect.min.x + 2.0, rect.min.y + 59.0), vec2(60.0, 2.0));
            painter.rect_filled(ta.snap(r), 0.0, Color32::from_black_alpha(170));
            bar(&painter, ta, Rect::from_min_size(r.min, vec2(60.0, 2.0)), done, &env.data.palette, 225);
        }
        if resp.hovered() {
            self.tip = Some(Tip::Unit(unit));
        }
        let facts = crate::hud::unit_facts(&game.sim, game.sim.types.get(unit));
        let resp = if facts.is_empty() { resp } else { resp.on_hover_text(facts.join("\n")) };
        if !allowed {
            return;
        }
        if resp.clicked() {
            let shift = ta_ui::widgets::click_modifiers(&resp, egui::PointerButton::Primary).shift;
            if queue.factory {
                game.produce(unit, if shift { 5 } else { 1 });
            } else {
                game.begin_placement(unit);
            }
        }
        if resp.secondary_clicked() && queue.factory {
            let shift = ta_ui::widgets::click_modifiers(&resp, egui::PointerButton::Secondary).shift;
            game.produce(unit, if shift { -5 } else { -1 });
        }
    }

    /// A button TA has no art for: a face made like its order buttons, lettered in their
    /// style (the first label that fits).
    #[allow(clippy::too_many_arguments)]
    fn plate_button(&mut self, ui: &egui::Ui, env: &Env, rect: Rect, id: egui::Id, labels: &[&str], on: bool, enabled: bool) -> egui::Response {
        let painter = ui.painter().clone();
        let ta = env.ta;
        let resp = ui.interact(ta.rect(rect), id, Sense::click());
        let down = enabled && (on || resp.is_pointer_button_down_on());
        // The art of TA's buttons is a pixel smaller than their gadgets.
        let face = Rect::from_min_size(rect.min, rect.size() - vec2(1.0, 1.0));
        let plate = self.art.plate(face.width() as u32, face.height() as u32);
        fill(&painter, &plate, ta, face, if enabled { Color32::WHITE } else { Color32::from_gray(150) });
        // Pressed (or on), lit up like TA's down frames.
        if down {
            painter.rect_filled(ta.snap(face.shrink(1.0)), 0.0, Color32::from_white_alpha(44));
        }
        if let Some(font) = &env.st.font_button {
            let label = labels.iter().map(|l| t(l)).find(|l| font.width(l) <= face.width() - 6.0).or(labels.last().map(|l| t(l))).unwrap_or("");
            let nudge = if down { 1.0 } else { 0.0 };
            let color = if !enabled {
                Color32::from_gray(110)
            } else if down {
                Color32::from_rgb(255, 255, 215)
            } else {
                BUTTON_TEXT
            };
            text(&painter, font, ta, face.center().x + nudge, face.center().y - font.height / 2.0 + nudge, Align::Center, label, color);
        }
        resp
    }

    /// A two-state toggle in a state button's place, on the side's ON/OFF art: its blank
    /// face, the light of the state and our label.
    fn wide_toggle(&mut self, ui: &egui::Ui, env: &Env, rect: Rect, id: egui::Id, labels: &[&str], on: bool) -> egui::Response {
        let Some(f) = self.art(env, "commongui", &format!("{}ONOFF", env.st.prefix)).filter(|f| f.len() >= 4) else {
            return self.plate_button(ui, env, rect, id, labels, on, true);
        };
        let painter = ui.painter().clone();
        let ta = env.ta;
        let resp = ui.interact(ta.rect(rect), id, Sense::click());
        blit(&painter, &f[3], ta, rect.min, Color32::WHITE);
        // The lights sit right of x = 90 (the first lit is OFF, the second ON).
        let lights = &f[if on { 1 } else { 0 }];
        let (x0, w) = (90.0, lights.size.x);
        let r = Rect::from_min_size(pos2(rect.min.x + x0, rect.min.y), vec2(w - x0, lights.size.y));
        part(&painter, lights, ta, r, Rect::from_min_max(pos2(x0 / w, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        if let Some(font) = &env.st.font_button {
            let label = labels.iter().map(|l| t(l)).find(|l| font.width(l) <= x0 - 8.0).or(labels.last().map(|l| t(l))).unwrap_or("");
            let nudge = if resp.is_pointer_button_down_on() { 1.0 } else { 0.0 };
            text(&painter, font, ta, rect.min.x + 5.0 + nudge, rect.center().y - font.height / 2.0 + nudge, Align::Left, label, BUTTON_TEXT);
        }
        resp
    }

    /// Commands TA's GUIs have no gadget for, in the page's free places: the order ones
    /// where order buttons would be (bottom right first), the toggles where state buttons
    /// would be (top first) or else in order places.
    fn extras(&mut self, ui: &egui::Ui, env: &Env, game: &mut Game, sel: &Selection, mut small: Vec<Rect>, mut wide: Vec<Rect>) {
        // Sorted so that `pop` takes the bottom-right order place and the top state place;
        // gadgets sharing a place (LOAD and BLAST) leave it once.
        small.sort_by(|a, b| a.min.y.total_cmp(&b.min.y).then(a.min.x.total_cmp(&b.min.x)));
        wide.sort_by(|a, b| b.min.y.total_cmp(&a.min.y));
        small.dedup_by(|a, b| a.intersects(b.shrink(1.0)));
        wide.dedup_by(|a, b| a.intersects(b.shrink(1.0)));
        let key = |game: &Game, action: &str| game.keys.text(action).to_string();
        for (mode, labels, what, action) in [
            (CommandMode::Fight, &[n("FIGHT")][..], n("Attack-move: head for a place, fighting on the way"), "fight"),
            (CommandMode::Resurrect, &[n("RESURRECT"), n("REVIVE")][..], n("Raise a wreck back into the unit it was"), "resurrect"),
        ] {
            if !game.mode_available(mode) {
                continue;
            }
            let Some(r) = small.pop() else { continue };
            let on = game.command_mode == Some(mode);
            let resp = self.plate_button(ui, env, r, ui.id().with(("extra", mode.label())), labels, on, true);
            self.anchors.order(action, resp.rect);
            if resp.hovered() {
                self.tip = Some(Tip::Text(ta_ui::i18n::upper(t(mode.label_for(game))), format!("{} ({})", t(what), key(game, action))));
            }
            if resp.clicked() {
                game.set_command_mode(if on { None } else { Some(mode) });
            }
        }
        if let Some(on) = sel.repeat {
            let labels = [n("REPEAT BUILD"), n("REPEAT")];
            let resp = match (wide.pop(), small.pop()) {
                (Some(r), unused) => {
                    small.extend(unused);
                    Some(self.wide_toggle(ui, env, r, ui.id().with("extra_repeat"), &labels, on))
                }
                (None, Some(r)) => Some(self.plate_button(ui, env, r, ui.id().with("extra_repeat"), &labels, on, true)),
                (None, None) => None,
            };
            if let Some(resp) = resp {
                self.anchors.repeat = Some(resp.rect);
                if resp.hovered() {
                    self.tip = Some(Tip::Text(n("REPEAT").into(), n("Finished units go back on the end of the queue").into()));
                }
                if resp.clicked() {
                    game.set_repeat(!on);
                }
            }
        }
        let on = sel.destructing;
        let labels = [n("SELF-DESTRUCT"), n("SELF-D")];
        let resp = match (small.pop(), wide.pop()) {
            (Some(r), _) => Some(self.plate_button(ui, env, r, ui.id().with("extra_destruct"), &labels, on, true)),
            (None, Some(r)) => Some(self.wide_toggle(ui, env, r, ui.id().with("extra_destruct"), &labels, on)),
            (None, None) => None,
        };
        if let Some(resp) = resp {
            if resp.hovered() {
                self.tip = Some(Tip::Text(n("SELF-DESTRUCT").into(), format!("{} ({})", t("Blow up after a five-second countdown; again to cancel"), key(game, "self_destruct"))));
            }
            if resp.clicked() {
                game.self_destruct_selection();
            }
        }
    }

    /// Nothing selected: the side's main page with the score and the ALLIES, SHARE and
    /// OPTIONS buttons.
    fn main_page(&mut self, ui: &egui::Ui, env: &Env, game: &mut Game) {
        let Some(gui) = self.side_gui(env, "main") else { return };
        let painter = ui.painter().clone();
        if let Some(f) = self.art(env, "commongui", &gui.panel) {
            blit(&painter, &f[0], env.ta, gui.origin, Color32::WHITE);
        }
        let Some(p) = game.sim.players.get(game.hud_player() as usize) else { return };
        let (kills, lost, units) = (p.stats.kills, p.stats.lost, p.unit_count);
        let limit = game.sim.unit_limit;
        let allies = !game.allies().is_empty();
        for (i, g) in gui.gadgets.iter().enumerate() {
            let rect = g.rect.translate(gui.origin.to_vec2());
            let id = ui.id().with(("main", i));
            match (g.kind, g.name.as_str()) {
                (5, label @ ("KILLS" | "LOSSES" | "TOTALUNITS")) => {
                    let line = match label {
                        "KILLS" => tf("Kills: {n}", &[("n", &kills)]),
                        "LOSSES" => tf("Losses: {n}", &[("n", &lost)]),
                        _ if limit < u32::MAX => tf("Units: {n}/{limit}", &[("n", &units), ("limit", &limit)]),
                        _ => tf("Units: {n}", &[("n", &units)]),
                    };
                    if let Some(font) = &env.st.font_gui {
                        text(&painter, font, env.ta, rect.min.x, rect.min.y, Align::Left, &line, BUTTON_TEXT);
                    }
                }
                (1, "ALLIES") => {
                    let resp = self.plate_button(ui, env, rect, id, &[g.text.as_str(), n("Allies")], self.allies, true);
                    if resp.hovered() {
                        self.tip = Some(Tip::Text(n("ALLIES").into(), tf("Every player at a glance (or hold {key})", &[("key", &game.keys.text("players"))])));
                    }
                    if resp.clicked() || self.key_hit(g.quickkey) {
                        self.allies = !self.allies;
                    }
                }
                (1, "SHARE") => {
                    let resp = self.plate_button(ui, env, rect, id, &[g.text.as_str(), n("Share")], self.share, allies);
                    if resp.hovered() {
                        self.tip = Some(Tip::Text(n("SHARE").into(), if allies { n("Send your allies metal, energy or units") } else { n("You have no allies to share with") }.into()));
                    }
                    if (resp.clicked() || self.key_hit(g.quickkey)) && allies {
                        self.share = !self.share;
                    }
                }
                (1, "OPTIONS") => {
                    let resp = self.plate_button(ui, env, rect, id, &[if g.text.is_empty() { n("Options Menu") } else { g.text.as_str() }, n("Options")], false, true);
                    if resp.hovered() {
                        self.tip = Some(Tip::Text(n("OPTIONS").into(), n("The game menu: options, saving, leaving").into()));
                    }
                    if resp.clicked() || self.key_hit(g.quickkey) {
                        self.open_menu = true;
                    }
                }
                _ => {}
            }
        }
    }

    // ---- Top bar ----------------------------------------------------------------------

    fn top_bar(&mut self, ui: &mut egui::Ui, env: &Env, game: &mut Game) {
        let painter = ui.painter().clone();
        let ta = env.ta;
        let end = (ui.max_rect().right() - ta.origin.x) / ta.s;
        // TA's bar starts a pixel into the side panel.
        match self.art(env, &env.st.intgaf, "PANELTOP") {
            Some(f) => strip(&painter, &f[0], ta, pos2(SIDE_W - 1.0, 0.0), end, 24.0),
            None => {
                painter.rect_filled(ui.max_rect(), 0.0, Color32::from_rgb(12, 13, 17));
            }
        }
        let (Some(l), Some(font)) = (env.st.layout.clone(), env.st.font.clone()) else { return };
        let Some(p) = game.sim.players.get(game.hud_player() as usize) else { return };
        if let (Some(b), Some(f)) = (l.get("LOGO"), self.art.frames(env.vfs, &env.data.palette, "logos", logo_entry(&p.side))) {
            fill(&painter, &f[(p.color as usize).min(f.len() - 1)], ta, inclusive(b), Color32::WHITE);
        }
        let e = &p.economy;
        let readouts = [
            ("METAL", e.metal, e.metal_capacity, e.metal_income, e.metal_expense, env.st.metal),
            ("ENERGY", e.energy, e.energy_capacity, e.energy_income, e.energy_expense, env.st.energy),
        ];
        for (name, amount, cap, income, expense, color) in readouts {
            if let Some(b) = l.get(&format!("{name}BAR")).map(inclusive) {
                let frac = if cap > Fx::ZERO { (amount / cap).to_f32() } else { 0.0 };
                bar(&painter, ta, b, frac, &env.data.palette, color);
                // The amount under the bar's middle, the storage over its ends.
                if let Some(r) = l.get(&format!("{name}NUM")) {
                    text(&painter, &font, ta, b.center().x, r.min.y, Align::Center, &format!("{:.0}", amount.to_f32()), TEXT);
                }
                if let Some(r) = l.get(&format!("{name}0")) {
                    text(&painter, &font, ta, r.min.x, r.min.y, Align::Left, "0", DIM);
                }
                // Its right end (the "+" of the art follows close behind).
                if let Some(r) = l.get(&format!("{name}MAX")) {
                    text(&painter, &font, ta, r.min.x, r.min.y, Align::Right, &format!("{:.0}", cap.to_f32()), DIM);
                }
            }
            if let Some(r) = l.get(&format!("{name}PRODUCED")) {
                text(&painter, &font, ta, r.min.x, r.min.y, Align::Left, &rate(income.to_f32()), TEXT);
            }
            if let Some(r) = l.get(&format!("{name}CONSUMED")) {
                text(&painter, &font, ta, r.min.x, r.min.y, Align::Left, &rate(expense.to_f32()), TEXT);
            }
        }
        self.top_extras(ui, env, game, &font, end);
    }

    /// Past TA's 640 pixels, right to left as room allows: the game clock, the unit count,
    /// idle factories and constructors (click selects the next), and the game speed.
    fn top_extras(&mut self, ui: &egui::Ui, env: &Env, game: &mut Game, font: &TaFont, end: f32) {
        let painter = ui.painter().clone();
        let ta = env.ta;
        let secs = game.sim.tick / 30;
        let p = game.sim.players.get(game.hud_player() as usize);
        let units = p.map_or(0, |p| p.unit_count);
        let limit = game.sim.unit_limit;
        let mut items: Vec<(String, Color32, u8)> = vec![(format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60), TEXT, 0)];
        items.push(if limit < u32::MAX {
            (tf("Units {n}/{limit}", &[("n", &units), ("limit", &limit)]), if units >= limit { Color32::from_rgb(255, 130, 90) } else { TEXT }, 0)
        } else {
            (tf("Units {n}", &[("n", &units)]), TEXT, 0)
        });
        let idle_factories = game.idle_factories().len();
        if idle_factories > 0 {
            items.push((tf("Idle factories {n}", &[("n", &idle_factories)]), Color32::from_rgb(255, 200, 110), 1));
        }
        let idle = game.idle_builders().len();
        if idle > 0 {
            items.push((tf("Idle builders {n}", &[("n", &idle)]), Color32::from_rgb(120, 220, 255), 2));
        }
        // (Pausing puts TA's own title over the battlefield.)
        if !game.paused && (game.speed - 1.0).abs() > 0.01 {
            items.push((tf("Speed x{speed}", &[("speed", &format!("{:.2}", game.speed))]), TEXT, 0));
        }
        let y = ((TOP_H - font.height) / 2.0).round();
        let mut x = end - 8.0;
        for (label, color, click) in items {
            let w = font.width(&label);
            if x - w < TA_WIDTH + 6.0 {
                break;
            }
            text(&painter, font, ta, x, y, Align::Right, &label, color);
            if click > 0 {
                let r = ta.rect(Rect::from_min_size(pos2(x - w, y), vec2(w, font.height)));
                let resp = ui.interact(r, ui.id().with(("idle", click)), Sense::click());
                if resp.hovered() {
                    let (what, key) = if click == 1 { (n("Select the next factory with nothing to build ({key})"), "idle_factory") } else { (n("Select the next idle constructor ({key})"), "idle_builder") };
                    self.tip = Some(Tip::Text(ta_ui::i18n::upper(&label), tf(what, &[("key", &game.keys.text(key))])));
                }
                if resp.clicked() {
                    if click == 1 {
                        game.select_next_idle_factory();
                    } else {
                        game.select_next_idle_builder();
                    }
                }
            }
            x -= w + 16.0;
        }
    }

    // ---- Bottom bar -------------------------------------------------------------------

    fn bottom_bar(&mut self, ui: &mut egui::Ui, env: &Env, game: &Game) {
        let painter = ui.painter().clone();
        let tb = env.tb;
        let end = (ui.max_rect().right() - tb.origin.x) / tb.s;
        match self.art(env, &env.st.intgaf, "PANELBOT") {
            Some(f) => strip(&painter, &f[0], tb, pos2(SIDE_W, TA_HEIGHT - BOTTOM_H), end, 40.0),
            None => {
                painter.rect_filled(ui.max_rect(), 0.0, Color32::from_rgb(12, 13, 17));
            }
        }
        let (Some(l), Some(font)) = (env.st.layout.clone(), env.st.font.clone()) else { return };
        let at = |name: &str, x: f32, y: f32| l.get(name).map_or(pos2(x, y), |r| r.min);
        match self.tip.take() {
            Some(Tip::Unit(id)) => {
                let t = game.sim.types.get(id);
                let (name, desc) = (at("NAME", 132.0, 452.0), at("DESCRIPTION", 132.0, 465.0));
                text(&painter, &font, tb, name.x, name.y, Align::Left, &t.def.display_name, TEXT);
                text(&painter, &font, tb, desc.x, desc.y, Align::Left, &t.def.description, DIM);
                // Costs where the hovered unit goes; the time with the selected builder.
                let x = at("DAMAGEBAR2", 510.0, 463.0).x - 34.0;
                let metal = format!("{}  ", tf("Metal {n}", &[("n", &format!("{:.0}", t.cost_metal.to_f32()))]));
                text(&painter, &font, tb, x, name.y, Align::Left, &metal, self.color(env, env.st.metal));
                text(&painter, &font, tb, x + font.width(&metal), name.y, Align::Left, &tf("Energy {n}", &[("n", &format!("{:.0}", t.cost_energy.to_f32()))]), self.color(env, env.st.energy));
                let rate = game.selection.first().and_then(|&b| game.sim.unit(b)).map_or(Fx::ZERO, |b| game.sim.unit_type(b).worker_rate);
                if rate > Fx::ZERO {
                    let secs = (t.build_time / rate).to_f32() / 30.0;
                    text(&painter, &font, tb, x, desc.y, Align::Left, &tf("Build time {time}", &[("time", &crate::hud::duration(secs))]), DIM);
                }
            }
            Some(Tip::Text(name, what)) => {
                let (n, d) = (at("NAME", 132.0, 452.0), at("DESCRIPTION", 132.0, 465.0));
                text(&painter, &font, tb, n.x, n.y, Align::Left, t(&name), TEXT);
                text(&painter, &font, tb, d.x, d.y, Align::Left, t(&what), DIM);
            }
            None => self.selected_readout(&painter, env, game, &l, &font),
        }
        self.hovered_readout(&painter, env, game, &l, &font);
    }

    fn color(&self, env: &Env, index: u8) -> Color32 {
        let [r, g, b] = env.data.palette.rgb(index);
        Color32::from_rgb(r, g, b)
    }

    /// The first selected unit: emblem, name, health (and build progress), and what it
    /// makes and uses.
    fn selected_readout(&mut self, painter: &egui::Painter, env: &Env, game: &Game, l: &SideLayout, font: &TaFont) {
        let tb = env.tb;
        let Some(u) = game.selection.first().and_then(|&id| game.sim.unit(id)) else { return };
        let t = game.sim.unit_type(u);
        let owner = game.sim.players.get(u.owner as usize);
        if let (Some(b), Some(p)) = (l.get("LOGO2"), owner) && let Some(f) = self.art.frames(env.vfs, &env.data.palette, "logos", logo_entry(&p.side)) {
            fill(painter, &f[(p.color as usize).min(f.len() - 1)], tb, inclusive(b), Color32::WHITE);
        }
        let n = game.selection.len();
        let same = game.selection.iter().all(|&id| game.sim.unit(id).is_some_and(|o| o.type_id == u.type_id));
        let name = match (n, same) {
            (1, _) => t.def.display_name.clone(),
            (_, true) => format!("{} x{n}", t.def.display_name),
            _ => tf("{name} +{n} more", &[("name", &t.def.display_name), ("n", &(n - 1))]),
        };
        let at = l.get("UNITNAME").map_or(pos2(245.0, 452.0), |r| r.min);
        text(painter, font, tb, at.x, at.y, Align::Center, &name, TEXT);
        if let Some(b) = l.get("DAMAGEBAR").map(inclusive) {
            health(painter, env, b, u, t);
        }
        // A missile silo or anti-missile system: the missiles ready and the next one, under the
        // health bar (where an unfinished unit's build progress goes).
        if u.is_complete() && let Some((line, _)) = crate::hud::stockpile(&game.sim, u) {
            let y = l.get("DAMAGEBAR").map_or(466.0, |r| inclusive(r).max.y) + 1.0;
            text(painter, font, tb, at.x, y, Align::Center, &line, DIM);
        }
        // A constructor on duty, there too (TA's panel has no button for it: the key gives one).
        if let Some(duty) = game.duty_of(u.id) {
            let y = l.get("DAMAGEBAR").map_or(466.0, |r| inclusive(r).max.y) + 1.0;
            text(painter, font, tb, at.x, y, Align::Center, &tf("On duty: {duty}", &[("duty", &ta_ui::i18n::t(crate::game::duty::label(duty)))]), TEXT);
        }
        let [mm, mu, em, eu] = flows(&game.sim, u);
        let (metal, energy) = (self.color(env, env.st.metal), self.color(env, env.st.energy));
        for (name, value, sign, color) in [("UNITMETALMAKE", mm, "+", metal), ("UNITMETALUSE", mu, "-", metal), ("UNITENERGYMAKE", em, "+", energy), ("UNITENERGYUSE", eu, "-", energy)] {
            if let Some(r) = l.get(name) {
                text(painter, font, tb, r.min.x, r.min.y, Align::Left, &format!("{sign}{}", rate(value)), color);
            }
        }
    }

    /// The unit (or wreck, rock, tree) under the pointer, in the bar's right part.
    fn hovered_readout(&mut self, painter: &egui::Painter, env: &Env, game: &Game, l: &SideLayout, font: &TaFont) {
        let tb = env.tb;
        let at = l.get("UNITNAME2").map_or(pos2(555.0, 452.0), |r| r.min);
        if let Some(u) = game.hovered.and_then(|h| game.sim.unit(h)).filter(|u| game.seen(u) == ta_sim::Seen::Visible) {
            let t = game.sim.unit_type(u);
            text(painter, font, tb, at.x, at.y, Align::Center, &t.def.display_name, TEXT);
            if let Some(b) = l.get("DAMAGEBAR2").map(inclusive) {
                health(painter, env, b, u, t);
            }
            // What a finished unit of ours (or an ally's) makes and uses, under the bar: the
            // metal's figures in the metal's colour, then the energy's.
            if game.may_inspect(u.owner) && u.is_complete() {
                let [mm, mu, em, eu] = flows(&game.sim, u);
                let y = l.get("DAMAGEBAR2").map_or(463.0, |r| inclusive(r).max.y) + 4.0;
                let parts: Vec<(String, Color32)> = [(mm, mu, self.color(env, env.st.metal)), (em, eu, self.color(env, env.st.energy))]
                    .into_iter()
                    .filter_map(|(make, uses, color)| {
                        let words: Vec<String> = [(make, "+"), (uses, "-")].into_iter().filter(|(v, _)| *v >= 0.05).map(|(v, sign)| format!("{sign}{}", rate(v))).collect();
                        (!words.is_empty()).then(|| (words.join(" "), color))
                    })
                    .collect();
                let gap = 8.0;
                let total: f32 = parts.iter().map(|(s, _)| font.width(s)).sum::<f32>() + gap * parts.len().saturating_sub(1) as f32;
                let mut x = at.x - total / 2.0;
                for (words, color) in parts {
                    text(painter, font, tb, x, y, Align::Left, &words, color);
                    x += font.width(&words) + gap;
                }
            }
            return;
        }
        if env.ctx.is_pointer_over_egui() || !matches!(game.command_mode, None | Some(CommandMode::Reclaim | CommandMode::Resurrect)) {
            return;
        }
        let Some(f) = game.hovered_feature() else { return };
        let feature = &game.sim.features[f];
        let def = &game.sim.data.features[feature.def];
        let left = (1.0 - feature.reclaimed.to_f32()).clamp(0.0, 1.0);
        let name = if def.description.is_empty() { def.name.clone() } else { def.description.clone() };
        text(painter, font, tb, at.x, at.y, Align::Center, &name, TEXT);
        let mut worth = Vec::new();
        if def.metal > 0.0 {
            worth.push(tf("Metal {n}", &[("n", &format!("{:.0}", def.metal * left as f64))]));
        }
        if def.energy > 0.0 {
            worth.push(tf("Energy {n}", &[("n", &format!("{:.0}", def.energy * left as f64))]));
        }
        if !worth.is_empty() {
            let y = l.get("DAMAGEBAR2").map_or(463.0, |r| r.min.y) + 1.0;
            text(painter, font, tb, at.x, y, Align::Center, &worth.join("  "), DIM);
        }
    }
}

/// A health bar in the unit bar, with the build progress under it while unfinished.
fn health(painter: &egui::Painter, env: &Env, b: Rect, u: &ta_sim::Unit, t: &ta_sim::UnitType) {
    let tb = env.tb;
    let frac = (u.health / t.max_health).to_f32().clamp(0.0, 1.0);
    painter.rect_filled(tb.snap(b.expand(0.5)), 0.0, Color32::from_black_alpha(170));
    bar(painter, tb, b, frac, &env.data.palette, health_color(frac));
    if !u.is_complete() {
        let r = Rect::from_min_size(pos2(b.min.x, b.max.y + 1.0), vec2(b.width(), 2.0));
        painter.rect_filled(tb.snap(r.expand(0.5)), 0.0, Color32::from_black_alpha(170));
        bar(painter, tb, r, u.build_progress.to_f32(), &env.data.palette, 225);
    }
}

/// What the bottom bar says about a gadget.
fn action_tip(action: Action, game: &Game, menu: Option<MenuAt>) -> Tip {
    let key = |a: &str| game.keys.text(a).to_string();
    let (name, what) = match action {
        Action::OrdersTab => (n("ORDERS").into(), n("The selected units' orders and states").into()),
        Action::BuildTab => (n("BUILD").into(), n("What the selected unit can build").into()),
        Action::Prev | Action::Next => {
            let what = menu.map_or_else(|| t("More of the build menu").to_string(), |m| tf("More of the build menu (page {page} of {pages})", &[("page", &(m.page + 1)), ("pages", &m.pages)]));
            ((if action == Action::Prev { n("PREVIOUS PAGE") } else { n("NEXT PAGE") }).to_string(), what)
        }
        Action::Stop => (n("STOP").into(), format!("{} ({})", t("Drop every order"), key("stop"))),
        Action::Mode(m) => {
            let (what, action) = match m {
                CommandMode::Move if game.order_button == ta_config::OrderButton::Left => (n("Click where to go (a click on the ground does it too)"), "move"),
                CommandMode::Move => (n("Click where to go (right-click the ground does it too)"), "move"),
                CommandMode::Attack => (n("Click a unit or the ground to attack; drag for an area"), "attack"),
                CommandMode::Patrol => (n("Click where to patrol to and back from"), "patrol"),
                CommandMode::Guard => (n("Click a unit to guard"), "guard"),
                CommandMode::DGun => (n("Click the target"), "dgun"),
                CommandMode::Reclaim => (n("Click a wreck, rock or tree (or your own unit) to reclaim; drag for an area"), "reclaim"),
                CommandMode::Repair => (n("Click a unit to repair or help build"), "repair"),
                CommandMode::Capture => (n("Click an enemy unit to take over; drag for an area"), "capture"),
                CommandMode::Load => (n("Click a unit to pick up"), "load"),
                CommandMode::Unload => (n("Click where to set the cargo down"), "unload"),
                CommandMode::Fight => (n("Click where to fight your way to"), "fight"),
                CommandMode::Resurrect => (n("Click a wreck to raise; drag for an area"), "resurrect"),
                CommandMode::AreaMex => (n("Drag a circle: extractors on every metal spot in it"), "area_mex"),
            };
            let what = if action.is_empty() { t(what).to_string() } else { format!("{} ({})", t(what), key(action)) };
            (ta_ui::i18n::upper(t(m.label_for(game))), what)
        }
        Action::Fire => (n("FIRE ORDERS").into(), n("Hold fire, return fire, fire at will (right-click steps back)").into()),
        Action::Movement => (n("MOVE ORDERS").into(), n("Hold position, maneuver, roam (right-click steps back)").into()),
        Action::OnOff => (n("ON / OFF").into(), n("Switch the selected units on or off").into()),
        Action::Cloak => (n("CLOAK").into(), format!("{} ({})", t("Cloak or decloak; cloaking costs energy"), key("cloak"))),
    };
    Tip::Text(name, what)
}
