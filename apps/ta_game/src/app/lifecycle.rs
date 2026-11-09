//! Lifecycle decisions kept independent of the window and renderer.
use std::time::{Duration, Instant};

use super::Screen;
use crate::game::Leave;

/// Direct battle starts obey the same speed preference as games started from the
/// menu. Spectator backdrops and measured/staged scenes retain their fixed pace.
pub(super) fn starting_speed(configured: f32, local_player: Option<u8>, technical: bool) -> f32 {
    if !technical && local_player.is_some() {
        configured
    } else {
        1.0
    }
}

/// Command-line dimensions bypass the settings file, but obey its allocation bounds.
pub(super) fn window_size(
    width: Option<u32>,
    height: Option<u32>,
    graphics: &ta_config::Graphics,
) -> (u32, u32) {
    use ta_config::limits;
    (
        width
            .unwrap_or(graphics.width)
            .clamp(*limits::WIDTH.start(), *limits::WIDTH.end()),
        height
            .unwrap_or(graphics.height)
            .clamp(*limits::HEIGHT.start(), *limits::HEIGHT.end()),
    )
}

/// OS close requests must keep asking until the player explicitly leaves. A second
/// click on the window's close button is not an answer to the save question.
pub(super) fn confirm_window_close(
    screen: Screen,
    networked: bool,
    leaving_asks: bool,
    asked: &mut Option<Leave>,
) -> bool {
    if screen == Screen::Playing && !networked && leaving_asks {
        *asked = Some(Leave::Quit);
        true
    } else {
        false
    }
}

const AUTOSAVE_TICKS: u64 = 30 * 60 * 5;
const RETRY_DELAY: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(super) struct Autosave {
    epoch: u64,
    retry_at: Option<Instant>,
}

impl Autosave {
    pub(super) fn due(&self, tick: u64, now: Instant) -> bool {
        tick / AUTOSAVE_TICKS > self.epoch && self.retry_at.is_none_or(|at| now >= at)
    }

    pub(super) fn saved(&mut self, tick: u64) {
        self.epoch = tick / AUTOSAVE_TICKS;
        self.retry_at = None;
    }

    pub(super) fn failed(&mut self, now: Instant) {
        self.retry_at = Some(now + RETRY_DELAY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_battles_use_the_speed_preference_without_changing_technical_runs() {
        assert_eq!(starting_speed(2.5, Some(0), false), 2.5);
        assert_eq!(starting_speed(0.5, Some(1), false), 0.5);
        assert_eq!(starting_speed(2.5, None, false), 1.0);
        assert_eq!(starting_speed(2.5, Some(0), true), 1.0);
        assert_eq!(starting_speed(2.5, None, true), 1.0);
    }

    #[test]
    fn startup_dimensions_bound_cli_overrides_and_keep_valid_capture_sizes() {
        let graphics = ta_config::Graphics {
            width: 1920,
            height: 1080,
            ..Default::default()
        };
        assert_eq!(window_size(None, None, &graphics), (1920, 1080));
        assert_eq!(window_size(Some(1280), None, &graphics), (1280, 1080));
        assert_eq!(window_size(Some(3840), Some(2160), &graphics), (3840, 2160));
        assert_eq!(window_size(Some(0), Some(0), &graphics), (640, 480));
        assert_eq!(
            window_size(Some(u32::MAX), Some(u32::MAX), &graphics),
            (16384, 16384)
        );
        let invalid = ta_config::Graphics {
            width: u32::MAX,
            height: 0,
            ..graphics
        };
        assert_eq!(window_size(None, None, &invalid), (16384, 480));
    }

    #[test]
    fn repeated_window_closes_never_answer_the_unsaved_battle_question() {
        let mut asked = None;
        for _ in 0..3 {
            assert!(confirm_window_close(
                Screen::Playing,
                false,
                true,
                &mut asked
            ));
            assert!(matches!(asked, Some(Leave::Quit)));
        }
        // A close request while another leave question is open still asks about quitting.
        for leave in [Leave::Menu, Leave::Restart] {
            let mut asked = Some(leave);
            assert!(confirm_window_close(
                Screen::Playing,
                false,
                true,
                &mut asked
            ));
            assert!(matches!(asked, Some(Leave::Quit)));
        }
    }

    #[test]
    fn only_live_single_player_battles_need_a_window_close_question() {
        for screen in [Screen::Intro, Screen::Menu, Screen::Editor, Screen::Space] {
            assert!(!confirm_window_close(screen, false, true, &mut None));
        }
        assert!(!confirm_window_close(
            Screen::Playing,
            true,
            true,
            &mut None
        ));
        // Replays, spectators, tutorials and finished battles all have leaving_asks=false.
        assert!(!confirm_window_close(
            Screen::Playing,
            false,
            false,
            &mut None
        ));
    }

    #[test]
    fn autosave_follows_game_time_and_success_is_not_repeated() {
        let now = Instant::now();
        let mut save = Autosave::default();
        assert!(!save.due(AUTOSAVE_TICKS - 1, now));
        assert!(save.due(AUTOSAVE_TICKS, now));
        save.saved(AUTOSAVE_TICKS + 12);
        assert!(!save.due(AUTOSAVE_TICKS + 13, now + Duration::from_secs(600)));
        assert!(save.due(AUTOSAVE_TICKS * 2, now));
        save.saved(AUTOSAVE_TICKS * 4);
        assert!(!save.due(AUTOSAVE_TICKS * 4 + 1, now));
    }

    #[test]
    fn failed_autosaves_retry_while_paused_without_hammering_storage() {
        let now = Instant::now();
        let mut save = Autosave::default();
        let tick = AUTOSAVE_TICKS + 1;
        save.failed(now);
        assert!(!save.due(tick, now));
        assert!(!save.due(tick, now + RETRY_DELAY - Duration::from_millis(1)));
        assert!(save.due(tick, now + RETRY_DELAY));
        save.failed(now + RETRY_DELAY);
        assert!(!save.due(tick, now + RETRY_DELAY));
        assert!(save.due(tick, now + RETRY_DELAY * 2));
        save.saved(tick);
        assert!(!save.due(tick, now + RETRY_DELAY * 3));
        assert!(save.due(AUTOSAVE_TICKS * 2, now + RETRY_DELAY * 3));
        // Starting another battle clears both the previous epoch and its retry delay.
        save = Autosave::default();
        assert!(!save.due(0, now));
        assert!(save.due(AUTOSAVE_TICKS, now));
    }
}
