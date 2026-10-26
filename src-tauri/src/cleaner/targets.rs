//! Everything the Cleanup screen knows how to clear. Each pattern matches a
//! folder whose *contents* are removed (the folder itself stays) or a file.
//! `%VAR%` expands from the environment and `*` matches one path segment.

pub struct Target {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub patterns: &'static [&'static str],
    pub requires_admin: bool,
    pub default_selected: bool,
    /// Only files older than this are removed (protects in-progress work).
    pub min_age_hours: u64,
    /// Executables that lock these files while running.
    pub processes: &'static [&'static str],
}

pub const RECYCLE_BIN: &str = "recycle-bin";

macro_rules! chromium {
    ($id:literal, $name:literal, $base:literal, $exe:literal) => {
        Target {
            id: $id,
            name: $name,
            category: "browsers",
            description: "Cached pages, scripts, and images. Sites load a little slower the first time afterward. Logins and history are kept.",
            patterns: &[
                concat!($base, r"\*\Cache"),
                concat!($base, r"\*\Code Cache"),
                concat!($base, r"\*\GPUCache"),
                concat!($base, r"\*\Service Worker\CacheStorage"),
                concat!($base, r"\*\DawnGraphiteCache"),
                concat!($base, r"\*\DawnWebGPUCache"),
                concat!($base, r"\ShaderCache"),
                concat!($base, r"\GrShaderCache"),
                concat!($base, r"\GraphiteDawnCache"),
            ],
            requires_admin: false,
            default_selected: true,
            min_age_hours: 0,
            processes: &[$exe],
        }
    };
}

macro_rules! electron {
    ($id:literal, $name:literal, $base:literal, $exe:literal) => {
        Target {
            id: $id,
            name: $name,
            category: "apps",
            description: "Cached images and scripts. Your account and messages are not affected.",
            patterns: &[
                concat!($base, r"\Cache"),
                concat!($base, r"\Code Cache"),
                concat!($base, r"\GPUCache"),
                concat!($base, r"\DawnGraphiteCache"),
                concat!($base, r"\DawnWebGPUCache"),
                concat!($base, r"\Service Worker\CacheStorage"),
            ],
            requires_admin: false,
            default_selected: true,
            min_age_hours: 0,
            processes: &[$exe],
        }
    };
}

macro_rules! dev_cache {
    ($id:literal, $name:literal, $desc:literal, [$($pattern:literal),+ $(,)?]) => {
        Target {
            id: $id,
            name: $name,
            category: "developer",
            description: $desc,
            patterns: &[$($pattern),+],
            requires_admin: false,
            default_selected: false,
            min_age_hours: 0,
            processes: &[],
        }
    };
}

pub const TARGETS: &[Target] = &[
    // System
    Target {
        id: "user-temp",
        name: "Temporary files",
        category: "system",
        description: "Files apps and installers left in your temp folder. Anything touched in the last 24 hours is kept.",
        patterns: &[r"%TEMP%"],
        requires_admin: false,
        default_selected: true,
        min_age_hours: 24,
        processes: &[],
    },
    Target {
        id: "windows-temp",
        name: "Windows temporary files",
        category: "system",
        description: "System-wide temp folder. Anything touched in the last 24 hours is kept.",
        patterns: &[r"%SystemRoot%\Temp"],
        requires_admin: true,
        default_selected: true,
        min_age_hours: 24,
        processes: &[],
    },
    Target {
        id: RECYCLE_BIN,
        name: "Recycle Bin",
        category: "system",
        description: "Permanently deletes everything in the Recycle Bin on every drive.",
        patterns: &[],
        requires_admin: false,
        default_selected: false,
        min_age_hours: 0,
        processes: &[],
    },
    Target {
        id: "windows-update",
        name: "Windows Update downloads",
        category: "system",
        description: "Update packages that were already installed. Windows downloads them again if it needs to.",
        patterns: &[r"%SystemRoot%\SoftwareDistribution\Download"],
        requires_admin: true,
        default_selected: true,
        min_age_hours: 24,
        processes: &[],
    },
    Target {
        id: "delivery-optimization",
        name: "Delivery Optimization cache",
        category: "system",
        description: "Update pieces Windows keeps to share with other PCs.",
        patterns: &[r"%SystemRoot%\ServiceProfiles\NetworkService\AppData\Local\Microsoft\Windows\DeliveryOptimization\Cache"],
        requires_admin: true,
        default_selected: true,
        min_age_hours: 0,
        processes: &[],
    },
    Target {
        id: "error-reports",
        name: "Error reports",
        category: "system",
        description: "Crash reports Windows collected for Microsoft.",
        patterns: &[
            r"%LOCALAPPDATA%\Microsoft\Windows\WER\ReportArchive",
            r"%LOCALAPPDATA%\Microsoft\Windows\WER\ReportQueue",
            r"%ProgramData%\Microsoft\Windows\WER\ReportArchive",
            r"%ProgramData%\Microsoft\Windows\WER\ReportQueue",
        ],
        requires_admin: false,
        default_selected: true,
        min_age_hours: 0,
        processes: &[],
    },
    Target {
        id: "crash-dumps",
        name: "App crash dumps",
        category: "system",
        description: "Memory snapshots saved when apps crashed. Only useful when debugging those crashes.",
        patterns: &[r"%LOCALAPPDATA%\CrashDumps"],
        requires_admin: false,
        default_selected: true,
        min_age_hours: 0,
        processes: &[],
    },
    Target {
        id: "system-crash-dumps",
        name: "System crash dumps",
        category: "system",
        description: "Memory dumps from blue screens. Can be several gigabytes.",
        patterns: &[r"%SystemRoot%\Minidump", r"%SystemRoot%\MEMORY.DMP", r"%SystemRoot%\LiveKernelReports"],
        requires_admin: true,
        default_selected: true,
        min_age_hours: 0,
        processes: &[],
    },
    Target {
        id: "thumbnail-cache",
        name: "Thumbnail cache",
        category: "system",
        description: "Picture previews File Explorer generates. They're rebuilt as you browse.",
        patterns: &[r"%LOCALAPPDATA%\Microsoft\Windows\Explorer\thumbcache_*.db"],
        requires_admin: false,
        default_selected: false,
        min_age_hours: 0,
        processes: &["explorer.exe"],
    },
    Target {
        id: "shader-cache",
        name: "Graphics shader caches",
        category: "system",
        description: "Compiled shaders from DirectX and NVIDIA. Games rebuild them, which can cause brief stutter on first launch.",
        patterns: &[
            r"%LOCALAPPDATA%\D3DSCache",
            r"%LOCALAPPDATA%\NVIDIA\DXCache",
            r"%LOCALAPPDATA%\NVIDIA\GLCache",
            r"%LOCALAPPDATA%\AMD\DxCache",
            r"%LOCALAPPDATA%\AMD\DxcCache",
        ],
        requires_admin: false,
        default_selected: false,
        min_age_hours: 0,
        processes: &[],
    },
    // Browsers
    chromium!("chrome", "Google Chrome", r"%LOCALAPPDATA%\Google\Chrome\User Data", "chrome.exe"),
    chromium!("edge", "Microsoft Edge", r"%LOCALAPPDATA%\Microsoft\Edge\User Data", "msedge.exe"),
    chromium!("brave", "Brave", r"%LOCALAPPDATA%\BraveSoftware\Brave-Browser\User Data", "brave.exe"),
    chromium!("vivaldi", "Vivaldi", r"%LOCALAPPDATA%\Vivaldi\User Data", "vivaldi.exe"),
    Target {
        id: "opera",
        name: "Opera",
        category: "browsers",
        description: "Cached pages, scripts, and images. Logins and history are kept.",
        patterns: &[
            r"%LOCALAPPDATA%\Opera Software\Opera Stable\Cache",
            r"%LOCALAPPDATA%\Opera Software\Opera GX Stable\Cache",
            r"%APPDATA%\Opera Software\Opera Stable\Code Cache",
            r"%APPDATA%\Opera Software\Opera GX Stable\Code Cache",
        ],
        requires_admin: false,
        default_selected: true,
        min_age_hours: 0,
        processes: &["opera.exe"],
    },
    Target {
        id: "firefox",
        name: "Firefox",
        category: "browsers",
        description: "Cached pages, scripts, and images. Logins and history are kept.",
        patterns: &[r"%LOCALAPPDATA%\Mozilla\Firefox\Profiles\*\cache2", r"%LOCALAPPDATA%\Mozilla\Firefox\Profiles\*\startupCache"],
        requires_admin: false,
        default_selected: true,
        min_age_hours: 0,
        processes: &["firefox.exe"],
    },
    // Apps
    electron!("discord", "Discord", r"%APPDATA%\discord", "Discord.exe"),
    electron!("slack", "Slack", r"%APPDATA%\Slack", "slack.exe"),
    electron!("vscode", "Visual Studio Code", r"%APPDATA%\Code", "Code.exe"),
    electron!("cursor", "Cursor", r"%APPDATA%\Cursor", "Cursor.exe"),
    Target {
        id: "spotify",
        name: "Spotify",
        category: "apps",
        description: "Streaming cache. Downloaded playlists are kept.",
        patterns: &[r"%LOCALAPPDATA%\Spotify\Data"],
        requires_admin: false,
        default_selected: true,
        min_age_hours: 0,
        processes: &["Spotify.exe"],
    },
    Target {
        id: "steam",
        name: "Steam",
        category: "apps",
        description: "Store and community page cache. Games and saves are not touched.",
        patterns: &[r"%ProgramFiles(x86)%\Steam\appcache\httpcache", r"%LOCALAPPDATA%\Steam\htmlcache"],
        requires_admin: false,
        default_selected: true,
        min_age_hours: 0,
        processes: &["steam.exe", "steamwebhelper.exe"],
    },
    // Developer
    dev_cache!(
        "npm",
        "npm cache",
        "Downloaded package tarballs. npm fetches them again when needed.",
        [r"%LOCALAPPDATA%\npm-cache\_cacache"]
    ),
    dev_cache!(
        "pnpm",
        "pnpm store",
        "Shared package store. Projects re-download what they need; `pnpm store prune` is the gentler option.",
        [r"%LOCALAPPDATA%\pnpm\store", r"%LOCALAPPDATA%\pnpm-cache"]
    ),
    dev_cache!("yarn", "Yarn cache", "Downloaded packages for Yarn.", [r"%LOCALAPPDATA%\Yarn\Cache"]),
    dev_cache!("bun", "Bun cache", "Downloaded packages for Bun.", [r"%USERPROFILE%\.bun\install\cache"]),
    dev_cache!("pip", "pip and uv caches", "Downloaded Python wheels.", [r"%LOCALAPPDATA%\pip\Cache", r"%LOCALAPPDATA%\uv\cache"]),
    dev_cache!(
        "cargo",
        "Cargo registry",
        "Downloaded crate sources. Cargo fetches them again on the next build.",
        [r"%USERPROFILE%\.cargo\registry\cache", r"%USERPROFILE%\.cargo\registry\src"]
    ),
    dev_cache!(
        "nuget",
        "NuGet packages",
        "Global NuGet package folder and HTTP cache.",
        [r"%USERPROFILE%\.nuget\packages", r"%LOCALAPPDATA%\NuGet\v3-cache"]
    ),
    dev_cache!("gradle", "Gradle caches", "Downloaded dependencies and build caches.", [r"%USERPROFILE%\.gradle\caches"]),
    dev_cache!("maven", "Maven repository", "Downloaded Java dependencies.", [r"%USERPROFILE%\.m2\repository"]),
    dev_cache!("go", "Go build cache", "Compiled Go packages.", [r"%LOCALAPPDATA%\go-build"]),
];
