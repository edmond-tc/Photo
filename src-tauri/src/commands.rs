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
        "recu_automatique": db::get_setting(&conn, "recu_automatique"),
        "wifi_mot_de_passe": db::get_setting(&conn, "wifi_mot_de_passe"),
        "wifi_type_reseau": db::get_setting(&conn, "wifi_type_reseau"),
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
        "recu_automatique",
        "wifi_mot_de_passe",
        "wifi_type_reseau",
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
    let type_reseau = db::get_setting(&conn, "wifi_type_reseau");
    drop(conn);
    qr::build_server_info(ssid, mot_de_passe, type_reseau)
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
/// Réessaie de démarrer un serveur : le port qu'occupait le précédent met
/// un instant à être rendu par le système, et un premier refus signifie
/// souvent « pas encore libre » plutôt que « occupé par un autre logiciel ».
/// Renoncer au premier essai laissait le gérant devant un message
/// d'indisponibilité alors qu'il suffisait d'attendre une seconde.
async fn demarrer_avec_reessais<F, Fut>(
    mut demarrage: F,
) -> Result<tauri::async_runtime::JoinHandle<()>, String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<tauri::async_runtime::JoinHandle<()>, String>>,
{
    const TENTATIVES: u32 = 5;
    const DELAI: std::time::Duration = std::time::Duration::from_millis(500);

    let mut dernier = Err("jamais tenté".to_string());
    for tentative in 0..TENTATIVES {
        if tentative > 0 {
            tokio::time::sleep(DELAI).await;
        }
        dernier = demarrage().await;
        if dernier.is_ok() {
            return dernier;
        }
    }
    dernier
}

/// Réessaie un contrôle quelques secondes avant de le déclarer en échec.
///
/// Les services viennent d'être lancés et l'adresse vient d'être validée :
/// un premier refus ne veut pas encore dire « ne marche pas », il peut
/// simplement vouloir dire « pas encore ». Déclarer rouge trop tôt enverrait
/// chercher une panne qui n'existe pas — l'inverse exact du service que ce
/// récapitulatif doit rendre.
async fn reessayer<F, Fut>(mut controle: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    const TENTATIVES: u32 = 6;
    const DELAI: std::time::Duration = std::time::Duration::from_millis(800);

    for tentative in 0..TENTATIVES {
        if tentative > 0 {
            tokio::time::sleep(DELAI).await;
        }
        if controle().await {
            return true;
        }
    }
    false
}

/// Ce que les téléphones ont réellement demandé depuis l'activation.
///
/// Après des jours d'hypothèses successives — chacune plausible, chacune
/// réfutée par l'essai suivant — c'est la seule question qui reste : ces
/// serveurs reçoivent-ils, oui ou non, les demandes du téléphone ? Le
/// gérant connecte un téléphone, appuie ici, et la réponse ne se discute
/// plus.
#[tauri::command]
pub fn journal_des_telephones() -> Vec<String> {
    let adresses = crate::dhcp::journal();
    let noms = crate::dns::journal();

    let mut lignes = Vec::new();
    lignes.push("— Demandes d'adresse reçues —".to_string());
    if adresses.is_empty() {
        lignes.push(
            "(aucune) Aucun téléphone n'a demandé d'adresse à CE serveur. S'il a pourtant \
             rejoint le réseau, c'est qu'un autre programme lui a répondu."
                .to_string(),
        );
    } else {
        lignes.extend(adresses);
    }

    lignes.push(String::new());
    lignes.push("— Pages demandées reçues —".to_string());
    let pages = crate::server::journal_pages();
    if pages.is_empty() {
        lignes.push(
            "(aucune) Aucun téléphone n'est venu frapper à la porte du portail. Sans cette \
             visite, aucune page ne peut s'ouvrir."
                .to_string(),
        );
    } else {
        lignes.extend(pages);
    }

    lignes.push(String::new());
    lignes.push("— Noms demandés reçus —".to_string());
    if noms.is_empty() {
        lignes.push(
            "(aucun) Aucun téléphone n'a posé de question à CE serveur de noms. Il ne peut \
             donc pas découvrir le portail, et la page ne s'ouvrira jamais d'elle-même."
                .to_string(),
        );
    } else {
        lignes.extend(noms);
    }
    lignes
}

#[derive(serde::Serialize)]
pub struct ResultatActivationWifi {
    pub methode: String,
    /// État de chaque service indispensable, réussite comprise.
    ///
    /// Les avertissements ci-dessous ne disent que ce qui a ÉCHOUÉ : quand
    /// tout démarre, ils sont vides, et le gérant — comme le support — n'a
    /// alors aucune idée de ce qui tourne réellement. Sur le terrain, cette
    /// absence a coûté plusieurs allers-retours à chercher lequel des trois
    /// services manquait. Ce récapitulatif est donc toujours affiché : une
    /// photo de cet écran suffit désormais à situer la panne.
    pub recapitulatif: Vec<String>,
    /// Vide quand tout a démarré normalement. Sinon, chaque entrée est un
    /// service qui n'a pas pu s'installer — DHCP et/ou DNS, chacun pouvant
    /// échouer indépendamment de l'autre (Windows fait parfois tourner ses
    /// propres services sur ces mêmes ports dès le Wi-Fi Direct activé).
    /// Avant ce champ, un tel échec s'écrivait dans une console qui
    /// n'existe pas dans l'application installée : le gérant voyait "Wi-Fi
    /// activé" sans jamais savoir que l'ouverture automatique était morte.
    pub avertissements: Vec<String>,
}

#[tauri::command]
pub async fn activer_point_acces_local(
    state: State<'_, DbState>,
    etat_point_acces: State<'_, crate::hotspot::EtatPointAcces>,
) -> Result<ResultatActivationWifi, String> {
    let ssid = {
        let conn = state.0.lock().map_err(|e| e.to_string())?;
        db::get_setting(&conn, "wifi_ssid").filter(|s| !s.trim().is_empty())
    };

    let mot_de_passe = {
        let conn = state.0.lock().map_err(|e| e.to_string())?;
        db::get_setting(&conn, "wifi_mot_de_passe").unwrap_or_default()
    };

    // Une installation neuve n'a encore aucun nom de réseau. Jusqu'ici, le
    // bouton refusait d'agir et renvoyait le gérant dans les réglages —
    // constaté sur le terrain juste après une réinstallation, et c'est un
    // mur que rencontrera CHAQUE boutique à sa première utilisation, avec un
    // client devant le comptoir.
    //
    // Il n'y a pourtant rien à décider : n'importe quel nom fait l'affaire.
    // L'application en pose donc un, l'enregistre pour que le gérant le voie
    // et puisse le changer dans Réglages, et continue.
    let (ssid, mot_de_passe) = match ssid {
        Some(ssid) => (ssid, mot_de_passe),
        None => {
            let nom_boutique = {
                let conn = state.0.lock().map_err(|e| e.to_string())?;
                db::get_setting(&conn, "boutique_nom")
            };
            let ssid = crate::hotspot::nom_reseau_par_defaut(nom_boutique.as_deref());
            let mot_de_passe = if mot_de_passe.chars().count() >= 8 {
                mot_de_passe
            } else {
                crate::hotspot::MOT_DE_PASSE_PAR_DEFAUT.to_string()
            };

            let conn = state.0.lock().map_err(|e| e.to_string())?;
            db::set_setting(&conn, "wifi_ssid", &ssid).map_err(|e| e.to_string())?;
            db::set_setting(&conn, "wifi_mot_de_passe", &mot_de_passe)
                .map_err(|e| e.to_string())?;
            (ssid, mot_de_passe)
        }
    };

    let type_reseau = {
        let conn = state.0.lock().map_err(|e| e.to_string())?;
        db::get_setting(&conn, "wifi_type_reseau")
    };

    // Un routeur dédié crée déjà son propre réseau, indépendamment de nous :
    // rien à activer côté Wi-Fi, seulement le tour de passe-passe DNS qui
    // ouvre la page toute seule (voir routeur_externe.rs).
    if type_reseau.as_deref() == Some("routeur_externe") {
        let (adresse, tache_dns) = crate::routeur_externe::activer().await?;
        crate::hotspot::definir_adresse_active(Some(adresse));

        // Le réseau vient d'ailleurs, mais le pare-feu de CE PC bloque tout
        // autant : sans ces règles, le téléphone rejoint bien le réseau et
        // n'atteint jamais la page (voir `pare_feu.rs`).
        let mut avertissements = Vec::new();
        if !crate::pare_feu::regles_presentes() {
            if let Err(e) = tauri::async_runtime::spawn_blocking(crate::pare_feu::autoriser)
                .await
                .map_err(|e| e.to_string())?
            {
                avertissements.push(format!(
                    "Le pare-feu Windows n'a pas pu être ouvert ({e}). Les téléphones \
                     risquent de ne pas atteindre la page d'envoi. Réessayez et acceptez \
                     la fenêtre d'autorisation Windows."
                ));
            }
        }

        let anciennes = std::mem::replace(
            &mut *etat_point_acces.0.lock().map_err(|e| e.to_string())?,
            vec![tache_dns],
        );
        for tache in anciennes {
            tache.abort();
        }

        return Ok(ResultatActivationWifi {
            methode: "réseau externe".to_string(),
            recapitulatif: vec![
                format!("Adresse de ce PC : {adresse}"),
                "Réseau : créé par une box, un routeur ou un téléphone".to_string(),
            ],
            avertissements,
        });
    }

    // Bloquant (attend la fenêtre d'autorisation Windows) : sur un thread
    // dédié, pour ne jamais geler les autres commandes pendant ce temps.
    let activation = tauri::async_runtime::spawn_blocking(move || {
        crate::hotspot::activer_par_tous_les_moyens(&ssid, &mot_de_passe)
    })
    .await
    .map_err(|e| e.to_string())??;

    // Couper les anciens serveurs AVANT d'en démarrer de nouveaux.
    //
    // L'ordre inverse — celui d'avant — rendait le conflit certain : deux
    // serveurs ne peuvent pas tenir le même port, et l'ancien le tenait
    // encore quand le nouveau tentait de s'y installer. D'où l'erreur
    // Windows 10048 (« une seule utilisation de chaque adresse de socket est
    // autorisée ») sur les ports 67 et 53, et des téléphones qui rejoignaient
    // le Wi-Fi sans jamais recevoir d'adresse.
    //
    // Ce n'était pas seulement le cas du gérant qui clique deux fois :
    // l'application démarre elle-même ces serveurs à son lancement quand
    // elle trouve un point d'accès déjà allumé (voir
    // `hotspot::reprendre_point_acces_existant`). Le conflit était donc
    // systématique dès qu'on appuyait sur le bouton.
    {
        let anciennes = std::mem::take(&mut *etat_point_acces.0.lock().map_err(|e| e.to_string())?);
        for tache in anciennes {
            tache.abort();
        }
    }
    // Arrêter une tâche ne rend pas le port dans l'instant : le système a
    // besoin d'un moment pour le libérer réellement.
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    let mut nouvelles_taches = Vec::new();
    let mut recapitulatif = vec![
        format!("Méthode : {}", activation.methode),
        format!("Adresse de ce PC : {}", activation.adresse),
    ];

    // Les méthodes qui ont échoué AVANT celle qui a réussi comptent comme
    // des avertissements : sur un PC dont le pilote ne convient qu'à la
    // méthode 1, une "réussite" de la méthode 2 peut n'être qu'apparente,
    // et c'est l'échec de la méthode 1 qui contient la vraie information.
    let mut avertissements = activation.avertissements.clone();

    // Rien ne sert de démarrer ni de tester quoi que ce soit tant que
    // l'adresse n'est pas utilisable : Windows met quelques secondes à la
    // valider, et tout ce qui se passe avant tombe dans le vide — y compris
    // les premières questions du téléphone qui vient de rejoindre le réseau
    // (voir `hotspot::attendre_adresse_utilisable`).
    let adresse_prete = crate::hotspot::attendre_adresse_utilisable(activation.adresse).await;
    recapitulatif.push(if adresse_prete {
        "✅ Adresse du PC prête (validée par Windows)".to_string()
    } else {
        let message = "L'adresse du point d'accès n'est toujours pas utilisable après 20 \
                       secondes. Rien ne pourra répondre aux téléphones. Coupez puis \
                       réactivez le Wi-Fi local ; si cela se reproduit, redémarrez le PC."
            .to_string();
        avertissements.push(message);
        "❌ Adresse du PC — TOUJOURS PAS UTILISABLE".to_string()
    });

    match demarrer_avec_reessais(|| crate::dhcp::demarrer(activation.adresse)).await {
        Ok(tache) => {
            nouvelles_taches.push(tache);
            recapitulatif.push("✅ Adresses distribuées aux téléphones".to_string());
        }
        Err(e) => {
            recapitulatif
                .push("❌ Adresses distribuées aux téléphones — NE TOURNE PAS".to_string());
            avertissements.push(e);
        }
    }
    let mut dns_demarre = false;
    match demarrer_avec_reessais(|| crate::dns::demarrer(activation.adresse)).await {
        Ok(tache) => {
            nouvelles_taches.push(tache);
            dns_demarre = true;
        }
        Err(e) => {
            recapitulatif.push("❌ Noms de domaine — NE TOURNE PAS".to_string());
            avertissements.push(e);
        }
    }

    // « Démarré » n'est pas « répond ». On pose donc au serveur la question
    // exacte que pose un téléphone, et on n'annonce vert que si la réponse
    // arrive. Plusieurs déplacements sur le terrain ont été perdus devant un
    // écran tout vert alors que rien ne répondait.
    if dns_demarre {
        recapitulatif.push(
            if reessayer(|| crate::dns::repond(activation.adresse)).await {
                "✅ Noms de domaine — testé, répond".to_string()
            } else {
                let message = "Le serveur de noms a démarré mais NE RÉPOND PAS à la question \
                           que pose un téléphone en rejoignant le réseau. La page ne \
                           pourra pas s'ouvrir toute seule."
                    .to_string();
                avertissements.push(message);
                "❌ Noms de domaine — démarré mais NE RÉPOND PAS".to_string()
            },
        );
    }
    // Le serveur qui fait s'ouvrir la page toute seule démarre au lancement
    // de l'application, bien avant ce bouton : son échec éventuel n'a aucune
    // autre occasion d'être dit au gérant qu'ici.
    match crate::server::probleme_portail_captif() {
        Some(probleme) => {
            recapitulatif.push("❌ Ouverture automatique de la page — NE TOURNE PAS".to_string());
            avertissements.push(probleme);
        }
        None => recapitulatif.push(
            if reessayer(|| crate::server::portail_repond(activation.adresse)).await {
                "✅ Ouverture automatique — testée, répond".to_string()
            } else {
                let message = "Le portail a démarré mais NE RÉPOND PAS sur le port 80. La \
                               page ne pourra pas s'ouvrir toute seule ; le client devra \
                               scanner le petit second QR."
                    .to_string();
                avertissements.push(message);
                "❌ Ouverture automatique — démarrée mais NE RÉPOND PAS".to_string()
            },
        ),
    }
    recapitulatif.push(
        if reessayer(|| crate::server::api_portail_repond(activation.adresse)).await {
            "✅ Annonce du portail aux téléphones — testée, répond".to_string()
        } else {
            let message = "L'annonce normalisée du portail (celle qui fait ouvrir la page \
                           toute seule sur les téléphones récents) ne répond pas. La page \
                           ne s'ouvrira pas d'elle-même."
                .to_string();
            avertissements.push(message);
            "❌ Annonce du portail aux téléphones — NE RÉPOND PAS".to_string()
        },
    );
    // Combien de téléphones EN MÊME TEMPS. Cela ne dépend pas de nous mais
    // du pilote Wi-Fi, et cela varie beaucoup d'un PC à l'autre. Un gérant
    // qui l'ignore découvrira la limite devant une file de clients, sans
    // comprendre pourquoi les derniers « n'arrivent pas à se connecter ».
    #[cfg(windows)]
    if let Some(maximum) = crate::hotspot::nombre_max_de_clients() {
        recapitulatif.push(format!(
            "ℹ️ Ce PC accepte {maximum} téléphones connectés en même temps"
        ));
        if maximum < 10 {
            avertissements.push(format!(
                "Le Wi-Fi de ce PC n'accepte que {maximum} téléphones à la fois. Au-delà, \
                 les clients suivants ne pourront pas se connecter tant qu'un autre ne \
                 s'est pas déconnecté. Un client qui a fini d'envoyer devrait quitter le \
                 réseau pour laisser la place."
            ));
        }
    }
    recapitulatif.push(if crate::pare_feu::regles_presentes() {
        "✅ Pare-feu Windows ouvert (5 ports)".to_string()
    } else {
        "❌ Pare-feu Windows — règles absentes".to_string()
    });

    // Les anciennes ont déjà été arrêtées plus haut : il ne reste qu'à
    // ranger celles qui viennent de démarrer.
    *etat_point_acces.0.lock().map_err(|e| e.to_string())? = nouvelles_taches;

    // La liste des programmes qui tiennent ces ports est jointe systéma-
    // tiquement : c'est le seul moyen de savoir si un autre logiciel répond
    // aux téléphones à notre place, et une photo de cet écran suffit alors
    // à le nommer.
    let ecoutes = tauri::async_runtime::spawn_blocking(crate::hotspot::qui_ecoute_sur_les_ports)
        .await
        .unwrap_or_default();
    if !ecoutes.trim().is_empty() {
        recapitulatif.push(String::new());
        recapitulatif.push("— Qui écoute sur les ports —".to_string());
        for ligne in ecoutes.lines() {
            recapitulatif.push(ligne.to_string());
        }
    }

    Ok(ResultatActivationWifi {
        methode: activation.methode.to_string(),
        recapitulatif,
        avertissements,
    })
}

/// Coupe le point d'accès Wi-Fi local activé par `activer_point_acces_local`,
/// ainsi que les serveurs DHCP/DNS qui l'accompagnent.
#[tauri::command]
pub async fn desactiver_point_acces_local(
    etat_point_acces: State<'_, crate::hotspot::EtatPointAcces>,
) -> Result<(), String> {
    let taches = std::mem::take(&mut *etat_point_acces.0.lock().map_err(|e| e.to_string())?);
    for tache in taches {
        tache.abort();
    }
    tauri::async_runtime::spawn_blocking(crate::hotspot::desactiver_par_tous_les_moyens)
        .await
        .map_err(|e| e.to_string())?
}

/// Répond, pour CE poste, à la question qui se pose en boutique : qu'est-ce
/// que je fais pour recevoir les fichiers des clients ? Sans rien activer ni
/// demander les droits administrateur. Joint la sortie brute de Windows, à
/// transmettre au support quand le verdict reste indécis.
#[tauri::command]
pub fn diagnostiquer_poste() -> crate::hotspot::DiagnosticPoste {
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
    /// Cette version exige-t-elle un code d'installation au premier
    /// lancement ? Mis en pause pendant les tests du porteur du projet, il
    /// doit être réactivé avant de confier les installations à des agents —
    /// d'où sa présence ici : c'est le moyen de vérifier, sur une machine
    /// déjà installée, laquelle des deux versions y a été posée.
    code_installation_exige: bool,
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
        (
            boutique_nom,
            nombre_transactions_total,
            dernier_fichier_recu,
        )
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
        code_installation_exige: crate::license::code_installation_exige(),
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
