use crate::db::DbState;
use crate::files;
use crate::models::QueueItem;
use chrono::Local;
use notify::{Event, RecursiveMode, Watcher};
use rusqlite::params;
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// Le dossier surveillé en ce moment. Changer de dossier dans Réglages
/// arrête la surveillance de l'ancien (son fil le voit et s'arrête).
static DOSSIER_ACTIF: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Temps pendant lequel la taille d'un fichier doit rester la même avant
/// qu'on le prenne : un envoi Bluetooth ou une copie en cours n'est pas un
/// document fini.
const DELAI_STABILITE: Duration = Duration::from_millis(1500);

/// Fichiers qui ne sont jamais des documents de client : fichiers en cours
/// d'écriture (Bluetooth, navigateurs, Word) et fichiers système.
fn est_temporaire(chemin: &std::path::Path) -> bool {
    let nom = chemin
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let extension = chemin
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    nom.starts_with("~$")
        || nom.starts_with('.')
        || nom == "desktop.ini"
        || nom == "thumbs.db"
        || ["tmp", "temp", "part", "partial", "crdownload", "download", "!ut"]
            .contains(&extension.as_str())
}

/// Depuis quand ce dossier est surveillé (secondes Unix). Les fichiers qui
/// s'y trouvaient déjà avant ne sont PAS importés : ce sont les documents
/// du gérant, pas des envois de clients — même leçon que la clé USB.
fn surveille_depuis(app: &AppHandle, dossier: &std::path::Path) -> u64 {
    let maintenant = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let state = app.state::<DbState>();
    let Ok(conn) = state.0.lock() else { return maintenant };
    let chemin = dossier.to_string_lossy().to_string();
    let meme_dossier = crate::db::get_setting(&conn, "dossier_surveille_depuis_chemin")
        .is_some_and(|c| c == chemin);
    if meme_dossier {
        if let Some(depuis) = crate::db::get_setting(&conn, "dossier_surveille_depuis")
            .and_then(|v| v.parse().ok())
        {
            return depuis;
        }
    }
    let _ = crate::db::set_setting(&conn, "dossier_surveille_depuis_chemin", &chemin);
    let _ = crate::db::set_setting(&conn, "dossier_surveille_depuis", &maintenant.to_string());
    maintenant
}

/// Démarre la surveillance du dossier de réception dans un thread dédié.
///
/// Trouvé sur le terrain : les fichiers reçus par Bluetooth n'apparaissaient
/// jamais. La surveillance ne réagissait qu'à l'événement Windows « fichier
/// créé ». Or Windows écrit un fichier reçu par Bluetooth sous un nom
/// temporaire, puis le RENOMME (ou le déplace) : événement différent, jamais
/// traité — et le fichier temporaire, lui, était pris puis disparaissait.
///
/// Désormais, le dossier est simplement RELU toutes les 3 secondes (et
/// aussitôt qu'un événement Windows, quel qu'il soit, arrive). Un fichier
/// est pris quand sa taille ne bouge plus, quelle que soit la façon dont il
/// est arrivé : création, renommage, déplacement, copie, ou pendant que
/// l'application était fermée.
pub fn watch_folder(app: AppHandle, folder: PathBuf) {
    if let Ok(mut actif) = DOSSIER_ACTIF.lock() {
        *actif = Some(folder.clone());
    }
    let depuis = surveille_depuis(&app, &folder);

    thread::spawn(move || {
        let (tx, rx) = channel::<notify::Result<Event>>();
        // Les événements ne servent qu'à réagir plus vite : si Windows
        // refuse de les fournir, la relecture régulière suffit.
        let mut _watcher = notify::recommended_watcher(tx).ok();
        if let Some(w) = _watcher.as_mut() {
            let _ = w.watch(&folder, RecursiveMode::NonRecursive);
        }

        let mut en_observation: std::collections::HashMap<PathBuf, (u64, std::time::Instant)> =
            std::collections::HashMap::new();
        loop {
            let _ = rx.recv_timeout(Duration::from_secs(3));
            while rx.try_recv().is_ok() {}

            let toujours_actif = DOSSIER_ACTIF
                .lock()
                .map(|a| a.as_deref() == Some(folder.as_path()))
                .unwrap_or(false);
            if !toujours_actif {
                return;
            }
            relire_dossier(&app, &folder, depuis, &mut en_observation);
        }
    });
}

fn relire_dossier(
    app: &AppHandle,
    dossier: &std::path::Path,
    depuis: u64,
    en_observation: &mut std::collections::HashMap<PathBuf, (u64, std::time::Instant)>,
) {
    let Ok(entrees) = std::fs::read_dir(dossier) else { return };
    let mut vus = std::collections::HashSet::new();
    for entree in entrees.flatten() {
        let chemin = entree.path();
        let Ok(meta) = entree.metadata() else { continue };
        if !meta.is_file() || est_temporaire(&chemin) {
            continue;
        }
        // Arrivé avant le début de la surveillance : document du gérant.
        let arrive = meta
            .created()
            .or_else(|_| meta.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(u64::MAX);
        if arrive + 5 < depuis {
            continue;
        }
        vus.insert(chemin.clone());
        let taille = meta.len();
        match en_observation.get(&chemin) {
            Some((precedente, depuis_quand))
                if *precedente == taille && depuis_quand.elapsed() >= DELAI_STABILITE =>
            {
                // `enqueue_file` ignore un chemin déjà connu : relire le même
                // fichier à chaque passage ne crée jamais de doublon.
                enqueue_file(app, &chemin, "dossier_surveille", None, None);
            }
            Some((precedente, _)) if *precedente == taille => {}
            _ => {
                en_observation.insert(chemin, (taille, std::time::Instant::now()));
            }
        }
    }
    en_observation.retain(|chemin, _| vus.contains(chemin));
}

#[cfg(test)]
mod tests_dossier {
    use super::est_temporaire;
    use std::path::Path;

    #[test]
    fn les_fichiers_en_cours_d_ecriture_sont_ignores() {
        for nom in ["recu.pdf.tmp", "~$memoire.docx", "photo.jpg.part", "x.crdownload", "Thumbs.db", "desktop.ini", ".cache"] {
            assert!(est_temporaire(Path::new(nom)), "{nom}");
        }
        for nom in ["memoire.pdf", "CV Awa.docx", "IMG-2026.jpg"] {
            assert!(!est_temporaire(Path::new(nom)), "{nom}");
        }
    }
}

/// Préférences d'impression indiquées par le client lui-même (ex: via le
/// formulaire QR), pour que le gérant n'ait qu'à confirmer plutôt qu'à
/// redemander à chacun comment il veut son document.
#[derive(Default)]
pub struct OptionsImpression {
    pub couleur: bool,
    pub format_papier: Option<String>,
    pub copies: Option<i64>,
    pub plage_pages: Option<String>,
}

/// Enregistre un fichier reçu (quel que soit le canal) dans la file d'attente
/// et prévient l'interface. Ignoré silencieusement si le chemin est déjà connu.
pub fn enqueue_file(
    app: &AppHandle,
    path: &std::path::Path,
    source: &str,
    client_name: Option<&str>,
    client_telephone: Option<&str>,
) -> Option<i64> {
    enqueue_file_avec_options(
        app,
        path,
        source,
        client_name,
        client_telephone,
        OptionsImpression::default(),
    )
    .map(|(id, _jeton)| id)
}

/// Renvoie l'identifiant interne du fichier et le jeton secret à remettre au
/// client pour qu'il puisse suivre l'avancement de sa seule commande.
pub fn enqueue_file_avec_options(
    app: &AppHandle,
    path: &std::path::Path,
    source: &str,
    client_name: Option<&str>,
    client_telephone: Option<&str>,
    options: OptionsImpression,
) -> Option<(i64, String)> {
    let original_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "fichier".to_string());
    let kind = files::classify(path);
    // Un .exe/.msi classé "installateur" obtient un bouton "Installer la mise
    // à jour" qui l'exécute en un clic (voir files::shell_open). Seule la clé
    // USB, que le porteur du projet branche lui-même lors d'une visite, peut
    // donc produire cette classification. Le QR et le dossier surveillé sont
    // ouverts à n'importe quel client (Wi-Fi de la boutique, envoi Bluetooth
    // que l'application invite elle-même à utiliser) : un exécutable arrivé
    // par là serait un piège nommé "Mise_a_jour.exe", jamais une vraie mise
    // à jour. Il retombe en "inconnu", qui affiche un avertissement explicite
    // et n'offre aucun bouton pour l'exécuter.
    let kind = if kind == "installateur" && source != "usb" {
        "inconnu"
    } else {
        kind
    };
    let received_at = Local::now().to_rfc3339();
    let taille_octets = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let (protege, format_detecte) = files::diagnostiquer_pdf(path);
    let format_detecte = format_detecte.map(str::to_string);

    let state = app.state::<DbState>();
    let conn = state.0.lock().expect("verrou base de données corrompu");

    let already = conn
        .query_row(
            "SELECT 1 FROM files_queue WHERE path = ?1",
            params![path.to_string_lossy()],
            |_| Ok(()),
        )
        .is_ok();
    if already {
        return None;
    }

    // Un installateur téléchargé deux fois sur le téléphone du gérant (par
    // erreur, ou en pensant que ça n'avait pas marché) porte souvent un nom
    // légèrement différent ("(1)") : le contrôle ci-dessus par chemin exact
    // ne le détecte pas. On compare plutôt la taille — deux fichiers de
    // taille identique déjà en attente, c'est le même installateur deux
    // fois. On ne garde que celui déjà présent, jamais deux invitations à
    // installer la même mise à jour.
    if kind == "installateur" && installateur_deja_en_attente(&conn, taille_octets as i64) {
        let _ = std::fs::remove_file(path);
        return None;
    }

    // Borné aussi ici (pas seulement côté serveur HTTP) : ce point d'entrée
    // sert à tous les canaux de réception, pas seulement le QR.
    let copies = options.copies.unwrap_or(1).clamp(1, 500);
    let format_papier = options
        .format_papier
        .clone()
        .unwrap_or_else(|| "A4".to_string());

    // Jeton secret propre à ce document : c'est lui, et non l'identifiant
    // (1, 2, 3...), que la page du client utilise pour suivre SA commande.
    // Avec un simple numéro, n'importe quel téléphone connecté au Wi-Fi de
    // la boutique pourrait consulter l'état des commandes des autres clients
    // en essayant les numéros les uns après les autres.
    let jeton = format!("{:032x}", rand::random::<u128>());

    let insert_result = conn.execute(
        "INSERT INTO files_queue
            (original_name, path, client_name, client_telephone, source, kind, status,
             received_at, taille_octets, protege, format_detecte, couleur, format_papier,
             copies, plage_pages, jeton)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'en_attente', ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            original_name,
            path.to_string_lossy(),
            client_name,
            client_telephone,
            source,
            kind,
            received_at,
            taille_octets as i64,
            protege,
            format_detecte,
            options.couleur,
            format_papier,
            copies,
            options.plage_pages,
            jeton
        ],
    );

    if let Err(e) = insert_result {
        eprintln!("Échec d'enregistrement du fichier reçu : {e}");
        return None;
    }
    let id = conn.last_insert_rowid();
    drop(conn);

    let item = QueueItem {
        id,
        original_name,
        path: path.to_string_lossy().to_string(),
        client_name: client_name.map(str::to_string),
        client_telephone: client_telephone.map(str::to_string),
        source: source.to_string(),
        kind: kind.to_string(),
        status: "en_attente".to_string(),
        received_at,
        taille_octets: taille_octets as i64,
        protege,
        format_detecte,
        copies,
        couleur: options.couleur,
        format_papier,
        plage_pages: options.plage_pages,
        finitions: vec![],
        prix: None,
        employe: None,
        raison_ignore: None,
        document_supprime: false,
        impression_confirmee: false,
        pages_imprimees: None,
        impression_erreur: None,
        recto_verso: false,
        impression_couleur_reelle: None,
        impression_recto_verso_reelle: None,
        impression_format_reel: None,
        impression_poste: None,
        impression_imprimante_reelle: None,
        impression_ecarts: None,
        // Un fichier qui arrive n'a pas encore été détaillé par le gérant :
        // le total de feuilles vaut le nombre d'exemplaires demandé, pour
        // un document dont on ne connaît pas encore le nombre de pages.
        pages_document: 1,
    };

    let _ = app.emit("nouveau-fichier", item);
    Some((id, jeton))
}

/// Un installateur de même taille est-il déjà en file, en attente d'être
/// installé ? Isolé de `enqueue_file_avec_options` (qui a besoin d'un vrai
/// `AppHandle` Tauri) pour rester testable directement contre une base.
fn installateur_deja_en_attente(conn: &rusqlite::Connection, taille_octets: i64) -> bool {
    conn.query_row(
        "SELECT 1 FROM files_queue
         WHERE kind = 'installateur' AND status = 'en_attente' AND taille_octets = ?1",
        params![taille_octets],
        |_| Ok(()),
    )
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn base_de_test() -> (tempfile::TempDir, rusqlite::Connection) {
        let dossier = tempfile::tempdir().expect("dossier temporaire");
        let conn = crate::db::open(dossier.path()).expect("ouverture de la base de test");
        (dossier, conn)
    }

    fn inserer_installateur(conn: &rusqlite::Connection, chemin: &str, statut: &str, taille: i64) {
        conn.execute(
            "INSERT INTO files_queue
                (original_name, path, source, kind, status, received_at, taille_octets, jeton)
             VALUES (?1, ?1, 'usb', 'installateur', ?2, '2026-01-01T00:00:00+01:00', ?3, ?4)",
            params![chemin, statut, taille, chemin],
        )
        .expect("insertion de test");
    }

    #[test]
    fn aucun_doublon_sur_base_vide() {
        let (_dossier, conn) = base_de_test();
        assert!(!installateur_deja_en_attente(&conn, 150_000_000));
    }

    #[test]
    fn detecte_un_installateur_de_meme_taille_deja_en_attente() {
        let (_dossier, conn) = base_de_test();
        inserer_installateur(&conn, "E:\\Installateur.exe", "en_attente", 150_000_000);
        assert!(installateur_deja_en_attente(&conn, 150_000_000));
    }

    #[test]
    fn une_taille_differente_n_est_pas_un_doublon() {
        let (_dossier, conn) = base_de_test();
        inserer_installateur(&conn, "E:\\Installateur.exe", "en_attente", 150_000_000);
        // Une VRAIE nouvelle version, compilée différemment, n'a presque
        // aucune chance de faire exactement le même nombre d'octets.
        assert!(!installateur_deja_en_attente(&conn, 150_312_009));
    }

    #[test]
    fn un_installateur_deja_installe_ne_bloque_pas_le_suivant() {
        let (_dossier, conn) = base_de_test();
        // "installe" ou "ignore" : plus "en_attente", donc plus un doublon
        // actif — sinon une VRAIE nouvelle mise à jour de même taille par
        // hasard resterait bloquée pour toujours après la précédente.
        inserer_installateur(&conn, "E:\\Ancien.exe", "traite", 150_000_000);
        assert!(!installateur_deja_en_attente(&conn, 150_000_000));
    }
}
