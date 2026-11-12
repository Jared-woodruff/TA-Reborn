//! A crash the panic hook never sees (Windows: an access violation in a graphics driver as the
//! skin preview builds its shaders, say): `launcher-crash.txt` and the log name the module that
//! failed, with the stack, and the player is told where; then Windows ends the launcher as it
//! would have. As the game's own (`report_native_crashes` in ta_game's main.rs), without its
//! memory and start-up step lines.

use std::path::PathBuf;
use std::sync::OnceLock;

static PATH: OnceLock<PathBuf> = OnceLock::new();
static UNATTENDED: OnceLock<bool> = OnceLock::new();

/// Whether a module (its file's name, lower case) is part of a graphics driver or the graphics
/// system: what a crash in it most likely means for the player.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn graphics_module(name: &str) -> bool {
    ["amdvlk", "amdxc", "amdxx", "atiumd", "atidxx", "atig6", "nvoglv", "nvwgf2", "nvd3dum", "nvldumd", "nvgpucomp", "igvk", "igd1", "igd12", "igdumd", "igc64", "intelocl", "vulkan-1", "d3d12", "dxgi", "dxcompiler", "d3dcompiler", "opengl32"]
        .iter()
        .any(|k| name.starts_with(k))
}

/// What the report's first line says of a crash in `module` (its file's name).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn what_failed(module: &str, code: u32) -> String {
    if graphics_module(&module.to_ascii_lowercase()) {
        format!("the graphics driver ({module}) failed")
    } else {
        format!("exception {code:#010x} in {module}")
    }
}

/// Reports native crashes to `path` from now on (no box in captures and films: `unattended`).
#[cfg(windows)]
pub fn report_native_crashes(path: PathBuf, unattended: bool) {
    use windows::Win32::System::Diagnostics::Debug::{EXCEPTION_POINTERS, SetUnhandledExceptionFilter};
    let _ = PATH.set(path);
    let _ = UNATTENDED.set(unattended);

    /// The module holding `address` (its file's name) and how far into it the address is.
    fn module_of(address: usize) -> (String, usize) {
        use windows::Win32::Foundation::HMODULE;
        use windows::Win32::System::LibraryLoader::{GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW, GetModuleHandleExW};
        let mut module = HMODULE::default();
        // SAFETY: a query of the module an address lies in, its count of users left alone.
        let found = unsafe {
            GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, windows::core::PCWSTR(address as *const u16), &mut module)
        };
        if found.is_err() || module.is_invalid() {
            return ("an unknown module".into(), address);
        }
        let mut name = [0u16; 260];
        // SAFETY: the module's file name into a buffer of our own.
        let len = unsafe { GetModuleFileNameW(Some(module), &mut name) } as usize;
        let file = String::from_utf16_lossy(&name[..len.min(name.len())]);
        (file.rsplit(['\\', '/']).next().unwrap_or(&file).to_string(), address.wrapping_sub(module.0 as usize))
    }

    unsafe extern "system" fn filter(info: *const EXCEPTION_POINTERS) -> i32 {
        // SAFETY: the system's own record of the exception, valid while this runs.
        let record = unsafe { info.as_ref().and_then(|i| i.ExceptionRecord.as_ref()) };
        let (code, address) = record.map_or((0, 0), |r| (r.ExceptionCode.0 as u32, r.ExceptionAddress as usize));
        let (module, offset) = module_of(address);
        let what = what_failed(&module, code);
        let thread = std::thread::current();
        let report = format!(
            "TA Reborn launcher {} crashed: {what}\nthread: {}\nexception {code:#010x} at {module}+{offset:#x}\n\n{:#}",
            crate::version_label(),
            thread.name().unwrap_or("unnamed"),
            std::backtrace::Backtrace::force_capture()
        );
        log::error!("{report}");
        if let Some(path) = PATH.get() {
            let _ = std::fs::write(path, &report);
        }
        if !UNATTENDED.get().copied().unwrap_or(false) {
            use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
            let advice = if graphics_module(&module.to_ascii_lowercase()) { "\n\nUpdating the graphics driver may help." } else { "" };
            let text = format!("The launcher crashed: {what}.{advice}\n\nThe details were saved to {}.", PATH.get().map_or_else(String::new, |p| p.display().to_string()));
            let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
            let (text, title) = (wide(&text), wide("TA Reborn"));
            // SAFETY: a message box of our own, its strings ending in a nul and alive while it shows.
            unsafe { MessageBoxW(None, windows::core::PCWSTR(text.as_ptr()), windows::core::PCWSTR(title.as_ptr()), MB_OK | MB_ICONERROR) };
        }
        // EXCEPTION_CONTINUE_SEARCH: Windows goes on as it would have (and ends the launcher).
        0
    }
    // SAFETY: a filter of our own, set once as the launcher starts.
    unsafe { SetUnhandledExceptionFilter(Some(filter)) };
}

/// Elsewhere a native crash ends the launcher as before (the system's own report).
#[cfg(not(windows))]
pub fn report_native_crashes(path: PathBuf, unattended: bool) {
    let _ = PATH.set(path);
    let _ = UNATTENDED.set(unattended);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crash_in_a_graphics_driver_is_named_as_one() {
        assert_eq!(what_failed("nvoglv64.dll", 0xc0000005), "the graphics driver (nvoglv64.dll) failed");
        assert_eq!(what_failed("amdvlk64.dll", 0xc0000005), "the graphics driver (amdvlk64.dll) failed");
        assert_eq!(what_failed("igvk64.dll", 0xc0000005), "the graphics driver (igvk64.dll) failed");
        assert_eq!(what_failed("ta-launcher.exe", 0xc0000005), "exception 0xc0000005 in ta-launcher.exe");
    }
}
