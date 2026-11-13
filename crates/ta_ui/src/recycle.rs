//! Removing the player's things (a mod, a skin, a profile, saved games and replays): they go
//! to the Recycle Bin (Windows) or the system's trash (Linux's freedesktop.org trash, macOS's
//! Trash), so what was removed by mistake can be put back. Where it cannot go there, it stays
//! where it was (Windows asks first, and deletes it for good if told to).

use std::path::Path;

/// What became of what was removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It is in the Recycle Bin (the trash).
    Recycled,
    /// Deleted: it could not go to the Recycle Bin (a network folder, too big for it), and
    /// Windows asked first; or Windows' Recycle Bin of that drive keeps nothing.
    #[cfg_attr(not(windows), allow(dead_code))] // Produced only by the Windows shell backend.
    Deleted,
    /// Left where it was: deleting it for good was not wanted.
    #[cfg_attr(not(windows), allow(dead_code))] // Windows can cancel its deletion dialog.
    Kept,
}

/// Moves `path` (a file or a folder) to the Recycle Bin.
pub fn recycle(path: &Path) -> Result<Outcome, String> {
    if !path.exists() {
        return Err(crate::i18n::tf("{path} is not there", &[("path", &path.display())]));
    }
    recycle_all(&[path])
}

/// Moves `paths` (files or folders, on one drive) to the Recycle Bin in one go; those no longer
/// there are passed over. Kept: some were left, deleting them for good was not wanted.
pub fn recycle_all<P: AsRef<Path>>(paths: &[P]) -> Result<Outcome, String> {
    let paths: Vec<&Path> = paths.iter().map(AsRef::as_ref).filter(|p| p.exists()).collect();
    if paths.is_empty() {
        return Ok(Outcome::Deleted);
    }
    #[cfg(windows)]
    {
        use windows::Win32::UI::Shell::{FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING, SHFILEOPSTRUCTW, SHFileOperationW};
        // (The shell wants full paths, each ended by a NUL and the list by another.)
        let full: Vec<std::path::PathBuf> = paths.iter().map(std::path::absolute).collect::<Result<_, _>>().map_err(|e| e.to_string())?;
        let wide: Vec<u16> = full.iter().flat_map(|p| std::os::windows::ffi::OsStrExt::encode_wide(p.as_os_str()).chain([0])).chain([0]).collect();
        let before = in_bin(&full[0]);
        let mut op = SHFILEOPSTRUCTW {
            // (Windows' question, should it ask one, over the window in front.)
            hwnd: unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() },
            wFunc: FO_DELETE,
            pFrom: windows::core::PCWSTR(wide.as_ptr()),
            // (What cannot go to the Recycle Bin is deleted only once Windows has asked.)
            fFlags: (FOF_ALLOWUNDO.0 | FOF_NOCONFIRMATION.0 | FOF_WANTNUKEWARNING.0 | FOF_NOERRORUI.0 | FOF_SILENT.0) as u16,
            ..Default::default()
        };
        let code = unsafe { SHFileOperationW(&mut op) };
        let left = full.iter().find(|p| p.exists());
        if op.fAnyOperationsAborted.as_bool() && left.is_some() {
            return Ok(Outcome::Kept);
        }
        if code != 0 {
            return Err(if full.len() == 1 {
                crate::i18n::tf("Windows could not move {path} to the Recycle Bin (error {code})", &[("path", &full[0].display()), ("code", &code)])
            } else {
                crate::i18n::tf("Windows could not move them to the Recycle Bin (error {code})", &[("code", &code)])
            });
        }
        if let Some(left) = left {
            return Err(crate::i18n::tf("{path} is still there (is a program using it?)", &[("path", &left.display())]));
        }
        // (Into the Recycle Bin, it holds more things than it did; when that cannot be told,
        // they are taken as deleted.)
        Ok(match (before, in_bin(&full[0])) {
            (Some(before), Some(after)) if after > before => Outcome::Recycled,
            _ => Outcome::Deleted,
        })
    }
    #[cfg(not(windows))]
    {
        #[allow(unused_mut)] // (Only macOS sets its way.)
        let mut context = trash::TrashContext::new();
        #[cfg(target_os = "macos")]
        {
            use trash::macos::{DeleteMethod, TrashContextExtMacos};
            context.set_delete_method(DeleteMethod::NsFileManager);
        }
        // (Never deleted for good instead: the player was told it could be put back.)
        context.delete_all(&paths).map_err(|error| crate::i18n::tf("{path} could not go to the trash: {error}", &[("path", &paths[0].display()), ("error", &error)]))?;
        Ok(Outcome::Recycled)
    }
}

/// How many things the Recycle Bin of `path`'s drive holds (None: it has none).
#[cfg(windows)]
fn in_bin(path: &Path) -> Option<i64> {
    use windows::Win32::UI::Shell::{SHQUERYRBINFO, SHQueryRecycleBinW};
    let root: std::path::PathBuf = path.components().take_while(|c| matches!(c, std::path::Component::Prefix(_) | std::path::Component::RootDir)).collect();
    let mut info = SHQUERYRBINFO { cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32, i64Size: 0, i64NumItems: 0 };
    unsafe { SHQueryRecycleBinW(&windows::core::HSTRING::from(root.as_path()), &mut info) }.ok()?;
    Some(info.i64NumItems)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Recycled, or deleted where the drive's Recycle Bin is set to remove files at once.
    fn gone(outcome: Result<Outcome, String>) -> bool {
        matches!(outcome, Ok(Outcome::Recycled | Outcome::Deleted))
    }

    /// (Ignored: it puts a folder in this computer's Recycle Bin.)
    #[test]
    #[ignore]
    fn a_folder_goes_to_the_recycle_bin() {
        let dir = std::env::temp_dir().join(format!("ta_ui_recycle_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let folder = dir.join("some mod");
        std::fs::create_dir_all(folder.join("units")).unwrap();
        std::fs::write(folder.join("mod.toml"), "id = \"x\"\n").unwrap();
        assert!(gone(recycle(&folder)));
        assert!(!folder.exists());
        assert!(recycle(&folder).is_err(), "nothing left to remove");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// (Ignored: it puts files in this computer's Recycle Bin.)
    #[test]
    #[ignore]
    fn files_go_to_the_recycle_bin_together() {
        let dir = std::env::temp_dir().join(format!("ta_ui_recycle_all_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let files: Vec<_> = (0..3).map(|i| dir.join(format!("replay {i}.tarep"))).collect();
        for file in &files {
            std::fs::write(file, b"x").unwrap();
        }
        let outcome = recycle_all(&[&files[0], &dir.join("already gone.tarep"), &files[1], &files[2]]);
        assert!(gone(outcome.clone()), "{outcome:?}");
        assert!(files.iter().all(|f| !f.exists()));
        assert_eq!(recycle_all(&files), Ok(Outcome::Deleted), "none left: nothing to do");
        let _ = std::fs::remove_dir_all(dir);
    }
}
