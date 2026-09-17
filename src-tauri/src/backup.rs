use crate::db::{self, DbState};
use chrono::Local;
use rusqlite::backup::Backup;
use rusqlite::Connection;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager};

const INTERVALLE: Duration = Duration::from_secs(15 * 60);
const SAUVEGARDES_A_CONSERVER: usize = 15;

/// Sauvegarde automatique et régulière de la base SQLite — essentielle vu
/// le risque de coupure de courant au Bénin (section 8 du cahier des
/// charges). Utilise l'API de backup SQLite (cohérente même en écriture
/// concurrente), pas une simple copie de fichier.
pub fn start(app: AppHandle) {
    thread::spawn(move || loop {
        thread::sleep(INTERVALLE);
        if let Err(e) = sauvegarder_une_fois(&app) {
            eprintln!("Échec de la sauvegarde automatique : {e}");
        }
    });
}

/// Déclenchement manuel (écran technique) — même logique que la sauvegarde
/// automatique périodique, appelée à la demande.
pub fn sauvegarder_une_fois(app: &AppHandle) -> Result<(), String> {
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;

    let dossier_sauvegarde = {
        let state = app.state::<DbState>();
        let conn = state.0.lock().map_err(|e| e.to_string())?;
        db::get_setting(&conn, "dossier_sauvegarde")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| data_dir.join("sauvegardes"))
    };
    std::fs::create_dir_all(&dossier_sauvegarde).map_err(|e| e.to_string())?;

    let nom_fichier = format!(
        "photocopie-{}.sqlite3",
        Local::now().format("%Y%m%d-%H%M%S")
    );
    let destination = dossier_sauvegarde.join(&nom_fichier);

    {
        let state = app.state::<DbState>();
        let conn_source = state.0.lock().map_err(|e| e.to_string())?;
        let mut conn_dest = Connection::open(&destination).map_err(|e| e.to_string())?;
        let backup = Backup::new(&conn_source, &mut conn_dest).map_err(|e| e.to_string())?;
        backup
            .run_to_completion(5, Duration::from_millis(250), None)
            .map_err(|e| e.to_string())?;
    }

    nettoyer_anciennes_sauvegardes(&dossier_sauvegarde);
    Ok(())
}

/// Date de la sauvegarde la plus récente, pour le rapport de diagnostic
/// exportable — utile au porteur du projet pour vérifier lors d'une visite
/// que la sauvegarde automatique fonctionne bien chez ce gérant.
pub fn derniere_sauvegarde(app: &AppHandle) -> Option<String> {
    let data_dir = app.path().app_data_dir().ok()?;
    let dossier_sauvegarde = {
        let state = app.state::<DbState>();
        let conn = state.0.lock().ok()?;
        db::get_setting(&conn, "dossier_sauvegarde")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| data_dir.join("sauvegardes"))
    };
    let plus_recent = std::fs::read_dir(&dossier_sauvegarde)
        .ok()?
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "sqlite3"))
        .max_by_key(|e| e.file_name())?;
    let metadata = plus_recent.metadata().ok()?;
    let modifie = metadata.modified().ok()?;
    let datetime: chrono::DateTime<chrono::Local> = modifie.into();
    Some(datetime.to_rfc3339())
}

#[derive(serde::Serialize)]
pub struct SauvegardeDisponible {
    pub chemin: String,
    pub nom: String,
    pub date_lisible: String,
    pub taille_octets: u64,
}

/// Liste les sauvegardes restaurables, la plus récente en premier.
#[tauri::command]
pub fn lister_sauvegardes(app: AppHandle) -> Result<Vec<SauvegardeDisponible>, String> {
    let dossier = dossier_sauvegarde(&app)?;
    let Ok(entries) = std::fs::read_dir(&dossier) else {
        return Ok(vec![]);
    };
    let mut sauvegardes: Vec<SauvegardeDisponible> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "sqlite3"))
        .filter_map(|e| {
            let metadata = e.metadata().ok()?;
            let modifie: chrono::DateTime<Local> = metadata.modified().ok()?.into();
            Some(SauvegardeDisponible {
                chemin: e.path().to_string_lossy().to_string(),
                nom: e.file_name().to_string_lossy().to_string(),
                date_lisible: modifie.format("%d/%m/%Y à %H:%M").to_string(),
                taille_octets: metadata.len(),
            })
        })
        .collect();
    sauvegardes.sort_by(|a, b| b.nom.cmp(&a.nom));
    Ok(sauvegardes)
}

/// Restaure une sauvegarde par-dessus la base en service.
///
/// Sans cette fonction, les sauvegardes automatiques ne servaient à rien : en
/// cas de base corrompue (coupure de courant au mauvais moment, disque
/// fatigué), un gérant qui n'est pas informaticien n'avait aucun moyen de
/// récupérer sa comptabilité.
///
/// La base actuelle est elle-même sauvegardée juste avant d'être remplacée :
/// se tromper de sauvegarde ne doit jamais être une erreur définitive.
#[tauri::command]
pub fn restaurer_sauvegarde(app: AppHandle, chemin: String) -> Result<String, String> {
    let dossier = dossier_sauvegarde(&app)?;
    let source = std::path::PathBuf::from(&chemin);

    // On n'ouvre que des fichiers du dossier de sauvegarde : l'interface ne
    // propose rien d'autre, et une restauration depuis n'importe où serait un
    // moyen détourné de faire ouvrir un fichier arbitraire à l'application.
    if !source.starts_with(&dossier) || source.extension().is_none_or(|e| e != "sqlite3") {
        return Err("Ce fichier n'est pas une sauvegarde de l'application.".to_string());
    }
    if !source.is_file() {
        return Err("Cette sauvegarde est introuvable.".to_string());
    }

    // Vérifie que la sauvegarde est lisible AVANT de toucher à la base en
    // service : restaurer un fichier corrompu par-dessus des données saines
    // serait le pire résultat possible.
    {
        let test = Connection::open(&source).map_err(|e| e.to_string())?;
        let verdict: String = test
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .map_err(|_| "Cette sauvegarde est illisible ou endommagée.".to_string())?;
        if verdict != "ok" {
            return Err("Cette sauvegarde est endommagée, choisissez-en une autre.".to_string());
        }
    }

    let secours = dossier.join(format!(
        "avant-restauration-{}.sqlite3",
        Local::now().format("%Y%m%d-%H%M%S")
    ));

    let state = app.state::<DbState>();
    let mut conn_en_service = state.0.lock().map_err(|e| e.to_string())?;

    {
        let mut conn_secours = Connection::open(&secours).map_err(|e| e.to_string())?;
        let sauvegarde =
            Backup::new(&conn_en_service, &mut conn_secours).map_err(|e| e.to_string())?;
        sauvegarde
            .run_to_completion(5, Duration::from_millis(250), None)
            .map_err(|e| e.to_string())?;
    }

    // L'API de backup SQLite écrit dans la connexion déjà ouverte : la base en
    // service est remplacée sans fermer ni rouvrir l'application.
    let conn_source = Connection::open(&source).map_err(|e| e.to_string())?;
    let restauration = Backup::new(&conn_source, &mut conn_en_service).map_err(|e| e.to_string())?;
    restauration
        .run_to_completion(5, Duration::from_millis(250), None)
        .map_err(|e| e.to_string())?;

    Ok(secours.to_string_lossy().to_string())
}

fn dossier_sauvegarde(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let state = app.state::<DbState>();
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    Ok(db::get_setting(&conn, "dossier_sauvegarde")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| data_dir.join("sauvegardes")))
}

/// Préfixe des sauvegardes automatiques — les seules que la rotation a le
/// droit d'effacer.
const PREFIXE_AUTOMATIQUE: &str = "photocopie-";

/// Une copie de sécurité prise juste avant une restauration
/// ("avant-restauration-…") n'est PAS une sauvegarde automatique : elle ne
/// doit jamais entrer dans la rotation.
///
/// C'est exactement ce qui se passait : la rotation triait tous les
/// `.sqlite3` par nom et effaçait les plus anciens. Or "avant-restauration"
/// passe avant "photocopie" dans l'ordre alphabétique — la copie de secours
/// était donc la PREMIÈRE effacée, au plus tard un quart d'heure après avoir
/// été créée. Un gérant qui se trompait de sauvegarde n'avait alors plus
/// aucun moyen de revenir en arrière, contrairement à ce que promet
/// `restaurer_sauvegarde`.
fn est_sauvegarde_automatique(nom: &str) -> bool {
    nom.starts_with(PREFIXE_AUTOMATIQUE) && nom.ends_with(".sqlite3")
}

fn nettoyer_anciennes_sauvegardes(dossier: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dossier) else {
        return;
    };
    let mut fichiers: Vec<_> = entries
        .flatten()
        .filter(|e| est_sauvegarde_automatique(&e.file_name().to_string_lossy()))
        .collect();
    fichiers.sort_by_key(|e| e.file_name());

    if fichiers.len() > SAUVEGARDES_A_CONSERVER {
        for e in &fichiers[..fichiers.len() - SAUVEGARDES_A_CONSERVER] {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnait_une_sauvegarde_automatique() {
        assert!(est_sauvegarde_automatique("photocopie-20260917-081500.sqlite3"));
    }

    #[test]
    fn la_copie_avant_restauration_echappe_a_la_rotation() {
        // Le cas qui cassait la promesse de `restaurer_sauvegarde` : cette
        // copie doit survivre, c'est le seul retour en arrière possible
        // après une restauration malheureuse.
        assert!(!est_sauvegarde_automatique("avant-restauration-20260917-081500.sqlite3"));
    }

    #[test]
    fn ignore_ce_qui_n_est_pas_une_sauvegarde() {
        assert!(!est_sauvegarde_automatique("photocopie-20260917.txt"));
        assert!(!est_sauvegarde_automatique("notes.sqlite3"));
        assert!(!est_sauvegarde_automatique(""));
    }

    #[test]
    fn la_rotation_ne_touche_qu_aux_sauvegardes_automatiques() {
        let dossier = tempfile::tempdir().expect("dossier temporaire");
        // Plus que la limite, pour forcer la rotation.
        for i in 0..(SAUVEGARDES_A_CONSERVER + 5) {
            std::fs::write(
                dossier.path().join(format!("photocopie-2026091{i:02}-080000.sqlite3")),
                b"x",
            )
            .unwrap();
        }
        let secours = dossier.path().join("avant-restauration-20260101-000000.sqlite3");
        std::fs::write(&secours, b"x").unwrap();

        nettoyer_anciennes_sauvegardes(dossier.path());

        assert!(secours.exists(), "la copie de secours a été effacée par la rotation");
        let restantes = std::fs::read_dir(dossier.path())
            .unwrap()
            .flatten()
            .filter(|e| est_sauvegarde_automatique(&e.file_name().to_string_lossy()))
            .count();
        assert_eq!(restantes, SAUVEGARDES_A_CONSERVER);
    }
}
