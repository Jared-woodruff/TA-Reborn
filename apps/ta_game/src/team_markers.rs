//! Colour-independent ownership labels. The number always identifies the player slot;
//! the prefix describes its relation to the player being watched. These labels accompany
//! the existing unit-class shapes, rather than repurposing those shapes as allegiances.

use egui::{Color32, Pos2, Rect, Stroke, Vec2};
use ta_sim::Seen;

use crate::game::Game;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Relation {
    Own,
    Ally,
    Enemy,
    /// An unrestricted spectator has no side: every player keeps a neutral label.
    Neutral,
}

impl Relation {
    fn of(viewer: Option<u8>, owner: u8, allied: bool) -> Self {
        match viewer {
            None => Self::Neutral,
            Some(player) if player == owner => Self::Own,
            Some(_) if allied => Self::Ally,
            Some(_) => Self::Enemy,
        }
    }

    fn symbol(self) -> char {
        match self {
            Self::Own => '=',
            Self::Ally => '+',
            Self::Enemy => '×',
            Self::Neutral => '·',
        }
    }

    pub(crate) fn name(self) -> &'static str {
        ta_ui::i18n::t(match self {
            Self::Own => ta_ui::i18n::n("Your units"),
            Self::Ally => ta_ui::i18n::n("Ally"),
            Self::Enemy => ta_ui::i18n::n("Enemy"),
            Self::Neutral => ta_ui::i18n::n("Player"),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Marker {
    pub(crate) owner: u8,
    pub(crate) relation: Relation,
}

impl Marker {
    pub(crate) fn label(self) -> String {
        // Cast before adding: player identifiers are zero based, labels are not.
        format!("{}{}", self.relation.symbol(), usize::from(self.owner) + 1)
    }
}

/// A playing user's side stays theirs even with a reveal cheat. Spectators may explicitly
/// watch a player's side; in the unrestricted view no player is declared an enemy.
fn viewpoint(game: &Game) -> Option<u8> {
    if game.spectator { game.view_as } else { Some(game.local_player) }
}

pub(crate) fn player(game: &Game, owner: u8) -> Marker {
    let viewer = viewpoint(game);
    Marker { owner, relation: Relation::of(viewer, owner, viewer.is_some_and(|p| game.sim.allied(p, owner))) }
}

/// Never turn radar-only contact data into an identified owner or a unit-class marker.
pub(crate) fn identified(game: &Game, owner: u8, seen: Seen) -> Option<Marker> {
    identified_marker(player(game, owner), seen)
}

fn identified_marker(marker: Marker, seen: Seen) -> Option<Marker> {
    (seen == Seen::Visible).then_some(marker)
}

pub(crate) fn legend(game: &Game) -> &'static str {
    if viewpoint(game).is_some() {
        ta_ui::i18n::t("Player numbers: = yours, + ally, × enemy. ? is an unidentified radar contact.")
    } else {
        ta_ui::i18n::t("Numbers identify players. Spectator markers do not label any side as an enemy.")
    }
}

/// Nearby units of the same player share a badge. Different players must never be merged,
/// even if they use the same colour or stand in exactly the same minimap cell.
pub(crate) struct Clusters {
    cell: Vec2,
    occupied: rustc_hash::FxHashSet<(i32, i32, u8)>,
    pub(crate) badges: Vec<(Pos2, Marker)>,
}

impl Clusters {
    pub(crate) fn new(cell: Vec2) -> Self {
        Self { cell, occupied: Default::default(), badges: Vec::new() }
    }

    pub(crate) fn push(&mut self, at: Pos2, marker: Marker) {
        let cell = ((at.x / self.cell.x).floor() as i32, (at.y / self.cell.y).floor() as i32, marker.owner);
        if self.occupied.insert(cell) {
            self.badges.push((at, marker));
        }
    }

    pub(crate) fn paint(&self, painter: &egui::Painter, size: f32, offset: Vec2) {
        // Keep different owners in the same cell legible: when plates collide, put the next
        // beside it and join it back to the unit. Dots and class icons retain their positions.
        let mut placed = BadgeLayout::default();
        for &(anchor, marker) in &self.badges {
            let text = painter.layout_no_wrap(marker.label(), ta_ui::theme::strong(size), Color32::WHITE);
            let extent = text.size() + Vec2::new(6.0, 2.0);
            let rect = placed.place(anchor + offset, extent, painter.clip_rect());
            let join = rect.clamp(anchor);
            painter.line_segment([anchor, join], Stroke::new(3.0, Color32::BLACK));
            painter.line_segment([anchor, join], Stroke::new(1.0, Color32::WHITE));
            painter.rect_filled(rect, 2.0, Color32::BLACK);
            painter.rect_stroke(rect, 2.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Inside);
            painter.galley(rect.min + Vec2::new(3.0, 1.0), text, Color32::WHITE);
        }
    }
}

/// Screen-space bins keep crowd avoidance proportional to the visible badges, rather than
/// comparing every player label with every other label in a large battle.
#[derive(Default)]
struct BadgeLayout {
    occupied: rustc_hash::FxHashMap<(i32, i32), Vec<Rect>>,
}

impl BadgeLayout {
    fn cells(rect: Rect) -> impl Iterator<Item = (i32, i32)> {
        let (x0, x1) = ((rect.left() / 32.0).floor() as i32, (rect.right() / 32.0).floor() as i32);
        let (y0, y1) = ((rect.top() / 20.0).floor() as i32, (rect.bottom() / 20.0).floor() as i32);
        (y0..=y1).flat_map(move |y| (x0..=x1).map(move |x| (x, y)))
    }

    fn overlaps(&self, rect: Rect) -> bool {
        Self::cells(rect).any(|cell| self.occupied.get(&cell).is_some_and(|plates| plates.iter().any(|p| p.intersects(rect))))
    }

    fn place(&mut self, center: Pos2, extent: Vec2, clip: Rect) -> Rect {
        let base = Rect::from_center_size(center, extent);
        let base = base.translate(Vec2::new(
            (clip.left() - base.left()).max(0.0) - (base.right() - clip.right()).max(0.0),
            (clip.top() - base.top()).max(0.0) - (base.bottom() - clip.bottom()).max(0.0),
        ));
        let mut chosen = base;
        // Search only the immediate neighbourhood, preserving the label's link to its unit.
        for (x, y) in [(0.0, 0.0), (1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
            let candidate = base.translate(Vec2::new(x * (extent.x + 2.0), y * (extent.y + 2.0)));
            if clip.contains_rect(candidate) && !self.overlaps(candidate.expand(0.5)) {
                chosen = candidate;
                break;
            }
        }
        for cell in Self::cells(chosen) {
            self.occupied.entry(cell).or_default().push(chosen);
        }
        chosen
    }
}

/// The labels are an interface overlay for both battle HUDs, including strategic zoom.
/// Identification always passes through the same fog-of-war gate as the visible model.
pub(crate) fn world(game: &Game, camera: &ta_render::Camera, ppp: f32, painter: &egui::Painter, project: impl Fn(glam::Vec3) -> Option<Pos2>) {
    if !game.team_markers {
        return;
    }
    let mut clusters = Clusters::new(Vec2::new(42.0, 28.0));
    let strategic = crate::strategic::shown(game, camera, ppp);
    let alpha = game.alpha();
    for unit in game.sim.units() {
        if unit.dead || unit.carried_by.is_some() {
            continue;
        }
        let Some(marker) = identified(game, unit.owner, game.seen(unit)) else { continue };
        let Some(position) = game.scene.unit_position(unit.id, alpha) else { continue };
        let kind = game.sim.unit_type(unit);
        let height = (kind.model.height.to_f32() + 6.0) * (1.0 - strategic) + 8.0 * strategic;
        let Some(point) = project(position + glam::Vec3::Y * height) else { continue };
        if painter.clip_rect().contains(point) {
            clusters.push(point, marker);
        }
    }
    clusters.paint(painter, 12.0, Vec2::new(0.0, -19.0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allegiance_is_relative_to_the_viewed_player_not_their_colour() {
        assert_eq!(Relation::of(Some(7), 7, true), Relation::Own);
        assert_eq!(Relation::of(Some(7), 4, true), Relation::Ally);
        assert_eq!(Relation::of(Some(7), 0, false), Relation::Enemy);
        assert_eq!(Relation::of(None, 0, false), Relation::Neutral);
        assert_eq!(Relation::of(None, 7, true), Relation::Neutral);
        let labels: std::collections::HashSet<_> = (0..ta_sim::unit::MAX_PLAYERS as u8)
            .map(|owner| Marker { owner, relation: Relation::Ally }.label()).collect();
        assert_eq!(labels.len(), ta_sim::unit::MAX_PLAYERS);
        assert!(labels.contains("+64"));
    }

    #[test]
    fn hidden_and_radar_only_units_never_gain_ownership_labels() {
        let marker = Marker { owner: 3, relation: Relation::Enemy };
        assert_eq!(identified_marker(marker, Seen::Hidden), None);
        assert_eq!(identified_marker(marker, Seen::Radar), None);
        assert_eq!(identified_marker(marker, Seen::Visible), Some(marker));
    }

    #[test]
    fn dense_armies_share_labels_without_erasing_another_player() {
        let mut clusters = Clusters::new(Vec2::splat(20.0));
        let own = Marker { owner: 0, relation: Relation::Own };
        let ally = Marker { owner: 1, relation: Relation::Ally };
        let enemy = Marker { owner: 2, relation: Relation::Enemy };
        clusters.push(Pos2::new(5.0, 5.0), own);
        clusters.push(Pos2::new(6.0, 6.0), own);
        clusters.push(Pos2::new(5.0, 5.0), ally);
        clusters.push(Pos2::new(5.0, 5.0), enemy);
        clusters.push(Pos2::new(25.0, 5.0), own);
        assert_eq!(clusters.badges.len(), 4);
        assert_eq!(clusters.badges.iter().map(|(_, marker)| *marker).collect::<Vec<_>>(), [own, ally, enemy, own]);
    }

    #[test]
    fn nearby_players_remain_legible_even_at_the_minimap_edge() {
        for anchor in [Pos2::new(100.0, 100.0), Pos2::new(199.0, 1.0)] {
            let clip = Rect::from_min_size(Pos2::ZERO, Vec2::splat(200.0));
            let mut layout = BadgeLayout::default();
            let plates: Vec<_> = (0..3).map(|_| layout.place(anchor, Vec2::new(24.0, 16.0), clip)).collect();
            for (i, plate) in plates.iter().enumerate() {
                assert!(clip.contains_rect(*plate));
                assert!(plates[..i].iter().all(|other| !plate.intersects(*other)), "different players need separate badges");
            }
        }
    }
}
