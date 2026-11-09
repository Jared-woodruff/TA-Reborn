//! What a player is told of the mods a saved game or replay was made with
//! (`ta_config::records::mods`), the same in the game's saved games and replays and in the
//! launcher's battles: a line for each mod it needs that is not as it was, for each on that it was
//! made without, and for another load order; in a list, the first of them; in a dialog, each with
//! a lamp.

use egui::{Color32, RichText, Ui, vec2};
use ta_config::records::RecordMod;
use ta_config::records::mods::{ModCheck, Need};

use crate::i18n::{t, tf};
use crate::theme::{AMBER, GREEN, RED, STEEL, TEXT, body};

/// How much a line matters (in that order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tone {
    /// As it was.
    Fine,
    /// Off, another copy, or on and not in the battle: the battle would not be the same.
    Warn,
    /// Not installed.
    Bad,
}

impl Tone {
    pub fn color(self) -> Color32 {
        match self {
            Tone::Fine => GREEN,
            Tone::Warn => AMBER,
            Tone::Bad => RED,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub tone: Tone,
    pub text: String,
}

/// "Total Mayhem 11.3.0, TA: Escalation Gold 10.2.0".
pub fn names(mods: &[RecordMod]) -> String {
    mods.iter().map(RecordMod::label).collect::<Vec<_>>().join(", ")
}

/// The line for a mod a record needs (`None` when it is as it was).
pub fn need_line(m: &RecordMod, need: &Need) -> Option<Line> {
    let name = m.label();
    let (tone, text) = match need {
        Need::Same => return None,
        Need::Missing => (Tone::Bad, tf("{mod}: not installed", &[("mod", &name)])),
        Need::Off { version, .. } if !version.is_empty() && *version != m.version => {
            (Tone::Warn, tf("{mod}: off, and installed as version {version}", &[("mod", &name), ("version", version)]))
        }
        Need::Off { .. } => (Tone::Warn, tf("{mod}: installed, but off", &[("mod", &name)])),
        Need::OtherVersion(version) if version.is_empty() => (Tone::Warn, tf("{mod}: another copy of it is on", &[("mod", &name)])),
        Need::OtherVersion(version) => (Tone::Warn, tf("{mod}: version {version} is on", &[("mod", &name), ("version", version)])),
        Need::OtherFiles => (Tone::Warn, tf("{mod}: on, but its files have changed", &[("mod", &name)])),
    };
    Some(Line { tone, text })
}

/// Every line of a check: the mods it needs that are not as they were (in its load order), those
/// on that it was made without, and another load order.
pub fn lines(check: &ModCheck) -> Vec<Line> {
    let mut out: Vec<Line> = check.needed.iter().filter_map(|(m, need)| need_line(m, need)).collect();
    out.extend(check.others().map(|m| Line { tone: Tone::Warn, text: tf("{mod}: on, but the battle was made without it", &[("mod", &m.label())]) }));
    if check.reordered {
        out.push(Line { tone: Tone::Warn, text: t("Its mods are on, in another load order").to_string() });
    }
    out
}

/// For a row of a list: the first line ("…, and 2 more"), in the tone of the worst; or, when its
/// mods are all as they were, which they are ("Mods: Total Mayhem 11.3.0"); `None` for a record
/// made without mods with none on.
pub fn summary(check: &ModCheck) -> Option<Line> {
    let lines = lines(check);
    match lines.split_first() {
        Some((first, [])) => Some(first.clone()),
        Some((first, rest)) => {
            let tone = lines.iter().map(|l| l.tone).max().unwrap_or(first.tone);
            Some(Line { tone, text: tf("{first} (and {n} more)", &[("first", &first.text), ("n", &rest.len())]) })
        }
        None if check.needed.is_empty() => None,
        None => {
            let mods: Vec<RecordMod> = check.needed.iter().map(|(m, _)| m.clone()).collect();
            Some(Line { tone: Tone::Fine, text: tf("Mods: {mods}", &[("mods", &names(&mods))]) })
        }
    }
}

/// The lines in a dialog, each by a lamp of its tone; at most `most`, then how many more.
pub fn show(ui: &mut Ui, lines: &[Line], most: usize) {
    ui.spacing_mut().item_spacing.y = 5.0;
    for line in lines.iter().take(most) {
        ui.horizontal_top(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(16.0, 18.0), egui::Sense::hover());
            ui.painter().add(crate::metal::lamp(rect.center(), 4.5, line.tone.color(), 1.0));
            ui.add(egui::Label::new(RichText::new(&line.text).font(body(14.5)).color(TEXT)).wrap());
        });
    }
    if lines.len() > most {
        ui.label(RichText::new(tf("And {n} more", &[("n", &(lines.len() - most))])).font(body(13.0)).color(STEEL));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_config::records::mods::check;

    fn m(id: &str, version: &str, digest: u8) -> RecordMod {
        RecordMod { id: id.into(), name: format!("Mod {id}"), version: version.into(), digest: [digest; 16] }
    }

    #[test]
    fn a_list_tells_the_worst_first_and_how_many_more() {
        let record = [m("a", "1", 1), m("b", "2", 2)];
        // b off, a missing, c on that the battle was made without.
        let lines = lines(&check(&record, &[m("c", "1", 3)], &[m("b", "2", 2), m("c", "1", 3)]));
        assert_eq!(lines.iter().map(|l| l.tone).collect::<Vec<_>>(), [Tone::Bad, Tone::Warn, Tone::Warn]);
        assert_eq!(lines[0].text, "Mod a 1: not installed");
        assert_eq!(lines[1].text, "Mod b 2: installed, but off");
        assert_eq!(lines[2].text, "Mod c 1: on, but the battle was made without it");
        let row = summary(&check(&record, &[m("c", "1", 3)], &[m("b", "2", 2), m("c", "1", 3)])).unwrap();
        assert_eq!((row.tone, row.text.as_str()), (Tone::Bad, "Mod a 1: not installed (and 2 more)"));
        // All as they were: which mods it uses; made without, none on: nothing.
        assert_eq!(summary(&check(&record, &record, &record)).unwrap().text, "Mods: Mod a 1, Mod b 2");
        assert_eq!(summary(&check(&[], &[], &record)), None);
    }

    #[test]
    fn other_copies_say_which() {
        let record = [m("a", "1", 1)];
        let text = |on: &[RecordMod], installed: &[RecordMod]| lines(&check(&record, on, installed)).remove(0).text;
        assert_eq!(text(&[m("a", "2", 9)], &[]), "Mod a 1: version 2 is on");
        assert_eq!(text(&[m("a", "1", 9)], &[]), "Mod a 1: on, but its files have changed");
        assert_eq!(text(&[m("a", "", 9)], &[]), "Mod a 1: another copy of it is on");
        assert_eq!(text(&[], &[m("a", "2", 0)]), "Mod a 1: off, and installed as version 2");
    }
}
