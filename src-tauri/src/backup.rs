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

fn sauvegarder_une_fois(app: &AppHandle) -> Result<(), String> {
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

fn nettoyer_anciennes_sauvegardes(dossier: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dossier) else {
        return;
    };
    let mut fichiers: Vec<_> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "sqlite3"))
        .collect();
    fichiers.sort_by_key(|e| e.file_name());

    if fichiers.len() > SAUVEGARDES_A_CONSERVER {
        for e in &fichiers[..fichiers.len() - SAUVEGARDES_A_CONSERVER] {
            let _ = std::fs::remove_file(e.path());
        }
    }
}
