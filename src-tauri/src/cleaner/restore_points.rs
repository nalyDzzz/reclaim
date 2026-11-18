//! System Restore points. Each one keeps old copies of whatever has changed
//! on the drive since it was made, so together they can hold gigabytes.
//! Reclaim removes the older ones and always keeps the newest, so the PC
//! still has a way back.

use anyhow::{Context, anyhow, bail};
use serde::Deserialize;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
use windows::core::{s, w};

use super::{CleanReport, TargetReport, format_bytes};
use crate::system::powershell;

#[derive(Deserialize)]
struct Snapshot {
    points: Vec<Point>,
    /// System Restore's space on each drive it protects.
    storage: Vec<Storage>,
}

#[derive(Deserialize)]
struct Point {
    seq: u32,
    /// Milliseconds since 1970.
    created: i64,
    /// Like "Nov 10, 2026".
    date: String,
    description: String,
}

#[derive(Deserialize)]
struct Storage {
    used: u64,
    drive: String,
}

impl Snapshot {
    fn used(&self) -> u64 {
        self.storage.iter().map(|s| s.used).sum()
    }

    fn newest(&self) -> Option<&Point> {
        self.points.iter().max_by_key(|p| (p.created, p.seq))
    }

    /// "C:" or "C: and D:".
    fn drives(&self) -> String {
        let drives: Vec<&str> = self.storage.iter().filter(|s| s.used > 0 && !s.drive.is_empty()).map(|s| s.drive.as_str()).collect();
        match drives.as_slice() {
            [] => "the drive".into(),
            [one] => (*one).into(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        }
    }
}

/// Reads the restore points and the space they use. Only administrators
/// can; for anyone else both lists come back empty.
fn read() -> anyhow::Result<Snapshot> {
    const SCRIPT: &str = r#"
        $inv = [Globalization.CultureInfo]::InvariantCulture
        $points = @(Get-CimInstance -Namespace root/default -ClassName SystemRestore -ErrorAction SilentlyContinue | ForEach-Object {
            $dt = [Management.ManagementDateTimeConverter]::ToDateTime($_.CreationTime)
            [pscustomobject]@{ seq = [uint32]$_.SequenceNumber; created = ([DateTimeOffset]$dt).ToUnixTimeMilliseconds(); date = $dt.ToString('MMM d, yyyy', $inv); description = [string]$_.Description }
        })
        $letters = @{}
        Get-CimInstance Win32_Volume -ErrorAction SilentlyContinue | ForEach-Object { $letters[$_.DeviceID] = $_.DriveLetter }
        $storage = @(Get-CimInstance Win32_ShadowStorage -ErrorAction SilentlyContinue | ForEach-Object {
            [pscustomobject]@{ used = [uint64]$_.UsedSpace; drive = [string]$letters[$_.Volume.DeviceID] }
        })
        [pscustomobject]@{ points = $points; storage = $storage } | ConvertTo-Json -Depth 3 -Compress
    "#;
    let out = powershell(SCRIPT)?;
    serde_json::from_str(out.trim()).context("Couldn't read Windows' list of restore points")
}

pub fn analyze(report: &mut TargetReport, elevated: bool) {
    if !elevated {
        // Can't tell whether there are any, so the row asks for admin rights.
        report.found = true;
        return;
    }
    let snapshot = match read() {
        Ok(s) => s,
        Err(e) => {
            report.found = true;
            report.note = Some(format!("{e:#}"));
            return;
        }
    };
    let Some(newest) = snapshot.newest() else { return };
    report.found = true;
    let used = format_bytes(snapshot.used());
    let older = snapshot.points.len() - 1;
    if older == 0 {
        report.note = Some(format!(
            "System Restore uses {used} on {} for one restore point, \"{}\" from {}. It's the newest, so it stays.",
            snapshot.drives(),
            newest.description,
            newest.date
        ));
        return;
    }
    // The closing comma ends the aside, after the date's own comma.
    let kept = format!("the newest, \"{}\" from {},", newest.description, newest.date);
    // Restore points share their space, so how much goes depends on how
    // much the kept one needs. All of it is the most that can be freed.
    report.bytes = snapshot.used();
    report.up_to = true;
    let going = if older == 1 { "The older one goes".to_string() } else { format!("The {older} older ones go") };
    report.note = Some(format!(
        "System Restore uses {used} on {} for {} restore points. {going}, and {kept} stays.",
        snapshot.drives(),
        snapshot.points.len(),
    ));
}

/// Removes every restore point but the newest.
pub fn clean(report: &mut CleanReport) {
    let result = (|| -> anyhow::Result<()> {
        let before = read()?;
        let Some(newest) = before.newest() else { return Ok(()) };
        let keep = newest.seq;
        let mut failed = 0;
        let mut last_error = None;
        for point in before.points.iter().filter(|p| p.seq != keep) {
            if let Err(e) = remove(point.seq) {
                failed += 1;
                last_error = Some(e);
            }
        }
        let after = read().map(|s| s.used()).unwrap_or(before.used());
        report.freed_bytes = before.used().saturating_sub(after);
        if let Some(e) = last_error {
            bail!("Windows couldn't remove {failed} of the older restore points: {e:#}");
        }
        Ok(())
    })();
    report.error = result.err().map(|e| format!("{e:#}"));
}

/// Removes one restore point and its snapshot. srclient.dll is loaded when
/// it's needed, as Microsoft asks: some editions of Windows don't have it.
fn remove(seq: u32) -> anyhow::Result<()> {
    unsafe {
        let module = LoadLibraryExW(w!("srclient.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).context("System Restore isn't available")?;
        let remove = GetProcAddress(module, s!("SRRemoveRestorePoint")).ok_or_else(|| anyhow!("System Restore isn't available"))?;
        let remove: unsafe extern "system" fn(u32) -> u32 = std::mem::transmute(remove);
        match remove(seq) {
            0 => Ok(()),
            code => Err(std::io::Error::from_raw_os_error(code as i32).into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(seq: u32, created: i64) -> Point {
        Point { seq, created, date: String::new(), description: String::new() }
    }

    #[test]
    fn keeps_the_newest_by_date_then_number() {
        let snapshot = Snapshot { points: vec![point(9, 100), point(12, 300), point(11, 300), point(3, 50)], storage: vec![] };
        assert_eq!(snapshot.newest().map(|p| p.seq), Some(12));
        assert!(Snapshot { points: vec![], storage: vec![] }.newest().is_none());
    }

    #[test]
    fn names_the_drives_in_use() {
        let storage = |drive: &str, used| Storage { used, drive: drive.into() };
        let names = |storage| Snapshot { points: vec![], storage }.drives();
        assert_eq!(names(vec![storage("C:", 5)]), "C:");
        assert_eq!(names(vec![storage("C:", 5), storage("D:", 0), storage("E:", 1)]), "C: and E:");
        assert_eq!(names(vec![storage("", 5)]), "the drive");
    }

    #[test]
    fn parses_what_powershell_prints() {
        let json = r#"{"points":[{"seq":7,"created":1794280522000,"date":"Nov 9, 2026","description":"Windows Update"}],"storage":[{"used":6657199308,"drive":"C:"}]}"#;
        let snapshot: Snapshot = serde_json::from_str(json).unwrap();
        assert_eq!((snapshot.points.len(), snapshot.used()), (1, 6657199308));
        let empty: Snapshot = serde_json::from_str(r#"{"points":[],"storage":[]}"#).unwrap();
        assert!(empty.newest().is_none());
    }
}
