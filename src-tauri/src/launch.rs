//! Reclaim's command line: Explorer's "Scan with Reclaim" menu, which the
//! installers register as `reclaim.exe --scan "<folder>"`, and the elevated
//! relaunch.

use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, State};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1, SE_KERNEL_OBJECT, SetSecurityInfo,
};
use windows::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, GetSecurityDescriptorSacl, LABEL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
};
use windows::Win32::System::Threading::{
    OpenMutexW, OpenProcess, PROCESS_SYNCHRONIZE, SYNCHRONIZATION_WRITE_DAC, SYNCHRONIZATION_WRITE_OWNER, WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::{ChangeWindowMessageFilterEx, FindWindowW, MSGFLT_ALLOW, WM_COPYDATA};
use windows::core::{BOOL, HSTRING};

const SCAN: &str = "--scan";
/// Passed by `relaunch_as_admin` with the process id of the copy it replaces.
pub const RELAUNCHED_FROM: &str = "--relaunched-from";

fn command_line() -> impl Iterator<Item = String> {
    std::env::args_os().map(|arg| arg.to_string_lossy().into_owned())
}

fn value_of(flag: &str, args: impl IntoIterator<Item = String>) -> Option<String> {
    let mut args = args.into_iter();
    args.find(|arg| arg == flag)?;
    args.next()
}

/// The folder in `--scan <folder>`, if there is one.
fn scan_arg(args: impl IntoIterator<Item = String>) -> Option<String> {
    let path = value_of(SCAN, args)?;
    // Explorer quotes a drive root as "C:\", and Windows reads \" as an
    // escaped quote, so it arrives as C:". Paths can't contain quotes.
    let path = path.trim_end_matches('"');
    if path.len() == 2 && path.ends_with(':') { Some(format!("{path}\\")) } else { Some(path.to_string()) }
}

/// A folder the UI was asked to scan but hasn't picked up yet.
pub struct PendingScan(Mutex<Option<String>>);

impl PendingScan {
    pub fn from_command_line() -> Self {
        Self(Mutex::new(scan_arg(command_line())))
    }
}

/// The UI takes the pending folder on startup and again whenever
/// `scan-requested` fires, so a request that lands before it's listening
/// still gets scanned.
#[tauri::command]
pub fn take_pending_scan(state: State<'_, PendingScan>) -> Option<String> {
    state.0.lock().unwrap().take()
}

/// Runs in the open copy of Reclaim when it's started again, which then exits.
pub fn on_second_instance(app: &AppHandle, args: Vec<String>, _cwd: String) {
    if let Some(path) = scan_arg(args) {
        *app.state::<PendingScan>().0.lock().unwrap() = Some(path);
        let _ = app.emit("scan-requested", ());
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// `relaunch_as_admin` starts the elevated copy before closing itself. If the
/// new copy got going first, it would find the old one still open, hand off
/// to it and exit, leaving nothing running.
pub fn wait_for_replaced_instance() {
    let Some(pid) = value_of(RELAUNCHED_FROM, command_line()).and_then(|pid| pid.parse().ok()) else {
        return;
    };
    unsafe {
        if let Ok(process) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            WaitForSingleObject(process, 10_000);
            let _ = CloseHandle(process);
        }
    }
}

/// Lets launches that aren't elevated, like Explorer's menu, reach an elevated
/// Reclaim instead of opening a second window beside it.
///
/// The single-instance plugin finds the open copy through a named mutex and
/// passes it the command line in a WM_COPYDATA message. When the open copy is
/// elevated, its mutex only admits administrators and Windows drops messages
/// sent up to it from a normal process, so open both. The names are the
/// plugin's own. All a forwarded command line can do is start a scan.
pub fn admit_unelevated_launches(app: &AppHandle) {
    let id = &app.config().identifier;
    unsafe {
        let mutex_name = HSTRING::from(format!("{id}-sim"));
        if let Ok(mutex) = OpenMutexW(SYNCHRONIZATION_WRITE_DAC | SYNCHRONIZATION_WRITE_OWNER, false, &mutex_name) {
            // Interactive users at medium integrity: what Explorer runs as.
            let _ = set_security(mutex, "D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;IU)S:(ML;;NW;;;ME)");
            let _ = CloseHandle(mutex);
        }
        let class = HSTRING::from(format!("{id}-sic"));
        let title = HSTRING::from(format!("{id}-siw"));
        if let Ok(window) = FindWindowW(&class, &title) {
            let _ = ChangeWindowMessageFilterEx(window, WM_COPYDATA, MSGFLT_ALLOW, None);
        }
    }
}

unsafe fn set_security(object: HANDLE, sddl: &str) -> windows::core::Result<()> {
    unsafe {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(&HSTRING::from(sddl), SDDL_REVISION_1, &mut descriptor, None)?;
        let (mut present, mut defaulted) = (BOOL::default(), BOOL::default());
        let (mut dacl, mut sacl): (*mut ACL, *mut ACL) = (std::ptr::null_mut(), std::ptr::null_mut());
        let result = GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted)
            .and_then(|()| GetSecurityDescriptorSacl(descriptor, &mut present, &mut sacl, &mut defaulted))
            .and_then(|()| {
                SetSecurityInfo(
                    object,
                    SE_KERNEL_OBJECT,
                    DACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION,
                    None,
                    None,
                    Some(dacl),
                    Some(sacl),
                )
                .ok()
            });
        LocalFree(Some(HLOCAL(descriptor.0)));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn reads_the_folder_after_scan() {
        assert_eq!(scan_arg(args(&["reclaim.exe", "--scan", r"D:\Games"])), Some(r"D:\Games".into()));
        assert_eq!(scan_arg(args(&["reclaim.exe"])), None);
        assert_eq!(scan_arg(args(&["reclaim.exe", "--scan"])), None);
    }

    #[test]
    fn repairs_a_quoted_drive_root() {
        assert_eq!(scan_arg(args(&["reclaim.exe", "--scan", "C:\""])), Some(r"C:\".into()));
        assert_eq!(scan_arg(args(&["reclaim.exe", "--scan", r"C:\"])), Some(r"C:\".into()));
    }
}
