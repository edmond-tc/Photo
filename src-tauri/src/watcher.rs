use crate::db::DbState;
use crate::files;
use chrono::Local;
use notify::{Event, EventKind, RecursiveMode, Watcher};
use rusqlite::params;
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, serde::Serialize)]
pub struct QueueItem {
    pub id: i64,
    pub original_name: String,
    pub path: String,
    pub client_name: Option<String>,
    pub source: String,
    pub kind: String,
    pub status: String,
    pub received_at: String,
}

/// Démarre la surveillance du dossier de réception dans un thread dédié.
/// À chaque fichier nouvellement créé, on l'enregistre en base et on
/// prévient l'interface via un événement Tauri — sans rechargement de page.
pub fn watch_folder(app: AppHandle, folder: PathBuf) {
    thread::spawn(move || {
        let (tx, rx) = channel::<notify::Result<Event>>();

        let mut watcher = match notify::recommended_watcher(tx) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("Impossible de démarrer la surveillance du dossier : {e}");
                return;
            }
        };

        if let Err(e) = watcher.watch(&folder, RecursiveMode::NonRecursive) {
            eprintln!(
                "Impossible de surveiller le dossier {} : {e}",
                folder.display()
            );
            return;
        }

        for res in rx {
            let Ok(event) = res else { continue };
            if !matches!(event.kind, EventKind::Create(_)) {
                continue;
            }
            for path in event.paths {
                if !path.is_file() {
                    continue;
                }
                // Laisse le temps à une copie de fichier de se terminer avant de la traiter.
                thread::sleep(Duration::from_millis(600));
                handle_new_file(&app, &path);
            }
        }
    });
}

fn handle_new_file(app: &AppHandle, path: &std::path::Path) {
    let original_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "fichier".to_string());
    let kind = files::classify(path);
    let received_at = Local::now().to_rfc3339();

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
        return;
    }

    let insert_result = conn.execute(
        "INSERT INTO files_queue (original_name, path, client_name, source, kind, status, received_at)
         VALUES (?1, ?2, NULL, 'dossier_surveille', ?3, 'en_attente', ?4)",
        params![original_name, path.to_string_lossy(), kind, received_at],
    );
    drop(conn);

    if let Err(e) = insert_result {
        eprintln!("Échec d'enregistrement du fichier reçu : {e}");
        return;
    }

    let conn = state.0.lock().expect("verrou base de données corrompu");
    let id = conn.last_insert_rowid();
    drop(conn);

    let item = QueueItem {
        id,
        original_name,
        path: path.to_string_lossy().to_string(),
        client_name: None,
        source: "dossier_surveille".to_string(),
        kind: kind.to_string(),
        status: "en_attente".to_string(),
        received_at,
    };

    let _ = app.emit("nouveau-fichier", item);
}
