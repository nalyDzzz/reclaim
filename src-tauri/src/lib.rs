mod cleaner;
mod debloat;
mod error;
mod paths;
mod programs;
mod storage;
mod system;

use tauri::Manager;
use tauri::window::{Effect, EffectsBuilder};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(storage::ScanState::default())
        .manage(programs::icon::IconCache::default())
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main")
                && system::supports_mica()
            {
                let _ = window.set_effects(EffectsBuilder::new().effect(Effect::MicaDark).build());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            system::system_info,
            system::list_drives,
            system::relaunch_as_admin,
            system::reveal_path,
            system::open_path,
            programs::list_programs,
            programs::program_exists,
            programs::measure_programs,
            programs::icon::program_icon,
            programs::uninstall::run_uninstaller,
            programs::leftovers::scan_leftovers,
            programs::leftovers::remove_leftovers,
            programs::leftovers::registry_backup_dir,
            storage::start_scan,
            storage::cancel_scan,
            storage::scan_node,
            storage::scan_treemap,
            storage::scan_largest_files,
            storage::scan_file_types,
            storage::delete_scan_item,
            cleaner::cleanup_targets,
            cleaner::analyze_cleanup,
            cleaner::run_cleanup,
            debloat::apps::list_appx,
            debloat::apps::remove_appx,
            debloat::apps::appx_logo,
            debloat::tweaks::list_tweaks,
            debloat::tweaks::set_tweak,
            debloat::tweaks::restart_explorer,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Reclaim");
}
