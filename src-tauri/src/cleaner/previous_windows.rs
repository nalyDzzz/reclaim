//! Windows.old: the previous Windows an upgrade keeps so you can go back.
//! Windows' own handler removes it. It knows which files are safe to delete
//! and clears the rollback state with them, and it never follows the
//! junctions in the old profiles, some of which point at today's folders.

use std::fs;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use super::{TargetReport, format_bytes, handlers, system_root};
use crate::paths::{self, DirStats};

/// Windows.old, and the numbered copies (Windows.old.000) that setup makes
/// when one is already there, on the system drive.
pub fn folders() -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(system_root()) else { return Vec::new() };
    entries
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_lowercase();
            (name == "windows.old" || name.starts_with("windows.old.")) && e.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink())
        })
        .map(|e| e.path())
        .collect()
}

pub fn analyze(report: &mut TargetReport, elevated: bool, fresh: bool) {
    let folders = folders();
    if folders.is_empty() {
        return;
    }
    report.found = true;
    report.reveal = folders.first().map(|f| f.to_string_lossy().to_string());
    if elevated {
        match handlers::measure(handlers::PREVIOUS_INSTALLATIONS, true, fresh) {
            Ok(bytes) => report.bytes = bytes,
            Err(e) => report.note = Some(format!("{e:#}")),
        }
        if report.bytes == 0 && report.note.is_none() {
            report.note = Some("Windows' own cleanup isn't offering to remove it right now.".into());
        }
    } else {
        // Without administrator rights some of it can't be read, so this
        // can come up short. It's only shown, never used to clean.
        let stats = folders.par_iter().map(|f| paths::dir_stats(f)).reduce(DirStats::default, |a, b| a + b);
        report.bytes = stats.bytes;
    }

    let personal = folders.iter().map(|f| personal_files(&f.join("Users"))).fold(DirStats::default(), |a, b| a + b);
    if personal.files > 0 {
        let what = if personal.files == 1 { "1 file".to_string() } else { format!("{} files", personal.files) };
        report.note = Some(format!(
            "Windows.old\\Users still has {what} ({}) from the old Windows accounts, like documents and pictures. Copy anything you want to keep first.",
            format_bytes(personal.bytes)
        ));
        report.reveal = folders.first().map(|f| f.join("Users").to_string_lossy().to_string());
    }
}

/// What's left in the old accounts' folders, apart from their app data and
/// registry files. After Windows is installed fresh over an old copy, this
/// is where that copy's documents end up.
fn personal_files(users: &Path) -> DirStats {
    let Ok(entries) = fs::read_dir(users) else { return DirStats::default() };
    let profiles: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_lowercase();
            !matches!(name.as_str(), "default" | "default user" | "all users") && e.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink())
        })
        .map(|e| e.path())
        .collect();
    profiles.par_iter().map(|p| walk(p, true)).reduce(DirStats::default, |a, b| a + b)
}

fn walk(dir: &Path, profile_root: bool) -> DirStats {
    let Ok(entries) = fs::read_dir(dir) else { return DirStats::default() };
    let mut stats = DirStats::default();
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        // Profiles are full of junctions, and some lead out of Windows.old.
        if meta.file_type().is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if meta.is_dir() {
            if !(profile_root && name == "appdata") {
                subdirs.push(entry.path());
            }
        } else if name != "desktop.ini" && !(profile_root && name.starts_with("ntuser")) {
            stats = stats + DirStats { bytes: meta.len(), files: 1 };
        }
    }
    stats + subdirs.par_iter().map(|d| walk(d, false)).reduce(DirStats::default, |a, b| a + b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_personal_files_but_not_app_data_or_links() {
        let base = std::env::temp_dir().join(format!("reclaim-oldusers-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let profile = base.join("Users").join("someone");
        for dir in ["Documents", "AppData/Local", "Desktop"] {
            fs::create_dir_all(profile.join(dir)).unwrap();
        }
        fs::create_dir_all(base.join("Users/Default/Documents")).unwrap();
        fs::write(profile.join("Documents/thesis.docx"), vec![1u8; 100]).unwrap();
        fs::write(profile.join("Desktop/desktop.ini"), b"x").unwrap();
        fs::write(profile.join("AppData/Local/cache.bin"), vec![1u8; 1000]).unwrap();
        fs::write(profile.join("NTUSER.DAT"), vec![1u8; 1000]).unwrap();
        fs::write(profile.join(".gitconfig"), vec![1u8; 10]).unwrap();
        fs::write(base.join("Users/Default/Documents/template.txt"), b"x").unwrap();

        let stats = personal_files(&base.join("Users"));
        assert_eq!((stats.files, stats.bytes), (2, 110));
        fs::remove_dir_all(&base).unwrap();
    }
}
