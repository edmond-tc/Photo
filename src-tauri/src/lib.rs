mod commands;
mod db;
mod files;
mod watcher;

use db::DbState;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("impossible de résoudre le dossier de données de l'application");

            let conn = db::open(&data_dir).expect("échec d'ouverture de la base SQLite");
            let watched_folder = db::get_setting(&conn, "dossier_surveille");

            app.manage(DbState(Mutex::new(conn)));

            if let Some(folder) = watched_folder {
                watcher::watch_folder(app.handle().clone(), PathBuf::from(folder));
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_queue,
            commands::get_watched_folder,
            commands::choose_watched_folder,
            commands::open_file,
            commands::print_file,
            commands::mark_processed,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
