use crate::db::{self, DbState};
use crate::files;
use crate::watcher::{self, QueueItem};
use rusqlite::params;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
pub fn get_queue(state: State<DbState>) -> Result<Vec<QueueItem>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, original_name, path, client_name, source, kind, status, received_at
             FROM files_queue
             WHERE status = 'en_attente'
             ORDER BY received_at ASC",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |row| {
            Ok(QueueItem {
                id: row.get(0)?,
                original_name: row.get(1)?,
                path: row.get(2)?,
                client_name: row.get(3)?,
                source: row.get(4)?,
                kind: row.get(5)?,
                status: row.get(6)?,
                received_at: row.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
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
pub fn open_file(state: State<DbState>, id: i64) -> Result<(), String> {
    let path = queue_item_path(&state, id)?;
    files::shell_open(&path, "open")
}

#[tauri::command]
pub fn print_file(state: State<DbState>, id: i64) -> Result<(), String> {
    let path = queue_item_path(&state, id)?;
    files::shell_open(&path, "print")
}

#[tauri::command]
pub fn mark_processed(state: State<DbState>, id: i64) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE files_queue SET status = 'traite' WHERE id = ?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
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
