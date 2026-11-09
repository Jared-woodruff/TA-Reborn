//! The lobby's chat: what people say, their names in their colours, and what happens (who
//! came and went, who is ready, the map, the countdown), told in each reader's own language
//! from the changes to the lobby they see, every line with the time it came.

use egui::text::LayoutJob;
use egui::{RichText, TextFormat};
use ta_net::{Lobby, MapStatus, SlotKind};
use ta_ui::i18n::{t, tf};

use crate::palette::team_color;
use crate::theme::*;

/// The most lines kept.
const KEEP: usize = 500;

/// Something said or done in the lobby.
#[derive(Clone, Debug, PartialEq)]
pub enum What {
    Said { from: String, text: String },
    Connecting,
    Connected,
    Joined(String),
    Left(String),
    /// Taken out of the lobby by the host.
    Removed(String),
    Ready(String),
    NotReady(String),
    Watching(String),
    Playing(String),
    /// The host chose this map (its title).
    Map(String),
    /// This person does not have the battle's map, or has another version of it.
    NoMap(String),
    OtherMap(String),
    /// The battle starts in this many seconds.
    Starting(u8),
    CalledOff,
    /// Back in the lobby after a battle.
    Back,
}

impl What {
    /// The line in the reader's language (a person's words as they said them).
    fn text(&self) -> String {
        match self {
            What::Said { text, .. } => text.clone(),
            What::Connecting => t("Connecting…").to_string(),
            What::Connected => t("Connected. Choose your side and colour, then get ready.").to_string(),
            What::Back => t("Back in the lobby after the battle: get ready for the next one.").to_string(),
            What::Joined(name) => tf("{name} joined", &[("name", name)]),
            What::Left(name) => tf("{name} left", &[("name", name)]),
            What::Removed(name) => tf("{name} was removed by the host", &[("name", name)]),
            What::Ready(name) => tf("{name} is ready", &[("name", name)]),
            What::NotReady(name) => tf("{name} is not ready", &[("name", name)]),
            What::Watching(name) => tf("{name} will watch", &[("name", name)]),
            What::Playing(name) => tf("{name} will play", &[("name", name)]),
            What::Map(title) => tf("The battlefield is now {map}", &[("map", title)]),
            What::NoMap(name) => tf("{name} does not have this map", &[("name", name)]),
            What::OtherMap(name) => tf("{name} has another version of this map", &[("name", name)]),
            What::Starting(n) => tf("The battle starts in {n} seconds", &[("n", n)]),
            What::CalledOff => t("The countdown was called off").to_string(),
        }
    }
}

/// A line of the chat, with the time (hour, minute) it came.
#[derive(Clone, Debug)]
pub struct Line {
    pub time: (u16, u16),
    pub what: What,
}

#[derive(Default)]
pub struct Chat {
    pub lines: Vec<Line>,
    /// What we are typing.
    pub draft: String,
}

impl Chat {
    pub fn push(&mut self, what: What) {
        self.lines.push(Line { time: ta_ui::seasonal::local_clock(), what });
        if self.lines.len() > KEEP {
            self.lines.drain(..self.lines.len() - KEEP);
        }
    }

    pub fn said(&mut self, from: &str, text: &str) {
        self.push(What::Said { from: from.to_string(), text: text.to_string() });
    }

    /// What changed from `before` to `after`, told: comings and goings (`removed`: names the
    /// host took out, said so), readiness, playing or watching, the map (`title` of a map's
    /// name), the countdown.
    pub fn changes(&mut self, before: &Lobby, after: &Lobby, removed: &[String], title: impl Fn(&str) -> String) {
        let people = |l: &Lobby| -> Vec<(String, Option<bool>)> {
            let players = l.slots.iter().filter_map(|s| match &s.kind {
                SlotKind::Human { name, .. } => Some((name.clone(), Some(s.ready))),
                _ => None,
            });
            players.chain(l.spectators.iter().map(|(_, name)| (name.clone(), None))).collect()
        };
        let (was, now) = (people(before), people(after));
        let find = |list: &[(String, Option<bool>)], name: &str| list.iter().find(|(n, _)| n == name).map(|(_, ready)| *ready);
        for (name, ready) in &now {
            match find(&was, name) {
                None => self.push(What::Joined(name.clone())),
                Some(was_ready) => match (was_ready, ready) {
                    (Some(false), Some(true)) => self.push(What::Ready(name.clone())),
                    (Some(true), Some(false)) => self.push(What::NotReady(name.clone())),
                    (Some(_), None) => self.push(What::Watching(name.clone())),
                    (None, Some(_)) => self.push(What::Playing(name.clone())),
                    _ => {}
                },
            }
        }
        for (name, _) in &was {
            if find(&now, name).is_none() {
                self.push(if removed.iter().any(|r| r == name) { What::Removed(name.clone()) } else { What::Left(name.clone()) });
            }
        }
        if before.map != after.map && !after.map.is_empty() && !before.map.is_empty() {
            self.push(What::Map(title(&after.map)));
        }
        // Who lacks the battle's map, or has another version of it, as the host finds it: told
        // once for each map chosen.
        let persons = |l: &Lobby| -> Vec<(u8, String)> {
            l.slots.iter().filter_map(|s| match &s.kind {
                SlotKind::Human { peer, name } => Some((*peer, name.clone())),
                _ => None,
            }).chain(l.spectators.iter().cloned()).collect()
        };
        for (peer, name) in persons(after) {
            let was = if before.map == after.map { before.map_status_of(peer) } else { MapStatus::Checking };
            match after.map_status_of(peer) {
                now if now == was => {}
                MapStatus::Missing => self.push(What::NoMap(name)),
                MapStatus::Other => self.push(What::OtherMap(name)),
                _ => {}
            }
        }
        match (before.countdown, after.countdown) {
            (None, Some(n)) => self.push(What::Starting(n)),
            (Some(n), None) if n > 1 => self.push(What::CalledOff),
            _ => {}
        }
    }

    /// The lines, oldest first, in `lobby`'s colours: a person's name in their player colour
    /// (a spectator's in steel), what happened in quiet italics.
    pub fn show(&self, ui: &mut egui::Ui, lobby: &Lobby) {
        let colour_of = |name: &str| {
            lobby.slots.iter().find_map(|s| match &s.kind {
                SlotKind::Human { name: n, .. } if n == name => Some(team_color(s.color)),
                _ => None,
            })
        };
        let width = ui.available_width();
        for line in &self.lines {
            let mut job = LayoutJob::default();
            job.wrap.max_width = width;
            let (h, m) = line.time;
            job.append(&format!("{h:02}:{m:02}  "), 0.0, TextFormat::simple(body(11.5), STEEL_DIM));
            match &line.what {
                What::Said { from, text } => {
                    let colour = colour_of(from).unwrap_or(STEEL);
                    job.append(from, 0.0, TextFormat::simple(strong(13.5), colour));
                    job.append(&format!("  {text}"), 0.0, TextFormat::simple(body(13.5), TEXT));
                }
                other => {
                    let mut format = TextFormat::simple(body(13.0), STEEL);
                    format.italics = true;
                    job.append(&other.text(), 0.0, format);
                }
            }
            ui.add(egui::Label::new(job).wrap());
        }
    }

    /// The line being typed and the hint under it; returns a line to send (Enter).
    pub fn input(&mut self, ui: &mut egui::Ui) -> Option<String> {
        let resp = ui.add(
            egui::TextEdit::singleline(&mut self.draft)
                .desired_width(ui.available_width())
                .char_limit(200)
                .hint_text(RichText::new(t("Say something… (Enter sends)")).color(STEEL_DIM))
                .margin(egui::Margin::symmetric(8, 6)),
        );
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let text: String = self.draft.trim().chars().take(200).collect();
            self.draft.clear();
            // (Typing on: the field keeps the keyboard.)
            resp.request_focus();
            if !text.is_empty() {
                return Some(text);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_net::LobbySlot;

    fn lobby(people: &[(&str, bool)], watching: &[&str]) -> Lobby {
        Lobby {
            map: "Painted Desert".into(),
            slots: people.iter().enumerate().map(|(i, (name, ready))| LobbySlot { kind: SlotKind::Human { peer: i as u8, name: name.to_string() }, side: "ARM".into(), color: i as u8, ready: *ready, team: i as u8 }).collect(),
            spectators: watching.iter().enumerate().map(|(i, name)| (20 + i as u8, name.to_string())).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn the_lobby_tells_what_changed() {
        let mut chat = Chat::default();
        let before = lobby(&[("Valkyrie", true), ("Ironside", false)], &[]);
        let mut after = lobby(&[("Valkyrie", true), ("Ironside", true), ("Nova", false)], &["Hex"]);
        after.map = "Coast To Coast".into();
        after.countdown = Some(5);
        chat.changes(&before, &after, &[], |m| m.to_uppercase());
        let what: Vec<What> = chat.lines.iter().map(|l| l.what.clone()).collect();
        assert_eq!(what, [What::Ready("Ironside".into()), What::Joined("Nova".into()), What::Joined("Hex".into()), What::Map("COAST TO COAST".into()), What::Starting(5)]);
        let mut chat = Chat::default();
        let gone = lobby(&[("Valkyrie", true)], &["Ironside"]);
        chat.changes(&after, &gone, &["Nova".into()], |m| m.into());
        let what: Vec<What> = chat.lines.iter().map(|l| l.what.clone()).collect();
        assert_eq!(what, [What::Watching("Ironside".into()), What::Removed("Nova".into()), What::Left("Hex".into()), What::Map("Painted Desert".into()), What::CalledOff]);
    }
}
