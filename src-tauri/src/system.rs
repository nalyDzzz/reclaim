use serde::Serialize;
use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::{SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR};
use winreg::RegKey;
use winreg::enums::HKEY_LOCAL_MACHINE;

use crate::error::CmdResult;

pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut core::ffi::c_void),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

pub fn os_build() -> u32 {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .and_then(|k| k.get_value::<String, _>("CurrentBuildNumber"))
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn os_name(build: u32) -> String {
    let edition = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .and_then(|k| k.get_value::<String, _>("EditionID"))
        .unwrap_or_default();
    let family = if build >= 22000 { "Windows 11" } else { "Windows 10" };
    if edition.is_empty() { family.to_string() } else { format!("{family} {edition}") }
}

/// Mica needs Windows 11; on older builds the frontend paints a solid backdrop.
pub fn supports_mica() -> bool {
    os_build() >= 22000
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    elevated: bool,
    os_build: u32,
    os_name: String,
    mica: bool,
    app_version: String,
}

#[tauri::command]
pub fn system_info(app: AppHandle) -> SystemInfo {
    let build = os_build();
    SystemInfo {
        elevated: is_elevated(),
        os_build: build,
        os_name: os_name(build),
        mica: supports_mica(),
        app_version: app.package_info().version.to_string(),
    }
}

/// Starts a new elevated copy of Reclaim and closes this one.
#[tauri::command]
pub fn relaunch_as_admin(app: AppHandle) -> CmdResult<()> {
    let exe = std::env::current_exe()?;
    let file = HSTRING::from(exe.as_os_str());
    let verb = HSTRING::from("runas");
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // Fails with ERROR_CANCELLED if the user dismisses the UAC prompt.
    unsafe { ShellExecuteExW(&mut info) }.map_err(|e| anyhow::anyhow!("Elevation was cancelled ({e})"))?;
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    app.exit(0);
    Ok(())
}

#[tauri::command]
pub fn reveal_path(path: String) -> CmdResult<()> {
    tauri_plugin_opener::reveal_item_in_dir(path)?;
    Ok(())
}

#[tauri::command]
pub fn open_path(path: String) -> CmdResult<()> {
    tauri_plugin_opener::open_path(path, None::<&str>)?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Drive {
    letter: String,
    root: String,
    label: String,
    file_system: String,
    kind: &'static str,
    total_bytes: u64,
    free_bytes: u64,
}

#[tauri::command]
pub fn list_drives() -> Vec<Drive> {
    let mask = unsafe { GetLogicalDrives() };
    let mut drives = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = char::from(b'A' + i as u8);
        let root = format!("{letter}:\\");
        let hroot = HSTRING::from(root.as_str());
        let kind = match unsafe { GetDriveTypeW(&hroot) } {
            2 => "removable",
            3 => "fixed",
            4 => "network",
            5 => "optical",
            6 => "ram",
            _ => continue,
        };
        let (mut free, mut total) = (0u64, 0u64);
        if unsafe { GetDiskFreeSpaceExW(&hroot, None, Some(&mut total), Some(&mut free)) }.is_err() {
            continue; // e.g. empty card reader or disconnected share
        }
        let mut label = [0u16; 261];
        let mut fs = [0u16; 261];
        let _ = unsafe { GetVolumeInformationW(&hroot, Some(&mut label), None, None, None, Some(&mut fs)) };
        let to_string = |buf: &[u16]| {
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            String::from_utf16_lossy(&buf[..end])
        };
        drives.push(Drive {
            letter: letter.to_string(),
            root,
            label: to_string(&label),
            file_system: to_string(&fs),
            kind,
            total_bytes: total,
            free_bytes: free,
        });
    }
    drives
}
