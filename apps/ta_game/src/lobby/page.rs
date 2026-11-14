//! The lobby page: the battlefield at the left (the host chooses it), the players in the middle
//! (each one's picture, ping, side, colour and team, whether they are ready and whether they have
//! the battle's map; the host's say over every seat), the battle's name, its address to share and
//! the chat at the right; READY, or the host's START and its countdown, under the players. On a
//! narrow screen the chat goes under the battlefield.

use egui::{Align2, Color32, CornerRadius, RichText, Sense, Shape, Stroke, StrokeKind, vec2};
use ta_net::{LobbySlot, MapStatus, Msg, SlotKind};
use ta_ui::i18n::{n, t, tf};
use ta_ui::panel::{self, Exit, Panel};
use ta_ui::widgets::{self as w, ActionButton};

use super::{LobbyAction, NetLobby, Role, first_free_color, first_free_team, max_slots, seat_side, set_seat_kind, set_spectator, signal, take_seat};
use crate::palette::team_color;
use crate::setup::Assets;
use crate::theme::*;

/// What is said under a person's name of their copy of the battle's map, in its colour (none:
/// they have it).
fn map_words(status: MapStatus) -> Option<(&'static str, Color32)> {
    match status {
        MapStatus::Ready => None,
        MapStatus::Checking => Some((n("CHECKING THE MAP…"), STEEL)),
        MapStatus::Missing => Some((n("MAP NOT INSTALLED"), RED)),
        MapStatus::Other => Some((n("OTHER MAP VERSION"), RED)),
    }
}

/// The same, told in full (a pointer over it, a list of names).
fn map_tip(name: &str, status: MapStatus) -> String {
    match status {
        MapStatus::Ready => tf("{name} has this map", &[("name", &name)]),
        MapStatus::Checking => tf("{name} has not yet told the host which maps they have", &[("name", &name)]),
        MapStatus::Missing => tf("{name} does not have this map", &[("name", &name)]),
        MapStatus::Other => tf("{name} has another version of this map (other terrain or start positions)", &[("name", &name)]),
    }
}

/// The people in the lobby but the host (players, then spectators), by peer, with their names.
fn guests(state: &ta_net::Lobby) -> Vec<(u8, String)> {
    state.slots.iter().filter_map(|s| match &s.kind {
        SlotKind::Human { peer, name } if *peer != 0 => Some((*peer, name.clone())),
        _ => None,
    }).chain(state.spectators.iter().filter(|(peer, _)| *peer != 0).cloned()).collect()
}

/// What the player asked for on the page, carried out once it is drawn.
enum Request {
    /// Host: take this person out of the lobby.
    Remove(u8),
    /// Watch (true) or play; the host can send another person to watch.
    Watch(u8, bool),
    /// Take this open seat.
    Sit(usize),
    /// Host: start the countdown, or call it off.
    Countdown(bool),
    /// Host: what sits at this seat nobody holds (open, closed, a computer).
    Kind(usize, SlotKind),
}

/// A row's height, and the widths of its columns.
const ROW: f32 = 34.0;
const NAME: f32 = 136.0;
const PING: f32 = 50.0;

/// A connection's bars (three, lit by how quick it is) and its milliseconds.
fn ping(ui: &mut egui::Ui, ms: Option<u16>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(PING, ROW), Sense::hover());
    let painter = ui.painter();
    signal::bars(painter, rect, ms);
    let text = ms.map_or_else(|| "—".to_string(), |ms| format!("{ms}"));
    painter.text(egui::pos2(rect.min.x + 19.0, rect.center().y + 1.0), Align2::LEFT_CENTER, text, strong(11.5), if ms.is_some() { TEXT } else { STEEL_DIM });
    resp.on_hover_text(match ms {
        Some(ms) => tf("{ms} ms to the host and back", &[("ms", &ms)]),
        None => t("Measuring…").to_string(),
    });
}

impl NetLobby {
    pub fn draw(&mut self, ui: &mut egui::Ui, assets: &Assets) -> LobbyAction {
        if let Some(a) = self.pump(assets) {
            return a;
        }
        // Coming back to a battle: no lobby, the battle so far on its way (then straight in).
        if let Some(Role::Client(c)) = &self.role
            && self.state.slots.is_empty()
            && let Some(tick) = c.history_received()
        {
            return self.rejoining(ui, tick);
        }
        let ctx = ui.ctx().clone();
        self.own_portrait(&ctx);
        // The countdown heard, a tick a second.
        if self.state.countdown != self.spoken {
            if self.state.countdown.is_some() {
                ta_ui::sound::play(&ctx, ta_ui::sound::UiSound::Toggle);
            }
            self.spoken = self.state.countdown;
        }
        let host = self.is_host();
        let max = max_slots(&self.state, assets);
        let mut changed = false;
        let mut mine = false;
        let mut requests = Vec::new();
        let map_title = self.map_title(assets);
        let title = if self.state.title.trim().is_empty() { t("LOBBY").to_string() } else { self.state.title.to_uppercase() };
        let players = self.state.slots.iter().take(max).filter(|s| s.kind.plays()).count();
        let host_name = self.state.slots.iter().find_map(|s| match &s.kind {
            SlotKind::Human { peer: 0, name } => Some(name.clone()),
            _ => None,
        }).or_else(|| self.state.spectators.iter().find(|(p, _)| *p == 0).map(|(_, n)| n.clone())).unwrap_or_default();
        let subtitle = tf("HOSTED BY {host} · {map} · {n} OF {max} PLAYERS", &[("host", &host_name.to_uppercase()), ("map", &map_title.to_uppercase()), ("n", &players), ("max", &max)]);
        let width = (ctx.content_rect().width() - 40.0).clamp(860.0, 1240.0);
        let wide = width >= 1140.0;
        let page = Panel::new(&title).subtitle(&subtitle).width(width).exit(Exit::Leave);
        let response = panel::page(&ctx, "mp_lobby", page, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 22.0;
                ui.vertical(|ui| {
                    ui.set_width(300.0);
                    self.battlefield(ui, assets, host, &mut changed);
                    if !wide {
                        ui.add_space(12.0);
                        self.talk(ui, host, &mut changed, 220.0);
                    }
                });
                ui.vertical(|ui| {
                    ui.set_width(if wide { 488.0 } else { ui.available_width() });
                    self.players(ui, assets, host, max, &mut changed, &mut mine, &mut requests);
                    ui.add_space(12.0);
                    if crate::menu::battle_options(ui, "lobby", &mut self.state.rules, host && self.state.countdown.is_none()) {
                        changed = true;
                    }
                    ui.add_space(12.0);
                    self.start_area(ui, assets, host, max, &mut mine, &mut requests);
                });
                if wide {
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        self.talk(ui, host, &mut changed, 380.0);
                    });
                }
            });
        });
        let mut action = LobbyAction::None;
        // The host leaving closes the lobby on everyone in it: asked first, while anyone is.
        let others = self.state.slots.iter().any(|s| matches!(&s.kind, SlotKind::Human { peer, .. } if *peer != self.me)) || self.state.spectators.iter().any(|(p, _)| *p != self.me);
        let mut leave = false;
        if self.closing {
            let mut stay = false;
            let asked = panel::modal(&ctx, "lobby_close", Panel::new("CLOSE THE LOBBY?").width(460.0).exit(Exit::Close).accent(RED), true, |ui| {
                ui.add(egui::Label::new(RichText::new(t("Everyone in it goes back to the Multiplayer page, and the battle is not played.")).color(TEXT)).wrap());
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    leave = ui.add(ActionButton::danger("CLOSE LOBBY")).clicked();
                    stay = ui.add(ActionButton::secondary("STAY IN THE LOBBY")).clicked();
                });
            });
            if stay || asked.exit || !others {
                self.closing = false;
            }
        } else if response.exit {
            if host && others { self.closing = true } else { leave = true }
        }
        if leave {
            self.closing = false;
            match &mut self.role {
                Some(Role::Host(h)) => h.broadcast(&Msg::Bye),
                Some(Role::Client(c)) => c.send(&Msg::Bye),
                None => {}
            }
            action = LobbyAction::Leave;
        }
        for request in requests {
            match (request, &mut self.role) {
                (Request::Remove(peer), _) => self.remove(peer),
                (Request::Watch(peer, watch), Some(Role::Host(_))) => {
                    changed |= set_spectator(&mut self.state, peer, watch, max, &self.sides);
                }
                (Request::Watch(_, watch), Some(Role::Client(c))) => c.send(&Msg::SetSpectator(watch)),
                (Request::Sit(seat), Some(Role::Host(_))) => {
                    changed |= take_seat(&mut self.state, 0, seat, max);
                }
                (Request::Sit(seat), Some(Role::Client(c))) => c.send(&Msg::TakeSlot(seat as u8)),
                (Request::Countdown(on), Some(Role::Host(_))) => {
                    if on {
                        ta_ui::sound::play(&ctx, ta_ui::sound::UiSound::BigButton);
                    }
                    self.countdown(on);
                }
                (Request::Kind(seat, kind), Some(Role::Host(_))) => {
                    changed |= set_seat_kind(&mut self.state, seat, kind);
                }
                _ => {}
            }
        }
        if changed && host {
            self.changed();
        }
        if mine && !host {
            let me = self.me;
            let slot = self.state.slots.iter().find(|s| matches!(&s.kind, SlotKind::Human { peer, .. } if *peer == me)).cloned();
            if let (Some(s), Some(Role::Client(c))) = (slot, &mut self.role) {
                c.send(&Msg::SetSlot { side: s.side, color: s.color, ready: s.ready });
            }
        }
        action
    }

    /// Coming back to a battle: how much of it has come so far (`tick`), until it is all here.
    fn rejoining(&mut self, ui: &mut egui::Ui, tick: u64) -> LobbyAction {
        let ctx = ui.ctx().clone();
        let page = Panel::new("REJOINING THE BATTLE").subtitle("RECEIVING THE BATTLE SO FAR").width(460.0).exit(Exit::Leave);
        let response = panel::page(&ctx, "mp_rejoining", page, |ui| {
            let text = tf("{time} of the battle received. You go back in as soon as it is all here.", &[("time", &crate::highlights::clock(tick))]);
            ui.add(egui::Label::new(RichText::new(text).color(TEXT)).wrap());
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        if response.exit {
            if let Some(Role::Client(c)) = &mut self.role {
                c.send(&Msg::Bye);
            }
            return LobbyAction::Leave;
        }
        LobbyAction::None
    }

    /// The battlefield: its picture with everyone's start, its name and facts, and for the host
    /// the maps to choose from.
    fn battlefield(&mut self, ui: &mut egui::Ui, assets: &Assets, host: bool, changed: &mut bool) {
        ui.spacing_mut().item_spacing.x = 8.0;
        panel::section(ui, "BATTLEFIELD");
        self.map_preview(ui, assets);
        ui.add_space(6.0);
        let entry = assets.maps.iter().find(|m| m.name == self.state.map);
        w::fitted_title(ui, &self.map_title(assets), 24.0, TEXT_BRIGHT);
        // A guest without the host's map is told so, and what it takes: their own copy of it
        // (the same as the host's) or another map.
        let own = if host { MapStatus::Ready } else { self.state.map_status_of(self.me) };
        let missing = match own {
            MapStatus::Missing => Some(t("You do not have this map: the battle cannot start until you have it (its map pack, on in the launcher) or the host chooses another.")),
            MapStatus::Other => Some(t("Your copy of this map is not the host's version: the battle cannot start until you have the host's, or the host chooses another map.")),
            MapStatus::Ready | MapStatus::Checking => None,
        };
        if let Some(why) = missing {
            ui.add(egui::Label::new(RichText::new(why).font(body(12.5)).color(RED)).wrap());
        }
        if let Some(m) = entry {
            ui.label(RichText::new(tf("{planet} · {size} · UP TO {n} PLAYERS", &[("planet", &m.planet.to_uppercase()), ("size", &m.size), ("n", &m.start_positions)])).font(strong(11.0)).color(AMBER).extra_letter_spacing(ta_ui::i18n::tracking(1.2)));
            if !m.description.is_empty() {
                ui.add(egui::Label::new(RichText::new(&m.description).font(body(12.5)).color(TEXT)).wrap());
            }
        }
        // The countdown leaves the map be.
        if host && self.state.countdown.is_none() {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let search = w::search_field(ui, &mut self.map_choice.words, t("Search maps…"), 300.0 - 6.0 - 32.0, None);
                // (A map at random, of those the list shows with room for the players, else of
                // them all.)
                let size = vec2(32.0, search.rect.height());
                if w::glyph_button(ui, w::Glyph::Shuffle, size, w::Tone::Secondary, assets.selectable_maps.len() > 1).on_hover_text(t("A map at random, with room for these players")).clicked() {
                    let players = self.state.slots.iter().filter(|s| s.kind.plays()).count().max(2);
                    let maps = &assets.selectable_maps;
                    let fits = |pool: Vec<usize>| -> Vec<usize> { pool.into_iter().filter(|&i| maps[i].name != self.state.map && maps[i].start_positions >= players).collect() };
                    let mut pool = fits(self.map_choice.shown(maps));
                    if pool.is_empty() {
                        pool = fits((0..maps.len()).collect());
                    }
                    self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    if !pool.is_empty() {
                        let chosen = &maps[pool[((self.seed >> 33) % pool.len() as u64) as usize]];
                        if !self.map_choice.accepts(chosen) {
                            self.map_choice.clear();
                        }
                        self.state.map = chosen.name.clone();
                        *changed = true;
                    }
                }
            });
            ui.add_space(4.0);
            // Where the maps come from, and their order.
            let reveal = self.map_choice.controls(ui, "mp_maps", &assets.selectable_maps, 300.0);
            ui.add_space(4.0);
            // (Maps someone in the lobby lacks, or has another version of, show it: their count
            // of players in red, and who, under the pointer.)
            let people = guests(&self.state);
            let transport = match &self.role {
                Some(Role::Host(h)) => Some(h),
                _ => None,
            };
            let own_maps = &self.own_maps;
            panel::well(ui, |ui| {
                let height = ta_ui::scroll::whole_rows(200.0, panel::LIST_ROW + 1.0, 1.0);
                let area = egui::ScrollArea::vertical().id_salt("mp_maps").max_height(height).min_scrolled_height(ta_ui::scroll::whole_rows(160.0, panel::LIST_ROW + 1.0, 1.0)).auto_shrink([false, true]);
                ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Well, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    let maps = &assets.selectable_maps;
                    let shown = self.map_choice.shown(maps);
                    let by_origin = self.map_choice.order == crate::map_list::Order::Origin;
                    let mut origin = None;
                    for m in shown.iter().map(|&i| &maps[i]) {
                        // (Put in order by origin: each origin's name over its maps.)
                        if by_origin && origin != Some(m.origin) {
                            origin = Some(m.origin);
                            crate::map_list::heading(ui, m.origin, shown.iter().filter(|&&j| maps[j].origin == m.origin).count());
                        }
                        let lacking: Vec<String> = transport.map_or_else(Vec::new, |h| people.iter().filter_map(|(peer, name)| {
                            let status = MapStatus::of(own_maps.get(&m.name).copied(), h.peer_map(*peer, &m.name));
                            matches!(status, MapStatus::Missing | MapStatus::Other).then(|| map_tip(name, status))
                        }).collect());
                        let players = RichText::new(tf("{n}P", &[("n", &m.start_positions)])).font(strong(11.5));
                        let players = if lacking.is_empty() { players } else { players.color(RED) };
                        let row = crate::map_list::row(ui, m, m.name == self.state.map, players);
                        // (The map chosen brought into view after the origins or the order changed.)
                        if reveal && m.name == self.state.map {
                            row.scroll_to_me(Some(egui::Align::Center));
                        }
                        let row = if lacking.is_empty() { row } else { row.on_hover_text(format!("{}\n{}", t("Not everyone in the lobby has this map:"), lacking.join("\n"))) };
                        if row.clicked() && self.state.map != m.name {
                            self.state.map = m.name.clone();
                            *changed = true;
                        }
                    }
                });
            });
        }
    }

    /// The players, a row each: the host's say over every seat, ours over our own.
    #[allow(clippy::too_many_arguments)]
    fn players(&mut self, ui: &mut egui::Ui, assets: &Assets, host: bool, max: usize, changed: &mut bool, mine: &mut bool, requests: &mut Vec<Request>) {
        ui.spacing_mut().item_spacing.x = 8.0;
        panel::section(ui, "PLAYERS");
        let ctx = ui.ctx().clone();
        let sides = crate::setup::side_choices(&assets.data);
        let side_width = w::combo_width(ui, sides.iter().map(|s| s.label.clone()), 66.0);
        let team_width = w::combo_width(ui, (1..=super::MAX_SLOTS as u8).map(|n| tf("T{n}", &[("n", &n)])), 50.0);
        let me = self.me;
        let counting = self.state.countdown.is_some();
        let watching = self.state.spectators.iter().any(|(p, _)| *p == me);
        let seated = self.state.slots.iter().any(|s| matches!(&s.kind, SlotKind::Human { peer, .. } if *peer == me));
        let ai = [n("AI — Easy"), n("AI — Normal"), n("AI — Hard")];
        // (The colours the seats have, for a click to pass over those taken.)
        let colours: Vec<(u8, bool)> = self.state.slots.iter().map(|s| (s.color, s.kind.plays())).collect();
        // (Whether each seat's person has the battle's map, as the host found.)
        let maps: Vec<MapStatus> = self.state.slots.iter().map(|s| match &s.kind {
            SlotKind::Human { peer, .. } => self.state.map_status_of(*peer),
            _ => MapStatus::Ready,
        }).collect();
        for i in 0..self.state.slots.len() {
            let pings = &self.pings;
            let slot = &mut self.state.slots[i];
            let outside = i >= max;
            ui.horizontal(|ui| {
                ui.set_height(ROW);
                // The seat's number, in its player's colour.
                let filled = slot.kind.plays();
                let (badge, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
                let painter = ui.painter();
                painter.rect(badge, CornerRadius::same(2), if filled { PANEL_HI } else { INSET }, Stroke::new(1.0, if filled { team_color(slot.color) } else { LINE }), StrokeKind::Inside);
                painter.text(badge.center(), Align2::CENTER_CENTER, format!("{}", i + 1), strong(13.0), if filled { TEXT_BRIGHT } else { STEEL_DIM });
                // Their picture: a person's own, a computer's chip, an empty seat's outline.
                let (face, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
                ui.painter().add(ta_ui::metal::slab(&ctx, face, ta_ui::metal::Slab::Slot, 0.0, false));
                let inside = face.shrink(2.0);
                match &slot.kind {
                    SlotKind::Human { peer, .. } => match self.textures.get(&ctx, &self.portraits, *peer) {
                        Some(texture) => {
                            ui.painter().image(texture.id(), inside, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                        }
                        None => {
                            let painter = ui.painter().with_clip_rect(inside);
                            let c = inside.center();
                            painter.circle_filled(c - vec2(0.0, 4.0), 5.0, STEEL_DIM);
                            painter.circle_filled(c + vec2(0.0, 13.0), 10.0, STEEL_DIM);
                        }
                    },
                    SlotKind::Ai { .. } => {
                        ui.painter().text(inside.center(), Align2::CENTER_CENTER, t("AI"), strong(12.0), team_color(slot.color));
                    }
                    SlotKind::Open => {}
                    SlotKind::Closed => {
                        let c = inside.center();
                        let stroke = Stroke::new(1.5, STEEL_DIM);
                        ui.painter().line_segment([c - vec2(6.0, 6.0), c + vec2(6.0, 6.0)], stroke);
                        ui.painter().line_segment([c - vec2(-6.0, 6.0), c + vec2(-6.0, 6.0)], stroke);
                    }
                }
                // Who: a name (ours in amber) over what they are; the host chooses an empty
                // seat's or a computer's kind.
                let is_human = matches!(slot.kind, SlotKind::Human { .. });
                let own = matches!(&slot.kind, SlotKind::Human { peer, .. } if *peer == me);
                ui.allocate_ui_with_layout(vec2(NAME, ROW), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    if host && !is_human && !counting {
                        ui.add_space(4.0);
                        let label = match &slot.kind {
                            SlotKind::Open => t("Open seat").to_string(),
                            SlotKind::Closed => t("Closed").to_string(),
                            SlotKind::Ai { difficulty } => t(ai[(*difficulty as usize).min(2)]).to_string(),
                            SlotKind::Human { .. } => String::new(),
                        };
                        egui::ComboBox::from_id_salt(("mp_kind", i)).selected_text(label).width(NAME).show_ui(ui, |ui| {
                            for (kind, text) in [(SlotKind::Open, n("Open seat")), (SlotKind::Closed, n("Closed")), (SlotKind::Ai { difficulty: 0 }, ai[0]), (SlotKind::Ai { difficulty: 1 }, ai[1]), (SlotKind::Ai { difficulty: 2 }, ai[2])] {
                                if ui.selectable_label(slot.kind == kind, t(text)).clicked() && slot.kind != kind {
                                    requests.push(Request::Kind(i, kind));
                                }
                            }
                        });
                    } else {
                        let (name, sub) = match &slot.kind {
                            // ("LOBBY HOST": the noun; "HOST" is the connect page's button.)
                            SlotKind::Human { peer, name } => (name.clone(), if *peer == 0 { t("LOBBY HOST").to_string() } else if outside { t("WATCHES: NO ROOM").to_string() } else { String::new() }),
                            SlotKind::Ai { difficulty } => (t(ai[(*difficulty as usize).min(2)]).to_string(), t("COMPUTER").to_string()),
                            SlotKind::Open => (t("Open seat").to_string(), t("WAITING FOR A PLAYER").to_string()),
                            SlotKind::Closed => (t("Closed").to_string(), String::new()),
                        };
                        // A person without the battle's map (or with another version of it): said
                        // under their name, in red, before anything else.
                        let map = map_words(maps[i]);
                        let (sub, sub_colour) = match map {
                            Some((words, colour)) => (t(words).to_string(), colour),
                            None => (sub, STEEL_DIM),
                        };
                        ui.add_space(if sub.is_empty() { 8.0 } else { 2.0 });
                        let colour = if own { AMBER } else if filled { TEXT_BRIGHT } else { STEEL };
                        ui.add(egui::Label::new(RichText::new(&name).font(strong(14.0)).color(colour)).truncate());
                        if !sub.is_empty() {
                            let line = ui.add(egui::Label::new(RichText::new(sub).font(strong(9.5)).color(sub_colour).extra_letter_spacing(ta_ui::i18n::tracking(1.0))).truncate());
                            if map.is_some() {
                                line.on_hover_text(map_tip(&name, maps[i]));
                            }
                        }
                    }
                });
                // (The host's own connection is the lobby itself: no ping.)
                match &slot.kind {
                    SlotKind::Human { peer, .. } if *peer != 0 => ping(ui, pings.iter().find(|(p, _)| p == peer).map(|(_, ms)| *ms)),
                    _ => {
                        ui.allocate_exact_size(vec2(PING, ROW), Sense::hover());
                    }
                }
                if filled {
                    // Side, colour and team: ours to change, and the host's for its computers
                    // (the host also sets every team).
                    let editable = !counting && (own || host && !is_human);
                    let shown = sides.iter().find(|s| s.key == slot.side).map_or(slot.side.clone(), |s| s.label.clone());
                    if editable {
                        egui::ComboBox::from_id_salt(("mp_side", i)).selected_text(&shown).width(side_width).show_ui(ui, |ui| {
                            for s in &sides {
                                if ui.selectable_label(slot.side == s.key, &s.label).clicked() {
                                    slot.side = s.key.clone();
                                    *changed = true;
                                    *mine |= own;
                                }
                            }
                        });
                    } else {
                        ui.add_sized([side_width, 24.0], egui::Label::new(RichText::new(&shown).color(TEXT)));
                    }
                    let (r, resp) = ui.allocate_exact_size(vec2(28.0, 26.0), if editable { Sense::click() } else { Sense::hover() });
                    let border = if editable && resp.hovered() { AMBER_BRIGHT } else { Color32::from_black_alpha(180) };
                    ui.painter().add(Shape::convex_polygon(w::chamfer(r, 6.0), team_color(slot.color), Stroke::new(1.0, border)));
                    if editable && resp.on_hover_text(t("Click to change colour")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        slot.color = crate::palette::next_free_colour(slot.color, |c| colours.iter().enumerate().any(|(j, &(other, plays))| j != i && plays && other == c));
                        *changed = true;
                        *mine |= own;
                    }
                    if host && !counting {
                        egui::ComboBox::from_id_salt(("mp_team", i)).selected_text(tf("T{n}", &[("n", &(slot.team + 1))])).width(team_width).show_ui(ui, |ui| {
                            for team in 0..super::MAX_SLOTS as u8 {
                                if ui.selectable_label(slot.team == team, tf("Team {n}", &[("n", &(team + 1))])).clicked() {
                                    slot.team = team;
                                    *changed = true;
                                }
                            }
                        });
                    } else {
                        ui.add_sized([team_width, 24.0], egui::Label::new(RichText::new(tf("T{n}", &[("n", &(slot.team + 1))])).color(TEXT)));
                    }
                    // Ready: a lamp (a computer always is).
                    if is_human {
                        let (lamp, resp) = ui.allocate_exact_size(vec2(16.0, ROW), Sense::hover());
                        let ready = slot.ready || matches!(&slot.kind, SlotKind::Human { peer: 0, .. });
                        ui.painter().add(ta_ui::metal::lamp(lamp.center(), 4.5, if ready { GREEN } else { AMBER }, if ready { 1.0 } else { 0.35 }));
                        resp.on_hover_text(if ready { t("Ready") } else { t("Not ready yet") });
                    }
                    // The host's say over another person: out of the lobby, or to watch.
                    if host && is_human && !own && !counting
                        && let SlotKind::Human { peer, .. } = &slot.kind
                    {
                        let peer = *peer;
                        ui.menu_button(RichText::new("⋯").font(strong(16.0)).color(STEEL), |ui| {
                            if ui.button(t("Move to the spectators")).clicked() {
                                requests.push(Request::Watch(peer, true));
                                ui.close();
                            }
                            if ui.button(RichText::new(t("Remove from the lobby")).color(RED)).clicked() {
                                requests.push(Request::Remove(peer));
                                ui.close();
                            }
                        });
                    }
                } else if slot.kind == SlotKind::Open && !outside && !counting && (watching || seated || host) && me != u8::MAX {
                    // An empty seat: ours for the asking.
                    if ui.add(ActionButton::secondary("SIT HERE").height(28.0)).on_hover_text(t("Take this seat (and its start position)")).clicked() {
                        requests.push(Request::Sit(i));
                    }
                }
            });
        }
        ui.add_space(4.0);
        if host && !self.state.countdown.is_some() {
            // Another seat, at the foot of the seats it adds to; the teams at a click in a row of
            // their own (four buttons wrapped three and one, the last alone).
            if self.state.slots.len() < max && ui.add(ActionButton::secondary("+ ADD SEAT")).clicked() {
                let team = first_free_team(&self.state);
                let color = first_free_color(&self.state);
                self.state.slots.push(LobbySlot { kind: SlotKind::Open, side: seat_side(&self.sides, self.state.slots.len()), color, ready: false, team });
                *changed = true;
            }
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                let playing: Vec<usize> = (0..self.state.slots.len().min(max)).filter(|&i| self.state.slots[i].kind.plays()).collect();
                if ui.add(ActionButton::secondary("FREE FOR ALL")).on_hover_text(t("Everyone on a team of their own")).clicked() {
                    for (team, &i) in playing.iter().enumerate() {
                        self.state.slots[i].team = team as u8;
                    }
                    *changed = true;
                }
                if ui.add_enabled(playing.len() >= 2, ActionButton::secondary("TWO TEAMS")).on_hover_text(t("Seats taken in turn by Team 1 and Team 2; allies start side by side")).clicked() {
                    for (k, &i) in playing.iter().enumerate() {
                        self.state.slots[i].team = (k % 2) as u8;
                    }
                    *changed = true;
                }
                // People together against the computers.
                let computers = playing.iter().any(|&i| matches!(self.state.slots[i].kind, SlotKind::Ai { .. }));
                let people = playing.iter().any(|&i| matches!(self.state.slots[i].kind, SlotKind::Human { .. }));
                if ui.add_enabled(computers && people, ActionButton::secondary("PEOPLE VS COMPUTERS")).on_hover_text(t("Every person on one team against the computer players")).clicked() {
                    for &i in &playing {
                        self.state.slots[i].team = u8::from(matches!(self.state.slots[i].kind, SlotKind::Ai { .. }));
                    }
                    *changed = true;
                }
            });
        }
        if self.state.slots.len() > max {
            ui.label(RichText::new(tf("This map has {max} start positions; extra slots stay out (their players watch).", &[("max", &max)])).font(body(12.5)).color(STEEL));
        }
        // The spectators, and switching between playing and watching.
        ui.add_space(10.0);
        panel::section(ui, "SPECTATORS");
        ui.horizontal(|ui| {
            if self.state.spectators.is_empty() {
                ui.label(RichText::new(t("Nobody is watching.")).color(STEEL));
            } else {
                // Their names (ours in amber), each without the battle's map said so after it.
                let mut job = egui::text::LayoutJob::default();
                let mut tips = Vec::new();
                for (k, (peer, name)) in self.state.spectators.iter().enumerate() {
                    if k > 0 {
                        job.append(", ", 0.0, egui::TextFormat::simple(body(13.0), TEXT_BRIGHT));
                    }
                    job.append(name, 0.0, egui::TextFormat::simple(body(13.0), if *peer == me { AMBER } else { TEXT_BRIGHT }));
                    let status = self.state.map_status_of(*peer);
                    if let Some((words, colour)) = map_words(status) {
                        job.append(&format!(" · {}", t(words)), 0.0, egui::TextFormat::simple(strong(10.5), colour));
                        tips.push(map_tip(name, status));
                    }
                }
                let line = ui.add(egui::Label::new(job).wrap());
                if !tips.is_empty() {
                    line.on_hover_text(tips.join("\n"));
                }
            }
            if me != u8::MAX && !counting {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if watching { n("PLAY INSTEAD") } else { n("WATCH INSTEAD") };
                    if ui.add(ActionButton::secondary(label)).on_hover_text(t("Spectators see the whole battle but take no part in it")).clicked() {
                        requests.push(Request::Watch(me, !watching));
                    }
                });
            }
        });
    }

    /// Under the players: how many are ready, and READY (ours) or START (the host's), or the
    /// countdown.
    fn start_area(&mut self, ui: &mut egui::Ui, assets: &Assets, host: bool, max: usize, mine: &mut bool, requests: &mut Vec<Request>) {
        let width = ui.available_width();
        let playing: Vec<&LobbySlot> = self.state.slots.iter().take(max).filter(|s| s.kind.plays()).collect();
        let humans: Vec<&&LobbySlot> = playing.iter().filter(|s| matches!(s.kind, SlotKind::Human { .. })).collect();
        let ready = humans.iter().filter(|s| s.ready || matches!(&s.kind, SlotKind::Human { peer: 0, .. })).count();
        let teams: std::collections::BTreeSet<u8> = playing.iter().map(|s| s.team).collect();
        let map_available = assets.selectable_maps.iter().any(|map| map.name == self.state.map);
        if let Some(n) = self.state.countdown {
            // The countdown, large; the host can call it off.
            ui.horizontal(|ui| {
                let label = tf("STARTING IN {n}", &[("n", &n)]);
                let room = if host { width - 130.0 } else { width };
                panel::hero_button(ui, &label, vec2(room, 56.0), false);
                if host && ui.add(ActionButton::danger("CANCEL").size(120.0, 56.0)).clicked() {
                    requests.push(Request::Countdown(false));
                }
            });
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
            return;
        }
        ui.label(RichText::new(tf("{ready} of {n} players ready", &[("ready", &ready), ("n", &humans.len())])).font(strong(12.0)).color(if ready == humans.len() { GREEN } else { STEEL }));
        ui.add_space(4.0);
        if host {
            // Everyone in the lobby (spectators too) needs the battle's map, the host's copy of it.
            let (without, lacking): (Vec<u8>, Vec<String>) = guests(&self.state).into_iter().filter_map(|(peer, name)| match self.state.map_status_of(peer) {
                MapStatus::Missing => Some((peer, tf("{name}: not installed", &[("name", &name)]))),
                MapStatus::Other => Some((peer, tf("{name}: another version", &[("name", &name)]))),
                MapStatus::Ready | MapStatus::Checking => None,
            }).unzip();
            let has_map = self.state.everyone_has_the_map();
            let ok = playing.len() >= 2 && teams.len() >= 2 && ready == humans.len() && map_available && has_map;
            let why = if !map_available {
                Some((t("Activate a map in the launcher before starting a new battle.").to_string(), STEEL))
            } else if playing.len() < 2 {
                Some((t("Needs two players: open a seat for a friend, or add a computer.").to_string(), STEEL))
            } else if !lacking.is_empty() {
                Some((tf("Not everyone has this map ({names}): choose another, wait while they get it, or remove them.", &[("names", &lacking.join(", "))]), RED))
            } else if teams.len() < 2 {
                Some((t("Everyone is on one team: nobody to fight.").to_string(), STEEL))
            } else if ready < humans.len() {
                Some((t("Waiting for everyone to be ready.").to_string(), STEEL))
            } else if !has_map {
                Some((t("Checking that everyone has this map…").to_string(), STEEL))
            } else {
                None
            };
            if let Some((why, colour)) = why {
                ui.add(egui::Label::new(RichText::new(why).color(colour)).wrap());
                ui.add_space(4.0);
            }
            // (Those without the map out of the lobby: a spectator has no seat's menu to be
            // removed from, and would hold the battle back for ever.)
            if !without.is_empty() && playing.len() >= 2 && map_available {
                if ui.add(ActionButton::secondary("REMOVE THEM")).on_hover_text(t("Those without the host's copy of this map are taken out of the lobby")).clicked() {
                    requests.extend(without.iter().map(|&peer| Request::Remove(peer)));
                }
                ui.add_space(4.0);
            }
            if panel::hero_button(ui, "START BATTLE", vec2(width, 56.0), ok).on_hover_text(t("Everyone gets five seconds' warning")).clicked() {
                requests.push(Request::Countdown(true));
            }
        } else {
            let me = self.me;
            let slot = self.state.slots.iter_mut().find(|s| matches!(&s.kind, SlotKind::Human { peer, .. } if *peer == me));
            match slot {
                Some(slot) => {
                    let label = if slot.ready { "NOT READY" } else { "READY" };
                    let clicked = if slot.ready {
                        ui.add(ActionButton::secondary(label).size(width, 46.0)).clicked()
                    } else {
                        panel::hero_button(ui, label, vec2(width, 56.0), true).clicked()
                    };
                    if clicked {
                        slot.ready = !slot.ready;
                        *mine = true;
                        ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Toggle);
                    }
                    if slot.ready {
                        ui.add_space(4.0);
                        ui.label(RichText::new(t("Waiting for the host to start…")).color(STEEL));
                    }
                }
                None => {
                    ui.label(RichText::new(t("Waiting for the host to start…")).color(STEEL));
                }
            }
        }
    }

    /// The battle's name (the host's to give), where others reach it (the host's to share),
    /// and the chat.
    fn talk(&mut self, ui: &mut egui::Ui, host: bool, changed: &mut bool, height: f32) {
        if host {
            panel::section(ui, "BATTLE NAME");
            let counting = self.state.countdown.is_some();
            let field = ui.add_enabled(!counting, egui::TextEdit::singleline(&mut self.state.title).char_limit(48).desired_width(ui.available_width()).margin(egui::Margin::symmetric(8, 6)));
            // (It goes out once typed: when the field is left.)
            if field.lost_focus() {
                *changed = true;
            }
            ui.add_space(8.0);
            if let Some(address) = self.address.clone() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tf("On your network: {address}", &[("address", &address)])).font(strong(12.5)).color(TEXT));
                    if ui.add(ActionButton::secondary("COPY").height(26.0)).on_hover_text(t("Copy the address to give to players")).clicked() {
                        ui.ctx().copy_text(address.clone());
                        ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Done);
                    }
                });
                if let Some(Role::Host(h)) = &self.role {
                    ui.add(egui::Label::new(RichText::new(tf("From the internet: your public address, with port {port} let through your router and firewall", &[("port", &h.port)])).font(body(12.0)).color(STEEL_DIM)).wrap());
                    if h.locked() {
                        ui.label(RichText::new(t("Players need the battle's password to join.")).font(body(12.0)).color(AMBER));
                    }
                }
                ui.add_space(8.0);
            }
        }
        panel::section(ui, "CHAT");
        panel::well(ui, |ui| {
            let area = egui::ScrollArea::vertical().id_salt("mp_chat").max_height(height).min_scrolled_height(height).auto_shrink([false, false]).stick_to_bottom(true);
            ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Well, |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                self.chat.show(ui, &self.state);
            });
        });
        ui.add_space(4.0);
        if let Some(text) = self.chat.input(ui) {
            match &mut self.role {
                Some(Role::Host(h)) => {
                    self.chat.said(&self.name, &text);
                    h.broadcast(&Msg::Chat { from: self.name.clone(), text });
                }
                Some(Role::Client(c)) => c.send(&Msg::Chat { from: self.name.clone(), text }),
                None => {}
            }
        }
    }

    /// The chosen map with its start positions, each marked with the colour and number of
    /// the player who will start there.
    fn map_preview(&mut self, ui: &mut egui::Ui, assets: &Assets) {
        let name = self.state.map.clone();
        let (frame, _) = ui.allocate_exact_size(vec2(300.0, 300.0), Sense::hover());
        let Some(picture) = crate::map_preview::picture(ui.ctx(), assets, &name, 290.0) else {
            ui.painter().add(ta_ui::metal::well(frame.shrink(5.0), INSET));
            // (A map we have no copy of: said in the well its picture would fill.)
            if !name.is_empty() && !assets.maps.iter().any(|m| m.name == name) {
                ui.painter().text(frame.center(), Align2::CENTER_CENTER, t("MAP NOT INSTALLED"), strong(13.0), STEEL_DIM);
            }
            return;
        };
        let placed = crate::map_preview::show(ui.painter(), &picture, frame);
        // Where each player will start, as the battle seats them (in slot order, empty seats
        // skipped; a team game's allies side by side); random starts are drawn when it begins.
        let random = self.state.rules.random_starts;
        // Only the positions the map offers players (some define spares).
        let offered = assets.maps.iter().find(|m| m.name == name).map_or(0, |m| m.start_positions);
        let starts = assets.maps.iter().find(|m| m.name == name).and_then(|m| m.info.schemas.first().map(|sc| sc.start_positions.iter().take(m.start_positions).copied().collect::<Vec<_>>())).unwrap_or_default();
        let mut seats: Vec<crate::setup::SlotSetup> = self.state.slots.iter().filter(|s| s.kind.plays() && !random).enumerate()
            .map(|(i, s)| crate::setup::SlotSetup { side: String::new(), color: s.color, controller: crate::setup::Controller::Human, start: i, team: s.team, name: String::new() }).collect();
        if !seats.is_empty() {
            crate::setup::assign_starts(&mut seats, offered, 0, false, &crate::setup::start_spots(assets, &name));
        }
        for (i, &(_, x, z)) in starts.iter().enumerate() {
            let fill = seats.iter().find(|s| s.start == i).map_or(Color32::from_gray(58), |s| team_color(s.color));
            crate::map_preview::start_marker(ui.painter(), placed.marker_at(x, z, 10.0), fill, &format!("{}", i + 1));
        }
    }
}
