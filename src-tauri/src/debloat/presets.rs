//! Ready-made sets of Debloat changes, for people who'd rather not go
//! through every switch.

use serde::Serialize;

use super::tweaks;

/// What every preset turns off: ads, suggestions, and apps Windows installs
/// on its own.
const ESSENTIALS: &[&str] = &[
    "start-recommendations",
    "silent-installs",
    "tips",
    "lock-screen-ads",
    "settings-suggestions",
    "bing-search",
    "advertising-id",
    "tailored-experiences",
];

struct Definition {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    /// Tweaks on top of the essentials.
    extra: &'static [&'static str],
}

const PRESETS: &[Definition] = &[
    Definition {
        id: "essentials",
        name: "Essentials",
        description: "Turns off ads, suggestions, and apps Windows installs on its own, and removes the preinstalled apps most people never open.",
        extra: &[],
    },
    Definition {
        id: "privacy",
        name: "Privacy",
        description: "Essentials, plus the least diagnostic data your edition of Windows allows, no activity history, and no Recall snapshots.",
        extra: &["telemetry", "activity-history", "recall"],
    },
    Definition {
        id: "gaming",
        name: "Gaming",
        description: "Essentials, plus raw mouse movement and no Sticky Keys prompt in the middle of a game. Xbox apps and Game Bar stay.",
        extra: &["mouse-acceleration", "accessibility-shortcuts"],
    },
];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    /// The tweaks it turns on, leaving out any this version of Windows lacks.
    tweaks: Vec<&'static str>,
    /// Every preset removes the commonly removed Store apps.
    remove_apps: bool,
}

#[tauri::command]
pub fn list_presets() -> Vec<Preset> {
    PRESETS
        .iter()
        .map(|p| Preset {
            id: p.id,
            name: p.name,
            description: p.description,
            tweaks: ESSENTIALS.iter().chain(p.extra).copied().filter(|id| tweaks::exists(id)).collect(),
            remove_apps: true,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_name_real_tweaks() {
        let known = tweaks::known_ids();
        for id in ESSENTIALS.iter().chain(PRESETS.iter().flat_map(|p| p.extra)) {
            assert!(known.contains(id), "{id} isn't a tweak");
        }
    }
}
