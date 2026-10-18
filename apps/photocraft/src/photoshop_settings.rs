//! Photoshop's live keyboard shortcut set on this machine, for the one-time offer to import it
//! at first launch (`photocraft_ui_egui::kys_import::offer_import`), and its Actions palette
//! (`Actions Palette.psp`: every set of its Actions panel), for the Actions panel menu's Import
//! Photoshop Actions.
//!
//! Photoshop keeps the set in use as `Keyboard Shortcuts.psp` in its per-version settings
//! folder; it is the same XML a saved `.kys` holds, named after the set the user chose.
//! macOS: `~/Library/Preferences/Adobe Photoshop <version> Settings/`. Windows:
//! `%APPDATA%\Adobe\Adobe Photoshop <version>\Adobe Photoshop <version> Settings\`. Every
//! installed version has one; the newest version wins (the number in the folder name, then
//! the file's modification time), since that is the Photoshop the user runs. Linux has no
//! Photoshop; a copied set is imported by hand (Edit › Keyboard Shortcuts).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The file is a few tens of kilobytes; anything far larger is not a shortcut set.
const MAX_BYTES: u64 = 4 << 20;

/// The platform whose folder layout applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
    Other,
}

const fn this_platform() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::Mac
    } else if cfg!(target_os = "windows") {
        Platform::Windows
    } else {
        Platform::Other
    }
}

/// Every `Adobe Photoshop <version> Settings` folder that could hold a set, newest first.
pub fn settings_dirs(os: Platform, env: &impl Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let var = |k: &str| env(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let mut dirs = match os {
        Platform::Mac => var("HOME").map(|h| h.join("Library/Preferences")).into_iter().flat_map(|prefs| settings_in(&prefs)).collect::<Vec<_>>(),
        // `%APPDATA%\Adobe\Adobe Photoshop 2026\Adobe Photoshop 2026 Settings`.
        Platform::Windows => var("APPDATA")
            .map(|a| a.join("Adobe"))
            .into_iter()
            .flat_map(|adobe| std::fs::read_dir(adobe).into_iter().flatten().flatten().map(|e| e.path()))
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("Adobe Photoshop ")))
            .flat_map(|p| settings_in(&p))
            .collect(),
        Platform::Other => Vec::new(),
    };
    dirs.sort_by_key(|d| std::cmp::Reverse(version_key(d)));
    dirs
}

/// `Adobe Photoshop <version> Settings` folders directly inside `parent`.
fn settings_in(parent: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(parent)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            name.starts_with("Adobe Photoshop ") && name.ends_with(" Settings") && p.is_dir()
        })
        .collect()
}

/// Newest first: the version number in the folder name (2026; CC 2019 → 2019; CS6 → 6), then
/// the set's modification time.
fn version_key(dir: &Path) -> (u64, std::time::SystemTime) {
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let number = name.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).filter_map(|s| s.parse::<u64>().ok()).max().unwrap_or(0);
    let modified = std::fs::metadata(dir.join(FILE)).and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    (number, modified)
}

/// The Actions palette's file name inside a settings folder.
pub const ACTIONS_FILE: &str = "Actions Palette.psp";

/// A palette is a few megabytes at most (the user's: 5 MB, nine sets); far larger is not one.
const MAX_ACTIONS_BYTES: u64 = 256 << 20;

/// The newest install's Actions palette as (path, bytes), `None` without a readable one.
pub fn actions_palette_on(os: Platform, env: &impl Fn(&str) -> Option<OsString>) -> Option<(String, Vec<u8>)> {
    settings_dirs(os, env).into_iter().map(|d| d.join(ACTIONS_FILE)).find_map(|path| {
        let len = std::fs::metadata(&path).ok()?.len();
        if len > MAX_ACTIONS_BYTES {
            return None;
        }
        let bytes = std::fs::read(&path).ok()?;
        // Version 16, then the set count.
        bytes.starts_with(&[0, 0, 0, 16]).then(|| (path.to_string_lossy().into_owned(), bytes))
    })
}

/// [`actions_palette_on`] for this platform.
pub fn actions_palette(env: &impl Fn(&str) -> Option<OsString>) -> Option<(String, Vec<u8>)> {
    actions_palette_on(this_platform(), env)
}

/// The live set's file name inside a settings folder.
pub const FILE: &str = "Keyboard Shortcuts.psp";

/// The newest install's live set as (path, XML text), `None` without a readable one. A file that
/// is not a shortcut set (another version's binary `.psp`, a truncated file) is skipped.
pub fn live_keyboard_shortcuts_on(os: Platform, env: &impl Fn(&str) -> Option<OsString>) -> Option<(String, String)> {
    settings_dirs(os, env).into_iter().map(|d| d.join(FILE)).find_map(|path| {
        let len = std::fs::metadata(&path).ok()?.len();
        if len > MAX_BYTES {
            return None;
        }
        let text = std::fs::read_to_string(&path).ok()?;
        text.contains("<photoshop-keyboard-shortcuts").then(|| (path.to_string_lossy().into_owned(), text))
    })
}

/// [`live_keyboard_shortcuts_on`] for this platform.
pub fn live_keyboard_shortcuts(env: &impl Fn(&str) -> Option<OsString>) -> Option<(String, String)> {
    live_keyboard_shortcuts_on(this_platform(), env)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SET: &str = "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<photoshop-keyboard-shortcuts version=\"4\" filename=\"texcuts\" modified=\"1\" multi-undo=\"1\">\n</photoshop-keyboard-shortcuts>\n";

    /// A fresh scratch directory per test (removed first, so a rerun starts clean).
    fn temp_root(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("photocraft-photoshop-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(dir: &Path, text: &[u8]) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(FILE), text).unwrap();
    }

    fn env_for(root: &Path) -> impl Fn(&str) -> Option<OsString> + '_ {
        move |k| match k {
            "HOME" => Some(root.join("home").into_os_string()),
            "APPDATA" => Some(root.join("appdata").into_os_string()),
            _ => None,
        }
    }

    #[test]
    fn newest_version_wins_on_macos() {
        let root = temp_root("mac");
        let prefs = root.join("home/Library/Preferences");
        write(&prefs.join("Adobe Photoshop 2024 Settings"), SET.as_bytes());
        write(&prefs.join("Adobe Photoshop 2026 Settings"), SET.replace("texcuts", "newest").as_bytes());
        write(&prefs.join("Adobe Photoshop CC 2019 Settings"), SET.as_bytes());
        write(&prefs.join("Adobe Photoshop 2025 Paths"), SET.as_bytes()); // not a Settings folder
        let env = env_for(&root);
        let dirs: Vec<String> = settings_dirs(Platform::Mac, &env).iter().map(|d| d.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(dirs, ["Adobe Photoshop 2026 Settings", "Adobe Photoshop 2024 Settings", "Adobe Photoshop CC 2019 Settings"]);
        let (path, text) = live_keyboard_shortcuts_on(Platform::Mac, &env).unwrap();
        assert!(path.replace('\\', "/").ends_with("Adobe Photoshop 2026 Settings/Keyboard Shortcuts.psp"), "{path}");
        assert!(text.contains("filename=\"newest\""));
    }

    #[test]
    fn windows_layout_and_unreadable_sets_are_skipped() {
        let root = temp_root("win");
        let adobe = root.join("appdata/Adobe");
        // The newest install's file is not a shortcut set (binary); the older one is used.
        write(&adobe.join("Adobe Photoshop 2026/Adobe Photoshop 2026 Settings"), &[0x80, 0x00, 0xff, 0xfe]);
        write(&adobe.join("Adobe Photoshop 2025/Adobe Photoshop 2025 Settings"), SET.as_bytes());
        std::fs::create_dir_all(adobe.join("Adobe Illustrator 30/Adobe Illustrator 30 Settings")).unwrap();
        let env = env_for(&root);
        let (path, _) = live_keyboard_shortcuts_on(Platform::Windows, &env).unwrap();
        assert!(path.replace('\\', "/").ends_with("Adobe Photoshop 2025/Adobe Photoshop 2025 Settings/Keyboard Shortcuts.psp"), "{path}");
        assert!(live_keyboard_shortcuts_on(Platform::Other, &env).is_none());
        assert!(live_keyboard_shortcuts_on(Platform::Mac, &|_| None).is_none(), "no HOME, no set");
    }

    /// The newest install's `Actions Palette.psp` (version 16, then the set count); a file that
    /// is not a palette is skipped for the next install's.
    #[test]
    fn the_newest_actions_palette_is_found() {
        let root = temp_root("actions");
        let adobe = root.join("appdata/Adobe");
        let put = |dir: &Path, bytes: &[u8]| {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(ACTIONS_FILE), bytes).unwrap();
        };
        put(&adobe.join("Adobe Photoshop 2025/Adobe Photoshop 2025 Settings"), &[0, 0, 0, 16, 0, 0, 0, 1]);
        put(&adobe.join("Adobe Photoshop 2024/Adobe Photoshop 2024 Settings"), &[0, 0, 0, 16, 0, 0, 0, 2]);
        put(&adobe.join("Adobe Photoshop 2026/Adobe Photoshop 2026 Settings"), b"not a palette");
        let env = env_for(&root);
        let (path, bytes) = actions_palette_on(Platform::Windows, &env).unwrap();
        assert!(path.replace('\\', "/").ends_with("Adobe Photoshop 2025/Adobe Photoshop 2025 Settings/Actions Palette.psp"), "{path}");
        assert_eq!(bytes, [0, 0, 0, 16, 0, 0, 0, 1]);
        assert!(actions_palette_on(Platform::Other, &env).is_none());
        assert!(actions_palette_on(Platform::Windows, &|_| None).is_none(), "no APPDATA, no palette");
    }
}
