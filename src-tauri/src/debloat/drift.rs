//! Debloat changes that have come undone: a setting the user turned off
//! that's back on, or a Store app they removed that's installed again.
//! Windows feature updates do both.

use serde::Serialize;

use super::apps::{self, AppxApp};
use super::tweaks::{self, TweakState};
use crate::error::CmdResult;
use crate::remembered;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Drift {
    tweaks: Vec<TweakState>,
    apps: Vec<AppxApp>,
}

#[tauri::command]
pub async fn check_drift() -> CmdResult<Drift> {
    let drift = tauri::async_runtime::spawn_blocking(|| {
        if !remembered::load().seeded {
            let _ = remembered::update(tweaks::seed);
        }
        let record = remembered::load();
        // Without PowerShell the settings can still be checked.
        Drift { tweaks: tweaks::drifted(&record), apps: apps::returned(&record).unwrap_or_default() }
    })
    .await?;
    Ok(drift)
}

/// Stops watching these settings and apps, for when the user would rather
/// keep things the way they are now.
#[tauri::command]
pub fn forget_drift(tweaks: Vec<String>, apps: Vec<String>) -> CmdResult<()> {
    remembered::update(|r| {
        for id in &tweaks {
            r.tweaks.remove(id);
        }
        for name in &apps {
            r.removed_apps.retain(|n| !n.eq_ignore_ascii_case(name));
        }
    })?;
    Ok(())
}
