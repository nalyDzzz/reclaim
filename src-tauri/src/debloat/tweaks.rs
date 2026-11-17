//! Reversible registry tweaks for privacy and Windows annoyances. Each one
//! records how to apply it and how to put the Windows default back.

use std::os::windows::process::CommandExt;
use std::process::Command;

use anyhow::Context;
use serde::Serialize;
use windows::Win32::UI::WindowsAndMessaging::{SPI_SETMOUSE, SPIF_SENDCHANGE, SystemParametersInfoW};
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};

use crate::error::CmdResult;
use crate::remembered::{self, Remembered};
use crate::system::{is_elevated, os_build};

#[derive(Clone, Copy)]
enum Hive {
    User,
    Machine,
}

impl Hive {
    fn key(self) -> RegKey {
        match self {
            Hive::User => RegKey::predef(HKEY_CURRENT_USER),
            Hive::Machine => RegKey::predef(HKEY_LOCAL_MACHINE),
        }
    }
}

#[derive(Clone, Copy)]
enum Value {
    Dword(u32),
    Str(&'static str),
    /// Flag bits saved as a decimal string, where the tweak clears the `off`
    /// bits and leaves the rest as they are. `default` is what Windows
    /// assumes when the value is missing.
    Flags {
        off: u32,
        default: u32,
    },
}

/// What restoring the Windows default means for one registry write.
#[derive(Clone, Copy)]
enum Revert {
    Set(Value),
    DeleteValue,
    DeleteKey(&'static str),
    /// Sets a `Flags` value's bits again.
    RestoreFlags,
}

struct Write {
    hive: Hive,
    path: &'static str,
    name: &'static str,
    value: Value,
    revert: Revert,
}

struct Tweak {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    category: &'static str,
    restart_explorer: bool,
    /// Windows only reads it when the user signs in.
    sign_out: bool,
    /// The first Windows build that has the setting.
    min_build: u32,
    writes: &'static [Write],
}

const fn w(hive: Hive, path: &'static str, name: &'static str, value: Value, revert: Revert) -> Write {
    Write { hive, path, name, value, revert }
}

const CDM: &str = r"Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager";
const MOUSE: &str = r"Control Panel\Mouse";
/// Windows 11's first release; builds before it are Windows 10.
const WINDOWS_11: u32 = 22000;
/// Windows 11 22H2.
const WINDOWS_11_22H2: u32 = 22621;
const ADVANCED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced";
use Hive::{Machine, User};
use Value::{Dword, Flags, Str};

const TWEAKS: &[Tweak] = &[
    Tweak {
        id: "advertising-id",
        name: "Turn off advertising ID",
        description: "Stops apps from using an ID to show you personalized ads.",
        category: "privacy",
        restart_explorer: false,
        sign_out: false,
        min_build: 0,
        writes: &[w(User, r"Software\Microsoft\Windows\CurrentVersion\AdvertisingInfo", "Enabled", Dword(0), Revert::Set(Dword(1)))],
    },
    Tweak {
        id: "tailored-experiences",
        name: "Turn off tailored experiences",
        description: "Stops Microsoft from using diagnostic data to show you tips, ads, and recommendations.",
        category: "privacy",
        restart_explorer: false,
        sign_out: false,
        min_build: 0,
        writes: &[w(
            User,
            r"Software\Microsoft\Windows\CurrentVersion\Privacy",
            "TailoredExperiencesWithDiagnosticDataEnabled",
            Dword(0),
            Revert::Set(Dword(1)),
        )],
    },
    Tweak {
        id: "recall",
        name: "Turn off Recall snapshots",
        description: "Stops Windows from saving snapshots of your screen on Copilot+ PCs.",
        category: "privacy",
        restart_explorer: false,
        sign_out: false,
        min_build: 0,
        writes: &[w(User, r"Software\Policies\Microsoft\Windows\WindowsAI", "DisableAIDataAnalysis", Dword(1), Revert::DeleteValue)],
    },
    Tweak {
        id: "telemetry",
        name: "Send only required diagnostic data",
        description: "Sets diagnostic data to the minimum your edition of Windows allows.",
        category: "privacy",
        restart_explorer: false,
        sign_out: false,
        min_build: 0,
        writes: &[w(Machine, r"SOFTWARE\Policies\Microsoft\Windows\DataCollection", "AllowTelemetry", Dword(0), Revert::DeleteValue)],
    },
    Tweak {
        id: "activity-history",
        name: "Turn off activity history",
        description: "Stops Windows from recording the apps and files you use.",
        category: "privacy",
        restart_explorer: false,
        sign_out: false,
        min_build: 0,
        writes: &[w(Machine, r"SOFTWARE\Policies\Microsoft\Windows\System", "PublishUserActivities", Dword(0), Revert::DeleteValue)],
    },
    Tweak {
        id: "bing-search",
        name: "Keep Start search local",
        description: "Removes Bing web results from the Start menu search box.",
        category: "start",
        restart_explorer: true,
        sign_out: false,
        min_build: 0,
        writes: &[
            w(User, r"Software\Policies\Microsoft\Windows\Explorer", "DisableSearchBoxSuggestions", Dword(1), Revert::DeleteValue),
            w(User, r"Software\Microsoft\Windows\CurrentVersion\Search", "BingSearchEnabled", Dword(0), Revert::DeleteValue),
        ],
    },
    Tweak {
        id: "start-recommendations",
        name: "Hide promoted apps in Start",
        description: "Turns off app suggestions and promotions in the Start menu.",
        category: "start",
        restart_explorer: true,
        sign_out: false,
        min_build: 0,
        writes: &[
            w(User, ADVANCED, "Start_IrisRecommendations", Dword(0), Revert::DeleteValue),
            w(User, CDM, "SystemPaneSuggestionsEnabled", Dword(0), Revert::Set(Dword(1))),
            w(User, CDM, "SubscribedContent-338388Enabled", Dword(0), Revert::Set(Dword(1))),
        ],
    },
    Tweak {
        id: "silent-installs",
        name: "Stop automatic app installs",
        description: "Prevents Windows from quietly installing promoted apps like games and streaming services.",
        category: "start",
        restart_explorer: false,
        sign_out: false,
        min_build: 0,
        writes: &[
            w(User, CDM, "SilentInstalledAppsEnabled", Dword(0), Revert::Set(Dword(1))),
            w(User, CDM, "OemPreInstalledAppsEnabled", Dword(0), Revert::Set(Dword(1))),
            w(User, CDM, "PreInstalledAppsEnabled", Dword(0), Revert::Set(Dword(1))),
        ],
    },
    Tweak {
        id: "tips",
        name: "Turn off tips and suggestions",
        description: "Hides \"get even more out of Windows\" prompts and tip notifications.",
        category: "start",
        restart_explorer: false,
        sign_out: false,
        min_build: 0,
        writes: &[
            w(User, CDM, "SubscribedContent-338389Enabled", Dword(0), Revert::Set(Dword(1))),
            w(User, CDM, "SubscribedContent-310093Enabled", Dword(0), Revert::Set(Dword(1))),
            w(User, CDM, "SoftLandingEnabled", Dword(0), Revert::Set(Dword(1))),
            w(
                User,
                r"Software\Microsoft\Windows\CurrentVersion\UserProfileEngagement",
                "ScoobeSystemSettingEnabled",
                Dword(0),
                Revert::DeleteValue,
            ),
        ],
    },
    Tweak {
        id: "lock-screen-ads",
        name: "Turn off lock screen fun facts",
        description: "Removes tips and ads layered over the lock screen picture.",
        category: "start",
        restart_explorer: false,
        sign_out: false,
        min_build: 0,
        writes: &[
            w(User, CDM, "RotatingLockScreenOverlayEnabled", Dword(0), Revert::Set(Dword(1))),
            w(User, CDM, "SubscribedContent-338387Enabled", Dword(0), Revert::Set(Dword(1))),
        ],
    },
    Tweak {
        id: "file-extensions",
        name: "Show file extensions",
        description: "Shows the .exe, .pdf, .zip part of file names in File Explorer.",
        category: "explorer",
        restart_explorer: true,
        sign_out: false,
        min_build: 0,
        writes: &[w(User, ADVANCED, "HideFileExt", Dword(0), Revert::Set(Dword(1)))],
    },
    Tweak {
        id: "hidden-files",
        name: "Show hidden files",
        description: "Shows hidden files and folders in File Explorer.",
        category: "explorer",
        restart_explorer: true,
        sign_out: false,
        min_build: 0,
        writes: &[w(User, ADVANCED, "Hidden", Dword(1), Revert::Set(Dword(2)))],
    },
    Tweak {
        id: "classic-context-menu",
        name: "Classic right-click menu",
        description: "Brings back the full Windows 10 right-click menu, without the \"Show more options\" step.",
        category: "explorer",
        restart_explorer: true,
        sign_out: false,
        min_build: 0,
        writes: &[w(
            User,
            r"Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\InprocServer32",
            "",
            Str(""),
            Revert::DeleteKey(r"Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}"),
        )],
    },
    Tweak {
        id: "settings-suggestions",
        name: "Hide suggestions in Settings",
        description: "Removes suggested content and offers from the Settings app.",
        category: "start",
        restart_explorer: false,
        sign_out: false,
        min_build: 0,
        writes: &[
            w(User, CDM, "SubscribedContent-338393Enabled", Dword(0), Revert::Set(Dword(1))),
            w(User, CDM, "SubscribedContent-353694Enabled", Dword(0), Revert::Set(Dword(1))),
            w(User, CDM, "SubscribedContent-353696Enabled", Dword(0), Revert::Set(Dword(1))),
        ],
    },
    Tweak {
        id: "taskbar-left",
        name: "Align the taskbar to the left",
        description: "Puts Start and your apps at the left edge, the way Windows 10 had them.",
        category: "taskbar",
        restart_explorer: true,
        sign_out: false,
        min_build: WINDOWS_11,
        writes: &[w(User, ADVANCED, "TaskbarAl", Dword(0), Revert::Set(Dword(1)))],
    },
    Tweak {
        id: "taskbar-search",
        name: "Hide the search box",
        description: "Removes search from the taskbar. Start still searches when you open it and type.",
        category: "taskbar",
        restart_explorer: true,
        sign_out: false,
        min_build: 0,
        writes: &[w(User, r"Software\Microsoft\Windows\CurrentVersion\Search", "SearchboxTaskbarMode", Dword(0), Revert::DeleteValue)],
    },
    Tweak {
        id: "taskbar-task-view",
        name: "Hide the Task View button",
        description: "Removes the button for virtual desktops. Win+Tab still opens Task View.",
        category: "taskbar",
        restart_explorer: true,
        sign_out: false,
        min_build: 0,
        writes: &[w(User, ADVANCED, "ShowTaskViewButton", Dword(0), Revert::Set(Dword(1)))],
    },
    Tweak {
        id: "taskbar-never-combine",
        name: "Never combine taskbar buttons",
        description: "Shows each window as its own button with its title, instead of one icon per app.",
        category: "taskbar",
        restart_explorer: true,
        sign_out: false,
        min_build: 0,
        writes: &[w(User, ADVANCED, "TaskbarGlomLevel", Dword(2), Revert::Set(Dword(0)))],
    },
    Tweak {
        id: "clock-seconds",
        name: "Show seconds in the clock",
        description: "Adds seconds to the time on the taskbar.",
        category: "taskbar",
        restart_explorer: true,
        sign_out: false,
        min_build: WINDOWS_11_22H2,
        writes: &[w(User, ADVANCED, "ShowSecondsInSystemClock", Dword(1), Revert::DeleteValue)],
    },
    Tweak {
        id: "end-task",
        name: "End task from the taskbar",
        description: "Adds \"End task\" to the right-click menu of taskbar apps.",
        category: "taskbar",
        restart_explorer: false,
        sign_out: false,
        min_build: 0,
        writes: &[w(
            User,
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced\TaskbarDeveloperSettings",
            "TaskbarEndTask",
            Dword(1),
            Revert::DeleteValue,
        )],
    },
    Tweak {
        id: "open-to-this-pc",
        name: "Open File Explorer to This PC",
        description: "New File Explorer windows show your drives instead of Home or Quick access.",
        category: "explorer",
        restart_explorer: false,
        sign_out: false,
        min_build: 0,
        writes: &[w(User, ADVANCED, "LaunchTo", Dword(1), Revert::DeleteValue)],
    },
    Tweak {
        id: "hide-gallery",
        name: "Hide Gallery",
        description: "Removes Gallery from the File Explorer side panel. Your photos stay where they are.",
        category: "explorer",
        restart_explorer: true,
        sign_out: false,
        min_build: WINDOWS_11_22H2,
        writes: &[w(
            User,
            r"Software\Classes\CLSID\{e88865ea-0e1c-4e20-9aa6-edcd0212c87c}",
            "System.IsPinnedToNameSpaceTree",
            Dword(0),
            Revert::DeleteKey(r"Software\Classes\CLSID\{e88865ea-0e1c-4e20-9aa6-edcd0212c87c}"),
        )],
    },
    Tweak {
        id: "mouse-acceleration",
        name: "Turn off mouse acceleration",
        description: "Turns off Enhance pointer precision, so the same hand movement always moves the pointer the same distance.",
        category: "gaming",
        restart_explorer: false,
        sign_out: false,
        min_build: 0,
        writes: &[
            w(User, MOUSE, "MouseSpeed", Str("0"), Revert::Set(Str("1"))),
            w(User, MOUSE, "MouseThreshold1", Str("0"), Revert::Set(Str("6"))),
            w(User, MOUSE, "MouseThreshold2", Str("0"), Revert::Set(Str("10"))),
        ],
    },
    Tweak {
        id: "accessibility-shortcuts",
        name: "Turn off the Sticky Keys shortcut",
        description: "Pressing Shift five times, or holding it down, no longer stops a game with the Sticky Keys or Filter Keys prompt.",
        category: "gaming",
        restart_explorer: false,
        sign_out: true,
        min_build: 0,
        // Each one's "the shortcut is on" bit.
        writes: &[
            w(User, r"Control Panel\Accessibility\StickyKeys", "Flags", Flags { off: 0x4, default: 510 }, Revert::RestoreFlags),
            w(User, r"Control Panel\Accessibility\Keyboard Response", "Flags", Flags { off: 0x4, default: 126 }, Revert::RestoreFlags),
            w(User, r"Control Panel\Accessibility\ToggleKeys", "Flags", Flags { off: 0x4, default: 62 }, Revert::RestoreFlags),
        ],
    },
    Tweak {
        id: "game-captures",
        name: "Turn off Game Bar recording",
        description: "Turns off Game Bar's clips and screenshots, including background recording, which can cost some performance. The Game Bar overlay still opens.",
        category: "gaming",
        restart_explorer: false,
        sign_out: false,
        min_build: 0,
        writes: &[
            w(User, r"System\GameConfigStore", "GameDVR_Enabled", Dword(0), Revert::Set(Dword(1))),
            w(User, r"Software\Microsoft\Windows\CurrentVersion\GameDVR", "AppCaptureEnabled", Dword(0), Revert::Set(Dword(1))),
        ],
    },
    Tweak {
        id: "fast-startup",
        name: "Turn off Fast Startup",
        description: "Makes Shut down turn the PC fully off. Starting up takes a few seconds longer, but drivers and updates load fresh, and another operating system on the PC can safely use the drive.",
        category: "system",
        restart_explorer: false,
        sign_out: false,
        min_build: 0,
        writes: &[w(
            Machine,
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Power",
            "HiberbootEnabled",
            Dword(0),
            Revert::Set(Dword(1)),
        )],
    },
];

/// Whether Reclaim watches a tweak for Windows undoing it: the ads,
/// suggestions, tracking, and policies that feature updates turn back on.
/// Taskbar layout, File Explorer views, and mouse and keyboard settings
/// are left out, since people change those in Windows' own settings too,
/// and that isn't Windows undoing anything.
fn watched(tweak: &Tweak) -> bool {
    matches!(tweak.category, "privacy" | "start") || tweak.id == "classic-context-menu"
}

fn supported(tweak: &Tweak, build: u32) -> bool {
    build == 0 || build >= tweak.min_build
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TweakState {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    category: &'static str,
    applied: bool,
    requires_admin: bool,
    blocked: bool,
    restart_explorer: bool,
    sign_out: bool,
}

fn open(write: &Write) -> Option<RegKey> {
    write.hive.key().open_subkey_with_flags(write.path, KEY_READ | KEY_WOW64_64KEY).ok()
}

fn read_flags(key: Option<&RegKey>, name: &str, default: u32) -> u32 {
    key.and_then(|k| k.get_value::<String, _>(name).ok()).and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

fn is_applied(write: &Write) -> bool {
    let key = open(write);
    match write.value {
        Value::Dword(v) => key.is_some_and(|k| k.get_value::<u32, _>(write.name).is_ok_and(|cur| cur == v)),
        Value::Str(v) => key.is_some_and(|k| k.get_value::<String, _>(write.name).is_ok_and(|cur| cur == v)),
        Value::Flags { off, default } => read_flags(key.as_ref(), write.name, default) & off == 0,
    }
}

fn set(key: &RegKey, name: &str, value: Value) -> std::io::Result<()> {
    match value {
        Value::Dword(v) => key.set_value(name, &v),
        Value::Str(v) => key.set_value(name, &v.to_string()),
        Value::Flags { off, default } => key.set_value(name, &(read_flags(Some(key), name, default) & !off).to_string()),
    }
}

fn create(write: &Write) -> std::io::Result<RegKey> {
    Ok(write.hive.key().create_subkey_with_flags(write.path, winreg::enums::KEY_ALL_ACCESS | KEY_WOW64_64KEY)?.0)
}

fn apply(write: &Write) -> anyhow::Result<()> {
    set(&create(write)?, write.name, write.value)?;
    Ok(())
}

fn revert(write: &Write) -> anyhow::Result<()> {
    let hive = write.hive.key();
    match write.revert {
        Revert::Set(value) => set(&create(write)?, write.name, value)?,
        Revert::DeleteValue => {
            if let Ok(key) = hive.open_subkey_with_flags(write.path, winreg::enums::KEY_ALL_ACCESS | KEY_WOW64_64KEY) {
                match key.delete_value(write.name) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                    _ => {}
                }
            }
        }
        Revert::DeleteKey(path) => match hive.delete_subkey_all(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        },
        Revert::RestoreFlags => {
            if let Value::Flags { off, default } = write.value {
                let key = create(write)?;
                key.set_value(write.name, &(read_flags(Some(&key), write.name, default) | off).to_string())?;
            }
        }
    }
    Ok(())
}

fn is_tweak_applied(tweak: &Tweak) -> bool {
    tweak.writes.iter().all(is_applied)
}

fn state(tweak: &Tweak, elevated: bool) -> TweakState {
    let requires_admin = tweak.writes.iter().any(|w| matches!(w.hive, Hive::Machine));
    TweakState {
        id: tweak.id,
        name: tweak.name,
        description: tweak.description,
        category: tweak.category,
        applied: is_tweak_applied(tweak),
        requires_admin,
        blocked: requires_admin && !elevated,
        restart_explorer: tweak.restart_explorer,
        sign_out: tweak.sign_out,
    }
}

/// The tweaks this version of Windows has.
fn available() -> impl Iterator<Item = &'static Tweak> {
    let build = os_build();
    TWEAKS.iter().filter(move |t| supported(t, build))
}

#[tauri::command]
pub fn list_tweaks() -> Vec<TweakState> {
    let elevated = is_elevated();
    available().map(|t| state(t, elevated)).collect()
}

/// Whether `id` names a tweak this version of Windows has.
pub fn exists(id: &str) -> bool {
    available().any(|t| t.id == id)
}

/// Every tweak's ID, whichever version of Windows has it.
#[cfg(test)]
pub fn known_ids() -> Vec<&'static str> {
    TWEAKS.iter().map(|t| t.id).collect()
}

/// The IDs of the tweaks that are on.
pub fn applied_ids() -> Vec<&'static str> {
    available().filter(|t| is_tweak_applied(t)).map(|t| t.id).collect()
}

fn was_denied(e: &anyhow::Error) -> bool {
    e.downcast_ref::<std::io::Error>().is_some_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied)
}

/// Windows reads the mouse settings when the user signs in. This hands it
/// the ones just saved, so they apply now.
fn apply_mouse_now() {
    let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(MOUSE) else { return };
    let read = |name: &str, default: i32| key.get_value::<String, _>(name).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default);
    let mut params = [read("MouseThreshold1", 6), read("MouseThreshold2", 10), read("MouseSpeed", 1)];
    unsafe {
        let _ = SystemParametersInfoW(SPI_SETMOUSE, 0, Some(params.as_mut_ptr().cast()), SPIF_SENDCHANGE);
    }
}

pub fn change(id: &str, enabled: bool) -> anyhow::Result<()> {
    let tweak = TWEAKS.iter().find(|t| t.id == id).context("Unknown tweak")?;
    crate::error::ensure!(supported(tweak, os_build()), "This setting isn't in your version of Windows.");
    for write in tweak.writes {
        let result = if enabled { apply(write) } else { revert(write) };
        result.map_err(|e| if was_denied(&e) { crate::system::refusal() } else { e })?;
    }
    // A policy or Windows' own protection can quietly put a value back.
    crate::error::ensure!(!enabled || is_tweak_applied(tweak), "Windows didn't keep this change. A policy on this PC may be setting it.");
    if tweak.id == "mouse-acceleration" {
        apply_mouse_now();
    }
    let _ = remembered::update(|r| {
        if enabled && watched(tweak) {
            r.tweaks.insert(tweak.id.to_string());
        } else {
            r.tweaks.remove(tweak.id);
        }
    });
    Ok(())
}

#[tauri::command]
pub fn set_tweak(id: String, enabled: bool) -> CmdResult<()> {
    change(&id, enabled)?;
    Ok(())
}

/// Adds the watched tweaks that are already on, the first time Reclaim
/// keeps a record, so changes made with an older version count too.
pub fn seed(record: &mut Remembered) {
    if record.seeded {
        return;
    }
    record.seeded = true;
    for tweak in available().filter(|t| watched(t) && is_tweak_applied(t)) {
        record.tweaks.insert(tweak.id.to_string());
    }
}

/// Tweaks the user turned on that are off again.
pub fn drifted(record: &Remembered) -> Vec<TweakState> {
    let elevated = is_elevated();
    available().filter(|t| record.tweaks.contains(t.id) && watched(t) && !is_tweak_applied(t)).map(|t| state(t, elevated)).collect()
}

/// Restarts File Explorer so Explorer and taskbar tweaks take effect.
#[tauri::command]
pub async fn restart_explorer() -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(|| -> anyhow::Result<()> {
        if is_elevated() {
            return crate::programs::leftovers::restart_shell();
        }
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        Command::new("taskkill.exe").args(["/f", "/im", "explorer.exe"]).creation_flags(CREATE_NO_WINDOW).status()?;
        std::thread::sleep(std::time::Duration::from_millis(600));
        Command::new("explorer.exe").spawn()?;
        Ok(())
    })
    .await??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watches_what_windows_turns_back_on() {
        let tweak = |id: &str| TWEAKS.iter().find(|t| t.id == id).unwrap();
        assert!(watched(tweak("tips")) && watched(tweak("telemetry")) && watched(tweak("classic-context-menu")));
        assert!(!watched(tweak("hidden-files")) && !watched(tweak("taskbar-left")) && !watched(tweak("mouse-acceleration")));
    }

    #[test]
    fn categories_are_ones_the_ui_shows() {
        for t in TWEAKS {
            assert!(["privacy", "start", "taskbar", "explorer", "gaming", "system"].contains(&t.category), "{}", t.id);
        }
    }

    #[test]
    fn flag_tweaks_clear_and_restore_only_their_bit() {
        let key_path = format!(r"Software\Reclaim test flags {}", std::process::id());
        let write = Write {
            hive: User,
            path: Box::leak(key_path.clone().into_boxed_str()),
            name: "Flags",
            value: Flags { off: 0x4, default: 510 },
            revert: Revert::RestoreFlags,
        };
        // Missing means the default, which has the bit set.
        assert!(!is_applied(&write));
        apply(&write).unwrap();
        assert!(is_applied(&write));
        let saved = |w: &Write| open(w).unwrap().get_value::<String, _>("Flags").unwrap();
        assert_eq!(saved(&write), "506");
        // Someone who turned other options off keeps them that way.
        create(&write).unwrap().set_value("Flags", &"2".to_string()).unwrap();
        assert!(is_applied(&write));
        revert(&write).unwrap();
        assert_eq!(saved(&write), "6");
        let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&key_path);
    }

    #[test]
    fn tweak_ids_are_unique() {
        let mut ids: Vec<_> = TWEAKS.iter().map(|t| t.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), TWEAKS.len());
    }
}
