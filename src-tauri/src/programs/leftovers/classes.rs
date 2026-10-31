//! What an app registers in the registry's Classes and the shell's lists:
//! its COM components (shell extensions) and everything that loads them,
//! like right-click handlers, thumbnail and preview handlers, and icon
//! overlays; the right-click entries it adds to files, folders, and drives;
//! its file types and link protocols, with their "Open with" and "New"
//! entries; and its Default apps registration.
//!
//! All of this sits among Windows' own, so a registration only counts when
//! what it runs is in the app's folder and nothing in it names Windows'
//! files. That second half is what gives away a Windows file type whose
//! command an app took over: `txtfile` pointing at an editor still has
//! Windows' icon. Removal re-checks it, since Classes is otherwise protected.

use std::collections::HashMap;

use rayon::prelude::*;
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY, REG_EXPAND_SZ, REG_SZ};
use winreg::types::FromRegValue;

use super::{Confidence, Leftover, LeftoverKind, points_into, reg_parts};
use crate::paths::{expand_env, known, normalize};
use crate::programs::split_command;

/// HKCU or HKLM. Holds no handle, so scans can share it across threads.
#[derive(Clone, Copy)]
struct Root {
    label: &'static str,
    software: &'static str,
}

impl Root {
    fn open(self, path: &str) -> Option<RegKey> {
        let hive = if self.label == "HKCU" { HKEY_CURRENT_USER } else { HKEY_LOCAL_MACHINE };
        RegKey::predef(hive).open_subkey_with_flags(path, KEY_READ | KEY_WOW64_64KEY).ok()
    }

    fn key(self, sub: String, confidence: Confidence, reason: &str) -> Leftover {
        self.item(LeftoverKind::RegistryKey, sub, None, confidence, reason)
    }

    fn value(self, sub: String, value: String, confidence: Confidence, reason: &str) -> Leftover {
        self.item(LeftoverKind::RegistryValue, sub, Some(value), confidence, reason)
    }

    fn item(self, kind: LeftoverKind, sub: String, value: Option<String>, confidence: Confidence, reason: &str) -> Leftover {
        let path = format!(r"{}\{sub}", self.label);
        Leftover { kind, path, value_name: value, size_bytes: 0, confidence, reason: reason.to_string(), label: None }
    }
}

const ROOTS: [Root; 2] = [Root { label: "HKCU", software: "Software" }, Root { label: "HKLM", software: "SOFTWARE" }];

/// Classes Windows defines for files, folders, and drives in general. Apps
/// add menu entries and handlers under them; the classes themselves stay.
const SHELL_CLASSES: &[&str] = &[
    "*",
    "AllFilesystemObjects",
    "Directory",
    r"Directory\Background",
    "Folder",
    "Drive",
    "DesktopBackground",
    "LibraryFolder",
    r"LibraryFolder\Background",
];

/// Verbs Windows puts on classes itself. An app that overwrote one left
/// nothing to go back to, so these always stay.
const WINDOWS_VERBS: &[&str] = &[
    "open",
    "opennewwindow",
    "opennewprocess",
    "opennewtab",
    "explore",
    "find",
    "edit",
    "print",
    "printto",
    "runas",
    "runasuser",
    "properties",
    "cmd",
    "powershell",
    "pintohome",
    "play",
    "preview",
    "openas",
];

/// Top-level Classes keys that are never one app's file type.
const WINDOWS_CLASSES: &[&str] = &[
    "clsid",
    "interface",
    "typelib",
    "appid",
    "applications",
    "systemfileassociations",
    "mime",
    "media type",
    "unknown",
    "exefile",
    "lnkfile",
    "batfile",
    "cmdfile",
    "comfile",
    "piffile",
    "scrfile",
    "regfile",
    "inffile",
    "dllfile",
    "sysfile",
    "network",
    "printers",
    "internetshortcut",
    "http",
    "https",
    "ftp",
    "mailto",
    "file",
    "wow6432node",
    "local settings",
    "directshow",
    "protocols",
    "installer",
];

/// Subkeys of a file extension that hold its settings rather than name a file type.
const EXTENSION_PARTS: &[&str] = &["openwithprogids", "openwithlist", "shellnew", "shell", "shellex", "persistenthandler"];

const OVERLAYS: &str = r"Microsoft\Windows\CurrentVersion\Explorer\ShellIconOverlayIdentifiers";
const APPROVED: &str = r"Microsoft\Windows\CurrentVersion\Shell Extensions\Approved";
const PREVIEW_HANDLERS: &str = r"Microsoft\Windows\CurrentVersion\PreviewHandlers";

fn is_windows_class(name: &str) -> bool {
    WINDOWS_CLASSES.contains(&name.to_lowercase().as_str()) || SHELL_CLASSES.iter().any(|c| c.eq_ignore_ascii_case(name))
}

fn is_windows_verb(verb: &str) -> bool {
    WINDOWS_VERBS.contains(&verb.to_lowercase().as_str())
}

/// A string value of `key` or of its subkey `sub`, if it's set.
fn string(key: &RegKey, sub: &str, name: &str) -> Option<String> {
    let value: String =
        if sub.is_empty() { key.get_value(name).ok()? } else { key.open_subkey_with_flags(sub, KEY_READ).ok()?.get_value(name).ok()? };
    let value = value.trim().trim_end_matches('\0').trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn mentions_windows(text: &str) -> bool {
    let text = expand_env(text).to_lowercase().replace('/', "\\");
    let k = known();
    let windows = normalize(&k.windows);
    text.contains(&format!("{windows}\\"))
        || text.ends_with(&windows)
        || [&k.program_files, &k.program_files_x86].iter().any(|pf| {
            let pf = normalize(pf);
            ["windows ", "windowsapps\\", "internet explorer\\"].iter().any(|d| text.contains(&format!("{pf}\\{d}")))
        })
}

/// Whether a file reference names one of Windows' files: a bare name like
/// `shell32.dll`, which Windows finds in System32, or a path in Windows'
/// folders. Takes commands (`"C:\x.exe" %1`), icons (`x.dll,-12`), and
/// resource strings (`@x.dll,-12`).
fn names_windows_file(reference: &str) -> bool {
    let expanded = expand_env(reference.trim().trim_start_matches('@'));
    let file = split_command(&expanded).0;
    !file.contains(['\\', '/']) || mentions_windows(&expanded)
}

fn inside(reference: &str, folders: &[(String, Confidence)]) -> Option<Confidence> {
    points_into(&expand_env(reference), folders)
}

fn is_guid(text: &str) -> bool {
    text.len() == 38 && text.starts_with('{') && text.ends_with('}')
}

/// What a file type, link protocol, or Applications entry runs and shows.
struct ClassFiles {
    /// Each verb's command, empty for a verb that has none.
    commands: Vec<String>,
    /// Its icon, and resource strings like `@app.exe,-12`.
    others: Vec<String>,
}

impl ClassFiles {
    fn read(class: &RegKey) -> Self {
        let mut commands = Vec::new();
        if let Ok(shell) = class.open_subkey_with_flags("shell", KEY_READ) {
            for verb in shell.enum_keys().flatten() {
                commands.push(string(&shell, &format!(r"{verb}\command"), "").unwrap_or_default());
            }
        }
        let mut others: Vec<String> = string(class, "DefaultIcon", "").into_iter().collect();
        others.extend(
            class
                .enum_values()
                .flatten()
                .filter(|(_, v)| matches!(v.vtype, REG_SZ | REG_EXPAND_SZ))
                .filter_map(|(_, v)| String::from_reg_value(&v).ok())
                .filter(|s| s.starts_with('@')),
        );
        ClassFiles { commands, others }
    }

    fn names_windows(&self) -> bool {
        self.commands.iter().filter(|c| !c.is_empty()).chain(&self.others).any(|f| names_windows_file(f))
    }

    /// How sure Reclaim is that the app owns it: every command and icon in
    /// the app's folder, and nothing of Windows'.
    fn owned(&self, folders: &[(String, Confidence)]) -> Option<Confidence> {
        if self.commands.is_empty() || self.names_windows() {
            return None;
        }
        self.commands.iter().chain(&self.others).map(|f| inside(f, folders)).collect::<Option<Vec<_>>>()?.into_iter().min()
    }

    /// The removal-time check, which can't know the app's folder: it runs
    /// something, and nothing of it is Windows'.
    fn removable(&self) -> bool {
        self.commands.iter().any(|c| !c.is_empty()) && !self.names_windows()
    }
}

/// The files a COM class loads: its DLL or program, and for .NET classes the assembly.
fn class_servers(clsid: &RegKey) -> Vec<String> {
    [("InprocServer32", ""), ("InprocServer32", "CodeBase"), ("LocalServer32", "")]
        .iter()
        .filter_map(|(sub, name)| string(clsid, sub, name))
        .collect()
}

fn clsid_owned(clsid: &RegKey, folders: &[(String, Confidence)]) -> Option<Confidence> {
    let servers = class_servers(clsid);
    if servers.is_empty() || servers.iter().any(|s| names_windows_file(s)) {
        return None;
    }
    servers.iter().map(|s| inside(s, folders)).collect::<Option<Vec<_>>>()?.into_iter().min()
}

fn clsid_removable(clsid: &RegKey) -> bool {
    let servers = class_servers(clsid);
    !servers.is_empty() && !servers.iter().any(|s| names_windows_file(s))
}

fn clsid_containers(root: Root) -> [String; 2] {
    [format!(r"{}\Classes\CLSID", root.software), format!(r"{}\Classes\WOW6432Node\CLSID", root.software)]
}

/// Whether every registration of the COM class may go, or there's none left.
fn clsid_removable_anywhere(guid: &str) -> bool {
    ROOTS.iter().all(|root| {
        clsid_containers(*root).iter().all(|container| root.open(&format!(r"{container}\{guid}")).is_none_or(|k| clsid_removable(&k)))
    })
}

/// Whether a file type named in an extension may go: it's gone already, or
/// every definition of it is removable.
fn progid_removable(progid: &str) -> bool {
    !is_windows_class(progid)
        && ROOTS
            .iter()
            .all(|root| root.open(&format!(r"{}\Classes\{progid}", root.software)).is_none_or(|k| ClassFiles::read(&k).removable()))
}

/// What the app owns, found first because everything else refers to it.
struct Owned<'a> {
    folders: &'a [(String, Confidence)],
    /// The app's COM classes and file types, lowercased, with how sure Reclaim is of each.
    clsids: HashMap<String, Confidence>,
    progids: HashMap<String, Confidence>,
}

impl Owned<'_> {
    fn clsid(&self, guid: &str) -> Option<Confidence> {
        self.clsids.get(&guid.to_lowercase()).copied()
    }

    fn progid(&self, name: &str) -> Option<Confidence> {
        self.progids.get(&name.to_lowercase()).copied()
    }
}

/// The names of `path`'s subkeys.
fn subkeys(root: Root, path: &str) -> Vec<String> {
    root.open(path).map(|k| k.enum_keys().flatten().collect()).unwrap_or_default()
}

/// COM classes whose files are in the app's folder, with their GUIDs.
fn com_classes(root: Root, folders: &[(String, Confidence)]) -> Vec<(String, Leftover)> {
    clsid_containers(root)
        .iter()
        .flat_map(|container| subkeys(root, container).into_iter().map(move |guid| (container.clone(), guid)))
        .collect::<Vec<_>>()
        .into_par_iter()
        .filter_map(|(container, guid)| {
            let clsid = root.open(&format!(r"{container}\{guid}"))?;
            let confidence = clsid_owned(&clsid, folders)?;
            let mut item =
                root.key(format!(r"{container}\{guid}"), confidence, "COM component from the app's folder, like a shell extension");
            item.label = string(&clsid, "", "").filter(|name| !name.starts_with('@'));
            Some((guid, item))
        })
        .collect()
}

/// File types, link protocols, and "Open with" entries the app owns outright, with their names.
fn file_types(root: Root, folders: &[(String, Confidence)]) -> Vec<(Option<String>, Leftover)> {
    let classes = format!(r"{}\Classes", root.software);
    let types =
        subkeys(root, &classes).into_par_iter().filter(|name| !name.starts_with('.') && !is_windows_class(name)).filter_map(|name| {
            let confidence = ClassFiles::read(&root.open(&format!(r"{classes}\{name}"))?).owned(folders)?;
            let item = root.key(format!(r"{classes}\{name}"), confidence, "File type or link protocol that opens with the app");
            Some((Some(name), item))
        });
    let apps = subkeys(root, &format!(r"{classes}\Applications")).into_par_iter().filter_map(|exe| {
        let path = format!(r"{classes}\Applications\{exe}");
        let confidence = ClassFiles::read(&root.open(&path)?).owned(folders)?;
        Some((None, root.key(path, confidence, "\"Open with\" entry for the app")))
    });
    types.chain(apps).collect()
}

/// Right-click entries and shell handlers the app added under `holder`, a
/// class that Windows or another app owns.
fn menu_entries(root: Root, holder: &str, owned: &Owned) -> Vec<Leftover> {
    let path = format!(r"{}\Classes\{holder}", root.software);
    let mut found = Vec::new();
    let Some(class) = root.open(&path) else { return found };
    if let Ok(shell) = class.open_subkey_with_flags("shell", KEY_READ) {
        for verb in shell.enum_keys().flatten() {
            if is_windows_verb(&verb) {
                continue;
            }
            let Ok(entry) = shell.open_subkey_with_flags(&verb, KEY_READ) else { continue };
            let confidence = match (string(&entry, "command", ""), string(&entry, "", "ExplorerCommandHandler")) {
                (Some(command), _) => (!names_windows_file(&command)).then(|| inside(&command, owned.folders)).flatten(),
                (None, Some(handler)) => owned.clsid(&handler),
                (None, None) => None,
            };
            if let Some(confidence) = confidence {
                found.push(root.key(format!(r"{path}\shell\{verb}"), confidence, "Right-click menu entry that runs the app"));
            }
        }
    }
    let Ok(shellex) = class.open_subkey_with_flags("shellex", KEY_READ) else { return found };
    for kind in shellex.enum_keys().flatten() {
        let Ok(handlers) = shellex.open_subkey_with_flags(&kind, KEY_READ) else { continue };
        // `.ext\shellex\{handler type}` names its handler as its own value.
        if let Some(confidence) = string(&handlers, "", "").and_then(|guid| owned.clsid(&guid)) {
            found.push(root.key(format!(r"{path}\shellex\{kind}"), confidence, "Thumbnail, preview, or other handler from the app"));
            continue;
        }
        // `*\shellex\ContextMenuHandlers\<name>` names it as its value, or as its own name.
        for name in handlers.enum_keys().flatten() {
            let guid = string(&handlers, &name, "").unwrap_or_else(|| name.clone());
            if let Some(confidence) = owned.clsid(&guid) {
                found.push(root.key(format!(r"{path}\shellex\{kind}\{name}"), confidence, "Shell extension registration for the app"));
            }
        }
    }
    found
}

/// A file extension the app's file types use. Taken whole when it names
/// nothing else; otherwise just the app's "Open with" and "New" entries.
fn extension(root: Root, ext: &str, owned: &Owned) -> Vec<Leftover> {
    let path = format!(r"{}\Classes\{ext}", root.software);
    let Some(key) = root.open(&path) else { return Vec::new() };
    let open_with: Vec<String> = key
        .open_subkey_with_flags("OpenWithProgids", KEY_READ)
        .map(|k| k.enum_values().flatten().map(|(name, _)| name).filter(|name| !name.is_empty()).collect())
        .unwrap_or_default();
    let subkeys: Vec<String> = key.enum_keys().flatten().collect();
    let only_default = key.enum_values().flatten().all(|(name, _)| name.is_empty());
    let default = string(&key, "", "");

    // Every file type it names: its value, its "Open with" list, and its "New" entries.
    let new_entries: Vec<&String> = subkeys.iter().filter(|s| !EXTENSION_PARTS.contains(&s.to_lowercase().as_str())).collect();
    let only_types = subkeys.iter().all(|s| s.eq_ignore_ascii_case("OpenWithProgids") || new_entries.contains(&s));
    let named: Option<Vec<Confidence>> =
        default.iter().chain(&open_with).chain(new_entries.iter().copied()).map(|p| owned.progid(p)).collect();
    if only_default
        && only_types
        && let Some(confidence) = named.and_then(|c| c.into_iter().min())
    {
        return vec![root.key(path, confidence, "File extension only the app used")];
    }
    let open_with = open_with.into_iter().filter_map(|progid| {
        let confidence = owned.progid(&progid)?;
        Some(root.value(format!(r"{path}\OpenWithProgids"), progid, confidence, "\"Open with\" entry for the app's file type"))
    });
    let new_entries = new_entries.into_iter().filter_map(|progid| {
        let confidence = owned.progid(progid)?;
        Some(root.key(format!(r"{path}\{progid}"), confidence, "\"New\" menu entry for the app's file type"))
    });
    open_with.chain(new_entries).collect()
}

/// Icon overlays, shell extension approvals, and preview handlers, which
/// Windows lists by the COM class they load.
fn shell_lists(root: Root, owned: &Owned) -> Vec<Leftover> {
    let mut found = Vec::new();
    let overlays = format!(r"{}\{OVERLAYS}", root.software);
    if let Some(key) = root.open(&overlays) {
        for name in key.enum_keys().flatten() {
            if let Some(confidence) = string(&key, &name, "").and_then(|guid| owned.clsid(&guid)) {
                found.push(root.key(format!(r"{overlays}\{name}"), confidence, "File icon overlay from the app"));
            }
        }
    }
    let lists =
        [(APPROVED, "Windows' approval of the app's shell extension"), (PREVIEW_HANDLERS, "Preview handler registration for the app")];
    for (list, reason) in lists {
        let path = format!(r"{}\{list}", root.software);
        let Some(key) = root.open(&path) else { continue };
        for (guid, _) in key.enum_values().flatten() {
            if let Some(confidence) = owned.clsid(&guid) {
                found.push(root.value(path.clone(), guid, confidence, reason));
            }
        }
    }
    found
}

/// Default apps entries under `Clients`, like a browser's or mail app's.
fn clients(root: Root, folders: &[(String, Confidence)]) -> Vec<Leftover> {
    let path = format!(r"{}\Clients", root.software);
    let mut found = Vec::new();
    for kind in subkeys(root, &path) {
        for name in subkeys(root, &format!(r"{path}\{kind}")) {
            let sub = format!(r"{path}\{kind}\{name}");
            if let Some(confidence) = root.open(&sub).and_then(|app| ClassFiles::read(&app).owned(folders)) {
                found.push(root.key(sub, confidence, "Default apps entry for the app"));
            }
        }
    }
    found
}

fn keep_best(map: &mut HashMap<String, Confidence>, name: &str, confidence: Confidence) {
    let best = map.entry(name.to_lowercase()).or_insert(confidence);
    *best = (*best).max(confidence);
}

/// Everything above that points into `folders`.
pub(super) fn find(folders: &[(String, Confidence)]) -> Vec<Leftover> {
    if folders.is_empty() {
        return Vec::new();
    }
    let mut owned = Owned { folders, clsids: HashMap::new(), progids: HashMap::new() };
    let mut found = Vec::new();
    // Classes and file types first: what comes after refers to them, from either hive.
    for root in ROOTS {
        for (guid, item) in com_classes(root, folders) {
            keep_best(&mut owned.clsids, &guid, item.confidence);
            found.push(item);
        }
        for (progid, item) in file_types(root, folders) {
            if let Some(progid) = progid {
                keep_best(&mut owned.progids, &progid, item.confidence);
            }
            found.push(item);
        }
    }
    for root in ROOTS {
        let classes = format!(r"{}\Classes", root.software);
        let extensions: Vec<String> = subkeys(root, &classes).into_iter().filter(|name| name.starts_with('.')).collect();
        let holders: Vec<String> = SHELL_CLASSES
            .iter()
            .map(|c| c.to_string())
            .chain(
                subkeys(root, &format!(r"{classes}\SystemFileAssociations"))
                    .into_iter()
                    .map(|kind| format!(r"SystemFileAssociations\{kind}")),
            )
            .chain(extensions.iter().cloned())
            .collect();
        found.extend(holders.par_iter().flat_map_iter(|holder| menu_entries(root, holder, &owned)).collect::<Vec<_>>());
        found.extend(extensions.par_iter().flat_map_iter(|ext| extension(root, ext, &owned)).collect::<Vec<_>>());
        found.extend(shell_lists(root, &owned));
        found.extend(clients(root, folders));
    }
    found
}

/// Default apps registrations (`RegisteredApplications`) for the app: their
/// capabilities name an icon in the app's folder, or sit inside a key
/// already in `found`, like the app's own settings key.
pub(super) fn registered_applications(folders: &[(String, Confidence)], found: &[Leftover]) -> Vec<Leftover> {
    let mut out = Vec::new();
    for root in ROOTS {
        let path = format!(r"{}\RegisteredApplications", root.software);
        let Some(key) = root.open(&path) else { continue };
        for (name, value) in key.enum_values().flatten() {
            let Ok(capabilities) = String::from_reg_value(&value) else { continue };
            let capabilities = capabilities.trim_matches('\\');
            let full = format!(r"{}\{capabilities}", root.label).to_lowercase();
            let within_found = found
                .iter()
                .filter(|item| item.kind == LeftoverKind::RegistryKey)
                .filter(|item| {
                    let key = item.path.to_lowercase();
                    full == key || full.starts_with(&format!(r"{key}\"))
                })
                .map(|item| item.confidence)
                .max();
            let by_icon = root
                .open(capabilities)
                .and_then(|k| string(&k, "", "ApplicationIcon"))
                .filter(|icon| !names_windows_file(icon))
                .and_then(|icon| inside(&icon, folders));
            if let Some(confidence) = within_found.max(by_icon) {
                out.push(root.value(path.clone(), name, confidence, "Default apps entry for the app"));
            }
        }
    }
    out
}

/// Whether a key in the otherwise protected parts of the registry may be
/// removed: one of the kinds above, holding nothing of Windows'.
pub(super) fn may_remove_key(hive: &RegKey, sub: &str) -> bool {
    let key = || hive.open_subkey_with_flags(sub, KEY_READ | KEY_WOW64_64KEY).ok();
    let parts = reg_parts(sub);
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    match parts.as_slice() {
        ["software", "classes", "clsid", _] | ["software", "classes", "wow6432node", "clsid", _] => {
            key().is_some_and(|k| clsid_removable(&k))
        }
        ["software", "classes", "applications", _] => key().is_some_and(|k| ClassFiles::read(&k).removable()),
        ["software", "classes", ext] if ext.starts_with('.') => key().is_some_and(|k| extension_removable(&k)),
        ["software", "classes", name] => !is_windows_class(name) && key().is_some_and(|k| ClassFiles::read(&k).removable()),
        ["software", "classes", ext, progid] if ext.starts_with('.') && !EXTENSION_PARTS.contains(progid) => progid_removable(progid),
        ["software", "classes", rest @ ..] => key().is_some_and(|k| menu_entry_removable(&k, rest)),
        ["software", "clients", _, _] => key().is_some_and(|k| ClassFiles::read(&k).removable()),
        ["software", "microsoft", "windows", "currentversion", "explorer", "shelliconoverlayidentifiers", _] => {
            key().and_then(|k| string(&k, "", "")).is_some_and(|guid| is_guid(&guid) && clsid_removable_anywhere(&guid))
        }
        _ => false,
    }
}

fn extension_removable(ext: &RegKey) -> bool {
    let only_default = ext.enum_values().flatten().all(|(name, _)| name.is_empty());
    let default_ok = string(ext, "", "").is_none_or(|progid| progid_removable(&progid));
    let subkeys_ok = ext.enum_keys().flatten().all(|sub| {
        if sub.eq_ignore_ascii_case("OpenWithProgids") {
            ext.open_subkey_with_flags(&sub, KEY_READ)
                .is_ok_and(|k| k.enum_values().flatten().all(|(name, _)| name.is_empty() || progid_removable(&name)))
        } else {
            !EXTENSION_PARTS.contains(&sub.to_lowercase().as_str()) && progid_removable(&sub)
        }
    });
    only_default && default_ok && subkeys_ok
}

/// `rest` is the path after `Classes`: a verb or shell handler under one of
/// Windows' general classes, a `SystemFileAssociations` kind, or an extension.
fn menu_entry_removable(key: &RegKey, rest: &[&str]) -> bool {
    let Some(at) = rest.iter().position(|p| *p == "shell" || *p == "shellex") else { return false };
    let holder = rest[..at].join("\\");
    let holder_ok = SHELL_CLASSES.iter().any(|c| c.eq_ignore_ascii_case(&holder))
        || matches!(rest[..at], ["systemfileassociations", _])
        || matches!(rest[..at], [ext] if ext.starts_with('.'));
    if !holder_ok {
        return false;
    }
    match &rest[at..] {
        ["shell", verb] => {
            !is_windows_verb(verb)
                && match string(key, "command", "") {
                    Some(command) => !names_windows_file(&command),
                    None => string(key, "", "ExplorerCommandHandler").is_some_and(|guid| is_guid(&guid) && clsid_removable_anywhere(&guid)),
                }
        }
        ["shellex", _] => string(key, "", "").is_some_and(|guid| is_guid(&guid) && clsid_removable_anywhere(&guid)),
        ["shellex", _, name] => {
            let guid = string(key, "", "").unwrap_or_else(|| name.to_string());
            is_guid(&guid) && clsid_removable_anywhere(&guid)
        }
        _ => false,
    }
}

/// Whether a value may be removed from one of the protected lists that
/// hold one value per program or file type.
pub(super) fn may_remove_value(sub: &str) -> bool {
    let parts = reg_parts(sub);
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    match parts.as_slice() {
        ["software", "classes", ext, "openwithprogids"] => ext.starts_with('.'),
        ["software", "registeredapplications"] => true,
        ["software", rest @ ..] => {
            let path = rest.join("\\");
            path == APPROVED.to_lowercase() || path == PREVIEW_HANDLERS.to_lowercase()
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winreg::enums::KEY_ALL_ACCESS;

    #[test]
    fn spots_windows_files() {
        assert!(names_windows_file("shell32.dll"));
        assert!(names_windows_file("@%SystemRoot%\\system32\\notepad.exe,-469"));
        assert!(names_windows_file(r#""C:\Windows\System32\rundll32.exe" C:\Apps\x.dll,Run"#));
        assert!(names_windows_file(r"%ProgramFiles%\Windows Photo Viewer\PhotoViewer.dll"));
        assert!(!names_windows_file(r#""C:\Program Files\NTLite\NTLite.exe" "%1""#));
        assert!(!names_windows_file(r"C:\Program Files\NTLite\NTLite.exe,0"));
        assert!(!names_windows_file("file:///C:/Program Files/NTLite/Shell.dll"));
    }

    #[test]
    fn never_removes_windows_own() {
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        // Explorer's "This PC", Windows' text file type and extension, a
        // Windows verb, and Windows' "Open With" menu handler.
        for sub in [
            r"SOFTWARE\Classes\CLSID\{20D04FE0-3AEA-1069-A2D8-08002B30309D}",
            r"SOFTWARE\Classes\txtfile",
            r"SOFTWARE\Classes\exefile",
            r"SOFTWARE\Classes\.txt",
            r"SOFTWARE\Classes\.txt\ShellNew",
            r"SOFTWARE\Classes\Folder\shell\open",
            r"SOFTWARE\Classes\*\shellex\ContextMenuHandlers",
            r"SOFTWARE\Classes\*\shellex\ContextMenuHandlers\Open With",
            r"SOFTWARE\Classes\Directory",
            r"SOFTWARE\Classes\http",
        ] {
            assert!(!may_remove_key(&hklm, sub), "{sub} must stay");
        }
        assert!(!may_remove_value(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run\Shell"));
    }

    /// Registers a made-up app's shell extension, file type, extension, and
    /// menu entry in HKCU, and removes them again when dropped.
    struct FakeApp {
        guid: String,
        progid: String,
        ext: String,
        verb: String,
    }

    impl FakeApp {
        fn register(folder: &str) -> Self {
            let id = std::process::id();
            let app = FakeApp {
                guid: format!("{{5EC1A1A1-0000-4000-8000-{id:012X}}}"),
                progid: format!("ReclaimTest{id}.Doc"),
                ext: format!(".reclaimtest{id}"),
                verb: format!("ReclaimTest{id}"),
            };
            let classes = RegKey::predef(HKEY_CURRENT_USER).create_subkey(r"Software\Classes").unwrap().0;
            let set = |path: &str, name: &str, value: &str| classes.create_subkey(path).unwrap().0.set_value(name, &value).unwrap();
            set(&format!(r"CLSID\{}", app.guid), "", "Reclaim Test Extension");
            set(&format!(r"CLSID\{}\InprocServer32", app.guid), "", &format!(r"{folder}\ext.dll"));
            set(&format!(r"*\shellex\ContextMenuHandlers\{}", app.verb), "", &app.guid);
            set(&format!(r"{}\shell\open\command", app.progid), "", &format!(r#""{folder}\app.exe" "%1""#));
            set(&format!(r"{}\DefaultIcon", app.progid), "", &format!(r"{folder}\app.exe,0"));
            set(&app.ext, "", &app.progid);
            set(&format!(r"Directory\shell\{}\command", app.verb), "", &format!(r#""{folder}\app.exe" "%V""#));
            app
        }
    }

    impl Drop for FakeApp {
        fn drop(&mut self) {
            let classes = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(r"Software\Classes", KEY_ALL_ACCESS).unwrap();
            for path in [
                format!(r"CLSID\{}", self.guid),
                format!(r"*\shellex\ContextMenuHandlers\{}", self.verb),
                self.progid.clone(),
                self.ext.clone(),
                format!(r"Directory\shell\{}", self.verb),
            ] {
                let _ = classes.delete_subkey_all(&path);
            }
        }
    }

    #[test]
    fn finds_and_may_remove_an_apps_shell_integration() {
        let folder = r"C:\Program Files\ReclaimTestApp";
        let app = FakeApp::register(folder);
        let found = find(&[(normalize(std::path::Path::new(folder)), Confidence::High)]);
        let paths: Vec<String> = found.iter().map(|f| f.path.to_lowercase()).collect();
        let expect = [
            format!(r"hkcu\software\classes\clsid\{}", app.guid),
            format!(r"hkcu\software\classes\*\shellex\contextmenuhandlers\{}", app.verb),
            format!(r"hkcu\software\classes\{}", app.progid),
            format!(r"hkcu\software\classes\{}", app.ext),
            format!(r"hkcu\software\classes\directory\shell\{}", app.verb),
        ];
        for path in &expect {
            assert!(paths.contains(&path.to_lowercase()), "missing {path} in {paths:#?}");
        }
        assert_eq!(found.len(), expect.len(), "{paths:#?}");
        assert!(found.iter().all(|f| f.confidence == Confidence::High));
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        for item in &found {
            let sub = item.path.split_once('\\').unwrap().1;
            assert!(may_remove_key(&hkcu, sub), "{sub} should be removable");
        }

        // Then removes them for real, each backed up first.
        let backups = std::env::temp_dir().join(format!("reclaim-classes-backups-{}", std::process::id()));
        let outcomes = super::super::remove_all(&found, &backups, false);
        let written = std::fs::read_dir(&backups).map(|d| d.count()).unwrap_or(0);
        let _ = std::fs::remove_dir_all(&backups);
        for (item, outcome) in found.iter().zip(&outcomes) {
            assert!(outcome.is_ok(), "{}: {outcome:?}", item.path);
            assert!(!super::super::still_present(item), "{} is still there", item.path);
        }
        assert_eq!(written, found.len(), "one backup per removed key");
        drop(app);
    }
}
