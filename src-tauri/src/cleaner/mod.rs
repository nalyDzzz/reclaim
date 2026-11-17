//! Junk and cache cleanup, in the spirit of BleachBit.

mod handlers;
mod previous_windows;
mod targets;

use std::collections::HashSet;
use std::fs;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use rayon::prelude::*;
use serde::Serialize;
use tauri::ipc::Channel;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::UI::Shell::{
    SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND, SHEmptyRecycleBinW, SHQUERYRBINFO, SHQueryRecycleBinW,
};
use windows::core::{HSTRING, PCWSTR};

use crate::error::CmdResult;
use crate::paths::{self, DirStats, is_cloud_placeholder};
use crate::system::is_elevated;
use targets::{Kind, TARGETS, Target};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetInfo {
    id: &'static str,
    name: &'static str,
    category: &'static str,
    description: &'static str,
    requires_admin: bool,
    default_selected: bool,
    gives_up: Option<&'static str>,
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
            gives_up: t.gives_up,
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
    /// More about what's there, worth knowing before cleaning it.
    note: Option<String>,
    /// A folder worth looking through before it's gone.
    reveal: Option<String>,
}

impl TargetReport {
    fn new(t: &Target, elevated: bool) -> Self {
        TargetReport {
            id: t.id,
            bytes: 0,
            files: 0,
            found: false,
            blocked: t.requires_admin && !elevated,
            running: false,
            note: None,
            reveal: None,
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CleanReport {
    id: &'static str,
    freed_bytes: u64,
    deleted_files: u64,
    failed_files: u64,
    /// Why a cleanup that isn't a file-by-file delete stopped or fell short.
    error: Option<String>,
}

impl CleanReport {
    fn new(id: &'static str) -> Self {
        CleanReport { id, freed_bytes: 0, deleted_files: 0, failed_files: 0, error: None }
    }
}

/// Sent as `run_cleanup` works through the targets.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CleanStep {
    id: &'static str,
    /// Set once the target is done. Until then it's being cleaned.
    report: Option<CleanReport>,
    /// How far along a long cleanup is, from 0 to 1, when Windows says.
    progress: Option<f64>,
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

/// Stats for what cleaning `path` would remove. This is only an estimate:
/// cleaning checks each file again before deleting it.
fn measure(path: &Path, cutoff: Option<SystemTime>) -> DirStats {
    let cleanable = |meta: &fs::Metadata| old_enough(meta, cutoff) && !is_cloud_placeholder(meta.file_attributes());
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => DirStats::default(),
        Ok(meta) if meta.is_dir() => paths::dir_stats_where(path, &cleanable),
        Ok(meta) if cleanable(&meta) => DirStats { bytes: meta.len(), files: 1 },
        _ => DirStats::default(),
    }
}

// Deletion has to stay safe when Reclaim runs elevated and the folder being
// emptied is writable by other users (C:\Windows\Temp is). A path-based walk
// is open to a swap race: check that `dir` is a real folder, then another
// user replaces it with a junction to System32 before it's opened again.
//
// So every directory from the drive root down to the one being worked on is
// "pinned": opened without following reparse points, confirmed not to be a
// junction or symlink *from the open handle*, and held open without sharing
// delete access. While pinned it can't be renamed, deleted, or replaced, so
// paths through it keep resolving to the folder that was checked. A file or
// link can still be swapped in at the final component, but DeleteFileW and
// RemoveDirectoryW act on a link itself, never its target.

const FILE_LIST_DIRECTORY: u32 = 0x1;
const FILE_READ_ATTRIBUTES: u32 = 0x80;
const FILE_SHARE_READ: u32 = 0x1;
const FILE_SHARE_WRITE: u32 = 0x2;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

fn is_link(meta: &fs::Metadata) -> bool {
    meta.file_type().is_symlink() || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

/// Opens a directory without following reparse points and without sharing
/// delete access. Returns `None` if it's a junction, symlink, or not a folder.
fn pin_dir(path: &Path) -> Option<fs::File> {
    let file = fs::OpenOptions::new()
        // List access (not just attributes) makes Windows enforce the share mode.
        .access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    (meta.is_dir() && !is_link(&meta)).then_some(file)
}

/// Pins every directory from just below the volume root down to `dir`.
fn pin_chain(dir: &Path) -> Option<Vec<fs::File>> {
    let mut pins = Vec::new();
    let mut current = PathBuf::new();
    for component in dir.components() {
        current.push(component);
        // The volume root can't be renamed or replaced, so it needs no pin.
        if matches!(component, Component::Prefix(_) | Component::RootDir) {
            continue;
        }
        pins.push(pin_dir(&current)?);
    }
    Some(pins)
}

/// Empties a cleanup root (or deletes it if it's a file). Locked files are
/// skipped and counted as failures; links are never followed or removed.
fn clean_root(root: &Path, cutoff: Option<SystemTime>) -> CleanTally {
    let Ok(meta) = fs::symlink_metadata(root) else { return CleanTally::default() };
    if is_link(&meta) {
        return CleanTally::default();
    }
    let Some(parent) = root.parent() else { return CleanTally::default() };
    let Some(_ancestors) = pin_chain(parent) else { return CleanTally::failed(1) };
    if meta.is_dir() {
        match pin_dir(root) {
            Some(pin) => clean_dir(root, pin, cutoff),
            None => CleanTally::failed(1),
        }
    } else {
        delete_file(root, &meta, cutoff)
    }
}

/// Deletes the contents of `dir`. The caller has pinned all of its
/// ancestors, and `pin` holds `dir` itself until this returns.
fn clean_dir(dir: &Path, pin: fs::File, cutoff: Option<SystemTime>) -> CleanTally {
    let Ok(entries) = fs::read_dir(dir) else { return CleanTally::failed(1) };
    let children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    let tally = children
        .par_iter()
        .map(|child| {
            let Ok(meta) = fs::symlink_metadata(child) else { return CleanTally::default() };
            if is_link(&meta) {
                return CleanTally::default();
            }
            if !meta.is_dir() {
                return delete_file(child, &meta, cutoff);
            }
            // Re-checked from the handle, so a folder swapped for a junction
            // since `symlink_metadata` is refused here.
            let Some(child_pin) = pin_dir(child) else { return CleanTally::failed(1) };
            let tally = clean_dir(child, child_pin, cutoff);
            let _ = fs::remove_dir(child); // only succeeds once empty
            tally
        })
        .reduce(CleanTally::default, |a, b| a + b);
    drop(pin);
    tally
}

fn delete_file(path: &Path, meta: &fs::Metadata, cutoff: Option<SystemTime>) -> CleanTally {
    if !old_enough(meta, cutoff) {
        return CleanTally::default();
    }
    let mut result = fs::remove_file(path);
    if result.is_err() && meta.permissions().readonly() {
        let mut perms = meta.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        let _ = fs::set_permissions(path, perms);
        result = fs::remove_file(path);
    }
    match result {
        Ok(()) => CleanTally { freed: meta.len(), deleted: 1, failed: 0 },
        Err(_) => CleanTally::failed(1),
    }
}

/// Variables that point at user-controlled locations. Anyone can change
/// these for their own account, so roots built from them are checked.
const PER_USER_VARS: &[&str] = &["%TEMP%", "%LOCALAPPDATA%", "%APPDATA%"];

/// Machine-wide variables resolve through Windows APIs instead of the
/// environment, which a standard user can override.
fn trusted_var(name: &str) -> Option<String> {
    let k = paths::known();
    let path = match name.to_ascii_lowercase().as_str() {
        "systemroot" | "windir" => &k.windows,
        "programdata" => &k.program_data,
        "programfiles" | "programw6432" => &k.program_files,
        "programfiles(x86)" => &k.program_files_x86,
        "userprofile" => &k.user_profile,
        _ => return std::env::var(name).ok(),
    };
    Some(path.to_string_lossy().to_string())
}

/// Folders and files a target may clean. Roots built from per-user
/// variables must sit inside your own profile, so a redirected `%TEMP%`
/// can never point an elevated cleanup at Windows, Program Files, or
/// another account's files.
fn target_roots(t: &Target) -> Vec<PathBuf> {
    t.patterns
        .iter()
        .flat_map(|pattern| {
            let per_user = PER_USER_VARS.iter().any(|v| pattern.to_ascii_uppercase().starts_with(v));
            paths::expand_glob(&paths::expand_vars(pattern, trusted_var))
                .into_iter()
                .filter(move |root| !per_user || per_user_root_allowed(root))
        })
        .collect()
}

fn per_user_root_allowed(root: &Path) -> bool {
    let k = paths::known();
    // %TEMP% is sometimes stored with 8.3 short names (C:\Users\RUNNER~1\...).
    let p = paths::normalize(&paths::long_path(root));
    let in_profile = p.starts_with(&format!("{}\\", paths::normalize(&k.user_profile)));
    in_profile && (!paths::is_protected(root) || p == paths::normalize(&k.temp))
}

#[derive(Default, Clone, Copy)]
struct CleanTally {
    freed: u64,
    deleted: u64,
    failed: u64,
}

impl CleanTally {
    fn failed(n: u64) -> Self {
        CleanTally { freed: 0, deleted: 0, failed: n }
    }
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

/// Checks how much each target would free. Windows' slow cleanup scans
/// reuse a recent answer unless `fresh` is set.
#[tauri::command]
pub async fn analyze_cleanup(ids: Vec<String>, fresh: Option<bool>, on_report: Channel<TargetReport>) -> CmdResult<Vec<TargetReport>> {
    let fresh = fresh.unwrap_or(false);
    let reports = tauri::async_runtime::spawn_blocking(move || {
        let elevated = is_elevated();
        let running = running_processes();
        let targets = find_targets(&ids);
        crate::pool::run_isolated("cleanup", || {
            targets
                .par_iter()
                .map(|t| {
                    let report = analyze_target(t, elevated, fresh, &running);
                    // Stream each result so the list fills in while slow targets finish.
                    let _ = on_report.send(report.clone());
                    report
                })
                .collect::<Vec<_>>()
        })
    })
    .await??;
    Ok(reports)
}

fn analyze_target(t: &Target, elevated: bool, fresh: bool, running: &HashSet<String>) -> TargetReport {
    let mut report = TargetReport::new(t, elevated);
    report.running = t.processes.iter().any(|p| running.contains(&p.to_lowercase()));
    match t.kind {
        Kind::Files => {
            let roots = target_roots(t);
            let cut = cutoff(t);
            let stats = roots.par_iter().map(|p| measure(p, cut)).reduce(DirStats::default, |a, b| a + b);
            (report.bytes, report.files, report.found) = (stats.bytes, stats.files, !roots.is_empty());
        }
        Kind::RecycleBin => {
            let stats = recycle_bin_stats();
            (report.bytes, report.files, report.found) = (stats.bytes, stats.files, true);
        }
        Kind::PreviousWindows => previous_windows::analyze(&mut report, elevated, fresh),
        Kind::UpdateCleanup => {
            // The component store is always there. Only an administrator
            // can ask Windows how much of it can go.
            report.found = true;
            if elevated {
                match handlers::measure(handlers::UPDATE_CLEANUP, true, fresh) {
                    Ok(bytes) => report.bytes = bytes,
                    Err(e) => report.note = Some(format!("{e:#}")),
                }
            }
            if servicing_restart_pending() {
                report.note = Some("Windows has updates waiting for a restart. Restart the PC first, or this may free little.".into());
            }
        }
    }
    report
}

/// Windows Update has installed something that needs a restart to finish.
/// Until then the component store can't be cleaned up fully.
fn servicing_restart_pending() -> bool {
    winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending")
        .is_ok()
}

#[tauri::command]
pub async fn run_cleanup(ids: Vec<String>, on_step: Channel<CleanStep>) -> CmdResult<Vec<CleanReport>> {
    let reports = tauri::async_runtime::spawn_blocking(move || {
        let elevated = is_elevated();
        let mut targets = find_targets(&ids);
        targets.retain(|t| !t.requires_admin || elevated);
        // Files first: they're quick, so their space shows up while Windows
        // works through the slow ones.
        targets.sort_by_key(|t| t.kind.handler().is_some());
        crate::pool::run_isolated("cleanup", || {
            targets
                .into_iter()
                .map(|t| {
                    let _ = on_step.send(CleanStep { id: t.id, report: None, progress: None });
                    let report = clean_target(t, elevated, &on_step);
                    let _ = on_step.send(CleanStep { id: t.id, report: Some(report.clone()), progress: None });
                    report
                })
                .collect::<Vec<_>>()
        })
    })
    .await??;
    Ok(reports)
}

fn clean_target(t: &Target, elevated: bool, on_step: &Channel<CleanStep>) -> CleanReport {
    let mut report = CleanReport::new(t.id);
    match t.kind {
        Kind::Files => {
            let cut = cutoff(t);
            let tally = target_roots(t).iter().map(|root| clean_root(root, cut)).fold(CleanTally::default(), |a, b| a + b);
            (report.freed_bytes, report.deleted_files, report.failed_files) = (tally.freed, tally.deleted, tally.failed);
        }
        Kind::RecycleBin => {
            let before = recycle_bin_stats();
            let flags = SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND;
            let ok = unsafe { SHEmptyRecycleBinW(None, PCWSTR::null(), flags) }.is_ok();
            let after = if ok { DirStats::default() } else { recycle_bin_stats() };
            report.freed_bytes = before.bytes.saturating_sub(after.bytes);
            report.deleted_files = before.files.saturating_sub(after.files);
            report.failed_files = after.files;
        }
        Kind::UpdateCleanup => run_handler(t, elevated, on_step, &mut report),
        Kind::PreviousWindows => {
            run_handler(t, elevated, on_step, &mut report);
            if report.error.is_none() && !previous_windows::folders().is_empty() {
                report.error = Some("Windows removed only part of it. Restart the PC, then clean again to remove the rest.".into());
            }
        }
    }
    report
}

/// Has the target's Windows Disk Cleanup handler free its space, passing on
/// its progress. Handlers don't say what they deleted, so what they freed is
/// read from the drive's free space.
fn run_handler(t: &Target, elevated: bool, on_step: &Channel<CleanStep>, report: &mut CleanReport) {
    let Some(handler) = t.kind.handler() else { return };
    let root = system_root();
    let before = free_space(&root);
    let (channel, id) = (on_step.clone(), t.id);
    let last = AtomicU32::new(u32::MAX);
    let result = handlers::purge(handler, elevated, move |freed, total| {
        if total == 0 {
            return;
        }
        let percent = (freed.min(total) as f64 / total as f64 * 100.0) as u32;
        if last.swap(percent, Ordering::Relaxed) != percent {
            let _ = channel.send(CleanStep { id, report: None, progress: Some(f64::from(percent) / 100.0) });
        }
    });
    report.freed_bytes = free_space(&root).saturating_sub(before);
    report.error = result.err().map(|e| format!("{e:#}"));
}

/// The system drive's root, like `C:\`.
fn system_root() -> PathBuf {
    let windows = paths::known().windows.to_string_lossy().to_string();
    PathBuf::from(format!("{}\\", windows.get(..2).unwrap_or("C:")))
}

fn free_space(root: &Path) -> u64 {
    let mut free = 0u64;
    match unsafe { GetDiskFreeSpaceExW(&HSTRING::from(root.as_os_str()), None, None, Some(&mut free)) } {
        Ok(()) => free,
        Err(_) => 0,
    }
}

/// 1536 -> "1.5 KB", the way the app shows sizes everywhere else.
fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let places = if unit == 0 || value >= 100.0 {
        0
    } else if value >= 10.0 {
        1
    } else {
        2
    };
    format!("{value:.places$} {}", UNITS[unit])
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

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reclaim-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn junction(link: &Path, target: &Path) {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "mklink /J failed");
    }

    #[test]
    fn cleans_contents_but_keeps_root() {
        let dir = scratch("clean");
        fs::create_dir_all(dir.join("nested")).unwrap();
        fs::write(dir.join("a.tmp"), vec![1u8; 10]).unwrap();
        fs::write(dir.join("nested/b.tmp"), vec![1u8; 20]).unwrap();
        assert_eq!(measure(&dir, None).bytes, 30);
        let tally = clean_root(&dir, None);
        assert_eq!((tally.freed, tally.deleted, tally.failed), (30, 2, 0));
        assert!(dir.exists());
        assert!(!dir.join("nested").exists());
        // Files newer than the cutoff are kept.
        fs::write(dir.join("fresh.tmp"), vec![1u8; 5]).unwrap();
        fs::create_dir_all(dir.join("nested")).unwrap();
        fs::write(dir.join("nested/fresh.tmp"), vec![1u8; 5]).unwrap();
        let hour_ago = Some(SystemTime::now() - Duration::from_secs(3600));
        assert_eq!(measure(&dir, hour_ago).files, 0);
        assert_eq!(measure(&dir, None).files, 2);
        assert_eq!(clean_root(&dir, hour_ago).deleted, 0);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn never_follows_junctions() {
        let base = scratch("junction");
        let victim = base.join("victim");
        fs::create_dir_all(victim.join("sub")).unwrap();
        fs::write(victim.join("keep.txt"), b"important").unwrap();
        fs::write(victim.join("sub").join("keep2.txt"), b"important").unwrap();

        // A junction inside the folder being cleaned is skipped, not followed.
        let target = base.join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("junk.tmp"), b"junk").unwrap();
        junction(&target.join("link"), &victim);
        assert_eq!(measure(&target, None).files, 1);
        assert_eq!(clean_root(&target, None).deleted, 1);
        assert!(victim.join("keep.txt").exists());

        // A root that is itself a junction is refused.
        let root_link = base.join("root-link");
        junction(&root_link, &victim);
        assert_eq!(measure(&root_link, None).files, 0);
        assert_eq!(clean_root(&root_link, None).deleted, 0);
        assert!(victim.join("keep.txt").exists());

        // So is a root reached through a junction higher up the path.
        let tally = clean_root(&root_link.join("sub"), None);
        assert_eq!((tally.deleted, tally.failed), (0, 1));
        assert!(victim.join("sub").join("keep2.txt").exists());

        fs::remove_dir(&root_link).unwrap();
        fs::remove_dir(target.join("link")).unwrap();
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn pinned_folders_cannot_be_swapped() {
        let base = scratch("pin");
        let dir = base.join("pinned");
        fs::create_dir_all(&dir).unwrap();
        let pin = pin_dir(&dir).expect("a normal folder can be pinned");
        assert!(fs::rename(&dir, base.join("moved")).is_err(), "rename must fail while pinned");
        assert!(fs::remove_dir(&dir).is_err(), "delete must fail while pinned");
        drop(pin);
        fs::rename(&dir, base.join("moved")).expect("rename works once unpinned");
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn per_user_roots_stay_in_your_profile() {
        let k = paths::known();
        assert!(per_user_root_allowed(&k.local_app_data.join(r"Google\Chrome\User Data\Default\Cache")));
        assert!(per_user_root_allowed(&k.temp));
        assert!(!per_user_root_allowed(&k.windows.join("System32")));
        assert!(!per_user_root_allowed(&k.program_files.join("SomeApp")));
        assert!(!per_user_root_allowed(&k.user_profile));
        assert!(!per_user_root_allowed(&k.user_profile.join("Documents")));
        // The same folders written with 8.3 short names get the same answers.
        if let Some(short_temp) = paths::short_path(&k.temp) {
            assert!(per_user_root_allowed(&short_temp), "{}", short_temp.display());
        }
        if let Some(short_pf) = paths::short_path(&k.program_files) {
            assert!(!per_user_root_allowed(&short_pf.join("SomeApp")));
        }
    }

    #[test]
    fn machine_variables_ignore_the_environment() {
        let k = paths::known();
        assert_eq!(trusted_var("SystemRoot").map(PathBuf::from), Some(k.windows.clone()));
        assert_eq!(trusted_var("ProgramData").map(PathBuf::from), Some(k.program_data.clone()));
        assert_eq!(trusted_var("ProgramFiles(x86)").map(PathBuf::from), Some(k.program_files_x86.clone()));
    }
}
