//! Finds what an uninstaller left behind (folders, shortcuts, registry keys,
//! startup entries) and removes the ones the person confirms.
//!
//! Matching is deliberately conservative: names must match exactly after
//! normalization, and every candidate carries a confidence level so the UI
//! only pre-selects the ones that are almost certainly safe.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_ALL_ACCESS, KEY_READ, KEY_WOW64_64KEY};

use super::open_entry;
use crate::error::CmdResult;
use crate::paths::{self, known, normalize};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeftoverQuery {
    program_id: String,
    name: String,
    publisher: Option<String>,
    install_location: Option<String>,
    inferred_location: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum LeftoverKind {
    Folder,
    File,
    RegistryKey,
    RegistryValue,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Leftover {
    kind: LeftoverKind,
    path: String,
    value_name: Option<String>,
    size_bytes: u64,
    confidence: Confidence,
    reason: String,
}

fn compact(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn is_version_token(t: &str) -> bool {
    let t = t.trim_start_matches(['v', 'V']);
    t.contains('.') && t.chars().all(|c| c.is_ascii_digit() || c == '.')
}

const NOISE_TOKENS: &[&str] = &["x64", "x86", "64-bit", "32-bit", "64bit", "32bit", "amd64", "arm64", "win64", "win32"];
const LEGAL_SUFFIXES: &[&str] = &[
    "inc",
    "llc",
    "ltd",
    "limited",
    "corporation",
    "corp",
    "co",
    "gmbh",
    "ag",
    "sa",
    "ab",
    "bv",
    "plc",
    "pty",
    "srl",
    "incorporated",
    "company",
];
const DESCRIPTIVE_WORDS: &[&str] = &["technologies", "software", "systems", "contributors", "team", "foundation", "the"];

/// Ways a program's folder/key might be named, from most to least specific.
struct NameVariants {
    full: Vec<String>,
    /// Name without the publisher prefix ("NVIDIA PhysX" -> "physx").
    short: Option<String>,
    first_word: Option<String>,
    publisher: Vec<String>,
}

impl NameVariants {
    fn new(name: &str, publisher: Option<&str>) -> Self {
        let mut cleaned = name.to_string();
        if let Some(i) = cleaned.find(" - ") {
            cleaned.truncate(i);
        }
        // Drop parenthesized qualifiers like "(x64)" or "(remove only)".
        while let (Some(a), Some(b)) = (cleaned.find('('), cleaned.find(')')) {
            if b <= a {
                break;
            }
            cleaned.replace_range(a..=b, " ");
        }
        let tokens: Vec<&str> =
            cleaned.split_whitespace().filter(|t| !is_version_token(t) && !NOISE_TOKENS.contains(&t.to_lowercase().as_str())).collect();

        let all_publisher_words: Vec<String> =
            publisher.unwrap_or("").split(|c: char| c.is_whitespace() || c == ',').map(compact).filter(|w| !w.is_empty()).collect();
        let without_legal: Vec<String> = all_publisher_words.iter().filter(|w| !LEGAL_SUFFIXES.contains(&w.as_str())).cloned().collect();
        let publisher_words: Vec<String> = without_legal.iter().filter(|w| !DESCRIPTIVE_WORDS.contains(&w.as_str())).cloned().collect();

        let mut full = vec![compact(&tokens.join(" ")), compact(name)];
        full.retain(|v| v.len() >= 3);
        full.dedup();

        let short_tokens: Vec<&&str> = tokens.iter().skip_while(|t| publisher_words.iter().any(|p| *p == compact(t))).collect();
        let short = (short_tokens.len() < tokens.len())
            .then(|| compact(&short_tokens.iter().map(|t| **t).collect::<Vec<_>>().join(" ")))
            .filter(|s| s.len() >= 4);
        let first_word = short_tokens
            .first()
            .map(|t| compact(t))
            .filter(|w| w.len() >= 4 && w.chars().all(char::is_alphabetic) && Some(w) != short.as_ref());

        let mut publisher_variants = Vec::new();
        if let Some(p) = publisher {
            publisher_variants.push(compact(p));
            publisher_variants.push(without_legal.concat());
            publisher_variants.push(publisher_words.concat());
            publisher_variants.retain(|v| v.len() >= 3);
            publisher_variants.dedup();
        }
        NameVariants { full, short, first_word, publisher: publisher_variants }
    }

    fn matches_name(&self, candidate: &str) -> bool {
        let c = compact(candidate);
        self.full.contains(&c) || self.short.as_ref() == Some(&c)
    }

    fn matches_publisher(&self, candidate: &str) -> bool {
        self.publisher.contains(&compact(candidate))
    }

    fn matches_first_word(&self, candidate: &str) -> bool {
        self.first_word.as_ref() == Some(&compact(candidate))
    }
}

struct Collector {
    items: Vec<Leftover>,
    seen: HashSet<String>,
}

impl Collector {
    fn push(&mut self, kind: LeftoverKind, path: String, value_name: Option<String>, confidence: Confidence, reason: &str) {
        let key = format!("{}|{}", path.to_lowercase(), value_name.as_deref().unwrap_or(""));
        if !self.seen.insert(key) {
            return;
        }
        self.items.push(Leftover { kind, path, value_name, size_bytes: 0, confidence, reason: reason.to_string() });
    }

    fn push_path(&mut self, path: &Path, confidence: Confidence, reason: &str) {
        if paths::is_protected(path) {
            return;
        }
        let kind = if path.is_dir() { LeftoverKind::Folder } else { LeftoverKind::File };
        // Registry values often carry a trailing slash; drop it so duplicates collapse.
        let display = path.to_string_lossy();
        let display = if display.len() > 3 { display.trim_end_matches('\\') } else { &display };
        self.push(kind, display.to_string(), None, confidence, reason);
    }
}

fn scan_filesystem(q: &LeftoverQuery, names: &NameVariants, out: &mut Collector) {
    let k = known();
    if let Some(loc) = &q.install_location {
        let path = PathBuf::from(loc.trim_matches('"'));
        // Some installers register a shared parent folder as their location,
        // so only pre-select it when the folder name matches the app.
        if !path.exists() {
        } else if super::related_to(&path, &q.name, q.publisher.as_deref()) {
            out.push_path(&path, Confidence::High, "Install folder still exists");
        } else {
            out.push_path(&path, Confidence::Low, "Registered install folder, but its name doesn't match the app");
        }
    }
    if let Some(loc) = &q.inferred_location {
        let path = PathBuf::from(loc.trim_matches('"'));
        if path.exists() {
            out.push_path(&path, Confidence::Medium, "Folder the app ran from still exists");
        }
    }

    let program_roots = [&k.program_files, &k.program_files_x86, &k.local_programs];
    let data_roots = [&k.program_data, &k.app_data, &k.local_app_data, &k.user_profile.join(r"AppData\LocalLow")];
    let roots = program_roots.iter().map(|r| (r.to_path_buf(), true)).chain(data_roots.iter().map(|r| (r.to_path_buf(), false)));

    for (root, is_program_root) in roots {
        let Ok(entries) = std::fs::read_dir(&root) else { continue };
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if names.matches_name(&name) {
                let confidence = if is_program_root { Confidence::High } else { Confidence::Medium };
                let reason = if is_program_root { "Program folder named after the app" } else { "App data folder named after the app" };
                out.push_path(&entry.path(), confidence, reason);
            } else if names.matches_publisher(&name) {
                scan_publisher_folder(&entry.path(), names, out);
            }
        }
    }

    // Start menu and desktop shortcuts.
    let shortcut_dirs = [
        k.start_menu_user.clone(),
        k.start_menu_common.clone(),
        k.user_profile.join("Desktop"),
        std::env::var_os("PUBLIC").map(|p| PathBuf::from(p).join("Desktop")).unwrap_or_default(),
    ];
    for dir in shortcut_dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if names.matches_name(&name) || names.matches_publisher(&name) {
                    let only_ours = std::fs::read_dir(&path)
                        .map(|it| it.flatten().all(|e| names.matches_name(&stem(&e.path())) || is_uninstall_shortcut(&e.path())))
                        .unwrap_or(false);
                    if names.matches_name(&name) || only_ours {
                        out.push_path(&path, Confidence::High, "Start menu folder");
                    }
                }
            } else if is_shortcut(&path) && names.matches_name(&stem(&path)) {
                out.push_path(&path, Confidence::High, "Shortcut");
            }
        }
    }
}

fn scan_publisher_folder(folder: &Path, names: &NameVariants, out: &mut Collector) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    let mut total = 0;
    let mut matched = Vec::new();
    for entry in entries.flatten() {
        total += 1;
        let name = entry.file_name().to_string_lossy().to_string();
        if names.matches_name(&name) {
            matched.push((entry.path(), Confidence::Medium));
        } else if names.matches_first_word(&name) {
            matched.push((entry.path(), Confidence::Low));
        }
    }
    if total == 0 {
        out.push_path(folder, Confidence::Medium, "Empty publisher folder");
    } else if !matched.is_empty() && matched.len() == total && matched.iter().all(|(_, c)| *c == Confidence::Medium) {
        out.push_path(folder, Confidence::Medium, "Publisher folder that only holds this app");
    } else {
        for (path, confidence) in matched {
            out.push_path(&path, confidence, "Folder inside the publisher's folder");
        }
    }
}

fn stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
}

fn is_shortcut(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk") || e.eq_ignore_ascii_case("url"))
}

fn is_uninstall_shortcut(path: &Path) -> bool {
    is_shortcut(path) && stem(path).to_lowercase().contains("uninstall")
}

struct RegRoot {
    label: &'static str,
    hive: RegKey,
    software: &'static str,
}

fn registry_roots() -> Vec<RegRoot> {
    vec![
        RegRoot { label: "HKCU", hive: RegKey::predef(HKEY_CURRENT_USER), software: "Software" },
        RegRoot { label: "HKLM", hive: RegKey::predef(HKEY_LOCAL_MACHINE), software: "SOFTWARE" },
        RegRoot { label: "HKLM", hive: RegKey::predef(HKEY_LOCAL_MACHINE), software: r"SOFTWARE\WOW6432Node" },
    ]
}

fn scan_registry(q: &LeftoverQuery, names: &NameVariants, out: &mut Collector) {
    // The uninstall entry itself, if the uninstaller forgot to remove it.
    if let Some(entry) = open_entry(&q.program_id) {
        let path = format!(r"{}\{}", entry.source.display_root(), entry.key_name);
        out.push(LeftoverKind::RegistryKey, path, None, Confidence::High, "Uninstall entry is still registered");
    }

    for root in registry_roots() {
        let Ok(software) = root.hive.open_subkey_with_flags(root.software, KEY_READ | KEY_WOW64_64KEY) else { continue };
        for sub in software.enum_keys().flatten() {
            let path = format!(r"{}\{}\{}", root.label, root.software, sub);
            if names.matches_name(&sub) {
                out.push(LeftoverKind::RegistryKey, path, None, Confidence::Medium, "Settings key named after the app");
            } else if names.matches_publisher(&sub) {
                let Ok(publisher_key) = software.open_subkey_with_flags(&sub, KEY_READ | KEY_WOW64_64KEY) else { continue };
                let children: Vec<String> = publisher_key.enum_keys().flatten().collect();
                let matched: Vec<&String> = children.iter().filter(|c| names.matches_name(c)).collect();
                let has_values = publisher_key.enum_values().next().is_some();
                if !children.is_empty()
                    && matched.len() == children.len()
                    && !has_values
                    && !is_protected_reg_key(&format!(r"{}\{}", root.software, sub))
                {
                    out.push(LeftoverKind::RegistryKey, path, None, Confidence::Medium, "Publisher key that only holds this app");
                } else {
                    for child in matched {
                        out.push(
                            LeftoverKind::RegistryKey,
                            format!(r"{path}\{child}"),
                            None,
                            Confidence::Medium,
                            "Settings key inside the publisher's key",
                        );
                    }
                }
            }
        }
    }

    // Startup entries and App Paths registrations that point into the app's folder.
    let locations: Vec<String> = [&q.install_location, &q.inferred_location]
        .into_iter()
        .flatten()
        .map(|l| normalize(Path::new(l.trim_matches('"'))))
        .filter(|l| l.len() > 3)
        .collect();
    let points_inside = |data: &str| {
        let d = data.to_lowercase().replace('/', "\\");
        locations.iter().any(|l| d.contains(l.as_str()))
    };
    let run_keys = [
        ("HKCU", RegKey::predef(HKEY_CURRENT_USER), r"Software\Microsoft\Windows\CurrentVersion\Run"),
        ("HKLM", RegKey::predef(HKEY_LOCAL_MACHINE), r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run"),
        ("HKLM", RegKey::predef(HKEY_LOCAL_MACHINE), r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run"),
    ];
    for (label, hive, path) in &run_keys {
        let Ok(key) = hive.open_subkey_with_flags(path, KEY_READ | KEY_WOW64_64KEY) else { continue };
        for (value_name, _) in key.enum_values().flatten() {
            let data: String = key.get_value(&value_name).unwrap_or_default();
            let (hit, confidence) =
                if points_inside(&data) { (true, Confidence::High) } else { (names.matches_name(&value_name), Confidence::Medium) };
            if hit {
                out.push(LeftoverKind::RegistryValue, format!(r"{label}\{path}"), Some(value_name), confidence, "Starts with Windows");
            }
        }
    }
    let app_paths = [
        ("HKCU", RegKey::predef(HKEY_CURRENT_USER), r"Software\Microsoft\Windows\CurrentVersion\App Paths"),
        ("HKLM", RegKey::predef(HKEY_LOCAL_MACHINE), r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths"),
    ];
    for (label, hive, path) in &app_paths {
        let Ok(key) = hive.open_subkey_with_flags(path, KEY_READ | KEY_WOW64_64KEY) else { continue };
        for exe in key.enum_keys().flatten() {
            let target: String =
                key.open_subkey_with_flags(&exe, KEY_READ | KEY_WOW64_64KEY).and_then(|k| k.get_value("")).unwrap_or_default();
            if !target.is_empty() && points_inside(&target) {
                out.push(LeftoverKind::RegistryKey, format!(r"{label}\{path}\{exe}"), None, Confidence::High, "Registered app path");
            }
        }
    }
}

#[tauri::command]
pub async fn scan_leftovers(query: LeftoverQuery) -> CmdResult<Vec<Leftover>> {
    let items = tauri::async_runtime::spawn_blocking(move || {
        let names = NameVariants::new(&query.name, query.publisher.as_deref());
        let mut out = Collector { items: Vec::new(), seen: HashSet::new() };
        scan_filesystem(&query, &names, &mut out);
        scan_registry(&query, &names, &mut out);

        // Drop entries nested inside another candidate folder.
        let folders: Vec<String> =
            out.items.iter().filter(|i| i.kind == LeftoverKind::Folder).map(|i| normalize(Path::new(&i.path))).collect();
        out.items.retain(|i| {
            if !matches!(i.kind, LeftoverKind::Folder | LeftoverKind::File) {
                return true;
            }
            let p = normalize(Path::new(&i.path));
            !folders.iter().any(|f| p != *f && p.starts_with(&format!("{f}\\")))
        });
        out.items.par_iter_mut().for_each(|item| {
            if matches!(item.kind, LeftoverKind::Folder | LeftoverKind::File) {
                item.size_bytes = paths::path_stats(Path::new(&item.path)).bytes;
            }
        });
        out.items.sort_by(|a, b| b.confidence.cmp(&a.confidence).then(b.size_bytes.cmp(&a.size_bytes)));
        out.items
    })
    .await?;
    Ok(items)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovalResult {
    path: String,
    value_name: Option<String>,
    ok: bool,
    error: Option<String>,
}

/// Core Microsoft keys that must never be removed, even if a name matches.
const PROTECTED_MICROSOFT_KEYS: &[&str] = &[
    "windows",
    "windows nt",
    "cryptography",
    "ole",
    "rpc",
    "com3",
    "wbem",
    "security center",
    "windows defender",
    "windowsruntime",
    "ctf",
    "input",
    "directx",
    "fusion",
    "sqmclient",
    "appx",
    "policies",
    "active setup",
    "internet explorer",
    "windows search",
    "net framework setup",
    ".netframework",
    "driverdatabase",
    "tracing",
    "windowsupdate",
    "provisioning",
    "userdata",
];

fn reg_parts(sub: &str) -> Vec<String> {
    sub.trim_matches('\\').split('\\').map(str::to_lowercase).collect()
}

/// Returns true for registry keys Reclaim must never delete.
pub fn is_protected_reg_key(sub: &str) -> bool {
    let parts = reg_parts(sub);
    if parts.len() < 2 || parts[0] != "software" {
        return true;
    }
    let rest = if parts[1] == "wow6432node" { &parts[2..] } else { &parts[1..] };
    let Some(first) = rest.first() else { return true };
    if matches!(first.as_str(), "classes" | "policies" | "clients" | "registeredapplications" | "wow6432node") {
        return true;
    }
    if first == "microsoft" {
        let Some(second) = rest.get(1) else { return true };
        if second == "windows" {
            // Only individual Uninstall / App Paths entries under CurrentVersion.
            return !(rest.len() >= 5 && rest[2] == "currentversion" && matches!(rest[3].as_str(), "uninstall" | "app paths"));
        }
        return PROTECTED_MICROSOFT_KEYS.contains(&second.as_str());
    }
    false
}

fn is_run_key(sub: &str) -> bool {
    let p = reg_parts(sub).join("\\");
    p == r"software\microsoft\windows\currentversion\run" || p == r"software\wow6432node\microsoft\windows\currentversion\run"
}

fn split_reg_path(path: &str) -> anyhow::Result<(&'static str, RegKey, String)> {
    let (hive, sub) = path.split_once('\\').context("Invalid registry path")?;
    let (label, key) = match hive.to_uppercase().as_str() {
        "HKCU" | "HKEY_CURRENT_USER" => ("HKCU", RegKey::predef(HKEY_CURRENT_USER)),
        "HKLM" | "HKEY_LOCAL_MACHINE" => ("HKLM", RegKey::predef(HKEY_LOCAL_MACHINE)),
        _ => anyhow::bail!("Only HKCU and HKLM keys can be removed"),
    };
    Ok((label, key, sub.to_string()))
}

fn backup_key(backup_dir: &Path, label: &str, sub: &str) -> anyhow::Result<()> {
    use std::os::windows::process::CommandExt;
    std::fs::create_dir_all(backup_dir)?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let safe: String = sub.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
    let file = backup_dir.join(format!("{stamp}_{label}_{}.reg", &safe[safe.len().saturating_sub(80)..]));
    let status = Command::new("reg.exe")
        .args(["export", &format!(r"{label}\{sub}"), &file.to_string_lossy(), "/y", "/reg:64"])
        .creation_flags(CREATE_NO_WINDOW)
        .status()?;
    crate::error::ensure!(status.success(), "Couldn't back up the key before removing it");
    Ok(())
}

fn remove_one(item: &Leftover, backup_dir: &Path) -> anyhow::Result<()> {
    match item.kind {
        LeftoverKind::Folder | LeftoverKind::File => {
            let path = Path::new(&item.path);
            crate::error::ensure!(!paths::is_protected(path), "This location is protected");
            if path.exists() {
                trash::delete(path).map_err(|e| anyhow::anyhow!("{e}"))?;
            }
        }
        LeftoverKind::RegistryKey => {
            let (label, hive, sub) = split_reg_path(&item.path)?;
            crate::error::ensure!(!is_protected_reg_key(&sub), "This registry key is protected");
            if hive.open_subkey_with_flags(&sub, KEY_READ | KEY_WOW64_64KEY).is_err() {
                return Ok(()); // already gone
            }
            backup_key(backup_dir, label, &sub)?;
            let (parent, leaf) = sub.rsplit_once('\\').context("Invalid registry path")?;
            let parent_key = hive.open_subkey_with_flags(parent, KEY_ALL_ACCESS | KEY_WOW64_64KEY).map_err(|e| access_error(e, label))?;
            parent_key.delete_subkey_all(leaf).map_err(|e| access_error(e, label))?;
        }
        LeftoverKind::RegistryValue => {
            let (label, hive, sub) = split_reg_path(&item.path)?;
            crate::error::ensure!(is_run_key(&sub) || !is_protected_reg_key(&sub), "This registry key is protected");
            let value = item.value_name.as_deref().context("Missing value name")?;
            backup_key(backup_dir, label, &sub)?;
            let key = hive.open_subkey_with_flags(&sub, KEY_ALL_ACCESS | KEY_WOW64_64KEY).map_err(|e| access_error(e, label))?;
            match key.delete_value(value) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(access_error(e, label)),
                _ => {}
            }
        }
    }
    Ok(())
}

fn access_error(e: std::io::Error, label: &str) -> anyhow::Error {
    if e.kind() == std::io::ErrorKind::PermissionDenied && label == "HKLM" {
        anyhow::anyhow!("Needs administrator rights. Restart Reclaim as administrator to remove this.")
    } else {
        anyhow::anyhow!("{e}")
    }
}

/// Moves files to the Recycle Bin and deletes registry entries after
/// exporting a `.reg` backup of each one.
#[tauri::command]
pub async fn remove_leftovers(app: AppHandle, items: Vec<Leftover>) -> CmdResult<Vec<RemovalResult>> {
    let backup_dir = app.path().app_local_data_dir()?.join("registry-backups");
    let results = tauri::async_runtime::spawn_blocking(move || {
        items
            .iter()
            .map(|item| {
                let result = remove_one(item, &backup_dir);
                RemovalResult {
                    path: item.path.clone(),
                    value_name: item.value_name.clone(),
                    ok: result.is_ok(),
                    error: result.err().map(|e| format!("{e:#}")),
                }
            })
            .collect::<Vec<_>>()
    })
    .await?;
    Ok(results)
}

/// Folder holding `.reg` backups of removed registry entries.
#[tauri::command]
pub fn registry_backup_dir(app: AppHandle) -> CmdResult<String> {
    Ok(app.path().app_local_data_dir()?.join("registry-backups").to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_variants() {
        let v = NameVariants::new("NVIDIA PhysX System Software 9.26.0904", Some("NVIDIA Corporation"));
        assert!(v.matches_name("NVIDIA PhysX System Software"));
        assert!(v.matches_name("PhysX System Software"));
        assert!(v.matches_first_word("PhysX"));
        assert!(v.matches_publisher("NVIDIA Corporation"));
        assert!(v.matches_publisher("NVIDIA"));

        let t = NameVariants::new("Termius 10.2.1", Some("Termius Corporation"));
        assert!(t.matches_name("Termius"));
        assert!(!t.matches_name("Terminal"));

        let b = NameVariants::new("Brave", Some("Brave Software Inc"));
        assert!(b.matches_publisher("BraveSoftware"));
    }

    #[test]
    fn registry_protection() {
        assert!(is_protected_reg_key("Software"));
        assert!(is_protected_reg_key(r"Software\Microsoft"));
        assert!(is_protected_reg_key(r"Software\Microsoft\Windows"));
        assert!(is_protected_reg_key(r"Software\Microsoft\Windows\CurrentVersion\Run"));
        assert!(is_protected_reg_key(r"SOFTWARE\Classes\CLSID"));
        assert!(is_protected_reg_key(r"SOFTWARE\WOW6432Node\Microsoft"));
        assert!(!is_protected_reg_key(r"Software\Termius"));
        assert!(!is_protected_reg_key(r"Software\Microsoft\OneDrive"));
        assert!(!is_protected_reg_key(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Discord"));
        assert!(is_run_key(r"Software\Microsoft\Windows\CurrentVersion\Run"));
    }
}
