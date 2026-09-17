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

    purger(&conn, &data_dir, &limite)
}

/// Séparé de `purger_une_fois` (qui a besoin d'un vrai AppHandle Tauri) pour
/// rester vérifiable avec une vraie base et de vrais fichiers.
pub fn purger(conn: &rusqlite::Connection, data_dir: &Path, limite: &str) -> (usize, u64) {
    let Ok(mut stmt) = conn.prepare(
        "SELECT id, path FROM files_queue
         WHERE status = 'traite' AND received_at < ?1 AND document_supprime = 0",
    ) else {
        return (0, 0);
    };
    let Ok(lignes) = stmt.query_map(rusqlite::params![limite], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    }) else {
        return (0, 0);
    };

    let mut nombre = 0usize;
    let mut octets = 0u64;
    for (id, chemin) in lignes.flatten() {
        let chemin = PathBuf::from(chemin);
        // Garde-fou : on ne supprime que ce que l'application a elle-même
        // recopié dans ses propres dossiers. Jamais le dossier surveillé du
        // gérant, jamais une clé USB, jamais un fichier de travail à lui.
        if !fichier_de_lapplication(data_dir, &chemin) {
            continue;
        }
        let taille = std::fs::metadata(&chemin).map(|m| m.len()).unwrap_or(0);
        if std::fs::remove_file(&chemin).is_ok() {
            nombre += 1;
            octets += taille;
            // Sans cette marque, l'historique continuait d'afficher le
            // document comme présent, avec son bouton "Supprimer le
            // document", alors que le fichier n'existait plus — le gérant
            // croyait pouvoir le rouvrir pour un client qui repasse.
            let _ = conn.execute(
                "UPDATE files_queue SET document_supprime = 1 WHERE id = ?1",
                rusqlite::params![id],
            );
        }
    }

    (nombre, octets)
}

fn fichier_de_lapplication(data_dir: &Path, chemin: &Path) -> bool {
    ["recus", "recus_usb"]
        .iter()
        .any(|dossier| chemin.starts_with(data_dir.join(dossier)))
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

    fn inserer(conn: &rusqlite::Connection, id: i64, chemin: &str, statut: &str, recu_le: &str) {
        conn.execute(
            "INSERT INTO files_queue (id, original_name, path, source, kind, status, received_at, jeton)
             VALUES (?1, 'doc.pdf', ?2, 'qr', 'imprimable', ?3, ?4, ?2)",
            params![id, chemin, statut, recu_le],
        )
        .expect("insertion de test");
    }

    /// Crée un vrai fichier dans le dossier "recus" de l'application.
    fn creer_document(data_dir: &Path, nom: &str) -> PathBuf {
        let dossier = data_dir.join("recus");
        std::fs::create_dir_all(&dossier).unwrap();
        let chemin = dossier.join(nom);
        std::fs::write(&chemin, b"contenu du client").unwrap();
        chemin
    }

    #[test]
    fn supprime_le_document_ancien_et_le_marque_comme_supprime() {
        let (dossier, conn) = base_de_test();
        let chemin = creer_document(dossier.path(), "vieux.pdf");
        inserer(&conn, 1, &chemin.to_string_lossy(), "traite", "2026-01-01T08:00:00+01:00");

        let (nombre, octets) = purger(&conn, dossier.path(), "2026-06-01");

        assert_eq!(nombre, 1);
        assert!(octets > 0);
        assert!(!chemin.exists(), "le fichier aurait dû être effacé du disque");
        let supprime: i64 = conn
            .query_row("SELECT document_supprime FROM files_queue WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(supprime, 1, "l'historique doit savoir que le document n'existe plus");
    }

    #[test]
    fn la_ligne_de_comptabilite_reste_intacte() {
        let (dossier, conn) = base_de_test();
        let chemin = creer_document(dossier.path(), "vieux.pdf");
        inserer(&conn, 1, &chemin.to_string_lossy(), "traite", "2026-01-01T08:00:00+01:00");

        purger(&conn, dossier.path(), "2026-06-01");

        let reste: i64 = conn
            .query_row("SELECT COUNT(*) FROM files_queue WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(reste, 1, "seul le fichier s'efface, jamais la comptabilité");
    }

    #[test]
    fn ne_touche_pas_une_commande_encore_en_attente() {
        let (dossier, conn) = base_de_test();
        let chemin = creer_document(dossier.path(), "en_cours.pdf");
        inserer(&conn, 1, &chemin.to_string_lossy(), "en_attente", "2026-01-01T08:00:00+01:00");

        let (nombre, _) = purger(&conn, dossier.path(), "2026-06-01");

        assert_eq!(nombre, 0);
        assert!(chemin.exists(), "un document pas encore servi ne doit jamais disparaître");
    }

    #[test]
    fn ne_touche_pas_un_document_recent() {
        let (dossier, conn) = base_de_test();
        let chemin = creer_document(dossier.path(), "recent.pdf");
        inserer(&conn, 1, &chemin.to_string_lossy(), "traite", "2026-09-15T08:00:00+01:00");

        let (nombre, _) = purger(&conn, dossier.path(), "2026-06-01");

        assert_eq!(nombre, 0);
        assert!(chemin.exists());
    }

    #[test]
    fn ne_sort_jamais_des_dossiers_de_l_application() {
        // Un fichier du dossier surveillé du gérant, ou resté sur une clé
        // USB : l'application ne l'a pas recopié, elle n'a rien à y effacer.
        let (dossier, conn) = base_de_test();
        let ailleurs = dossier.path().join("documents_du_gerant");
        std::fs::create_dir_all(&ailleurs).unwrap();
        let chemin = ailleurs.join("travail.pdf");
        std::fs::write(&chemin, b"fichier personnel").unwrap();
        inserer(&conn, 1, &chemin.to_string_lossy(), "traite", "2026-01-01T08:00:00+01:00");

        let (nombre, _) = purger(&conn, dossier.path(), "2026-06-01");

        assert_eq!(nombre, 0);
        assert!(chemin.exists(), "un fichier hors des dossiers de l'appli ne doit jamais être effacé");
    }

    #[test]
    fn ne_repasse_pas_sur_un_document_deja_supprime() {
        let (dossier, conn) = base_de_test();
        let chemin = creer_document(dossier.path(), "deja.pdf");
        inserer(&conn, 1, &chemin.to_string_lossy(), "traite", "2026-01-01T08:00:00+01:00");
        conn.execute("UPDATE files_queue SET document_supprime = 1 WHERE id = 1", [])
            .unwrap();

        let (nombre, _) = purger(&conn, dossier.path(), "2026-06-01");

        assert_eq!(nombre, 0);
    }
}
