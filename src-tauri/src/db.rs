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
            id               INTEGER PRIMARY KEY AUTOINCREMENT,
            original_name    TEXT NOT NULL,
            path             TEXT NOT NULL,
            client_name      TEXT,
            client_telephone TEXT,
            source           TEXT NOT NULL,   -- 'dossier_surveille' | 'usb' | 'qr'
            kind             TEXT NOT NULL,   -- 'imprimable' | 'editable' | 'inconnu'
            status           TEXT NOT NULL DEFAULT 'en_attente', -- 'en_attente' | 'traite'
            received_at      TEXT NOT NULL,
            taille_octets    INTEGER NOT NULL DEFAULT 0,
            protege          INTEGER NOT NULL DEFAULT 0, -- PDF probablement protégé par mot de passe
            format_detecte   TEXT,             -- ex: 'US Letter' si différent de A4 (indicatif)
            copies           INTEGER NOT NULL DEFAULT 1,
            couleur          INTEGER NOT NULL DEFAULT 0,
            format_papier    TEXT NOT NULL DEFAULT 'A4',
            plage_pages      TEXT,             -- ex: 1-5, 8 -- indicatif, saisi par le client via QR
            finitions        TEXT,             -- JSON: ['agrafage', 'plastification', ...]
            prix             INTEGER,
            employe          TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_files_queue_status
            ON files_queue(status, received_at);

        CREATE TABLE IF NOT EXISTS tarifs (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            service        TEXT NOT NULL UNIQUE,
            libelle        TEXT NOT NULL,
            prix_unitaire  INTEGER NOT NULL,
            unite          TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS transactions (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            file_queue_id  INTEGER REFERENCES files_queue(id) ON DELETE SET NULL,
            description    TEXT NOT NULL,
            montant        INTEGER NOT NULL,
            moyen_paiement TEXT NOT NULL,   -- 'especes' | 'mobile_money' | 'credit'
            statut         TEXT NOT NULL DEFAULT 'paye', -- 'paye' | 'impaye'
            employe        TEXT,
            created_at     TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS depenses (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            description TEXT NOT NULL,
            montant     INTEGER NOT NULL,
            categorie   TEXT NOT NULL,  -- 'papier' | 'encre' | 'electricite' | 'autre'
            created_at  TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS stock (
            item          TEXT PRIMARY KEY,
            libelle       TEXT NOT NULL,
            quantite      REAL NOT NULL,
            seuil_alerte  REAL NOT NULL,
            unite         TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS employes (
            id    INTEGER PRIMARY KEY AUTOINCREMENT,
            nom   TEXT NOT NULL UNIQUE,
            actif INTEGER NOT NULL DEFAULT 1
        );

        CREATE TABLE IF NOT EXISTS clotures_caisse (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            date           TEXT NOT NULL,
            total_attendu  INTEGER NOT NULL,
            total_reel     INTEGER NOT NULL,
            ecart          INTEGER NOT NULL,
            created_at     TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS imprimante_compteur (
            cle                       TEXT PRIMARY KEY,
            feuilles_depuis_entretien INTEGER NOT NULL DEFAULT 0,
            seuil_entretien           INTEGER NOT NULL DEFAULT 2000
        );
        ",
    )?;

    seed_defaults(&conn)?;

    Ok(conn)
}

fn seed_defaults(conn: &Connection) -> rusqlite::Result<()> {
    let tarifs_count: i64 = conn.query_row("SELECT COUNT(*) FROM tarifs", [], |r| r.get(0))?;
    if tarifs_count == 0 {
        let defaults = [
            ("impression_nb", "Impression Noir & Blanc", 25, "page"),
            ("impression_couleur", "Impression Couleur", 100, "page"),
            ("agrafage", "Agrafage", 25, "document"),
            ("reliure_spirale", "Reliure spirale", 500, "document"),
            (
                "reliure_dos_carre",
                "Reliure dos carré collé",
                1500,
                "document",
            ),
            ("plastification", "Plastification", 300, "feuille"),
            ("decoupe", "Découpe / massicotage", 100, "document"),
            ("perforation", "Perforation", 25, "document"),
            ("pliage", "Pliage", 25, "document"),
            ("saisie", "Saisie / dactylographie", 100, "page"),
        ];
        for (service, libelle, prix, unite) in defaults {
            conn.execute(
                "INSERT INTO tarifs (service, libelle, prix_unitaire, unite) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![service, libelle, prix, unite],
            )?;
        }
    }

    let stock_count: i64 = conn.query_row("SELECT COUNT(*) FROM stock", [], |r| r.get(0))?;
    if stock_count == 0 {
        let defaults = [
            ("papier_a4", "Papier A4", 500.0, 100.0, "feuille"),
            ("toner_noir", "Toner / encre noir", 100.0, 20.0, "%"),
            ("toner_couleur", "Toner / encre couleur", 100.0, 20.0, "%"),
        ];
        for (item, libelle, quantite, seuil, unite) in defaults {
            conn.execute(
                "INSERT INTO stock (item, libelle, quantite, seuil_alerte, unite) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![item, libelle, quantite, seuil, unite],
            )?;
        }
    }

    conn.execute(
        "INSERT OR IGNORE INTO imprimante_compteur (cle, feuilles_depuis_entretien, seuil_entretien)
         VALUES ('principale', 0, 2000)",
        [],
    )?;

    Ok(())
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
