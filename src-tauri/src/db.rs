use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

pub struct DbState(pub Mutex<Connection>);

pub fn open(data_dir: &Path) -> rusqlite::Result<Connection> {
    std::fs::create_dir_all(data_dir).expect("impossible de créer le dossier de données");
    let db_path = data_dir.join("photocopie.sqlite3");
    let conn = Connection::open(db_path)?;

    // WAL : résiste mieux aux coupures de courant qu'un journal classique.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS settings (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS files_queue (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            original_name TEXT NOT NULL,
            path          TEXT NOT NULL,
            client_name   TEXT,
            source        TEXT NOT NULL,   -- 'dossier_surveille' | 'usb' | 'qr'
            kind          TEXT NOT NULL,   -- 'imprimable' | 'editable' | 'inconnu'
            status        TEXT NOT NULL DEFAULT 'en_attente', -- 'en_attente' | 'traite'
            received_at   TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_files_queue_status
            ON files_queue(status, received_at);
        ",
    )?;

    Ok(conn)
}

pub fn get_setting(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
        row.get(0)
    })
    .ok()
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )?;
    Ok(())
}
