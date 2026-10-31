//! Which programs have a leftover's files open, asked of Restart Manager,
//! the service installers use to find files in use. An app's shell
//! extension stays loaded in File Explorer after the app is gone, which
//! keeps its folder from being removed until Explorer lets go of it.

use std::path::{Path, PathBuf};

use anyhow::Context;
use windows::Win32::Foundation::{CloseHandle, ERROR_MORE_DATA, ERROR_SUCCESS};
use windows::Win32::System::RestartManager::{
    CCH_RM_SESSION_KEY, RM_PROCESS_INFO, RmEndSession, RmExplorer, RmForceShutdown, RmGetList, RmRegisterResources, RmRestart, RmShutdown,
    RmStartSession,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows::core::{HSTRING, PCWSTR, PWSTR};

use crate::paths::{known, normalize};

/// Files checked per leftover, programs and libraries first. A cache folder
/// can hold far more, and what keeps a folder in use is nearly always a
/// loaded DLL.
const MAX_FILES: usize = 2000;

/// What Windows runs to load shell extensions for Explorer: thumbnails and
/// other handlers in COM Surrogates, previews in the preview host. Closing
/// them loses nothing, and Windows starts them again when needed.
const SHELL_HOSTS: &[&str] = &["dllhost.exe", "prevhost.exe"];

pub(super) struct Holder {
    pub name: String,
    /// File Explorer, or a host that loads shell extensions for it.
    pub shell: bool,
}

/// Up to `MAX_FILES` files under `path`, programs and libraries first.
/// Doesn't follow junctions or symlinks.
fn files_under(path: &Path) -> Vec<PathBuf> {
    if path.is_file() {
        return vec![path.to_path_buf()];
    }
    let mut files = Vec::new();
    let mut dirs = vec![path.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => dirs.push(entry.path()),
                Ok(kind) if kind.is_file() => files.push(entry.path()),
                _ => {}
            }
        }
        // Plenty to pick the programs and libraries from.
        if files.len() > MAX_FILES * 4 {
            break;
        }
    }
    let is_code = |p: &PathBuf| {
        p.extension().is_some_and(|e| ["dll", "exe", "sys", "ocx", "ax", "cpl", "node"].iter().any(|x| e.eq_ignore_ascii_case(x)))
    };
    files.sort_by_key(|p| !is_code(p));
    files.truncate(MAX_FILES);
    files
}

/// A Restart Manager session watching some files, ended when dropped.
struct Session(u32);

impl Session {
    fn over(paths: &[PathBuf]) -> Option<Self> {
        let mut handle = 0;
        let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
        if unsafe { RmStartSession(&mut handle, None, PWSTR(key.as_mut_ptr())) } != ERROR_SUCCESS {
            return None;
        }
        let session = Session(handle);
        let files: Vec<HSTRING> = paths.iter().flat_map(|p| files_under(p)).map(|f| HSTRING::from(f.as_os_str())).collect();
        for chunk in files.chunks(500) {
            let names: Vec<PCWSTR> = chunk.iter().map(|f| PCWSTR(f.as_ptr())).collect();
            if unsafe { RmRegisterResources(handle, Some(&names), None, None) } != ERROR_SUCCESS {
                return None;
            }
        }
        Some(session)
    }

    fn processes(&self) -> Vec<RM_PROCESS_INFO> {
        let mut infos: Vec<RM_PROCESS_INFO> = Vec::new();
        // The list can grow between asking how long it is and fetching it.
        for _ in 0..5 {
            let (mut needed, mut count, mut reasons) = (0u32, infos.len() as u32, 0u32);
            let buffer = (!infos.is_empty()).then_some(infos.as_mut_ptr());
            match unsafe { RmGetList(self.0, &mut needed, &mut count, buffer, &mut reasons) } {
                ERROR_SUCCESS => {
                    infos.truncate(count as usize);
                    return infos;
                }
                ERROR_MORE_DATA => infos = vec![unsafe { std::mem::zeroed() }; needed as usize],
                _ => break,
            }
        }
        Vec::new()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            let _ = RmEndSession(self.0);
        }
    }
}

fn image_path(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let found = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(process);
        found.then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
}

fn holder(info: &RM_PROCESS_INFO) -> Holder {
    let image = image_path(info.Process.dwProcessId);
    let windows = normalize(&known().windows);
    let shell_host = image.as_deref().is_some_and(|image| {
        let image = normalize(Path::new(image));
        image.starts_with(&format!(r"{windows}\")) && SHELL_HOSTS.iter().any(|host| image.ends_with(&format!(r"\{host}")))
    });
    let app = &info.strAppName;
    let app = String::from_utf16_lossy(&app[..app.iter().position(|&c| c == 0).unwrap_or(app.len())]);
    let file = image.as_deref().and_then(|i| Path::new(i).file_name()).map(|f| f.to_string_lossy().to_string());
    let name = if app.trim().is_empty() { file.unwrap_or_else(|| "Another program".to_string()) } else { app.trim().to_string() };
    Holder { name, shell: info.ApplicationType == RmExplorer || shell_host }
}

/// The programs that have files under `path` open or loaded.
pub(super) fn holders(path: &Path) -> Vec<Holder> {
    Session::over(&[path.to_path_buf()]).map(|session| session.processes().iter().map(holder).collect()).unwrap_or_default()
}

/// Closes File Explorer and the shell hosts holding files under `paths`,
/// then has Restart Manager start Explorer again, the way installers do,
/// which brings it back as the signed-in user even from an elevated
/// Reclaim. Refuses if any other program holds them, since closing that
/// could lose someone's work.
pub fn release_from_shell(paths: &[PathBuf]) -> anyhow::Result<()> {
    let session = Session::over(paths).context("Couldn't ask Windows which programs are using these files")?;
    let holders: Vec<Holder> = session.processes().iter().map(holder).collect();
    let others: Vec<&str> = holders.iter().filter(|h| !h.shell).map(|h| h.name.as_str()).collect();
    crate::error::ensure!(others.is_empty(), "{} is using these files too. Close it, then try again.", others.join(", "));
    if holders.is_empty() {
        return Ok(());
    }
    unsafe {
        let closed = RmShutdown(session.0, RmForceShutdown.0 as u32, None);
        // Restart even after a partial shutdown, so the taskbar comes back.
        let restarted = RmRestart(session.0, None, None);
        crate::error::ensure!(closed == ERROR_SUCCESS, "Windows couldn't close File Explorer (error {}).", closed.0);
        crate::error::ensure!(
            restarted == ERROR_SUCCESS,
            "Windows closed File Explorer but couldn't start it again (error {}).",
            restarted.0
        );
    }
    Ok(())
}

/// PowerShell, holding a file open with no sharing until it's killed.
#[cfg(test)]
pub(super) struct HeldOpen(std::process::Child);

#[cfg(test)]
impl HeldOpen {
    /// Creates `file` and waits until PowerShell has it open.
    pub fn new(file: &Path) -> Self {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, b"x").unwrap();
        let ready = file.with_extension("ready");
        let quote = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', "''"));
        let script =
            format!("$f = [IO.File]::Open({}, 'Open', 'Read', 'None'); New-Item {} | Out-Null; Start-Sleep 30", quote(file), quote(&ready));
        let child = std::process::Command::new(crate::paths::system32(r"WindowsPowerShell\v1.0\powershell.exe"))
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .spawn()
            .unwrap();
        for _ in 0..100 {
            if ready.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(ready.exists(), "PowerShell never opened the file");
        HeldOpen(child)
    }
}

#[cfg(test)]
impl Drop for HeldOpen {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_program_holding_a_file() {
        let dir = std::env::temp_dir().join(format!("reclaim-in-use-test-{}", std::process::id()));
        let held = HeldOpen::new(&dir.join(r"bin\held.dat"));
        let found = holders(&dir);
        let refused = release_from_shell(std::slice::from_ref(&dir));
        drop(held);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(found.len(), 1, "{:?}", found.iter().map(|h| &h.name).collect::<Vec<_>>());
        assert!(!found[0].shell, "{} isn't a shell host", found[0].name);
        // PowerShell isn't Explorer, so it's never closed for you.
        assert!(refused.is_err());
    }

    #[test]
    fn nothing_holds_a_closed_folder() {
        let dir = std::env::temp_dir().join(format!("reclaim-not-in-use-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.dll"), b"x").unwrap();
        let found = holders(&dir);
        let released = release_from_shell(std::slice::from_ref(&dir));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(found.is_empty());
        assert!(released.is_ok(), "nothing to close, so nothing to refuse");
    }
}
