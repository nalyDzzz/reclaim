//! Junk and cache cleanup, in the spirit of BleachBit.

mod targets;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use rayon::prelude::*;
use serde::Serialize;
use tauri::ipc::Channel;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::UI::Shell::{
    SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND, SHEmptyRecycleBinW, SHQUERYRBINFO, SHQueryRecycleBinW,
};
use windows::core::PCWSTR;

use crate::error::CmdResult;
use crate::paths::{DirStats, expand_pattern, is_cloud_placeholder};
use crate::system::is_elevated;
use targets::{RECYCLE_BIN, TARGETS, Target};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetInfo {
    id: &'static str,
    name: &'static str,
    category: &'static str,
    description: &'static str,
    requires_admin: bool,
    default_selected: bool,
}

#[tauri::command]
pub fn cleanup_targets() -> Vec<TargetInfo> {
    TARGETS
        .iter()
        .map(|t| TargetInfo {
            id: t.id,
            name: t.name,
            category: t.category,
            description: t.description,
            requires_admin: t.requires_admin,
            default_selected: t.default_selected,
        })
        .collect()
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TargetReport {
    id: &'static str,
    bytes: u64,
    files: u64,
    /// Whether anything for this target exists on this PC.
    found: bool,
    /// Needs administrator rights that Reclaim doesn't currently have.
    blocked: bool,
    /// The app is running, so some files may be locked.
    running: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanReport {
    id: &'static str,
    freed_bytes: u64,
    deleted_files: u64,
    failed_files: u64,
}

fn running_processes() -> HashSet<String> {
    let mut names = HashSet::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return names };
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let end = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                names.insert(String::from_utf16_lossy(&entry.szExeFile[..end]).to_lowercase());
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    names
}

fn cutoff(target: &Target) -> Option<SystemTime> {
    (target.min_age_hours > 0).then(|| SystemTime::now() - Duration::from_secs(target.min_age_hours * 3600))
}

fn old_enough(meta: &fs::Metadata, cutoff: Option<SystemTime>) -> bool {
    match cutoff {
        None => true,
        Some(c) => meta.modified().map(|m| m < c).unwrap_or(false),
    }
}

/// Stats for what cleaning `path` would remove.
fn measure(path: &Path, cutoff: Option<SystemTime>) -> DirStats {
    let Ok(meta) = fs::symlink_metadata(path) else { return DirStats::default() };
    if meta.file_type().is_symlink() {
        return DirStats::default();
    }
    if !meta.is_dir() {
        use std::os::windows::fs::MetadataExt;
        return if old_enough(&meta, cutoff) && !is_cloud_placeholder(meta.file_attributes()) {
            DirStats { bytes: meta.len(), files: 1 }
        } else {
            DirStats::default()
        };
    }
    let Ok(entries) = fs::read_dir(path) else { return DirStats::default() };
    let children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    children.par_iter().map(|c| measure(c, cutoff)).reduce(DirStats::default, |a, b| a + b)
}

/// Deletes everything under `path` (but not `path` itself if it's a
/// folder). Locked files are skipped and counted as failures.
fn clean(path: &Path, cutoff: Option<SystemTime>, keep_root: bool) -> CleanTally {
    let Ok(meta) = fs::symlink_metadata(path) else { return CleanTally::default() };
    if meta.file_type().is_symlink() {
        return CleanTally::default();
    }
    if !meta.is_dir() {
        if !old_enough(&meta, cutoff) {
            return CleanTally::default();
        }
        let size = meta.len();
        let mut result = fs::remove_file(path);
        if result.is_err() && meta.permissions().readonly() {
            let mut perms = meta.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            let _ = fs::set_permissions(path, perms);
            result = fs::remove_file(path);
        }
        return match result {
            Ok(()) => CleanTally { freed: size, deleted: 1, failed: 0 },
            Err(_) => CleanTally { freed: 0, deleted: 0, failed: 1 },
        };
    }
    let Ok(entries) = fs::read_dir(path) else { return CleanTally { failed: 1, ..Default::default() } };
    let children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    let tally = children.par_iter().map(|c| clean(c, cutoff, false)).reduce(CleanTally::default, |a, b| a + b);
    if !keep_root {
        let _ = fs::remove_dir(path); // only succeeds once empty
    }
    tally
}

#[derive(Default, Clone, Copy)]
struct CleanTally {
    freed: u64,
    deleted: u64,
    failed: u64,
}

impl std::ops::Add for CleanTally {
    type Output = CleanTally;
    fn add(self, o: CleanTally) -> CleanTally {
        CleanTally { freed: self.freed + o.freed, deleted: self.deleted + o.deleted, failed: self.failed + o.failed }
    }
}

fn recycle_bin_stats() -> DirStats {
    let mut info = SHQUERYRBINFO { cbSize: size_of::<SHQUERYRBINFO>() as u32, ..Default::default() };
    match unsafe { SHQueryRecycleBinW(PCWSTR::null(), &mut info) } {
        Ok(()) => DirStats { bytes: info.i64Size.max(0) as u64, files: info.i64NumItems.max(0) as u64 },
        Err(_) => DirStats::default(),
    }
}

fn find_targets(ids: &[String]) -> Vec<&'static Target> {
    TARGETS.iter().filter(|t| ids.iter().any(|id| id == t.id)).collect()
}

#[tauri::command]
pub async fn analyze_cleanup(ids: Vec<String>, on_report: Channel<TargetReport>) -> CmdResult<Vec<TargetReport>> {
    let reports = tauri::async_runtime::spawn_blocking(move || {
        let elevated = is_elevated();
        let running = running_processes();
        find_targets(&ids)
            .par_iter()
            .map(|t| {
                let report = analyze_target(t, elevated, &running);
                // Stream each result so the list fills in while slow targets finish.
                let _ = on_report.send(report.clone());
                report
            })
            .collect::<Vec<_>>()
    })
    .await?;
    Ok(reports)
}

fn analyze_target(t: &Target, elevated: bool, running: &HashSet<String>) -> TargetReport {
    let (stats, found) = if t.id == RECYCLE_BIN {
        (recycle_bin_stats(), true)
    } else {
        let paths: Vec<PathBuf> = t.patterns.iter().flat_map(|p| expand_pattern(p)).collect();
        let cut = cutoff(t);
        let stats = paths.par_iter().map(|p| measure(p, cut)).reduce(DirStats::default, |a, b| a + b);
        (stats, !paths.is_empty())
    };
    TargetReport {
        id: t.id,
        bytes: stats.bytes,
        files: stats.files,
        found,
        blocked: t.requires_admin && !elevated,
        running: t.processes.iter().any(|p| running.contains(&p.to_lowercase())),
    }
}

#[tauri::command]
pub async fn run_cleanup(ids: Vec<String>) -> CmdResult<Vec<CleanReport>> {
    let reports = tauri::async_runtime::spawn_blocking(move || {
        let elevated = is_elevated();
        find_targets(&ids)
            .into_iter()
            .filter(|t| !t.requires_admin || elevated)
            .map(|t| {
                if t.id == RECYCLE_BIN {
                    let before = recycle_bin_stats();
                    let flags = SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND;
                    let ok = unsafe { SHEmptyRecycleBinW(None, PCWSTR::null(), flags) }.is_ok();
                    let after = if ok { DirStats::default() } else { recycle_bin_stats() };
                    return CleanReport {
                        id: t.id,
                        freed_bytes: before.bytes.saturating_sub(after.bytes),
                        deleted_files: before.files.saturating_sub(after.files),
                        failed_files: after.files,
                    };
                }
                let cut = cutoff(t);
                let tally = t
                    .patterns
                    .iter()
                    .flat_map(|p| expand_pattern(p))
                    .map(|p| clean(&p, cut, true))
                    .fold(CleanTally::default(), |a, b| a + b);
                CleanReport { id: t.id, freed_bytes: tally.freed, deleted_files: tally.deleted, failed_files: tally.failed }
            })
            .collect::<Vec<_>>()
    })
    .await?;
    Ok(reports)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_ids_are_unique() {
        let mut seen = HashSet::new();
        for t in TARGETS {
            assert!(seen.insert(t.id), "duplicate target id {}", t.id);
        }
    }

    #[test]
    fn cleans_contents_but_keeps_root() {
        let dir = std::env::temp_dir().join(format!("reclaim-clean-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("nested")).unwrap();
        fs::write(dir.join("a.tmp"), vec![1u8; 10]).unwrap();
        fs::write(dir.join("nested/b.tmp"), vec![1u8; 20]).unwrap();
        assert_eq!(measure(&dir, None).bytes, 30);
        let tally = clean(&dir, None, true);
        assert_eq!((tally.freed, tally.deleted, tally.failed), (30, 2, 0));
        assert!(dir.exists());
        assert!(!dir.join("nested").exists());
        // Files newer than the cutoff are kept.
        fs::write(dir.join("fresh.tmp"), vec![1u8; 5]).unwrap();
        let hour_ago = Some(SystemTime::now() - Duration::from_secs(3600));
        assert_eq!(clean(&dir, hour_ago, true).deleted, 0);
        fs::remove_dir_all(&dir).unwrap();
    }
}
