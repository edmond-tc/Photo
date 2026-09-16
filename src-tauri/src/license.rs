use crate::db::{self, DbState};
use base32::Alphabet;
use chrono::{Local, NaiveDate};
use tauri::State;

/// Clé PUBLIQUE de vérification des licences.
///
/// Elle ne permet que de vérifier une clé, jamais d'en fabriquer une : on
/// peut donc la livrer dans chaque installation sans aucun risque. La clé
/// privée correspondante, seule capable de signer, reste chez le porteur du
/// projet (secret du tableau de bord Cloudflare) et n'est présente dans
/// aucun fichier livré ni dans le dépôt.
///
/// C'est tout l'intérêt du changement : avant, la même valeur servait à
/// signer ET à vérifier, elle était donc forcément dans l'exe distribué —
/// n'importe qui pouvait l'en extraire et se fabriquer des licences à vie.
const CLE_PUBLIQUE: [u8; 32] = [
    45, 128, 43, 247, 147, 198, 43, 195, 181, 78, 252, 243, 244, 6, 244, 54, 167, 204, 112, 248,
    108, 15, 214, 235, 109, 226, 1, 168, 95, 187, 33, 144,
];

const DUREE_ESSAI_JOURS: i64 = 30;

pub fn machine_id() -> String {
    machine_uid::get().unwrap_or_else(|_| "MACHINE-INCONNUE".to_string())
}

/// Ce qui est signé : l'identifiant de la machine et la date d'expiration.
/// Une clé fabriquée pour un PC ne vaut donc rien sur un autre.
fn message_a_signer(machine_id: &str, expiration_compacte: &str) -> Vec<u8> {
    format!("{machine_id}|{expiration_compacte}").into_bytes()
}

/// Signe une clé de licence. Réservé à l'outil du porteur du projet, qui
/// fournit lui-même la clé privée — elle n'est écrite nulle part ici.
pub fn generer_cle(
    cle_privee: &ed25519_dalek::SigningKey,
    machine_id: &str,
    expiration: NaiveDate,
) -> String {
    use ed25519_dalek::Signer;
    let expiration_compacte = expiration.format("%Y%m%d").to_string();
    let signature = cle_privee.sign(&message_a_signer(machine_id, &expiration_compacte));
    format!(
        "{}-{}",
        base32::encode(Alphabet::Crockford, &signature.to_bytes()),
        expiration_compacte
    )
}

fn signature_valide(machine_id: &str, expiration_compacte: &str, signature: &str) -> bool {
    use ed25519_dalek::{Signature, VerifyingKey};

    let message = message_a_signer(machine_id, expiration_compacte);

    let Some(octets) = base32::decode(Alphabet::Crockford, signature) else {
        return false;
    };
    let Ok(octets) = <[u8; 64]>::try_from(octets.as_slice()) else {
        return false;
    };
    let Ok(cle) = VerifyingKey::from_bytes(&CLE_PUBLIQUE) else {
        return false;
    };
    cle.verify_strict(&message, &Signature::from_bytes(&octets))
        .is_ok()
}

fn verifier_cle(machine_id: &str, cle: &str) -> Option<NaiveDate> {
    // Une clé collée depuis WhatsApp arrive souvent avec des espaces ou un
    // retour à la ligne : ce n'est pas une raison de refuser le gérant.
    let cle: String = cle.chars().filter(|c| !c.is_whitespace()).collect();
    let cle = cle.to_uppercase();

    let (signature, expiration_compacte) = cle.rsplit_once('-')?;
    let expiration = NaiveDate::parse_from_str(expiration_compacte, "%Y%m%d").ok()?;

    if signature_valide(machine_id, expiration_compacte, signature) {
        Some(expiration)
    } else {
        None
    }
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

/// Émis vers l'écran pour qu'il se rafraîchisse aussitôt, sans attendre le
/// prochain contrôle périodique du blocage.
#[derive(serde::Serialize, Clone)]
pub struct ResultatActivationUsb {
    pub reussi: bool,
}

/// Tente d'activer la licence à partir du contenu d'un fichier `licence.txt`
/// trouvé sur une clé USB (voir usb.rs). Évite au gérant de retaper à la
/// main une clé signée de plus de 100 caractères : la longueur protège
/// contre la fabrication de fausses clés (section forgery), la clé USB
/// évite qu'elle doive être saisie caractère par caractère.
pub fn tenter_activation_depuis_usb(app: &tauri::AppHandle, contenu: &str) {
    use tauri::{Emitter, Manager};

    let Some(state) = app.try_state::<DbState>() else {
        return;
    };
    let Ok(conn) = state.0.lock() else {
        return;
    };

    let id = machine_id();
    let reussi = verifier_cle(&id, contenu).is_some();
    if reussi {
        let cle_normalisee: String = contenu.chars().filter(|c| !c.is_whitespace()).collect();
        let _ = db::set_setting(&conn, "cle_licence", &cle_normalisee.to_uppercase());
    }
    drop(conn);

    let _ = app.emit("licence-usb", ResultatActivationUsb { reussi });
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "MACHINE-DE-TEST-1234";
    // Clé produite par le tableau de bord (WebCrypto Ed25519), pour vérifier
    // que les deux côtés parlent bien la même langue : si ce test casse, plus
    // aucun gérant ne peut activer sa licence.
    const CLE_DU_TABLEAU_DE_BORD: &str = "Y552T7QK7S0BE3ACTMW751Z9GP0Q9VVWHPX9AFSRNA95E9SZRM7V0MG78S89RV0ETKHKJKRHP8FMX20FYYYY22KPRX9W1JDXGKG2T2R-20261120";

    #[test]
    fn accepte_une_cle_signee_par_le_tableau_de_bord() {
        let expiration = verifier_cle(ID, CLE_DU_TABLEAU_DE_BORD);
        assert_eq!(
            expiration,
            Some(NaiveDate::from_ymd_opt(2026, 11, 20).unwrap())
        );
    }

    #[test]
    fn accepte_une_cle_collee_avec_espaces_et_retours_a_la_ligne() {
        let collee = format!("  {}\n", CLE_DU_TABLEAU_DE_BORD);
        assert!(verifier_cle(ID, &collee).is_some());
    }

    #[test]
    fn refuse_la_cle_d_une_autre_machine() {
        assert!(verifier_cle("UNE-AUTRE-MACHINE", CLE_DU_TABLEAU_DE_BORD).is_none());
    }

    #[test]
    fn refuse_une_date_d_expiration_repoussee() {
        // Reculer la date sans refaire signer ne doit rien donner.
        let trafiquee = CLE_DU_TABLEAU_DE_BORD.replace("-20261120", "-20991231");
        assert!(verifier_cle(ID, &trafiquee).is_none());
    }

    #[test]
    fn refuse_une_signature_modifiee() {
        let mut trafiquee = CLE_DU_TABLEAU_DE_BORD.to_string();
        trafiquee.replace_range(0..1, "Z");
        assert!(verifier_cle(ID, &trafiquee).is_none());
    }

    #[test]
    fn refuse_n_importe_quoi() {
        assert!(verifier_cle(ID, "").is_none());
        assert!(verifier_cle(ID, "bonjour").is_none());
        assert!(verifier_cle(ID, "-20261120").is_none());
    }

    #[test]
    fn refuse_une_cle_de_l_ancien_systeme() {
        // L'ancien secret symétrique était livré dans chaque exe : n'importe
        // qui pouvait l'en extraire. Plus aucune clé de ce type n'est admise.
        assert!(!signature_valide(ID, "20261001", "ABCDEFGHJKMNPQRS"));
    }

#[test]
fn cle_generee_par_le_worker_reel_est_acceptee() {
    // Clé produite par le Worker Cloudflare tournant en local (workerd),
    // via le vrai parcours : connexion, création de boutique, génération.
    let cle = "YXZJGFCYSJ4S6HAQCYEKSBF69R1JVCKH0QPGFVJ1FZ34W4AS2A85XEHZTW8AK0ZEZWN72QJ04HJ1AN8KB4MXF2JR2RFGP0475GDK838-20261015";
    assert!(super::verifier_cle("MACHINE-DE-TEST-1234", cle).is_some());
    assert!(super::verifier_cle("AUTRE-MACHINE", cle).is_none());
}
}
