//! The connect page: our name; the battles hosted on this network (their names, hosts, maps,
//! players and seats, whether they take a password) and those joined lately; hosting one
//! (its name, port and password); joining one by its address. Connecting goes on away from
//! the interface (an address that does not answer takes seconds), with a way to give up.

use std::sync::mpsc::{self, Receiver, TryRecvError};

use egui::{CornerRadius, Margin, RichText, Stroke, vec2};
use ta_net::Client;
use ta_net::lan::LanBrowser;
use ta_ui::i18n::{t, tf};
use ta_ui::panel::{self, Exit, Panel};
use ta_ui::widgets::{self as w, ActionButton};

use super::{NetLobby, bind_host, reconnect, refusal_text};
use crate::setup::Assets;
use crate::theme::*;

/// How many games joined lately the page keeps.
pub const RECENT: usize = 6;

/// The connect page's fields (kept between visits, and some in the settings).
pub struct ConnectForm {
    pub name: String,
    pub address: String,
    pub port: String,
    /// The password typed to join by address, or a locked game on the network.
    password: String,
    /// The battle's name and password when we host.
    pub host_title: String,
    host_password: String,
    /// Why the last try failed, shown beside what it was about.
    error: Option<(Place, String)>,
    /// Games heard on the local network (`None` when the LAN port is taken on this PC).
    lan: Option<LanBrowser>,
    lan_tried: bool,
    /// Games joined lately (address, the battle's name as last seen), the latest first.
    pub recent: Vec<(String, String)>,
    /// A network game that takes a password, being asked for it.
    unlocking: Option<String>,
    /// The password to be typed next (the host asked for it): on that game's card, or under
    /// JOIN BY ADDRESS.
    ask_password: bool,
    connecting: Option<Connecting>,
    /// A change for the settings to keep: what the last lobby made here was made with, or a
    /// game forgotten (the app takes it).
    pub keep: Option<Keep>,
}

/// See [`ConnectForm::keep`].
pub enum Keep {
    /// We hosted on this port, the battle with this name.
    Hosted { port: u16, title: String },
    /// We joined this address (typed, or from the recent games).
    Joined(String),
    /// This game is no longer among those joined lately.
    Forget(String),
}

/// A connection being made, away from the interface.
struct Connecting {
    address: String,
    /// Where it was asked for (typed or chosen from the recent games, it is kept among them).
    from: Place,
    result: Receiver<std::io::Result<(Client, reconnect::Key)>>,
}

/// The page's parts: where a connection is asked for, and where its fate is told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// At the top (the lobby left behind, the command line).
    Top,
    Network,
    Recent,
    Hosting,
    Address,
}

impl Default for ConnectForm {
    fn default() -> Self {
        ConnectForm {
            name: "Commander".into(),
            // (No address until one is typed: the hint says what goes there.)
            address: String::new(),
            port: ta_net::DEFAULT_PORT.to_string(),
            password: String::new(),
            host_title: String::new(),
            host_password: String::new(),
            error: None,
            lan: None,
            lan_tried: false,
            recent: Vec::new(),
            unlocking: None,
            ask_password: false,
            connecting: None,
            keep: None,
        }
    }
}

// (One action a frame at most: the size difference between its variants costs nothing.)
#[allow(clippy::large_enum_variant)]
pub enum ConnectAction {
    None,
    Back,
    Lobby(NetLobby),
}

/// The height of a game's row in the network's list.
const GAME_ROW: f32 = 52.0;

/// A text field as tall as the page's other controls, its text in the middle.
fn field(text: &mut String, width: f32) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(text).desired_width(width).margin(Margin::symmetric(10, 6)).min_size(vec2(0.0, w::CONTROL_HEIGHT)).vertical_align(egui::Align::Center)
}

/// The width of the captions' column, for hosting and joining alike: the two forms stand one
/// over the other, so their fields start at one place.
fn caption_column(ui: &egui::Ui) -> f32 {
    [ta_ui::i18n::n("BATTLE NAME"), ta_ui::i18n::n("PORT"), ta_ui::i18n::n("PASSWORD"), ta_ui::i18n::n("ADDRESS")].iter().map(|c| w::galley(ui, w::caption(c)).size().x).fold(0.0, f32::max) + 10.0
}

/// A field's caption in a slot `width` wide (a row's fields then start at one place).
fn captioned(ui: &mut egui::Ui, width: f32, caption: &str) {
    ui.allocate_ui_with_layout(vec2(width, w::CONTROL_HEIGHT), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_size(vec2(width, w::CONTROL_HEIGHT));
        ui.label(w::caption(caption));
    });
}

/// Enter pressed in a field just left: what was typed is sent.
fn entered(ui: &egui::Ui, response: &egui::Response) -> bool {
    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

/// The network list with no game in it: a lamp breathing while it listens, and what turns up
/// there; dark when it cannot listen (another program holds the port).
fn searching_row(ui: &mut egui::Ui, listening: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), GAME_ROW), egui::Sense::hover());
    let time = ui.input(|i| i.time) as f32;
    let on = if listening { 0.35 + 0.65 * (0.5 + 0.5 * (time * 2.4).sin()) } else { 0.0 };
    if listening {
        ta_ui::motion::next_frame(ui.ctx(), std::time::Duration::from_millis(40));
    }
    let painter = ui.painter();
    painter.add(ta_ui::metal::lamp(egui::pos2(rect.min.x + 16.0, rect.center().y), 4.0, GREEN, on));
    let (title, line) = if listening {
        (t("SEARCHING YOUR NETWORK…"), t("A lobby hosted on your network shows up here."))
    } else {
        (t("NOT LISTENING ON YOUR NETWORK"), t("Another program holds the network port: join by address below."))
    };
    let title = w::galley(ui, RichText::new(title).font(strong(13.0)).color(STEEL).extra_letter_spacing(ta_ui::i18n::tracking(1.4)));
    let line = w::galley(ui, RichText::new(line).font(body(12.5)).color(STEEL_DIM));
    let x = rect.min.x + 32.0;
    let top = rect.center().y - (title.size().y + line.size().y) / 2.0;
    painter.galley(egui::pos2(x, top), title.clone(), STEEL);
    painter.galley(egui::pos2(x, top + title.size().y), line, STEEL_DIM);
}

/// Why a connection could not be made, in words a player can act on.
pub(super) fn connect_error(error: &std::io::Error, address: &str) -> String {
    use std::io::ErrorKind as K;
    match error.kind() {
        K::ConnectionRefused => tf("Nobody is hosting at {address}: check the address and port, and that the host is still in its lobby.", &[("address", &address)]),
        K::TimedOut | K::WouldBlock => tf("{address} did not answer: check the address, and that the host's router and firewall let its port through.", &[("address", &address)]),
        K::NotFound | K::InvalidInput => tf("{address} is not an address this computer can find.", &[("address", &address)]),
        K::ConnectionAborted | K::ConnectionReset => t("The host closed the connection before answering.").to_string(),
        _ => tf("Could not connect: {error}", &[("error", &error)]),
    }
}

impl ConnectForm {
    /// The form as the settings left it: our name, the address last joined, the games joined
    /// lately, the port last hosted on and the battle's name then.
    pub fn remembered(gameplay: &ta_config::Gameplay) -> ConnectForm {
        let mut form = ConnectForm { name: gameplay.player_name.clone(), address: gameplay.join_address.clone(), host_title: gameplay.host_title.clone(), ..Default::default() };
        if gameplay.host_port != 0 {
            form.port = gameplay.host_port.to_string();
        }
        form.recent = gameplay.recent_games.iter().take(RECENT).map(|g| (g.address.clone(), g.title.clone())).collect();
        form
    }

    /// Starts connecting to `address`, away from the interface.
    fn connect(&mut self, address: String, password: String, from: Place, assets: &Assets) {
        let (tx, result) = mpsc::channel();
        let (name, data_hash, to) = (self.name.clone(), assets.network_hash(), address.clone());
        let started = std::thread::Builder::new().name("mp-connect".into()).spawn(move || {
            // (Connected, and the host's answer heard: welcomed, or why not.)
            let made = ta_net::resolve(&to).and_then(|a| reconnect::connect(a, &name, data_hash, &password)).and_then(|(mut client, key)| {
                client.wait_for_answer(super::ANSWER)?;
                Ok((client, key))
            });
            let _ = tx.send(made);
        });
        match started {
            Ok(_) => {
                self.error = None;
                self.connecting = Some(Connecting { address, from, result });
            }
            Err(e) => self.error = Some((from, tf("Could not connect: {error}", &[("error", &e)]))),
        }
    }

    pub fn draw(&mut self, ui: &mut egui::Ui, assets: &Assets) -> ConnectAction {
        let mut action = ConnectAction::None;
        let ctx = ui.ctx().clone();
        // A connection being made: done, failed, or still going.
        if let Some(c) = &self.connecting {
            match c.result.try_recv() {
                Ok(Ok((client, key))) => {
                    let c = self.connecting.take().unwrap();
                    match &client.rejected {
                        // The password asked for again: on the game's card, or under JOIN BY
                        // ADDRESS with the address filled in.
                        Some(ta_net::Refusal::Password) => {
                            self.password.clear();
                            self.ask_password = true;
                            if c.from == Place::Network {
                                self.unlocking = Some(c.address);
                                self.error = Some((Place::Network, refusal_text(&ta_net::Refusal::Password, assets)));
                            } else {
                                self.address = c.address;
                                self.error = Some((Place::Address, refusal_text(&ta_net::Refusal::Password, assets)));
                            }
                        }
                        Some(reason) => {
                            log::info!("{} turned us away: {reason:?}", c.address);
                            self.error = Some((c.from, refusal_text(reason, assets)));
                        }
                        None => {
                            if c.from != Place::Network {
                                self.keep = Some(Keep::Joined(c.address.clone()));
                            }
                            return ConnectAction::Lobby(NetLobby::joined(client, &self.name, key, Some(c.address)));
                        }
                    }
                }
                Ok(Err(e)) => {
                    let c = self.connecting.take().unwrap();
                    self.error = Some((c.from, connect_error(&e, &c.address)));
                }
                Err(TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(50)),
                Err(TryRecvError::Disconnected) => {
                    self.connecting = None;
                }
            }
        }
        // Games on the local network, announced by their lobbies.
        if !self.lan_tried {
            self.lan_tried = true;
            self.lan = LanBrowser::new().map_err(|e| log::info!("not listening for LAN games: {e}")).ok();
        }
        let busy = self.connecting.is_some();
        let mut join: Option<(String, String, Place)> = None;
        let mut host = false;
        let width = (ctx.content_rect().width() - 40.0).clamp(640.0, 1040.0);
        let wide = width >= 900.0;
        let page = Panel::new("MULTIPLAYER").subtitle("PLAY ON YOUR NETWORK OR OVER THE INTERNET").width(width).exit(Exit::Back);
        let response = panel::page(&ctx, "mp_connect", page, |ui| {
            self.status(ui, Place::Top);
            ui.add_enabled_ui(!busy, |ui| {
                panel::setting(ui, "YOUR NAME", Some("Your name in the lobby and the battle"), |ui| {
                    ui.add(field(&mut self.name, 280.0).char_limit(24));
                });
            });
            ui.add_space(space::XS);
            // (Each part's connecting, or why it could not, under it: the way to give up stays
            // to hand while the rest waits.)
            let column = |ui: &mut egui::Ui, left: bool, this: &mut ConnectForm, join: &mut Option<(String, String, Place)>, host: &mut bool| {
                if left {
                    ui.add_enabled_ui(!busy, |ui| this.network_games(ui, assets, join));
                    this.status(ui, Place::Network);
                    ui.add_space(space::M);
                    ui.add_enabled_ui(!busy, |ui| this.recent_games(ui, join));
                    this.status(ui, Place::Recent);
                } else {
                    ui.add_enabled_ui(!busy, |ui| this.hosting(ui, host));
                    this.status(ui, Place::Hosting);
                    ui.add_space(space::M);
                    panel::rule(ui);
                    ui.add_enabled_ui(!busy, |ui| this.by_address(ui, join));
                    this.status(ui, Place::Address);
                }
            };
            if wide {
                // (The columns' gap, not their rows'.)
                let spacing = ui.spacing().item_spacing.x;
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 28.0;
                    let half = (ui.available_width() - 28.0) / 2.0;
                    ui.vertical(|ui| {
                        ui.set_width(half + 40.0);
                        ui.spacing_mut().item_spacing.x = spacing;
                        column(ui, true, self, &mut join, &mut host);
                    });
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.x = spacing;
                        column(ui, false, self, &mut join, &mut host);
                    });
                });
            } else {
                column(ui, true, self, &mut join, &mut host);
                ui.add_space(space::M);
                column(ui, false, self, &mut join, &mut host);
            }
        });
        if response.exit {
            self.connecting = None;
            action = ConnectAction::Back;
        }
        // Leaving the page stops listening (what is heard meanwhile is stale on return).
        if !matches!(action, ConnectAction::None) {
            self.lan = None;
            self.lan_tried = false;
        }
        // (Everyone in a lobby needs a name.)
        if (host || join.is_some()) && self.name.trim().is_empty() {
            self.error = Some((if host { Place::Hosting } else { join.as_ref().map_or(Place::Top, |j| j.2) }, t("Choose your name first.").to_string()));
            return action;
        }
        let port = self.port.trim().parse::<u16>().ok().filter(|&p| p != 0);
        if host && port.is_none() {
            self.error = Some((Place::Hosting, tf("The port is a number from 1 to 65535 ({port} is the game's own).", &[("port", &ta_net::DEFAULT_PORT)])));
        } else if host && let Some(port) = port {
            match bind_host(port, assets) {
                Ok(mut h) => {
                    self.error = None;
                    h.set_password(Some(&self.host_password));
                    let title = if self.host_title.trim().is_empty() { tf("{name}'s battle", &[("name", &self.name)]) } else { self.host_title.trim().to_string() };
                    self.keep = Some(Keep::Hosted { port, title: self.host_title.trim().to_string() });
                    action = ConnectAction::Lobby(NetLobby::hosting(h, &self.name, assets, &title));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => self.error = Some((Place::Hosting, tf("Port {port} is already in use on this computer (another game hosting?): choose another.", &[("port", &port)]))),
                Err(e) => self.error = Some((Place::Hosting, tf("Could not host on port {port}: {error}", &[("port", &port), ("error", &e)]))),
            }
        }
        if let Some((address, password, from)) = join {
            self.unlocking = None;
            self.connect(address, password, from, assets);
        }
        action
    }

    /// Coming back to a battle whose connection failed: connecting again (the credential kept
    /// for it lets us in).
    pub fn rejoin(&mut self, address: String, assets: &Assets) {
        let password = self.password.clone();
        self.connect(address, password, Place::Top, assets);
    }

    /// Why the multiplayer page was come back to (the lobby left behind, the command line):
    /// told at its top.
    pub fn tell(&mut self, why: String) {
        self.error = Some((Place::Top, why));
    }

    /// What became of the last try from `place`: connecting (with a way to give up), or why it
    /// could not.
    fn status(&mut self, ui: &mut egui::Ui, place: Place) {
        if let Some(c) = self.connecting.as_ref().filter(|c| c.from == place) {
            ui.add_space(space::S);
            let mut cancel = false;
            ui.horizontal(|ui| {
                let time = ui.input(|i| i.time) as f32;
                let (lamp, _) = ui.allocate_exact_size(vec2(16.0, 24.0), egui::Sense::hover());
                ui.painter().add(ta_ui::metal::lamp(lamp.center(), 4.0, AMBER, 0.4 + 0.6 * (0.5 + 0.5 * (time * 5.0).sin())));
                let room = (ui.available_width() - 130.0).max(80.0);
                let text = RichText::new(tf("CONNECTING TO {address}…", &[("address", &c.address)])).font(strong(13.0)).color(AMBER).extra_letter_spacing(ta_ui::i18n::tracking(1.2));
                ui.allocate_ui(vec2(room, 24.0), |ui| ui.add(egui::Label::new(text).truncate()));
                cancel = ui.add(ActionButton::secondary("CANCEL")).clicked();
            });
            if cancel {
                // (The connection, if it comes, is dropped with its thread's message.)
                self.connecting = None;
            }
        } else if let Some((at, why)) = &self.error
            && *at == place
        {
            ui.add_space(if place == Place::Top { 0.0 } else { space::S });
            ui.add(egui::Label::new(RichText::new(why).font(body(13.0)).color(RED)).wrap());
            if place == Place::Top {
                ui.add_space(space::S);
            }
        }
    }

    /// The battles hosted on this network, a card each: their name, host, map, players and
    /// seats, whether they take a password, and JOIN (a password first, for those that do).
    fn network_games(&mut self, ui: &mut egui::Ui, assets: &Assets, join: &mut Option<(String, String, Place)>) {
        panel::section(ui, "GAMES ON YOUR NETWORK");
        let games = self.lan.as_mut().map(|browser| browser.poll().to_vec());
        panel::well(ui, |ui| {
            let room = 3.0 * (GAME_ROW + 6.0);
            let area = egui::ScrollArea::vertical().id_salt("mp_games").max_height(room).min_scrolled_height(room).auto_shrink([false, false]);
            ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Well, |ui| {
                ui.set_width(ui.available_width());
                match &games {
                    Some(games) if games.is_empty() => searching_row(ui, true),
                    None => searching_row(ui, false),
                    Some(_) => {}
                }
                for g in games.into_iter().flatten() {
                    let i = &g.info;
                    let address = g.address.to_string();
                    // (Another version cannot join; other data can try, to be told which of its
                    // mods differ. Maps are not the data: a map pack keeps nobody out.)
                    let same_version = i.protocol == ta_net::PROTOCOL;
                    let same_data = same_version && i.data_hash == assets.network_hash();
                    let map = assets.maps.iter().find(|m| m.name.eq_ignore_ascii_case(&i.map)).map_or(i.map.clone(), |m| m.title.clone());
                    egui::Frame::new().fill(PANEL_HI).stroke(Stroke::new(1.0, LINE)).corner_radius(CornerRadius::same(2)).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let unlocking = self.unlocking.as_deref() == Some(address.as_str());
                        // (Its buttons first, its name and line in what room is left.)
                        egui::Sides::new().shrink_left().truncate().height(40.0).show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 1.0;
                                let title = if i.title.trim().is_empty() { tf("{name}'s battle", &[("name", &i.host)]) } else { i.title.clone() };
                                ui.label(RichText::new(title).font(strong(15.0)).color(TEXT_BRIGHT));
                                let seats = i.seats.max(i.players.saturating_add(i.open));
                                ui.label(RichText::new(tf("{host} · {map} · {players} of {seats} players", &[("host", &i.host), ("map", &map), ("players", &i.players), ("seats", &seats)])).font(body(12.5)).color(STEEL))
                                    .on_hover_text(tf("At {address}", &[("address", &address)]));
                            });
                        }, |ui| {
                            if unlocking {
                                // Its password, then in.
                                let clicked = ui.add(ActionButton::primary("JOIN")).clicked();
                                let edit = ui.add(field(&mut self.password, 120.0).char_limit(64).password(true).hint_text(RichText::new(t("Password")).color(STEEL_DIM)));
                                if std::mem::take(&mut self.ask_password) {
                                    edit.request_focus();
                                }
                                if clicked || entered(ui, &edit) {
                                    *join = Some((address.clone(), self.password.clone(), Place::Network));
                                }
                            } else if ui.add_enabled(same_version, ActionButton::primary("JOIN")).clicked() {
                                if i.locked {
                                    self.unlocking = Some(address.clone());
                                    self.ask_password = true;
                                } else {
                                    *join = Some((address.clone(), String::new(), Place::Network));
                                }
                            }
                            if !same_version {
                                w::tag(ui, "OTHER VERSION", RED).on_hover_text(t("The host runs another version of TA Reborn: you both need the same one."));
                            } else if !same_data {
                                w::tag(ui, "OTHER MODS OR DATA", RED).on_hover_text(t("The host plays with other mods or game data: JOIN to see which of yours differ."));
                            } else if i.open == 0 {
                                w::tag(ui, "FULL · YOU CAN WATCH", STEEL);
                            }
                            // (Asked for, the password needs no tag.)
                            if i.locked && !unlocking {
                                w::tag(ui, "PASSWORD", AMBER);
                            }
                        });
                    });
                }
            });
        });
    }

    /// The games joined lately: JOIN again, or forget one.
    fn recent_games(&mut self, ui: &mut egui::Ui, join: &mut Option<(String, String, Place)>) {
        if self.recent.is_empty() {
            return;
        }
        panel::section(ui, "RECENT GAMES");
        let mut forget = None;
        for (address, title) in &self.recent {
            egui::Sides::new().shrink_left().truncate().height(w::CONTROL_HEIGHT + 4.0).show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    // (Its name once its lobby has told it, and where it is.)
                    if title.trim().is_empty() {
                        ui.label(RichText::new(address).font(strong(13.5)).color(TEXT_BRIGHT));
                    } else {
                        ui.label(RichText::new(title).font(strong(13.5)).color(TEXT_BRIGHT));
                        ui.label(RichText::new(address).font(body(11.5)).color(STEEL_DIM));
                    }
                });
            }, |ui| {
                if w::icon_button(ui, w::Glyph::Close, true).on_hover_text(t("Forget this game")).clicked() {
                    forget = Some(address.clone());
                }
                if ui.add(ActionButton::secondary("JOIN")).on_hover_text(t("Join again (the password under JOIN BY ADDRESS, if it has one)")).clicked() {
                    *join = Some((address.clone(), self.password.clone(), Place::Recent));
                }
            });
            ui.add_space(2.0);
        }
        if let Some(address) = forget {
            self.recent.retain(|(a, _)| *a != address);
            self.keep = Some(Keep::Forget(address));
        }
    }

    /// Hosting: the battle's name, the port and a password (none: anybody may join).
    fn hosting(&mut self, ui: &mut egui::Ui, host: &mut bool) {
        panel::section(ui, "HOST A GAME");
        let caption = caption_column(ui);
        let rest = (ui.available_width() - caption - 8.0).max(120.0);
        let mut enter = false;
        ui.horizontal(|ui| {
            captioned(ui, caption, ta_ui::i18n::n("BATTLE NAME"));
            let hint = tf("{name}'s battle", &[("name", &self.name)]);
            let edit = ui.add(field(&mut self.host_title, rest).char_limit(48).hint_text(RichText::new(hint).color(STEEL_DIM)));
            enter |= entered(ui, &edit);
        });
        ui.add_space(space::XS);
        ui.horizontal(|ui| {
            captioned(ui, caption, ta_ui::i18n::n("PORT"));
            let edit = ui.add(field(&mut self.port, 90.0).char_limit(5));
            enter |= entered(ui, &edit);
        });
        ui.add_space(space::XS);
        ui.horizontal(|ui| {
            captioned(ui, caption, ta_ui::i18n::n("PASSWORD"));
            let edit = ui.add(field(&mut self.host_password, rest.min(300.0)).char_limit(64).password(true).hint_text(RichText::new(t("None: anybody may join")).color(STEEL_DIM)));
            enter |= entered(ui, &edit);
        });
        *host |= enter;
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(ActionButton::primary("HOST").size(140.0, 34.0)).clicked() {
                    *host = true;
                }
            });
        });
        for line in [t("Players on your network see your game listed."), t("Others join with your address and this port (let the port through your firewall).")] {
            ui.add(egui::Label::new(RichText::new(line).font(body(12.5)).color(STEEL_DIM)).wrap());
        }
    }

    /// Joining by address (and the password, if the host set one).
    fn by_address(&mut self, ui: &mut egui::Ui, join: &mut Option<(String, String, Place)>) {
        panel::section(ui, "JOIN BY ADDRESS");
        let caption = caption_column(ui);
        let rest = (ui.available_width() - caption - 8.0).max(120.0);
        let mut enter = false;
        ui.horizontal(|ui| {
            captioned(ui, caption, ta_ui::i18n::n("ADDRESS"));
            let edit = ui.add(field(&mut self.address, rest).hint_text(RichText::new(t("host or host:port")).color(STEEL_DIM)));
            enter |= entered(ui, &edit);
        });
        ui.add_space(space::XS);
        ui.horizontal(|ui| {
            captioned(ui, caption, ta_ui::i18n::n("PASSWORD"));
            let edit = ui.add(field(&mut self.password, rest.min(300.0)).char_limit(64).password(true).hint_text(RichText::new(t("If the host set one")).color(STEEL_DIM)));
            if self.unlocking.is_none() && std::mem::take(&mut self.ask_password) {
                edit.request_focus();
            }
            enter |= entered(ui, &edit);
        });
        ui.add_space(6.0);
        let ready = !self.address.trim().is_empty();
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(ready, ActionButton::primary("JOIN").size(140.0, 34.0)).clicked() || (enter && ready) {
                    *join = Some((self.address.trim().to_string(), self.password.clone(), Place::Address));
                }
            });
        });
    }
}
