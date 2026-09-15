use crate::db::{self, DbState};
use base32::Alphabet;
use chrono::{Local, NaiveDate};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tauri::State;

type HmacSha256 = Hmac<Sha256>;

/// Secret utilisé pour signer les clés de licence, partagé avec l'outil de
/// génération du porteur du projet (`src-tauri/src/bin/generer-licence.rs`,
/// qui appelle ce même module). Mécanisme léger tel que décrit au cahier
/// des charges (section 7) : pas de vérification serveur, mais empêche la
/// génération de clés sans connaître ce secret.
const SECRET: &[u8] = b"890e198971a7863481701131b3b36386156972a265bbaa2c61a78446efa0e328";
const DUREE_ESSAI_JOURS: i64 = 30;

pub fn machine_id() -> String {
    machine_uid::get().unwrap_or_else(|_| "MACHINE-INCONNUE".to_string())
}

fn signature(machine_id: &str, expiration_compacte: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(SECRET).expect("HMAC accepte des clés de toute longueur");
    mac.update(machine_id.as_bytes());
    mac.update(b"|");
    mac.update(expiration_compacte.as_bytes());
    let resultat = mac.finalize().into_bytes();
    base32::encode(Alphabet::Crockford, &resultat[..10])
}

/// Formatte une clé de licence lisible : SIGNATURE-AAAAMMJJ
pub fn generer_cle(machine_id: &str, expiration: NaiveDate) -> String {
    let expiration_compacte = expiration.format("%Y%m%d").to_string();
    format!(
        "{}-{}",
        signature(machine_id, &expiration_compacte),
        expiration_compacte
    )
}

fn verifier_cle(machine_id: &str, cle: &str) -> Option<NaiveDate> {
    let (sig_fournie, expiration_compacte) = cle.trim().rsplit_once('-')?;
    let sig_attendue = signature(machine_id, expiration_compacte);
    if sig_fournie.to_uppercase() != sig_attendue {
        return None;
    }
    NaiveDate::parse_from_str(expiration_compacte, "%Y%m%d").ok()
}

#[derive(serde::Serialize)]
pub struct StatutLicence {
    pub statut: String, // 'essai' | 'actif' | 'expire' | 'invalide'
    pub jours_restants: i64,
    pub machine_id: String,
    pub date_expiration: Option<String>,
}

/// Clé de registre miroir de `essai_debut` (Windows uniquement). Le fichier
/// SQLite de l'app est facile à supprimer pour relancer un essai gratuit —
/// ce second emplacement, moins évident, relève le niveau sans prétendre à
/// une protection absolue (mécanisme "léger" assumé, cf. section 7).
#[cfg(windows)]
mod registre {
    use winreg::enums::*;
    use winreg::RegKey;

    const CHEMIN: &str = r"Software\AtinzPhotocopieBenin";
    const VALEUR: &str = "EssaiDebut";

    pub fn lire() -> Option<String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let cle = hkcu.open_subkey(CHEMIN).ok()?;
        cle.get_value(VALEUR).ok()
    }

    pub fn ecrire(valeur: &str) {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok((cle, _)) = hkcu.create_subkey(CHEMIN) {
            let _ = cle.set_value(VALEUR, &valeur);
        }
    }
}

#[cfg(not(windows))]
mod registre {
    pub fn lire() -> Option<String> {
        None
    }
    pub fn ecrire(_valeur: &str) {}
}

/// Date la plus avancée jamais observée sur cette machine.
///
/// Sans cela, reculer l'horloge de Windows de quelques mois suffit à
/// prolonger l'essai indéfiniment et à faire revivre une licence expirée —
/// l'application ne connaît aucune autre source de temps, puisqu'elle n'est
/// jamais connectée à internet. On ne peut pas empêcher le geste, mais on
/// peut refuser de l'oublier : la date de référence n'avance jamais à
/// reculons.
fn date_de_reference(conn: &rusqlite::Connection) -> chrono::DateTime<Local> {
    let maintenant = Local::now();
    let vue = db::get_setting(conn, "date_maximale_vue")
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
        .map(|d| d.with_timezone(&Local));

    match vue {
        Some(vue) if vue > maintenant => vue,
        _ => {
            let _ = db::set_setting(conn, "date_maximale_vue", &maintenant.to_rfc3339());
            maintenant
        }
    }
}

pub fn assurer_debut_essai(conn: &rusqlite::Connection) {
    let depuis_db = db::get_setting(conn, "essai_debut");
    let depuis_registre = registre::lire();

    let plus_ancienne = [depuis_db, depuis_registre]
        .into_iter()
        .flatten()
        .filter_map(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
        .min()
        .map(|d| d.to_rfc3339())
        .unwrap_or_else(|| Local::now().to_rfc3339());

    let _ = db::set_setting(conn, "essai_debut", &plus_ancienne);
    registre::ecrire(&plus_ancienne);
}

#[tauri::command]
pub fn get_license_status(state: State<DbState>) -> Result<StatutLicence, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let id = machine_id();
    let maintenant = date_de_reference(&conn);

    if let Some(cle) = db::get_setting(&conn, "cle_licence") {
        return Ok(match verifier_cle(&id, &cle) {
            Some(expiration) => {
                let jours = (expiration - maintenant.date_naive()).num_days();
                StatutLicence {
                    statut: if jours >= 0 { "actif" } else { "expire" }.to_string(),
                    jours_restants: jours,
                    machine_id: id,
                    date_expiration: Some(expiration.format("%d/%m/%Y").to_string()),
                }
            }
            None => StatutLicence {
                statut: "invalide".to_string(),
                jours_restants: 0,
                machine_id: id,
                date_expiration: None,
            },
        });
    }

    let debut = db::get_setting(&conn, "essai_debut")
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
        .map(|d| d.with_timezone(&Local))
        .unwrap_or_else(Local::now);
    let jours_ecoules = (maintenant - debut).num_days();
    let jours_restants = DUREE_ESSAI_JOURS - jours_ecoules;

    Ok(StatutLicence {
        statut: if jours_restants > 0 {
            "essai"
        } else {
            "expire"
        }
        .to_string(),
        jours_restants: jours_restants.max(0),
        machine_id: id,
        date_expiration: None,
    })
}

#[tauri::command]
pub fn set_license_key(state: State<DbState>, cle: String) -> Result<bool, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let id = machine_id();
    if verifier_cle(&id, &cle).is_none() {
        return Ok(false);
    }
    db::set_setting(&conn, "cle_licence", &cle).map_err(|e| e.to_string())?;
    Ok(true)
}
