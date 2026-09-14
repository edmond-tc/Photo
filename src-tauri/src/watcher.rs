use crate::db::DbState;
use crate::files;
use crate::models::QueueItem;
use chrono::Local;
use notify::{Event, EventKind, RecursiveMode, Watcher};
use rusqlite::params;
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

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
                enqueue_file(&app, &path, "dossier_surveille", None, None);
            }
        }
    });
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
    let original_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "fichier".to_string());
    let kind = files::classify(path);
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

    let insert_result = conn.execute(
        "INSERT INTO files_queue
            (original_name, path, client_name, client_telephone, source, kind, status,
             received_at, taille_octets, protege, format_detecte)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'en_attente', ?7, ?8, ?9, ?10)",
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
            format_detecte
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
        copies: 1,
        couleur: false,
        format_papier: "A4".to_string(),
        finitions: vec![],
        prix: None,
        employe: None,
    };

    let _ = app.emit("nouveau-fichier", item);
    Some(id)
}
