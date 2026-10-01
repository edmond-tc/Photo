//! Le canal Bluetooth entre le téléphone (application Envoyeur) et le PC :
//! une vraie conversation, comme Quick Share.
//!
//! Le téléphone lit l'adresse Bluetooth du PC dans sa balise (voir
//! `balise_ble.rs`) et s'y relie directement, sans recherche ni appairage.
//! Sur ce canal :
//! - « wifi » : le téléphone donne le nom ET le mot de passe du réseau qu'il
//!   vient de créer ; le PC le rejoint et répond « relié, voici mon
//!   adresse », ou « échec, voici pourquoi ». Si le Wi-Fi de la boutique
//!   tourne, le PC répond plutôt son nom et son mot de passe : le
//!   téléphone le rejoint tout seul.
//! - « http » : quand aucun Wi-Fi ne passe, la commande entière (infos,
//!   morceaux de fichiers, commande, suivi) passe par le Bluetooth. Plus
//!   lent, mais ça passe. Le PC relaie chaque requête à son propre serveur
//!   (127.0.0.1), pour une LISTE FERMÉE d'adresses seulement.
//!
//! Format : chaque message est une trame = longueur (4 octets, gros-boutien)
//! puis un en-tête JSON, un saut de ligne, et le corps brut éventuel.

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::time::{Duration, Instant};

/// Identifiant du service (le même dans `CanalBt.kt`).
pub const SERVICE_UUID: u128 = 0x6b1f0c2e_8a4d_4f3b_9c55_4b5051434f50;

/// Une trame ne dépasse jamais ça : un morceau de fichier fait 1 Mo.
pub const TAILLE_MAX_TRAME: usize = 8 * 1024 * 1024;

pub fn ecrire_trame(entete: &Value, corps: &[u8]) -> Vec<u8> {
    let mut contenu = serde_json::to_vec(entete).unwrap_or_default();
    contenu.push(b'\n');
    contenu.extend_from_slice(corps);
    let mut trame = (contenu.len() as u32).to_be_bytes().to_vec();
    trame.extend(contenu);
    trame
}

/// Contenu d'une trame (sans les 4 octets de longueur) : en-tête et corps.
pub fn lire_trame(contenu: &[u8]) -> Option<(Value, &[u8])> {
    let fin = contenu.iter().position(|&o| o == b'\n')?;
    let entete: Value = serde_json::from_slice(&contenu[..fin]).ok()?;
    Some((entete, &contenu[fin + 1..]))
}

/// Les seules adresses du serveur que le téléphone peut joindre par le
/// canal : celles de la page d'envoi, rien d'autre.
pub fn chemin_autorise(methode: &str, chemin: &str) -> bool {
    let (route, requete) = chemin.split_once('?').unwrap_or((chemin, ""));
    let jeton_ok = |t: &str| !t.is_empty() && t.len() <= 64 && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    match (methode, route) {
        ("GET", "/infos") => requete.is_empty(),
        ("POST", "/envoyer") => requete.is_empty(),
        ("GET", r) if r.starts_with("/statut/") => jeton_ok(&r[8..]) && requete.is_empty(),
        ("GET", r) if r.starts_with("/morceau/") => jeton_ok(&r[9..]) && requete.is_empty(),
        ("POST", r) if r.starts_with("/morceau/") => {
            jeton_ok(&r[9..])
                && requete.strip_prefix("debut=").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        }
        _ => false,
    }
}

/// Lit la réponse HTTP brute du serveur local : code et corps (longueur
/// annoncée, découpage « chunked », ou jusqu'à la fin).
pub fn lire_reponse_http(brut: &[u8]) -> Option<(u16, Vec<u8>)> {
    let fin_entetes = brut.windows(4).position(|w| w == b"\r\n\r\n")?;
    let entetes = std::str::from_utf8(&brut[..fin_entetes]).ok()?;
    let mut lignes = entetes.split("\r\n");
    let code: u16 = lignes.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let mut longueur = None;
    let mut decoupe = false;
    for l in lignes {
        let (nom, valeur) = l.split_once(':')?;
        let (nom, valeur) = (nom.trim().to_ascii_lowercase(), valeur.trim());
        if nom == "content-length" {
            longueur = valeur.parse::<usize>().ok();
        } else if nom == "transfer-encoding" && valeur.eq_ignore_ascii_case("chunked") {
            decoupe = true;
        }
    }
    let corps = &brut[fin_entetes + 4..];
    if decoupe {
        let mut sortie = Vec::new();
        let mut reste = corps;
        loop {
            let fin_ligne = reste.windows(2).position(|w| w == b"\r\n")?;
            let taille = usize::from_str_radix(std::str::from_utf8(&reste[..fin_ligne]).ok()?.split(';').next()?.trim(), 16).ok()?;
            reste = &reste[fin_ligne + 2..];
            if taille == 0 {
                break;
            }
            sortie.extend_from_slice(reste.get(..taille)?);
            reste = reste.get(taille + 2..)?;
        }
        return Some((code, sortie));
    }
    Some((code, match longueur {
        Some(n) => corps.get(..n)?.to_vec(),
        None => corps.to_vec(),
    }))
}

/// Relaie une requête au serveur de ce PC.
fn requete_locale(methode: &str, chemin: &str, type_contenu: &str, corps: &[u8]) -> std::io::Result<(u16, Vec<u8>)> {
    let mut flux = std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], crate::server::PORT)),
        Duration::from_secs(5),
    )?;
    flux.set_read_timeout(Some(Duration::from_secs(120)))?;
    let mut requete = format!(
        "{methode} {chemin} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Length: {}\r\n",
        corps.len()
    );
    if !type_contenu.is_empty() && !type_contenu.contains(['\r', '\n']) {
        requete.push_str(&format!("Content-Type: {type_contenu}\r\n"));
    }
    requete.push_str("\r\n");
    flux.write_all(requete.as_bytes())?;
    flux.write_all(corps)?;
    let mut brut = Vec::new();
    flux.read_to_end(&mut brut)?;
    lire_reponse_http(&brut).ok_or_else(|| std::io::Error::other("réponse illisible"))
}

/// Ce que le PC répond à un message du téléphone. `appelant` : une adresse
/// propre à ce téléphone, pour la limite d'envois du serveur.
pub fn traiter(app: &tauri::AppHandle, entete: &Value, corps: &[u8], appelant: std::net::IpAddr) -> (Value, Vec<u8>) {
    let noter = crate::reception_directe::noter;
    match entete["t"].as_str() {
        Some("bonjour") => (json!({ "t": "bonjour", "v": 1 }), Vec::new()),
        Some("wifi") => {
            let (Some(ssid), Some(mdp)) = (entete["ssid"].as_str(), entete["mdp"].as_str()) else {
                return (json!({ "t": "wifi", "etat": "echec", "raison": "demande incomplète" }), Vec::new());
            };
            if ssid.is_empty() || ssid.len() > 32 || mdp.len() < 8 || mdp.len() > 63 {
                return (json!({ "t": "wifi", "etat": "echec", "raison": "nom ou mot de passe invalide" }), Vec::new());
            }
            // Le Wi-Fi de la boutique tourne : la carte est prise. On donne
            // au téléphone de quoi le rejoindre tout seul.
            if crate::reception_directe::wifi_boutique_prioritaire(app) {
                if let Some((s, m)) = wifi_boutique(app) {
                    noter(format!("🔵 Canal Bluetooth : téléphone envoyé sur le Wi-Fi de la boutique « {s} »."));
                    return (json!({ "t": "wifi", "etat": "boutique", "ssid": s, "mdp": m }), Vec::new());
                }
                return (json!({ "t": "wifi", "etat": "echec", "raison": "carte Wi-Fi occupée" }), Vec::new());
            }
            noter(format!("🔵 Canal Bluetooth : le téléphone donne son réseau « {ssid} »."));
            crate::appel_ble::demande_directe(ssid, mdp);
            let debut = Instant::now();
            while debut.elapsed() < Duration::from_secs(50) {
                if let Some(ip) = crate::reception_directe::lien_avec(ssid) {
                    return (json!({ "t": "wifi", "etat": "ok", "ip": ip.to_string() }), Vec::new());
                }
                std::thread::sleep(Duration::from_millis(300));
            }
            (json!({ "t": "wifi", "etat": "echec", "raison": "le PC n'a pas pu rejoindre le réseau du téléphone" }), Vec::new())
        }
        Some("http") => {
            let id = entete["id"].clone();
            let methode = entete["m"].as_str().unwrap_or("");
            let chemin = entete["p"].as_str().unwrap_or("");
            if !chemin_autorise(methode, chemin) {
                return (json!({ "t": "http", "id": id, "s": 403 }), b"adresse interdite".to_vec());
            }
            if methode == "POST" && chemin == "/envoyer" && !crate::server::envoi_permis(appelant) {
                return (
                    json!({ "t": "http", "id": id, "s": 429 }),
                    "Trop d'envois depuis ce téléphone en peu de temps. Attendez quelques minutes, ou voyez le guichet.".as_bytes().to_vec(),
                );
            }
            crate::reception_directe::signaler_activite();
            match requete_locale(methode, chemin, entete["ct"].as_str().unwrap_or(""), corps) {
                Ok((code, reponse)) => (json!({ "t": "http", "id": id, "s": code }), reponse),
                Err(e) => (json!({ "t": "http", "id": id, "s": 502 }), e.to_string().into_bytes()),
            }
        }
        _ => (json!({ "t": "erreur", "raison": "message inconnu" }), Vec::new()),
    }
}

fn wifi_boutique(app: &tauri::AppHandle) -> Option<(String, String)> {
    use tauri::Manager;
    let state = app.state::<crate::db::DbState>();
    let conn = state.0.lock().ok()?;
    let ssid = crate::db::get_setting(&conn, "wifi_ssid").filter(|s| !s.trim().is_empty())?;
    let mdp = crate::db::get_setting(&conn, "wifi_mot_de_passe").unwrap_or_default();
    Some((ssid, mdp))
}

/// Une adresse stable par téléphone (tirée de son adresse Bluetooth), pour
/// que la limite d'envois du serveur s'applique aussi par ce canal.
pub fn adresse_du_telephone(bluetooth: &str) -> std::net::IpAddr {
    use sha2::{Digest, Sha256};
    let e = Sha256::digest(bluetooth.as_bytes());
    std::net::IpAddr::V4(std::net::Ipv4Addr::new(10, e[0], e[1], e[2]))
}

#[cfg(windows)]
pub fn demarrer(app: tauri::AppHandle) {
    use std::sync::Mutex;
    use windows::core::GUID;
    use windows::Devices::Bluetooth::Rfcomm::{RfcommServiceId, RfcommServiceProvider};
    use windows::Foundation::TypedEventHandler;
    use windows::Networking::Sockets::{
        SocketProtectionLevel, StreamSocket, StreamSocketListener, StreamSocketListenerConnectionReceivedEventArgs,
    };
    use windows::Storage::Streams::{DataReader, DataWriter};

    static SERVICE: Mutex<Option<(RfcommServiceProvider, StreamSocketListener)>> = Mutex::new(None);

    fn lire(lecteur: &DataReader, n: u32) -> windows::core::Result<Option<Vec<u8>>> {
        let mut sortie = Vec::with_capacity(n as usize);
        while (sortie.len() as u32) < n {
            let recus = lecteur.LoadAsync(n - sortie.len() as u32)?.get()?;
            if recus == 0 {
                return Ok(None);
            }
            let mut tampon = vec![0u8; recus as usize];
            lecteur.ReadBytes(&mut tampon)?;
            sortie.extend(tampon);
        }
        Ok(Some(sortie))
    }

    fn servir(socket: &StreamSocket, app: &tauri::AppHandle) -> windows::core::Result<()> {
        let telephone = socket.Information()?.RemoteAddress()?.DisplayName()?.to_string();
        let appelant = adresse_du_telephone(&telephone);
        crate::reception_directe::noter(format!("🔵 Canal Bluetooth ouvert par un téléphone ({telephone})."));
        let lecteur = DataReader::CreateDataReader(&socket.InputStream()?)?;
        let ecrivain = DataWriter::CreateDataWriter(&socket.OutputStream()?)?;
        loop {
            let Some(longueur) = lire(&lecteur, 4)? else { break };
            let longueur = u32::from_be_bytes([longueur[0], longueur[1], longueur[2], longueur[3]]);
            if longueur as usize > TAILLE_MAX_TRAME {
                break;
            }
            let Some(contenu) = lire(&lecteur, longueur)? else { break };
            let Some((entete, corps)) = lire_trame(&contenu) else { break };
            let (reponse, corps_reponse) = traiter(app, &entete, corps, appelant);
            ecrivain.WriteBytes(&ecrire_trame(&reponse, &corps_reponse))?;
            ecrivain.StoreAsync()?.get()?;
        }
        crate::reception_directe::noter("🔵 Canal Bluetooth fermé.");
        Ok(())
    }

    fn enregistrer(app: tauri::AppHandle) -> windows::core::Result<()> {
        let fournisseur =
            RfcommServiceProvider::CreateAsync(&RfcommServiceId::FromUuid(GUID::from_u128(SERVICE_UUID))?)?.get()?;
        let ecouteur = StreamSocketListener::new()?;
        ecouteur.ConnectionReceived(&TypedEventHandler::<
            StreamSocketListener,
            StreamSocketListenerConnectionReceivedEventArgs,
        >::new(move |_, arguments| {
            let Some(arguments) = arguments.as_ref() else { return Ok(()) };
            let socket = arguments.Socket()?;
            let app = app.clone();
            std::thread::spawn(move || {
                if let Err(e) = servir(&socket, &app) {
                    crate::reception_directe::noter(format!("🔵 Canal Bluetooth interrompu : {e}"));
                }
            });
            Ok(())
        }))?;
        // Sans appairage : le téléphone se relie en mode « non sécurisé ».
        // La page d'envoi, elle, est déjà ouverte à tout téléphone relié.
        ecouteur
            .BindServiceNameWithProtectionLevelAsync(&fournisseur.ServiceId()?.AsString()?, SocketProtectionLevel::PlainSocket)?
            .get()?;
        fournisseur.StartAdvertising(&ecouteur)?;
        if let Ok(mut s) = SERVICE.lock() {
            *s = Some((fournisseur, ecouteur));
        }
        Ok(())
    }

    std::thread::spawn(move || match enregistrer(app) {
        Ok(()) => crate::reception_directe::noter("🔵 Canal Bluetooth prêt : les téléphones peuvent se relier au PC."),
        Err(e) => crate::reception_directe::noter(format!("⚠️ Canal Bluetooth indisponible sur ce PC ({e}).")),
    });
}

#[cfg(not(windows))]
pub fn demarrer(_app: tauri::AppHandle) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trame_aller_retour() {
        let t = ecrire_trame(&json!({"t": "http", "id": 3}), b"\x00\x01corps\n");
        let n = u32::from_be_bytes([t[0], t[1], t[2], t[3]]) as usize;
        assert_eq!(n, t.len() - 4);
        let (entete, corps) = lire_trame(&t[4..]).unwrap();
        assert_eq!(entete["id"], 3);
        assert_eq!(corps, b"\x00\x01corps\n");
        assert!(lire_trame(b"pas du json\n").is_none());
    }

    #[test]
    fn seules_les_adresses_de_la_page_d_envoi_passent() {
        assert!(chemin_autorise("GET", "/infos"));
        assert!(chemin_autorise("POST", "/envoyer"));
        assert!(chemin_autorise("GET", "/statut/a1B2-c3"));
        assert!(chemin_autorise("GET", "/morceau/0af3"));
        assert!(chemin_autorise("POST", "/morceau/0af3?debut=1048576"));
        assert!(!chemin_autorise("POST", "/morceau/0af3?debut=abc"));
        assert!(!chemin_autorise("POST", "/morceau/0af3"));
        assert!(!chemin_autorise("GET", "/statut/../../admin"));
        assert!(!chemin_autorise("GET", "/"));
        assert!(!chemin_autorise("DELETE", "/infos"));
        assert!(!chemin_autorise("GET", "/infos?x=1"));
        assert!(!chemin_autorise("GET", "/classique"));
    }

    #[test]
    fn lit_les_reponses_du_serveur() {
        let r = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 11\r\n\r\n{\"recu\":12}";
        assert_eq!(lire_reponse_http(r), Some((200, b"{\"recu\":12}".to_vec())));
        let r = b"HTTP/1.1 409 Conflict\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"re\r\n6\r\ncu\":5}\r\n0\r\n\r\n";
        assert_eq!(lire_reponse_http(r), Some((409, b"{\"recu\":5}".to_vec())));
        let r = b"HTTP/1.1 204 No Content\r\n\r\n";
        assert_eq!(lire_reponse_http(r), Some((204, Vec::new())));
        assert_eq!(lire_reponse_http(b"n'importe quoi"), None);
    }

    #[test]
    fn adresse_stable_par_telephone() {
        assert_eq!(adresse_du_telephone("AA:BB"), adresse_du_telephone("AA:BB"));
        assert_ne!(adresse_du_telephone("AA:BB"), adresse_du_telephone("AA:BC"));
    }

    /// Doit rester identique à `CanalBt.kt` : sinon le téléphone ne trouve
    /// jamais le service, sans erreur visible.
    #[test]
    fn meme_service_que_l_envoyeur_android() {
        let kotlin = include_str!("../../android/app/src/main/java/bj/photocopie/envoyeur/CanalBt.kt");
        assert!(kotlin.contains("\"6b1f0c2e-8a4d-4f3b-9c55-4b5051434f50\""));
        assert_eq!(format!("{SERVICE_UUID:032x}"), "6b1f0c2e8a4d4f3b9c554b5051434f50");
    }
}
