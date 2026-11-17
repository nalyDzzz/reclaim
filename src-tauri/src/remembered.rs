//! What the user changed through Reclaim that Windows might quietly undo,
//! like a tweak a feature update resets or a Store app it reinstalls. Kept
//! so Reclaim can notice later and offer to redo it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Remembered {
    /// Tweaks the user turned on.
    pub tweaks: BTreeSet<String>,
    /// Package names of Store apps the user removed.
    pub removed_apps: BTreeSet<String>,
    /// Whether the tweaks that were already on when Reclaim started keeping
    /// this have been added to it.
    pub seeded: bool,
}

static FILE: OnceLock<PathBuf> = OnceLock::new();
static LOCK: Mutex<()> = Mutex::new(());

/// Sets where the record lives. Called once at startup.
pub fn init(app_data: &Path) {
    let _ = FILE.set(app_data.join("remembered.json"));
}

fn read(file: &Path) -> Remembered {
    std::fs::read(file).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

fn write(file: &Path, data: &Remembered) -> anyhow::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Written beside it and swapped in, so a crash can't leave half a file.
    let temp = file.with_extension("json.tmp");
    std::fs::write(&temp, serde_json::to_vec_pretty(data)?)?;
    std::fs::rename(&temp, file)?;
    Ok(())
}

pub fn load() -> Remembered {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    FILE.get().map(|f| read(f)).unwrap_or_default()
}

/// Changes the record and saves it. Forgetting isn't worth failing the
/// action the user took, so callers usually ignore the error.
pub fn update(change: impl FnOnce(&mut Remembered)) -> anyhow::Result<()> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let file = FILE.get().context("Reclaim hasn't set up its data folder yet")?;
    update_file(file, change)
}

fn update_file(file: &Path, change: impl FnOnce(&mut Remembered)) -> anyhow::Result<()> {
    let mut data = read(file);
    change(&mut data);
    write(file, &data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn survives_a_round_trip_and_a_missing_or_broken_file() {
        let dir = std::env::temp_dir().join(format!("reclaim-remembered-test-{}", std::process::id()));
        let file = dir.join("remembered.json");
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(read(&file), Remembered::default());
        update_file(&file, |r| {
            r.tweaks.insert("tips".into());
            r.removed_apps.insert("Microsoft.BingNews".into());
        })
        .unwrap();
        let back = read(&file);
        assert!(back.tweaks.contains("tips") && back.removed_apps.contains("Microsoft.BingNews"));

        std::fs::write(&file, b"{ not json").unwrap();
        assert_eq!(read(&file), Remembered::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_records_from_older_versions() {
        let old: Remembered = serde_json::from_str(r#"{"tweaks":["tips"]}"#).unwrap();
        assert!(old.tweaks.contains("tips") && old.removed_apps.is_empty() && !old.seeded);
    }
}
