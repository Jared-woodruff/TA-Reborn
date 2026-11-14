//! The battle's map lists, skirmish's and the lobby's: each map tagged with where it comes from
//! (TA's own, an expansion's, TA Reborn's, Supreme Commander's, a mod's), and the maps shown
//! chosen by words and by origin and put in an order, so the right map for a game is quick to
//! find.

use egui::{Color32, Response, RichText, Ui};
use ta_data::map::Origin;
use ta_ui::i18n::{n, t, tf};
use ta_ui::panel;
use ta_ui::theme::{AMBER_BRIGHT, CREAM, GREEN, SKY, STEEL, strong};
use ta_ui::widgets as w;

use crate::setup::MapEntry;

/// Which maps a list shows, by where they come from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Origins {
    #[default]
    All,
    /// Total Annihilation's own.
    Ta,
    /// Its expansions': Core Contingency's and Battle Tactics'.
    Expansions,
    Reborn,
    SupremeCommander,
    Mods,
}

impl Origins {
    const ALL: [Origins; 6] = [Origins::All, Origins::Ta, Origins::Expansions, Origins::Reborn, Origins::SupremeCommander, Origins::Mods];

    fn accepts(self, origin: Origin) -> bool {
        match self {
            Origins::All => true,
            Origins::Ta => origin == Origin::TotalAnnihilation,
            Origins::Expansions => matches!(origin, Origin::CoreContingency | Origin::BattleTactics),
            Origins::Reborn => origin == Origin::Reborn,
            Origins::SupremeCommander => origin == Origin::SupremeCommander,
            Origins::Mods => origin == Origin::Mod,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Origins::All => n("All maps"),
            Origins::Ta => n("Total Annihilation"),
            Origins::Expansions => n("Expansions"),
            Origins::Reborn => n("TA Reborn"),
            Origins::SupremeCommander => n("Supreme Commander"),
            Origins::Mods => n("Mods and map packs"),
        }
    }
}

/// The order a list puts its maps in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    /// By name (the catalogue's own order).
    #[default]
    Name,
    /// By origin, TA's own first, each under its name; by name within it.
    Origin,
    /// The most players first; by name among those of as many.
    Players,
}

impl Order {
    const ALL: [Order; 3] = [Order::Name, Order::Origin, Order::Players];

    fn label(self) -> &'static str {
        match self {
            Order::Name => n("By name"),
            Order::Origin => n("By origin"),
            Order::Players => n("By players"),
        }
    }
}

/// A list's choice of maps: the words searched for, the origins shown and the order.
#[derive(Clone, Debug, Default)]
pub struct Choice {
    pub words: String,
    pub origins: Origins,
    pub order: Order,
}

impl Choice {
    /// Whether map `m` is among those shown: of the origins chosen, and the words in its name,
    /// its world or its origin's name or tag (as shown, or in English).
    pub fn accepts(&self, m: &MapEntry) -> bool {
        if !self.origins.accepts(m.origin) {
            return false;
        }
        let words = self.words.trim().to_lowercase();
        words.is_empty() || [m.title.as_str(), m.planet.as_str(), t(m.origin.name()), m.origin.name(), t(m.origin.tag())].iter().any(|text| text.to_lowercase().contains(&words))
    }

    /// The maps of `maps` shown, in the order chosen (their places in `maps`, by name there).
    pub fn shown(&self, maps: &[MapEntry]) -> Vec<usize> {
        let mut shown: Vec<usize> = (0..maps.len()).filter(|&i| self.accepts(&maps[i])).collect();
        // (A stable sort: by name within the same origin, or the same count of players.)
        match self.order {
            Order::Name => {}
            Order::Origin => shown.sort_by_key(|&i| maps[i].origin),
            Order::Players => shown.sort_by_key(|&i| std::cmp::Reverse(maps[i].start_positions)),
        }
        shown
    }

    /// Whether fewer than all of them are asked for.
    pub fn narrowed(&self) -> bool {
        !self.words.trim().is_empty() || self.origins != Origins::All
    }

    /// All of them again (the order kept).
    pub fn clear(&mut self) {
        self.words.clear();
        self.origins = Origins::All;
    }

    /// The origins to show and the order, side by side, `width` wide: of the origins, those the
    /// maps of `maps` come from. True when either was changed (the list to show the map chosen).
    pub fn controls(&mut self, ui: &mut Ui, id: &str, maps: &[MapEntry], width: f32) -> bool {
        let before = (self.origins, self.order);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let offered: Vec<Origins> = Origins::ALL.into_iter().filter(|&o| o == Origins::All || o == self.origins || maps.iter().any(|m| o.accepts(m.origin))).collect();
            let order_width = w::combo_width(ui, Order::ALL.iter().map(|o| t(o.label())), 96.0);
            let origins_width = (width - 6.0 - order_width).max(110.0);
            egui::ComboBox::from_id_salt((id, "origins")).selected_text(t(self.origins.label())).width(origins_width).truncate().show_ui(ui, |ui| {
                for o in offered {
                    ui.selectable_value(&mut self.origins, o, t(o.label()));
                }
            });
            egui::ComboBox::from_id_salt((id, "order")).selected_text(t(self.order.label())).width(order_width).show_ui(ui, |ui| {
                for o in Order::ALL {
                    ui.selectable_value(&mut self.order, o, t(o.label()));
                }
            });
        });
        (self.origins, self.order) != before
    }
}

/// A map's row in a list: its name, its origin's tag and `players` (its count of them, as the
/// list shows it); its origin said in full under the pointer.
pub fn row(ui: &mut Ui, m: &MapEntry, selected: bool, players: RichText) -> Response {
    panel::tagged_row(ui, selected, true, RichText::new(&m.title), Some((t(m.origin.tag()), color(m.origin))), Some(players))
        .on_hover_text(format!("{}\n{}", t(m.origin.name()), t(m.origin.about())))
}

/// Over the maps of `origin` in a list put in order by origin: its name and how many there are,
/// a row's height.
pub fn heading(ui: &mut Ui, origin: Origin, count: usize) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), panel::LIST_ROW), egui::Sense::hover());
    let text = tf("{origin} · {n}", &[("origin", &t(origin.name()).to_uppercase()), ("n", &count)]);
    let words = w::galley(ui, RichText::new(text).font(strong(11.0)).color(color(origin)).extra_letter_spacing(ta_ui::i18n::tracking(1.2)));
    ui.painter().galley(egui::pos2(rect.min.x + 8.0, rect.max.y - words.size().y - 4.0), words, color(origin));
}

/// The colour of an origin's tag.
pub fn color(origin: Origin) -> Color32 {
    match origin {
        Origin::TotalAnnihilation => STEEL,
        Origin::CoreContingency | Origin::BattleTactics => CREAM,
        Origin::Reborn => AMBER_BRIGHT,
        Origin::SupremeCommander => SKY,
        Origin::Mod => GREEN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(title: &str, origin: Origin, players: usize) -> MapEntry {
        MapEntry { name: title.to_lowercase(), title: title.into(), description: String::new(), planet: "Green Planet".into(), size: String::new(), start_positions: players, origin, info: Default::default() }
    }

    /// The maps shown: by origin (the expansions together), by words in their names or their
    /// origin's, and in the order chosen, by name among equals.
    #[test]
    fn a_list_finds_maps_by_origin_and_puts_them_in_order() {
        let maps = [
            map("Coast to Coast", Origin::TotalAnnihilation, 2),
            map("Crystal Isle", Origin::Reborn, 8),
            map("Frostmill Ruins", Origin::SupremeCommander, 8),
            map("Hula Hoop", Origin::CoreContingency, 4),
            map("Seton's Clutch", Origin::SupremeCommander, 8),
            map("Wretched Ridges", Origin::BattleTactics, 2),
        ];
        let mut choice = Choice::default();
        assert_eq!(choice.shown(&maps), [0, 1, 2, 3, 4, 5]);
        choice.origins = Origins::Expansions;
        assert_eq!(choice.shown(&maps), [3, 5]);
        choice.origins = Origins::SupremeCommander;
        assert_eq!(choice.shown(&maps), [2, 4]);
        assert!(choice.narrowed());
        choice.clear();
        choice.words = "supreme".into();
        assert_eq!(choice.shown(&maps), [2, 4], "found by their origin's name");
        choice.words = "cc".into();
        assert_eq!(choice.shown(&maps), [3], "and by its tag");
        choice.words.clear();
        choice.order = Order::Origin;
        assert_eq!(choice.shown(&maps), [0, 3, 5, 1, 2, 4]);
        choice.order = Order::Players;
        assert_eq!(choice.shown(&maps), [1, 2, 4, 3, 0, 5]);
    }
}
