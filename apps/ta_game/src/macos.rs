//! A Mac's Dock and menu bar kept off the game in full screen.
//!
//! The game's full screen is macOS's own (a space of its own; winit's `Fullscreen::Borderless`).
//! There the menu bar comes down when the pointer touches the top of the screen, and the Dock
//! comes up at its edge: scrolling the map by the screen's edges brought them over the battle,
//! and the pointer, over them, left the game, so the scrolling stopped. While the game's window
//! is in full screen, AppKit's presentation options hide both outright; Cmd+Tab still switches
//! apps, and when the window leaves full screen the system restores the options it had before.
//! They are set whenever the window comes into view, takes the keyboard or changes size, as
//! winit's later "borderless game" option does (on coming into view): a full screen's options
//! are AppKit's own again when the player comes back to its space, and options set before the
//! space opened (as winit 0.30's option sets them) are the ones the system restores when it
//! closes, so out of full screen any left over are undone.

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationPresentationOptions as Options};

/// Hides the Dock and the menu bar while the game's window is in full screen (`full_screen`),
/// else undoes any such hiding left over. Called as the window comes into view, takes the
/// keyboard or changes size.
pub fn keep_dock_and_menu_bar_away(full_screen: bool) {
    // (AppKit's application is the main thread's; winit's events come on it.)
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    let hidden = Options::HideDock | Options::HideMenuBar;
    let now = app.presentationOptions();
    if full_screen {
        if !now.contains(hidden) {
            app.setPresentationOptions(hidden);
        }
    } else if now.intersects(hidden) {
        // (Restored with the options from before a full screen began, should a window event
        // have set them before its space opened.)
        app.setPresentationOptions(Options::Default);
    }
}
