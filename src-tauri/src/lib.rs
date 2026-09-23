pub mod backup;
pub mod bluetooth;
pub mod commands;
pub mod db;
pub mod dhcp;
pub mod dns;
pub mod files;
pub mod gestion;
pub mod hotspot;
pub mod impression;
pub mod license;
pub mod models;
pub mod obex;
pub mod pare_feu;
pub mod qr;
pub mod retention;
pub mod routeur_externe;
pub mod server;
pub mod updates;
pub mod usb;
pub mod watcher;
pub mod wifi_direct;

use db::DbState;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Trouvé sur le terrain : rien n'empêchait de lancer l'application
        // deux fois (double-clic sur l'icône par habitude, ou parce que le
        // premier lancement semblait lent). Le second processus tentait de
        // reprendre le port 4173 déjà pris par le premier et échouait,
        // désactivant silencieusement la réception par QR/Wi-Fi — jusqu'à
        // ce que le gérant s'en aperçoive devant un client. Ce plugin doit
        // être enregistré EN PREMIER (exigence de Tauri) : un second
        // lancement ne crée plus de second processus, il ramène au premier
        // au lieu de lui faire concurrence sur le même port.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(fenetre) = app.get_webview_window("main") {
                let _ = fenetre.unminimize();
                let _ = fenetre.show();
                let _ = fenetre.set_focus();
            }
        }))
        // Glisser-déposer sur la fenêtre : le chemin le plus court quand le
        // client arrive avec un câble.
        //
        // Un téléphone branché en USB n'apparaît PAS comme une clé : Windows
        // le présente en MTP, sans lettre de lecteur, et la surveillance des
        // clés (voir `usb.rs`) ne peut donc rien y voir. Restait à ouvrir le
        // téléphone dans l'Explorateur et à retrouver le dossier surveillé —
        // deux fenêtres et un chemin à mémoriser, devant un client qui
        // attend. Lâcher les fichiers sur l'application fait la même chose
        // en un geste, sans rien à configurer.
        .on_window_event(|fenetre, evenement| {
            if let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) =
                evenement
            {
                for chemin in paths {
                    // Un dossier lâché par erreur ne doit pas faire de bruit.
                    if chemin.is_file() {
                        watcher::enqueue_file(fenetre.app_handle(), chemin, "glisser", None, None);
                    }
                }
            }
        })
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
            app.manage(hotspot::EtatPointAcces::default());

            if let Some(folder) = watched_folder {
                watcher::watch_folder(app.handle().clone(), PathBuf::from(folder));
            }
            usb::watch_usb_drives(app.handle().clone());
            server::start(app.handle().clone());
            backup::start(app.handle().clone());
            retention::start(app.handle().clone());
            bluetooth::demarrer(app.handle().clone());
            // Reprend un point d'accès rallumé par Windows au démarrage du
            // PC (voir `hotspot::reprendre_point_acces_existant`) : le gérant
            // n'a alors plus rien à cliquer le matin.
            hotspot::reprendre_point_acces_existant(app.handle().clone());

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
            commands::lister_imprimantes,
            commands::ignorer_fichier,
            commands::supprimer_document,
            commands::get_boutique_settings,
            commands::set_boutique_setting,
            commands::choisir_logo_boutique,
            commands::get_server_info,
            commands::ouvrir_parametres_partage_connexion,
            commands::activer_point_acces_local,
            commands::desactiver_point_acces_local,
            commands::diagnostiquer_poste,
            commands::generer_rapport_diagnostic,
            commands::sauvegarder_maintenant,
            backup::lister_sauvegardes,
            backup::restaurer_sauvegarde,
            commands::ouvrir_dossier_donnees,
            commands::verifier_et_marquer_affichage_du_jour,
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
            gestion::rapport_hier,
            gestion::rapport_periode,
            gestion::rapport_periode_impressions,
            gestion::rapport_reconciliation,
            gestion::exporter_transactions_csv,
            gestion::list_impayes,
            gestion::marquer_impaye_regle,
            gestion::cloturer_caisse,
            gestion::activer_mode_demo,
            gestion::desactiver_mode_demo,
            license::get_license_status,
            license::set_license_key,
            license::code_installation_deja_valide,
            license::valider_code_installation,
            updates::verifier_mise_a_jour,
            updates::version_actuelle,
            updates::recuperer_nouveautes_et_marquer_vues,
            updates::marquer_version_actuelle_vue,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
