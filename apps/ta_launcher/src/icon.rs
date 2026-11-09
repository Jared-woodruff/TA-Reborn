//! The window and taskbar icon: the emblem, rasterised by `ta_ui::icon` at startup.

use egui::IconData;

/// Icon edge length in pixels.
const SIZE: u32 = 64;

pub fn window_icon() -> IconData {
    IconData { rgba: ta_ui::icon::emblem_rgba(SIZE), width: SIZE, height: SIZE }
}
