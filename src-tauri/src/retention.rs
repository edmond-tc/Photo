use crate::db::{self, DbState};
use chrono::Local;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager};

const INTERVALLE: Duration = Duration::from_secs(6 * 60 * 60);

/// Durée par défaut de conservation des documents reçus, en jours.
pub const RETENTION_JOURS_DEFAUT: i64 = 30;

/// Les documents des clients (CV, relevés de notes, pièces d'identité,
/// devoirs...) s'accumulent sinon indéfiniment sur le PC de la boutique.
/// Deux problèmes bien réels :
///   - le disque finit par se remplir, et l'appli ne peut plus rien recevoir ;
///   - garder les données personnelles d'inconnus des années après le service
///     rendu n'est ni nécessaire ni défendable si quelqu'un le demande.
/// On ne supprime que le fichier lui-même, et seulement pour une commande déjà
/// traitée : la ligne de comptabilité, elle, reste intacte pour toujours.
pub fn start(app: AppHandle) {
    thread::spawn(move || loop {
        let (nombre, octets) = purger_une_fois(&app);
        if nombre > 0 {
            println!("Purge : {nombre} document(s) client supprimé(s) ({octets} octets libérés).");
        }
        thread::sleep(INTERVALLE);
    });
}

fn jours_de_retention(app: &AppHandle) -> i64 {
    let state = app.state::<DbState>();
    let Ok(conn) = state.0.lock() else {
        return RETENTION_JOURS_DEFAUT;
    };
    db::get_setting(&conn, "retention_jours")
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(RETENTION_JOURS_DEFAUT)
        .clamp(0, 3650)
}

/// Renvoie (nombre de fichiers supprimés, octets libérés).
pub fn purger_une_fois(app: &AppHandle) -> (usize, u64) {
    let jours = jours_de_retention(app);
    if jours == 0 {
        return (0, 0); // 0 = le gérant a choisi de tout garder
    }

    let Ok(data_dir) = app.path().app_data_dir() else {
        return (0, 0);
    };
    let limite = (Local::now() - chrono::Duration::days(jours)).to_rfc3339();

    let state = app.state::<DbState>();
    let Ok(conn) = state.0.lock() else {
        return (0, 0);
    };

    let Ok(mut stmt) = conn.prepare(
        "SELECT path FROM files_queue WHERE status = 'traite' AND received_at < ?1",
    ) else {
        return (0, 0);
    };
    let Ok(chemins) = stmt.query_map(rusqlite::params![limite], |r| r.get::<_, String>(0)) else {
        return (0, 0);
    };

    let mut nombre = 0usize;
    let mut octets = 0u64;
    for chemin in chemins.flatten() {
        let chemin = PathBuf::from(chemin);
        // Garde-fou : on ne supprime que ce que l'application a elle-même
        // recopié dans ses propres dossiers. Jamais le dossier surveillé du
        // gérant, jamais une clé USB, jamais un fichier de travail à lui.
        if !fichier_de_lapplication(&data_dir, &chemin) {
            continue;
        }
        let taille = std::fs::metadata(&chemin).map(|m| m.len()).unwrap_or(0);
        if std::fs::remove_file(&chemin).is_ok() {
            nombre += 1;
            octets += taille;
        }
    }

    (nombre, octets)
}

fn fichier_de_lapplication(data_dir: &Path, chemin: &Path) -> bool {
    ["recus", "recus_usb"]
        .iter()
        .any(|dossier| chemin.starts_with(data_dir.join(dossier)))
}
