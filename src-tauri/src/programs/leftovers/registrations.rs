//! What an app registers with Windows outside its own folders and settings
//! keys: services, scheduled tasks, and firewall rules. Each counts as the
//! app's only when the program it runs is inside one of the app's folders.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::Context;
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SERVICE_DOES_NOT_EXIST, ERROR_SERVICE_MARKED_FOR_DELETE,
};
use windows::Win32::Storage::FileSystem::DELETE;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::System::Services::{
    CloseServiceHandle, ControlService, DeleteService, OpenSCManagerW, OpenServiceW, QueryServiceStatus, SC_HANDLE, SC_MANAGER_CONNECT,
    SERVICE_CONTROL_STOP, SERVICE_DRIVER, SERVICE_QUERY_STATUS, SERVICE_STATUS, SERVICE_STOP, SERVICE_STOPPED, SERVICE_WIN32,
};
use windows::Win32::System::TaskScheduler::{IExecAction, IRegisteredTask, ITaskService, TASK_ENUM_HIDDEN, TaskScheduler};
use windows::Win32::System::Variant::VARIANT;
use windows::core::{BSTR, HSTRING, Interface, PCWSTR};
use winreg::RegKey;
use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};

use super::{CREATE_NO_WINDOW, Confidence, Leftover, LeftoverKind, Refused, backup_file, backup_key, points_into, write_utf16};
use crate::paths::{self, expand_env};
use crate::programs::split_command;

const SERVICES: &str = r"SYSTEM\CurrentControlSet\Services";
const FIREWALL_RULES: &str = r"SYSTEM\CurrentControlSet\Services\SharedAccess\Parameters\FirewallPolicy\FirewallRules";
/// Windows' own tasks. Never touched, and skipping them keeps the scan quick.
const WINDOWS_TASKS: &str = r"\Microsoft\Windows";

pub(super) struct Found {
    pub kind: LeftoverKind,
    pub id: String,
    pub label: Option<String>,
    pub confidence: Confidence,
    pub reason: &'static str,
}

/// Services, tasks, and firewall rules that run a program inside `folders`.
pub(super) fn find(folders: &[(String, Confidence)]) -> Vec<Found> {
    if folders.is_empty() {
        return Vec::new();
    }
    let runs_from = |command: &str| points_into(&expand_env(command), folders);
    let mut found = services(&runs_from);
    found.extend(tasks(&runs_from).unwrap_or_default());
    found.extend(firewall_rules(&runs_from));
    found
}

pub(super) fn exists(item: &Leftover) -> bool {
    match item.kind {
        LeftoverKind::Service => service_exists(&item.path),
        LeftoverKind::ScheduledTask => task_exists(&item.path),
        _ => read_rule(&item.path).is_some(),
    }
}

pub(super) fn backup(item: &Leftover, dir: &Path) -> anyhow::Result<()> {
    match item.kind {
        LeftoverKind::Service => backup_key(dir, "HKLM", &format!(r"{SERVICES}\{}", item.path)),
        LeftoverKind::ScheduledTask => {
            let xml = unsafe { get_task(&item.path)?.Xml()? };
            write_utf16(&backup_file(dir, "task", &item.path, "xml")?, &xml.to_string())?;
            Ok(())
        }
        _ => {
            // Just the one rule: exporting the whole key would copy hundreds.
            let rule = read_rule(&item.path).context("The firewall rule is gone")?;
            let escape = |s: &str| s.replace('\\', r"\\").replace('"', "\\\"");
            let reg = format!(
                "Windows Registry Editor Version 5.00\r\n\r\n[HKEY_LOCAL_MACHINE\\{FIREWALL_RULES}]\r\n\"{}\"=\"{}\"\r\n",
                escape(&item.path),
                escape(&rule)
            );
            write_utf16(&backup_file(dir, "HKLM", &format!(r"{FIREWALL_RULES}\{}", item.path), "reg")?, &reg)?;
            Ok(())
        }
    }
}

/// Removes the item, after checking it against the same rules the scan
/// uses, so nothing else can be passed off as a leftover.
pub(super) fn remove(item: &Leftover) -> anyhow::Result<()> {
    match item.kind {
        LeftoverKind::Service => remove_service(&item.path),
        LeftoverKind::ScheduledTask => remove_task(&item.path),
        _ => remove_firewall_rule(&item.path),
    }
}

fn hklm(sub: &str) -> std::io::Result<RegKey> {
    RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(sub, KEY_READ | KEY_WOW64_64KEY)
}

// --- Services ---

/// What a service runs: its own program, or the DLL svchost loads for it.
fn service_programs(key: &RegKey) -> Vec<String> {
    let image: String = key.get_value("ImagePath").unwrap_or_default();
    let dll: String = key.open_subkey("Parameters").and_then(|p| p.get_value("ServiceDll")).unwrap_or_default();
    [image, dll].into_iter().filter(|s| !s.is_empty()).collect()
}

/// Services only, never drivers: removing a driver a device class lists as a
/// filter can stop that device, the keyboard for one, from working.
fn is_service(key: &RegKey) -> bool {
    key.get_value::<u32, _>("Type").is_ok_and(|t| t & SERVICE_WIN32.0 != 0 && t & SERVICE_DRIVER.0 == 0)
}

fn services(runs_from: &dyn Fn(&str) -> Option<Confidence>) -> Vec<Found> {
    let Ok(root) = hklm(SERVICES) else { return Vec::new() };
    root.enum_keys()
        .flatten()
        .filter_map(|name| {
            let key = root.open_subkey_with_flags(&name, KEY_READ).ok()?;
            if !is_service(&key) {
                return None;
            }
            let confidence = service_programs(&key).iter().filter_map(|p| runs_from(p)).max()?;
            let display: String = key.get_value("DisplayName").unwrap_or_default();
            Some(Found {
                kind: LeftoverKind::Service,
                id: name,
                label: (!display.is_empty() && !display.starts_with('@')).then_some(display),
                confidence,
                reason: "Windows service that runs from the app's folder",
            })
        })
        .collect()
}

fn service_exists(name: &str) -> bool {
    match hklm(&format!(r"{SERVICES}\{name}")) {
        // Windows flags a service it will remove as soon as it's stopped.
        Ok(key) => key.get_value::<u32, _>("DeleteFlag").ok().is_none_or(|flag| flag == 0),
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
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

fn remove_service(name: &str) -> anyhow::Result<()> {
    let key = match hklm(&format!(r"{SERVICES}\{name}")) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        key => key?,
    };
    crate::error::ensure!(is_service(&key), "Drivers are never removed");
    let runs_outside_windows = service_programs(&key).iter().any(|p| !paths::is_protected(Path::new(&split_command(&expand_env(p)).0)));
    crate::error::ensure!(runs_outside_windows, "This is one of Windows' own services");
    unsafe {
        let scm = ServiceHandle(OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT)?);
        let service = match OpenServiceW(scm.0, &HSTRING::from(name), SERVICE_STOP | SERVICE_QUERY_STATUS | DELETE.0) {
            Ok(handle) => ServiceHandle(handle),
            Err(e) if e.code() == ERROR_SERVICE_DOES_NOT_EXIST.to_hresult() => return Ok(()),
            Err(e) if e.code() == ERROR_ACCESS_DENIED.to_hresult() => {
                return Err(Refused::Denied("Removing a service needs administrator rights.").into());
            }
            Err(e) => return Err(e.into()),
        };
        // Stopped first, it goes now rather than at the next restart.
        let mut status = SERVICE_STATUS::default();
        if ControlService(service.0, SERVICE_CONTROL_STOP, &mut status).is_ok() {
            for _ in 0..40 {
                if QueryServiceStatus(service.0, &mut status).is_err() || status.dwCurrentState == SERVICE_STOPPED {
                    break;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
        match DeleteService(service.0) {
            Err(e) if e.code() != ERROR_SERVICE_MARKED_FOR_DELETE.to_hresult() => Err(e.into()),
            _ => Ok(()),
        }
    }
}

// --- Scheduled tasks ---

fn task_service() -> windows::core::Result<ITaskService> {
    unsafe {
        // Either apartment works, and the thread may already be in one.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let service: ITaskService = CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)?;
        let none = VARIANT::default();
        service.Connect(&none, &none, &none, &none)?;
        Ok(service)
    }
}

/// Every program a task starts, with its arguments, which is where a
/// `rundll32` task names the app's DLL.
fn task_commands(task: &IRegisteredTask) -> Vec<String> {
    let mut commands = Vec::new();
    unsafe {
        let Ok(actions) = task.Definition().and_then(|d| d.Actions()) else { return commands };
        let mut count = 0;
        if actions.Count(&mut count).is_err() {
            return commands;
        }
        for i in 1..=count {
            let Ok(exec) = actions.get_Item(i).and_then(|a| a.cast::<IExecAction>()) else { continue };
            let (mut path, mut args) = (BSTR::new(), BSTR::new());
            if exec.Path(&mut path).is_ok() {
                let _ = exec.Arguments(&mut args);
                commands.push(format!("{path} {args}"));
            }
        }
    }
    commands
}

fn tasks(runs_from: &dyn Fn(&str) -> Option<Confidence>) -> windows::core::Result<Vec<Found>> {
    let service = task_service()?;
    let mut found = Vec::new();
    let mut folders = vec![unsafe { service.GetFolder(&BSTR::from("\\"))? }];
    while let Some(folder) = folders.pop() {
        unsafe {
            if folder.Path().is_ok_and(|p| p.to_string().eq_ignore_ascii_case(WINDOWS_TASKS)) {
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
                let Ok(task) = tasks.get_Item(&VARIANT::from(i)) else { continue };
                let Some(confidence) = task_commands(&task).iter().filter_map(|c| runs_from(c)).max() else { continue };
                let Ok(path) = task.Path() else { continue };
                found.push(Found {
                    kind: LeftoverKind::ScheduledTask,
                    id: path.to_string(),
                    label: None,
                    confidence,
                    reason: "Scheduled task that runs from the app's folder",
                });
            }
        }
    }
    Ok(found)
}

fn split_task_path(path: &str) -> anyhow::Result<(&str, &str)> {
    let (folder, name) = path.rsplit_once('\\').context("Invalid task path")?;
    Ok((if folder.is_empty() { "\\" } else { folder }, name))
}

fn get_task(path: &str) -> anyhow::Result<IRegisteredTask> {
    let (folder, name) = split_task_path(path)?;
    Ok(unsafe { task_service()?.GetFolder(&BSTR::from(folder))?.GetTask(&BSTR::from(name))? })
}

fn is_missing(e: &anyhow::Error) -> bool {
    e.downcast_ref::<windows::core::Error>()
        .is_some_and(|e| e.code() == ERROR_FILE_NOT_FOUND.to_hresult() || e.code() == ERROR_PATH_NOT_FOUND.to_hresult())
}

fn task_exists(path: &str) -> bool {
    get_task(path).map_or_else(|e| !is_missing(&e), |_| true)
}

fn remove_task(path: &str) -> anyhow::Result<()> {
    let windows_task = path.to_lowercase().starts_with(&format!("{}\\", WINDOWS_TASKS.to_lowercase()));
    crate::error::ensure!(!windows_task, "This is one of Windows' own tasks");
    let (folder, name) = split_task_path(path)?;
    let deleted = unsafe { task_service().and_then(|s| s.GetFolder(&BSTR::from(folder))).and_then(|f| f.DeleteTask(&BSTR::from(name), 0)) };
    match deleted.map_err(anyhow::Error::from) {
        Err(e) if is_missing(&e) => Ok(()),
        Err(e) if e.downcast_ref::<windows::core::Error>().is_some_and(|e| e.code() == ERROR_ACCESS_DENIED.to_hresult()) => {
            Err(Refused::Denied("Removing this task needs administrator rights.").into())
        }
        result => result,
    }
}

// --- Firewall rules ---

/// A field of a rule as Windows stores it: `v2.31|Action=Allow|Dir=In|App=...|Name=...|`.
fn rule_field<'a>(rule: &'a str, name: &str) -> Option<&'a str> {
    rule.split('|').find_map(|part| part.strip_prefix(name)?.strip_prefix('='))
}

fn read_rule(id: &str) -> Option<String> {
    hklm(FIREWALL_RULES).ok()?.get_value(id).ok()
}

fn firewall_rules(runs_from: &dyn Fn(&str) -> Option<Confidence>) -> Vec<Found> {
    let Ok(key) = hklm(FIREWALL_RULES) else { return Vec::new() };
    key.enum_values()
        .flatten()
        .filter_map(|(id, _)| {
            let rule: String = key.get_value(&id).ok()?;
            let app = rule_field(&rule, "App")?;
            let confidence = runs_from(app)?;
            let name = rule_field(&rule, "Name")
                .filter(|n| !n.starts_with('@'))
                .or_else(|| Path::new(app).file_name().and_then(|f| f.to_str()))
                .unwrap_or(app);
            let direction = if rule_field(&rule, "Dir") == Some("Out") { "outbound" } else { "inbound" };
            Some(Found {
                kind: LeftoverKind::FirewallRule,
                id,
                label: Some(format!("{name} ({direction})")),
                confidence,
                reason: "Firewall rule for a program in the app's folder",
            })
        })
        .collect()
}

fn remove_firewall_rule(id: &str) -> anyhow::Result<()> {
    let Some(rule) = read_rule(id) else { return Ok(()) };
    let app = rule_field(&rule, "App").context("This firewall rule isn't for a program")?;
    crate::error::ensure!(!paths::is_protected(Path::new(&expand_env(app))), "This firewall rule is for one of Windows' own programs");
    let name = rule_field(&rule, "Name").context("This firewall rule has no name")?;
    // netsh can't pick a rule by its ID, so it narrows by everything else.
    let mut args = vec!["advfirewall".to_string(), "firewall".into(), "delete".into(), "rule".into()];
    args.push(format!("name={name}"));
    args.push(format!("program={app}"));
    if let Some(dir) = rule_field(&rule, "Dir") {
        args.push(format!("dir={}", dir.to_lowercase()));
    }
    if let Some(protocol) = rule_field(&rule, "Protocol") {
        args.push(format!("protocol={protocol}"));
    }
    Command::new(paths::system32("netsh.exe")).args(&args).creation_flags(CREATE_NO_WINDOW).output()?;
    // netsh fails the same way whether it needed elevation or matched
    // nothing, so the rule itself says whether it worked.
    if read_rule(id).is_some() {
        return Err(Refused::Denied("Windows kept this firewall rule. Removing it may need administrator rights.").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_firewall_rule_fields() {
        let rule = r"v2.31|Action=Allow|Active=TRUE|Dir=In|Protocol=6|App=C:\Program Files\Parsec\parsecd.exe|Name=Parsec|";
        assert_eq!(rule_field(rule, "App"), Some(r"C:\Program Files\Parsec\parsecd.exe"));
        assert_eq!(rule_field(rule, "Dir"), Some("In"));
        assert_eq!(rule_field(rule, "Protocol"), Some("6"));
        assert_eq!(rule_field(rule, "Name"), Some("Parsec"));
        assert_eq!(rule_field(rule, "Desc"), None);
        // "Active" must not be read as "Action".
        assert_eq!(rule_field(rule, "Act"), None);
    }

    #[test]
    fn splits_task_paths() {
        assert_eq!(split_task_path(r"\Vendor\Updater").unwrap(), (r"\Vendor", "Updater"));
        assert_eq!(split_task_path(r"\Updater").unwrap(), ("\\", "Updater"));
    }

    #[test]
    fn never_removes_windows_own() {
        assert!(remove_task(r"\Microsoft\Windows\Defrag\ScheduledDefrag").is_err());
        // A real Windows service: its program is in System32.
        let result = remove_service("Dhcp");
        assert!(result.as_ref().is_err_and(|e| !super::super::was_denied(e)), "{result:?}");
    }
}
