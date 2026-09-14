pub mod backup;
pub mod commands;
pub mod db;
pub mod files;
pub mod gestion;
pub mod license;
pub mod models;
pub mod qr;
pub mod server;
pub mod updates;
pub mod usb;
pub mod watcher;

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
            license::assurer_debut_essai(&conn);
            let watched_folder = db::get_setting(&conn, "dossier_surveille");

            app.manage(DbState(Mutex::new(conn)));
            app.manage(server::EtatServeur::default());

            if let Some(folder) = watched_folder {
                watcher::watch_folder(app.handle().clone(), PathBuf::from(folder));
            }
            usb::watch_usb_drives(app.handle().clone());
            server::start(app.handle().clone());
            backup::start(app.handle().clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_queue,
            commands::get_historique,
            commands::rechercher_client,
            commands::get_thumbnail,
            commands::get_apercu,
            commands::get_watched_folder,
            commands::choose_watched_folder,
            commands::open_file,
            commands::print_file,
            commands::ignorer_fichier,
            commands::get_boutique_settings,
            commands::set_boutique_setting,
            commands::choisir_logo_boutique,
            commands::get_server_info,
            commands::ouvrir_parametres_partage_connexion,
            gestion::list_tarifs,
            gestion::update_tarif,
            gestion::list_historique_tarifs,
            gestion::calculer_prix,
            gestion::set_print_options,
            gestion::finaliser_commande,
            gestion::reinitialiser_compteur_imprimante,
            gestion::imprimer_recu,
            gestion::list_stock,
            gestion::ajuster_stock,
            gestion::list_depenses,
            gestion::ajouter_depense,
            gestion::list_employes,
            gestion::ajouter_employe,
            gestion::list_transactions,
            gestion::rapport_du_jour,
            gestion::rapport_reconciliation,
            gestion::exporter_transactions_csv,
            gestion::list_impayes,
            gestion::marquer_impaye_regle,
            gestion::cloturer_caisse,
            gestion::activer_mode_demo,
            gestion::desactiver_mode_demo,
            license::get_license_status,
            license::set_license_key,
            updates::verifier_mise_a_jour,
            updates::version_actuelle,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
