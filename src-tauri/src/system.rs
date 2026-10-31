use serde::Serialize;
use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_FAILED};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW};
use windows::Win32::System::Threading::{GetCurrentProcess, GetExitCodeProcess, INFINITE, OpenProcessToken, WaitForSingleObject};
use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
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
    install_kind: &'static str,
    user_profile: String,
}

/// How this copy was installed, which decides whether it can update itself.
/// The bundler stamps the bundle type into the exe it packs into each
/// installer; the plain exe (portable and dev builds) stays unstamped.
fn install_kind() -> &'static str {
    use tauri::utils::config::BundleType;
    match tauri::utils::platform::bundle_type() {
        Some(BundleType::Nsis) => "nsis",
        Some(BundleType::Msi) => "msi",
        _ => "portable",
    }
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
        install_kind: install_kind(),
        user_profile: crate::paths::known().user_profile.to_string_lossy().to_string(),
    }
}

/// Starts `file` and waits for it to exit, returning its exit code when
/// Windows reports one. Uses ShellExecuteEx (not CreateProcess) so programs
/// that need elevation, or are started with the `runas` verb, get a normal
/// UAC prompt. A declined prompt fails with `ERROR_CANCELLED`.
pub fn shell_execute_and_wait(verb: Option<&str>, file: &str, params: &str) -> windows::core::Result<Option<u32>> {
    let h_verb = verb.map(HSTRING::from);
    let h_file = HSTRING::from(file);
    let h_params = HSTRING::from(params);
    let dir = std::path::Path::new(file).parent().map(|p| HSTRING::from(p.as_os_str())).unwrap_or_default();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: h_verb.as_ref().map_or(PCWSTR::null(), |v| PCWSTR(v.as_ptr())),
        lpFile: PCWSTR(h_file.as_ptr()),
        lpParameters: if params.is_empty() { PCWSTR::null() } else { PCWSTR(h_params.as_ptr()) },
        lpDirectory: if dir.is_empty() { PCWSTR::null() } else { PCWSTR(dir.as_ptr()) },
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }?;
    if info.hProcess.is_invalid() {
        return Ok(None);
    }
    let mut code = 0u32;
    unsafe {
        let waited = WaitForSingleObject(info.hProcess, INFINITE);
        let got_code = waited != WAIT_FAILED && GetExitCodeProcess(info.hProcess, &mut code).is_ok();
        let _ = CloseHandle(info.hProcess);
        Ok(got_code.then_some(code))
    }
}

/// Starts a new elevated copy of Reclaim and closes this one.
#[tauri::command]
pub fn relaunch_as_admin(app: AppHandle) -> CmdResult<()> {
    let exe = std::env::current_exe()?;
    let file = HSTRING::from(exe.as_os_str());
    let verb = HSTRING::from("runas");
    let params = HSTRING::from(format!("{} {}", crate::launch::RELAUNCHED_FROM, std::process::id()));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
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
