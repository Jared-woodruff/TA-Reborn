//! The orders the selection can give, as the command plate lists them: each with its glyph, the
//! action its key is bound to and the group it belongs to. One list for every place that offers
//! orders (the plate, the radial menu), so that they never differ.

use ta_sim::UnitId;
use ta_ui::i18n::t;

use crate::game::{CommandMode, Game};
use crate::hud_icons::Glyph;

/// What an order does when chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Stop what the selection is doing.
    Stop,
    /// A targeted order: the next click on the battlefield gives it.
    Mode(CommandMode),
    /// Perspective: into this unit, to ride along.
    Perspective(UnitId),
}

/// The rows the plate sets the orders in: moving and fighting; work (mending, reclaiming,
/// taking over, raising wrecks, extractors); and the special ones (cargo, the D-gun, Perspective).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Command,
    Work,
    Special,
}

/// One order the selection can give.
#[derive(Clone, Copy, Debug)]
pub struct Def {
    pub kind: Kind,
    pub glyph: Glyph,
    /// The key binding that gives it (`Keys::text` names the key).
    pub action: &'static str,
    pub group: Group,
}

impl Def {
    /// Its key as the player has it bound.
    pub fn key(&self, game: &Game) -> String {
        game.keys.text(self.action).to_string()
    }

    /// Its name, key and what it does, for the pointer.
    pub fn tip(&self, game: &Game) -> String {
        let key = self.key(game);
        match self.kind {
            Kind::Stop => format!("{} ({key})", t("Stop")),
            Kind::Mode(mode) => format!("{} ({key}): {}", t(mode.label_for(game)), t(mode.hint_for(game))),
            Kind::Perspective(_) => format!("{} ({key}): {}", t("Perspective"), t("See through its eyes as it goes on with its orders. Escape leaves.")),
        }
    }
}

/// The orders the selection can give, in the order the plate sets them: every group's in turn
/// (Stop always; Perspective for the first selected unit when it can be entered).
pub fn available(game: &Game) -> Vec<Def> {
    const MODES: [(CommandMode, Glyph, &str, Group); 13] = [
        (CommandMode::Move, Glyph::Move, "move", Group::Command),
        (CommandMode::Attack, Glyph::Attack, "attack", Group::Command),
        (CommandMode::Fight, Glyph::Fight, "fight", Group::Command),
        (CommandMode::Patrol, Glyph::Patrol, "patrol", Group::Command),
        (CommandMode::Guard, Glyph::Guard, "guard", Group::Command),
        (CommandMode::Repair, Glyph::Repair, "repair", Group::Work),
        (CommandMode::Reclaim, Glyph::Reclaim, "reclaim", Group::Work),
        (CommandMode::Capture, Glyph::Capture, "capture", Group::Work),
        (CommandMode::Resurrect, Glyph::Resurrect, "resurrect", Group::Work),
        (CommandMode::AreaMex, Glyph::AreaMex, "area_mex", Group::Work),
        (CommandMode::Load, Glyph::Load, "load", Group::Special),
        (CommandMode::Unload, Glyph::Unload, "unload", Group::Special),
        (CommandMode::DGun, Glyph::DGun, "dgun", Group::Special),
    ];
    let mut out = vec![Def { kind: Kind::Stop, glyph: Glyph::Stop, action: "stop", group: Group::Command }];
    // (Stop first, then the movers' orders; the work and special rows after.)
    for (mode, glyph, action, group) in MODES {
        if game.mode_available(mode) {
            out.push(Def { kind: Kind::Mode(mode), glyph, action, group });
        }
    }
    if let Some(&id) = game.selection.first()
        && crate::perspective::Perspective::can_enter(game, id)
    {
        out.push(Def { kind: Kind::Perspective(id), glyph: Glyph::Perspective, action: "perspective", group: Group::Special });
    }
    out
}

/// The orders split into the rows they stand in: each group's, in `columns` to a row (a group
/// with more than that goes on a second row), the groups apart.
pub fn rows(defs: &[Def], columns: usize) -> Vec<Vec<Def>> {
    let columns = columns.max(1);
    let mut rows = Vec::new();
    for group in [Group::Command, Group::Work, Group::Special] {
        let of: Vec<Def> = defs.iter().copied().filter(|d| d.group == group).collect();
        rows.extend(of.chunks(columns).map(<[Def]>::to_vec));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(group: Group, glyph: Glyph) -> Def {
        Def { kind: Kind::Stop, glyph, action: "stop", group }
    }

    /// The groups stand in rows of their own, in order, and one wider than a row goes on.
    #[test]
    fn the_orders_stand_in_rows_by_group() {
        let defs = [
            def(Group::Command, Glyph::Stop),
            def(Group::Special, Glyph::Load),
            def(Group::Command, Glyph::Move),
            def(Group::Work, Glyph::Repair),
            def(Group::Command, Glyph::Attack),
        ];
        let rows = rows(&defs, 6);
        let glyphs: Vec<Vec<Glyph>> = rows.iter().map(|r| r.iter().map(|d| d.glyph).collect()).collect();
        assert_eq!(glyphs, [vec![Glyph::Stop, Glyph::Move, Glyph::Attack], vec![Glyph::Repair], vec![Glyph::Load]]);
        // Six to a row at most: a seventh goes on a row of its own.
        let seven: Vec<Def> = (0..7).map(|_| def(Group::Command, Glyph::Move)).collect();
        assert_eq!(super::rows(&seven, 6).iter().map(Vec::len).collect::<Vec<_>>(), [6, 1]);
        assert!(super::rows(&[], 6).is_empty());
    }
}
