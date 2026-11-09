//! Settings validation shared by direct game launches and the launcher.
use crate::{Audio, Config, Graphics, limits};

impl Config {
    /// Clamp settings to supported ranges, replacing NaN and infinities with defaults.
    /// Returns whether anything changed; call before creating graphics or consuming input.
    pub fn sanitize(&mut self) -> bool {
        let config = self;
        let before = config.clone();
        let clamp = |v: f32, range: &std::ops::RangeInclusive<f32>, default: f32| {
            if v.is_finite() {
                v.clamp(*range.start(), *range.end())
            } else {
                default
            }
        };

        let g = &mut config.graphics;
        let gd = Graphics::default();
        g.width = g.width.clamp(*limits::WIDTH.start(), *limits::WIDTH.end());
        g.height = g
            .height
            .clamp(*limits::HEIGHT.start(), *limits::HEIGHT.end());
        g.fps_limit = g.fps_limit.min(*limits::FPS_LIMIT.end());
        if !limits::MSAA.contains(&g.msaa) {
            g.msaa = gd.msaa;
        }
        g.terrain_relight = clamp(
            g.terrain_relight,
            &limits::TERRAIN_RELIGHT,
            gd.terrain_relight,
        );
        g.render_scale = clamp(g.render_scale, &limits::RENDER_SCALE, gd.render_scale);

        let a = &mut config.audio;
        let ad = Audio::default();
        a.master = clamp(a.master, &limits::VOLUME, ad.master);
        a.music = clamp(a.music, &limits::VOLUME, ad.music);
        a.effects = clamp(a.effects, &limits::VOLUME, ad.effects);
        a.voice = clamp(a.voice, &limits::VOLUME, ad.voice);
        a.max_voices = a
            .max_voices
            .clamp(*limits::MAX_VOICES.start(), *limits::MAX_VOICES.end());

        let p = &mut config.gameplay;
        p.unit_limit = p
            .unit_limit
            .clamp(*limits::UNIT_LIMIT.start(), *limits::UNIT_LIMIT.end());
        p.ui_scale = if p.ui_scale.is_finite() {
            p.ui_scale
                .clamp(*limits::UI_SCALE.start(), *limits::UI_SCALE.end())
        } else {
            1.0
        };
        p.cursor_scale = clamp(p.cursor_scale, &limits::CURSOR_SCALE, 1.0);
        p.message_duration = clamp(p.message_duration, &limits::MESSAGE_DURATION, 1.0);
        p.gamepad_deadzone = clamp(p.gamepad_deadzone, &limits::GAMEPAD_DEADZONE, 0.18);
        p.gamepad_sensitivity = clamp(p.gamepad_sensitivity, &limits::GAMEPAD_SENSITIVITY, 1.0);
        p.speed = clamp(p.speed, &limits::SPEED, 1.0);
        p.scroll_speed = clamp(p.scroll_speed, &limits::SCROLL_SPEED, 1.0);
        if p.player_name.chars().count() > limits::PLAYER_NAME_CHARS {
            p.player_name = p
                .player_name
                .chars()
                .take(limits::PLAYER_NAME_CHARS)
                .collect();
        }

        *config != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_settings_are_unchanged_and_validation_is_idempotent() {
        let mut config = Config::default();
        assert!(!config.sanitize());
        config.gameplay.speed = 2.5;
        config.gameplay.ui_scale = 1.5;
        config.audio.music = 0.37;
        config.graphics.render_scale = 0.75;
        let before = config.clone();
        assert!(!config.sanitize());
        assert_eq!(config, before);
    }

    #[test]
    fn direct_launch_settings_never_feed_nonfinite_values_to_game_or_ui() {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut config = Config::default();
            config.graphics.terrain_relight = value;
            config.graphics.render_scale = value;
            config.audio.master = value;
            config.audio.music = value;
            config.audio.effects = value;
            config.audio.voice = value;
            config.gameplay.ui_scale = value;
            config.gameplay.cursor_scale = value;
            config.gameplay.message_duration = value;
            config.gameplay.gamepad_deadzone = value;
            config.gameplay.gamepad_sensitivity = value;
            config.gameplay.speed = value;
            config.gameplay.scroll_speed = value;
            assert!(config.sanitize());
            assert_eq!(config, Config::default());
            assert!(
                !config.sanitize(),
                "repaired NaN settings must allow ordinary change detection"
            );
        }
    }

    #[test]
    fn hand_edited_toml_is_repaired_before_a_direct_game_launch() {
        let mut config: Config = toml::from_str("[graphics]\nwidth = 4294967295\nheight = 0\nmsaa = 3\nfps_limit = 1000\nrender_scale = inf\n[audio]\nmax_voices = 0\nmaster = -1.0\n[gameplay]\nspeed = nan\nscroll_speed = -2.0\nunit_limit = 4294967295\nui_scale = 99.0\nplayer_name = '指揮官指揮官指揮官指揮官指揮官指揮官指揮官指揮官多'\n").unwrap();
        assert!(config.sanitize());
        assert_eq!(
            (
                config.graphics.width,
                config.graphics.height,
                config.graphics.fps_limit
            ),
            (16384, 480, 480)
        );
        assert!(limits::MSAA.contains(&config.graphics.msaa));
        assert_eq!(
            config.graphics.render_scale,
            Graphics::default().render_scale
        );
        assert_eq!((config.audio.master, config.audio.max_voices), (0.0, 8));
        assert_eq!(
            (config.gameplay.speed, config.gameplay.scroll_speed),
            (1.0, 0.25)
        );
        assert_eq!(config.gameplay.unit_limit, 20_000);
        assert_eq!(config.gameplay.ui_scale, 2.0);
        assert_eq!(
            config.gameplay.player_name.chars().count(),
            limits::PLAYER_NAME_CHARS
        );
        assert!(!config.sanitize());
        let serialized = toml::to_string(&config).unwrap();
        let restored: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(restored, config);
    }
}
