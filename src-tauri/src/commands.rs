use crate::db::{self, DbState};
use crate::files;
use crate::impression;
use crate::models::QueueItem;
use crate::qr;
use crate::watcher;
use rusqlite::params;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

fn lire_ligne(row: &rusqlite::Row) -> rusqlite::Result<QueueItem> {
    let finitions_json: Option<String> = row.get(16)?;
    let finitions = finitions_json
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();

    Ok(QueueItem {
        id: row.get(0)?,
        original_name: row.get(1)?,
        path: row.get(2)?,
        client_name: row.get(3)?,
        client_telephone: row.get(4)?,
        source: row.get(5)?,
        kind: row.get(6)?,
        status: row.get(7)?,
        received_at: row.get(8)?,
        taille_octets: row.get(9)?,
        protege: row.get(10)?,
        format_detecte: row.get(11)?,
        copies: row.get(12)?,
        couleur: row.get(13)?,
        format_papier: row.get(14)?,
        plage_pages: row.get(15)?,
        finitions,
        prix: row.get(17)?,
        employe: row.get(18)?,
        raison_ignore: row.get(19)?,
        document_supprime: row.get(20)?,
        impression_confirmee: row.get(21)?,
        pages_imprimees: row.get(22)?,
        impression_erreur: row.get(23)?,
        recto_verso: row.get(24)?,
        impression_couleur_reelle: row.get(25)?,
        impression_recto_verso_reelle: row.get(26)?,
        impression_format_reel: row.get(27)?,
        impression_poste: row.get(28)?,
        impression_imprimante_reelle: row.get(29)?,
        impression_ecarts: row.get(30)?,
        pages_document: row.get(31)?,
    })
}

const COLONNES_QUEUE: &str = "id, original_name, path, client_name, client_telephone, source, kind,
     status, received_at, taille_octets, protege, format_detecte, copies, couleur, format_papier,
     plage_pages, finitions, prix, employe, raison_ignore, document_supprime,
     impression_confirmee, pages_imprimees, impression_erreur,
     recto_verso, impression_couleur_reelle, impression_recto_verso_reelle, impression_format_reel,
     impression_poste, impression_imprimante_reelle, impression_ecarts, pages_document";

#[tauri::command]
pub fn get_queue(state: State<DbState>) -> Result<Vec<QueueItem>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let sql = format!(
        "SELECT {COLONNES_QUEUE} FROM files_queue WHERE status = 'en_attente' ORDER BY received_at ASC"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], lire_ligne).map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_historique(state: State<DbState>, limite: i64) -> Result<Vec<QueueItem>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let sql = format!(
        "SELECT {COLONNES_QUEUE} FROM files_queue WHERE status = 'traite'
         ORDER BY received_at DESC LIMIT ?1"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![limite], lire_ligne)
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn rechercher_client(state: State<DbState>, terme: String) -> Result<Vec<QueueItem>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let motif = format!("%{terme}%");
    let sql = format!(
        "SELECT {COLONNES_QUEUE} FROM files_queue
         WHERE client_name LIKE ?1 OR client_telephone LIKE ?1 OR original_name LIKE ?1
         ORDER BY received_at DESC LIMIT 100"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![motif], lire_ligne)
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_thumbnail(state: State<DbState>, id: i64) -> Result<Option<String>, String> {
    let path = queue_item_path(&state, id)?;
    Ok(files::miniature_base64(&path))
}

/// Pour l'aperçu intégré avant impression (voir modal-apercu côté
/// interface) : évite d'avoir à ouvrir une autre application pour
/// simplement regarder le document.
#[tauri::command]
pub fn get_apercu(state: State<DbState>, id: i64) -> Result<String, String> {
    let path = queue_item_path(&state, id)?;
    files::apercu_data_uri(&path)
}

#[tauri::command]
pub fn get_watched_folder(state: State<DbState>) -> Result<Option<String>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    Ok(db::get_setting(&conn, "dossier_surveille"))
}

#[tauri::command]
pub async fn choose_watched_folder(app: AppHandle) -> Result<Option<String>, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog().file().pick_folder(move |folder| {
        let _ = tx.send(folder);
    });
    let picked = rx.recv().map_err(|e| e.to_string())?;

    let Some(folder_path) = picked else {
        return Ok(None);
    };
    let path_str = folder_path.to_string();

    {
        let state = app.state::<DbState>();
        let conn = state.0.lock().map_err(|e| e.to_string())?;
        db::set_setting(&conn, "dossier_surveille", &path_str).map_err(|e| e.to_string())?;
    }

    watcher::watch_folder(app.clone(), PathBuf::from(&path_str));
    Ok(Some(path_str))
}

#[tauri::command]
pub async fn choisir_logo_boutique(app: AppHandle) -> Result<Option<String>, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog()
        .file()
        .add_filter("Image", &["png", "jpg", "jpeg", "bmp"])
        .pick_file(move |fichier| {
            let _ = tx.send(fichier);
        });
    let picked = rx.recv().map_err(|e| e.to_string())?;

    let Some(chemin) = picked else {
        return Ok(None);
    };
    let path_str = chemin.to_string();

    let state = app.state::<DbState>();
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    db::set_setting(&conn, "boutique_logo_chemin", &path_str).map_err(|e| e.to_string())?;
    Ok(Some(path_str))
}

#[tauri::command]
pub fn open_file(state: State<DbState>, id: i64) -> Result<(), String> {
    let path = queue_item_path(&state, id)?;
    files::shell_open(&path, "open")
}

/// `imprimante` : nom exact d'une imprimante choisie dans la liste
/// déroulante (voir `lister_imprimantes`) — utile aux gérants qui changent
/// d'imprimante en cours de journée. `None` : comportement inchangé,
/// imprimante par défaut du PC.
#[tauri::command]
pub fn print_file(
    app: AppHandle,
    state: State<DbState>,
    id: i64,
    imprimante: Option<String>,
) -> Result<(), String> {
    let path = queue_item_path(&state, id)?;
    let nom_original: String = {
        let conn = state.0.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT original_name FROM files_queue WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|_| "Fichier introuvable dans la file d'attente".to_string())?
    };
    match &imprimante {
        Some(nom) => files::shell_print_vers(&path, nom)?,
        None => files::shell_open(&path, "print")?,
    }
    // Ne bloque jamais le clic "Imprimer" : la confirmation se fait en
    // arrière-plan et prévient l'écran quand elle est connue (voir
    // impression.rs). Si le fichier venait à être introuvable dans la file
    // (course improbable avec une suppression concurrente), la confirmation
    // n'aura simplement personne à mettre à jour.
    impression::confirmer_en_arriere_plan(app, id, nom_original, imprimante);
    Ok(())
}

/// Liste des imprimantes installées sur ce PC (locales et réseau), pour la
/// liste déroulante à côté du bouton "Imprimer" — la même liste que celle
/// des paramètres Windows, rien à configurer côté application.
#[tauri::command]
pub fn lister_imprimantes() -> Vec<String> {
    impression::imprimantes_disponibles()
}

/// Retire un fichier de la file sans encaissement (ex: format non
/// supporté, doublon, client absent). Pour une commande payante, voir
/// `gestion::finaliser_commande`. La raison est obligatoire et
/// définitivement enregistrée : aucun fichier reçu ne peut disparaître de
/// la file sans laisser une trace expliquant pourquoi (cf.
/// docs/fonctionnalites-confiance.md).
#[tauri::command]
pub fn ignorer_fichier(state: State<DbState>, id: i64, raison: String) -> Result<(), String> {
    let raison = raison.trim();
    if raison.is_empty() {
        return Err("Une raison est obligatoire pour ignorer un fichier".to_string());
    }
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE files_queue SET status = 'traite', raison_ignore = ?1 WHERE id = ?2",
        params![raison, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Supprime définitivement le document d'un client, à sa demande — utilisable
/// par n'importe quel gérant, sans mot de passe technique (contrairement à
/// `ouvrir_dossier_donnees`, réservé au porteur du projet). Seul le document
/// disparaît : la ligne de comptabilité (montant, date) reste, elle ne
/// contient jamais le contenu du fichier.
///
/// Réservé aux commandes déjà traitées : un document encore en attente n'a
/// pas encore été servi, le supprimer perdrait la commande elle-même.
#[tauri::command]
pub fn supprimer_document(state: State<DbState>, id: i64) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;

    let (chemin, statut): (String, String) = conn
        .query_row(
            "SELECT path, status FROM files_queue WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "Document introuvable".to_string())?;

    if statut != "traite" {
        return Err(
            "Cette commande n'a pas encore été traitée — on ne peut supprimer que le document d'une commande déjà servie.".to_string(),
        );
    }

    // Le fichier peut déjà être absent (purge automatique, ou déjà
    // supprimé) : ce n'est pas une erreur, l'objectif est déjà atteint.
    if let Err(e) = std::fs::remove_file(&chemin) {
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(format!("Impossible de supprimer le fichier : {e}"));
        }
    }

    conn.execute(
        "UPDATE files_queue SET document_supprime = 1 WHERE id = ?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_boutique_settings(state: State<DbState>) -> Result<serde_json::Value, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "nom": db::get_setting(&conn, "boutique_nom"),
        "whatsapp": db::get_setting(&conn, "boutique_whatsapp"),
        "url_verification_maj": db::get_setting(&conn, "url_verification_maj"),
        "dossier_sauvegarde": db::get_setting(&conn, "dossier_sauvegarde"),
        "logo_chemin": db::get_setting(&conn, "boutique_logo_chemin"),
        "wifi_ssid": db::get_setting(&conn, "wifi_ssid"),
        "wifi_mot_de_passe": db::get_setting(&conn, "wifi_mot_de_passe"),
        "fidelite_seuil_visites": db::get_setting(&conn, "fidelite_seuil_visites"),
        "fidelite_remise_pourcent": db::get_setting(&conn, "fidelite_remise_pourcent"),
        "bluetooth_nom": db::get_setting(&conn, "bluetooth_nom"),
        "retention_jours": db::get_setting(&conn, "retention_jours"),
        "visite_guidee_vue": db::get_setting(&conn, "visite_guidee_vue"),
    }))
}

#[tauri::command]
pub fn set_boutique_setting(
    state: State<DbState>,
    cle: String,
    valeur: String,
) -> Result<(), String> {
    const CLES_AUTORISEES: &[&str] = &[
        "boutique_nom",
        "boutique_whatsapp",
        "url_verification_maj",
        "dossier_sauvegarde",
        "wifi_ssid",
        "wifi_mot_de_passe",
        "fidelite_seuil_visites",
        "fidelite_remise_pourcent",
        "bluetooth_nom",
        "retention_jours",
        // Visite guidée déjà suivie : évite de la reproposer à chaque
        // démarrage (voir demarrerVisite, côté interface).
        "visite_guidee_vue",
    ];
    if !CLES_AUTORISEES.contains(&cle.as_str()) {
        return Err("réglage inconnu".to_string());
    }

    // Réglages chiffrés : refusés plutôt que corrigés en douce, pour que le
    // gérant sache tout de suite que sa saisie n'a pas été prise en compte.
    let valeur = valeur.trim();
    match cle.as_str() {
        "fidelite_seuil_visites" if !valeur.is_empty() => {
            let n: i64 = valeur
                .parse()
                .map_err(|_| "Le nombre de commandes doit être un chiffre.".to_string())?;
            if !(1..=1000).contains(&n) {
                return Err("Le nombre de commandes doit être entre 1 et 1000.".to_string());
            }
        }
        "fidelite_remise_pourcent" if !valeur.is_empty() => {
            let n: i64 = valeur
                .parse()
                .map_err(|_| "La réduction doit être un chiffre.".to_string())?;
            if !(0..=100).contains(&n) {
                return Err("La réduction doit être entre 0 et 100 %.".to_string());
            }
        }
        "retention_jours" if !valeur.is_empty() => {
            let n: i64 = valeur
                .parse()
                .map_err(|_| "La durée doit être un nombre de jours.".to_string())?;
            if !(0..=3650).contains(&n) {
                return Err("La durée doit être entre 0 et 3650 jours.".to_string());
            }
        }
        _ => {}
    }

    let conn = state.0.lock().map_err(|e| e.to_string())?;
    db::set_setting(&conn, &cle, valeur).map_err(|e| e.to_string())
}

/// Pour les messages d'accueil / résumé de fin de journée : ne les montrer
/// qu'une fois par jour civil. Renvoie true (et marque comme fait) la
/// première fois qu'on l'appelle pour une `cle` donnée un jour donné ;
/// false les fois suivantes ce même jour.
#[tauri::command]
pub fn verifier_et_marquer_affichage_du_jour(
    state: State<DbState>,
    cle: String,
) -> Result<bool, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let aujourdhui = chrono::Local::now().format("%Y-%m-%d").to_string();
    let cle_stockage = format!("dernier_affichage_{cle}");
    if db::get_setting(&conn, &cle_stockage).as_deref() == Some(aujourdhui.as_str()) {
        return Ok(false);
    }
    db::set_setting(&conn, &cle_stockage, &aujourdhui).map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
pub fn get_server_info(app: AppHandle) -> Result<qr::ServerInfo, String> {
    if !crate::server::est_actif(&app) {
        return Err(
            "Le service de réception QR n'a pas pu démarrer (port 4173 déjà utilisé par un \
             autre programme ?). Les autres canaux (dossier surveillé, clé USB) fonctionnent \
             normalement. Redémarrez l'application pour réessayer."
                .to_string(),
        );
    }
    let state = app.state::<DbState>();
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let ssid = db::get_setting(&conn, "wifi_ssid");
    let mot_de_passe = db::get_setting(&conn, "wifi_mot_de_passe");
    drop(conn);
    qr::build_server_info(ssid, mot_de_passe)
}

/// Ouvre directement la page des paramètres Windows pour le partage de
/// connexion Wi-Fi (Mobile Hotspot) — gardé comme solution de secours pour
/// une carte Wi-Fi qui ne supporterait pas `netsh wlan hostednetwork` (voir
/// `activer_point_acces_local`), ou pour un PC qui a une vraie connexion
/// internet à partager.
#[tauri::command]
pub fn ouvrir_parametres_partage_connexion() -> Result<(), String> {
    #[cfg(windows)]
    {
        files::shell_open(
            std::path::Path::new("ms-settings:network-mobilehotspot"),
            "open",
        )
    }
    #[cfg(not(windows))]
    {
        Err("Disponible uniquement sur Windows".to_string())
    }
}

/// Active le point d'accès Wi-Fi local (voir `hotspot.rs`) avec le SSID et
/// le mot de passe enregistrés dans Réglages. Contrairement au "Point
/// d'accès mobile" des paramètres Windows, ne demande aucune connexion
/// internet ou Ethernet à partager — vérifié sur le terrain comme étant le
/// blocage réel rencontré par une boutique 100% hors ligne.
///
/// Démarre aussi les petits serveurs DHCP et DNS locaux (voir `dhcp.rs` et
/// `dns.rs`) : sans eux, un téléphone connecté au Wi-Fi n'obtient ni
/// adresse IP, ni moyen d'être redirigé automatiquement vers la page
/// d'envoi.
#[tauri::command]
pub async fn activer_point_acces_local(
    state: State<'_, DbState>,
    etat_point_acces: State<'_, crate::hotspot::EtatPointAcces>,
) -> Result<String, String> {
    let ssid = {
        let conn = state.0.lock().map_err(|e| e.to_string())?;
        db::get_setting(&conn, "wifi_ssid").filter(|s| !s.trim().is_empty())
    };
    let mot_de_passe = {
        let conn = state.0.lock().map_err(|e| e.to_string())?;
        db::get_setting(&conn, "wifi_mot_de_passe").unwrap_or_default()
    };

    let Some(ssid) = ssid else {
        return Err(
            "Configurez d'abord un nom de réseau (SSID) dans Réglages → Wi-Fi local.".to_string(),
        );
    };

    // Bloquant (attend la fenêtre d'autorisation Windows) : sur un thread
    // dédié, pour ne jamais geler les autres commandes pendant ce temps.
    let activation = tauri::async_runtime::spawn_blocking(move || {
        crate::hotspot::activer_par_tous_les_moyens(&ssid, &mot_de_passe)
    })
    .await
    .map_err(|e| e.to_string())??;

    let tache_dhcp = tauri::async_runtime::spawn(crate::dhcp::demarrer(activation.adresse));
    let tache_dns = tauri::async_runtime::spawn(crate::dns::demarrer(activation.adresse));
    // Une activation précédente laissée en cours (le gérant a cliqué deux
    // fois) ne doit pas faire tourner deux serveurs DHCP/DNS en même temps
    // sur les mêmes ports.
    let ancien = etat_point_acces
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .replace((tache_dhcp, tache_dns));
    if let Some((ancien_dhcp, ancien_dns)) = ancien {
        ancien_dhcp.abort();
        ancien_dns.abort();
    }

    Ok(activation.methode.to_string())
}

/// Coupe le point d'accès Wi-Fi local activé par `activer_point_acces_local`,
/// ainsi que les serveurs DHCP/DNS qui l'accompagnent.
#[tauri::command]
pub async fn desactiver_point_acces_local(
    etat_point_acces: State<'_, crate::hotspot::EtatPointAcces>,
) -> Result<(), String> {
    if let Some((tache_dhcp, tache_dns)) = etat_point_acces
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .take()
    {
        tache_dhcp.abort();
        tache_dns.abort();
    }
    tauri::async_runtime::spawn_blocking(crate::hotspot::desactiver_par_tous_les_moyens)
        .await
        .map_err(|e| e.to_string())?
}

/// Rapport de compatibilité Wi-Fi de CE PC, sans rien activer ni demander
/// les droits administrateur : ce que Windows déclare savoir faire, plus la
/// sortie brute à transmettre au support quand le verdict reste indécis.
#[tauri::command]
pub fn diagnostiquer_wifi() -> crate::hotspot::DiagnosticWifi {
    crate::hotspot::diagnostiquer()
}

#[derive(serde::Serialize)]
pub struct RapportDiagnostic {
    genere_le: String,
    version: String,
    machine_id: String,
    boutique_nom: Option<String>,
    statut_licence: String,
    jours_restants: i64,
    taille_base_octets: u64,
    derniere_sauvegarde: Option<String>,
    dernier_fichier_recu: Option<String>,
    nombre_transactions_total: i64,
}

/// Rapport texte que le porteur du projet récupère lors d'une visite (clé
/// USB, ou envoyé par le gérant s'il a du réseau sur son téléphone) pour
/// suivre l'état des boutiques déployées sans que leur PC soit jamais
/// connecté à internet — voir admin/README.md.
#[tauri::command]
pub fn generer_rapport_diagnostic(
    app: AppHandle,
    state: State<DbState>,
) -> Result<RapportDiagnostic, String> {
    let (boutique_nom, nombre_transactions_total, dernier_fichier_recu) = {
        let conn = state.0.lock().map_err(|e| e.to_string())?;
        let boutique_nom = db::get_setting(&conn, "boutique_nom");
        let nombre_transactions_total: i64 = conn
            .query_row("SELECT COUNT(*) FROM transactions", [], |r| r.get(0))
            .unwrap_or(0);
        let dernier_fichier_recu: Option<String> = conn
            .query_row(
                "SELECT received_at FROM files_queue ORDER BY received_at DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .ok();
        (boutique_nom, nombre_transactions_total, dernier_fichier_recu)
    };

    let licence = crate::license::get_license_status(state)?;

    let taille_base_octets = app
        .path()
        .app_data_dir()
        .ok()
        .map(|d| d.join("photocopie.sqlite3"))
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .unwrap_or(0);

    Ok(RapportDiagnostic {
        genere_le: chrono::Local::now().to_rfc3339(),
        version: crate::updates::version_actuelle(),
        machine_id: licence.machine_id,
        boutique_nom,
        statut_licence: licence.statut,
        jours_restants: licence.jours_restants,
        taille_base_octets,
        derniere_sauvegarde: crate::backup::derniere_sauvegarde(&app),
        dernier_fichier_recu,
        nombre_transactions_total,
    })
}

/// Écran technique (Réglages, déverrouillé par mot de passe côté
/// interface) : force une sauvegarde immédiate plutôt que d'attendre le
/// prochain cycle automatique (toutes les 15 minutes).
#[tauri::command]
pub fn sauvegarder_maintenant(app: AppHandle) -> Result<(), String> {
    crate::backup::sauvegarder_une_fois(&app)
}

/// Écran technique : ouvre le dossier de données de l'appli (base SQLite,
/// sauvegardes) dans l'explorateur Windows, pour un dépannage sur place.
#[tauri::command]
pub fn ouvrir_dossier_donnees(app: AppHandle) -> Result<(), String> {
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    files::shell_open(&data_dir, "open")
}

fn queue_item_path(state: &State<DbState>, id: i64) -> Result<PathBuf, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    conn.query_row(
        "SELECT path FROM files_queue WHERE id = ?1",
        params![id],
        |row| row.get::<_, String>(0),
    )
    .map(PathBuf::from)
    .map_err(|_| "Fichier introuvable dans la file d'attente".to_string())
}
