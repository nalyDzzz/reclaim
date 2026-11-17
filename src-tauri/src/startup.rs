//! What starts with Windows: the Run keys and Startup folders that Task
//! Manager's Startup apps page lists, plus what it leaves out: scheduled
//! tasks that run at sign-in or boot, and services from installed apps that
//! start automatically.
//!
//! Turning something off never deletes it. Run keys and Startup folders
//! use the same StartupApproved switch as Task Manager, so the two agree.
//! Tasks are disabled. Services are set to start manually rather than
//! disabled, so an app that needs its service can still start it.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use base64::Engine;
use serde::Serialize;
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, VARIANT_FALSE, VARIANT_TRUE};
use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Services::{
    ChangeServiceConfigW, CloseServiceHandle, ENUM_SERVICE_TYPE, OpenSCManagerW, OpenServiceW, SC_HANDLE, SC_MANAGER_CONNECT,
    SERVICE_AUTO_START, SERVICE_CHANGE_CONFIG, SERVICE_DEMAND_START, SERVICE_ERROR, SERVICE_NO_CHANGE,
};
use windows::Win32::System::TaskScheduler::{
    IExecAction, IRegisteredTask, ITaskDefinition, ITaskFolder, TASK_ENUM_HIDDEN, TASK_RUNLEVEL_HIGHEST, TASK_RUNLEVEL_TYPE,
    TASK_TRIGGER_BOOT, TASK_TRIGGER_LOGON, TASK_TRIGGER_TYPE2,
};
use windows::Win32::System::Variant::VARIANT;
use windows::core::{BSTR, HSTRING, Interface, PCWSTR, w};
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY, REG_BINARY};
use winreg::{RegKey, RegValue};

use crate::error::CmdResult;
use crate::paths::{self, expand_env, is_under, known, normalize};
use crate::programs::leftovers::{shortcut_target, task_service};
use crate::programs::split_command;
use crate::remembered::{self, Remembered};
use crate::system::is_elevated;

const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";
const SERVICES: &str = r"SYSTEM\CurrentControlSet\Services";
/// Windows' own tasks, which this never lists or changes.
const WINDOWS_TASKS: &str = r"\Microsoft\Windows";

#[derive(Serialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    Registry,
    Folder,
    Task,
    Service,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StartupItem {
    /// Says where the entry is, for `set_startup_item`.
    id: String,
    name: String,
    /// The entry's own name: the registry value, file, task, or service.
    entry: String,
    publisher: Option<String>,
    command: String,
    /// The program it starts, when that's a file Reclaim could find.
    program: Option<String>,
    source: Source,
    /// Starts for everyone who signs in to this PC, not just this user.
    all_users: bool,
    enabled: bool,
    requires_admin: bool,
    blocked: bool,
    /// The program it would start is gone, usually left by an uninstall.
    missing: bool,
    /// A service that waits until shortly after startup.
    delayed: bool,
}

// --- Programs and their names ---

/// What `GetDriveTypeW` says for a mapped network drive.
const DRIVE_REMOTE: u32 = 4;

/// A path on one of this PC's own drives. Reading anything else, like
/// `\\server\share\app.exe` or a mapped network drive, has Windows connect
/// to that server and sign in to it as the user, and can hang the list
/// while an unreachable share times out.
fn is_local(path: &Path) -> bool {
    let text = path.to_string_lossy();
    let bytes = text.as_bytes();
    let drive_path = bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/');
    drive_path && unsafe { GetDriveTypeW(&HSTRING::from(&text[..3])) } != DRIVE_REMOTE
}

/// The program a command starts, when it's on this PC. A bare name like
/// `rundll32.exe` is looked for in System32, which is where Windows finds it.
fn program_of(command: &str) -> Option<PathBuf> {
    let (exe, _) = split_command(&expand_env(command));
    if exe.is_empty() {
        return None;
    }
    let path = PathBuf::from(&exe);
    if path.is_absolute() {
        return is_local(&path).then_some(path);
    }
    let system = paths::system32(&exe);
    system.exists().then_some(system)
}

#[derive(Default)]
struct FileInfo {
    description: Option<String>,
    company: Option<String>,
}

/// The description and company an executable reports about itself, as
/// shown on the Details tab of its properties.
fn file_info(path: &Path) -> FileInfo {
    let file = HSTRING::from(path.as_os_str());
    unsafe {
        let size = GetFileVersionInfoSizeW(&file, None);
        if size == 0 {
            return FileInfo::default();
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(&file, None, size, data.as_mut_ptr().cast()).is_err() {
            return FileInfo::default();
        }
        let block = data.as_ptr().cast();
        let mut languages = Vec::new();
        let (mut ptr, mut len) = (std::ptr::null_mut(), 0u32);
        if VerQueryValueW(block, w!(r"\VarFileInfo\Translation"), &mut ptr, &mut len).as_bool() && len >= 4 {
            let pairs = std::slice::from_raw_parts(ptr as *const u16, len as usize / 2);
            languages.extend(pairs.chunks_exact(2).map(|p| (p[0], p[1])));
        }
        // Common ones, for files whose translation table is wrong.
        languages.extend([(0x0409, 0x04B0), (0x0409, 0x04E4), (0x0000, 0x04B0)]);
        let field = |name: &str| {
            languages.iter().find_map(|(lang, codepage)| {
                let sub = HSTRING::from(format!(r"\StringFileInfo\{lang:04x}{codepage:04x}\{name}"));
                let (mut ptr, mut len) = (std::ptr::null_mut(), 0u32);
                if !VerQueryValueW(block, &sub, &mut ptr, &mut len).as_bool() || len == 0 {
                    return None;
                }
                let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr as *const u16, len as usize));
                // Some files report a length that runs past the end of the text.
                let text = text.split('\0').next().unwrap_or_default().trim();
                (!text.is_empty()).then(|| text.to_string())
            })
        };
        FileInfo { description: field("FileDescription"), company: field("CompanyName") }
    }
}

/// Programs that start something else, whose own description says nothing
/// about what's starting.
const LAUNCHERS: &[&str] = &[
    "update.exe",
    "squirrel.exe",
    "rundll32.exe",
    "regsvr32.exe",
    "cmd.exe",
    "conhost.exe",
    "powershell.exe",
    "pwsh.exe",
    "wscript.exe",
    "cscript.exe",
    "mshta.exe",
    "java.exe",
    "javaw.exe",
    "explorer.exe",
];

fn is_launcher(program: &Path) -> bool {
    program.file_name().is_some_and(|n| LAUNCHERS.iter().any(|l| n.eq_ignore_ascii_case(l)))
}

/// `MicrosoftEdgeAutoLaunch_E098B21127C4BD7A4333C0E779B643B9` -> `MicrosoftEdgeAutoLaunch`,
/// `OneDrive Reporting Task-S-1-5-21-1-2-3-1001` -> `OneDrive Reporting Task`,
/// `NvProfileUpdaterOnLogon_{B2FE1952-0186-46C3-BAEC-A80AA35AC5B8}` -> `NvProfileUpdaterOnLogon`.
fn tidy_entry_name(name: &str) -> &str {
    let name = match name.find("-S-1-5-") {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    };
    let name = match name.rfind('{') {
        Some(i) if i > 0 && name.ends_with('}') => name[..i].trim_end_matches(['_', '-', ' ']),
        _ => name,
    };
    match name.rsplit_once('_') {
        Some((head, tail)) if !head.is_empty() && tail.len() >= 16 && tail.chars().all(|c| c.is_ascii_hexdigit()) => head,
        _ => name,
    }
}

struct Described {
    name: String,
    publisher: Option<String>,
    program: Option<PathBuf>,
    missing: bool,
}

/// Names an entry the way Task Manager does, by the description of the
/// program it starts, unless that program only launches something else.
fn describe(command: &str, entry: &str) -> Described {
    let program = program_of(command);
    let missing = program.as_ref().is_some_and(|p| !p.exists());
    let info = program.as_deref().filter(|p| !missing && !is_launcher(p)).map(file_info).unwrap_or_default();
    let fallback = tidy_entry_name(entry).trim();
    Described {
        name: info.description.unwrap_or_else(|| if fallback.is_empty() { entry.to_string() } else { fallback.to_string() }),
        publisher: info.company,
        program,
        missing,
    }
}

// --- Run keys and Startup folders ---

struct RunKey {
    id: &'static str,
    all_users: bool,
    path: &'static str,
    /// Its switch under StartupApproved.
    approved: &'static str,
}

const RUN_KEYS: &[RunKey] = &[
    RunKey { id: "user", all_users: false, path: r"Software\Microsoft\Windows\CurrentVersion\Run", approved: "Run" },
    RunKey { id: "machine", all_users: true, path: r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run", approved: "Run" },
    RunKey { id: "machine32", all_users: true, path: r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run", approved: "Run32" },
];

fn hive(all_users: bool) -> RegKey {
    RegKey::predef(if all_users { HKEY_LOCAL_MACHINE } else { HKEY_CURRENT_USER })
}

/// Task Manager's on/off switch: the first byte is even when it's on, and a
/// missing value means on.
fn approved(all_users: bool, list: &str, name: &str) -> bool {
    hive(all_users)
        .open_subkey_with_flags(format!(r"{APPROVED}\{list}"), KEY_READ | KEY_WOW64_64KEY)
        .and_then(|k| k.get_raw_value(name))
        .map_or(true, |v| v.bytes.first().is_none_or(|b| b & 1 == 0))
}

/// The value Task Manager writes: 2 for on; 3 for off, followed by when.
fn approved_value(enabled: bool, now: SystemTime) -> Vec<u8> {
    let mut bytes = vec![0u8; 12];
    if enabled {
        bytes[0] = 2;
    } else {
        bytes[0] = 3;
        // FILETIME: 100 ns ticks since 1601.
        let ticks = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64 / 100) + 116_444_736_000_000_000;
        bytes[4..].copy_from_slice(&ticks.to_le_bytes());
    }
    bytes
}

fn set_approved(all_users: bool, list: &str, name: &str, enabled: bool) -> anyhow::Result<()> {
    let (key, _) =
        hive(all_users).create_subkey_with_flags(format!(r"{APPROVED}\{list}"), KEY_SET_VALUE | KEY_WOW64_64KEY).map_err(denied)?;
    let value = RegValue { bytes: approved_value(enabled, SystemTime::now()).into(), vtype: REG_BINARY };
    key.set_raw_value(name, &value).map_err(denied)?;
    Ok(())
}

fn denied(e: std::io::Error) -> anyhow::Error {
    if e.kind() == std::io::ErrorKind::PermissionDenied { crate::system::refusal() } else { e.into() }
}

fn run_key_items(elevated: bool) -> Vec<StartupItem> {
    let mut items = Vec::new();
    for run in RUN_KEYS {
        let Ok(key) = hive(run.all_users).open_subkey_with_flags(run.path, KEY_READ | KEY_WOW64_64KEY) else { continue };
        for (name, _) in key.enum_values().flatten() {
            let Ok(command) = key.get_value::<String, _>(&name) else { continue };
            if name.is_empty() || command.trim().is_empty() {
                continue;
            }
            let about = describe(&command, &name);
            items.push(StartupItem {
                id: format!("run|{}|{name}", run.id),
                name: about.name,
                entry: name.clone(),
                publisher: about.publisher,
                program: about.program.map(|p| p.to_string_lossy().to_string()),
                source: Source::Registry,
                all_users: run.all_users,
                enabled: approved(run.all_users, run.approved, &name),
                requires_admin: run.all_users,
                blocked: run.all_users && !elevated,
                missing: about.missing,
                delayed: false,
                command,
            });
        }
    }
    items
}

fn startup_folder(all_users: bool) -> PathBuf {
    let k = known();
    if all_users {
        k.program_data.join(r"Microsoft\Windows\Start Menu\Programs\StartUp")
    } else {
        k.app_data.join(r"Microsoft\Windows\Start Menu\Programs\Startup")
    }
}

fn folder_items(elevated: bool) -> Vec<StartupItem> {
    let mut items = Vec::new();
    for all_users in [false, true] {
        let Ok(entries) = std::fs::read_dir(startup_folder(all_users)) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let file = entry.file_name().to_string_lossy().to_string();
            if !entry.file_type().is_ok_and(|t| t.is_file()) || file.eq_ignore_ascii_case("desktop.ini") {
                continue;
            }
            let command = shortcut_target(&path).unwrap_or_else(|| path.to_string_lossy().to_string());
            let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| file.clone());
            let about = describe(&command, &stem);
            items.push(StartupItem {
                id: format!("folder|{}|{file}", if all_users { "common" } else { "user" }),
                // A shortcut's own name is what its creator chose to show.
                name: stem,
                entry: file.clone(),
                publisher: about.publisher,
                program: about.program.map(|p| p.to_string_lossy().to_string()),
                source: Source::Folder,
                all_users,
                enabled: approved(all_users, "StartupFolder", &file),
                requires_admin: all_users,
                blocked: all_users && !elevated,
                missing: about.missing,
                delayed: false,
                command,
            });
        }
    }
    items
}

// --- Scheduled tasks ---

fn starts_with_windows(definition: &ITaskDefinition) -> bool {
    unsafe {
        let Ok(triggers) = definition.Triggers() else { return false };
        let mut count = 0;
        if triggers.Count(&mut count).is_err() {
            return false;
        }
        (1..=count).any(|i| {
            let mut kind = TASK_TRIGGER_TYPE2::default();
            triggers.get_Item(i).and_then(|t| t.Type(&mut kind)).is_ok() && (kind == TASK_TRIGGER_LOGON || kind == TASK_TRIGGER_BOOT)
        })
    }
}

/// The program the task's first action starts, with its arguments.
fn task_command(definition: &ITaskDefinition) -> Option<String> {
    unsafe {
        let actions = definition.Actions().ok()?;
        let mut count = 0;
        actions.Count(&mut count).ok()?;
        (1..=count).find_map(|i| {
            let exec = actions.get_Item(i).ok()?.cast::<IExecAction>().ok()?;
            let (mut path, mut args) = (BSTR::new(), BSTR::new());
            exec.Path(&mut path).ok()?;
            let _ = exec.Arguments(&mut args);
            let path = path.to_string();
            let quoted = if path.contains(' ') && !path.starts_with('"') { format!("\"{path}\"") } else { path };
            Some(format!("{quoted} {args}").trim().to_string())
        })
    }
}

/// Tasks set up by an administrator: ones that run as a service account,
/// for a group of users, or with full rights. Only an administrator can
/// change those.
fn task_needs_admin(definition: &ITaskDefinition) -> bool {
    const SERVICE_ACCOUNTS: &[&str] = &[
        "system",
        "localsystem",
        "s-1-5-18",
        "local service",
        "localservice",
        "s-1-5-19",
        "network service",
        "networkservice",
        "s-1-5-20",
    ];
    unsafe {
        let Ok(principal) = definition.Principal() else { return true };
        let (mut user, mut group, mut level) = (BSTR::new(), BSTR::new(), TASK_RUNLEVEL_TYPE::default());
        let _ = principal.UserId(&mut user);
        let _ = principal.GroupId(&mut group);
        let _ = principal.RunLevel(&mut level);
        let user = user.to_string().to_lowercase();
        let account = user.rsplit('\\').next().unwrap_or_default();
        !group.is_empty() || level == TASK_RUNLEVEL_HIGHEST || SERVICE_ACCOUNTS.contains(&account)
    }
}

fn task_item(task: &IRegisteredTask, elevated: bool) -> Option<StartupItem> {
    unsafe {
        let definition = task.Definition().ok()?;
        if !starts_with_windows(&definition) {
            return None;
        }
        let command = task_command(&definition)?;
        let path = task.Path().ok()?.to_string();
        let entry = task.Name().ok()?.to_string();
        let about = describe(&command, &entry);
        let requires_admin = task_needs_admin(&definition);
        Some(StartupItem {
            id: format!("task|{path}"),
            // Tasks are named for what they do, and one program often has several.
            name: tidy_entry_name(&entry).to_string(),
            entry,
            publisher: about.publisher,
            program: about.program.map(|p| p.to_string_lossy().to_string()),
            source: Source::Task,
            all_users: requires_admin,
            enabled: task.Enabled().is_ok_and(|e| e.as_bool()),
            requires_admin,
            blocked: requires_admin && !elevated,
            missing: about.missing,
            delayed: false,
            command,
        })
    }
}

fn is_windows_task(path: &str) -> bool {
    let path = path.to_lowercase();
    let windows = WINDOWS_TASKS.to_lowercase();
    path == windows || path.starts_with(&format!(r"{windows}\"))
}

fn task_items(elevated: bool) -> Vec<StartupItem> {
    let Ok(service) = task_service() else { return Vec::new() };
    let mut items = Vec::new();
    let Ok(root) = (unsafe { service.GetFolder(&BSTR::from("\\")) }) else { return items };
    let mut folders: Vec<ITaskFolder> = vec![root];
    while let Some(folder) = folders.pop() {
        unsafe {
            if folder.Path().is_ok_and(|p| is_windows_task(&p.to_string())) {
                continue;
            }
            if let Ok(children) = folder.GetFolders(0) {
                for i in 1..=children.Count().unwrap_or(0) {
                    if let Ok(child) = children.get_Item(&VARIANT::from(i)) {
                        folders.push(child);
                    }
                }
            }
            let Ok(tasks) = folder.GetTasks(TASK_ENUM_HIDDEN.0) else { continue };
            for i in 1..=tasks.Count().unwrap_or(0) {
                if let Some(item) = tasks.get_Item(&VARIANT::from(i)).ok().and_then(|t| task_item(&t, elevated)) {
                    items.push(item);
                }
            }
        }
    }
    items
}

fn set_task(path: &str, enabled: bool) -> anyhow::Result<()> {
    crate::error::ensure!(!is_windows_task(path), "This is one of Windows' own tasks");
    let (folder, name) = path.rsplit_once('\\').context("Invalid task path")?;
    let folder = if folder.is_empty() { "\\" } else { folder };
    let result = unsafe {
        task_service()
            .and_then(|s| s.GetFolder(&BSTR::from(folder)))
            .and_then(|f| f.GetTask(&BSTR::from(name)))
            .and_then(|t| t.SetEnabled(if enabled { VARIANT_TRUE } else { VARIANT_FALSE }))
    };
    match result {
        Err(e) if e.code() == ERROR_ACCESS_DENIED.to_hresult() => Err(crate::system::refusal()),
        result => Ok(result?),
    }
}

// --- Services ---

fn hklm(sub: &str) -> std::io::Result<RegKey> {
    RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(sub, KEY_READ | KEY_WOW64_64KEY)
}

/// A service of its own process, not a driver or a per-user copy.
fn is_plain_service(key: &RegKey) -> bool {
    const WIN32: u32 = 0x30;
    const DRIVER: u32 = 0x0B;
    const USER_SERVICE: u32 = 0x40;
    key.get_value::<u32, _>("Type").is_ok_and(|t| t & WIN32 != 0 && t & (DRIVER | USER_SERVICE) == 0)
}

/// Services from installed apps. Windows' own, including Defender's, which
/// lives outside the Windows folder, stay off the list.
fn from_an_app(program: &Path) -> bool {
    let k = known();
    let windows_own = [
        k.windows.clone(),
        k.program_data.join(r"Microsoft\Windows Defender"),
        k.program_files.join("Windows Defender"),
        k.program_files.join("Windows Defender Advanced Threat Protection"),
        k.program_files.join("Microsoft Security Client"),
    ];
    let program = normalize(program);
    program.contains('\\') && !windows_own.iter().any(|root| is_under(&program, &normalize(root)))
}

fn service_program(key: &RegKey) -> Option<PathBuf> {
    key.get_value::<String, _>("ImagePath").ok().and_then(|image| program_of(&image))
}

fn service_items(record: &Remembered, elevated: bool) -> Vec<StartupItem> {
    const AUTO: u32 = 2;
    const MANUAL: u32 = 3;
    let Ok(root) = hklm(SERVICES) else { return Vec::new() };
    root.enum_keys()
        .flatten()
        .filter_map(|name| {
            let key = root.open_subkey_with_flags(&name, KEY_READ).ok()?;
            if !is_plain_service(&key) {
                return None;
            }
            let start: u32 = key.get_value("Start").ok()?;
            // Manual ones only when Reclaim set them that way, so they can be switched back.
            let switched_off = start == MANUAL && record.manual_services.contains(&name);
            if start != AUTO && !switched_off {
                return None;
            }
            let program = service_program(&key)?;
            if !from_an_app(&program) {
                return None;
            }
            let display: String = key.get_value("DisplayName").unwrap_or_default();
            let display = if display.is_empty() || display.starts_with('@') { name.clone() } else { display };
            let missing = !program.exists();
            let info = if missing { FileInfo::default() } else { file_info(&program) };
            Some(StartupItem {
                id: format!("service|{name}"),
                name: display,
                publisher: info.company,
                command: key.get_value("ImagePath").unwrap_or_default(),
                program: Some(program.to_string_lossy().to_string()),
                source: Source::Service,
                all_users: true,
                enabled: start == AUTO,
                requires_admin: true,
                blocked: !elevated,
                missing,
                delayed: key.get_value::<u32, _>("DelayedAutostart").is_ok_and(|d| d == 1),
                entry: name,
            })
        })
        .collect()
}

/// Closes a service handle when dropped.
struct ServiceHandle(SC_HANDLE);

impl Drop for ServiceHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseServiceHandle(self.0);
        }
    }
}

fn set_service(name: &str, enabled: bool) -> anyhow::Result<()> {
    let key = hklm(&format!(r"{SERVICES}\{name}")).context("That service isn't installed")?;
    let from_app = is_plain_service(&key) && service_program(&key).is_some_and(|p| from_an_app(&p));
    crate::error::ensure!(from_app, "This is one of Windows' own services");
    unsafe {
        let scm = ServiceHandle(OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT)?);
        let service = match OpenServiceW(scm.0, &HSTRING::from(name), SERVICE_CHANGE_CONFIG) {
            Ok(handle) => ServiceHandle(handle),
            Err(e) if e.code() == ERROR_ACCESS_DENIED.to_hresult() => return Err(crate::system::refusal()),
            Err(e) => return Err(e.into()),
        };
        ChangeServiceConfigW(
            service.0,
            ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE),
            if enabled { SERVICE_AUTO_START } else { SERVICE_DEMAND_START },
            SERVICE_ERROR(SERVICE_NO_CHANGE),
            PCWSTR::null(),
            PCWSTR::null(),
            None,
            PCWSTR::null(),
            PCWSTR::null(),
            PCWSTR::null(),
            PCWSTR::null(),
        )?;
    }
    let _ = remembered::update(|r| {
        if enabled {
            r.manual_services.remove(name);
        } else {
            r.manual_services.insert(name.to_string());
        }
    });
    Ok(())
}

// --- Commands ---

fn read_all() -> Vec<StartupItem> {
    let elevated = is_elevated();
    let record = remembered::load();
    let mut items = run_key_items(elevated);
    items.extend(folder_items(elevated));
    items.extend(task_items(elevated));
    items.extend(service_items(&record, elevated));
    items.sort_by(|a, b| a.source.cmp(&b.source).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    items
}

#[tauri::command]
pub async fn list_startup() -> CmdResult<Vec<StartupItem>> {
    Ok(tauri::async_runtime::spawn_blocking(read_all).await?)
}

/// Turns one entry on or off. The entry is found again from its ID rather
/// than trusting anything else the UI sends.
fn set(id: &str, enabled: bool) -> anyhow::Result<()> {
    let mut parts = id.splitn(3, '|');
    match (parts.next(), parts.next(), parts.next()) {
        (Some("run"), Some(key), Some(name)) => {
            let run = RUN_KEYS.iter().find(|r| r.id == key).context("Unknown startup entry")?;
            let exists =
                hive(run.all_users).open_subkey_with_flags(run.path, KEY_READ | KEY_WOW64_64KEY).and_then(|k| k.get_raw_value(name));
            crate::error::ensure!(exists.is_ok(), "That startup entry is gone");
            set_approved(run.all_users, run.approved, name, enabled)
        }
        (Some("folder"), Some(scope @ ("user" | "common")), Some(file)) => {
            let all_users = scope == "common";
            let is_plain_name = !file.contains(['\\', '/']) && file != ".." && file != ".";
            crate::error::ensure!(is_plain_name && startup_folder(all_users).join(file).is_file(), "That startup entry is gone");
            set_approved(all_users, "StartupFolder", file, enabled)
        }
        (Some("task"), Some(path), None) => set_task(path, enabled),
        (Some("service"), Some(name), None) => set_service(name, enabled),
        _ => anyhow::bail!("Unknown startup entry"),
    }
}

#[tauri::command]
pub async fn set_startup_item(id: String, enabled: bool) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || set(&id, enabled)).await??;
    Ok(())
}

/// The icon of a program, as a PNG data URL.
#[tauri::command]
pub async fn file_icon(path: String) -> CmdResult<Option<String>> {
    let icon = tauri::async_runtime::spawn_blocking(move || {
        if !is_local(Path::new(&path)) {
            return None;
        }
        crate::programs::icon::extract_png(&path, 0)
            .map(|png| format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)))
    })
    .await?;
    Ok(icon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidies_generated_entry_names() {
        assert_eq!(tidy_entry_name("MicrosoftEdgeAutoLaunch_E098B21127C4BD7A4333C0E779B643B9"), "MicrosoftEdgeAutoLaunch");
        assert_eq!(tidy_entry_name("OneDrive Reporting Task-S-1-5-21-111-222-333-1001"), "OneDrive Reporting Task");
        assert_eq!(tidy_entry_name("NvProfileUpdaterOnLogon_{B2FE1952-0186-46C3-BAEC-A80AA35AC5B8}"), "NvProfileUpdaterOnLogon");
        assert_eq!(tidy_entry_name("{B2FE1952-0186-46C3-BAEC-A80AA35AC5B8}"), "{B2FE1952-0186-46C3-BAEC-A80AA35AC5B8}");
        assert_eq!(tidy_entry_name("Adobe_Updater"), "Adobe_Updater");
        assert_eq!(tidy_entry_name("Steam"), "Steam");
    }

    #[test]
    fn writes_task_managers_switch() {
        let on = approved_value(true, SystemTime::now());
        assert_eq!(on, [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let off = approved_value(false, UNIX_EPOCH + std::time::Duration::from_secs(1));
        assert_eq!(off[0], 3);
        assert_eq!(u64::from_le_bytes(off[4..].try_into().unwrap()), 116_444_736_000_000_000 + 10_000_000);
    }

    #[test]
    fn never_reaches_out_to_network_paths() {
        assert!(!is_local(Path::new(r"\\attacker\share\app.exe")));
        assert!(!is_local(Path::new("//attacker/share/app.exe")));
        assert!(!is_local(Path::new(r"\\?\UNC\attacker\share\app.exe")));
        assert!(!is_local(Path::new(r"\\.\pipe\x")));
        assert!(!is_local(Path::new("relative.exe")));
        assert!(is_local(&paths::system32("notepad.exe")));
        assert_eq!(program_of(r#""\\attacker\share\app.exe" /start"#), None);
    }

    #[test]
    fn finds_programs_in_commands() {
        let notepad = paths::system32("notepad.exe");
        assert_eq!(program_of(&format!("\"{}\" /a", notepad.display())), Some(notepad.clone()));
        assert_eq!(program_of("notepad.exe /a"), Some(notepad));
        assert_eq!(program_of(""), None);
    }

    #[test]
    fn reads_what_a_program_says_about_itself() {
        let info = file_info(&paths::system32("notepad.exe"));
        assert!(info.company.is_some_and(|c| c.contains("Microsoft")));
    }

    #[test]
    fn leaves_windows_services_alone() {
        let k = known();
        assert!(!from_an_app(&paths::system32("svchost.exe")));
        assert!(!from_an_app(&k.program_data.join(r"Microsoft\Windows Defender\Platform\4.18\MsMpEng.exe")));
        assert!(from_an_app(&k.program_files.join(r"Steam\steamservice.exe")));
        assert!(is_windows_task(r"\Microsoft\Windows\Defrag\ScheduledDefrag"));
        assert!(is_windows_task(r"\Microsoft\Windows"));
        assert!(!is_windows_task(r"\Microsoft\WindowsTools"));
        assert!(!is_windows_task(r"\Microsoft\Office\OfficeTelemetryAgentLogOn"));
    }

    #[test]
    fn refuses_entries_it_cannot_place() {
        assert!(set("run|nowhere|x", false).is_err());
        assert!(set(r"folder|user|..\..\evil.lnk", false).is_err());
        assert!(set(r"task|\Microsoft\Windows\Defrag\ScheduledDefrag", false).is_err());
        assert!(set("service|WinDefend", false).is_err());
        assert!(set("nonsense", false).is_err());
    }

    /// Removes the test's Run entry and its switch even if an assertion fails.
    struct RemoveRunEntry(String);

    impl Drop for RemoveRunEntry {
        fn drop(&mut self) {
            let user = RegKey::predef(HKEY_CURRENT_USER);
            let _ = user.open_subkey_with_flags(RUN_KEYS[0].path, KEY_SET_VALUE).and_then(|k| k.delete_value(&self.0));
            let _ = user.open_subkey_with_flags(format!(r"{APPROVED}\Run"), KEY_SET_VALUE).and_then(|k| k.delete_value(&self.0));
        }
    }

    #[test]
    fn turns_a_run_entry_off_and_on_like_task_manager() {
        let name = format!("ReclaimStartupTest{}", std::process::id());
        let _cleanup = RemoveRunEntry(name.clone());
        // Points nowhere, so nothing would start even if the cleanup never ran.
        let (run, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEYS[0].path).unwrap();
        run.set_value(&name, &r#""C:\Reclaim test\missing.exe" --flag"#.to_string()).unwrap();

        let find = || read_all().into_iter().find(|i| i.entry == name).expect("the test entry is listed");
        let item = find();
        assert!(item.enabled && item.missing && !item.all_users && item.source == Source::Registry);
        set(&item.id, false).unwrap();
        assert!(!find().enabled);
        set(&item.id, true).unwrap();
        assert!(find().enabled);
    }

    #[test]
    fn lists_without_failing() {
        let items = read_all();
        for item in &items {
            assert!(!item.name.is_empty(), "{item:?}");
        }
    }
}
