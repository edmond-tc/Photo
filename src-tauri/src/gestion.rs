use crate::db::{self, DbState};
use crate::files;
use crate::models::{Depense, Employe, StockItem, Tarif, Transaction};
use chrono::Local;
use rusqlite::params;
use tauri::{AppHandle, Manager, State};

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

/// Modifie un tarif — journalise systématiquement l'ancien et le nouveau
/// prix, de façon permanente (aucune fonction ne permet d'effacer cet
/// historique), pour qu'un changement de prix ne puisse jamais passer
/// inaperçu (cf. docs/fonctionnalites-confiance.md).
#[tauri::command]
pub fn update_tarif(state: State<DbState>, id: i64, prix_unitaire: i64) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;

    let (service, libelle, ancien_prix): (String, String, i64) = conn
        .query_row(
            "SELECT service, libelle, prix_unitaire FROM tarifs WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| "Tarif introuvable".to_string())?;

    conn.execute(
        "UPDATE tarifs SET prix_unitaire = ?1 WHERE id = ?2",
        params![prix_unitaire, id],
    )
    .map_err(|e| e.to_string())?;

    if ancien_prix != prix_unitaire {
        conn.execute(
            "INSERT INTO tarifs_historique (service, libelle, ancien_prix, nouveau_prix, changed_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![service, libelle, ancien_prix, prix_unitaire, Local::now().to_rfc3339()],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[derive(serde::Serialize)]
pub struct ChangementTarif {
    pub libelle: String,
    pub ancien_prix: i64,
    pub nouveau_prix: i64,
    pub changed_at: String,
}

#[tauri::command]
pub fn list_historique_tarifs(state: State<DbState>) -> Result<Vec<ChangementTarif>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT libelle, ancien_prix, nouveau_prix, changed_at
             FROM tarifs_historique ORDER BY changed_at DESC LIMIT 100",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(ChangementTarif {
                libelle: r.get(0)?,
                ancien_prix: r.get(1)?,
                nouveau_prix: r.get(2)?,
                changed_at: r.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn tarif_prix(conn: &rusqlite::Connection, service: &str) -> i64 {
    conn.query_row(
        "SELECT prix_unitaire FROM tarifs WHERE service = ?1",
        params![service],
        |r| r.get(0),
    )
    .unwrap_or(0)
}

const SEUIL_VISITES_FIDELITE_DEFAUT: i64 = 5;
const REMISE_FIDELITE_POURCENT_DEFAUT: i64 = 10;

/// Le seuil et le pourcentage de la réduction fidélité sont des choix du
/// gérant (Réglages), pas des valeurs qu'on lui impose — un pourcentage à 0
/// désactive simplement la réduction.
/// Les valeurs sont bornées à la lecture, pas seulement à la saisie : une
/// remise de 150% rendrait le prix négatif et bloquerait l'encaissement, et
/// un seuil à 0 ferait afficher "encore -3 commandes avant votre réduction".
pub(crate) fn parametres_fidelite(conn: &rusqlite::Connection) -> (i64, i64) {
    let seuil = db::get_setting(conn, "fidelite_seuil_visites")
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(SEUIL_VISITES_FIDELITE_DEFAUT)
        .clamp(1, 1000);
    let remise_pourcent = db::get_setting(conn, "fidelite_remise_pourcent")
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(REMISE_FIDELITE_POURCENT_DEFAUT)
        .clamp(0, 100);
    (seuil, remise_pourcent)
}

#[derive(serde::Serialize)]
pub struct PrixCalcule {
    pub total: i64,
    pub remise_fidelite_appliquee: bool,
}

/// Calcule un prix indicatif pour un fichier de la file, à partir de la
/// grille tarifaire de la boutique. Le gérant peut toujours l'ajuster à la
/// main avant d'encaisser (cf. `finaliser_commande`). Applique
/// automatiquement une remise fidélité (section 5bis) si le numéro de
/// téléphone du client a déjà réglé plusieurs commandes précédentes.
#[tauri::command]
pub fn calculer_prix(state: State<DbState>, id: i64) -> Result<PrixCalcule, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;

    let (copies, couleur, finitions_json, client_telephone): (
        i64,
        bool,
        Option<String>,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT copies, couleur, finitions, client_telephone FROM files_queue WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
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

    let mut remise_appliquee = false;
    let (seuil, remise_pourcent) = parametres_fidelite(&conn);
    if remise_pourcent > 0 {
        if let Some(telephone) = client_telephone.filter(|t| !t.is_empty()) {
            let visites_payees: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM transactions t
                     JOIN files_queue f ON f.id = t.file_queue_id
                     WHERE f.client_telephone = ?1 AND t.statut = 'paye'",
                    params![telephone],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if visites_payees >= seuil {
                total -= total * remise_pourcent / 100;
                remise_appliquee = true;
            }
        }
    }

    Ok(PrixCalcule {
        total,
        remise_fidelite_appliquee: remise_appliquee,
    })
}

// ───────────── Détails de facturation (pas des options d'impression) ─────────────
// L'impression réelle passe toujours par la boîte de dialogue native de Windows
// (qui gère copies/couleur/recto-verso/format) — ceci ne sert qu'à calculer le
// prix et enregistrer ce qui a été facturé (y compris le recto-verso, pour le
// comparer ensuite à ce que le spouleur a réellement vu passer, voir
// impression::comparer_a_la_facturation).

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn set_print_options(
    state: State<DbState>,
    id: i64,
    copies: i64,
    couleur: bool,
    recto_verso: bool,
    format_papier: String,
    finitions: Vec<String>,
) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let finitions_json = serde_json::to_string(&finitions).map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE files_queue SET copies=?1, couleur=?2, recto_verso=?3, format_papier=?4, finitions=?5
         WHERE id = ?6",
        params![copies.max(1), couleur, recto_verso, format_papier, finitions_json, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ───────────────────────── Finalisation / encaissement ─────────────────────────

#[derive(serde::Serialize)]
pub struct ResultatEncaissement {
    pub transaction_id: i64,
    pub alerte_entretien_imprimante: bool,
}

/// Marque une commande comme traitée, enregistre la transaction et met à
/// jour le stock (approximation : le décompte exact de pages nécessiterait
/// de lire les compteurs de l'imprimante, non implémenté pour l'instant —
/// le gérant peut toujours corriger le stock manuellement dans Rapports).
///
/// Le montant calculé par la grille tarifaire est toujours enregistré à
/// côté du montant réellement encaissé. S'ils diffèrent, une raison est
/// obligatoire — jamais un simple écrasement silencieux du prix (cf.
/// docs/fonctionnalites-confiance.md).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn finaliser_commande(
    state: State<DbState>,
    id: i64,
    montant_calcule: i64,
    montant: i64,
    raison_ecart: Option<String>,
    moyen_paiement: String,
    statut: String,
    employe: Option<String>,
) -> Result<ResultatEncaissement, String> {
    if montant < 0 || montant_calcule < 0 {
        return Err("Le montant ne peut pas être négatif.".to_string());
    }
    let raison_ecart = raison_ecart.filter(|r| !r.trim().is_empty());
    if montant != montant_calcule && raison_ecart.is_none() {
        return Err(
            "Le montant diffère du prix calculé : une raison est obligatoire (ex: remise, \
             négociation, erreur de calcul corrigée)."
                .to_string(),
        );
    }

    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let now = Local::now().to_rfc3339();

    let (original_name, copies, couleur, kind): (String, i64, bool, String) = conn
        .query_row(
            "SELECT original_name, copies, couleur, kind FROM files_queue WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(|_| "Fichier introuvable".to_string())?;

    // Condition `status='en_attente'` volontaire : sans elle, un double-clic
    // (ou un encaissement relancé sur une commande déjà réglée) insérerait
    // une deuxième transaction pour le même document et gonflerait la recette
    // du jour sans que personne ne s'en aperçoive.
    let lignes_modifiees = conn
        .execute(
            "UPDATE files_queue SET status='traite', prix=?1, employe=?2
             WHERE id=?3 AND status='en_attente'",
            params![montant, employe, id],
        )
        .map_err(|e| e.to_string())?;
    if lignes_modifiees == 0 {
        return Err(
            "Cette commande a déjà été encaissée — elle n'est plus dans la file d'attente."
                .to_string(),
        );
    }

    conn.execute(
        "INSERT INTO transactions
            (file_queue_id, description, montant_calcule, montant, raison_ecart,
             moyen_paiement, statut, employe, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id,
            original_name,
            montant_calcule,
            montant,
            raison_ecart,
            moyen_paiement,
            statut,
            employe,
            now
        ],
    )
    .map_err(|e| e.to_string())?;
    let transaction_id = conn.last_insert_rowid();

    let mut alerte_entretien = false;
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

        let _ = conn.execute(
            "UPDATE imprimante_compteur SET feuilles_depuis_entretien = feuilles_depuis_entretien + ?1
             WHERE cle = 'principale'",
            params![copies],
        );
        let (feuilles, seuil): (i64, i64) = conn
            .query_row(
                "SELECT feuilles_depuis_entretien, seuil_entretien FROM imprimante_compteur WHERE cle = 'principale'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or((0, i64::MAX));
        alerte_entretien = feuilles >= seuil;
    }

    Ok(ResultatEncaissement {
        transaction_id,
        alerte_entretien_imprimante: alerte_entretien,
    })
}

/// Remet le compteur à zéro après un entretien réel de l'imprimante.
#[tauri::command]
pub fn reinitialiser_compteur_imprimante(state: State<DbState>) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE imprimante_compteur SET feuilles_depuis_entretien = 0 WHERE cle = 'principale'",
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ───────────────────────────── Reçu imprimable ─────────────────────────────

/// Génère un reçu texte simple pour une transaction et l'envoie directement
/// à l'impression (même mécanisme que pour les fichiers clients : dialogue
/// Windows natif). Le texte brut s'imprime sans dépendance supplémentaire —
/// pas besoin d'un générateur de PDF pour un reçu de quelques lignes.
#[tauri::command]
pub fn imprimer_recu(app: AppHandle, transaction_id: i64) -> Result<(), String> {
    let state = app.state::<DbState>();
    let conn = state.0.lock().map_err(|e| e.to_string())?;

    let (description, montant, moyen_paiement, employe, created_at): (
        String,
        i64,
        String,
        Option<String>,
        String,
    ) = conn
        .query_row(
            "SELECT description, montant, moyen_paiement, employe, created_at
             FROM transactions WHERE id = ?1",
            params![transaction_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .map_err(|_| "Transaction introuvable".to_string())?;

    let boutique_nom =
        db::get_setting(&conn, "boutique_nom").unwrap_or_else(|| "Photocopie".to_string());
    let logo_chemin = db::get_setting(&conn, "boutique_logo_chemin");
    drop(conn);

    let moyen_libelle = match moyen_paiement.as_str() {
        "especes" => "Espèces",
        "mobile_money" => "Mobile Money",
        "credit" => "À crédit",
        autre => autre,
    };
    let date_lisible = chrono::DateTime::parse_from_rfc3339(&created_at)
        .map(|d| d.format("%d/%m/%Y %H:%M").to_string())
        .unwrap_or(created_at);
    let employe_ligne = employe
        .map(|e| format!("Servi par : {e}"))
        .unwrap_or_default();

    // Le nom du fichier (description) vient du client via le formulaire QR,
    // donc non fiable — on l'échappe avant de l'injecter dans le HTML du
    // reçu pour empêcher une injection de script (ex: nom de fichier
    // "<script>...</script>.pdf").
    let echapper_html = |s: &str| -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    let boutique_nom_html = echapper_html(&boutique_nom);
    let description_html = echapper_html(&description);
    let employe_ligne_html = echapper_html(&employe_ligne);
    let moyen_libelle_html = echapper_html(moyen_libelle);

    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let dossier_recus = data_dir.join("recus_emis");
    std::fs::create_dir_all(&dossier_recus).map_err(|e| e.to_string())?;

    // Avec logo configuré : reçu HTML (s'imprime via le navigateur par défaut).
    // Sans logo : texte brut, plus simple et tout aussi fonctionnel.
    let logo_base64 = logo_chemin
        .as_ref()
        .and_then(|p| std::fs::read(p).ok())
        .map(|octets| {
            use base64::engine::general_purpose::STANDARD;
            use base64::Engine;
            STANDARD.encode(octets)
        });

    let chemin = if let Some(logo_base64) = logo_base64 {
        let chemin = dossier_recus.join(format!("recu_{transaction_id}.html"));
        let html = format!(
            r#"<!doctype html><html lang="fr"><head><meta charset="utf-8">
<title>Reçu n°{transaction_id}</title>
<style>
  body {{ font-family: "Segoe UI", Calibri, Arial, sans-serif; max-width: 320px; margin: 1rem auto; color:#222; }}
  img {{ max-width: 120px; display:block; margin: 0 auto 0.5rem; }}
  h1 {{ text-align:center; font-size:1.1rem; margin: 0 0 1rem; }}
  hr {{ border: none; border-top: 1px dashed #999; margin: 0.75rem 0; }}
  .ligne {{ display:flex; justify-content:space-between; font-size:0.9rem; margin:0.2rem 0; }}
  .merci {{ text-align:center; margin-top:1rem; font-size:0.85rem; }}
</style></head><body>
<img src="data:image/png;base64,{logo_base64}" alt="Logo" />
<h1>{boutique_nom_html}</h1>
<div class="ligne"><span>Reçu n°</span><span>{transaction_id}</span></div>
<div class="ligne"><span>Date</span><span>{date_lisible}</span></div>
<hr>
<div class="ligne"><span>{description_html}</span></div>
<div class="ligne"><strong>Montant</strong><strong>{montant} FCFA</strong></div>
<div class="ligne"><span>Paiement</span><span>{moyen_libelle_html}</span></div>
<div class="ligne"><span>{employe_ligne_html}</span></div>
<hr>
<p class="merci">Merci de votre visite !</p>
</body></html>"#
        );
        std::fs::write(&chemin, html).map_err(|e| e.to_string())?;
        chemin
    } else {
        let separateur = "=".repeat(32);
        let contenu = format!(
            "{separateur}\n{boutique_nom:^32}\n{separateur}\n\
             Reçu n°{transaction_id}\n\
             Date : {date_lisible}\n\
             {tiret}\n\
             {description}\n\
             Montant : {montant} FCFA\n\
             Paiement : {moyen_libelle}\n\
             {employe_ligne}\n\
             {separateur}\n\
             {merci:^32}\n\
             {separateur}\n",
            tiret = "-".repeat(32),
            merci = "Merci de votre visite !",
        );
        let chemin = dossier_recus.join(format!("recu_{transaction_id}.txt"));
        std::fs::write(&chemin, contenu).map_err(|e| e.to_string())?;
        chemin
    };

    files::shell_open(&chemin, "print")
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
            "SELECT id, file_queue_id, description, montant_calcule, montant, raison_ecart,
                    moyen_paiement, statut, employe, created_at
             FROM transactions ORDER BY created_at DESC LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![limite], |r| {
            Ok(Transaction {
                id: r.get(0)?,
                file_queue_id: r.get(1)?,
                description: r.get(2)?,
                montant_calcule: r.get(3)?,
                montant: r.get(4)?,
                raison_ecart: r.get(5)?,
                moyen_paiement: r.get(6)?,
                statut: r.get(7)?,
                employe: r.get(8)?,
                created_at: r.get(9)?,
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

fn calculer_rapport(conn: &rusqlite::Connection, date: String) -> RapportJour {
    let motif = format!("{date}%");

    // COALESCE(regle_le, created_at) = le jour où l'argent est réellement
    // entré en caisse : la date de la vente pour un paiement immédiat, la
    // date du règlement pour une dette soldée plus tard.
    let nombre_commandes: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions
             WHERE COALESCE(regle_le, created_at) LIKE ?1 AND statut='paye'",
            params![motif],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let total_encaisse: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(montant), 0) FROM transactions
             WHERE COALESCE(regle_le, created_at) LIKE ?1 AND statut='paye'",
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

    RapportJour {
        date,
        nombre_commandes,
        total_encaisse,
        total_impaye,
        total_depenses,
        benefice_net: total_encaisse - total_depenses,
    }
}

#[tauri::command]
pub fn rapport_du_jour(state: State<DbState>) -> Result<RapportJour, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let date = Local::now().format("%Y-%m-%d").to_string();
    Ok(calculer_rapport(&conn, date))
}

/// Pour le message d'accueil du matin : "hier, vous aviez fait X commandes".
#[tauri::command]
pub fn rapport_hier(state: State<DbState>) -> Result<RapportJour, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let hier = Local::now() - chrono::Duration::days(1);
    let date = hier.format("%Y-%m-%d").to_string();
    Ok(calculer_rapport(&conn, date))
}

#[derive(serde::Serialize)]
pub struct RaisonCompte {
    pub raison: String,
    pub nombre: i64,
}

#[derive(serde::Serialize)]
pub struct RapportReconciliation {
    pub date: String,
    pub recus: i64,
    pub payes: i64,
    pub ignores: i64,
    pub ignores_par_raison: Vec<RaisonCompte>,
    pub en_attente: i64,
    /// Doit toujours valoir 0 : recus - (payes + ignores + en_attente).
    /// Un chiffre différent de zéro signalerait un bug, pas une fraude —
    /// chaque fichier reçu a forcément l'un de ces trois statuts.
    pub ecart_verification: i64,
}

/// Compare les fichiers reçus (comptés automatiquement dès leur arrivée,
/// avant tout geste humain) aux fichiers payés, ignorés (avec raison) et
/// encore en attente. Aucun fichier ne peut disparaître de cette
/// comptabilité sans laisser de trace (cf. docs/fonctionnalites-confiance.md).
#[tauri::command]
pub fn rapport_reconciliation(state: State<DbState>) -> Result<RapportReconciliation, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let date = Local::now().format("%Y-%m-%d").to_string();
    let motif = format!("{date}%");

    let recus: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM files_queue WHERE received_at LIKE ?1",
            params![motif],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let payes: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM files_queue WHERE received_at LIKE ?1 AND status = 'traite' AND raison_ignore IS NULL",
            params![motif],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let ignores: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM files_queue WHERE received_at LIKE ?1 AND status = 'traite' AND raison_ignore IS NOT NULL",
            params![motif],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let en_attente: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM files_queue WHERE received_at LIKE ?1 AND status = 'en_attente'",
            params![motif],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let mut stmt = conn
        .prepare(
            "SELECT raison_ignore, COUNT(*) FROM files_queue
             WHERE received_at LIKE ?1 AND status = 'traite' AND raison_ignore IS NOT NULL
             GROUP BY raison_ignore ORDER BY COUNT(*) DESC",
        )
        .map_err(|e| e.to_string())?;
    let ignores_par_raison = stmt
        .query_map(params![motif], |r| {
            Ok(RaisonCompte {
                raison: r.get(0)?,
                nombre: r.get(1)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(RapportReconciliation {
        date,
        recus,
        payes,
        ignores,
        ignores_par_raison,
        en_attente,
        ecart_verification: recus - (payes + ignores + en_attente),
    })
}

/// Prépare une valeur pour le CSV.
///
/// Deux pièges, tous deux déclenchables par un simple nom de fichier choisi
/// par le client (la description d'une transaction est le nom du document
/// qu'il a envoyé) :
///   - un point-virgule, un guillemet ou un retour à la ligne casse la
///     structure du fichier et décale toutes les colonnes suivantes ;
///   - une valeur commençant par =, +, - ou @ est interprétée par Excel
///     comme une FORMULE à exécuter à l'ouverture du fichier. Un client
///     pourrait ainsi nommer son document `=cmd|'/c ...'!A1.pdf` et faire
///     exécuter une commande sur le PC du gérant le jour où il exporte sa
///     comptabilité.
/// On met donc tout entre guillemets (en doublant les guillemets internes,
/// comme le veut le format CSV) et on préfixe d'une apostrophe les valeurs
/// qui seraient prises pour des formules.
fn champ_csv(valeur: &str) -> String {
    let neutralise = if valeur.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{valeur}")
    } else {
        valeur.to_string()
    };
    format!("\"{}\"", neutralise.replace('"', "\"\""))
}

#[tauri::command]
pub fn exporter_transactions_csv(state: State<DbState>) -> Result<String, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT created_at, description, montant_calcule, montant, COALESCE(raison_ecart, ''),
                    moyen_paiement, statut, COALESCE(employe, '')
             FROM transactions ORDER BY created_at DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            let created_at: String = r.get(0)?;
            let description: String = r.get(1)?;
            let montant_calcule: i64 = r.get(2)?;
            let montant: i64 = r.get(3)?;
            let raison_ecart: String = r.get(4)?;
            let moyen: String = r.get(5)?;
            let statut: String = r.get(6)?;
            let employe: String = r.get(7)?;
            Ok(format!(
                "{};{};{montant_calcule};{montant};{};{};{};{}",
                champ_csv(&created_at),
                champ_csv(&description),
                champ_csv(&raison_ecart),
                champ_csv(&moyen),
                champ_csv(&statut),
                champ_csv(&employe)
            ))
        })
        .map_err(|e| e.to_string())?;

    // BOM UTF-8 en tête : sans lui, Excel (notamment en français) affiche mal
    // les accents d'un CSV ouvert par double-clic.
    let mut csv = String::from(
        "\u{FEFF}date;description;montant_calcule_fcfa;montant_encaisse_fcfa;raison_ecart;moyen_paiement;statut;employe\n",
    );
    for row in rows {
        csv.push_str(&row.map_err(|e| e.to_string())?);
        csv.push('\n');
    }
    Ok(csv)
}

// ───────────────────────────── Impayés ─────────────────────────────

#[derive(serde::Serialize)]
pub struct Impaye {
    pub transaction_id: i64,
    pub description: String,
    pub montant: i64,
    pub client_name: Option<String>,
    pub client_telephone: Option<String>,
    pub created_at: String,
}

/// Liste les commandes réglées "à crédit" et jamais soldées depuis (section
/// 5bis : "rappel des impayés, avec relance suggérée"). Une fois payé, le
/// gérant retrouve le client dans Recherche et enregistre un nouvel
/// encaissement — il n'y a pas de "solder" séparé pour rester simple.
#[tauri::command]
pub fn list_impayes(state: State<DbState>) -> Result<Vec<Impaye>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT t.id, t.description, t.montant, f.client_name, f.client_telephone, t.created_at
             FROM transactions t
             LEFT JOIN files_queue f ON f.id = t.file_queue_id
             WHERE t.statut = 'impaye'
             ORDER BY t.created_at ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Impaye {
                transaction_id: r.get(0)?,
                description: r.get(1)?,
                montant: r.get(2)?,
                client_name: r.get(3)?,
                client_telephone: r.get(4)?,
                created_at: r.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn marquer_impaye_regle(state: State<DbState>, transaction_id: i64) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    // `regle_le` est indispensable : sans lui, l'argent d'une dette réglée
    // aujourd'hui serait compté dans la recette du jour où la commande avait
    // été prise (parfois des semaines plus tôt). La caisse du soir afficherait
    // alors un excédent inexpliqué, et le rapport d'un jour déjà clôturé
    // changerait après coup.
    let lignes = conn
        .execute(
            "UPDATE transactions SET statut = 'paye', regle_le = ?2
             WHERE id = ?1 AND statut = 'impaye'",
            params![transaction_id, Local::now().to_rfc3339()],
        )
        .map_err(|e| e.to_string())?;
    if lignes == 0 {
        return Err("Cette dette a déjà été réglée.".to_string());
    }
    Ok(())
}

// ───────────────────────────── Clôture de caisse ─────────────────────────────

#[derive(serde::Serialize)]
pub struct ResultatCloture {
    pub total_attendu: i64,
    pub total_reel: i64,
    pub ecart: i64,
}

/// Compare ce que la caisse devrait contenir (somme des encaissements du
/// jour enregistrés dans l'app) à ce que le gérant compte réellement, pour
/// repérer un écart immédiatement (section 5bis).
#[tauri::command]
pub fn cloturer_caisse(state: State<DbState>, total_reel: i64) -> Result<ResultatCloture, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let date = Local::now().format("%Y-%m-%d").to_string();
    let motif = format!("{date}%");

    // Ce qui devrait être dans le tiroir ce soir : les ventes réglées en
    // espèces aujourd'hui, plus les dettes soldées aujourd'hui (réglées en
    // main propre dans l'immense majorité des cas). Les ignorer ferait
    // apparaître un excédent inexpliqué chaque fois qu'un client vient payer
    // une ancienne ardoise.
    let total_attendu: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(montant), 0) FROM transactions
             WHERE statut = 'paye'
               AND ( (regle_le IS NULL AND created_at LIKE ?1 AND moyen_paiement = 'especes')
                  OR (regle_le IS NOT NULL AND regle_le LIKE ?1) )",
            params![motif],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let ecart = total_reel - total_attendu;

    conn.execute(
        "INSERT INTO clotures_caisse (date, total_attendu, total_reel, ecart, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            date,
            total_attendu,
            total_reel,
            ecart,
            Local::now().to_rfc3339()
        ],
    )
    .map_err(|e| e.to_string())?;

    Ok(ResultatCloture {
        total_attendu,
        total_reel,
        ecart,
    })
}

// ───────────────────────────── Mode démonstration ─────────────────────────────

/// Ajoute quelques commandes factices dans la file d'attente, pour qu'un
/// agent terrain puisse faire une démonstration sans client réel sur place
/// (suggestion de la section 10 du cahier des charges).
#[tauri::command]
pub fn activer_mode_demo(app: AppHandle) -> Result<(), String> {
    let state = app.state::<DbState>();
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let now = Local::now().to_rfc3339();

    let exemples = [
        (
            "Devoir_Maths_Terminale.pdf",
            "imprimable",
            Some("Chimène A."),
        ),
        ("CV_Candidature.docx", "editable", Some("Yves K.")),
        ("Presentation.pages", "inconnu", None),
    ];
    for (nom, kind, client) in exemples {
        conn.execute(
            "INSERT INTO files_queue
                (original_name, path, client_name, source, kind, status, received_at)
             VALUES (?1, ?2, ?3, 'demo', ?4, 'en_attente', ?5)",
            params![nom, format!("demo://{nom}"), client, kind, now],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn desactiver_mode_demo(state: State<DbState>) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM files_queue WHERE source = 'demo'", [])
        .map_err(|e| e.to_string())?;
    Ok(())
}
