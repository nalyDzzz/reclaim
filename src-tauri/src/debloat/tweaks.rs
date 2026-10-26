//! Reversible registry tweaks for privacy and Windows annoyances. Each one
//! records how to apply it and how to put the Windows default back.

use std::os::windows::process::CommandExt;
use std::process::Command;

use anyhow::Context;
use serde::Serialize;
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};

use crate::error::CmdResult;
use crate::system::is_elevated;

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
}

/// What restoring the Windows default means for one registry write.
#[derive(Clone, Copy)]
enum Revert {
    Set(Value),
    DeleteValue,
    DeleteKey(&'static str),
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
    writes: &'static [Write],
}

const fn w(hive: Hive, path: &'static str, name: &'static str, value: Value, revert: Revert) -> Write {
    Write { hive, path, name, value, revert }
}

const CDM: &str = r"Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager";
const ADVANCED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced";
use Hive::{Machine, User};
use Value::{Dword, Str};

const TWEAKS: &[Tweak] = &[
    Tweak {
        id: "advertising-id",
        name: "Turn off advertising ID",
        description: "Stops apps from using an ID to show you personalized ads.",
        category: "privacy",
        restart_explorer: false,
        writes: &[w(User, r"Software\Microsoft\Windows\CurrentVersion\AdvertisingInfo", "Enabled", Dword(0), Revert::Set(Dword(1)))],
    },
    Tweak {
        id: "tailored-experiences",
        name: "Turn off tailored experiences",
        description: "Stops Microsoft from using diagnostic data to show you tips, ads, and recommendations.",
        category: "privacy",
        restart_explorer: false,
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
        writes: &[w(User, r"Software\Policies\Microsoft\Windows\WindowsAI", "DisableAIDataAnalysis", Dword(1), Revert::DeleteValue)],
    },
    Tweak {
        id: "telemetry",
        name: "Send only required diagnostic data",
        description: "Sets diagnostic data to the minimum your edition of Windows allows.",
        category: "privacy",
        restart_explorer: false,
        writes: &[w(Machine, r"SOFTWARE\Policies\Microsoft\Windows\DataCollection", "AllowTelemetry", Dword(0), Revert::DeleteValue)],
    },
    Tweak {
        id: "activity-history",
        name: "Turn off activity history",
        description: "Stops Windows from recording the apps and files you use.",
        category: "privacy",
        restart_explorer: false,
        writes: &[w(Machine, r"SOFTWARE\Policies\Microsoft\Windows\System", "PublishUserActivities", Dword(0), Revert::DeleteValue)],
    },
    Tweak {
        id: "bing-search",
        name: "Keep Start search local",
        description: "Removes Bing web results from the Start menu search box.",
        category: "start",
        restart_explorer: true,
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
        writes: &[w(User, ADVANCED, "HideFileExt", Dword(0), Revert::Set(Dword(1)))],
    },
    Tweak {
        id: "hidden-files",
        name: "Show hidden files",
        description: "Shows hidden files and folders in File Explorer.",
        category: "explorer",
        restart_explorer: true,
        writes: &[w(User, ADVANCED, "Hidden", Dword(1), Revert::Set(Dword(2)))],
    },
    Tweak {
        id: "classic-context-menu",
        name: "Classic right-click menu",
        description: "Brings back the full Windows 10 right-click menu, without the \"Show more options\" step.",
        category: "explorer",
        restart_explorer: true,
        writes: &[w(
            User,
            r"Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\InprocServer32",
            "",
            Str(""),
            Revert::DeleteKey(r"Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}"),
        )],
    },
    Tweak {
        id: "end-task",
        name: "End task from the taskbar",
        description: "Adds \"End task\" to the right-click menu of taskbar apps.",
        category: "explorer",
        restart_explorer: false,
        writes: &[w(
            User,
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced\TaskbarDeveloperSettings",
            "TaskbarEndTask",
            Dword(1),
            Revert::DeleteValue,
        )],
    },
];

#[derive(Serialize)]
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
}

fn is_applied(write: &Write) -> bool {
    let Ok(key) = write.hive.key().open_subkey_with_flags(write.path, KEY_READ | KEY_WOW64_64KEY) else { return false };
    match write.value {
        Value::Dword(v) => key.get_value::<u32, _>(write.name).is_ok_and(|cur| cur == v),
        Value::Str(v) => key.get_value::<String, _>(write.name).is_ok_and(|cur| cur == v),
    }
}

fn set(key: &RegKey, name: &str, value: Value) -> std::io::Result<()> {
    match value {
        Value::Dword(v) => key.set_value(name, &v),
        Value::Str(v) => key.set_value(name, &v.to_string()),
    }
}

fn apply(write: &Write) -> anyhow::Result<()> {
    let (key, _) = write.hive.key().create_subkey_with_flags(write.path, winreg::enums::KEY_ALL_ACCESS | KEY_WOW64_64KEY)?;
    set(&key, write.name, write.value)?;
    Ok(())
}

fn revert(write: &Write) -> anyhow::Result<()> {
    let hive = write.hive.key();
    match write.revert {
        Revert::Set(value) => {
            let (key, _) = hive.create_subkey_with_flags(write.path, winreg::enums::KEY_ALL_ACCESS | KEY_WOW64_64KEY)?;
            set(&key, write.name, value)?;
        }
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
    }
    Ok(())
}

#[tauri::command]
pub fn list_tweaks() -> Vec<TweakState> {
    let elevated = is_elevated();
    TWEAKS
        .iter()
        .map(|t| {
            let requires_admin = t.writes.iter().any(|w| matches!(w.hive, Hive::Machine));
            TweakState {
                id: t.id,
                name: t.name,
                description: t.description,
                category: t.category,
                applied: t.writes.iter().all(is_applied),
                requires_admin,
                blocked: requires_admin && !elevated,
                restart_explorer: t.restart_explorer,
            }
        })
        .collect()
}

#[tauri::command]
pub fn set_tweak(id: String, enabled: bool) -> CmdResult<()> {
    let tweak = TWEAKS.iter().find(|t| t.id == id).context("Unknown tweak")?;
    for write in tweak.writes {
        let result = if enabled { apply(write) } else { revert(write) };
        result.map_err(|e| {
            if matches!(write.hive, Hive::Machine) {
                anyhow::anyhow!("Needs administrator rights. Restart Reclaim as administrator to change this. ({e})")
            } else {
                e
            }
        })?;
    }
    Ok(())
}

/// Restarts File Explorer so Explorer and taskbar tweaks take effect.
#[tauri::command]
pub async fn restart_explorer() -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(|| -> anyhow::Result<()> {
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
    fn tweak_ids_are_unique() {
        let mut ids: Vec<_> = TWEAKS.iter().map(|t| t.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), TWEAKS.len());
    }
}
