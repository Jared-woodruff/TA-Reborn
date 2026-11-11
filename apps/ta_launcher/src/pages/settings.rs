//! Settings page: the settings screen the game's options share (`ta_ui::settings`), editing
//! `config.toml`. Changes are saved automatically (see [`crate::persist`]).

use ta_ui::tooltip::TooltipExt as _;
use egui::{Align, Layout, RichText, Ui};
use crate::app::LauncherApp;
use crate::theme::*;
use crate::widgets::{self, ActionButton, caption};
use ta_ui::metal;

impl LauncherApp {
    pub(crate) fn settings_page(&mut self, ui: &mut Ui) {
        let top = ui.cursor().min.y;
        let compact = ta_ui::settings::compact(ui.available_width() - 32.0);
        let heading = |ui: &mut Ui| {
            metal::embossed_label(ui, RichText::new(ta_ui::i18n::t("SETTINGS")).font(display(32.0)).extra_letter_spacing(ta_ui::i18n::tracking(1.8)), TEXT_BRIGHT);
        };
        // The compact header already names this page. Keep the viewport for its settings,
        // with destructive reset actions after them instead of above the first control.
        if !compact {
            ui.horizontal(|ui| {
                heading(ui);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    self.reset_settings_controls(ui);
                });
            });
        }
        // (A short window keeps its room for the settings.)
        if ui.clip_rect().height() > 560.0 {
            ui.label(RichText::new(ta_ui::i18n::t("Changes save by themselves and apply the next time the game starts. The game's Options show the same settings.")).color(STEEL));
        }
        if let Some(error) = self.store.load_error.clone() {
            ui.add_space(space::M);
            self.config_error_card(ui, &error);
        }
        if !compact {
            ui.add_space(space::L);
        }
        // The list scrolls beside the categories, in what the window leaves: the page ends
        // above the window's foot by the column's margin there (30, after a gap), by how tall
        // the page is besides the list, as laid out last frame (no scroll bar of the window's
        // for a few points more).
        let chrome_id = egui::Id::new("launcher_settings_chrome");
        let chrome = ui.ctx().data(|d| d.get_temp::<f32>(chrome_id)).unwrap_or(330.0);
        let height = (ui.clip_rect().bottom() - 30.0 - ui.spacing().item_spacing.y - top - chrome).max(280.0);
        let host = ta_ui::settings::Host {
            in_game: false,
            msaa_running: 0,
            ray_tracing_running: false,
            native: native_resolution(ui),
            gpu_kind: super::about::computer(ui.ctx()).and_then(|hw| hw.gpu_kind),
            languages: &self.languages,
            language_dirs: &self.language_dirs,
            height: (!compact).then_some(height),
        };
        let enabled = !self.dialog_open();
        widgets::card(ui, |ui| {
            ui.add_enabled_ui(enabled, |ui| self.settings_view.show(ui, &mut self.config, &host));
        });
        if compact {
            ui.add_space(space::L);
            ui.horizontal_wrapped(|ui| self.reset_settings_controls(ui));
        }
        let measured = ui.min_rect().bottom() - top - height;
        if !compact && (measured - chrome).abs() > 0.5 {
            ui.ctx().data_mut(|d| d.insert_temp(chrome_id, measured));
            ui.ctx().request_repaint();
        }
    }

    fn reset_settings_controls(&mut self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing.x = 10.0;
        if self.confirm_reset {
            if ui.add(ActionButton::secondary("CANCEL")).clicked() {
                self.confirm_reset = false;
            }
            if ui.add(ActionButton::danger("RESET ALL SETTINGS")).clicked() {
                ta_ui::settings::restore_all(&mut self.config);
                self.confirm_reset = false;
                self.notify("Display, audio and gameplay settings restored to defaults", AMBER);
            }
            ui.label(RichText::new(ta_ui::i18n::t("Game folder and mods are kept.")).color(STEEL));
        } else if ui.add(ActionButton::secondary("RESET ALL SETTINGS")).hover_tip(ta_ui::i18n::t("Every category back to how it came")).clicked() {
            self.confirm_reset = true;
        }
    }

    fn config_error_card(&mut self, ui: &mut Ui, error: &str) {
        let backup = self.store.backup.clone();
        widgets::card_accent(ui, Some(AMBER), |ui| {
            widgets::card_title(ui, "SETTINGS FILE COULD NOT BE READ");
            ui.label(
                RichText::new(match backup {
                    Some(backup) => ta_ui::i18n::tf("Defaults are in use. The unreadable file was backed up to {path}.", &[("path", &backup.display())]),
                    None => ta_ui::i18n::t(
                        "Defaults are in use. The file will be backed up to config.toml.bak before your next change \
                             is saved.",
                    )
                    .to_owned(),
                })
                .color(TEXT),
            );
            egui::CollapsingHeader::new(caption("DETAILS")).id_salt("config_error").show(ui, |ui| {
                ui.label(RichText::new(error).font(mono(11.5)).color(STEEL));
            });
        });
    }
}

/// The monitor's size in physical pixels, if known.
fn native_resolution(ui: &Ui) -> Option<(u32, u32)> {
    ui.input(|i| {
        let size = i.viewport().monitor_size?;
        // winit reports monitor_size in zoomed egui points. Both display DPI and
        // the player's interface zoom must be undone to recover physical pixels.
        let scale = ui.pixels_per_point();
        Some(((size.x * scale).round() as u32, (size.y * scale).round() as u32))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_config::{Audio, Graphics};

    #[test]
    fn native_display_resolution_is_independent_of_interface_zoom() {
        for zoom in [0.75, 1.0, 1.5, 2.0] {
            for dpi in [1.0, 1.5, 2.0] {
                let ctx = egui::Context::default();
                ctx.set_zoom_factor(zoom);
                let mut native = None;
                for _ in 0..3 {
                    let mut input = egui::RawInput::default();
                    let viewport = input.viewports.entry(egui::ViewportId::ROOT).or_default();
                    viewport.native_pixels_per_point = Some(dpi);
                    viewport.monitor_size = Some(egui::vec2(3840.0, 2160.0) / (zoom * dpi));
                    ctx.run_ui(input, |ui| native = native_resolution(ui)).drop_without_applying_deltas();
                }
                assert_eq!(native, Some((3840, 2160)), "zoom {zoom}, display scale {dpi}");
            }
        }
    }

    #[test]
    fn resetting_preferences_preserves_profile_identity_and_installed_content() {
        let mut config = ta_config::Config::default();
        config.gameplay.player_name = "ARM Commander".into();
        config.gameplay.language = "de".into();
        config.gameplay.reduced_motion = true;
        config.audio.music = 0.2;
        config.graphics.bloom = false;
        config.launcher.themed_cursor = false;
        config.game_dir = Some("original-game".into());
        config.mods.enabled = vec!["my-mod".into()];
        // Every category's own (the launcher's included), and the keys.
        config.launcher.on_play = ta_config::OnPlay::Close;
        config.updates.check = false;
        config.gameplay.radial_menu = false;
        config.keys.set("attack", "Z");
        // Not settings: the graphics card found, the games joined lately, the launcher's window.
        config.graphics.gpu = "Some Card".into();
        config.gameplay.recent_games = vec![ta_config::RecentGame { address: "10.0.0.2:7777".into(), title: "Friday".into() }];
        config.launcher.window = Some([1200.0, 800.0]);
        let folder = config.game_dir.clone();
        let mods = config.mods.clone();
        ta_ui::settings::restore_all(&mut config);
        assert_eq!(config.gameplay.player_name, "ARM Commander");
        assert_eq!(config.gameplay.language, "de", "reset keeps the language already displayed, as category resets do");
        assert!(!config.gameplay.reduced_motion && config.gameplay.radial_menu);
        assert_eq!(config.graphics, Graphics { gpu: "Some Card".into(), ..Graphics::default() });
        assert_eq!(config.audio, Audio::default());
        assert_eq!(config.keys, ta_config::Keys::default());
        assert_eq!(config.game_dir, folder);
        assert_eq!(config.mods, mods);
        assert_eq!(config.launcher.themed_cursor, ta_config::Launcher::default().themed_cursor);
        assert_eq!(config.launcher.on_play, ta_config::OnPlay::default(), "the launcher's own category comes back too");
        assert!(config.updates.check);
        assert_eq!(config.gameplay.recent_games.len(), 1, "the games joined lately are not a setting");
        assert_eq!(config.launcher.window, Some([1200.0, 800.0]));
    }
}
