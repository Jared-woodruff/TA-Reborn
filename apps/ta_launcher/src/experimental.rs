//! Experimental features: the launcher's features still being tried out. They show only with
//! ENABLE EXPERIMENTAL FEATURES on (Settings, LAUNCHER; `ta_config::Launcher::
//! experimental_features`, off by default), so they can ship in a release before they are ready
//! for everyone, and testers can try them early. Turning the switch off hides a feature's way in;
//! what it made before (an imported mod, a map pack) stays installed as any mod does, and an
//! import already under way is seen through to its end.
//!
//! A feature is tagged by adding it to [`Feature`] (and [`Feature::ALL`]) and asking
//! [`LauncherApp::experimental`] before showing its way in; its own dialog shows [`tag`] so
//! testers know what they are trying. Settings, LAUNCHER shows the switch only while some feature
//! is listed. A feature ready for everyone leaves the list: its gate and tag go, and it is shown
//! as any other. (The Supreme Commander import, the first, has left it.)

use egui::{RichText, Ui};
use ta_ui::i18n::t;
use ta_ui::tooltip::TooltipExt as _;

use crate::app::LauncherApp;
use crate::theme::*;

/// The features still being tried out: none at the moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {}

impl Feature {
    /// Every feature still being tried out (ENABLE EXPERIMENTAL FEATURES is shown while there is
    /// one).
    pub const ALL: &'static [Feature] = &[];
}

impl LauncherApp {
    /// Whether `feature` is shown: the player turned experimental features on, or this run was
    /// started to show one (`--experimental`, or `--show` at an experimental dialog). (One switch
    /// shows them all.)
    #[allow(dead_code)] // (The next feature tried out asks it.)
    pub(crate) fn experimental(&self, _feature: Feature) -> bool {
        self.config.launcher.experimental_features || self.experimental_forced
    }
}

/// The tag an experimental feature's dialog carries: EXPERIMENTAL, in amber, saying what that
/// means on hover.
#[allow(dead_code)] // (The next feature tried out shows it.)
pub fn tag(ui: &mut Ui) {
    ui.label(RichText::new(t("EXPERIMENTAL")).font(strong(11.5)).color(AMBER).extra_letter_spacing(ta_ui::i18n::tracking(1.2)))
        .hover_tip(t("Still being tested: it may change, or not always work as it should. Settings, LAUNCHER: ENABLE EXPERIMENTAL FEATURES hides it again."));
}
