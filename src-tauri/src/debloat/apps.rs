//! Preinstalled Microsoft Store (AppX) apps.

use std::path::{Path, PathBuf};

use anyhow::Context;
use base64::Engine;
use serde::{Deserialize, Serialize};
use winreg::RegKey;
use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};

use crate::error::CmdResult;
use crate::paths::{known, wildcard_match};
use crate::remembered;
use crate::system::{is_elevated, powershell};

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
    k(
        "MicrosoftWindows.Client.WebExperience",
        "Widgets",
        false,
        Some("The Widgets board and its news feed. To only hide the button, use Settings › Personalization › Taskbar"),
    ),
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

/// Packages other apps and Windows itself rely on. Removing one for your
/// own account is your call, but Reclaim never stops Windows from setting
/// them up for new accounts.
const NEVER_DEPROVISION: &[&str] = &[
    "Microsoft.WindowsStore",
    "Microsoft.StorePurchaseApp",
    "Microsoft.DesktopAppInstaller",
    "Microsoft.Winget.Source",
    "Microsoft.SecHealthUI",
    "Microsoft.ApplicationCompatibilityEnhancements",
    "Microsoft.VCLibs.*",
    "Microsoft.UI.Xaml.*",
    "Microsoft.NET.*",
    "Microsoft.WindowsAppRuntime.*",
    "Microsoft.WinAppRuntime.*",
    "MicrosoftCorporationII.WinAppRuntime.*",
    "Microsoft.*Extension",
    "Microsoft.*Extensions",
];

fn never_deprovision(name: &str) -> bool {
    let name = name.to_lowercase();
    NEVER_DEPROVISION.iter().any(|p| wildcard_match(&p.to_lowercase(), &name))
}

/// Where Windows lists the apps it sets up for every new account and puts
/// back during feature updates, by full package name.
const PROVISIONED: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Appx\AppxAllUserStore\Applications";

/// Provisioned packages as (name, full package name). A package name can't
/// contain an underscore, so it's everything before the first one.
fn provisioned() -> Vec<(String, String)> {
    let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(PROVISIONED, KEY_READ | KEY_WOW64_64KEY) else {
        return Vec::new();
    };
    key.enum_keys().flatten().filter_map(|full| Some((full.split_once('_')?.0.to_string(), full))).collect()
}

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
    /// Installed for the signed-in user. When false, only Windows' copy for
    /// new accounts is left.
    installed: bool,
    /// Windows keeps a copy it sets up for new accounts and can put back
    /// after a feature update.
    provisioned: bool,
    /// Reclaim leaves Windows' copy alone, since other apps rely on it.
    keeps_copy: bool,
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

fn installed_packages() -> anyhow::Result<Vec<PsPackage>> {
    let script = r#"
        [Console]::OutputEncoding = [Text.Encoding]::UTF8
        $ProgressPreference = 'SilentlyContinue'
        $pkgs = Get-AppxPackage | Where-Object { -not $_.IsFramework -and -not $_.NonRemovable -and "$($_.SignatureKind)" -ne 'System' -and -not $_.IsResourcePackage } |
            Select-Object Name, PackageFullName, Publisher, @{n='Version';e={"$($_.Version)"}}, InstallLocation, @{n='SignatureKind';e={"$($_.SignatureKind)"}}
        ConvertTo-Json -InputObject @($pkgs) -Compress
    "#;
    let json = powershell(script)?;
    serde_json::from_str(json.trim()).context("Couldn't read the app list from PowerShell")
}

fn known_app(name: &str) -> Option<&'static Known> {
    KNOWN_APPS.iter().find(|k| k.name.eq_ignore_ascii_case(name))
}

fn installed_app(p: PsPackage, provisioned: &[(String, String)]) -> AppxApp {
    let known = known_app(&p.name);
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
        installed: true,
        provisioned: provisioned.iter().any(|(name, _)| name.eq_ignore_ascii_case(&p.name)),
        keeps_copy: never_deprovision(&p.name),
        name: p.name,
        package_full_name: p.package_full_name,
        install_location: p.install_location,
    }
}

/// An app Windows keeps a copy of that isn't installed for this user.
fn copy_only_app(name: &str, full_name: &str) -> AppxApp {
    let known = known_app(name);
    // Full names end in the publisher ID, which is all a copy tells us.
    let publisher_id = full_name.rsplit('_').next().unwrap_or_default();
    AppxApp {
        display_name: known.map(|k| k.friendly.to_string()).unwrap_or_else(|| prettify(name)),
        publisher: if publisher_id == "8wekyb3d8bbwe" { "Microsoft Corporation".to_string() } else { String::new() },
        version: full_name.split('_').nth(1).unwrap_or_default().to_string(),
        from_store: true,
        known: known.is_some(),
        recommended: known.is_some_and(|k| k.recommended),
        note: known.and_then(|k| k.note),
        installed: false,
        provisioned: true,
        keeps_copy: false,
        name: name.to_string(),
        package_full_name: full_name.to_string(),
        install_location: None,
    }
}

fn sort_apps(apps: &mut [AppxApp]) {
    apps.sort_by(|a, b| b.recommended.cmp(&a.recommended).then(a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase())));
}

fn read_apps() -> anyhow::Result<Vec<AppxApp>> {
    let provisioned = provisioned();
    let mut apps: Vec<AppxApp> = installed_packages()?.into_iter().map(|p| installed_app(p, &provisioned)).collect();
    for (name, full) in &provisioned {
        let shown = apps.iter().any(|a| a.name.eq_ignore_ascii_case(name));
        if !shown && !never_deprovision(name) {
            apps.push(copy_only_app(name, full));
        }
    }
    sort_apps(&mut apps);
    Ok(apps)
}

#[tauri::command]
pub async fn list_appx() -> CmdResult<Vec<AppxApp>> {
    Ok(tauri::async_runtime::spawn_blocking(read_apps).await??)
}

/// The apps a setup file lists as removed: the ones removed through
/// Reclaim, and the commonly removed apps this user doesn't have.
pub fn removed_for_setup(record: &remembered::Remembered) -> anyhow::Result<Vec<String>> {
    let installed = installed_packages()?;
    let has = |name: &str| installed.iter().any(|p| p.name.eq_ignore_ascii_case(name));
    let mut names: Vec<String> = record.removed_apps.iter().filter(|n| !has(n)).cloned().collect();
    for k in KNOWN_APPS.iter().filter(|k| k.recommended && !has(k.name)) {
        if !names.iter().any(|n| n.eq_ignore_ascii_case(k.name)) {
            names.push(k.name.to_string());
        }
    }
    names.sort_by_key(|n| n.to_lowercase());
    Ok(names)
}

/// Store apps the user removed that are installed for them again.
pub fn returned(record: &remembered::Remembered) -> anyhow::Result<Vec<AppxApp>> {
    if record.removed_apps.is_empty() {
        return Ok(Vec::new());
    }
    let provisioned = provisioned();
    let mut apps: Vec<AppxApp> = installed_packages()?
        .into_iter()
        .filter(|p| record.removed_apps.iter().any(|name| name.eq_ignore_ascii_case(&p.name)))
        .map(|p| installed_app(p, &provisioned))
        .collect();
    sort_apps(&mut apps);
    Ok(apps)
}

pub(super) fn valid_package_name(name: &str) -> bool {
    !name.is_empty() && name.len() < 256 && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '~'))
}

fn ps_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn removal_script(package_full_name: Option<&str>, copies: &[String]) -> String {
    let mut script = String::from("$ProgressPreference = 'SilentlyContinue'\n");
    if let Some(full) = package_full_name {
        script.push_str(&format!("Remove-AppxPackage -Package {} -ErrorAction Stop\n", ps_quote(full)));
    }
    for copy in copies {
        script.push_str(&format!("Remove-AppxProvisionedPackage -Online -PackageName {} -ErrorAction Stop | Out-Null\n", ps_quote(copy)));
    }
    script
}

/// Removes `name` for the signed-in user (its installed package is
/// `package_full_name`, if it's installed) and, as administrator, removes
/// Windows' copy too, so it isn't set up again for new accounts or put back
/// by a feature update.
fn remove(name: &str, package_full_name: Option<&str>) -> anyhow::Result<()> {
    crate::error::ensure!(valid_package_name(name) && !name.contains('_'), "Invalid package name");
    if let Some(full) = package_full_name {
        crate::error::ensure!(valid_package_name(full) && full.split('_').next() == Some(name), "Invalid package name");
    }
    // Looked up again here rather than taken from the UI.
    let copies: Vec<String> = if is_elevated() && !never_deprovision(name) {
        provisioned().into_iter().filter(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, full)| full).collect()
    } else {
        Vec::new()
    };
    crate::error::ensure!(
        package_full_name.is_some() || !copies.is_empty(),
        "Removing Windows' copy of an app needs administrator rights."
    );

    powershell(&removal_script(package_full_name, &copies))?;
    let _ = remembered::update(|r| {
        r.removed_apps.insert(name.to_string());
    });
    Ok(())
}

#[tauri::command]
pub async fn remove_appx(name: String, package_full_name: Option<String>) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || remove(&name, package_full_name.as_deref())).await??;
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
    fn keeps_windows_copy_of_what_other_apps_need() {
        assert!(never_deprovision("Microsoft.WindowsStore"));
        assert!(never_deprovision("Microsoft.VCLibs.140.00"));
        assert!(never_deprovision("MicrosoftCorporationII.WinAppRuntime.Main.1.8"));
        assert!(never_deprovision("Microsoft.HEVCVideoExtension"));
        assert!(never_deprovision("Microsoft.VP9VideoExtensions"));
        assert!(!never_deprovision("Microsoft.BingNews"));
        assert!(!never_deprovision("MicrosoftWindows.Client.WebExperience"));
    }

    #[test]
    fn reads_provisioned_apps() {
        // Every Windows install provisions some apps; names never hold an underscore.
        for (name, full) in provisioned() {
            assert!(!name.contains('_') && full.starts_with(&format!("{name}_")), "{full}");
        }
    }

    #[test]
    fn copies_show_a_publisher_and_version() {
        let app = copy_only_app("Microsoft.BingNews", "Microsoft.BingNews_4.7.28001.0_neutral_~_8wekyb3d8bbwe");
        assert_eq!(app.display_name, "News");
        assert_eq!(app.version, "4.7.28001.0");
        assert_eq!(app.publisher, "Microsoft Corporation");
        assert!(!app.installed && app.provisioned);
    }

    #[test]
    fn removes_the_app_then_windows_copies() {
        let full = "Microsoft.BingNews_4.55.62231.0_x64__8wekyb3d8bbwe";
        let copy = "Microsoft.BingNews_4.7.28001.0_neutral_~_8wekyb3d8bbwe".to_string();
        let script = removal_script(Some(full), std::slice::from_ref(&copy));
        let lines: Vec<&str> = script.lines().collect();
        assert_eq!(lines[1], format!("Remove-AppxPackage -Package '{full}' -ErrorAction Stop"));
        assert_eq!(lines[2], format!("Remove-AppxProvisionedPackage -Online -PackageName '{copy}' -ErrorAction Stop | Out-Null"));
        // A copy-only app has nothing installed to remove first.
        assert!(!removal_script(None, &[copy]).contains("Remove-AppxPackage "));
        assert_eq!(ps_quote("it's"), "'it''s'");
    }

    #[test]
    fn validates_package_names() {
        assert!(valid_package_name("Microsoft.BingNews_4.55.62231.0_x64__8wekyb3d8bbwe"));
        assert!(!valid_package_name("x'; Remove-Item C:\\ -Recurse; '"));
    }
}
