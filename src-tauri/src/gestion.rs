use crate::db::DbState;
use crate::models::{Depense, Employe, StockItem, Tarif, Transaction};
use chrono::Local;
use rusqlite::params;
use tauri::State;

// ───────────────────────────── Tarifs ─────────────────────────────

#[tauri::command]
pub fn list_tarifs(state: State<DbState>) -> Result<Vec<Tarif>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT id, service, libelle, prix_unitaire, unite FROM tarifs ORDER BY libelle")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Tarif {
                id: r.get(0)?,
                service: r.get(1)?,
                libelle: r.get(2)?,
                prix_unitaire: r.get(3)?,
                unite: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_tarif(state: State<DbState>, id: i64, prix_unitaire: i64) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE tarifs SET prix_unitaire = ?1 WHERE id = ?2",
        params![prix_unitaire, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn tarif_prix(conn: &rusqlite::Connection, service: &str) -> i64 {
    conn.query_row(
        "SELECT prix_unitaire FROM tarifs WHERE service = ?1",
        params![service],
        |r| r.get(0),
    )
    .unwrap_or(0)
}

/// Calcule un prix indicatif pour un fichier de la file, à partir de la
/// grille tarifaire de la boutique. Le gérant peut toujours l'ajuster à la
/// main avant d'encaisser (cf. `finaliser_commande`).
#[tauri::command]
pub fn calculer_prix(state: State<DbState>, id: i64) -> Result<i64, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;

    let (copies, couleur, finitions_json): (i64, bool, Option<String>) = conn
        .query_row(
            "SELECT copies, couleur, finitions FROM files_queue WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| "Fichier introuvable".to_string())?;

    let prix_page = if couleur {
        tarif_prix(&conn, "impression_couleur")
    } else {
        tarif_prix(&conn, "impression_nb")
    };
    let mut total = prix_page * copies.max(1);

    if let Some(json) = finitions_json {
        if let Ok(finitions) = serde_json::from_str::<Vec<String>>(&json) {
            for f in finitions {
                total += tarif_prix(&conn, &f);
            }
        }
    }

    Ok(total)
}

// ───────────────────────── Options d'impression ─────────────────────────

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn set_print_options(
    state: State<DbState>,
    id: i64,
    copies: i64,
    couleur: bool,
    format_papier: String,
    recto_verso: bool,
    orientation: String,
    finitions: Vec<String>,
) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let finitions_json = serde_json::to_string(&finitions).map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE files_queue SET copies=?1, couleur=?2, format_papier=?3, recto_verso=?4,
                                 orientation=?5, finitions=?6
         WHERE id = ?7",
        params![
            copies.max(1),
            couleur,
            format_papier,
            recto_verso,
            orientation,
            finitions_json,
            id
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ───────────────────────── Finalisation / encaissement ─────────────────────────

/// Marque une commande comme traitée, enregistre la transaction et met à
/// jour le stock (approximation : le décompte exact de pages nécessiterait
/// de lire les compteurs de l'imprimante, non implémenté pour l'instant —
/// le gérant peut toujours corriger le stock manuellement dans Réglages).
#[tauri::command]
pub fn finaliser_commande(
    state: State<DbState>,
    id: i64,
    montant: i64,
    moyen_paiement: String,
    statut: String,
    employe: Option<String>,
) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let now = Local::now().to_rfc3339();

    let (original_name, copies, couleur, kind): (String, i64, bool, String) = conn
        .query_row(
            "SELECT original_name, copies, couleur, kind FROM files_queue WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(|_| "Fichier introuvable".to_string())?;

    conn.execute(
        "UPDATE files_queue SET status='traite', prix=?1, employe=?2 WHERE id=?3",
        params![montant, employe, id],
    )
    .map_err(|e| e.to_string())?;

    conn.execute(
        "INSERT INTO transactions (file_queue_id, description, montant, moyen_paiement, statut, employe, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, original_name, montant, moyen_paiement, statut, employe, now],
    )
    .map_err(|e| e.to_string())?;

    if kind == "imprimable" {
        let _ = conn.execute(
            "UPDATE stock SET quantite = MAX(0, quantite - ?1) WHERE item = 'papier_a4'",
            params![copies as f64],
        );
        let toner_item = if couleur {
            "toner_couleur"
        } else {
            "toner_noir"
        };
        let _ = conn.execute(
            "UPDATE stock SET quantite = MAX(0, quantite - ?1) WHERE item = ?2",
            params![copies as f64 * 0.2, toner_item],
        );
    }

    Ok(())
}

// ───────────────────────────── Stock ─────────────────────────────

#[tauri::command]
pub fn list_stock(state: State<DbState>) -> Result<Vec<StockItem>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT item, libelle, quantite, seuil_alerte, unite FROM stock ORDER BY libelle")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            let quantite: f64 = r.get(2)?;
            let seuil: f64 = r.get(3)?;
            Ok(StockItem {
                item: r.get(0)?,
                libelle: r.get(1)?,
                quantite,
                seuil_alerte: seuil,
                unite: r.get(4)?,
                alerte: quantite <= seuil,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn ajuster_stock(state: State<DbState>, item: String, quantite: f64) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE stock SET quantite = ?1 WHERE item = ?2",
        params![quantite, item],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ───────────────────────────── Dépenses ─────────────────────────────

#[tauri::command]
pub fn list_depenses(state: State<DbState>) -> Result<Vec<Depense>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT id, description, montant, categorie, created_at FROM depenses ORDER BY created_at DESC LIMIT 200")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Depense {
                id: r.get(0)?,
                description: r.get(1)?,
                montant: r.get(2)?,
                categorie: r.get(3)?,
                created_at: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn ajouter_depense(
    state: State<DbState>,
    description: String,
    montant: i64,
    categorie: String,
) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO depenses (description, montant, categorie, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![description, montant, categorie, Local::now().to_rfc3339()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ───────────────────────────── Employés ─────────────────────────────

#[tauri::command]
pub fn list_employes(state: State<DbState>) -> Result<Vec<Employe>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT id, nom, actif FROM employes ORDER BY nom")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Employe {
                id: r.get(0)?,
                nom: r.get(1)?,
                actif: r.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn ajouter_employe(state: State<DbState>, nom: String) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT OR IGNORE INTO employes (nom, actif) VALUES (?1, 1)",
        params![nom],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ───────────────────────────── Historique / rapports ─────────────────────────────

#[tauri::command]
pub fn list_transactions(state: State<DbState>, limite: i64) -> Result<Vec<Transaction>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, file_queue_id, description, montant, moyen_paiement, statut, employe, created_at
             FROM transactions ORDER BY created_at DESC LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![limite], |r| {
            Ok(Transaction {
                id: r.get(0)?,
                file_queue_id: r.get(1)?,
                description: r.get(2)?,
                montant: r.get(3)?,
                moyen_paiement: r.get(4)?,
                statut: r.get(5)?,
                employe: r.get(6)?,
                created_at: r.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
pub struct RapportJour {
    pub date: String,
    pub nombre_commandes: i64,
    pub total_encaisse: i64,
    pub total_impaye: i64,
    pub total_depenses: i64,
    pub benefice_net: i64,
}

#[tauri::command]
pub fn rapport_du_jour(state: State<DbState>) -> Result<RapportJour, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let date = Local::now().format("%Y-%m-%d").to_string();
    let motif = format!("{date}%");

    let nombre_commandes: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE created_at LIKE ?1 AND statut='paye'",
            params![motif],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let total_encaisse: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(montant), 0) FROM transactions WHERE created_at LIKE ?1 AND statut='paye'",
            params![motif],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let total_impaye: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(montant), 0) FROM transactions WHERE created_at LIKE ?1 AND statut='impaye'",
            params![motif],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let total_depenses: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(montant), 0) FROM depenses WHERE created_at LIKE ?1",
            params![motif],
            |r| r.get(0),
        )
        .unwrap_or(0);

    Ok(RapportJour {
        date,
        nombre_commandes,
        total_encaisse,
        total_impaye,
        total_depenses,
        benefice_net: total_encaisse - total_depenses,
    })
}

#[tauri::command]
pub fn exporter_transactions_csv(state: State<DbState>) -> Result<String, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT created_at, description, montant, moyen_paiement, statut, COALESCE(employe, '')
             FROM transactions ORDER BY created_at DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            let created_at: String = r.get(0)?;
            let description: String = r.get(1)?;
            let montant: i64 = r.get(2)?;
            let moyen: String = r.get(3)?;
            let statut: String = r.get(4)?;
            let employe: String = r.get(5)?;
            Ok(format!(
                "{created_at};{};{montant};{moyen};{statut};{employe}",
                description.replace(';', ",")
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut csv = String::from("date;description;montant_fcfa;moyen_paiement;statut;employe\n");
    for row in rows {
        csv.push_str(&row.map_err(|e| e.to_string())?);
        csv.push('\n');
    }
    Ok(csv)
}
