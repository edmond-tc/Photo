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

/// Identifiant historique de la machine : le `MachineGuid` que Windows
/// fabrique **à l'installation du système**. Toutes les licences déjà
/// livrées sont signées pour cette valeur — elle ne doit donc jamais
/// changer, sous peine de bloquer d'un coup toutes les boutiques déjà
/// abonnées. Son défaut : réinstaller Windows en fabrique un nouveau, et
/// la machine redevient inconnue.
pub fn machine_id() -> String {
    machine_uid::get().unwrap_or_else(|_| "MACHINE-INCONNUE".to_string())
}

// ───────────────────── Empreinte matérielle de la machine ─────────────────────
// Contrairement au MachineGuid ci-dessus, elle est calculée à partir de ce
// que le PC dit de LUI-MÊME (fabricant, modèle, numéros de série de la carte
// mère et du BIOS, écrits par le constructeur). Reformater le disque ou
// réinstaller Windows ne la change pas.
//
// Ce que ça apporte concrètement : la licence payée d'un gérant continue de
// fonctionner après une réinstallation de Windows, au lieu de devoir lui en
// refabriquer une. Ce que ça n'apporte PAS : empêcher quelqu'un de relancer
// un essai gratuit après un formatage complet — plus rien ne survit sur le
// disque à ce moment-là, et l'application n'a aucun accès à internet pour
// aller vérifier ailleurs. Cette limite est assumée, pas contournée.

/// Valeurs que les constructeurs laissent en place faute de les renseigner.
/// Les retenir reviendrait à donner la même empreinte à des milliers de PC
/// différents — et donc à accepter la licence d'un autre.
const VALEURS_BIDON: [&str; 12] = [
    "to be filled by o.e.m.",
    "default string",
    "system serial number",
    "system manufacturer",
    "system product name",
    "chassis serial number",
    "not specified",
    "not applicable",
    "none",
    "n/a",
    "invalid",
    "0",
];

/// Nombre minimal de valeurs matérielles exploitables. En dessous, on
/// renonce : une empreinte fabriquée à partir d'une seule information
/// générique (« le fabricant est HP ») serait commune à trop de machines.
const VALEURS_MATERIELLES_MINIMUM: usize = 2;

fn valeur_materielle_utilisable(valeur: &str) -> Option<String> {
    let propre = valeur.trim();
    if propre.is_empty() {
        return None;
    }
    let comparaison = propre.to_lowercase();
    if VALEURS_BIDON.contains(&comparaison.as_str()) {
        return None;
    }
    // Suites de zéros, de « F » ou de tirets : un numéro de série vide
    // déguisé, vu sur beaucoup de machines d'entrée de gamme.
    if comparaison
        .chars()
        .all(|c| c == '0' || c == 'f' || c == '-' || c == ' ')
    {
        return None;
    }
    Some(propre.to_string())
}

/// Assemble les informations matérielles en un identifiant court, stable et
/// lisible à voix haute au téléphone (c'est ainsi que le gérant le
/// communique). Renvoie `None` si la machine n'en dit pas assez sur elle.
pub fn composer_empreinte(valeurs: &[String]) -> Option<String> {
    use sha2::{Digest, Sha256};

    let utiles: Vec<String> = valeurs
        .iter()
        .filter_map(|v| valeur_materielle_utilisable(v))
        .collect();
    if utiles.len() < VALEURS_MATERIELLES_MINIMUM {
        return None;
    }

    let empreinte = Sha256::digest(utiles.join("|").as_bytes());
    let hexa: String = empreinte
        .iter()
        .take(10)
        .map(|octet| format!("{octet:02X}"))
        .collect();
    // Groupes de 4 : « A1B2-C3D4-… » se dicte sans se perdre, contrairement
    // à une suite de 20 caractères d'affilée.
    let groupes: Vec<String> = hexa
        .as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).to_string())
        .collect();
    Some(format!("MAT-{}", groupes.join("-")))
}

#[cfg(windows)]
fn valeurs_materielles() -> Vec<String> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let mut valeurs = Vec::new();

    // Écrit par le constructeur dans le BIOS, recopié par Windows au
    // démarrage : survit à une réinstallation du système.
    if let Ok(bios) = hklm.open_subkey(r"HARDWARE\DESCRIPTION\System\BIOS") {
        for nom in [
            "SystemManufacturer",
            "SystemProductName",
            "SystemSerialNumber",
            "BaseBoardManufacturer",
            "BaseBoardProduct",
            "BaseBoardSerialNumber",
        ] {
            if let Ok(v) = bios.get_value::<String, _>(nom) {
                valeurs.push(v);
            }
        }
    }

    // Identifiant matériel calculé par Windows lui-même à partir des mêmes
    // informations — utile quand les valeurs ci-dessus sont vides.
    if let Ok(infos) = hklm.open_subkey(r"SYSTEM\CurrentControlSet\Control\SystemInformation") {
        if let Ok(v) = infos.get_value::<String, _>("ComputerHardwareId") {
            valeurs.push(v);
        }
    }

    valeurs
}

#[cfg(not(windows))]
fn valeurs_materielles() -> Vec<String> {
    Vec::new()
}

/// Empreinte matérielle de CE PC, ou `None` si elle n'est pas exploitable.
pub fn empreinte_materielle() -> Option<String> {
    composer_empreinte(&valeurs_materielles())
}

/// L'identifiant à montrer au gérant — celui qu'il dictera au téléphone
/// pour obtenir sa clé.
///
/// Règle volontairement prudente : une machine qui tourne déjà avec une
/// licence liée à l'ancien identifiant continue d'afficher CET
/// identifiant-là. Sinon, le gérant lirait à l'écran un numéro différent de
/// celui auquel sa licence est attachée, et le porteur du projet créerait
/// une deuxième fiche pour une boutique qu'il a déjà. Les machines neuves,
/// elles, prennent directement l'empreinte matérielle et n'auront plus
/// jamais besoin d'une nouvelle clé après une réinstallation de Windows.
pub fn identifiant_affiche(
    id_herite: &str,
    empreinte: Option<&str>,
    licence_liee_a_l_id_herite: bool,
) -> String {
    match empreinte {
        Some(e) if !licence_liee_a_l_id_herite => e.to_string(),
        _ => id_herite.to_string(),
    }
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

/// Décode une signature encodée (Crockford base32) et la vérifie contre un
/// message et une clé publique donnés. Partagé entre les clés de licence et
/// les codes d'installation, qui ne diffèrent que par le message signé.
fn decoder_et_verifier_signature(
    message: &[u8],
    signature: &str,
    cle_publique: &ed25519_dalek::VerifyingKey,
) -> bool {
    use ed25519_dalek::Signature;

    let Some(octets) = base32::decode(Alphabet::Crockford, signature) else {
        return false;
    };
    let Ok(octets) = <[u8; 64]>::try_from(octets.as_slice()) else {
        return false;
    };
    cle_publique
        .verify_strict(message, &Signature::from_bytes(&octets))
        .is_ok()
}

fn signature_valide(machine_id: &str, expiration_compacte: &str, signature: &str) -> bool {
    use ed25519_dalek::VerifyingKey;

    let Ok(cle) = VerifyingKey::from_bytes(&CLE_PUBLIQUE) else {
        return false;
    };
    decoder_et_verifier_signature(&message_a_signer(machine_id, expiration_compacte), signature, &cle)
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

/// Accepte une clé signée pour l'un OU l'autre des identifiants de cette
/// machine. Indispensable pendant la transition : les licences déjà livrées
/// sont liées au `MachineGuid`, les nouvelles le seront à l'empreinte
/// matérielle. Aucune des deux ne doit cesser de fonctionner.
///
/// Ce n'est pas un affaiblissement : les deux identifiants désignent la même
/// machine, et une clé signée pour une AUTRE machine reste refusée dans les
/// deux cas — c'est la signature qui protège, pas le choix de l'identifiant.
fn verifier_cle_sur_cette_machine(cle: &str) -> Option<NaiveDate> {
    verifier_cle(&machine_id(), cle)
        .or_else(|| empreinte_materielle().and_then(|e| verifier_cle(&e, cle)))
}

// ───────────────────── Code d'installation ─────────────────────
// Distinct de la licence : une licence dit "cette machine a payé jusqu'à
// telle date", un code d'installation dit juste "le porteur du projet a été
// prévenu AVANT que cette machine précise soit installée". Indispensable
// quand quelqu'un d'autre que le porteur du projet (un maintenancier sur le
// terrain, par exemple) installe le logiciel : sans ce verrou, il pourrait
// démarcher et installer des boutiques entières sans jamais en informer
// personne, l'essai gratuit de 30 jours démarrant tout seul.
//
// Signé avec la même paire de clés Ed25519 que les licences (le porteur du
// projet est le seul à pouvoir en fabriquer un, depuis son tableau de bord),
// mais avec un message et un préfixe différents ("INSTALL|" / "INST-") pour
// qu'un code d'installation ne puisse jamais être confondu avec — ni recyclé
// comme — une clé de licence, ou l'inverse.

fn message_a_signer_installation(id: &str) -> Vec<u8> {
    format!("INSTALL|{id}").into_bytes()
}

fn code_installation_valide_pour(id: &str, code: &str) -> bool {
    use ed25519_dalek::VerifyingKey;

    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let code = code.to_uppercase();
    let Some(signature) = code.strip_prefix("INST-") else {
        return false;
    };
    let Ok(cle) = VerifyingKey::from_bytes(&CLE_PUBLIQUE) else {
        return false;
    };
    decoder_et_verifier_signature(&message_a_signer_installation(id), signature, &cle)
}

/// Accepte un code signé pour l'un OU l'autre des identifiants de cette
/// machine — même raison que `verifier_cle_sur_cette_machine` : l'identifiant
/// affiché au tout premier lancement peut être le `MachineGuid` hérité ou
/// l'empreinte matérielle, selon ce que cette machine sait dire d'elle-même.
fn code_installation_valide_sur_cette_machine(code: &str) -> bool {
    code_installation_valide_pour(&machine_id(), code)
        || empreinte_materielle().is_some_and(|e| code_installation_valide_pour(&e, code))
}

/// A-t-on déjà validé un code d'installation sur cette machine ?
///
/// On revérifie la SIGNATURE du code stocké contre l'identifiant de la
/// machine ACTUELLE à chaque appel — on ne se contente pas d'un simple
/// drapeau "1" enregistré une fois. Sans ça, copier le dossier de données de
/// l'application (le fichier SQLite) d'un PC déjà validé vers un tout autre
/// PC aurait suffi à déverrouiller ce second PC sans jamais obtenir de code
/// pour lui — exactement le contournement que ce verrou doit empêcher.
/// Même principe que la licence (`cle_licence`, jamais un simple booléen).
///
/// Le registre Windows sert de secours si la base SQLite a été supprimée
/// (désinstallation, réinstallation après un souci antivirus) : sans lui, un
/// gérant honnête qui réinstalle devrait rappeler inutilement pour un code
/// qu'il possède déjà.
#[tauri::command]
pub fn code_installation_deja_valide(state: State<DbState>) -> Result<bool, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let code = db::get_setting(&conn, "code_installation").or_else(|| registre::lire("CodeInstallation"));
    Ok(code.is_some_and(|code| code_installation_valide_sur_cette_machine(&code)))
}

#[tauri::command]
pub fn valider_code_installation(state: State<DbState>, code: String) -> Result<bool, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    if !code_installation_valide_sur_cette_machine(&code) {
        return Ok(false);
    }
    let code_normalise: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let code_normalise = code_normalise.to_uppercase();
    db::set_setting(&conn, "code_installation", &code_normalise).map_err(|e| e.to_string())?;
    registre::ecrire("CodeInstallation", &code_normalise);
    Ok(true)
}

#[derive(serde::Serialize)]
pub struct StatutLicence {
    pub statut: String, // 'essai' | 'actif' | 'expire' | 'invalide'
    pub jours_restants: i64,
    pub machine_id: String,
    pub date_expiration: Option<String>,
}

/// Miroir dans le registre Windows de certaines valeurs par ailleurs
/// stockées dans la base SQLite de l'app — laquelle est facile à supprimer
/// (désinstallation, réinstallation après un souci antivirus...). Ce second
/// emplacement, moins évident, relève le niveau sans prétendre à une
/// protection absolue (mécanisme "léger" assumé, cf. section 7). Un seul
/// module paramétré par nom de valeur : `EssaiDebut` (date de début
/// d'essai) et `CodeInstallation` (code d'installation déjà validé) s'y
/// stockent côte à côte, sous la même clé.
#[cfg(windows)]
mod registre {
    use winreg::enums::*;
    use winreg::RegKey;

    const CHEMIN: &str = r"Software\AtinzPhotocopieBenin";

    pub fn lire(valeur: &str) -> Option<String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let cle = hkcu.open_subkey(CHEMIN).ok()?;
        cle.get_value(valeur).ok()
    }

    pub fn ecrire(valeur: &str, contenu: &str) {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok((cle, _)) = hkcu.create_subkey(CHEMIN) {
            let _ = cle.set_value(valeur, &contenu);
        }
    }
}

#[cfg(not(windows))]
mod registre {
    pub fn lire(_valeur: &str) -> Option<String> {
        None
    }
    pub fn ecrire(_valeur: &str, _contenu: &str) {}
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

/// Troisième trace du début d'essai, dans un dossier partagé de Windows
/// (`ProgramData`) plutôt que dans les données de l'application.
///
/// Les deux premières (base SQLite, registre de l'utilisateur) partent
/// ensemble si quelqu'un désinstalle l'application et efface son dossier de
/// données pour repartir sur 30 jours gratuits. Celle-ci reste. Elle ne
/// survit pas à un formatage complet — rien ne le peut sur une machine sans
/// internet — mais elle ferme la porte la plus facile.
mod marqueur_partage {
    fn chemin() -> Option<std::path::PathBuf> {
        let base = std::env::var_os("ProgramData")
            .or_else(|| std::env::var_os("ALLUSERSPROFILE"))
            .or_else(|| std::env::var_os("XDG_DATA_HOME"))?;
        Some(
            std::path::PathBuf::from(base)
                .join("AtinzPhotocopie")
                .join("inst.dat"),
        )
    }

    pub fn lire() -> Option<String> {
        let contenu = std::fs::read_to_string(chemin()?).ok()?;
        let contenu = contenu.trim();
        if contenu.is_empty() {
            None
        } else {
            Some(contenu.to_string())
        }
    }

    /// Échoue en silence si le dossier n'est pas accessible en écriture :
    /// une trace manquante ne doit jamais empêcher un gérant honnête de
    /// travailler.
    pub fn ecrire(valeur: &str) {
        let Some(chemin) = chemin() else { return };
        if let Some(dossier) = chemin.parent() {
            let _ = std::fs::create_dir_all(dossier);
        }
        let _ = std::fs::write(chemin, valeur);
    }
}

/// Retient la plus ANCIENNE date connue parmi toutes les traces : c'est
/// celle qui fait foi. Effacer une seule trace ne rajeunit donc pas
/// l'essai, il faut les effacer toutes — et la plus ancienne retrouvée
/// écrase aussitôt les autres.
pub fn debut_essai_le_plus_ancien(traces: &[Option<String>]) -> Option<String> {
    traces
        .iter()
        .flatten()
        .filter_map(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .min()
        .map(|d| d.to_rfc3339())
}

pub fn assurer_debut_essai(conn: &rusqlite::Connection) {
    let traces = [
        db::get_setting(conn, "essai_debut"),
        registre::lire("EssaiDebut"),
        marqueur_partage::lire(),
    ];

    let plus_ancienne =
        debut_essai_le_plus_ancien(&traces).unwrap_or_else(|| Local::now().to_rfc3339());

    let _ = db::set_setting(conn, "essai_debut", &plus_ancienne);
    registre::ecrire("EssaiDebut", &plus_ancienne);
    marqueur_partage::ecrire(&plus_ancienne);
}

#[tauri::command]
pub fn get_license_status(state: State<DbState>) -> Result<StatutLicence, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let id_herite = machine_id();
    let empreinte = empreinte_materielle();
    let maintenant = date_de_reference(&conn);

    let cle_enregistree = db::get_setting(&conn, "cle_licence");
    // Une licence déjà liée à l'ancien identifiant fige l'affichage sur
    // celui-ci : le gérant ne doit jamais lire à l'écran un numéro
    // différent de celui auquel sa licence est attachée.
    let licence_liee_a_l_id_herite = cle_enregistree
        .as_deref()
        .is_some_and(|cle| verifier_cle(&id_herite, cle).is_some());
    let id = identifiant_affiche(&id_herite, empreinte.as_deref(), licence_liee_a_l_id_herite);

    if let Some(cle) = cle_enregistree {
        return Ok(match verifier_cle_sur_cette_machine(&cle) {
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
    if verifier_cle_sur_cette_machine(&cle).is_none() {
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

    let reussi = verifier_cle_sur_cette_machine(contenu).is_some();
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

    // ───────────── Empreinte matérielle (survit au formatage) ─────────────

    #[test]
    fn compose_une_empreinte_stable_et_lisible() {
        let valeurs = vec![
            "LENOVO".to_string(),
            "20HRS0PY00".to_string(),
            "PF0ZABCD".to_string(),
        ];
        let a = composer_empreinte(&valeurs).expect("empreinte attendue");
        let b = composer_empreinte(&valeurs).expect("empreinte attendue");
        assert_eq!(a, b, "la même machine doit toujours donner la même empreinte");
        assert!(a.starts_with("MAT-"));
        // Dictable au téléphone : des groupes courts, pas une longue suite.
        assert!(a.len() <= 30, "identifiant trop long à dicter : {a}");
    }

    #[test]
    fn deux_machines_differentes_ont_des_empreintes_differentes() {
        let a = composer_empreinte(&["LENOVO".to_string(), "PF0ZABCD".to_string()]);
        let b = composer_empreinte(&["LENOVO".to_string(), "PF0ZABCE".to_string()]);
        assert!(a.is_some() && b.is_some());
        assert_ne!(a, b);
    }

    #[test]
    fn ignore_les_valeurs_bidon_des_constructeurs() {
        // Ces valeurs se retrouvent à l'identique sur des milliers de PC :
        // les garder reviendrait à accepter la licence d'une autre machine.
        assert_eq!(
            composer_empreinte(&[
                "To Be Filled By O.E.M.".to_string(),
                "Default string".to_string(),
                "System Serial Number".to_string(),
            ]),
            None
        );
        assert_eq!(
            composer_empreinte(&["00000000".to_string(), "FFFFFFFF".to_string(), "---".to_string()]),
            None
        );
    }

    #[test]
    fn renonce_quand_la_machine_en_dit_trop_peu() {
        // Une seule information générique ("le fabricant est HP") serait
        // commune à trop de machines : mieux vaut pas d'empreinte du tout.
        assert_eq!(composer_empreinte(&["HP".to_string()]), None);
        assert_eq!(composer_empreinte(&[]), None);
    }

    #[test]
    fn une_valeur_bidon_parmi_de_bonnes_ne_gene_pas() {
        let empreinte = composer_empreinte(&[
            "Default string".to_string(),
            "LENOVO".to_string(),
            "PF0ZABCD".to_string(),
        ]);
        assert!(empreinte.is_some());
    }

    // ───────────── Identifiant affiché au gérant ─────────────

    #[test]
    fn une_machine_deja_sous_licence_garde_son_ancien_identifiant() {
        // Sinon le gérant lirait à l'écran un numéro différent de celui
        // auquel sa licence payée est attachée.
        assert_eq!(
            identifiant_affiche("ANCIEN-ID", Some("MAT-1234-5678"), true),
            "ANCIEN-ID"
        );
    }

    #[test]
    fn une_machine_neuve_prend_l_empreinte_materielle() {
        assert_eq!(
            identifiant_affiche("ANCIEN-ID", Some("MAT-1234-5678"), false),
            "MAT-1234-5678"
        );
    }

    #[test]
    fn sans_empreinte_exploitable_on_garde_l_ancien_identifiant() {
        assert_eq!(identifiant_affiche("ANCIEN-ID", None, false), "ANCIEN-ID");
    }

    // ───────────── Traces du début d'essai ─────────────

    #[test]
    fn retient_la_date_la_plus_ancienne_parmi_les_traces() {
        // Effacer une seule trace ne doit pas rajeunir l'essai.
        let traces = [
            Some("2026-09-01T10:00:00+01:00".to_string()),
            None,
            Some("2026-06-15T08:00:00+01:00".to_string()),
        ];
        assert_eq!(
            debut_essai_le_plus_ancien(&traces).as_deref(),
            Some("2026-06-15T08:00:00+01:00")
        );
    }

    #[test]
    fn aucune_trace_lisible_ne_plante_pas() {
        assert_eq!(debut_essai_le_plus_ancien(&[]), None);
        assert_eq!(debut_essai_le_plus_ancien(&[None, Some("n'importe quoi".to_string())]), None);
    }

#[test]
fn cle_generee_par_le_worker_reel_est_acceptee() {
    // Clé produite par le Worker Cloudflare tournant en local (workerd),
    // via le vrai parcours : connexion, création de boutique, génération.
    let cle = "YXZJGFCYSJ4S6HAQCYEKSBF69R1JVCKH0QPGFVJ1FZ34W4AS2A85XEHZTW8AK0ZEZWN72QJ04HJ1AN8KB4MXF2JR2RFGP0475GDK838-20261015";
    assert!(super::verifier_cle("MACHINE-DE-TEST-1234", cle).is_some());
    assert!(super::verifier_cle("AUTRE-MACHINE", cle).is_none());
}

// ───────────── Code d'installation ─────────────
// Utilise une paire de clés jetable générée sur place (pas le vrai secret du
// Worker, inconnu ici) : on vérifie la mécanique de `decoder_et_verifier_signature`
// et du préfixe "INST-", indépendamment de la vraie clé publique embarquée.

fn fabriquer_code_de_test(cle_privee: &ed25519_dalek::SigningKey, id: &str) -> String {
    use ed25519_dalek::Signer;
    let signature = cle_privee.sign(&message_a_signer_installation(id));
    format!("INST-{}", base32::encode(Alphabet::Crockford, &signature.to_bytes()))
}

#[test]
fn accepte_un_code_signe_pour_cette_machine() {
    use ed25519_dalek::SigningKey;
    let cle_privee = SigningKey::generate(&mut rand::rngs::OsRng);
    let cle_publique = cle_privee.verifying_key();
    let code = fabriquer_code_de_test(&cle_privee, "MACHINE-1");
    let message = message_a_signer_installation("MACHINE-1");
    let signature = code.strip_prefix("INST-").unwrap();
    assert!(super::decoder_et_verifier_signature(&message, signature, &cle_publique));
}

#[test]
fn refuse_un_code_sans_le_prefixe_inst() {
    assert!(!code_installation_valide_pour("MACHINE-1", "ABCDEFGH"));
}

#[test]
fn refuse_un_code_signe_pour_une_autre_machine() {
    use ed25519_dalek::SigningKey;
    let cle_privee = SigningKey::generate(&mut rand::rngs::OsRng);
    let code = fabriquer_code_de_test(&cle_privee, "MACHINE-1");
    let message_autre_machine = message_a_signer_installation("MACHINE-2");
    let signature = code.strip_prefix("INST-").unwrap();
    assert!(!super::decoder_et_verifier_signature(
        &message_autre_machine,
        signature,
        &cle_privee.verifying_key()
    ));
}

#[test]
fn refuse_un_code_dont_la_signature_est_modifiee() {
    use ed25519_dalek::SigningKey;
    let cle_privee = SigningKey::generate(&mut rand::rngs::OsRng);
    let code = fabriquer_code_de_test(&cle_privee, "MACHINE-1");
    // Jamais le tout dernier caractère : en base32 (512 bits de signature
    // sur 103 caractères), il ne porte que 2 bits utiles sur 5 — certaines
    // paires de caractères n'y diffèrent que sur un bit de bourrage ignoré
    // au décodage, ce qui rendrait ce test bogué (pas seulement rare).
    let mut caracteres: Vec<char> = code.chars().collect();
    let position = caracteres.len() / 2;
    caracteres[position] = if caracteres[position] == 'A' { 'B' } else { 'A' };
    let code: String = caracteres.into_iter().collect();
    let message = message_a_signer_installation("MACHINE-1");
    let signature = code.strip_prefix("INST-").unwrap();
    assert!(!super::decoder_et_verifier_signature(
        &message,
        signature,
        &cle_privee.verifying_key()
    ));
}

#[test]
fn accepte_un_code_colle_avec_espaces_et_en_minuscules() {
    use ed25519_dalek::SigningKey;
    let cle_privee = SigningKey::generate(&mut rand::rngs::OsRng);
    let cle_publique = cle_privee.verifying_key();
    let code = fabriquer_code_de_test(&cle_privee, "MACHINE-1");
    let collee = format!("  {}\n", code.to_lowercase());
    let normalisee: String = collee.chars().filter(|c| !c.is_whitespace()).collect();
    let normalisee = normalisee.to_uppercase();
    let signature = normalisee.strip_prefix("INST-").unwrap();
    assert!(super::decoder_et_verifier_signature(
        &message_a_signer_installation("MACHINE-1"),
        signature,
        &cle_publique
    ));
}
}
