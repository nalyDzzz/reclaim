# Reclaim

An open-source Windows companion for taking your disk back: uninstall apps and the junk they leave behind, see exactly what's filling a drive, clear caches, and strip out preinstalled bloat. It covers the jobs of Revo Uninstaller, WizTree, BleachBit, and the usual debloat scripts in one app.

> Status: early development. It works, but read what each action will do before confirming it.

## What it does

**Apps.** Lists everything in the Windows uninstall registry (machine-wide 64/32-bit and per-user), with real icons and sizes measured from each app's folder. Uninstalling runs the app's own uninstaller and waits for it, including uninstallers that hand off to a helper process. Then Reclaim scans for leftovers: install folders, AppData, ProgramData, Start menu and desktop shortcuts, registry settings keys, startup entries, and App Paths registrations. Each leftover is rated Certain, Likely, or Possible; only Certain ones are pre-selected.

**Storage.** A parallel scan of any drive or folder builds a size tree you can drill into with a treemap, a sortable folder list, the largest files, and a breakdown by file type. Delete to the Recycle Bin or permanently. OneDrive online-only files count as zero, and junctions and symlinks aren't followed, so nothing is counted twice.

**Cleanup.** Temp folders, Windows Update downloads, Delivery Optimization, error reports, crash dumps, browser caches (Chrome, Edge, Brave, Vivaldi, Opera, Firefox), app caches (Discord, Slack, VS Code, Cursor, Spotify, Steam), and developer caches (npm, pnpm, Yarn, Bun, pip/uv, Cargo, NuGet, Gradle, Maven, Go). It warns when an app is open and holding its cache.

**Debloat.** Remove preinstalled Store apps, with the commonly removed ones flagged and notes on the ones some people need (Xbox, Phone Link…). Plus reversible registry switches for ads, suggestions, Bing in Start, Recall, telemetry, the classic right-click menu, and more.

## Safety model

- Leftover files go to the **Recycle Bin**, never straight to deletion.
- Registry keys and values are exported to a `.reg` backup before removal, under `%LOCALAPPDATA%\com.reclaim.desktop\registry-backups`.
- A protected-path list (`src-tauri/src/paths.rs`) blocks deletion of drive roots, Windows, Program Files itself, user profile folders, and other shared locations, no matter what the UI asks for. Registry removals have an equivalent guard (`is_protected_reg_key`).
- A folder is only treated as an app's own if its path contains a distinctive word from the app or publisher name. That keeps shared folders like Steam's icon cache from being picked up as leftovers.
- Commands that run uninstallers re-read the uninstall command from the registry instead of trusting the UI.
- Reclaim runs without admin rights by default. System-wide cleanup and HKLM leftovers ask you to restart it as administrator.

## Stack

- [Tauri 2](https://tauri.app) shell, with a ~10 MB installer that uses the WebView2 runtime already on Windows 10/11
- Rust backend: `winreg` for the registry, `windows` crate for Win32 (icons, ShellExecute, Recycle Bin, drives), `rayon` for parallel scans, `trash` for Recycle Bin moves
- React 19 + TypeScript + Tailwind CSS 4 frontend, `d3-hierarchy` for treemap layout
- Windows 11 Mica backdrop with a custom title bar

```
src/                    React UI
  features/apps/        installed apps, uninstall + leftover flow
  features/storage/     drive picker, scan progress, treemap, lists
  features/cleanup/     junk and cache cleanup
  features/debloat/     Store apps and Windows tweaks
  lib/api.ts            typed wrappers for every Rust command
src-tauri/src/
  programs/             registry reader, icons, uninstaller, leftovers
  storage/              parallel scanner and in-memory size tree
  cleaner/              cleanup targets and deletion
  debloat/              AppX packages and registry tweaks
  paths.rs              known folders and the protected-path rules
```

## Development

Prerequisites: [Rust](https://rustup.rs) (MSVC toolchain), Node 20+, pnpm, and the Visual Studio C++ Build Tools.

```powershell
pnpm install
pnpm tauri dev          # run the app with hot reload
pnpm tauri build        # build MSI and NSIS installers into src-tauri/target/release/bundle
cd src-tauri; cargo test   # backend tests
```

To test machine-wide cleanup and HKLM leftovers, use **Restart as administrator** in the sidebar; it relaunches the dev build elevated.

## Roadmap

- Read the NTFS master file table directly when elevated, for WizTree-speed scans, exact on-disk sizes, and hard-link de-duplication
- Batch uninstall and silent uninstall where supported
- Install tracing (snapshot before and after an install) for perfect leftover removal
- Duplicate file finder, and "stale" highlighting for files untouched in a year
- Browser extensions manager, startup apps manager, scheduled tasks review
- Restore points before risky operations, plus a history of everything removed
- Light theme
