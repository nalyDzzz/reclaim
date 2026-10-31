//! Finds what an uninstaller left behind (folders, shortcuts, registry keys,
//! startup entries, services, scheduled tasks, firewall rules) and removes
//! the ones the person confirms.
//!
//! Matching is deliberately conservative: names must match exactly after
//! normalization, and every candidate carries a confidence level so the UI
//! only pre-selects the ones that are almost certainly safe.

mod classes;
mod registrations;
mod shortcuts;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Context;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::ERROR_CANCELLED;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_ALL_ACCESS, KEY_READ, KEY_WOW64_64KEY};
use winreg::{HKEY, RegKey};

use super::open_entry;
use crate::error::CmdResult;
use crate::paths::{self, known, normalize};
use crate::system::{is_elevated, shell_execute_and_wait};

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
    /// `path` is the service's name.
    Service,
    /// `path` is the task's full path, like `\Vendor\Updater`.
    ScheduledTask,
    /// `path` is the rule's ID in the firewall's registry key.
    FirewallRule,
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
    /// What to show instead of `path` when that's only an ID.
    #[serde(default)]
    label: Option<String>,
}

fn compact(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn is_version_token(t: &str) -> bool {
    let t = t.trim_start_matches(['v', 'V']);
    t.contains('.') && t.chars().all(|c| c.is_ascii_digit() || c == '.')
}

const NOISE_TOKENS: &[&str] = &["x64", "x86", "64-bit", "32-bit", "64bit", "32bit", "amd64", "arm64", "win64", "win32", "version"];
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
        self.items.push(Leftover { kind, path, value_name, size_bytes: 0, confidence, reason: reason.to_string(), label: None });
    }

    fn push_item(&mut self, item: Leftover) {
        let key = format!("{:?}|{}|{}", item.kind, item.path.to_lowercase(), item.value_name.as_deref().unwrap_or(""));
        if self.seen.insert(key) {
            self.items.push(item);
        }
    }

    fn push_registration(&mut self, found: registrations::Found) {
        self.push_item(Leftover {
            kind: found.kind,
            path: found.id,
            value_name: None,
            size_bytes: 0,
            confidence: found.confidence,
            reason: found.reason.to_string(),
            label: found.label,
        });
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

    const PROGRAM: &str = "Program folder named after the app";
    const DATA: &str = "App data folder named after the app";
    const SHARED: &str = "Shared files folder named after the app";
    let roots = [
        (k.program_files.clone(), Confidence::High, PROGRAM),
        (k.program_files_x86.clone(), Confidence::High, PROGRAM),
        (k.local_programs.clone(), Confidence::High, PROGRAM),
        (k.program_files.join("Common Files"), Confidence::Medium, SHARED),
        (k.program_files_x86.join("Common Files"), Confidence::Medium, SHARED),
        (k.program_data.clone(), Confidence::Medium, DATA),
        (k.app_data.clone(), Confidence::Medium, DATA),
        (k.local_app_data.clone(), Confidence::Medium, DATA),
        (k.user_profile.join(r"AppData\LocalLow"), Confidence::Medium, DATA),
        (k.temp.clone(), Confidence::Medium, "Temporary files folder named after the app"),
        // Apps often keep projects and saves here, so these are never pre-selected.
        (k.documents.clone(), Confidence::Low, "Folder in Documents named after the app. It may hold your own files"),
        (k.saved_games.clone(), Confidence::Low, "Saved games folder named after the app"),
    ];

    for (root, confidence, reason) in roots {
        let Ok(entries) = std::fs::read_dir(&root) else { continue };
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if names.matches_name(&name) {
                out.push_path(&entry.path(), confidence, reason);
            } else if names.matches_publisher(&name) {
                scan_publisher_folder(&entry.path(), names, confidence.min(Confidence::Medium), out);
            }
        }
    }

    // Developer and command-line tools keep settings and downloads (Ollama's
    // models, for one) in dot-folders in the profile. Only dot-folders: the
    // profile's other folders belong to the person.
    if let Ok(entries) = std::fs::read_dir(&k.user_profile) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') && entry.file_type().is_ok_and(|t| t.is_dir()) && names.matches_name(&name) {
                out.push_path(&entry.path(), Confidence::Medium, "Settings folder in your user folder");
            }
        }
    }

    shortcuts::find(names, &app_folders(q), out);
}

/// Looks inside a folder named after the publisher. `confidence` is the most
/// a match here can get; partial name matches always get Low.
fn scan_publisher_folder(folder: &Path, names: &NameVariants, confidence: Confidence, out: &mut Collector) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    let mut total = 0;
    let mut matched = Vec::new();
    for entry in entries.flatten() {
        total += 1;
        let name = entry.file_name().to_string_lossy().to_string();
        if names.matches_name(&name) {
            matched.push((entry.path(), true));
        } else if names.matches_first_word(&name) {
            matched.push((entry.path(), false));
        }
    }
    if total == 0 {
        out.push_path(folder, confidence, "Empty publisher folder");
    } else if !matched.is_empty() && matched.len() == total && matched.iter().all(|(_, exact)| *exact) {
        out.push_path(folder, confidence, "Publisher folder that only holds this app");
    } else {
        for (path, exact) in matched {
            out.push_path(&path, if exact { confidence } else { Confidence::Low }, "Folder inside the publisher's folder");
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

    // Startup entries, App Paths registrations, and Windows' own records that
    // point into the app's folder.
    let folders = app_folders(q);
    let points_inside = |data: &str| points_into(data, &folders);
    let run_keys = [
        ("HKCU", RegKey::predef(HKEY_CURRENT_USER), r"Software\Microsoft\Windows\CurrentVersion\Run"),
        ("HKLM", RegKey::predef(HKEY_LOCAL_MACHINE), r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run"),
        ("HKLM", RegKey::predef(HKEY_LOCAL_MACHINE), r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run"),
    ];
    for (label, hive, path) in &run_keys {
        let Ok(key) = hive.open_subkey_with_flags(path, KEY_READ | KEY_WOW64_64KEY) else { continue };
        for (value_name, _) in key.enum_values().flatten() {
            let data: String = key.get_value(&value_name).unwrap_or_default();
            let named = names.matches_name(&value_name).then_some(Confidence::Medium);
            if let Some(confidence) = points_inside(&data).max(named) {
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
            if let Some(confidence) = points_inside(&target) {
                out.push(LeftoverKind::RegistryKey, format!(r"{label}\{path}\{exe}"), None, confidence, "Registered app path");
            }
        }
    }
    for (label, hive, path) in PER_PROGRAM_TRACE_KEYS {
        let Ok(key) = RegKey::predef(*hive).open_subkey_with_flags(path, KEY_READ | KEY_WOW64_64KEY) else { continue };
        for (value_name, _) in key.enum_values().flatten() {
            if let Some(confidence) = points_inside(&value_name) {
                out.push(
                    LeftoverKind::RegistryValue,
                    format!(r"{label}\{path}"),
                    Some(value_name),
                    confidence,
                    "Windows' record of the app",
                );
            }
        }
    }
}

/// The app's folders, normalized, each with how sure Reclaim is that it's
/// the app's own: a registered folder whose name doesn't match the app may
/// be shared with others. A protected folder like Program Files itself
/// would match every app, so it never counts.
fn app_folders(q: &LeftoverQuery) -> Vec<(String, Confidence)> {
    let registered = q.install_location.as_ref().map(|l| {
        let related = super::related_to(Path::new(l.trim_matches('"')), &q.name, q.publisher.as_deref());
        (l, if related { Confidence::High } else { Confidence::Low })
    });
    registered
        .into_iter()
        .chain(q.inferred_location.as_ref().map(|l| (l, Confidence::High)))
        .map(|(l, confidence)| (PathBuf::from(l.trim_matches('"')), confidence))
        .filter(|(l, _)| !paths::is_protected(l))
        .map(|(l, confidence)| (normalize(&l), confidence))
        .collect()
}

/// How sure Reclaim is that `text` (a command line or path) points into the
/// app, going by the surest of `folders` it mentions. None if it mentions none.
fn points_into(text: &str, folders: &[(String, Confidence)]) -> Option<Confidence> {
    folders.iter().filter(|(folder, _)| mentions_path_inside(text, folder)).map(|(_, confidence)| *confidence).max()
}

/// Keys where Windows records facts about each program it has run, one value
/// per program, named by the program's full path.
const PER_PROGRAM_TRACE_KEYS: &[(&str, HKEY, &str)] = &[
    ("HKCU", HKEY_CURRENT_USER, r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\MuiCache"),
    ("HKCU", HKEY_CURRENT_USER, r"Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Compatibility Assistant\Store"),
    ("HKCU", HKEY_CURRENT_USER, r"Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers"),
    ("HKLM", HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers"),
];

/// Whether `text` (a command line or path) mentions a path inside the
/// normalized `folder`. The folder name has to end there, so a mention of
/// `C:\Apps\Foobar` isn't inside `C:\Apps\Foo`.
fn mentions_path_inside(text: &str, folder: &str) -> bool {
    let text = text.to_lowercase().replace('/', "\\");
    text.match_indices(folder).any(|(at, _)| {
        text[at + folder.len()..].chars().next().is_none_or(|c| matches!(c, '\\' | '"' | '\'' | ',' | ';') || c.is_whitespace())
    })
}

#[tauri::command]
pub async fn scan_leftovers(query: LeftoverQuery) -> CmdResult<Vec<Leftover>> {
    let items = tauri::async_runtime::spawn_blocking(move || {
        let names = NameVariants::new(&query.name, query.publisher.as_deref());
        let mut out = Collector { items: Vec::new(), seen: HashSet::new() };
        scan_filesystem(&query, &names, &mut out);
        scan_registry(&query, &names, &mut out);
        let app_folders = app_folders(&query);
        for found in registrations::find(&app_folders) {
            out.push_registration(found);
        }
        for item in classes::find(&app_folders) {
            out.push_item(item);
        }
        for item in classes::registered_applications(&app_folders, &out.items) {
            out.push_item(item);
        }

        // Drop entries inside a folder or registry key that's already listed.
        let folders: Vec<String> =
            out.items.iter().filter(|i| i.kind == LeftoverKind::Folder).map(|i| normalize(Path::new(&i.path))).collect();
        let keys: Vec<String> = out.items.iter().filter(|i| i.kind == LeftoverKind::RegistryKey).map(|i| i.path.to_lowercase()).collect();
        let inside = |path: &str, parents: &[String], or_equal: bool| {
            parents.iter().any(|parent| (or_equal || path != parent) && (path == parent || path.starts_with(&format!("{parent}\\"))))
        };
        out.items.retain(|i| match i.kind {
            LeftoverKind::Folder | LeftoverKind::File => !inside(&normalize(Path::new(&i.path)), &folders, false),
            LeftoverKind::RegistryKey => !inside(&i.path.to_lowercase(), &keys, false),
            LeftoverKind::RegistryValue => !inside(&i.path.to_lowercase(), &keys, true),
            _ => true,
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
    /// Failed only because the Recycle Bin can't take it, so deleting it
    /// permanently would work.
    too_big_for_recycle_bin: bool,
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

/// Protected keys whose values each describe one program. Single values may
/// be removed from these, but never the key itself.
fn is_per_program_value_key(sub: &str) -> bool {
    let p = reg_parts(sub).join("\\");
    p == r"software\microsoft\windows\currentversion\run"
        || p == r"software\wow6432node\microsoft\windows\currentversion\run"
        || PER_PROGRAM_TRACE_KEYS.iter().any(|(_, _, key)| p == key.to_lowercase())
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

/// A new file in `dir` to back `what` up into, named so backups sort by time.
fn backup_file(dir: &Path, label: &str, what: &str, extension: &str) -> anyhow::Result<PathBuf> {
    // Names get cut short, so two backups in the same second could
    // otherwise share one.
    static COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    std::fs::create_dir_all(dir)?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // ASCII only, so cutting it to length can't split a character.
    let safe: String = what.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    Ok(dir.join(format!("{stamp}_{n:04}_{label}_{}.{extension}", &safe[safe.len().saturating_sub(80)..])))
}

/// Writes `text` as UTF-16 with a byte order mark: what `.reg` files use,
/// and what task XML declares.
fn write_utf16(path: &Path, text: &str) -> std::io::Result<()> {
    let bytes: Vec<u8> = std::iter::once(0xFEFF).chain(text.encode_utf16()).flat_map(u16::to_le_bytes).collect();
    std::fs::write(path, bytes)
}

/// A value as regedit writes it in a `.reg` file.
fn reg_data(value: &winreg::RegValue) -> String {
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(",");
    let text = || {
        let units: Vec<u16> = value.bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units).trim_end_matches('\0').to_string()
    };
    match value.vtype {
        // A quoted string can't hold line breaks, so those keep their bytes.
        winreg::enums::REG_SZ if !text().contains(['\r', '\n', '\0']) => format!("\"{}\"", reg_escape(&text())),
        winreg::enums::REG_DWORD if value.bytes.len() == 4 => {
            format!("dword:{:08x}", u32::from_le_bytes([value.bytes[0], value.bytes[1], value.bytes[2], value.bytes[3]]))
        }
        winreg::enums::REG_BINARY => format!("hex:{}", hex(&value.bytes)),
        ref other => format!("hex({:x}):{}", other.clone() as u32, hex(&value.bytes)),
    }
}

fn reg_escape(text: &str) -> String {
    text.replace('\\', r"\\").replace('"', "\\\"")
}

/// Appends `key` and everything under it to `out` in regedit's format.
fn export_key(key: &RegKey, path: &str, out: &mut String) {
    out.push_str(&format!("[{path}]\r\n"));
    for (name, value) in key.enum_values().flatten() {
        let name = if name.is_empty() { "@".to_string() } else { format!("\"{}\"", reg_escape(&name)) };
        out.push_str(&format!("{name}={}\r\n", reg_data(&value)));
    }
    out.push_str("\r\n");
    for sub in key.enum_keys().flatten() {
        if let Ok(child) = key.open_subkey_with_flags(&sub, KEY_READ | KEY_WOW64_64KEY) {
            export_key(&child, &format!(r"{path}\{sub}"), out);
        }
    }
}

/// Saves a key and everything under it as a `.reg` file, which restores it
/// when opened. Written directly rather than by `reg export`: that takes a
/// tenth of a second a key, and an app can leave hundreds.
fn backup_key(backup_dir: &Path, label: &str, sub: &str) -> anyhow::Result<()> {
    let (hive, root) = if label == "HKCU" { (HKEY_CURRENT_USER, "HKEY_CURRENT_USER") } else { (HKEY_LOCAL_MACHINE, "HKEY_LOCAL_MACHINE") };
    let failed = "Couldn't back up the key before removing it";
    let key = RegKey::predef(hive).open_subkey_with_flags(sub, KEY_READ | KEY_WOW64_64KEY).context(failed)?;
    let mut reg = String::from("Windows Registry Editor Version 5.00\r\n\r\n");
    export_key(&key, &format!(r"{root}\{sub}"), &mut reg);
    write_utf16(&backup_file(backup_dir, label, sub, "reg")?, &reg).context(failed)
}

/// Why Windows refused a removal, when Reclaim can still do something about it.
#[derive(Debug)]
enum Refused {
    /// An elevated copy of Reclaim may be allowed to make it.
    Denied(&'static str),
    /// Too large for the Recycle Bin on its drive. Deleting permanently works.
    TooBigForRecycleBin(&'static str),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::Denied(message) | Refused::TooBigForRecycleBin(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for Refused {}

fn was_denied(e: &anyhow::Error) -> bool {
    matches!(e.downcast_ref::<Refused>(), Some(Refused::Denied(_)))
}

fn was_too_big(e: &anyhow::Error) -> bool {
    matches!(e.downcast_ref::<Refused>(), Some(Refused::TooBigForRecycleBin(_)))
}

/// The shell reports every failure the same way, so this can't say which.
const RECYCLE_FAILED: &str =
    "Windows couldn't move this to the Recycle Bin. A program may be using it, or it may be too large for the Recycle Bin.";

/// How many bytes the Recycle Bin on `path`'s drive holds, or 0 when it's
/// turned off. None when Windows has no settings recorded for the drive.
fn recycle_bin_capacity(path: &Path) -> Option<u64> {
    use windows::Win32::Storage::FileSystem::{GetVolumeNameForVolumeMountPointW, GetVolumePathNameW};
    use windows::core::{HSTRING, PCWSTR};
    let mut mount = [0u16; 261];
    unsafe { GetVolumePathNameW(&HSTRING::from(path.as_os_str()), &mut mount) }.ok()?;
    let mut volume = [0u16; 64];
    unsafe { GetVolumeNameForVolumeMountPointW(PCWSTR(mount.as_ptr()), &mut volume) }.ok()?;
    let volume = String::from_utf16_lossy(&volume[..volume.iter().position(|&c| c == 0)?]);
    let guid = volume.strip_prefix(r"\\?\Volume")?.trim_end_matches('\\');
    let settings = format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket\Volume\{guid}");
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(settings, KEY_READ).ok()?;
    if key.get_value::<u32, _>("NukeOnDelete").is_ok_and(|nuke| nuke != 0) {
        return Some(0);
    }
    key.get_value::<u32, _>("MaxCapacity").ok().map(|mb| u64::from(mb) << 20)
}

fn delete_permanently(path: &Path) -> anyhow::Result<()> {
    // Neither follows a junction or symlink; each removes only the link.
    let result = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
    result.map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            Refused::Denied("Windows denied access to part of this.").into()
        } else {
            anyhow::Error::from(e)
        }
    })
}

/// Backs up a registry entry, service, or task before it's removed. Files
/// need none, since they go to the Recycle Bin or are deleted on purpose.
fn back_up(item: &Leftover, dir: &Path) -> anyhow::Result<()> {
    match item.kind {
        LeftoverKind::Folder | LeftoverKind::File => Ok(()),
        LeftoverKind::RegistryKey | LeftoverKind::RegistryValue => {
            let (label, hive, sub) = split_reg_path(&item.path)?;
            if hive.open_subkey_with_flags(&sub, KEY_READ | KEY_WOW64_64KEY).is_err() {
                return Ok(()); // already gone
            }
            backup_key(dir, label, &sub)
        }
        LeftoverKind::Service | LeftoverKind::ScheduledTask | LeftoverKind::FirewallRule => {
            if registrations::exists(item) {
                registrations::backup(item, dir)
            } else {
                Ok(())
            }
        }
    }
}

/// Removes one leftover, to the Recycle Bin unless `permanent`. Callers
/// back it up with `back_up` first.
fn remove_one(item: &Leftover, permanent: bool) -> anyhow::Result<()> {
    match item.kind {
        LeftoverKind::Folder | LeftoverKind::File => {
            let path = Path::new(&item.path);
            crate::error::ensure!(!paths::is_protected(path), "This location is protected");
            if !path.exists() {
                return Ok(());
            }
            if permanent {
                return delete_permanently(path);
            }
            // The shell would refuse, or with any UI allowed, offer to delete it permanently itself.
            match recycle_bin_capacity(path) {
                Some(0) => return Err(Refused::TooBigForRecycleBin("The Recycle Bin is turned off for this drive.").into()),
                Some(capacity) if item.size_bytes > capacity => {
                    return Err(Refused::TooBigForRecycleBin("Too large for the Recycle Bin on this drive.").into());
                }
                _ => trash::delete(path).map_err(|_| Refused::Denied(RECYCLE_FAILED))?,
            }
        }
        LeftoverKind::Service | LeftoverKind::ScheduledTask | LeftoverKind::FirewallRule => registrations::remove(item)?,
        LeftoverKind::RegistryKey => {
            let (_, hive, sub) = split_reg_path(&item.path)?;
            if hive.open_subkey_with_flags(&sub, KEY_READ | KEY_WOW64_64KEY).is_err() {
                return Ok(()); // already gone
            }
            let allowed = !is_protected_reg_key(&sub) || classes::may_remove_key(&hive, &sub);
            crate::error::ensure!(allowed, "This registry key is protected");
            let (parent, leaf) = sub.rsplit_once('\\').context("Invalid registry path")?;
            let parent_key = hive.open_subkey_with_flags(parent, KEY_ALL_ACCESS | KEY_WOW64_64KEY).map_err(access_error)?;
            parent_key.delete_subkey_all(leaf).map_err(access_error)?;
        }
        LeftoverKind::RegistryValue => {
            let (_, hive, sub) = split_reg_path(&item.path)?;
            if hive.open_subkey_with_flags(&sub, KEY_READ | KEY_WOW64_64KEY).is_err() {
                return Ok(()); // already gone, key and all
            }
            let allowed = is_per_program_value_key(&sub) || classes::may_remove_value(&sub) || !is_protected_reg_key(&sub);
            crate::error::ensure!(allowed, "This registry key is protected");
            let value = item.value_name.as_deref().context("Missing value name")?;
            let key = hive.open_subkey_with_flags(&sub, KEY_ALL_ACCESS | KEY_WOW64_64KEY).map_err(access_error)?;
            match key.delete_value(value) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(access_error(e)),
                _ => {}
            }
        }
    }
    Ok(())
}

fn access_error(e: std::io::Error) -> anyhow::Error {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        Refused::Denied("Windows denied access to this registry entry.").into()
    } else {
        e.into()
    }
}

/// Whether `item` is in HKLM or outside every user profile, where removing it
/// can take administrator rights. Only these go to the elevated copy: it may
/// run as another account, whose HKCU and profile weren't the ones scanned.
fn is_machine_wide(item: &Leftover) -> bool {
    match item.kind {
        LeftoverKind::Folder | LeftoverKind::File => {
            let profile = &known().user_profile;
            let profiles = profile.parent().filter(|p| p.parent().is_some()).unwrap_or(profile);
            !paths::is_under(&normalize(&paths::long_path(Path::new(&item.path))), &normalize(profiles))
        }
        LeftoverKind::RegistryKey | LeftoverKind::RegistryValue => split_reg_path(&item.path).is_ok_and(|(label, ..)| label == "HKLM"),
        LeftoverKind::Service | LeftoverKind::ScheduledTask | LeftoverKind::FirewallRule => true,
    }
}

fn still_present(item: &Leftover) -> bool {
    use std::io::ErrorKind::NotFound;
    match item.kind {
        LeftoverKind::Folder | LeftoverKind::File => std::fs::symlink_metadata(&item.path).is_ok(),
        LeftoverKind::Service | LeftoverKind::ScheduledTask | LeftoverKind::FirewallRule => registrations::exists(item),
        LeftoverKind::RegistryKey | LeftoverKind::RegistryValue => {
            let Ok((_, hive, sub)) = split_reg_path(&item.path) else { return true };
            let key = match hive.open_subkey_with_flags(&sub, KEY_READ | KEY_WOW64_64KEY) {
                Ok(key) => key,
                Err(e) => return e.kind() != NotFound,
            };
            match (item.kind, &item.value_name) {
                (LeftoverKind::RegistryValue, Some(value)) => key.get_raw_value(value).map_or_else(|e| e.kind() != NotFound, |_| true),
                _ => true,
            }
        }
    }
}

/// Removes `items` in an elevated copy of Reclaim, which shows a UAC prompt,
/// the same way an uninstaller gets its rights. The copy reports nothing
/// back, so callers check afterwards what's gone.
fn remove_elevated(items: &[&Leftover], permanent: bool) -> anyhow::Result<()> {
    use crate::launch::{LIST_SHA256, PERMANENTLY, REMOVE_LEFTOVERS};
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let list = std::env::temp_dir().join(format!("reclaim-leftovers-{}-{nanos}.json", std::process::id()));
    let data = serde_json::to_vec(items)?;
    std::fs::write(&list, &data)?;
    let exe = std::env::current_exe()?;
    let mut params = format!(r#"{REMOVE_LEFTOVERS} "{}" {LIST_SHA256} {}"#, list.display(), sha256_hex(&data));
    if permanent {
        params = format!("{params} {PERMANENTLY}");
    }
    let launched = shell_execute_and_wait(Some("runas"), &exe.to_string_lossy(), &params);
    let _ = std::fs::remove_file(&list);
    match launched {
        Err(err) if err.code() == ERROR_CANCELLED.to_hresult() => {
            anyhow::bail!("Needs administrator approval, and the prompt was declined.")
        }
        Err(err) => anyhow::bail!("Couldn't start Reclaim as administrator: {}", err.message()),
        Ok(_) => Ok(()),
    }
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// Reads the list `remove_elevated` wrote. It sits in the user's temp folder,
/// where anything running as them could rewrite it while the UAC prompt is
/// up, so it's only trusted if it matches the hash from the command line.
/// Parses the same bytes it hashed, so there's no second read to swap.
fn read_list(list: &Path, sha256: &str) -> Option<Vec<Leftover>> {
    let data = std::fs::read(list).ok()?;
    if !sha256_hex(&data).eq_ignore_ascii_case(sha256) {
        return None;
    }
    serde_json::from_slice(&data).ok()
}

/// What the elevated copy started by `remove_elevated` runs. It only takes
/// machine-wide items, and the usual protections still apply to each one.
pub fn remove_listed(list: &Path, sha256: &str, permanent: bool) {
    let Some(items) = read_list(list, sha256) else { return };
    for item in items.iter().filter(|item| is_machine_wide(item)) {
        // The copy that asked already backed these up: it only asks after a
        // backup succeeded and the removal was denied.
        let _ = remove_one(item, permanent);
    }
}

/// Backs every item up, then removes the ones whose backup worked, and
/// retries the machine-wide ones Windows refused in an elevated copy, so
/// one UAC prompt covers Program Files and HKLM.
fn remove_all(items: &[Leftover], backup_dir: &Path, permanent: bool) -> Vec<anyhow::Result<()>> {
    let backed_up: Vec<anyhow::Result<()>> = items.iter().map(|item| back_up(item, backup_dir)).collect();
    let mut outcomes: Vec<_> =
        items.iter().zip(backed_up).map(|(item, backup)| backup.and_then(|()| remove_one(item, permanent))).collect();
    if is_elevated() {
        return outcomes;
    }
    let retry: Vec<usize> =
        (0..items.len()).filter(|&i| outcomes[i].as_ref().is_err_and(was_denied) && is_machine_wide(&items[i])).collect();
    if retry.is_empty() {
        return outcomes;
    }
    let elevated = remove_elevated(&retry.iter().map(|&i| &items[i]).collect::<Vec<_>>(), permanent);
    for i in retry {
        let item = &items[i];
        outcomes[i] = match &elevated {
            Err(e) => Err(anyhow::anyhow!("{e}")),
            Ok(()) if !still_present(item) => Ok(()),
            Ok(()) => Err(anyhow::anyhow!(match item.kind {
                LeftoverKind::Folder | LeftoverKind::File if permanent => {
                    "Still there after trying as administrator. A program may be using it."
                }
                LeftoverKind::Folder | LeftoverKind::File => {
                    "Still there after trying as administrator. A program may be using it, or it may be too large for the Recycle Bin."
                }
                LeftoverKind::RegistryKey | LeftoverKind::RegistryValue => {
                    "Still there after trying as administrator. Windows may protect this entry."
                }
                LeftoverKind::Service => "Still there after trying as administrator. The service may not have stopped.",
                LeftoverKind::ScheduledTask | LeftoverKind::FirewallRule => "Still there after trying as administrator.",
            })),
        };
    }
    outcomes
}

/// Moves files to the Recycle Bin, or deletes them if `permanent`, and
/// removes registry entries, services, tasks, and firewall rules after
/// backing each one up.
#[tauri::command]
pub async fn remove_leftovers(app: AppHandle, items: Vec<Leftover>, permanent: bool) -> CmdResult<Vec<RemovalResult>> {
    let backup_dir = app.path().app_local_data_dir()?.join("registry-backups");
    let results = tauri::async_runtime::spawn_blocking(move || {
        let outcomes = remove_all(&items, &backup_dir, permanent);
        items
            .iter()
            .zip(outcomes)
            .map(|(item, result)| RemovalResult {
                path: item.path.clone(),
                value_name: item.value_name.clone(),
                ok: result.is_ok(),
                too_big_for_recycle_bin: result.as_ref().is_err_and(was_too_big),
                error: result.err().map(|e| format!("{e:#}")),
            })
            .collect::<Vec<_>>()
    })
    .await?;
    Ok(results)
}

/// Folder holding backups of removed registry entries, services, and tasks.
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

        let o = NameVariants::new("Ollama version 0.12.3", Some("Ollama"));
        assert!(o.matches_name(".ollama"));
        assert!(!o.matches_name("Version"));
    }

    #[test]
    fn path_mentions_stop_at_folder_boundaries() {
        let folder = normalize(Path::new(r"C:\Program Files\NTLite"));
        assert!(mentions_path_inside(r"C:\Program Files\NTLite\unins000.exe.FriendlyAppName", &folder));
        assert!(mentions_path_inside(r#""C:\Program Files\NTLite\NTLite.exe" /tray"#, &folder));
        assert!(mentions_path_inside(r"c:/program files/ntlite", &folder));
        assert!(!mentions_path_inside(r"C:\Program Files\NTLite2\NTLite.exe", &folder));
        assert!(!mentions_path_inside(r"C:\Users\me\Downloads\NTLite_setup_x64.exe", &folder));
    }

    #[test]
    fn a_folder_that_might_be_shared_never_pre_selects() {
        let query = |install: &str| LeftoverQuery {
            program_id: String::new(),
            name: "NTLite".into(),
            publisher: Some("Nlitesoft".into()),
            install_location: Some(install.into()),
            inferred_location: None,
        };
        let pf = &known().program_files;
        let ours = app_folders(&query(&pf.join(r"NTLite\").to_string_lossy()));
        assert_eq!(points_into(&format!(r"{}\NTLite\svc.exe", pf.display()), &ours), Some(Confidence::High));
        let shared = app_folders(&query(&pf.join("Tools").to_string_lossy()));
        assert_eq!(points_into(&format!(r"{}\Tools\other.exe", pf.display()), &shared), Some(Confidence::Low));
        // Program Files itself would match every app.
        assert!(app_folders(&query(&pf.to_string_lossy())).is_empty());
    }

    fn leftover(kind: LeftoverKind, path: &Path) -> Leftover {
        Leftover {
            kind,
            path: path.to_string_lossy().into(),
            value_name: None,
            size_bytes: 0,
            confidence: Confidence::High,
            reason: String::new(),
            label: None,
        }
    }

    #[test]
    fn only_machine_wide_items_need_elevation() {
        let k = known();
        assert!(is_machine_wide(&leftover(LeftoverKind::Folder, &k.program_files.join("NTLite"))));
        assert!(is_machine_wide(&leftover(LeftoverKind::Folder, &k.program_data.join("NTLite"))));
        assert!(!is_machine_wide(&leftover(LeftoverKind::Folder, &k.local_app_data.join("NTLite"))));
        assert!(!is_machine_wide(&leftover(LeftoverKind::Folder, &k.user_profile.join(".ollama"))));
        assert!(is_machine_wide(&leftover(LeftoverKind::RegistryKey, Path::new(r"HKLM\SOFTWARE\NTLite"))));
        assert!(!is_machine_wide(&leftover(LeftoverKind::RegistryKey, Path::new(r"HKCU\Software\NTLite"))));
        assert!(is_machine_wide(&leftover(LeftoverKind::Service, Path::new("NTLiteSvc"))));
    }

    #[test]
    fn elevated_helper_only_trusts_the_list_it_was_given() {
        let list = std::env::temp_dir().join(format!("reclaim-list-test-{}.json", std::process::id()));
        let data = serde_json::to_vec(&[leftover(LeftoverKind::Folder, &known().program_files.join("NTLite"))]).unwrap();
        let sha256 = sha256_hex(&data);
        std::fs::write(&list, &data).unwrap();
        let original = read_list(&list, &sha256).map(|items| items.len());
        let swapped = serde_json::to_vec(&[leftover(LeftoverKind::Folder, &known().program_files.join("Antivirus"))]).unwrap();
        std::fs::write(&list, swapped).unwrap();
        let tampered = read_list(&list, &sha256).map(|items| items.len());
        let _ = std::fs::remove_file(&list);
        assert_eq!(original, Some(1));
        assert_eq!(tampered, None, "a list rewritten after launch must be ignored");
    }

    #[test]
    fn elevated_helper_ignores_items_in_profiles() {
        let dir = std::env::temp_dir().join(format!("reclaim-helper-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let list = dir.with_extension("json");
        let data = serde_json::to_vec(&[leftover(LeftoverKind::Folder, &dir)]).unwrap();
        std::fs::write(&list, &data).unwrap();
        remove_listed(&list, &sha256_hex(&data), true);
        let survived = dir.exists();
        let _ = std::fs::remove_file(&list);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(survived, "the elevated helper must only remove machine-wide items");
    }

    #[test]
    fn refused_removal_can_be_retried_elevated() {
        use std::process::Command;
        for permanent in [false, true] {
            let dir = std::env::temp_dir().join(format!("reclaim-denied-test-{}-{permanent}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("file.bin"), b"x").unwrap();
            let user = std::env::var("USERNAME").unwrap();
            let acl = |args: &[&str]| Command::new("icacls").arg(&dir).args(args).output().unwrap().status.success();
            assert!(acl(&["/deny", &format!("{user}:(OI)(CI)(DE,DC)")]));
            let result = remove_one(&leftover(LeftoverKind::Folder, &dir), permanent);
            acl(&["/remove:d", &user, "/T"]);
            let _ = std::fs::remove_dir_all(&dir);
            let err = result.expect_err("removal should be refused");
            assert!(was_denied(&err), "permanent={permanent}: {err:#}");
        }
    }

    #[test]
    fn registry_backups_restore_exactly() {
        use winreg::RegValue;
        use winreg::enums::{KEY_ALL_ACCESS, REG_BINARY, REG_EXPAND_SZ, REG_SZ};
        let sub = format!(r"Software\ReclaimBackupTest{}", std::process::id());
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(&sub).unwrap();
        let utf16 = |s: &str| s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect::<Vec<u8>>();
        key.set_value("", &r#"C:\Program Files\App "quoted""#).unwrap();
        key.set_raw_value("Lines", &RegValue { bytes: utf16("one\r\ntwo").into(), vtype: REG_SZ }).unwrap();
        key.set_raw_value("Expand", &RegValue { bytes: utf16(r"%SystemRoot%\x.dll").into(), vtype: REG_EXPAND_SZ }).unwrap();
        key.set_value("Dword", &0xDEAD_BEEFu32).unwrap();
        key.set_value("Qword", &0x0123_4567_89AB_CDEFu64).unwrap();
        key.set_raw_value("Binary", &RegValue { bytes: vec![0, 1, 255].into(), vtype: REG_BINARY }).unwrap();
        key.set_value("Multi", &vec!["a".to_string(), "b c".to_string()]).unwrap();
        key.create_subkey(r"Child\Grandchild").unwrap().0.set_value("Leaf", &"x").unwrap();

        fn snapshot(key: &RegKey) -> Vec<String> {
            let mut out: Vec<String> = key.enum_values().flatten().map(|(n, v)| format!("{n}={:?}:{:?}", v.vtype, v.bytes)).collect();
            for sub in key.enum_keys().flatten() {
                out.extend(snapshot(&key.open_subkey(&sub).unwrap()).into_iter().map(|line| format!(r"{sub}\{line}")));
            }
            out.sort();
            out
        }
        let before = snapshot(&key);
        drop(key);
        let dir = std::env::temp_dir().join(format!("reclaim-backup-test-{}", std::process::id()));
        backup_key(&dir, "HKCU", &sub).unwrap();
        let file = std::fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path();
        hkcu.open_subkey_with_flags("Software", KEY_ALL_ACCESS).unwrap().delete_subkey_all(sub.split('\\').nth(1).unwrap()).unwrap();
        let imported = std::process::Command::new(paths::system32("reg.exe")).arg("import").arg(&file).output().unwrap();
        let after = hkcu.open_subkey(&sub).map(|k| snapshot(&k));
        let _ = hkcu.delete_subkey_all(&sub);
        let _ = std::fs::remove_dir_all(&dir);

        assert!(imported.status.success(), "{}", String::from_utf8_lossy(&imported.stderr));
        assert_eq!(after.unwrap(), before);
    }

    #[test]
    fn permanent_removal_skips_the_recycle_bin() {
        let dir = std::env::temp_dir().join(format!("reclaim-permanent-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join(r"nested\file.bin"), b"x").unwrap();
        let result = remove_one(&leftover(LeftoverKind::Folder, &dir), true);
        let survived = dir.exists();
        let _ = std::fs::remove_dir_all(&dir);
        result.unwrap();
        assert!(!survived);
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
        assert!(is_per_program_value_key(r"Software\Microsoft\Windows\CurrentVersion\Run"));
        let mui_cache = r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\MuiCache";
        assert!(is_per_program_value_key(mui_cache));
        assert!(is_protected_reg_key(mui_cache), "values can go, but never the whole key");
        assert!(!is_per_program_value_key(r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell"));
    }
}
