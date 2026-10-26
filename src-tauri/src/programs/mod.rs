//! Installed programs, read from the same registry keys that Windows'
//! "Installed apps" page and every uninstaller tool uses.

pub mod icon;
pub mod leftovers;
pub mod uninstall;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY};

use crate::error::CmdResult;
use crate::paths::{self, known, normalize};

pub const UNINSTALL_PATH: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    Machine64,
    Machine32,
    User,
}

impl Source {
    pub const ALL: [Source; 3] = [Source::Machine64, Source::Machine32, Source::User];

    fn tag(self) -> &'static str {
        match self {
            Source::Machine64 => "HKLM64",
            Source::Machine32 => "HKLM32",
            Source::User => "HKCU",
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.tag() == tag)
    }

    pub fn hive(self) -> RegKey {
        match self {
            Source::User => RegKey::predef(HKEY_CURRENT_USER),
            _ => RegKey::predef(HKEY_LOCAL_MACHINE),
        }
    }

    pub fn view_flag(self) -> u32 {
        match self {
            Source::Machine32 => KEY_WOW64_32KEY,
            _ => KEY_WOW64_64KEY,
        }
    }

    /// Registry path as shown to people and as accepted by `reg.exe`.
    pub fn display_root(self) -> &'static str {
        match self {
            Source::Machine64 => r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
            Source::Machine32 => r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
            Source::User => r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Program {
    /// Stable identifier: `<source tag>|<uninstall subkey name>`.
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub publisher: Option<String>,
    /// ISO date (`YYYY-MM-DD`).
    pub install_date: Option<String>,
    /// Size reported by the installer, in bytes. Often stale or missing.
    pub estimated_size: Option<u64>,
    pub install_location: Option<String>,
    /// Best guess at the program's own folder, used for sizing and leftovers.
    pub inferred_location: Option<String>,
    pub uninstall_string: Option<String>,
    pub quiet_uninstall_string: Option<String>,
    pub display_icon: Option<String>,
    pub url: Option<String>,
    pub comments: Option<String>,
    pub registry_path: String,
    pub source: Source,
    pub is_msi: bool,
}

pub(crate) struct RawEntry {
    pub source: Source,
    pub key_name: String,
    pub key: RegKey,
}

fn get_str(key: &RegKey, name: &str) -> Option<String> {
    key.get_value::<String, _>(name).ok().map(|s| s.trim().trim_end_matches('\0').trim().to_string()).filter(|s| !s.is_empty())
}

fn get_u32(key: &RegKey, name: &str) -> Option<u32> {
    key.get_value::<u32, _>(name).ok()
}

pub(crate) fn open_uninstall_root(source: Source) -> Option<RegKey> {
    source.hive().open_subkey_with_flags(UNINSTALL_PATH, KEY_READ | source.view_flag()).ok()
}

pub(crate) fn open_entry(id: &str) -> Option<RawEntry> {
    let (tag, key_name) = id.split_once('|')?;
    let source = Source::from_tag(tag)?;
    let key = open_uninstall_root(source)?.open_subkey_with_flags(key_name, KEY_READ | source.view_flag()).ok()?;
    Some(RawEntry { source, key_name: key_name.to_string(), key })
}

fn parse_install_date(raw: &str) -> Option<String> {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if raw.len() == 8 && digits.len() == 8 {
        let (y, m, d) = (&digits[0..4], &digits[4..6], &digits[6..8]);
        let month: u32 = m.parse().ok()?;
        let day: u32 = d.parse().ok()?;
        if (1..=12).contains(&month) && (1..=31).contains(&day) {
            return Some(format!("{y}-{m}-{d}"));
        }
    }
    // Some installers write M/D/YYYY.
    let parts: Vec<&str> = raw.split(['/', '-', '.']).collect();
    if parts.len() == 3 {
        let nums: Vec<u32> = parts.iter().filter_map(|p| p.trim().parse().ok()).collect();
        if nums.len() == 3 {
            let (y, m, d) = if nums[0] > 1900 { (nums[0], nums[1], nums[2]) } else { (nums[2], nums[0], nums[1]) };
            if y > 1990 && (1..=12).contains(&m) && (1..=31).contains(&d) {
                return Some(format!("{y:04}-{m:02}-{d:02}"));
            }
        }
    }
    None
}

fn key_write_date(key: &RegKey) -> Option<String> {
    let st = key.query_info().ok()?.get_last_write_time_system();
    (st.wYear > 1990).then(|| format!("{:04}-{:02}-{:02}", st.wYear, st.wMonth, st.wDay))
}

/// Splits a command line into the executable and its arguments, coping
/// with the unquoted paths-with-spaces that many installers write.
pub fn split_command(cmd: &str) -> (String, String) {
    let s = cmd.trim();
    if let Some(rest) = s.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            return (rest[..end].to_string(), rest[end + 1..].trim().to_string());
        }
        return (rest.to_string(), String::new());
    }
    let lower = s.to_lowercase();
    if let Some(i) = lower.find(".exe") {
        let end = i + 4;
        return (s[..end].to_string(), s[end..].trim().to_string());
    }
    match s.split_once(char::is_whitespace) {
        Some((exe, args)) => (exe.to_string(), args.trim().to_string()),
        None => (s.to_string(), String::new()),
    }
}

/// `"C:\path\app.exe",0` -> (`C:\path\app.exe`, 0)
pub fn parse_icon_ref(raw: &str) -> (String, i32) {
    let s = raw.trim();
    if let Some((path, idx)) = s.rsplit_once(',')
        && let Ok(i) = idx.trim().parse::<i32>()
    {
        return (path.trim().trim_matches('"').to_string(), i);
    }
    (s.trim_matches('"').to_string(), 0)
}

fn is_generic_host(exe: &str) -> bool {
    let name = Path::new(exe).file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    matches!(
        name.as_str(),
        "msiexec.exe"
            | "msiexec"
            | "rundll32.exe"
            | "rundll32"
            | "cmd.exe"
            | "powershell.exe"
            | "wscript.exe"
            | "steam.exe"
            // Browsers host installed web apps (PWAs); their folder isn't the web app's.
            | "chrome.exe"
            | "chrome_proxy.exe"
            | "msedge.exe"
            | "msedge_proxy.exe"
            | "brave.exe"
            | "vivaldi.exe"
            | "opera.exe"
            | "firefox.exe"
    )
}

/// Climbs out of version-number and installer subfolders, e.g.
/// `App\Application\1.2.3\Installer` -> `App\Application`.
fn program_root_from(dir: &Path) -> &Path {
    let mut cur = dir;
    while let (Some(name), Some(parent)) = (cur.file_name(), cur.parent()) {
        let name = name.to_string_lossy().to_lowercase();
        let version_like = name.chars().all(|c| c.is_ascii_digit() || c == '.') && name.contains('.');
        if version_like || matches!(name.as_str(), "installer" | "uninstall" | "uninst" | "_uninst" | "bin") {
            cur = parent;
        } else {
            break;
        }
    }
    cur
}

const GENERIC_WORDS: &[&str] = &[
    "the",
    "and",
    "for",
    "app",
    "apps",
    "version",
    "software",
    "edition",
    "windows",
    "program",
    "programs",
    "files",
    "game",
    "games",
    "gaming",
    "client",
    "desktop",
    "tools",
    "tool",
    "update",
    "service",
    "driver",
    "runtime",
    "system",
    "common",
    "data",
    "user",
    "inc",
    "llc",
    "ltd",
    "corporation",
    "corp",
    "studios",
    "studio",
    "entertainment",
    "technologies",
    "team",
    "x64",
    "x86",
];

/// Whether `dir` plausibly belongs to this program: one of its last few
/// path components must contain a distinctive word from the program's or
/// publisher's name. Guards against shared folders like Steam's icon cache.
pub(crate) fn related_to(dir: &Path, name: &str, publisher: Option<&str>) -> bool {
    let words: Vec<String> = name
        .split(|c: char| !c.is_alphanumeric())
        .chain(publisher.unwrap_or("").split(|c: char| !c.is_alphanumeric()))
        .map(str::to_lowercase)
        .filter(|w| w.len() >= 3 && !GENERIC_WORDS.contains(&w.as_str()) && !w.chars().all(|c| c.is_ascii_digit()))
        .collect();
    dir.components().rev().take(3).any(|c| {
        let component: String = c.as_os_str().to_string_lossy().to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect();
        words.iter().any(|w| component.contains(w.as_str()))
    })
}

fn usable_program_dir(dir: &Path) -> bool {
    if paths::is_protected(dir) || !dir.is_dir() {
        return false;
    }
    let p = normalize(dir);
    let k = known();
    let package_cache = normalize(&k.program_data.join("Package Cache"));
    !(p.starts_with(&package_cache) || p.contains(r"\installer\") || p.contains(r"\temp\"))
}

fn infer_location(
    name: &str,
    publisher: Option<&str>,
    install_location: Option<&str>,
    display_icon: Option<&str>,
    uninstall: Option<&str>,
) -> Option<String> {
    if let Some(loc) = install_location {
        let path = PathBuf::from(loc.trim_matches('"'));
        if usable_program_dir(&path) {
            return Some(path.to_string_lossy().trim_end_matches('\\').to_string());
        }
    }
    if uninstall.is_some_and(|u| u.contains("--uninstall-app-id")) {
        return None; // browser-installed web app
    }
    let candidates = [uninstall.map(|u| split_command(u).0), display_icon.map(|i| parse_icon_ref(i).0)];
    for exe in candidates.into_iter().flatten() {
        if exe.is_empty() || is_generic_host(&exe) {
            continue;
        }
        if let Some(parent) = Path::new(&exe).parent() {
            let root = program_root_from(parent);
            if usable_program_dir(root) && related_to(root, name, publisher) {
                return Some(root.to_string_lossy().to_string());
            }
        }
    }
    None
}

pub(crate) fn read_program(entry: &RawEntry) -> Option<Program> {
    let key = &entry.key;
    let name = get_str(key, "DisplayName")?;
    if get_u32(key, "SystemComponent") == Some(1) || get_str(key, "ParentKeyName").is_some() {
        return None;
    }
    if let Some(release) = get_str(key, "ReleaseType") {
        let r = release.to_lowercase();
        if r.contains("update") || r.contains("hotfix") {
            return None;
        }
    }
    let uninstall_string = get_str(key, "UninstallString");
    let is_msi =
        get_u32(key, "WindowsInstaller") == Some(1) || uninstall_string.as_deref().is_some_and(|u| u.to_lowercase().contains("msiexec"));
    if uninstall_string.is_none() && !is_msi {
        return None;
    }
    let install_location = get_str(key, "InstallLocation");
    let display_icon = get_str(key, "DisplayIcon");
    let publisher = get_str(key, "Publisher");
    let inferred_location =
        infer_location(&name, publisher.as_deref(), install_location.as_deref(), display_icon.as_deref(), uninstall_string.as_deref());
    Some(Program {
        id: format!("{}|{}", entry.source.tag(), entry.key_name),
        name,
        version: get_str(key, "DisplayVersion"),
        publisher,
        install_date: get_str(key, "InstallDate").and_then(|d| parse_install_date(&d)).or_else(|| key_write_date(key)),
        estimated_size: get_u32(key, "EstimatedSize").filter(|&kb| kb > 0).map(|kb| kb as u64 * 1024),
        install_location,
        inferred_location,
        quiet_uninstall_string: get_str(key, "QuietUninstallString"),
        uninstall_string,
        display_icon,
        url: get_str(key, "URLInfoAbout").or_else(|| get_str(key, "HelpLink")),
        comments: get_str(key, "Comments"),
        registry_path: format!(r"{}\{}", entry.source.display_root(), entry.key_name),
        source: entry.source,
        is_msi,
    })
}

pub fn read_all() -> Vec<Program> {
    let mut programs = Vec::new();
    let mut seen = HashSet::new();
    for source in Source::ALL {
        let Some(root) = open_uninstall_root(source) else { continue };
        for key_name in root.enum_keys().flatten() {
            let Ok(key) = root.open_subkey_with_flags(&key_name, KEY_READ | source.view_flag()) else { continue };
            let entry = RawEntry { source, key_name, key };
            if let Some(p) = read_program(&entry) {
                let dedupe = (p.name.to_lowercase(), p.version.clone().unwrap_or_default().to_lowercase());
                if seen.insert(dedupe) {
                    programs.push(p);
                }
            }
        }
    }
    programs.sort_by_key(|p| p.name.to_lowercase());
    programs
}

#[tauri::command]
pub async fn list_programs() -> CmdResult<Vec<Program>> {
    Ok(tauri::async_runtime::spawn_blocking(read_all).await?)
}

#[tauri::command]
pub fn program_exists(id: String) -> bool {
    open_entry(&id).is_some_and(|e| read_program(&e).is_some())
}

#[derive(Deserialize)]
pub struct MeasureRequest {
    id: String,
    path: String,
}

/// Measures the real on-disk size of each program's folder.
#[tauri::command]
pub async fn measure_programs(items: Vec<MeasureRequest>) -> CmdResult<HashMap<String, u64>> {
    let sizes = tauri::async_runtime::spawn_blocking(move || {
        items
            .par_iter()
            .filter_map(|item| {
                let path = Path::new(&item.path);
                if !usable_program_dir(path) {
                    return None;
                }
                Some((item.id.clone(), paths::dir_stats(path).bytes))
            })
            .collect::<HashMap<_, _>>()
    })
    .await?;
    Ok(sizes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_commands() {
        assert_eq!(
            split_command(r#""C:\Program Files\App\unins000.exe" /SILENT"#),
            (r"C:\Program Files\App\unins000.exe".into(), "/SILENT".into())
        );
        assert_eq!(
            split_command(r"C:\Program Files\App\uninstall.exe --force"),
            (r"C:\Program Files\App\uninstall.exe".into(), "--force".into())
        );
        assert_eq!(
            split_command("MsiExec.exe /X{0DA71DCF-F605-435D-9B16-051}"),
            ("MsiExec.exe".into(), "/X{0DA71DCF-F605-435D-9B16-051}".into())
        );
    }

    #[test]
    fn parses_dates() {
        assert_eq!(parse_install_date("20261024").as_deref(), Some("2026-10-24"));
        assert_eq!(parse_install_date("10/24/2026").as_deref(), Some("2026-10-24"));
        assert_eq!(parse_install_date("garbage"), None);
    }

    #[test]
    fn parses_icon_refs() {
        assert_eq!(parse_icon_ref(r#""C:\a b\app.exe",-101"#), (r"C:\a b\app.exe".into(), -101));
        assert_eq!(parse_icon_ref(r"C:\app\icon.ico"), (r"C:\app\icon.ico".into(), 0));
    }

    #[test]
    fn relates_folders_to_programs() {
        assert!(related_to(Path::new(r"C:\Users\me\AppData\Local\Discord"), "Discord", Some("Discord Inc.")));
        assert!(related_to(Path::new(r"C:\Program Files\Microsoft OneDrive\26.183.0920.0005"), "Microsoft OneDrive", None));
        assert!(related_to(Path::new(r"C:\Program Files\BraveSoftware\Brave-Browser\Application"), "Brave", None));
        assert!(!related_to(Path::new(r"C:\Program Files (x86)\Steam\steam\games"), "Lethal Company", Some("Zeekerss")));
        assert!(!related_to(Path::new(r"C:\Program Files (x86)\Steam\steam\games"), "Crosshair X", Some("CenterPoint Gaming")));
    }

    #[test]
    fn reads_registry() {
        let programs = read_all();
        assert!(!programs.is_empty(), "expected at least one installed program");
    }
}
