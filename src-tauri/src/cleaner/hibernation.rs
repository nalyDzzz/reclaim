//! The hibernation file, hiberfil.sys. Windows keeps it at a good share of
//! the PC's memory so it can hibernate and use Fast Startup. Turning
//! hibernation off deletes it, and turning it back on makes it again.

use std::fs;
use std::os::windows::process::CommandExt;
use std::process::Command;

use anyhow::{Context, bail};
use windows::Win32::Foundation::{ERROR_SUCCESS, HLOCAL, LocalFree};
use windows::Win32::System::Power::{GetSystemPowerStatus, PowerGetActiveScheme, PowerReadDCValueIndex, SYSTEM_POWER_STATUS};
use windows::core::GUID;
use winreg::RegKey;
use winreg::enums::HKEY_LOCAL_MACHINE;

use super::{TargetReport, system_root};
use crate::paths;

const POWER: &str = r"SYSTEM\CurrentControlSet\Control\Power";
const SESSION_POWER: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Power";

/// What the Turn back on button says once hibernation is off.
pub const UNDO: &str = "Turn hibernation back on";

/// hiberfil.sys's size, if there is one. Read from the drive's folder
/// listing, since Windows keeps the file itself open exclusively.
pub fn file_size() -> Option<u64> {
    fs::read_dir(system_root())
        .ok()?
        .flatten()
        .find(|e| e.file_name().eq_ignore_ascii_case("hiberfil.sys"))
        .and_then(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
}

pub fn analyze(report: &mut TargetReport) {
    match file_size() {
        Some(bytes) => {
            report.found = true;
            report.bytes = bytes;
            report.note = details();
        }
        // No file because someone turned hibernation off, maybe here. PCs
        // that can't hibernate at all don't get a row.
        None if dword(POWER, "HibernateEnabled") == Some(0) => {
            report.found = true;
            report.note = Some("Hibernation is off, so there's no file to remove.".into());
            report.undo = Some(UNDO);
        }
        None => {}
    }
}

/// What turning hibernation off changes on this particular PC.
fn details() -> Option<String> {
    let mut notes = Vec::new();
    // 1 is the reduced file that only holds enough for Fast Startup.
    if dword(POWER, "HiberFileType") == Some(1) {
        notes.push("This PC keeps the smaller file that only Fast Startup uses, so it can't hibernate now anyway.");
    }
    // Missing means Windows' default, which is on.
    if dword(SESSION_POWER, "HiberbootEnabled") == Some(0) {
        notes.push("Fast Startup is already off on this PC.");
    }
    if hibernates_on_low_battery() {
        notes.push(
            "This PC hibernates when its battery runs critically low, which saves your work. With hibernation off it can't, so save your work before the battery runs out.",
        );
    }
    (!notes.is_empty()).then(|| notes.join(" "))
}

/// Whether the active power plan hibernates on a critically low battery.
fn hibernates_on_low_battery() -> bool {
    const BATTERY: GUID = GUID::from_u128(0xe73a048d_bf27_4f12_9731_8b2076e8891f);
    const CRITICAL_ACTION: GUID = GUID::from_u128(0x637ea02f_bbcb_4015_8e2c_a1c7b9c0b546);
    const HIBERNATE: u32 = 2;
    const NO_BATTERY: u8 = 128;
    unsafe {
        let mut status = SYSTEM_POWER_STATUS::default();
        if GetSystemPowerStatus(&mut status).is_err() || status.BatteryFlag & NO_BATTERY != 0 || status.BatteryFlag == 255 {
            return false;
        }
        let mut scheme: *mut GUID = std::ptr::null_mut();
        if PowerGetActiveScheme(None, &mut scheme) != ERROR_SUCCESS || scheme.is_null() {
            return false;
        }
        let mut action = 0u32;
        let read = PowerReadDCValueIndex(None, Some(scheme), Some(&BATTERY), Some(&CRITICAL_ACTION), &mut action);
        let _ = LocalFree(Some(HLOCAL(scheme.cast())));
        read == ERROR_SUCCESS.0 && action == HIBERNATE
    }
}

fn dword(path: &str, name: &str) -> Option<u32> {
    RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(path).ok()?.get_value(name).ok()
}

/// Turns hibernation on or off with powercfg, which makes or deletes the
/// file and switches Fast Startup with it.
pub fn set(on: bool) -> anyhow::Result<()> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = Command::new(paths::system32("powercfg.exe"))
        .args(["/hibernate", if on { "on" } else { "off" }])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .context("Couldn't start powercfg")?;
    if !out.status.success() {
        // powercfg prints its errors to standard output.
        let text = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        let reason = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("powercfg reported an error");
        bail!("Windows couldn't turn hibernation {}: {reason}", if on { "on" } else { "off" });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_this_pcs_hibernation_file() {
        // Read-only. Every PC either has the file or doesn't; when it
        // does, the folder listing must report a real size.
        if let Some(bytes) = file_size() {
            assert!(bytes > 0);
        }
        let _ = details();
    }
}
