pub mod activite;
pub mod appel_ble;
pub mod arrivees;
pub mod gardien_wifi;
pub mod backup;
pub mod balise_ble;
pub mod bluetooth;
pub mod canal_bt;
pub mod commands;
pub mod controle_impressions;
pub mod db;
pub mod dhcp;
pub mod dns;
pub mod etat_poste;
pub mod files;
pub mod gestion;
pub mod hotspot;
pub mod impression;
pub mod license;
pub mod mdns;
pub mod models;
pub mod obex;
pub mod pare_feu;
pub mod permanence;
pub mod point_acces_mobile;
pub mod qr;
pub mod reception_directe;
pub mod retention;
pub mod routeur_externe;
pub mod server;
pub mod signature_maj;
pub mod snmp;
pub mod telephone_usb;
pub mod updates;
pub mod usb;
pub mod watcher;
pub mod wifi_direct;

use db::DbState;
use std::path::PathBuf;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Lancée par la tâche Windows de démarrage : rallumer le Wi-Fi de la
    // boutique, sans fenêtre, et s'arrêter là (voir hotspot.rs).
    if std::env::args().any(|a| a == hotspot::ARGUMENT_DEMARRAGE_WIFI) {
        hotspot::executer_demarrage_wifi();
        return;
    }
    // Relance de sécurité après un arrêt volontaire : on ne démarre pas.
    let lancement = permanence::lancement(&std::env::args().collect::<Vec<_>>());
    if permanence::doit_sortir(lancement) {
        return;
    }
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
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // Windows (ouverture de session) ou la veille de sécurité qui
            // rappellent le logiciel déjà ouvert : rien à montrer.
            if permanence::lancement(&args) != permanence::Lancement::Normal {
                return;
            }
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
            // La croix cache la fenêtre : le logiciel doit continuer de
            // recevoir. Voir permanence.rs.
            if let tauri::WindowEvent::CloseRequested { api, .. } = evenement {
                api.prevent_close();
                let _ = fenetre.hide();
                permanence::croix_cliquee(fenetre.app_handle());
                return;
            }
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
        .setup(move |app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("impossible de résoudre le dossier de données de l'application");

            let conn = db::open(&data_dir).expect("échec d'ouverture de la base SQLite");
            license::assurer_debut_essai(&conn);
            let watched_folder = db::get_setting(&conn, "dossier_surveille");

            app.manage(DbState(db::VerrouSain::new(conn)));
            app.manage(server::EtatServeur::default());
            app.manage(hotspot::EtatPointAcces::default());
            permanence::demarrer(app.handle(), lancement);

            if let Some(folder) = watched_folder {
                watcher::watch_folder(app.handle().clone(), PathBuf::from(folder));
            }
            usb::watch_usb_drives(app.handle().clone());
            telephone_usb::surveiller_telephones(app.handle().clone());
            activite::surveiller(app.handle().clone());
            server::start(app.handle().clone());
            backup::start(app.handle().clone());
            retention::start(app.handle().clone());
            bluetooth::demarrer(app.handle().clone());
            // Réception directe (le PC rejoint le réseau du téléphone) : elle
            // ne sert que si le Wi-Fi de la boutique ne tourne pas — PC qui
            // ne sait pas en créer. Le téléphone l'appelle par Bluetooth.
            appel_ble::demarrer();
            canal_bt::demarrer(app.handle().clone());
            reception_directe::demarrer(app.handle().clone());
            arrivees::demarrer(app.handle().clone());
            hotspot::rafraichir_script_demarrage();
            hotspot::remplacer_ancienne_tache_demarrage();
            gardien_wifi::demarrer(app.handle().clone());
            std::thread::spawn(controle_impressions::fenetre_impression_windows_dans_les_navigateurs);
            mdns::demarrer();
            balise_ble::demarrer(app.handle().clone());
            // Reprend un point d'accès rallumé par Windows au démarrage du
            // PC (voir `hotspot::reprendre_point_acces_existant`) : le gérant
            // n'a alors plus rien à cliquer le matin.
            hotspot::reprendre_point_acces_existant(app.handle().clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_queue,
            commands::definir_etape,
            commands::lire_vocal,
            commands::get_historique,
            commands::rechercher_client,
            commands::get_thumbnail,
            commands::get_apercu,
            commands::get_watched_folder,
            commands::choose_watched_folder,
            commands::open_file,
            commands::mise_a_jour_recue,
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
            etat_poste::etat_du_poste,
            etat_poste::reparer_poste,
            etat_poste::rapport_assistance,
            etat_poste::qr_whatsapp_support,
            permanence::get_ouvrir_avec_windows,
            permanence::set_ouvrir_avec_windows,
            permanence::arreter_le_logiciel,
            usb::documents_cle_usb,
            usb::relire_cles_usb,
            usb::importer_documents_usb,
            telephone_usb::documents_whatsapp_telephone,
            telephone_usb::importer_documents_telephone,
            controle_impressions::controle_impressions,
            controle_impressions::activer_controle_impressions,
            activite::activite_du_jour,
            reception_directe::reception_directe_etat,
            gardien_wifi::wifi_verifier_maintenant,
            commands::oublier_programmes_ouverture,
            commands::assistant_compteurs,
            commands::assistant_methode_actuelle,
            commands::assistant_pare_feux,
            commands::assistant_methode_suivante,
            reception_directe::reception_directe_regler,
            reception_directe::reception_directe_preparer,
            reception_directe::reception_directe_qr,
            reception_directe::reception_directe_ouvrir_localisation,
            commands::journal_des_telephones,
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
