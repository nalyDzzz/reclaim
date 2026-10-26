//! Runs a program's own uninstaller and waits for it to exit.

use anyhow::Context;
use serde::Serialize;
use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, WAIT_FAILED};
use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR};

use super::{Program, open_entry, read_program, split_command};
use crate::error::CmdResult;
use crate::paths::expand_env;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallOutcome {
    exit_code: Option<u32>,
    /// Whether the program's registry entry is still present. Some
    /// uninstallers hand off to a helper process and exit early, so the UI
    /// keeps watching for the entry to disappear.
    still_installed: bool,
}

#[tauri::command]
pub async fn run_uninstaller(id: String, quiet: bool) -> CmdResult<UninstallOutcome> {
    let outcome = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<UninstallOutcome> {
        let entry = open_entry(&id).context("This program is no longer installed")?;
        let program = read_program(&entry).context("This program is no longer installed")?;
        let (file, params) = build_command(&entry.key_name, &program, quiet)?;
        let exit_code = launch_and_wait(&file, &params)?;
        Ok(UninstallOutcome { exit_code, still_installed: super::program_exists(id) })
    })
    .await??;
    Ok(outcome)
}

fn msi_product_code(key_name: &str, uninstall: Option<&str>) -> Option<String> {
    let looks_like_guid = |s: &str| s.len() == 38 && s.starts_with('{') && s.ends_with('}');
    if looks_like_guid(key_name) {
        return Some(key_name.to_string());
    }
    let cmd = uninstall?;
    let start = cmd.find('{')?;
    let end = cmd[start..].find('}')? + start + 1;
    let guid = &cmd[start..end];
    looks_like_guid(guid).then(|| guid.to_string())
}

fn build_command(key_name: &str, program: &Program, quiet: bool) -> anyhow::Result<(String, String)> {
    if program.is_msi
        && let Some(code) = msi_product_code(key_name, program.uninstall_string.as_deref())
    {
        // Always /X: many MSI entries register /I, which opens a "modify" dialog.
        let params = if quiet { format!("/X{code} /qb /norestart") } else { format!("/X{code}") };
        return Ok(("msiexec.exe".into(), params));
    }
    let cmd = if quiet {
        program.quiet_uninstall_string.as_ref().or(program.uninstall_string.as_ref())
    } else {
        program.uninstall_string.as_ref()
    };
    let cmd = cmd.context("This program didn't register an uninstaller")?;
    Ok(split_command(&expand_env(cmd)))
}

/// Uses ShellExecuteEx (not CreateProcess) so uninstallers that require
/// elevation trigger a normal UAC prompt instead of failing.
fn launch_and_wait(file: &str, params: &str) -> anyhow::Result<Option<u32>> {
    let h_file = HSTRING::from(file);
    let h_params = HSTRING::from(params);
    let dir = std::path::Path::new(file).parent().map(|p| HSTRING::from(p.as_os_str())).unwrap_or_default();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpFile: PCWSTR(h_file.as_ptr()),
        lpParameters: if params.is_empty() { PCWSTR::null() } else { PCWSTR(h_params.as_ptr()) },
        lpDirectory: if dir.is_empty() { PCWSTR::null() } else { PCWSTR(dir.as_ptr()) },
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    if let Err(err) = unsafe { ShellExecuteExW(&mut info) } {
        if err.code() == ERROR_CANCELLED.to_hresult() {
            anyhow::bail!("The uninstaller needs administrator approval, and the prompt was declined.");
        }
        anyhow::bail!("Couldn't start the uninstaller ({file}): {}", err.message());
    }
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
