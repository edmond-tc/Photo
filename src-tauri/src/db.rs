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
            employe          TEXT,
            raison_ignore    TEXT -- obligatoire quand le fichier est classé sans encaissement
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
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            file_queue_id   INTEGER REFERENCES files_queue(id) ON DELETE SET NULL,
            description     TEXT NOT NULL,
            montant_calcule INTEGER NOT NULL, -- prix proposé par la grille tarifaire, jamais modifié
            montant         INTEGER NOT NULL, -- montant réellement encaissé
            raison_ecart    TEXT,             -- obligatoire si montant != montant_calcule
            moyen_paiement  TEXT NOT NULL,   -- 'especes' | 'mobile_money' | 'credit'
            statut          TEXT NOT NULL DEFAULT 'paye', -- 'paye' | 'impaye'
            employe         TEXT,
            created_at      TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS tarifs_historique (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            service      TEXT NOT NULL,
            libelle      TEXT NOT NULL,
            ancien_prix  INTEGER NOT NULL,
            nouveau_prix INTEGER NOT NULL,
            changed_at   TEXT NOT NULL
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

        -- Ce que font les machines, au fil de la journée (voir activite.rs).
        CREATE TABLE IF NOT EXISTS activite (
            id        INTEGER PRIMARY KEY AUTOINCREMENT,
            heure     TEXT NOT NULL,     -- AAAA-MM-JJTHH:MM:SS, heure locale
            machine   TEXT NOT NULL,
            genre     TEXT NOT NULL,     -- 'impression_pc' | 'sur_la_machine'
            document  TEXT,
            pages     INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_activite_heure ON activite(heure);

        -- Dernier compteur lu sur chaque machine réseau : permet de compter
        -- aussi ce qui a été fait pendant que l'application était fermée.
        CREATE TABLE IF NOT EXISTS compteurs_machines (
            machine TEXT PRIMARY KEY,
            valeur  INTEGER NOT NULL,
            lu_le   TEXT NOT NULL
        );
        ",
    )?;

    appliquer_migrations(&conn)?;
    seed_defaults(&conn)?;

    Ok(conn)
}

/// Version du schéma attendue par cette version du logiciel.
const VERSION_SCHEMA: i64 = 6;

/// Les boutiques déjà installées ont une base créée par une version
/// antérieure : les `CREATE TABLE IF NOT EXISTS` ci-dessus ne leur ajoutent
/// aucune colonne nouvelle. Sans ce mécanisme, livrer une mise à jour qui
/// touche au schéma casserait l'application chez tous les gérants qui ont
/// déjà des données — c'est-à-dire exactement ceux qu'on ne peut pas se
/// permettre de casser. Chaque migration est écrite pour pouvoir être
/// rejouée sans dommage.
fn appliquer_migrations(conn: &Connection) -> rusqlite::Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;

    if version < 1 {
        // Date de règlement réelle d'une commande d'abord prise à crédit.
        ajouter_colonne_si_absente(conn, "transactions", "regle_le", "TEXT")?;
        // Jeton secret remis au client pour qu'il suive SA commande, sans
        // pouvoir consulter celles des autres (voir server.rs).
        ajouter_colonne_si_absente(conn, "files_queue", "jeton", "TEXT")?;

        // Index sur les colonnes réellement interrogées : la page du client
        // interroge son statut toutes les 4,5 secondes et le calcul de la
        // remise fidélité compte les visites par numéro de téléphone — sans
        // index, chaque appel relit toute la table.
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_transactions_file_queue_id
                ON transactions(file_queue_id);
             CREATE INDEX IF NOT EXISTS idx_transactions_created_at
                ON transactions(created_at);
             CREATE INDEX IF NOT EXISTS idx_transactions_statut
                ON transactions(statut);
             CREATE INDEX IF NOT EXISTS idx_files_queue_telephone
                ON files_queue(client_telephone);
             CREATE INDEX IF NOT EXISTS idx_files_queue_received_at
                ON files_queue(received_at);
             CREATE UNIQUE INDEX IF NOT EXISTS idx_files_queue_jeton
                ON files_queue(jeton);",
        )?;
    }

    if version < 2 {
        // Permet au gérant de supprimer le document d'un client sur demande,
        // sans passer par le mot de passe technique (réservé au porteur du
        // projet) — voir gestion::supprimer_document. On garde une trace du
        // fait que le document a été supprimé, jamais de la comptabilité.
        ajouter_colonne_si_absente(
            conn,
            "files_queue",
            "document_supprime",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
    }

    if version < 3 {
        // Confirmation réelle de l'impression, lue dans le spouleur Windows —
        // avant, "Imprimer" ne faisait que transmettre l'ordre à Windows,
        // sans jamais savoir si le papier était vraiment sorti. Voir
        // impression.rs.
        ajouter_colonne_si_absente(
            conn,
            "files_queue",
            "impression_confirmee",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        ajouter_colonne_si_absente(conn, "files_queue", "pages_imprimees", "INTEGER")?;
        ajouter_colonne_si_absente(conn, "files_queue", "impression_erreur", "TEXT")?;
    }

    if version < 4 {
        // Le recto-verso n'était jusqu'ici géré que par la boîte de dialogue
        // Windows, jamais facturé — impossible de savoir si le client a payé
        // le bon tarif. S'ajoute aux colonnes de la version précédente : le
        // détail réel de ce que l'imprimante a reçu (couleur, recto-verso,
        // format, poste, imprimante utilisée) et la comparaison avec ce qui
        // a été facturé — voir impression.rs.
        ajouter_colonne_si_absente(
            conn,
            "files_queue",
            "recto_verso",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        ajouter_colonne_si_absente(conn, "files_queue", "impression_couleur_reelle", "INTEGER")?;
        ajouter_colonne_si_absente(
            conn,
            "files_queue",
            "impression_recto_verso_reelle",
            "INTEGER",
        )?;
        ajouter_colonne_si_absente(conn, "files_queue", "impression_format_reel", "TEXT")?;
        ajouter_colonne_si_absente(conn, "files_queue", "impression_copies_reelles", "INTEGER")?;
        ajouter_colonne_si_absente(conn, "files_queue", "impression_poste", "TEXT")?;
        ajouter_colonne_si_absente(conn, "files_queue", "impression_imprimante_reelle", "TEXT")?;
        ajouter_colonne_si_absente(conn, "files_queue", "impression_ecarts", "TEXT")?;
    }

    if version < 5 {
        // Horodatage exact de la fin de traitement de l'impression — jusqu'ici
        // seule `received_at` (l'arrivée du fichier) était connue. Nécessaire
        // pour le détail chronologique du rapport imprimable (une ligne par
        // impression réussie, triée par heure de fin) et pour calculer le
        // temps d'attente du client. Les lignes déjà confirmées avant cette
        // migration n'ont pas cette date : le rapport se rabat alors sur
        // `received_at` (voir gestion::rapport_periode_impressions).
        ajouter_colonne_si_absente(conn, "files_queue", "impression_confirmee_le", "TEXT")?;
    }

    if version < 6 {
        // Nombre de pages du document, mémorisé séparément du nombre total de
        // feuilles (`copies`, = pages × exemplaires, qui reste la base du
        // calcul du prix et du stock).
        //
        // Sans cette colonne, rouvrir « Détails » réaffichait 1 page ×
        // N feuilles pour une commande saisie en N pages × M exemplaires :
        // un gérant qui rétablissait le vrai nombre de pages multipliait
        // aussitôt le total par ce nombre (10 pages × 2 exemplaires
        // rouverts, corrigés, puis enregistrés = 200 feuilles facturées au
        // lieu de 20). Les lignes antérieures valent 1, ce qui reproduit
        // exactement l'affichage d'avant : aucune commande existante ne
        // change de prix.
        ajouter_colonne_si_absente(
            conn,
            "files_queue",
            "pages_document",
            "INTEGER NOT NULL DEFAULT 1",
        )?;
    }

    conn.pragma_update(None, "user_version", VERSION_SCHEMA)?;
    Ok(())
}

fn ajouter_colonne_si_absente(
    conn: &Connection,
    table: &str,
    colonne: &str,
    type_sql: &str,
) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let existe = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .filter_map(Result::ok)
        .any(|nom| nom == colonne);
    if !existe {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {colonne} {type_sql}"),
            [],
        )?;
    }
    Ok(())
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
