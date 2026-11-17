//! A file holding someone's Debloat setup, to repeat it on another PC: the
//! tweaks they have on and the Store apps they removed.

use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use super::{apps, tweaks};
use crate::error::CmdResult;
use crate::remembered;

/// The file format's version. A newer Reclaim can read older files.
const VERSION: u32 = 1;
/// Far more than any real setup, small enough to read without worry.
const MAX_SIZE: u64 = 256 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetupFile {
    /// Marks the file as a Reclaim setup, and which version of the format.
    reclaim_setup: u32,
    #[serde(default)]
    tweaks: Vec<String>,
    #[serde(default)]
    removed_apps: Vec<String>,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Setup {
    tweaks: Vec<String>,
    apps: Vec<String>,
    /// Tweaks in the file that this version of Windows, or of Reclaim, doesn't have.
    unknown_tweaks: usize,
}

fn is_json(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json"))
}

#[tauri::command]
pub async fn export_setup(path: String) -> CmdResult<Setup> {
    let setup = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<Setup> {
        let path = Path::new(&path);
        crate::error::ensure!(is_json(path), "Save the setup as a .json file.");
        let file = SetupFile {
            reclaim_setup: VERSION,
            tweaks: tweaks::applied_ids().into_iter().map(String::from).collect(),
            removed_apps: apps::removed_for_setup(&remembered::load())?,
        };
        std::fs::write(path, serde_json::to_vec_pretty(&file)?).context("Couldn't save the file")?;
        Ok(Setup { tweaks: file.tweaks, apps: file.removed_apps, unknown_tweaks: 0 })
    })
    .await??;
    Ok(setup)
}

fn parse(bytes: &[u8]) -> anyhow::Result<Setup> {
    let file: SetupFile =
        serde_json::from_slice(bytes).ok().filter(|f: &SetupFile| f.reclaim_setup > 0).context("This isn't a Reclaim setup file.")?;
    crate::error::ensure!(file.reclaim_setup <= VERSION, "This setup was saved by a newer version of Reclaim. Update Reclaim to load it.");
    let (known, unknown): (Vec<String>, Vec<String>) = file.tweaks.into_iter().partition(|id| tweaks::exists(id));
    let mut apps: Vec<String> = file.removed_apps.into_iter().filter(|n| apps::valid_package_name(n) && !n.contains('_')).collect();
    apps.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    Ok(Setup { tweaks: known, apps, unknown_tweaks: unknown.len() })
}

#[tauri::command]
pub async fn read_setup(path: String) -> CmdResult<Setup> {
    let setup = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<Setup> {
        let size = std::fs::metadata(&path).context("Couldn't open the file")?.len();
        crate::error::ensure!(size <= MAX_SIZE, "This isn't a Reclaim setup file.");
        parse(&std::fs::read(&path).context("Couldn't open the file")?)
    })
    .await??;
    Ok(setup)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_setup_and_drops_what_it_cannot_use() {
        let setup = parse(
            br#"{ "reclaimSetup": 1,
                  "tweaks": ["tips", "no-such-tweak"],
                  "removedApps": ["Microsoft.BingNews", "x'; Remove-Item C:\\ -Recurse; '", "Microsoft.BingNews_1.0_x64__8wekyb3d8bbwe"] }"#,
        )
        .unwrap();
        assert_eq!(setup, Setup { tweaks: vec!["tips".into()], apps: vec!["Microsoft.BingNews".into()], unknown_tweaks: 1 });
    }

    #[test]
    fn refuses_files_that_are_not_setups() {
        assert!(parse(b"{}").is_err());
        assert!(parse(b"not json").is_err());
        assert!(parse(br#"{ "tweaks": ["tips"] }"#).is_err());
        let newer = parse(br#"{ "reclaimSetup": 99 }"#).unwrap_err().to_string();
        assert!(newer.contains("newer version"), "{newer}");
    }
}
