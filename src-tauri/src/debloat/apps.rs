//! Preinstalled Microsoft Store (AppX) apps.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::error::CmdResult;
use crate::paths::known;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

struct Known {
    name: &'static str,
    friendly: &'static str,
    recommended: bool,
    note: Option<&'static str>,
}

const fn k(name: &'static str, friendly: &'static str, recommended: bool, note: Option<&'static str>) -> Known {
    Known { name, friendly, recommended, note }
}

/// Apps commonly removed by debloat tools. `recommended` marks the ones
/// nearly everyone can drop; the rest are useful to some people.
const KNOWN_APPS: &[Known] = &[
    k("Microsoft.BingNews", "News", true, None),
    k("Microsoft.BingWeather", "Weather", true, None),
    k("Microsoft.BingSearch", "Bing Search", true, None),
    k("Microsoft.GetHelp", "Get Help", true, None),
    k("Microsoft.Getstarted", "Tips", true, None),
    k("Microsoft.MicrosoftSolitaireCollection", "Solitaire & Casual Games", true, None),
    k("Microsoft.People", "People", true, None),
    k("Microsoft.WindowsFeedbackHub", "Feedback Hub", true, None),
    k("Microsoft.WindowsMaps", "Maps", true, None),
    k("Microsoft.ZuneVideo", "Movies & TV", true, None),
    k("Microsoft.MicrosoftOfficeHub", "Microsoft 365 Copilot (Office hub)", true, None),
    k("Microsoft.Todos", "Microsoft To Do", false, None),
    k("Microsoft.PowerAutomateDesktop", "Power Automate", true, None),
    k("Clipchamp.Clipchamp", "Clipchamp", true, None),
    k("MicrosoftTeams", "Microsoft Teams (personal)", true, None),
    k("MSTeams", "Microsoft Teams", false, Some("Keep if you use Teams for work or school")),
    k("Microsoft.Windows.DevHome", "Dev Home", true, Some("Retired by Microsoft")),
    k("Microsoft.OutlookForWindows", "Outlook (new)", false, None),
    k("MicrosoftCorporationII.MicrosoftFamily", "Microsoft Family", true, None),
    k("MicrosoftCorporationII.QuickAssist", "Quick Assist", false, Some("Lets someone remotely help you")),
    k("Microsoft.549981C3F5F10", "Cortana", true, Some("Retired by Microsoft")),
    k("Microsoft.YourPhone", "Phone Link", false, Some("Keep if you connect your phone")),
    k("Microsoft.WindowsSoundRecorder", "Sound Recorder", false, None),
    k("Microsoft.WindowsAlarms", "Clock", false, None),
    k("Microsoft.Copilot", "Copilot", false, None),
    k("Microsoft.Windows.Ai.Copilot.Provider", "Copilot provider", false, None),
    k("Microsoft.MicrosoftStickyNotes", "Sticky Notes", false, None),
    k("Microsoft.ZuneMusic", "Media Player", false, Some("The default music and video player")),
    k("Microsoft.WindowsCamera", "Camera", false, None),
    k("Microsoft.Paint", "Paint", false, None),
    k("Microsoft.ScreenSketch", "Snipping Tool", false, None),
    k("Microsoft.WindowsCalculator", "Calculator", false, None),
    k("Microsoft.WindowsNotepad", "Notepad", false, None),
    k("Microsoft.Windows.Photos", "Photos", false, None),
    k("Microsoft.WindowsTerminal", "Terminal", false, None),
    k("Microsoft.GamingApp", "Xbox", false, Some("Needed for PC Game Pass")),
    k("Microsoft.XboxGamingOverlay", "Game Bar", false, Some("Used for game capture and the Win+G overlay")),
    k("Microsoft.XboxGameOverlay", "Xbox Game Overlay", false, Some("Used by Xbox and Game Pass games")),
    k("Microsoft.XboxIdentityProvider", "Xbox Identity Provider", false, Some("Needed to sign in to Xbox games")),
    k("Microsoft.XboxSpeechToTextOverlay", "Xbox Speech to Text", false, None),
    k("Microsoft.Xbox.TCUI", "Xbox TCUI", false, Some("Used by Xbox and Game Pass games")),
    k("Microsoft.MixedReality.Portal", "Mixed Reality Portal", true, None),
    k("Microsoft.SkypeApp", "Skype", true, Some("Retired by Microsoft")),
    k("Microsoft.Microsoft3DViewer", "3D Viewer", true, None),
    k("Microsoft.MSPaint", "Paint 3D", true, None),
    k("Microsoft.Print3D", "Print 3D", true, None),
    k("Microsoft.Wallet", "Wallet", true, None),
    k("Microsoft.Messaging", "Messaging", true, None),
    k("Microsoft.OneConnect", "Mobile Plans", true, None),
    k("Microsoft.WindowsCommunicationsApps", "Mail and Calendar", true, Some("Replaced by the new Outlook")),
    k("Microsoft.BingFinance", "Money", true, None),
    k("Microsoft.BingSports", "Sports", true, None),
    k("Microsoft.BingTravel", "Travel", true, None),
    k("Microsoft.BingFoodAndDrink", "Food & Drink", true, None),
    k("Microsoft.BingHealthAndFitness", "Health & Fitness", true, None),
    k("Microsoft.News", "News", true, None),
    k("Microsoft.StartExperiencesApp", "Start Experiences (Widgets content)", false, None),
    k("SpotifyAB.SpotifyMusic", "Spotify", true, Some("Preinstalled promo copy")),
    k("Disney.37853FC22B2CE", "Disney+", true, Some("Preinstalled promo")),
    k("AmazonVideo.PrimeVideo", "Prime Video", true, Some("Preinstalled promo")),
    k("BytedancePte.Ltd.TikTok", "TikTok", true, Some("Preinstalled promo")),
    k("Facebook.Facebook", "Facebook", true, Some("Preinstalled promo")),
    k("Facebook.Instagram", "Instagram", true, Some("Preinstalled promo")),
    k("king.com.CandyCrushSaga", "Candy Crush Saga", true, Some("Preinstalled promo")),
    k("king.com.CandyCrushSodaSaga", "Candy Crush Soda Saga", true, Some("Preinstalled promo")),
    k("7EE7776C.LinkedInforWindows", "LinkedIn", true, Some("Preinstalled promo")),
];

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PsPackage {
    name: String,
    package_full_name: String,
    publisher: Option<String>,
    version: Option<String>,
    install_location: Option<String>,
    signature_kind: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppxApp {
    name: String,
    display_name: String,
    package_full_name: String,
    publisher: String,
    version: String,
    install_location: Option<String>,
    from_store: bool,
    known: bool,
    recommended: bool,
    note: Option<&'static str>,
}

fn powershell(script: &str) -> anyhow::Result<String> {
    let out = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .context("Couldn't start PowerShell")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let first = err.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("PowerShell reported an error");
        anyhow::bail!("{first}");
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// "Microsoft.BingWeather" -> "Bing Weather"
fn prettify(name: &str) -> String {
    let base = name.split_once('.').map(|(_, rest)| rest).unwrap_or(name).replace('.', " ");
    let mut out = String::new();
    let chars: Vec<char> = base.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let prev = i.checked_sub(1).map(|j| chars[j]);
        let next = chars.get(i + 1);
        if c.is_uppercase() && prev.is_some_and(|p| p.is_lowercase() || (p.is_uppercase() && next.is_some_and(|n| n.is_lowercase()))) {
            out.push(' ');
        }
        out.push(c);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn manifest_display_name(install_location: &str) -> Option<String> {
    let xml = std::fs::read_to_string(Path::new(install_location).join("AppxManifest.xml")).ok()?;
    let props = &xml[xml.find("<Properties>")?..];
    let start = props.find("<DisplayName>")? + "<DisplayName>".len();
    let end = props[start..].find("</DisplayName>")? + start;
    let name = props[start..end].trim();
    (!name.is_empty() && !name.starts_with("ms-resource:")).then(|| name.replace("&amp;", "&"))
}

fn publisher_name(raw: &str) -> String {
    raw.split(',').find_map(|part| part.trim().strip_prefix("CN=")).unwrap_or(raw).trim_matches('"').to_string()
}

#[tauri::command]
pub async fn list_appx() -> CmdResult<Vec<AppxApp>> {
    let apps = tauri::async_runtime::spawn_blocking(|| -> anyhow::Result<Vec<AppxApp>> {
        let script = r#"
            [Console]::OutputEncoding = [Text.Encoding]::UTF8
            $ProgressPreference = 'SilentlyContinue'
            $pkgs = Get-AppxPackage | Where-Object { -not $_.IsFramework -and -not $_.NonRemovable -and "$($_.SignatureKind)" -ne 'System' -and -not $_.IsResourcePackage } |
                Select-Object Name, PackageFullName, Publisher, @{n='Version';e={"$($_.Version)"}}, InstallLocation, @{n='SignatureKind';e={"$($_.SignatureKind)"}}
            ConvertTo-Json -InputObject @($pkgs) -Compress
        "#;
        let json = powershell(script)?;
        let packages: Vec<PsPackage> = serde_json::from_str(json.trim()).context("Couldn't read the app list from PowerShell")?;
        let mut apps: Vec<AppxApp> = packages
            .into_iter()
            .map(|p| {
                let known = KNOWN_APPS.iter().find(|k| k.name.eq_ignore_ascii_case(&p.name));
                let display_name = known
                    .map(|k| k.friendly.to_string())
                    .or_else(|| p.install_location.as_deref().and_then(manifest_display_name))
                    .unwrap_or_else(|| prettify(&p.name));
                AppxApp {
                    display_name,
                    publisher: p.publisher.as_deref().map(publisher_name).unwrap_or_default(),
                    version: p.version.unwrap_or_default(),
                    from_store: p.signature_kind.as_deref() == Some("Store"),
                    known: known.is_some(),
                    recommended: known.is_some_and(|k| k.recommended),
                    note: known.and_then(|k| k.note),
                    name: p.name,
                    package_full_name: p.package_full_name,
                    install_location: p.install_location,
                }
            })
            .collect();
        apps.sort_by(|a, b| b.recommended.cmp(&a.recommended).then(a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase())));
        Ok(apps)
    })
    .await??;
    Ok(apps)
}

fn valid_package_name(name: &str) -> bool {
    !name.is_empty() && name.len() < 256 && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '~'))
}

#[tauri::command]
pub async fn remove_appx(package_full_name: String) -> CmdResult<()> {
    crate::error::ensure!(valid_package_name(&package_full_name), "Invalid package name");
    tauri::async_runtime::spawn_blocking(move || {
        powershell(&format!(
            "$ProgressPreference = 'SilentlyContinue'; Remove-AppxPackage -Package '{package_full_name}' -ErrorAction Stop"
        ))
    })
    .await??;
    Ok(())
}

/// The app's Store logo as a data URL.
#[tauri::command]
pub async fn appx_logo(install_location: String) -> CmdResult<Option<String>> {
    let logo = tauri::async_runtime::spawn_blocking(move || {
        let dir = PathBuf::from(&install_location);
        let k = known();
        let allowed = [k.program_files.join("WindowsApps"), k.windows.join("SystemApps")];
        if !allowed.iter().any(|root| dir.starts_with(root)) {
            return None;
        }
        let xml = std::fs::read_to_string(dir.join("AppxManifest.xml")).ok()?;
        let props = &xml[xml.find("<Properties>")?..];
        let start = props.find("<Logo>")? + "<Logo>".len();
        let rel = props[start..start + props[start..].find("</Logo>")?].trim().replace('/', "\\");
        let logo = dir.join(&rel);
        let file = if logo.exists() { Some(logo) } else { find_scaled_variant(&logo) }?;
        let bytes = std::fs::read(file).ok()?;
        Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
    })
    .await?;
    Ok(logo)
}

/// Store logos ship as `StoreLogo.scale-200.png` etc. Prefer larger scales.
fn find_scaled_variant(logo: &Path) -> Option<PathBuf> {
    let dir = logo.parent()?;
    let stem = logo.file_stem()?.to_string_lossy().to_lowercase();
    let mut variants: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            name.starts_with(&format!("{stem}.")) && name.ends_with(".png")
        })
        .collect();
    let rank = |p: &PathBuf| {
        let n = p.to_string_lossy().to_lowercase();
        ["scale-200", "scale-150", "scale-125", "scale-100"].iter().position(|s| n.contains(s)).unwrap_or(9)
    };
    variants.sort_by_key(rank);
    variants.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prettifies_names() {
        assert_eq!(prettify("Microsoft.BingWeather"), "Bing Weather");
        assert_eq!(prettify("MicrosoftCorporationII.QuickAssist"), "Quick Assist");
        assert_eq!(prettify("5319275A.WhatsAppDesktop"), "Whats App Desktop");
    }

    #[test]
    fn validates_package_names() {
        assert!(valid_package_name("Microsoft.BingNews_4.55.62231.0_x64__8wekyb3d8bbwe"));
        assert!(!valid_package_name("x'; Remove-Item C:\\ -Recurse; '"));
    }
}
