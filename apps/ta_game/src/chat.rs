//! The chat line: messages (to everyone or to allies in multiplayer; in single player to the
//! computer players, who may answer: see `chatter.rs`) and TA's cheat codes, lines starting
//! with `+` (`+atm`, `+nowisee`...), with TA Reborn's own (`+god`, `+spawn`...).

use glam::Vec3;
use ta_sim::{Cheat, Command, Sim};
use ta_ui::i18n::{n, t, tf};

use crate::game::{ALLY_PREFIX, Game, fx_vec};

/// A line on the chat.
#[derive(Clone, Debug)]
pub struct ChatLine {
    /// Who said it (empty for the game's own answers to cheat codes).
    pub from: String,
    pub text: String,
    /// Seconds it stays up.
    pub secs: f32,
    /// The speaker, for their colour, when known.
    pub player: Option<u8>,
    /// A computer player's transmission: the speaker's title (translated where shown). It
    /// comes in on its own card (`hud/comms.rs`), and stays in the chat's history.
    pub title: Option<&'static str>,
}

impl ChatLine {
    pub fn said(from: String, text: String, player: Option<u8>) -> ChatLine {
        ChatLine { from, text, secs: 12.0, player, title: None }
    }

    /// A computer player's line, from its persona, `title` under its name.
    pub fn transmission(from: String, title: &'static str, text: String, player: u8) -> ChatLine {
        ChatLine { from, text, secs: 12.0, player: Some(player), title: Some(title) }
    }

    /// The game's answer to a cheat code.
    pub fn answer(text: String) -> ChatLine {
        ChatLine { from: String::new(), text, secs: 8.0, player: None, title: None }
    }
}

/// Seeing more than the rules allow: TA's `+radar`, `+nowisee` and `+view`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reveal {
    #[default]
    Off,
    /// Every unit shows on radar.
    Radar,
    /// Everything is in sight.
    All,
    /// What another player sees.
    Player(u8),
}

/// Cheats that change only what this computer shows or plays, not the battle.
#[derive(Clone, Debug, Default)]
pub struct ViewCheats {
    pub reveal: Reveal,
    /// No camera shake from blasts (TA's `+noshake`).
    pub no_shake: bool,
    /// Units sing their acknowledgements (TA's `+sing`).
    pub sing: bool,
}

/// The music the chat line asks for (TA's `+cd...` codes, from its CD player days).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MusicRequest {
    Start,
    Stop,
    Next,
    Random,
    Track(String),
    /// A briefing's narration (a VFS path), over the music.
    Narration(String),
    /// The track before (the jukebox).
    Previous,
}

/// The cheat panel's spawn tool: each click on the map places this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpawnTool {
    pub type_id: usize,
    pub count: u16,
    pub owner: u8,
}

/// Lines kept on the chat.
const CHAT_LINES: usize = 8;

/// The codes `+help` lists.
/// TA's chat commands that are not cheats (see `Game::ta_command`).
const COMMANDS: &str = "+sharemetal [N]  +shareenergy [N]  +setsharemetal N  +setshareenergy N  +sharemapping  +shootall  +switchalt  +contour  +clock  +noshake  +cdstart  +cdstop  +cdnext  +cdrandom  +cdplay N  +sing";
const CODES: &str = "+atm  +nowisee  +radar  +doubleshot  +halfshot  +view N  +control N  ·  +god  +instant  +unlimited  +heal  +elite  +kill  +unlock  +win  +spawn UNIT [COUNT] [ENEMY]  +cheats";

impl Game {
    /// Puts a line on the chat.
    pub fn post_chat(&mut self, line: ChatLine) {
        // Someone else's line (a computer player's, another player's): TA's message beep.
        if !line.from.is_empty() && line.player != Some(self.local_player) {
            self.chat_beep = true;
        }
        if !line.from.is_empty() {
            log::info!("chat: {}: {}", line.from, line.text);
        }
        self.chat_history.push(line.clone());
        if self.chat_history.len() > 300 {
            self.chat_history.remove(0);
        }
        self.chat_log.push(line);
        if self.chat_log.len() > CHAT_LINES {
            self.chat_log.remove(0);
        }
    }

    /// Opens the chat line (to allies only in multiplayer when asked). The key that opened
    /// it must not also send it, the same frame.
    pub fn open_chat(&mut self, allies: bool) {
        if self.chat_input.is_none() && (self.playback.is_none() || self.net.is_some()) {
            self.chat_input = Some(String::new());
            // (To allies: other people, or, alone, our computer allies.)
            self.chat_allies = allies && !self.spectator && (self.net.is_some() || !self.computer_allies().is_empty());
            self.chat_fresh = true;
        }
    }

    /// The local player's name on the chat.
    pub fn speaker_name(&self) -> String {
        if self.player_name.trim().is_empty() { t("Commander").to_string() } else { self.player_name.trim().to_string() }
    }

    /// Sends a chat line to everyone, or to our allies only; a line starting with `+` is a
    /// cheat code. In single player the line goes on the chat for the computer players to
    /// read.
    pub fn say(&mut self, text: &str, allies: bool) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        if let Some(code) = text.strip_prefix('+') {
            // TA's own commands first (they are not cheats: multiplayer takes them), then the
            // cheat codes.
            let answer = match self.ta_command(code) {
                Some(answer) => answer,
                None => self.cheat_code(code),
            };
            if !answer.is_empty() {
                self.post_chat(ChatLine::answer(answer));
            }
            return;
        }
        let name = self.speaker_name();
        let me = self.local_player;
        if let Some(net) = self.net.as_mut() {
            let line = if allies { format!("{ALLY_PREFIX}{me} {text}") } else { text.to_string() };
            net.send_chat(&name, &line);
            return;
        }
        if allies {
            // (A line for our computer allies: they say how they take orders.)
            self.post_chat(ChatLine::said(ta_ui::i18n::tf("{name} (allies)", &[("name", &name)]), text.to_string(), Some(me)));
            self.computer_allies_hear();
            return;
        }
        self.post_chat(ChatLine::said(name, text.to_string(), Some(me)));
        let now = self.game_secs();
        if let Some(c) = self.chatter.as_mut() {
            c.heard(&self.sim, text, now);
        }
    }

    /// Game time in seconds.
    pub fn game_secs(&self) -> f32 {
        self.sim.tick as f32 / 30.0
    }

    /// Whether this game takes cheats: a live game with one person in it.
    pub fn cheats_available(&self) -> bool {
        self.net.is_none() && self.playback.is_none() && !self.spectator && self.sim.cheats_allowed
    }

    /// Issues a cheat for the local player (the computer players notice).
    pub fn cheat(&mut self, cheat: Cheat) {
        if !self.cheats_available() {
            return;
        }
        let now = self.game_secs();
        if let Some(c) = self.chatter.as_mut() {
            c.cheated(now);
        }
        self.issue(Command::Cheat { player: self.local_player, cheat });
    }

    /// The local player's standing cheats (as of the last tick).
    pub fn my_cheats(&self) -> ta_sim::Cheats {
        self.sim.players.get(self.local_player as usize).map(|p| p.cheats.clone()).unwrap_or_default()
    }

    /// Whether any cheat was used, by the simulation's count or this computer's.
    pub fn cheated(&self) -> bool {
        self.view_cheats_used || self.sim.players.get(self.local_player as usize).is_some_and(|p| p.cheats.used)
    }

    /// Shows more than the rules allow (see [`Reveal`]).
    pub fn set_reveal(&mut self, reveal: Reveal) {
        if reveal != Reveal::Off {
            self.view_cheats_used = true;
            if let Some(c) = self.chatter.as_mut() {
                c.cheated(self.sim.tick as f32 / 30.0);
            }
        }
        self.view_cheats.reveal = reveal;
    }

    /// The first undefeated enemy of the local player, else anyone else.
    pub fn first_enemy(&self) -> Option<u8> {
        let me = self.local_player;
        let players = &self.sim.players;
        players.iter().find(|p| self.sim.is_enemy(me, p.index) && !p.defeated).or_else(|| players.iter().find(|p| p.index != me)).map(|p| p.index)
    }

    /// Where a spawn without a place goes: the pointer's ground, else the Commander, else the
    /// middle of the map.
    fn spawn_spot(&self) -> Vec3 {
        if let Some(g) = self.cursor_ground {
            return g;
        }
        if let Some(u) = self.local_commander().and_then(|c| self.sim.unit(c)) {
            return Vec3::from_array(u.pos.to_f32()) + Vec3::new(0.0, 0.0, 96.0);
        }
        let (w, h) = self.sim.world.size_px();
        Vec3::new(w.to_f32() / 2.0, 0.0, h.to_f32() / 2.0)
    }

    /// TA's chat commands that are not cheats (the text after the `+`): sharing resources with
    /// allies (`+sharemetal`, `+shareenergy`, `+setsharemetal N`, `+setshareenergy N`);
    /// `+sharemapping`, `+shootall`, `+switchalt`, `+contour` and `+clock`, which TA Reborn
    /// always does or does on a key; and what only changes this computer (`+noshake`, the
    /// music's `+cd...`, `+sing`). None when `code` is none of them.
    pub fn ta_command(&mut self, code: &str) -> Option<String> {
        let words: Vec<&str> = code.split_whitespace().collect();
        let first = words.first()?.to_ascii_lowercase();
        let number = || words.get(1).and_then(|w| w.parse::<f32>().ok()).filter(|v| v.is_finite() && *v >= 0.0);
        let sharing = |game: &Self| game.sim.players.get(game.local_player as usize).map(|p| p.sharing).unwrap_or_default();
        let answer = match first.as_str() {
            "sharemetal" | "shareenergy" | "setsharemetal" | "setshareenergy" => {
                if self.spectator || self.playback.is_some() {
                    return Some(t("Spectators have nothing to share").to_string());
                }
                let mut s = sharing(self);
                let metal = first.ends_with("metal");
                // (TA's readme: "+sharemetal 5000 will assure that you have 5000 metal": a
                // number makes either form keep that much.)
                let answer = match (first.starts_with("set") || number().is_some(), number()) {
                    (true, None) => return Some(if metal { t("How much to keep? (+setsharemetal 5000)") } else { t("How much to keep? (+setshareenergy 5000)") }.to_string()),
                    (true, Some(keep)) => {
                        let kept = Some(ta_sim::Fx::from_f32(keep));
                        let n = format!("{keep:.0}");
                        if metal {
                            (s.metal, s.keep_metal) = (true, kept);
                            tf("Sharing your metal above {n} with your allies", &[("n", &n)])
                        } else {
                            (s.energy, s.keep_energy) = (true, kept);
                            tf("Sharing your energy above {n} with your allies", &[("n", &n)])
                        }
                    }
                    (false, _) => {
                        let on = if metal { !s.metal } else { !s.energy };
                        if metal {
                            (s.metal, s.keep_metal) = (on, None);
                        } else {
                            (s.energy, s.keep_energy) = (on, None);
                        }
                        match (metal, on) {
                            (true, true) => t("Sharing the metal your storage cannot hold with your allies").to_string(),
                            (true, false) => t("No longer sharing metal").to_string(),
                            (false, true) => t("Sharing the energy your storage cannot hold with your allies").to_string(),
                            (false, false) => t("No longer sharing energy").to_string(),
                        }
                    }
                };
                self.issue(Command::SetSharing { player: self.local_player, sharing: s });
                answer
            }
            "sharemapping" => t("Allies always share what they see and have explored").to_string(),
            "shootall" => t("Your units already shoot at whatever enemy they see, buildings included (F sets their fire orders)").to_string(),
            "switchalt" => t("Control groups answer to 0-9 and to Alt+0-9 alike").to_string(),
            "contour" => {
                let on = self.overlay != Some(crate::overlay::Overlay::Height);
                self.overlay = on.then_some(crate::overlay::Overlay::Height);
                if on { tf("Heights shown ({key} toggles them)", &[("key", &self.keys.text("overlay_height"))]) } else { t("Heights hidden").to_string() }
            }
            "clock" => t("The game time is always on the top bar").to_string(),
            // These only change this computer: any game takes them.
            "noshake" => {
                self.view_cheats.no_shake = !self.view_cheats.no_shake;
                t(if self.view_cheats.no_shake { "The screen no longer shakes" } else { "The screen shakes again" }).to_string()
            }
            "cdstart" => self.music(MusicRequest::Start, n("Music on")),
            "cdstop" => self.music(MusicRequest::Stop, n("Music off")),
            "cdnext" => self.music(MusicRequest::Next, n("Next track")),
            "cdrandom" => self.music(MusicRequest::Random, n("A track at random")),
            // (The rest of the line: a path may have spaces in it.)
            "cdplay" | "cdtrack" => match Some(words[1..].join(" ")).filter(|w| !w.is_empty()) {
                Some(track) => self.music(MusicRequest::Track(track), n("Playing the track")),
                None => t("Which track? (+cdplay 3)").to_string(),
            },
            "sing" => {
                self.view_cheats.sing = !self.view_cheats.sing;
                t(if self.view_cheats.sing { "Your units sing" } else { "Your units speak again" }).to_string()
            }
            _ => return None,
        };
        Some(answer)
    }

    /// Runs a cheat code (the text after the `+`); returns what it did.
    pub fn cheat_code(&mut self, code: &str) -> String {
        let words: Vec<&str> = code.split_whitespace().collect();
        let Some(first) = words.first() else { return String::new() };
        let code = first.to_ascii_lowercase();
        let args = &words[1..];
        if code == "help" || code == "?" {
            if !self.cheats_available() {
                return tf("Commands: {commands}", &[("commands", &COMMANDS)]);
            }
            return tf("Commands: {commands}
Cheat codes: {codes}", &[("commands", &COMMANDS), ("codes", &CODES)]);
        }
        if !self.cheats_available() {
            return t("Cheats are for games with one person in them").to_string();
        }
        let now = self.game_secs();
        let me = self.local_player;
        let mine = self.my_cheats();
        let on_off = |on: bool, yes: &str, no: &str| t(if on { yes } else { no }).to_string();
        let number = |i: usize| args.get(i).and_then(|a| a.parse::<usize>().ok());
        let player = |n: Option<usize>| n.filter(|&n| n >= 1 && n <= self.sim.players.len()).map(|n| (n - 1) as u8);
        match code.as_str() {
            "atm" => {
                self.cheat(Cheat::Resources);
                t("Metal and energy storage filled").to_string()
            }
            "nowisee" => {
                let on = self.view_cheats.reveal != Reveal::All;
                self.set_reveal(if on { Reveal::All } else { Reveal::Off });
                on_off(on, n("Everything is in sight"), n("Line of sight as the rules have it"))
            }
            "radar" => {
                let on = self.view_cheats.reveal != Reveal::Radar;
                self.set_reveal(if on { Reveal::Radar } else { Reveal::Off });
                on_off(on, n("Every unit on radar"), n("Radar as the rules have it"))
            }
            "doubleshot" | "halfshot" => {
                let shift = if code == "doubleshot" { 1 } else { -1 };
                let on = mine.damage != shift;
                self.cheat(Cheat::Damage(if on { shift } else { 0 }));
                match (on, shift) {
                    (true, 1) => t("Your weapons do double damage").to_string(),
                    (true, _) => t("Your weapons do half damage").to_string(),
                    _ => t("Your weapons do normal damage").to_string(),
                }
            }
            "view" => match player(number(0)) {
                Some(p) if p != me => {
                    self.set_reveal(Reveal::Player(p));
                    tf("Seeing what {player} sees", &[("player", &self.player_label(p))])
                }
                _ => {
                    self.set_reveal(Reveal::Off);
                    t("Seeing with your own eyes").to_string()
                }
            },
            "control" => match player(number(0)) {
                Some(p) if !self.sim.players[p as usize].defeated => {
                    self.view_cheats_used = true;
                    if let Some(c) = self.chatter.as_mut() {
                        c.cheated(now);
                    }
                    self.selection.clear();
                    self.local_player = p;
                    tf("You now command player {n}", &[("n", &(p + 1))])
                }
                _ => t("No such player (+control 1 is the first)").to_string(),
            },
            "god" => {
                self.cheat(Cheat::Invulnerable(!mine.invulnerable));
                on_off(!mine.invulnerable, n("God mode: your units take no harm"), n("God mode off"))
            }
            "instant" | "instantbuild" => {
                self.cheat(Cheat::InstantBuild(!mine.instant_build));
                on_off(!mine.instant_build, n("Instant build: construction is done at once, for nothing"), n("Instant build off"))
            }
            "unlimited" => {
                self.cheat(Cheat::Unlimited(!mine.unlimited));
                on_off(!mine.unlimited, n("Unlimited resources: your storage stays full"), n("Unlimited resources off"))
            }
            "heal" | "elite" | "veteran" | "promote" | "kill" | "destroy" => {
                let units = self.selection.clone();
                if units.is_empty() {
                    return t("Select units first").to_string();
                }
                let count = units.len();
                let (cheat, text) = match code.as_str() {
                    "heal" => (Cheat::Heal(units), n("{n} units mended")),
                    "kill" | "destroy" => (Cheat::Destroy(units), n("{n} units destroyed")),
                    _ => (Cheat::Promote(units), n("{n} units made elite")),
                };
                self.cheat(cheat);
                tf(text, &[("n", &count)])
            }
            "unlock" => {
                self.cheat(Cheat::Unlock);
                t("Every unit may be built").to_string()
            }
            "win" => {
                self.cheat(Cheat::Win);
                t("Your enemies surrender").to_string()
            }
            "spawn" | "give" => self.spawn_code(args),
            "cheats" => {
                self.cheat_panel = true;
                String::new()
            }
            _ => tf("Unknown code +{code} (+help lists them)", &[("code", &code)]),
        }
    }

    fn music(&mut self, request: MusicRequest, answer: &str) -> String {
        self.music_requests.push(request);
        t(answer).to_string()
    }

    /// `+spawn UNIT [COUNT] [ENEMY|ALLY]`: units at the pointer.
    fn spawn_code(&mut self, args: &[&str]) -> String {
        let mut words: Vec<&str> = args.to_vec();
        let mut owner = self.local_player;
        if let Some(last) = words.last().map(|w| w.to_ascii_lowercase()) {
            match last.as_str() {
                "enemy" | "enemies" | "them" => {
                    owner = self.first_enemy().unwrap_or(owner);
                    words.pop();
                }
                "me" | "mine" | "you" => {
                    words.pop();
                }
                _ => {}
            }
        }
        let count = match words.last().and_then(|w| w.parse::<u16>().ok()) {
            Some(n) => {
                words.pop();
                n.clamp(1, ta_sim::cheats::MAX_SPAWN)
            }
            None => 1,
        };
        let query = words.join(" ");
        if query.is_empty() {
            return t("Spawn what? (+spawn peewee 5)").to_string();
        }
        let Some(&type_id) = find_unit_types(&self.sim, &query).first() else {
            return tf("No unit called {name}", &[("name", &query)]);
        };
        let pos = self.spawn_spot();
        self.cheat(Cheat::Spawn { type_id, owner, pos: fx_vec(pos), count });
        let name = unit_label(&self.sim, type_id);
        if owner == self.local_player {
            tf("{count} × {unit} for you", &[("count", &count), ("unit", &name)])
        } else {
            tf("{count} × {unit} for {player}", &[("count", &count), ("unit", &name), ("player", &self.player_label(owner))])
        }
    }

    /// The spawn tool's clicks: the left button places, the right one puts the tool away
    /// (Escape too, see `handle_input`). Returns whether it took the input.
    pub fn spawn_tool_input(&mut self, input: &crate::input::Input, ui_has_pointer: bool) -> bool {
        use crate::input::MouseButtonId;
        let Some(tool) = self.spawn_tool else { return false };
        if !self.cheats_available() || input.button_pressed(MouseButtonId::Right) {
            self.spawn_tool = None;
            return true;
        }
        if input.button_pressed(MouseButtonId::Left) && !ui_has_pointer && let Some(g) = self.cursor_ground {
            self.cheat(Cheat::Spawn { type_id: tool.type_id, owner: tool.owner, pos: fx_vec(g), count: tool.count });
        }
        true
    }
}

/// A unit type as players know it: its name, else its key.
pub fn unit_label(sim: &Sim, type_id: usize) -> String {
    let t = sim.types.get(type_id);
    if t.def.display_name.trim().is_empty() { t.name.clone() } else { t.def.display_name.trim().to_string() }
}

/// Unit types matching a search, best first: by key or name, whole, then the start of a
/// word, then anywhere (the description counts last); among equals by name.
pub fn find_unit_types(sim: &Sim, query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    let mut found: Vec<(u8, String, usize)> = sim
        .types
        .types
        .iter()
        .filter_map(|ty| {
            let key = ty.name.to_lowercase();
            let name = ty.def.display_name.trim().to_lowercase();
            let rank = if q.is_empty() {
                3
            } else if key == q || name == q {
                0
            } else if name.starts_with(&q) || key.starts_with(&q) || name.split_whitespace().any(|w| w.starts_with(&q)) {
                1
            } else if name.contains(&q) || key.contains(&q) {
                2
            } else if ty.def.description.to_lowercase().contains(&q) {
                3
            } else {
                return None;
            };
            let sort = if name.is_empty() { key } else { name };
            Some((rank, sort, ty.id))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, _, id)| id).collect()
}
