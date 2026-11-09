//! One launcher at a time (Windows): the first to start makes a named event and waits on it;
//! a second finds the event already there, sets it (letting the first come to the front) and
//! closes, and the first brings its window up. Two launchers saving the same settings file
//! would undo each other's changes. (A claim is made before the log is opened: a second
//! launcher would empty the first's.)

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether this launcher is the one to run.
pub enum Claim {
    /// It is: keep this until the launcher closes ([`Instance::listen`] for the others' calls).
    First(Instance),
    /// Another launcher is running, and has been asked to come to the front.
    #[cfg_attr(not(windows), allow(dead_code))] // Produced only by the Windows event backend.
    Another,
}

/// The event stays open as long as the launcher runs (a thread waits on it): the system closes
/// it with the process.
pub struct Instance {
    #[cfg(windows)]
    event: windows::Win32::Foundation::HANDLE,
    /// Why there is no event (for the log, once it is open).
    #[cfg(windows)]
    failed: Option<String>,
    /// Set when another launcher asked for this one's window.
    pub wanted: Arc<AtomicBool>,
}

#[cfg(windows)]
const NAME: &str = "Local\\TA-Reborn-Launcher-Show";

/// Claims the launcher's place (see the module documentation).
pub fn claim() -> Claim {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
        use windows::Win32::System::Threading::{CreateEventW, SetEvent};
        use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
        let name = windows::core::HSTRING::from(NAME);
        // (Auto-reset: each call wakes the first once.)
        match unsafe { CreateEventW(None, false, false, &name) } {
            Ok(event) if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS => {
                unsafe {
                    // (This process was just started by the player, so it may pass on its right
                    // to bring a window to the front.)
                    let _ = AllowSetForegroundWindow(ASFW_ANY);
                    let _ = SetEvent(event);
                    let _ = windows::Win32::Foundation::CloseHandle(event);
                }
                Claim::Another
            }
            Ok(event) => Claim::First(Instance { event, failed: None, wanted: Arc::default() }),
            Err(e) => Claim::First(Instance { event: windows::Win32::Foundation::HANDLE::default(), failed: Some(e.to_string()), wanted: Arc::default() }),
        }
    }
    #[cfg(not(windows))]
    Claim::First(Instance { wanted: Arc::default() })
}

impl Instance {
    /// Waits (on a thread of its own) for other launchers' calls: each sets [`Instance::wanted`]
    /// and wakes the window.
    pub fn listen(&self, ctx: egui::Context) {
        #[cfg(windows)]
        {
            if self.event.is_invalid() {
                if let Some(e) = &self.failed {
                    log::warn!("single launcher: {e}");
                }
                return;
            }
            let event = self.event.0 as usize;
            let wanted = self.wanted.clone();
            let _ = std::thread::Builder::new().name("launcher calls".into()).spawn(move || {
                use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
                use windows::Win32::System::Threading::{INFINITE, WaitForSingleObject};
                while unsafe { WaitForSingleObject(HANDLE(event as *mut _), INFINITE) } == WAIT_OBJECT_0 {
                    wanted.store(true, Ordering::SeqCst);
                    ctx.request_repaint();
                }
            });
        }
        #[cfg(not(windows))]
        let _ = ctx;
    }

    /// Whether another launcher asked for this one's window since the last look.
    pub fn take_wanted(&self) -> bool {
        self.wanted.swap(false, Ordering::SeqCst)
    }
}

/// Where a second launcher leaves what it was started to show (`--mod`: the game asks for a mod a
/// battle needs), for the first to read when it is called to the front.
fn request_path() -> std::path::PathBuf {
    ta_config::config_dir().join("launcher-request.txt")
}

/// Leaves the mod `id` for the launcher that runs to show (before calling it: [`claim`]).
pub fn ask_for_mod(id: &str) {
    if let Err(e) = std::fs::write(request_path(), id) {
        log::warn!("could not leave the request for the launcher: {e}");
    }
}

/// The mod another launcher left to show, taken (one written over a minute ago is left over from
/// a launcher that never called: not shown).
pub fn take_mod_request() -> Option<String> {
    let path = request_path();
    let fresh = std::fs::metadata(&path).ok()?.modified().ok()?.elapsed().is_ok_and(|age| age.as_secs() < 60);
    let id = std::fs::read_to_string(&path).ok();
    let _ = std::fs::remove_file(&path);
    id.map(|id| id.trim().to_lowercase()).filter(|id| fresh && !id.is_empty() && id.len() <= 1024)
}
