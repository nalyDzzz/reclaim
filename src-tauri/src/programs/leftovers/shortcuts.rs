//! Shortcuts to the app in the Start menu, the Startup folders, and on the
//! desktops, found by where they point as well as by name.

use std::path::{Path, PathBuf};

use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, IPersistFile, STGM_READ};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::core::{HSTRING, Interface};

use super::{Collector, Confidence, NameVariants, is_shortcut, is_uninstall_shortcut, points_into, stem};
use crate::paths::known;

/// Start menu folders deeper than this are left alone.
const MAX_DEPTH: usize = 3;

/// Where a `.lnk` file points, as saved in it. None for other files and for
/// shortcuts to things that aren't files, like Control Panel pages.
pub(super) fn target(path: &Path) -> Option<String> {
    if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk")) {
        return None;
    }
    unsafe {
        // Either apartment works, and the thread may already be in one.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        link.cast::<IPersistFile>().ok()?.Load(&HSTRING::from(path.as_os_str()), STGM_READ).ok()?;
        let mut buf = [0u16; 1024];
        link.GetPath(&mut buf, std::ptr::null_mut(), 0).ok()?;
        let len = buf.iter().position(|&c| c == 0)?;
        (len > 0).then(|| String::from_utf16_lossy(&buf[..len]))
    }
}

/// How sure Reclaim is that a shortcut is the app's: it points into the
/// app's folder, or it's named after the app.
fn confidence(path: &Path, names: &NameVariants, folders: &[(String, Confidence)]) -> Option<Confidence> {
    let by_target = target(path).and_then(|t| points_into(&t, folders));
    let by_name = names.matches_name(&stem(path)).then_some(Confidence::High);
    by_target.max(by_name)
}

/// What one Start menu folder holds of the app's.
struct Walked {
    items: Vec<(PathBuf, Confidence, &'static str)>,
    /// Everything in it is the app's: its shortcuts, its uninstall shortcut,
    /// and folders that are all the app's too.
    all_ours: bool,
}

fn walk(dir: &Path, depth: usize, names: &NameVariants, folders: &[(String, Confidence)]) -> Walked {
    let k = known();
    let startup = [k.start_menu_user.join("Startup"), k.start_menu_common.join("Startup")];
    // The Start menu's roots and the Startup folders are Windows', so only
    // what's inside them can go.
    let is_root = depth == 0 || startup.iter().any(|s| s == dir);
    let reason = if startup.iter().any(|s| s == dir) { "Startup shortcut" } else { "Shortcut to the app" };
    let mut walked = Walked { items: Vec::new(), all_ours: !is_root };
    let Ok(entries) = std::fs::read_dir(dir) else { return Walked { items: Vec::new(), all_ours: false } };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth + 1 >= MAX_DEPTH {
                walked.all_ours = false;
                continue;
            }
            let inner = walk(&path, depth + 1, names, folders);
            walked.all_ours &= inner.all_ours;
            walked.items.extend(inner.items);
        } else if let Some(confidence) = is_shortcut(&path).then(|| confidence(&path, names, folders)).flatten() {
            walked.items.push((path, confidence, reason));
        } else if !is_uninstall_shortcut(&path) && !entry.file_name().eq_ignore_ascii_case("desktop.ini") {
            walked.all_ours = false;
        }
    }
    let named = dir.file_name().is_some_and(|n| names.matches_name(&n.to_string_lossy()));
    if !is_root && (named || (walked.all_ours && !walked.items.is_empty())) {
        let confidence = if named { Confidence::High } else { walked.items.iter().map(|(_, c, _)| *c).min().unwrap_or(Confidence::Low) };
        return Walked { items: vec![(dir.to_path_buf(), confidence, "Start menu folder")], all_ours: walked.all_ours };
    }
    walked
}

/// Finds the app's shortcuts. The Start menu is searched through, but the
/// desktops only at the top, since their folders belong to the person.
pub(super) fn find(names: &NameVariants, folders: &[(String, Confidence)], out: &mut Collector) {
    let k = known();
    for root in [&k.start_menu_user, &k.start_menu_common] {
        for (path, confidence, reason) in walk(root, 0, names, folders).items {
            out.push_path(&path, confidence, reason);
        }
    }
    let public_desktop = std::env::var_os("PUBLIC").map(|p| PathBuf::from(p).join("Desktop"));
    for desktop in [Some(k.user_profile.join("Desktop")), public_desktop].into_iter().flatten() {
        let Ok(entries) = std::fs::read_dir(&desktop) else { continue };
        for path in entries.flatten().map(|e| e.path()).filter(|p| p.is_file() && is_shortcut(p)) {
            if let Some(confidence) = confidence(&path, names, folders) {
                out.push_path(&path, confidence, "Desktop shortcut");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_link(at: &Path, to: &str) {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
            link.SetPath(&HSTRING::from(to)).unwrap();
            link.cast::<IPersistFile>().unwrap().Save(&HSTRING::from(at.as_os_str()), true).unwrap();
        }
    }

    #[test]
    fn matches_shortcuts_by_where_they_point() {
        let base = std::env::temp_dir().join(format!("reclaim-shortcut-test-{}", std::process::id()));
        let app = r"C:\Program Files\ReclaimTestApp";
        let folders = [(crate::paths::normalize(Path::new(app)), Confidence::High)];
        let names = NameVariants::new("Reclaim Test App", Some("Reclaim Test Co"));
        // A folder of only the app's shortcuts, one shared with something else,
        // and a shortcut named nothing like the app.
        let ours = base.join("Tools Suite");
        let shared = base.join("Utilities");
        for dir in [&ours, &shared] {
            std::fs::create_dir_all(dir).unwrap();
        }
        make_link(&ours.join("Launch.lnk"), &format!(r"{app}\app.exe"));
        make_link(&ours.join("Uninstall.lnk"), &format!(r"{app}\unins000.exe"));
        make_link(&shared.join("Helper.lnk"), &format!(r"{app}\helper.exe"));
        make_link(&shared.join("Other.lnk"), r"C:\Program Files\Other\other.exe");

        assert_eq!(target(&ours.join("Launch.lnk")).as_deref(), Some(&*format!(r"{app}\app.exe")));
        let walked = walk(&base, 0, &names, &folders);
        let _ = std::fs::remove_dir_all(&base);

        let mut items: Vec<String> = walked.items.iter().map(|(p, _, _)| p.strip_prefix(&base).unwrap().display().to_string()).collect();
        items.sort();
        assert_eq!(items, ["Tools Suite", r"Utilities\Helper.lnk"]);
        assert!(!walked.all_ours, "the root itself is never taken");
    }
}
