//! Well-known Windows folders, environment expansion, and the safety rules
//! that decide which paths Reclaim is never allowed to delete.

use std::fs;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rayon::prelude::*;

pub const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;
pub const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x40000;
pub const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x400000;

/// Cloud placeholders (OneDrive "online-only" files etc.) report their full
/// size but occupy nothing on disk, so they must not count toward usage.
pub fn is_cloud_placeholder(attrs: u32) -> bool {
    attrs & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) != 0
}

pub struct KnownDirs {
    pub windows: PathBuf,
    pub program_files: PathBuf,
    pub program_files_x86: PathBuf,
    pub program_data: PathBuf,
    pub user_profile: PathBuf,
    pub app_data: PathBuf,
    pub local_app_data: PathBuf,
    pub local_programs: PathBuf,
    pub temp: PathBuf,
    pub start_menu_user: PathBuf,
    pub start_menu_common: PathBuf,
    /// Can be redirected, often into OneDrive, so it isn't always under the profile.
    pub documents: PathBuf,
    pub saved_games: PathBuf,
}

fn env_dir(name: &str, fallback: &str) -> PathBuf {
    std::env::var_os(name).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(fallback))
}

/// A known folder resolved by the shell. The machine-wide ones read HKLM, so
/// unlike environment variables a standard user can't redirect them.
fn known_folder(id: &windows::core::GUID) -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{KF_FLAG_DONT_VERIFY, SHGetKnownFolderPath};
    unsafe {
        let raw = SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY, None).ok()?;
        let path = raw.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(raw.0 as *const _));
        path.filter(|p| p.is_absolute())
    }
}

fn system_windows_dir() -> Option<PathBuf> {
    use windows::Win32::System::SystemInformation::GetSystemWindowsDirectoryW;
    let mut buf = [0u16; 260];
    let len = unsafe { GetSystemWindowsDirectoryW(Some(&mut buf)) } as usize;
    (len > 0 && len < buf.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len])))
}

/// Folder locations. Machine-wide folders (Windows, Program Files,
/// ProgramData, the profile root) come from Windows APIs rather than the
/// environment, because the protected-path rules depend on them and an
/// elevated Reclaim must not trust variables a standard user can set.
pub fn known() -> &'static KnownDirs {
    use windows::Win32::UI::Shell::{
        FOLDERID_Documents, FOLDERID_Profile, FOLDERID_ProgramData, FOLDERID_ProgramFiles, FOLDERID_ProgramFilesX86, FOLDERID_SavedGames,
    };
    static DIRS: OnceLock<KnownDirs> = OnceLock::new();
    DIRS.get_or_init(|| {
        let user_profile = known_folder(&FOLDERID_Profile).unwrap_or_else(|| env_dir("USERPROFILE", r"C:\Users\Default"));
        let app_data = long_path(&env_dir("APPDATA", &user_profile.join(r"AppData\Roaming").to_string_lossy()));
        let local_app_data = long_path(&env_dir("LOCALAPPDATA", &user_profile.join(r"AppData\Local").to_string_lossy()));
        let program_data = known_folder(&FOLDERID_ProgramData).unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
        KnownDirs {
            windows: system_windows_dir().unwrap_or_else(|| PathBuf::from(r"C:\Windows")),
            program_files: known_folder(&FOLDERID_ProgramFiles).unwrap_or_else(|| PathBuf::from(r"C:\Program Files")),
            program_files_x86: known_folder(&FOLDERID_ProgramFilesX86).unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)")),
            temp: long_path(&env_dir("TEMP", &local_app_data.join("Temp").to_string_lossy())),
            local_programs: local_app_data.join("Programs"),
            start_menu_user: app_data.join(r"Microsoft\Windows\Start Menu\Programs"),
            start_menu_common: program_data.join(r"Microsoft\Windows\Start Menu\Programs"),
            documents: known_folder(&FOLDERID_Documents).map(|p| long_path(&p)).unwrap_or_else(|| user_profile.join("Documents")),
            saved_games: known_folder(&FOLDERID_SavedGames).map(|p| long_path(&p)).unwrap_or_else(|| user_profile.join("Saved Games")),
            program_data,
            user_profile,
            app_data,
            local_app_data,
        }
    })
}

/// A Windows tool by full path, so an exe of the same name next to Reclaim or
/// on PATH can't stand in for it, which matters when Reclaim runs elevated.
pub fn system32(exe: &str) -> PathBuf {
    known().windows.join("System32").join(exe)
}

/// Expands 8.3 short names (`C:\PROGRA~1` -> `C:\Program Files`) so path
/// comparisons see the real names. Paths that don't exist come back as-is.
pub fn long_path(path: &Path) -> PathBuf {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows::Win32::Storage::FileSystem::GetLongPathNameW;
    use windows::core::PCWSTR;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut buf = vec![0u16; 512];
    loop {
        let len = unsafe { GetLongPathNameW(PCWSTR(wide.as_ptr()), Some(&mut buf)) } as usize;
        if len == 0 {
            return path.to_path_buf();
        }
        if len < buf.len() {
            return PathBuf::from(std::ffi::OsString::from_wide(&buf[..len]));
        }
        buf.resize(len + 1, 0); // too small: `len` is the size needed
    }
}

/// The 8.3 short form of `path`, if the volume has one that differs.
#[cfg(test)]
pub fn short_path(path: &Path) -> Option<PathBuf> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows::Win32::Storage::FileSystem::GetShortPathNameW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut buf = [0u16; 512];
    let len = unsafe { GetShortPathNameW(windows::core::PCWSTR(wide.as_ptr()), Some(&mut buf)) } as usize;
    let short = (len > 0 && len < buf.len()).then(|| PathBuf::from(std::ffi::OsString::from_wide(&buf[..len])))?;
    (short != path).then_some(short)
}

/// Expands `%VAR%` references using the process environment. Unknown
/// variables are left untouched so they fail safely later.
pub fn expand_env(input: &str) -> String {
    expand_vars(input, |name| std::env::var(name).ok())
}

/// Expands `%VAR%` references using `lookup`.
pub fn expand_vars(input: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match lookup(name) {
                    Some(value) if !name.is_empty() => out.push_str(&value),
                    _ => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Lowercased, backslash-normalized form without a trailing separator.
pub fn normalize(path: &Path) -> String {
    let mut s = path.to_string_lossy().replace('/', "\\").to_lowercase();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        s = stripped.to_string();
    }
    while s.ends_with('\\') && s.len() > 3 {
        s.pop();
    }
    s
}

/// Whether normalized `path` is `root` or inside it.
pub fn is_under(path: &str, root: &str) -> bool {
    path == root || (path.starts_with(root) && path.as_bytes().get(root.len()) == Some(&b'\\'))
}

/// Returns true when deleting `path` could damage Windows or wipe out a
/// whole category of user data. Every destructive command checks this.
pub fn is_protected(path: &Path) -> bool {
    if !path.is_absolute() {
        return true;
    }
    // Without this, C:\PROGRA~1 would slip past the C:\Program Files rule.
    let p = normalize(&long_path(path));
    // Drive roots and anything too shallow to be specific.
    if p.len() <= 3 || p.matches('\\').count() < 1 || p.ends_with(':') {
        return true;
    }

    let k = known();
    let n = |pb: &Path| normalize(pb);

    // Never touch anything inside these trees.
    let protected_trees = [
        n(&k.windows),
        n(&k.program_files.join("WindowsApps")),
        n(&k.program_files.join("Windows Defender")),
        n(&k.program_data.join("Microsoft")),
        n(&k.local_app_data.join("Microsoft\\WindowsApps")),
        n(&k.local_app_data.join("Packages")),
    ];
    // The all-users Start menu sits inside ProgramData\Microsoft, but what's
    // in it are apps' shortcuts. The folders themselves stay protected below.
    if protected_trees.iter().any(|root| is_under(&p, root)) && !is_under(&p, &n(&k.start_menu_common)) {
        return true;
    }
    let drive = &p[..2];
    for tail in [r"\system volume information", r"\$recycle.bin", r"\recovery", r"\boot", r"\efi"] {
        if is_under(&p, &format!("{drive}{tail}")) {
            return true;
        }
    }

    // These folders may contain deletable children, but never themselves.
    let up = &k.user_profile;
    let exact = [
        n(&k.program_files),
        n(&k.program_files_x86),
        n(&k.program_files.join("Common Files")),
        n(&k.program_files_x86.join("Common Files")),
        n(&k.program_data),
        n(&k.app_data),
        n(&k.local_app_data),
        n(&k.local_programs),
        n(&k.local_app_data.join("Microsoft")),
        n(&k.app_data.join("Microsoft")),
        n(&up.join(r"AppData\LocalLow")),
        n(&up.join("AppData")),
        n(&k.temp),
        n(up),
        n(up.parent().unwrap_or(up)),
        n(&up.join("Desktop")),
        n(&up.join("Documents")),
        n(&up.join("Downloads")),
        n(&up.join("Pictures")),
        n(&up.join("Music")),
        n(&up.join("Videos")),
        n(&up.join("OneDrive")),
        n(&up.join("Saved Games")),
        n(&k.documents),
        n(&k.saved_games),
        n(&k.start_menu_user),
        n(&k.start_menu_common),
        n(&k.start_menu_user.join("Startup")),
        n(&k.start_menu_common.join("Startup")),
        n(k.start_menu_user.parent().unwrap_or(&k.start_menu_user)),
        n(k.start_menu_common.parent().unwrap_or(&k.start_menu_common)),
    ];
    if exact.contains(&p) {
        return true;
    }

    // Shared publisher folders under Program Files like "Microsoft Office".
    for pf in [&k.program_files, &k.program_files_x86] {
        let root = n(pf);
        if let Some(rest) = p.strip_prefix(&format!("{root}\\")) {
            let first = rest.split('\\').next().unwrap_or("");
            if first.starts_with("windows ") || first == "windows nt" || first == "internet explorer" {
                return true;
            }
        }
    }
    false
}

/// Case-insensitive wildcard match supporting `*` and `?`.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Expands a path whose components may contain wildcards, e.g.
/// `C:\Users\me\AppData\Local\Google\Chrome\User Data\*\Cache`. Variables
/// must already be expanded; anything still containing `%` matches nothing.
pub fn expand_glob(expanded: &str) -> Vec<PathBuf> {
    if expanded.contains('%') {
        return Vec::new();
    }
    let mut parts = expanded.split('\\').filter(|s| !s.is_empty());
    let Some(first) = parts.next() else { return Vec::new() };
    let mut current = vec![PathBuf::from(format!("{first}\\"))];
    for part in parts {
        let mut next = Vec::new();
        if part.contains('*') || part.contains('?') {
            for base in &current {
                if let Ok(entries) = fs::read_dir(base) {
                    for entry in entries.flatten() {
                        if wildcard_match(part, &entry.file_name().to_string_lossy()) {
                            next.push(entry.path());
                        }
                    }
                }
            }
        } else {
            for base in &current {
                let candidate = base.join(part);
                if candidate.exists() {
                    next.push(candidate);
                }
            }
        }
        current = next;
        if current.is_empty() {
            break;
        }
    }
    current
}

#[derive(Default, Clone, Copy)]
pub struct DirStats {
    pub bytes: u64,
    pub files: u64,
}

impl std::ops::Add for DirStats {
    type Output = DirStats;
    fn add(self, o: DirStats) -> DirStats {
        DirStats { bytes: self.bytes + o.bytes, files: self.files + o.files }
    }
}

/// Recursively sums file sizes under `path` in parallel. Junctions and
/// symlinks are not followed so nothing is counted twice.
pub fn dir_stats(path: &Path) -> DirStats {
    let Ok(entries) = fs::read_dir(path) else { return DirStats::default() };
    let mut stats = DirStats::default();
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.file_type().is_symlink() {
            continue;
        }
        let attrs = meta.file_attributes();
        if meta.is_dir() {
            subdirs.push(entry.path());
        } else {
            stats.files += 1;
            if !is_cloud_placeholder(attrs) {
                stats.bytes += meta.len();
            }
        }
    }
    stats + subdirs.par_iter().map(|d| dir_stats(d)).reduce(DirStats::default, |a, b| a + b)
}

/// Size of a file or directory tree.
pub fn path_stats(path: &Path) -> DirStats {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => dir_stats(path),
        Ok(m) => DirStats { bytes: m.len(), files: 1 },
        Err(_) => DirStats::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard() {
        assert!(wildcard_match("thumbcache_*.db", "thumbcache_1024.db"));
        assert!(wildcard_match("*", "Default"));
        assert!(wildcard_match("Profile ?", "Profile 1"));
        assert!(!wildcard_match("thumbcache_*.db", "iconcache_1024.db"));
    }

    #[test]
    fn protection() {
        let k = known();
        assert!(is_protected(Path::new(r"C:\")));
        assert!(is_protected(&k.windows.join("System32")));
        assert!(is_protected(&k.program_files));
        assert!(is_protected(&k.local_app_data));
        assert!(is_protected(&k.user_profile));
        assert!(is_protected(Path::new("relative\\path")));
        assert!(is_protected(&k.start_menu_common));
        assert!(is_protected(&k.start_menu_common.join("Startup")));
        assert!(is_protected(&k.start_menu_user.join("Startup")));
        assert!(!is_protected(&k.start_menu_common.join("Git")));
        assert!(is_protected(&k.program_data.join(r"Microsoft\Windows\WER")));
        assert!(is_protected(&k.documents));
        assert!(is_protected(&k.saved_games));
        assert!(!is_protected(&k.program_files.join("SomeVendor")));
        assert!(!is_protected(&k.local_app_data.join("Discord")));
    }

    #[test]
    fn short_names_do_not_bypass_protection() {
        let k = known();
        // Only meaningful where the volume still generates 8.3 names.
        let Some(short) = short_path(&k.program_files) else { return };
        assert!(is_protected(&short), "{} should be protected", short.display());
        assert_eq!(normalize(&long_path(&short)), normalize(&k.program_files));
    }

    #[test]
    fn env_expansion() {
        let windir = std::env::var("SystemRoot").unwrap();
        assert_eq!(expand_env(r"%SystemRoot%\Temp"), format!(r"{windir}\Temp"));
        assert_eq!(expand_env("%NOPE_NOT_SET%"), "%NOPE_NOT_SET%");
    }
}
