//! A Mac's Dock and menu bar kept off the game in full screen, and the system's pointer off TA's.
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
//!
//! winit hides the system's pointer (while TA's is drawn) with AppKit's cursor rectangles: an
//! invisible cursor over the window's view, which AppKit sets as the pointer enters it, or when the
//! rectangles are made again. Where macOS puts its arrow back with the pointer already inside (a
//! full screen opening, the presentation options changing), it stayed, over TA's pointer: in full
//! screen the pointer never leaves the window to come back in (it used to, into the Dock). So the
//! rectangles are made again whenever the options are set, and once a second while the window has
//! the keyboard (`refresh_cursor_now_and_then`): AppKit then sets the pointer the window wants.

use std::cell::Cell;
use std::time::{Duration, Instant};

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationPresentationOptions as Options, NSView};
use winit::window::Window;

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

/// Has AppKit make the window's cursor rectangles again, and so set the pointer the window wants
/// where it is now (winit's: the system's, or none while TA's is drawn).
pub fn refresh_cursor(window: &Window) {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    if MainThreadMarker::new().is_none() {
        return;
    }
    let Ok(handle) = window.window_handle() else { return };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else { return };
    // SAFETY: the handle is the view of a window we hold (alive), and this is the main thread.
    let view: &NSView = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    if let Some(ns_window) = view.window() {
        ns_window.invalidateCursorRectsForView(view);
    }
}

/// [`refresh_cursor`] once a second at most, while the window has the keyboard: called every frame.
pub fn refresh_cursor_now_and_then(window: &Window) {
    thread_local! {
        static LAST: Cell<Option<Instant>> = const { Cell::new(None) };
    }
    if !window.has_focus() {
        return;
    }
    let now = Instant::now();
    let due = LAST.with(|last| {
        let due = last.get().is_none_or(|t| now.duration_since(t) >= Duration::from_secs(1));
        if due {
            last.set(Some(now));
        }
        due
    });
    if due {
        refresh_cursor(window);
    }
}
