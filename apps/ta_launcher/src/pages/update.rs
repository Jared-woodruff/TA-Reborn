//! A new version: the Play page's card that says so, and the controls it shares with the About
//! page's updates card (UPDATE NOW, the download's progress, WHAT'S NEW).

use ta_ui::tooltip::TooltipExt as _;
use egui::{Align, Layout, RichText, Sense, Ui, vec2};
use ta_ui::i18n::{t, tf};

use super::getmods::{megabytes, progress};
use super::{paragraph, wrapped};
use crate::app::{Action, LauncherApp, UpdateWait};
use crate::theme::*;
use crate::update::{Release, Step, UpdateState};
use crate::widgets::{self, ActionButton, Glyph};

/// Room kept at the right of a flagged card's top row for the lamp in its corner.
const CORNER_LAMP: f32 = 18.0;

impl LauncherApp {
    /// A new version is out: what it is, and the way to it (put away until the next start with
    /// its cross, unless it is on its way).
    pub(super) fn update_card(&mut self, ui: &mut Ui) {
        let UpdateState::Available(release) = &self.update else { return };
        let coming = self.download.as_ref().is_some_and(|d| !matches!(d.step, Step::Failed(_)));
        if self.update_hidden && !coming {
            return;
        }
        let release = release.clone();
        let mut hide = false;
        let card = widgets::card_accent(ui, Some(SKY), |ui| {
            ui.horizontal(|ui| {
                let (lamp, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                ui.painter().add(ta_ui::metal::lamp(lamp.center(), 4.5, SKY, 1.0));
                ui.label(RichText::new(tf("VERSION {version} IS OUT", &[("version", &release.number())])).font(strong(13.0)).color(SKY).extra_letter_spacing(ta_ui::i18n::tracking(1.4)));
                if !coming {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(CORNER_LAMP);
                        hide = widgets::icon_button(ui, Glyph::Close, true).hover_tip(t("Later")).clicked();
                    });
                }
            });
            // (A release's notes are in English: their first lines shown in English, a word of
            // the new version in other languages.)
            let summary = release.summary();
            if ta_ui::i18n::language().is_empty() && !summary.is_empty() {
                wrapped(ui, &crate::news::brief(&summary, 2), TEXT);
            } else {
                paragraph(ui, "A new version of TA Reborn is out. WHAT'S NEW tells what it brings.");
            }
            ui.add_space(4.0);
            self.update_controls(ui, &release);
        });
        if std::mem::take(&mut self.focus_update) {
            ui.scroll_to_rect(card.response.rect, Some(Align::Min));
        }
        ui.add_space(14.0);
        if hide {
            self.update_hidden = true;
        }
    }

    /// UPDATE NOW (the release's page instead, for a copy Setup did not install), the update's
    /// progress once it is on its way, and WHAT'S NEW.
    pub(super) fn update_controls(&mut self, ui: &mut Ui, release: &Release) {
        let step = self.download.as_ref().map(|d| d.step.clone());
        let size = release.setup.as_ref().map_or(0, |a| a.size);
        // (What holds the update back: `update_blocked`.)
        let wait = self.update_blocked().map(UpdateWait::why);
        let width = (ui.available_width() - 44.0).clamp(1.0, 560.0);
        ui.horizontal_wrapped(|ui| match &step {
            None | Some(Step::Failed(_)) => {
                if self.can_install(release) {
                    let label = if size > 0 { tf("UPDATE NOW · {size}", &[("size", &megabytes(size))]) } else { t("UPDATE NOW").to_string() };
                    let button = ui.add_enabled(wait.is_none(), ActionButton::primary(&label).min_width(170.0).height(34.0));
                    let button = match wait {
                        Some(why) => button.disabled_tip(why),
                        None => button.hover_tip(t("Downloads it, checks it and installs it: the launcher closes and comes back on the new version")),
                    };
                    if button.clicked() {
                        self.push(Action::Update);
                    }
                } else if !release.url.is_empty()
                    && ui.add(ActionButton::primary("GET THE UPDATE").height(34.0)).hover_tip(t("Opens its page: this copy was not installed with Setup, so it cannot update itself")).clicked()
                {
                    self.push(Action::OpenUrl(release.url.clone()));
                }
                if ui.add(ActionButton::secondary("WHAT'S NEW").height(34.0)).hover_tip(t("What the new version brings")).clicked() {
                    self.push(Action::ReleaseNotes);
                }
            }
            Some(Step::Downloading { got, total, since }) => {
                let secs = since.elapsed().as_secs_f32();
                let mut label = if *total > 0 { format!("{} / {}", megabytes(*got), megabytes(*total)) } else { megabytes(*got) };
                if *total > 0 && *got > 0 && secs > 1.0 {
                    let left = ((*total - (*got).min(*total)) as f32 * secs / *got as f32).round();
                    label.push_str(&format!("  ·  {}", tf("{s} s left", &[("s", &left)])));
                }
                progress(ui, width, (*total > 0).then(|| *got as f32 / *total as f32), &label);
                if widgets::icon_button(ui, Glyph::Close, true).hover_tip(t("Cancel")).clicked() {
                    self.push(Action::CancelUpdate);
                }
            }
            Some(Step::Checking) => progress(ui, width, None, t("CHECKING THE DOWNLOAD…")),
            // (Asked for, it starts by itself once nothing holds it back.)
            Some(Step::Ready(_)) if self.update_go => {
                let label = self.update_waits_for().map_or(t("STARTING THE UPDATE…"), UpdateWait::queued);
                progress(ui, width, Some(1.0), label);
            }
            Some(Step::Ready(_)) => {
                let button = ui.add_enabled(wait.is_none(), ActionButton::primary("INSTALL NOW").min_width(170.0).height(34.0));
                let button = match wait {
                    Some(why) => button.disabled_tip(why),
                    None => button.hover_tip(t("The new version is downloaded and checked: the launcher closes and comes back on it")),
                };
                if button.clicked() {
                    self.push(Action::Update);
                }
            }
        });
        if let Some(Step::Failed(e)) = &step {
            wrapped(ui, &tf("The update failed: {error}", &[("error", e)]), RED);
        }
        if let Some(e) = &self.update_error {
            wrapped(ui, &tf("The update did not start: {error}", &[("error", e)]), RED);
        }
    }

    /// Whether this launcher can install `release` itself: Setup installed it, and the release
    /// has a setup.
    fn can_install(&self, release: &Release) -> bool {
        release.setup.is_some() && release.sums.is_some() && (!self.offline || release.available_offline()) && (self.pretend_installed || crate::update::installed())
    }
}
