//! Native accessibility routing and battle summaries for assistive technology.

pub fn native_event(state: &mut egui_winit::State, event: egui_winit::accesskit_winit::WindowEvent) {
    use egui_winit::accesskit_winit::WindowEvent;
    match event {
        WindowEvent::InitialTreeRequested => state.egui_ctx().enable_accesskit(),
        WindowEvent::ActionRequested(request) => state.on_accesskit_action_request(request),
        WindowEvent::AccessibilityDeactivated => state.egui_ctx().disable_accesskit(),
    }
}


/// A readable snapshot of custom-painted information. It follows the HUD's viewpoint and
/// never enumerates enemies, radar contacts or hidden units.
pub fn battle_summary(game: &crate::game::Game) -> String {
    use ta_ui::i18n::tf;
    let economy = &game.sim.players[game.hud_player() as usize].economy;
    let resources = tf("Metal {metal} of {metal_capacity}. Energy {energy} of {energy_capacity}.", &[
        ("metal", &economy.metal.to_int()), ("metal_capacity", &economy.metal_capacity.to_int()),
        ("energy", &economy.energy.to_int()), ("energy_capacity", &economy.energy_capacity.to_int()),
    ]);
    format!("{resources} {}", selection_summary(game))
}

fn selection_summary(game: &crate::game::Game) -> String {
    use ta_ui::i18n::{t, tf};
    let selected: Vec<_> = game.selection.iter().filter_map(|&id| game.sim.unit(id))
        .filter(|u| !u.dead && u.carried_by.is_none() && game.seen(u) == ta_sim::Seen::Visible).collect();
    match selected.as_slice() {
        [] => t("No units selected").to_string(),
        [unit] => tf("Selected {unit}. Health {health} of {maximum}.", &[
            ("unit", &crate::chat::unit_label(&game.sim, unit.type_id)),
            ("health", &unit.health.to_int()), ("maximum", &game.sim.unit_type(unit).max_health.to_int()),
        ]),
        units => tf("{count} units selected", &[("count", &units.len())]),
    }
}

/// Coalesce rapidly changing feedback and limit live-region updates to one per 1.5 seconds.
/// Resource values stay available on demand, without continuously interrupting speech.
#[derive(Default)]
pub struct BattleAccess {
    revision: Option<u64>,
    selection: Vec<ta_sim::UnitId>,
    notice_serial: u64,
    pending: Option<String>,
    spoken: String,
    last_announcement: f64,
}

impl BattleAccess {
    pub fn show(&mut self, ctx: &egui::Context, game: &crate::game::Game, announcements: bool) {
        let active = ctx.accesskit_node_builder(egui::Id::new("battle_snapshot"), |node| {
            node.set_role(egui::accesskit::Role::Label);
            node.set_value(battle_summary(game));
        }).is_some();
        if !active { return; }
        self.follow_timeline(game.event_history.revision());
        let selection_changed = self.selection != game.selection;
        if selection_changed {
            self.selection.clone_from(&game.selection);
            self.pending = Some(selection_summary(game));
        }
        if let Some(notice) = game.event_history.latest() && notice.serial != self.notice_serial {
            self.notice_serial = notice.serial;
            self.pending = Some(notice.text.clone());
        }
        let time = ctx.input(|i| i.time);
        self.advance(time, announcements);
        ctx.accesskit_node_builder(egui::Id::new("battle_announcement"), |node| {
            node.set_role(egui::accesskit::Role::Status);
            node.set_live(if announcements { egui::accesskit::Live::Polite } else { egui::accesskit::Live::Off });
            node.set_live_atomic();
            node.set_label(self.spoken.as_str());
        });
    }

    fn follow_timeline(&mut self, revision: u64) {
        if self.revision != Some(revision) {
            *self = Self { revision: Some(revision), ..Default::default() };
        }
    }

    fn advance(&mut self, time: f64, enabled: bool) {
        if !enabled {
            self.pending = None;
            self.spoken.clear();
        } else if time - self.last_announcement >= 1.5 || time < self.last_announcement {
            if let Some(text) = self.pending.take() {
                self.spoken = text;
                self.last_announcement = time;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_battle_or_rewind_discards_old_speech_and_selection() {
        let mut state = BattleAccess::default();
        state.follow_timeline(1);
        state.spoken = "Old battle".into();
        state.pending = Some("Do not carry over".into());
        state.notice_serial = 40;
        state.follow_timeline(2);
        assert!(state.spoken.is_empty() && state.pending.is_none());
        assert_eq!(state.notice_serial, 0);
        assert_eq!(state.revision, Some(2));
    }

    #[test]
    fn live_announcements_coalesce_and_respect_the_preference() {
        let mut state = BattleAccess::default();
        state.pending = Some("First".into());
        state.advance(2.0, true);
        assert_eq!(state.spoken, "First");
        state.pending = Some("Second".into());
        state.advance(2.2, true);
        assert_eq!(state.spoken, "First");
        state.pending = Some("Latest".into());
        state.advance(3.5, true);
        assert_eq!(state.spoken, "Latest");
        state.pending = Some("Do not read".into());
        state.advance(5.0, false);
        assert!(state.spoken.is_empty() && state.pending.is_none());
        state.advance(7.0, true);
        assert!(state.spoken.is_empty(), "enabling does not replay old notices");
    }
}
