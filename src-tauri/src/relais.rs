//! Réception par Internet : le PC va chercher, sur le relais du service, les
//! commandes que des clients ont envoyées avec leurs données mobiles (voir
//! `admin/site/relais.js`).
//!
//! Ni le téléphone ni le PC ne sont joignables directement sur Internet : le
//! relais est le point de rendez-vous. Le PC l'interroge quand il a une
//! connexion — c'est lui qui appelle, jamais l'inverse — et, pour chaque
//! commande, la remet à SON PROPRE serveur local, exactement comme le ferait
//! un téléphone de la boutique. Même vérification, même numéro de guichet,
//! même file d'attente : rien n'est dupliqué. Puis il prévient le relais,
//! qui efface les fichiers et donne au client son numéro.
//!
//! Sans connexion, rien ne se passe et rien ne s'affiche : la boutique
//! continue de recevoir par son Wi-Fi, le Bluetooth ou la clé USB.

use std::path::PathBuf;
use std::time::{Duration, Instant};
use tauri::Manager;

/// Adresse du relais et du site public.
pub const ADRESSE_RELAIS: &str = "https://photocopie-benin.atinzed2.workers.dev";

const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Code public de la boutique (dans son QR « Par Internet »), 6 caractères
/// sans lettres ambiguës.
pub fn nouveau_code() -> String {
    (0..6).map(|_| ALPHABET[rand::random::<usize>() % ALPHABET.len()] as char).collect()
}

/// Clé secrète du PC : seule elle permet de récupérer les documents.
pub fn nouveau_secret() -> String {
    (0..32).map(|_| format!("{:02x}", rand::random::<u8>())).collect()
}

/// Où joindre quoi. Séparé de l'application pour être essayé à part.
#[derive(Clone)]
pub struct Config {
    pub relais: String,
    pub local: String,
    pub code: String,
    pub secret: String,
    pub dossier: PathBuf,
}

#[derive(Debug, PartialEq)]
pub enum ErreurInscription {
    /// Le code est déjà pris par une autre boutique : en tirer un autre.
    CodePris,
    Autre(String),
}

#[derive(serde::Deserialize)]
struct Attente {
    commandes: Vec<CommandeEnLigne>,
}

#[derive(serde::Deserialize)]
struct CommandeEnLigne {
    id: String,
    champs: serde_json::Map<String, serde_json::Value>,
    fichiers: Vec<FichierEnLigne>,
}

#[derive(serde::Deserialize)]
struct FichierEnLigne {
    champ: String,
    nom: String,
}

/// Une commande arrivée par Internet et remise au guichet : de quoi en
/// suivre l'étape pour le client.
#[derive(Clone, Debug)]
pub struct Suivi {
    pub commande: String,
    pub jeton_local: String,
    pub depuis: Instant,
    pub dernier: Option<(String, bool)>,
}

/// Corps « multipart/form-data » écrit à la main : quelques champs texte et,
/// au plus, de petits fichiers (messages vocaux).
pub fn multipart(champs: &[(String, String)], fichiers: &[(String, String, Vec<u8>)]) -> (String, Vec<u8>) {
    let frontiere = format!("----relais{}", nouveau_secret());
    let mut corps = Vec::new();
    for (nom, valeur) in champs {
        corps.extend_from_slice(
            format!("--{frontiere}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n", nettoyer(nom)).as_bytes(),
        );
        corps.extend_from_slice(valeur.as_bytes());
        corps.extend_from_slice(b"\r\n");
    }
    for (nom, fichier, octets) in fichiers {
        corps.extend_from_slice(
            format!(
                "--{frontiere}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: application/octet-stream\r\n\r\n",
                nettoyer(nom),
                nettoyer(fichier)
            )
            .as_bytes(),
        );
        corps.extend_from_slice(octets);
        corps.extend_from_slice(b"\r\n");
    }
    corps.extend_from_slice(format!("--{frontiere}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={frontiere}"), corps)
}

/// Un nom venu du relais ne doit pas pouvoir casser l'en-tête.
fn nettoyer(texte: &str) -> String {
    texte.chars().filter(|c| !matches!(c, '"' | '\r' | '\n' | '\\')).take(200).collect()
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(600))
        .build()
        .unwrap_or_default()
}

/// Se fait connaître du relais (une première fois, puis de temps en temps,
/// pour le nom et les prix affichés au client).
/// Ce que le relais montre de la boutique aux clients (nom, prix, fiche) et
/// les numéros qu'elle refuse.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct Vitrine {
    pub nom: String,
    pub tarifs: serde_json::Value,
    pub fiche: serde_json::Value,
    pub bloques: Vec<String>,
}

pub async fn inscrire(client: &reqwest::Client, cfg: &Config, vitrine: &Vitrine) -> Result<(), ErreurInscription> {
    let r = client
        .post(format!("{}/api/relais/inscrire", cfg.relais))
        .json(&serde_json::json!({
            "code": cfg.code,
            "secret": cfg.secret,
            "nom": vitrine.nom,
            "tarifs": vitrine.tarifs,
            "fiche": vitrine.fiche,
            "bloques": vitrine.bloques,
        }))
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| ErreurInscription::Autre(e.to_string()))?;
    match r.status().as_u16() {
        200 => Ok(()),
        409 => Err(ErreurInscription::CodePris),
        s => Err(ErreurInscription::Autre(format!("réponse {s}"))),
    }
}

/// Commandes déjà remises au guichet : identifiant sur le relais → (numéro,
/// jeton de suivi). Si l'accusé de réception se perd en route (connexion qui
/// tombe), le relais la représente au tour suivant : sans ce registre, le
/// guichet l'aurait reçue deux fois.
pub type Remises = std::collections::HashMap<String, (String, String)>;

/// Récupère les commandes en attente, les remet au guichet, et prévient le
/// relais. Rend celles qui ont abouti, pour en suivre l'étape.
pub async fn traiter_attente(client: &reqwest::Client, cfg: &Config, remises: &mut Remises) -> Result<Vec<Suivi>, String> {
    let attente: Attente = client
        .get(format!("{}/api/relais/{}/attente", cfg.relais, cfg.code))
        .header("x-secret", &cfg.secret)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    let mut suivis = Vec::new();
    for commande in attente.commandes {
        let deja = remises.get(&commande.id).cloned();
        let resultat = match deja {
            Some(remise) => Ok(remise),
            None => remettre(client, cfg, &commande).await,
        };
        match resultat {
            Ok((numero, jeton)) => {
                remises.insert(commande.id.clone(), (numero.clone(), jeton.clone()));
                let _ = client
                    .post(format!("{}/api/relais/{}/accuse/{}", cfg.relais, cfg.code, commande.id))
                    .header("x-secret", &cfg.secret)
                    .json(&serde_json::json!({ "numero": numero }))
                    .timeout(Duration::from_secs(20))
                    .send()
                    .await;
                suivis.push(Suivi {
                    commande: commande.id.clone(),
                    jeton_local: jeton,
                    depuis: Instant::now(),
                    dernier: Some(("recu".to_string(), false)),
                });
            }
            // Laissée sur le relais : elle sera reprise au prochain tour.
            // Celles déjà remises restent suivies.
            Err(e) if suivis.is_empty() => return Err(e),
            Err(_) => break,
        }
    }
    Ok(suivis)
}

/// Télécharge les fichiers d'une commande puis la remet au serveur local du
/// guichet : morceaux pour les documents (comme un téléphone), commande
/// ensuite. Rend le numéro du guichet et le jeton de suivi.
async fn remettre(client: &reqwest::Client, cfg: &Config, commande: &CommandeEnLigne) -> Result<(String, String), String> {
    let dossier = cfg.dossier.join(&commande.id);
    tokio::fs::create_dir_all(&dossier).await.map_err(|e| e.to_string())?;
    let resultat = remettre_dans(client, cfg, commande, &dossier).await;
    let _ = tokio::fs::remove_dir_all(&dossier).await;
    resultat
}

async fn remettre_dans(
    client: &reqwest::Client,
    cfg: &Config,
    commande: &CommandeEnLigne,
    dossier: &std::path::Path,
) -> Result<(String, String), String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut champs: Vec<(String, String)> = commande
        .champs
        .iter()
        .filter(|(cle, _)| !cle.starts_with("reprise_") && !cle.starts_with("nomreprise_"))
        .map(|(cle, valeur)| (cle.clone(), valeur.as_str().unwrap_or_default().to_string()))
        .collect();
    let mut petits = Vec::new();

    for f in &commande.fichiers {
        // 1. Du relais vers le disque, sans tout garder en mémoire.
        let chemin = dossier.join(nettoyer(&f.champ));
        let mut reponse = client
            .get(format!("{}/api/relais/{}/fichier/{}/{}", cfg.relais, cfg.code, commande.id, f.champ))
            .header("x-secret", &cfg.secret)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;
        let mut sortie = tokio::fs::File::create(&chemin).await.map_err(|e| e.to_string())?;
        while let Some(bout) = reponse.chunk().await.map_err(|e| e.to_string())? {
            sortie.write_all(&bout).await.map_err(|e| e.to_string())?;
        }
        sortie.flush().await.map_err(|e| e.to_string())?;
        drop(sortie);

        let indice = f.champ.rsplit('_').next().unwrap_or("0").to_string();
        if f.champ.starts_with("vocal_") {
            let octets = tokio::fs::read(&chemin).await.map_err(|e| e.to_string())?;
            petits.push((f.champ.clone(), f.nom.clone(), octets));
            continue;
        }
        // 2. Vers le guichet, en morceaux d'1 Mo, comme un téléphone.
        let id = nouveau_secret()[..24].to_string();
        let mut fichier = tokio::fs::File::open(&chemin).await.map_err(|e| e.to_string())?;
        let mut debut: u64 = 0;
        let mut tampon = vec![0u8; 1024 * 1024];
        loop {
            let mut lu = 0;
            while lu < tampon.len() {
                let n = fichier.read(&mut tampon[lu..]).await.map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                lu += n;
            }
            if lu == 0 {
                break;
            }
            let r = client
                .post(format!("{}/morceau/{id}?debut={debut}", cfg.local))
                .body(tampon[..lu].to_vec())
                .timeout(Duration::from_secs(60))
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !r.status().is_success() {
                return Err(format!("guichet : morceau refusé ({})", r.status()));
            }
            debut += lu as u64;
        }
        if debut == 0 {
            // Document vide : envoyé tel quel, le guichet décidera.
            petits.push((f.champ.clone(), f.nom.clone(), Vec::new()));
            continue;
        }
        champs.push((format!("nomreprise_{indice}"), f.nom.clone()));
        champs.push((format!("reprise_{indice}"), id));
    }
    champs.retain(|(cle, _)| cle != "origine");
    champs.push(("origine".to_string(), "internet".to_string()));

    let (type_contenu, corps) = multipart(&champs, &petits);
    let reponse: serde_json::Value = client
        .post(format!("{}/envoyer", cfg.local))
        .header("content-type", type_contenu)
        .body(corps)
        .timeout(Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let numero = reponse["numero"].as_str().unwrap_or_default().to_string();
    let jeton = reponse["jetons"][0].as_str().unwrap_or_default().to_string();
    if numero.is_empty() {
        return Err("le guichet n'a pas donné de numéro".to_string());
    }
    Ok((numero, jeton))
}

/// Renvoie au relais l'étape de chaque commande venue d'Internet (imprimée,
/// prête, payée), pour que le client la voie sur son téléphone.
pub async fn synchroniser_etats(client: &reqwest::Client, cfg: &Config, suivis: &mut Vec<Suivi>) {
    suivis.retain(|s| s.depuis.elapsed() < Duration::from_secs(12 * 3600) && !s.jeton_local.is_empty());
    for s in suivis.iter_mut() {
        let Ok(r) = client
            .get(format!("{}/statut/{}", cfg.local, s.jeton_local))
            .timeout(Duration::from_secs(5))
            .send()
            .await
        else {
            continue;
        };
        let Ok(v) = r.json::<serde_json::Value>().await else { continue };
        let etape = v["etape"].as_str().unwrap_or("recu").to_string();
        let paye = v["paye"].as_bool().unwrap_or(false);
        let maintenant = Some((etape.clone(), paye));
        if maintenant == s.dernier {
            continue;
        }
        let envoye = client
            .post(format!("{}/api/relais/{}/etat/{}", cfg.relais, cfg.code, s.commande))
            .header("x-secret", &cfg.secret)
            .json(&serde_json::json!({ "etape": etape, "paye": paye, "message": v["message"] }))
            .timeout(Duration::from_secs(15))
            .send()
            .await;
        if envoye.is_ok_and(|r| r.status().is_success()) {
            s.dernier = maintenant;
        }
    }
    // Payée : le client n'a plus rien à suivre.
    suivis.retain(|s| !s.dernier.as_ref().is_some_and(|(_, paye)| *paye));
}

// ───────────────────────────── Dans l'application ─────────────────────────────

/// Ce que l'écran du QR montre : l'adresse « Par Internet » de la boutique,
/// seulement une fois que le relais la connaît (sinon le client tomberait
/// sur « Boutique introuvable »).
#[derive(serde::Serialize)]
pub struct InfoRelais {
    pub actif: bool,
    pub url: Option<String>,
    pub qr_data_uri: Option<String>,
}

#[tauri::command]
pub fn relais_info(state: tauri::State<'_, crate::db::DbState>) -> Result<InfoRelais, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let actif = crate::db::get_setting(&conn, "relais_actif").as_deref() != Some("non");
    let inscrit = crate::db::get_setting(&conn, "relais_inscrit").as_deref() == Some("oui");
    let code = crate::db::get_setting(&conn, "relais_code");
    match (actif && inscrit, code) {
        (true, Some(code)) => {
            let url = format!("{ADRESSE_RELAIS}/b/{code}");
            Ok(InfoRelais {
                actif: true,
                qr_data_uri: crate::qr::build_qr_data_uri(&url).ok(),
                url: Some(url),
            })
        }
        _ => Ok(InfoRelais { actif: false, url: None, qr_data_uri: None }),
    }
}

/// Identité de la boutique sur le relais : créée au premier lancement.
fn identite(app: &tauri::AppHandle) -> Option<(String, String)> {
    let state = app.state::<crate::db::DbState>();
    let conn = state.0.lock().ok()?;
    let code = match crate::db::get_setting(&conn, "relais_code") {
        Some(c) if !c.is_empty() => c,
        _ => {
            let c = nouveau_code();
            crate::db::set_setting(&conn, "relais_code", &c).ok()?;
            c
        }
    };
    let secret = match crate::db::get_setting(&conn, "relais_secret") {
        Some(s) if s.len() == 64 => s,
        _ => {
            let s = nouveau_secret();
            crate::db::set_setting(&conn, "relais_secret", &s).ok()?;
            s
        }
    };
    Some((code, secret))
}

fn reglage(app: &tauri::AppHandle, cle: &str) -> Option<String> {
    let state = app.state::<crate::db::DbState>();
    let conn = state.0.lock().ok()?;
    crate::db::get_setting(&conn, cle)
}

fn noter(app: &tauri::AppHandle, cle: &str, valeur: &str) {
    let state = app.state::<crate::db::DbState>();
    if let Ok(conn) = state.0.lock() {
        let _ = crate::db::set_setting(&conn, cle, valeur);
    };
}

/// La vitrine de la boutique, d'après ses réglages.
fn vitrine(app: &tauri::AppHandle) -> Vitrine {
    let r = |cle: &str| reglage(app, cle).unwrap_or_default();
    let pays = crate::pays::pays(reglage(app, "pays").as_deref());
    Vitrine {
        nom: r("boutique_nom"),
        tarifs: tarifs(app),
        fiche: serde_json::json!({
            "adresse": r("fiche_adresse"),
            "horaires": r("fiche_horaires"),
            "telephone": r("fiche_telephone"),
            "plan": r("fiche_plan"),
            "devise": pays.devise,
        }),
        bloques: numeros_bloques(&r("numeros_bloques")),
    }
}

/// Un numéro par ligne (ou séparés par des virgules), tel que le gérant
/// l'a copié de la commande.
pub fn numeros_bloques(texte: &str) -> Vec<String> {
    texte
        .split(['\n', ',', ';'])
        .map(|l| l.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect::<String>())
        .filter(|n| n.chars().filter(char::is_ascii_digit).count() >= 8)
        .take(1000)
        .collect()
}

fn tarifs(app: &tauri::AppHandle) -> serde_json::Value {
    let state = app.state::<crate::db::DbState>();
    let Ok(conn) = state.0.lock() else { return serde_json::json!({}) };
    let carte: serde_json::Map<String, serde_json::Value> = conn
        .prepare("SELECT service, prix_unitaire FROM tarifs")
        .and_then(|mut stmt| {
            stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
                .map(|l| l.filter_map(Result::ok).map(|(k, v)| (k, serde_json::Value::from(v))).collect())
        })
        .unwrap_or_default();
    serde_json::Value::Object(carte)
}

/// Le tour du relais, en tâche de fond, tant que l'application tourne.
pub fn demarrer(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let client = client();
        let mut suivis: Vec<Suivi> = Vec::new();
        let mut remises: Remises = reglage(&app, "relais_remises")
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let mut derniere_inscription: Option<Instant> = None;
        let mut vitrine_inscrite = Vitrine::default();
        let mut activite: Option<Instant> = None;
        let mut echecs: u32 = 0;
        // Laisse d'abord démarrer le serveur local.
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            // Un client vient d'envoyer : on regarde souvent. Sinon, moins :
            // chaque visite compte dans le quota gratuit du relais.
            let pause = if echecs > 0 {
                Duration::from_secs(60)
            } else if activite.is_some_and(|t| t.elapsed() < Duration::from_secs(600)) {
                Duration::from_secs(5)
            } else {
                Duration::from_secs(15)
            };
            if reglage(&app, "relais_actif").as_deref() == Some("non") {
                tokio::time::sleep(Duration::from_secs(60)).await;
                continue;
            }
            let Some((code, secret)) = identite(&app) else {
                tokio::time::sleep(Duration::from_secs(60)).await;
                continue;
            };
            let Ok(dossier) = app.path().app_data_dir().map(|d| d.join("relais-en-cours")) else {
                tokio::time::sleep(Duration::from_secs(60)).await;
                continue;
            };
            let cfg = Config {
                relais: ADRESSE_RELAIS.to_string(),
                local: format!("http://127.0.0.1:{}", crate::server::PORT),
                code,
                secret,
                dossier,
            };

            // Inscription : au début, dès que le nom, les prix, la fiche ou
            // les numéros bloqués changent, puis toutes les 6 h.
            let actuelle = vitrine(&app);
            if derniere_inscription.is_none_or(|t| t.elapsed() > Duration::from_secs(6 * 3600)) || actuelle != vitrine_inscrite {
                match inscrire(&client, &cfg, &actuelle).await {
                    Ok(()) => {
                        derniere_inscription = Some(Instant::now());
                        vitrine_inscrite = actuelle;
                        noter(&app, "relais_inscrit", "oui");
                        echecs = 0;
                    }
                    Err(ErreurInscription::CodePris) => {
                        noter(&app, "relais_code", &nouveau_code());
                        noter(&app, "relais_inscrit", "non");
                        continue;
                    }
                    // Pas de connexion : on réessaiera, sans rien afficher.
                    Err(ErreurInscription::Autre(_)) => {
                        echecs = echecs.saturating_add(1);
                        tokio::time::sleep(pause).await;
                        continue;
                    }
                }
            }

            let avant = remises.len();
            let tour = traiter_attente(&client, &cfg, &mut remises).await;
            if remises.len() != avant {
                // Les 200 dernières suffisent : le relais efface tout en 24 h.
                if remises.len() > 200 {
                    let mut cles: Vec<String> = remises.keys().cloned().collect();
                    cles.sort();
                    for c in cles.into_iter().take(remises.len() - 200) {
                        remises.remove(&c);
                    }
                }
                if let Ok(t) = serde_json::to_string(&remises) {
                    noter(&app, "relais_remises", &t);
                }
            }
            match tour {
                Ok(nouvelles) => {
                    echecs = 0;
                    if !nouvelles.is_empty() {
                        activite = Some(Instant::now());
                        suivis.extend(nouvelles);
                    }
                }
                Err(_) => echecs = echecs.saturating_add(1),
            }
            if !suivis.is_empty() {
                synchroniser_etats(&client, &cfg, &mut suivis).await;
            }
            tokio::time::sleep(pause).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_et_secret_ont_la_bonne_forme() {
        for _ in 0..200 {
            let c = nouveau_code();
            assert_eq!(c.len(), 6);
            assert!(c.bytes().all(|b| ALPHABET.contains(&b)), "{c}");
            assert!(!c.contains(['I', 'L', 'O', 'U']));
        }
        let s = nouveau_secret();
        assert_eq!(s.len(), 64);
        assert!(s.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn le_corps_multipart_est_lisible_par_le_guichet() {
        let (type_contenu, corps) = multipart(
            &[("nom".into(), "Awa".into()), ("copies_0".into(), "2".into())],
            &[("vocal_0".into(), "vocal.webm".into(), vec![1, 2, 3])],
        );
        let frontiere = type_contenu.split("boundary=").nth(1).unwrap();
        let texte = String::from_utf8_lossy(&corps);
        assert!(texte.starts_with(&format!("--{frontiere}\r\n")));
        assert!(texte.contains("name=\"nom\"\r\n\r\nAwa\r\n"));
        assert!(texte.contains("name=\"vocal_0\"; filename=\"vocal.webm\""));
        assert!(texte.ends_with(&format!("--{frontiere}--\r\n")));
    }

    #[test]
    fn numeros_bloques_copies_tels_quels() {
        assert_eq!(numeros_bloques("+229 01 97 12 34 56\n\n0197000001, 12\nAwa"), vec!["+2290197123456", "0197000001"]);
    }

    #[test]
    fn un_nom_ne_peut_pas_casser_l_en_tete() {
        assert_eq!(nettoyer("a\"b\r\nc\\d"), "abcd");
    }

    /// Essai réel, de bout en bout : relais (wrangler dev) + guichet (faux
    /// PC du banc d'essai). Lancé à la main :
    /// RELAIS=http://127.0.0.1:8787 GUICHET=http://127.0.0.1:4173 CODE=… SECRET=…
    /// cargo test --lib relais::tests::bout_en_bout -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn bout_en_bout() {
        let cfg = Config {
            relais: std::env::var("RELAIS").unwrap(),
            local: std::env::var("GUICHET").unwrap(),
            code: std::env::var("CODE").unwrap(),
            secret: std::env::var("SECRET").unwrap(),
            dossier: std::env::temp_dir().join("relais-essai"),
        };
        let c = client();
        let v = Vitrine { nom: "Boutique d'essai".into(), tarifs: serde_json::json!({"nb_a4": 25}), ..Default::default() };
        inscrire(&c, &cfg, &v).await.unwrap();
        let mut remises = Remises::new();
        let mut suivis = traiter_attente(&c, &cfg, &mut remises).await.unwrap();
        println!("commandes remises : {}", suivis.len());
        assert!(!suivis.is_empty(), "aucune commande en attente");
        synchroniser_etats(&c, &cfg, &mut suivis).await;
    }

    /// L'accusé de réception s'est perdu : le relais représente une commande
    /// déjà remise. Le guichet ne doit PAS la recevoir une seconde fois ; le
    /// relais reçoit simplement l'accusé, avec le numéro déjà donné.
    /// Lancé à la main (mêmes variables que `bout_en_bout`) :
    /// cargo test --lib relais::tests::pas_de_doublon -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn pas_de_doublon_si_l_accuse_s_est_perdu() {
        let cfg = Config {
            relais: std::env::var("RELAIS").unwrap(),
            local: std::env::var("GUICHET").unwrap(),
            code: std::env::var("CODE").unwrap(),
            secret: std::env::var("SECRET").unwrap(),
            dossier: std::env::temp_dir().join("relais-essai-doublon"),
        };
        let c = client();
        let attente: serde_json::Value = c
            .get(format!("{}/api/relais/{}/attente", cfg.relais, cfg.code))
            .header("x-secret", &cfg.secret)
            .send().await.unwrap().json().await.unwrap();
        let id = attente["commandes"][0]["id"].as_str().expect("une commande en attente").to_string();
        let mut remises = Remises::new();
        remises.insert(id.clone(), ("A-9".to_string(), "jeton-deja-remis".to_string()));
        let suivis = traiter_attente(&c, &cfg, &mut remises).await.unwrap();
        assert_eq!(suivis.len(), 1);
        assert_eq!(suivis[0].jeton_local, "jeton-deja-remis");
        println!("commande {id} : accusée sans nouvelle remise au guichet");
    }
}
