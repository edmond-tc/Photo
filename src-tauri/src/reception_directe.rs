//! Réception directe : le PC REJOINT le réseau du téléphone du client.
//!
//! Pendant des mois, tout a reposé sur un PC qui CRÉE un Wi-Fi — fonction
//! que beaucoup de cartes ont abandonnée, d'où des échecs d'un PC à
//! l'autre. Ici c'est l'inverse : le téléphone du client (Android ou
//! iPhone) partage sa connexion avec le mot de passe de notre réseau, et le
//! PC le rejoint. Rejoindre un réseau, toutes les cartes Wi-Fi le savent :
//! c'est leur fonction de base.
//!
//! Déroulé, en boucle, sans aucun geste au PC :
//! 1. regarder les réseaux autour (API Wi-Fi officielle de Windows) ;
//! 2. prendre le plus fort au-dessus du seuil — avec des kiosques en cages,
//!    c'est le téléphone du client au guichet ;
//! 3. le rejoindre avec le mot de passe de notre réseau ;
//! 4. attendre l'envoi (page `http://kiosque.local:4173`, voir `mdns.rs`) ;
//! 5. se libérer et guetter le client suivant.
//!
//! VERSION D'ESSAI : chaque étape est notée dans un journal horodaté,
//! affiché à l'écran. Une photo de cet écran dit ce qui a marché et ce qui
//! a échoué, sans avoir à refaire les essais au hasard.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::Ipv4Addr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

/// Mot de passe de notre réseau, par défaut. Le gérant peut le changer dans
/// l'écran d'essai ; les clients le mettent une fois sur leur partage de
/// connexion.
pub const MOT_DE_PASSE_PAR_DEFAUT: &str = "kiosque2026";
/// Force de signal minimale (0 à 100, telle que Windows la donne) pour
/// considérer qu'un téléphone est au guichet.
pub const SEUIL_PAR_DEFAUT: u32 = 60;
/// Seuil maximal pour un réseau de l'application sans numéro de kiosque.
const SEUIL_SANS_NUMERO: u32 = 25;

/// Sans envoi dans ce délai après la connexion, on libère : le client est
/// peut-être parti, ou ce n'était pas un client.
const ATTENTE_PREMIER_ENVOI: Duration = Duration::from_secs(120);
/// Après le dernier envoi, on laisse ce délai au client pour un autre
/// fichier, puis on libère pour le suivant.
const CALME_APRES_ENVOI: Duration = Duration::from_secs(20);
/// Un client déjà servi n'est pas repris pendant ce délai.
const MISE_A_L_ECART: Duration = Duration::from_secs(30);
/// Un réseau d'envoyeur qui a échoué n'est pas retenté avant ce délai (le
/// téléphone crée un nouveau nom à chaque envoi).
const MISE_A_L_ECART_ECHEC: Duration = Duration::from_secs(30 * 60);
fn envoi_recu() -> bool {
    DERNIER_ENVOI.lock().ok().and_then(|d| *d).is_some()
}
const PAUSE_ENTRE_TOURS: Duration = Duration::from_secs(1);
/// Écoute Bluetooth en marche : le PC attend l'appel d'un téléphone ce
/// temps-là avant de regarder, en secours, la liste des Wi-Fi.
const ATTENTE_APPEL: Duration = Duration::from_secs(10);

/// Le PC est relié (ou se relie) au téléphone d'un client : le gardien du
/// Wi-Fi de la boutique ne doit pas lui reprendre la carte à ce moment-là
/// (voir `gardien_wifi.rs`).
static OCCUPEE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn occupee() -> bool {
    OCCUPEE.load(std::sync::atomic::Ordering::SeqCst)
}

struct Occupation;

impl Occupation {
    fn prendre() -> Self {
        OCCUPEE.store(true, std::sync::atomic::Ordering::SeqCst);
        Occupation
    }
}

impl Drop for Occupation {
    fn drop(&mut self) {
        OCCUPEE.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Ratés successifs par réseau appelé (voir `ecart_echec`).
static RATES_PAR_APPEL: Mutex<Vec<(String, u32)>> = Mutex::new(Vec::new());

/// Après un raté. Un téléphone qui APPELLE est là, devant le guichet : on
/// le retente aussitôt, jusqu'à trois fois (un premier essai raté est
/// courant : le réseau du téléphone finit à peine de naître). Avant, un
/// seul raté écartait le téléphone 30 minutes — le client attendait pour
/// rien.
fn ecart_echec(ssid: &str, par_appel: bool) -> Duration {
    if !par_appel {
        return ecart_apres_echec(ssid);
    }
    let Ok(mut rates) = RATES_PAR_APPEL.lock() else { return ecart_apres_echec(ssid) };
    let n = match rates.iter_mut().find(|(s, _)| s == ssid) {
        Some((_, n)) => {
            *n += 1;
            *n
        }
        None => {
            rates.push((ssid.to_string(), 1));
            if rates.len() > 50 {
                rates.remove(0);
            }
            1
        }
    };
    delai_apres_rates(n, ssid)
}

pub fn delai_apres_rates(rates: u32, ssid: &str) -> Duration {
    if rates < 3 { Duration::from_secs(2) } else { ecart_apres_echec(ssid) }
}

/// Force d'un appel Bluetooth (dBm) sur l'échelle de Windows (0 à 100).
pub fn signal_en_pourcentage(dbm: i16) -> u32 {
    (2 * (i32::from(dbm) + 100)).clamp(1, 100) as u32
}

// ───────────────────────────── Journal de diagnostic ─────────────────────────────

const TAILLE_JOURNAL: usize = 200;
static JOURNAL: Mutex<VecDeque<(String, String)>> = Mutex::new(VecDeque::new());
static ETAT: Mutex<String> = Mutex::new(String::new());
static ADRESSE: Mutex<Option<Ipv4Addr>> = Mutex::new(None);
static DERNIER_ENVOI: Mutex<Option<Instant>> = Mutex::new(None);

pub(crate) fn noter(texte: impl Into<String>) {
    let heure = chrono::Local::now().format("%H:%M:%S").to_string();
    if let Ok(mut j) = JOURNAL.lock() {
        j.push_front((heure, texte.into()));
        j.truncate(TAILLE_JOURNAL);
    }
}

fn etat(texte: impl Into<String>) {
    if let Ok(mut e) = ETAT.lock() {
        *e = texte.into();
    }
}

/// L'adresse du PC sur le réseau du client en cours, s'il y en a un. Lue
/// par le répondeur `kiosque.local` (voir `mdns.rs`).
pub fn adresse_actuelle() -> Option<Ipv4Addr> {
    ADRESSE.lock().ok().and_then(|a| *a)
}

/// Appelé par le serveur à chaque envoi réussi : le client est servi.
/// Le client est là et agit : un morceau de fichier vient d'arriver, ou
/// son application a donné signe de vie. Constaté à l'essai : un fichier
/// de 209 Mo était encore à 13 % au bout de 2 minutes, et le PC, qui
/// n'attendait qu'un envoi TERMINÉ, a quitté le téléphone en plein
/// transfert.
static DERNIERE_ACTIVITE: Mutex<Option<Instant>> = Mutex::new(None);

pub fn signaler_activite() {
    if let Ok(mut a) = DERNIERE_ACTIVITE.lock() {
        *a = Some(Instant::now());
    }
}

/// Pas de nouvelles du client depuis ce délai : il est parti.
const ACTIVITE_RECENTE: Duration = Duration::from_secs(30);

fn derniere_activite() -> Option<Instant> {
    DERNIERE_ACTIVITE.lock().ok().and_then(|a| *a)
}

static DEJA_NOTES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Une ligne de journal par réseau, pas une toutes les 5 secondes.
fn noter_une_fois(ssid: &str, ligne: String) {
    if let Ok(mut d) = DEJA_NOTES.lock() {
        if d.iter().any(|s| s == ssid) {
            return;
        }
        d.push(ssid.to_string());
        if d.len() > 100 {
            d.remove(0);
        }
    }
    noter(ligne);
}

pub fn signaler_envoi() {
    if let Ok(mut d) = DERNIER_ENVOI.lock() {
        *d = Some(Instant::now());
    }
}

// ───────────────────────────── Logique pure (testable) ─────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct Reseau {
    pub ssid: String,
    /// 0 à 100.
    pub signal: u32,
    /// WPA3 seul : le profil doit le dire, sinon Windows refuse.
    pub wpa3: bool,
    /// Protégé par mot de passe (WPA2 ou WPA3 personnel).
    pub protege: bool,
    /// Réseau que ce PC connaît déjà (Wi-Fi de la boutique, box) ou auquel
    /// il est connecté : jamais un client, on n'y touche pas.
    pub connu: bool,
    /// Sécurité annoncée, pour le journal (diagnostic).
    pub securite: String,
}

/// Le réseau à rejoindre : protégé, au-dessus du seuil, pas mis à l'écart,
/// et le plus fort. `ignorer` : nos propres réseaux et ceux qu'on ne doit
/// jamais tenter.
///
/// Deux sortes de clients :
/// - l'application Android, dont le réseau « DIRECT-KQ-… » passe toujours
///   en premier ;
/// - l'iPhone (ou un Android sans l'application), qui allume son partage de
///   connexion à la main. Constaté à l'essai : tenter tous les réseaux fait
///   perdre son temps au PC sur les box et partages du voisinage (HUAWEI,
///   CPE, TECNO…). Seul un réseau qui n'était pas là au démarrage (`fond`)
///   est donc tenté : un téléphone qu'on vient d'allumer au guichet.
pub fn choisir<'a>(
    reseaux: &'a [Reseau],
    seuil: u32,
    a_l_ecart: &dyn Fn(&str) -> bool,
    ignorer: &[String],
    fond: &HashSet<String>,
    mon_numero: &str,
) -> Option<&'a Reseau> {
    let _ = fond;
    reseaux
        .iter()
        // SEULS les réseaux de l'application (« DIRECT-KQ-… ») : constaté
        // sur le terrain, tenter les partages de connexion et box du
        // voisinage faisait se connecter et se déconnecter le PC en boucle
        // pendant que le vrai client attendait.
        .filter(|r| est_envoyeur(&r.ssid) && r.protege && !r.connu)
        // Le réseau porte le numéro de CE kiosque : c'est forcément son
        // client, quel que soit le signal (PC à antenne faible, téléphone
        // dans la main à un mètre). Constaté : un seuil de 60 % cachait le
        // téléphone, et le journal ne disait rien. Sans numéro : seuil bas.
        .filter(|r| {
            if kiosque_du_reseau(&r.ssid) == Some(mon_numero) {
                r.signal >= 1
            } else {
                r.signal >= seuil.min(SEUIL_SANS_NUMERO)
            }
        })
        // Le téléphone d'un client qui envoie à un AUTRE kiosque.
        .filter(|r| kiosque_du_reseau(&r.ssid).is_none_or(|n| n == mon_numero))
        .filter(|r| !a_l_ecart(&r.ssid) && !ignorer.iter().any(|i| i == &r.ssid))
        .max_by_key(|r| (kiosque_du_reseau(&r.ssid).is_some(), est_envoyeur(&r.ssid), r.signal))
}

/// Dernière réparation du Wi-Fi du PC tentée (voir `hotspot::remettre_wifi_en_marche`).
static DERNIERE_REPARATION: Mutex<Option<Instant>> = Mutex::new(None);
const PAUSE_ENTRE_REPARATIONS: Duration = Duration::from_secs(10 * 60);

/// Répare le Wi-Fi du PC, si ce n'a pas été tenté il y a moins de 10 minutes.
/// Rend la phrase à écrire au journal, ou None si on attend encore.
fn reparer_wifi() -> Option<String> {
    {
        let mut derniere = DERNIERE_REPARATION.lock().ok()?;
        if derniere.is_some_and(|t| t.elapsed() < PAUSE_ENTRE_REPARATIONS) {
            return None;
        }
        *derniere = Some(Instant::now());
    }
    etat("Remise en marche du Wi-Fi du PC… (cliquez « Oui » si Windows le demande)");
    Some(match crate::hotspot::remettre_wifi_en_marche() {
        Ok(sortie) => format!("🔧 Wi-Fi du PC remis en marche : {sortie}"),
        Err(raison) => format!("⚠️ Wi-Fi du PC : {raison}"),
    })
}

/// Temps laissé à une connexion. Un réseau qui n'est pas notre envoyeur a
/// droit à moins : pendant qu'on l'essaie, le PC ne voit pas un vrai client
/// arriver.
fn attente_connexion(ssid: &str) -> Duration {
    Duration::from_secs(if est_envoyeur(ssid) { 25 } else { 12 })
}

/// Un réseau qui a échoué n'est pas retenté avant ce délai. Celui de
/// l'envoyeur change de nom à chaque envoi ; un autre (box, partage d'un
/// inconnu) n'acceptera pas mieux notre mot de passe plus tard.
fn ecart_apres_echec(ssid: &str) -> Duration {
    if est_envoyeur(ssid) { MISE_A_L_ECART_ECHEC } else { Duration::from_secs(6 * 3600) }
}

/// Réseau créé par notre envoyeur Android (voir `android/`, `Reglages.kt`).
pub const PREFIXE_ENVOYEUR: &str = "DIRECT-KQ-";

pub fn est_envoyeur(ssid: &str) -> bool {
    ssid.starts_with(PREFIXE_ENVOYEUR)
}

// ─────────────── Un code par kiosque ───────────────
//
// Chaque PC a son numéro (6 caractères), qu'il émet dans sa balise
// Bluetooth. Le téléphone entend la balise du guichet où il se trouve et
// crée son réseau POUR CE KIOSQUE : « DIRECT-KQ-3FA92C-XXXX », avec un mot
// de passe propre à ce kiosque. Un kiosque voisin ne le rejoint donc
// jamais, et le mot de passe commun « kiosque2026 » ne sert plus qu'aux
// téléphones qui n'ont pas entendu de balise.
//
// Le mot de passe est dérivé du numéro et d'un secret partagé avec
// l'application (voir `Reglages.kt`) : il sépare les kiosques ; il n'est
// pas un verrou contre quelqu'un qui démonterait l'application.

pub const SECRET_KIOSQUE: &str = "photocopie-benin/kiosque/v1/8d41c7e2b9f05a36";

pub fn mot_de_passe_kiosque(numero: &str) -> String {
    use sha2::{Digest, Sha256};
    let empreinte = Sha256::digest(format!("{SECRET_KIOSQUE}:{numero}"));
    empreinte.iter().take(8).map(|o| format!("{o:02x}")).collect()
}

/// Le numéro de kiosque dans le nom d'un réseau d'envoyeur, s'il y en a un.
pub fn kiosque_du_reseau(ssid: &str) -> Option<&str> {
    let reste = ssid.strip_prefix(PREFIXE_ENVOYEUR)?;
    let (numero, suite) = reste.split_once('-')?;
    (numero.len() == 6
        && numero.bytes().all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
        && !suite.is_empty())
    .then_some(numero)
}

/// Numéro de ce kiosque, créé une fois pour toutes au premier besoin.
pub fn numero_kiosque(app: &AppHandle) -> String {
    let state = app.state::<crate::db::DbState>();
    let Ok(conn) = state.0.lock() else {
        return "000000".to_string();
    };
    if let Some(n) = crate::db::get_setting(&conn, "kiosque_numero").filter(|n| n.len() == 6) {
        return n;
    }
    let n = format!("{:06X}", rand::random::<u32>() & 0x00FF_FFFF);
    let _ = crate::db::set_setting(&conn, "kiosque_numero", &n);
    n
}

/// Port où le PC annonce sa présence sur le réseau du client : l'envoyeur
/// Android l'y trouve sans connaître son adresse.
pub const PORT_ANNONCE: u16 = 48173;

/// Diffuse « KIOSQUE 4173 » chaque seconde tant que le PC est sur ce réseau.
fn annoncer_presence(adresse: Ipv4Addr) {
    std::thread::spawn(move || {
        let Ok(socket) = std::net::UdpSocket::bind((adresse, 0)) else { return };
        if socket.set_broadcast(true).is_err() {
            return;
        }
        let message = format!("KIOSQUE {}", crate::server::PORT);
        let o = adresse.octets();
        let destinations = [
            std::net::SocketAddrV4::new(Ipv4Addr::BROADCAST, PORT_ANNONCE),
            std::net::SocketAddrV4::new(Ipv4Addr::new(o[0], o[1], o[2], 255), PORT_ANNONCE),
        ];
        while adresse_actuelle() == Some(adresse) {
            for d in destinations {
                let _ = socket.send_to(message.as_bytes(), d);
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

fn echapper_xml(texte: &str) -> String {
    texte
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn nom_profil(ssid: &str) -> String {
    format!("PB-client-{ssid}")
}

/// Profil Wi-Fi Windows pour rejoindre le téléphone du client. En mode
/// MANUEL : Windows ne s'y reconnectera jamais tout seul plus tard.
///
/// « nonBroadcast » : Windows appelle le réseau par son nom au lieu
/// d'attendre de le voir passer dans sa liste. Un réseau annoncé par
/// l'appel Bluetooth du téléphone, né il y a deux secondes, est rejoint
/// sans recherche préalable.
pub fn profil_xml(ssid: &str, mot_de_passe: &str, wpa3: bool) -> String {
    let hex: String = ssid.bytes().map(|o| format!("{o:02X}")).collect();
    let authentification = if wpa3 { "WPA3SAE" } else { "WPA2PSK" };
    format!(
        "<?xml version=\"1.0\"?>\
<WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\">\
<name>{nom}</name>\
<SSIDConfig><SSID><hex>{hex}</hex><name>{ssid}</name></SSID><nonBroadcast>true</nonBroadcast></SSIDConfig>\
<connectionType>ESS</connectionType>\
<connectionMode>manual</connectionMode>\
<MSM><security>\
<authEncryption><authentication>{authentification}</authentication><encryption>AES</encryption><useOneX>false</useOneX></authEncryption>\
<sharedKey><keyType>passPhrase</keyType><protected>false</protected><keyMaterial>{mdp}</keyMaterial></sharedKey>\
</security></MSM>\
</WLANProfile>",
        nom = echapper_xml(&nom_profil(ssid)),
        ssid = echapper_xml(ssid),
        mdp = echapper_xml(mot_de_passe),
    )
}

/// Ce que Windows raconte pendant une tentative de connexion.
#[derive(Debug, Clone, PartialEq)]
pub enum Evenement {
    /// Étape franchie (association, authentification, connecté…).
    Etape(&'static str),
    /// Connexion réussie selon Windows.
    Reussie,
    /// Connexion refusée, avec la raison donnée par Windows.
    Echec { code: u32, texte: String },
    /// Déconnecté, avec la raison.
    Deconnecte { code: u32, texte: String },
}

/// L'adresse apparue après la connexion : celle que le téléphone du client
/// a donnée au PC. À défaut, une adresse du réseau Wi-Fi Direct d'Android
/// (toujours 192.168.49.x) : constaté à l'essai, le téléphone avait trouvé
/// le PC à 192.168.49.96 alors que le PC ne voyait aucune adresse « nouvelle ».
pub fn nouvelle_adresse(avant: &[Ipv4Addr], apres: &[Ipv4Addr]) -> Option<Ipv4Addr> {
    let utilisable = |ip: &Ipv4Addr| !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified();
    apres
        .iter()
        .copied()
        .find(|ip| !avant.contains(ip) && utilisable(ip))
        .or_else(|| {
            apres
                .iter()
                .copied()
                .find(|ip| ip.octets()[..3] == [192, 168, 49] && ip.octets()[3] != 1 && utilisable(ip))
        })
}

// ───────────────────────────── Réglages ─────────────────────────────

#[derive(serde::Serialize, Clone)]
pub struct Reglages {
    pub active: bool,
    pub mot_de_passe: String,
    pub seuil: u32,
}

fn lire_reglages(app: &AppHandle) -> Reglages {
    let state = app.state::<crate::db::DbState>();
    let conn = state.0.lock();
    let lire = |cle: &str| conn.as_ref().ok().and_then(|c| crate::db::get_setting(c, cle));
    Reglages {
        // Active par défaut : elle ne sert que quand ce PC n'a pas de Wi-Fi
        // de boutique en marche (voir `wifi_boutique_prioritaire`). Les
        // anciennes versions forçaient « non » à chaque démarrage : seul
        // « coupee » (choix exprès) l'arrête désormais.
        active: lire("reception_directe_active").as_deref() != Some("coupee"),
        mot_de_passe: lire("reception_directe_mdp")
            .filter(|m| m.chars().count() >= 8)
            .unwrap_or_else(|| MOT_DE_PASSE_PAR_DEFAUT.to_string()),
        seuil: lire("reception_directe_seuil")
            .and_then(|s| s.parse().ok())
            .map(|s: u32| s.clamp(1, 100))
            .unwrap_or(SEUIL_PAR_DEFAUT),
    }
}

/// La réception directe est-elle activée ? (Lu aussi par `balise_ble.rs`.)
pub fn est_active(app: &AppHandle) -> bool {
    lire_reglages(app).active
}

// ───────────────────────────── Commandes ─────────────────────────────

#[derive(serde::Serialize)]
pub struct EtatReception {
    pub reglages: Reglages,
    pub etat: String,
    pub adresse: Option<String>,
    pub journal: Vec<(String, String)>,
}

#[tauri::command]
pub fn reception_directe_etat(app: AppHandle) -> EtatReception {
    EtatReception {
        reglages: lire_reglages(&app),
        etat: ETAT.lock().map(|e| e.clone()).unwrap_or_default(),
        adresse: adresse_actuelle().map(|a| a.to_string()),
        journal: JOURNAL.lock().map(|j| j.iter().cloned().collect()).unwrap_or_default(),
    }
}

#[tauri::command]
pub fn reception_directe_regler(
    app: AppHandle,
    active: bool,
    mot_de_passe: String,
    seuil: u32,
) -> Result<(), String> {
    if mot_de_passe.chars().count() < 8 {
        return Err("Le mot de passe doit faire au moins 8 caractères (règle du Wi-Fi).".to_string());
    }
    let state = app.state::<crate::db::DbState>();
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    crate::db::set_setting(&conn, "reception_directe_active", if active { "oui" } else { "non" })
        .map_err(|e| e.to_string())?;
    crate::db::set_setting(&conn, "reception_directe_mdp", &mot_de_passe).map_err(|e| e.to_string())?;
    crate::db::set_setting(&conn, "reception_directe_seuil", &seuil.clamp(1, 100).to_string())
        .map_err(|e| e.to_string())?;
    noter(if active { "Réception directe ACTIVÉE" } else { "Réception directe arrêtée" });
    Ok(())
}

/// Adresse fixe de la page : la même pour tous les clients, quel que soit
/// le téléphone. C'est elle que porte le QR imprimé du guichet.
pub fn adresse_fixe() -> String {
    format!("http://{}:{}/", crate::mdns::NOM, crate::server::PORT)
}

#[derive(serde::Serialize)]
pub struct QrReception {
    /// QR imprimé, collé au guichet : `http://kiosque.local:4173/`.
    pub fixe_url: String,
    pub fixe: String,
    /// QR affiché à l'écran pendant qu'un client est connecté, avec
    /// l'adresse en chiffres : secours si le téléphone ne trouve pas
    /// `kiosque.local`.
    pub direct_url: Option<String>,
    pub direct: Option<String>,
    /// LE QR de l'affiche du guichet, le même pour tous : la page reconnaît
    /// le téléphone (Android : installer l'application ; iPhone : envoyer
    /// par WhatsApp à CETTE boutique, dont le numéro voyage dans le lien).
    pub application_url: String,
    pub application: String,
}

/// Page d'installation de l'application Envoyeur Kiosque (admin/src/index.js, `/app`).
pub const ADRESSE_APPLICATION: &str = "https://photocopie-admin.atinzed2.workers.dev/app";

/// Adresse du QR de l'affiche, avec le numéro WhatsApp de la boutique et le
/// nom Bluetooth du PC : la page propose alors d'envoyer par l'un ou
/// l'autre sans installer l'application, en montrant quel nom toucher.
pub fn adresse_application(whatsapp: Option<&str>, bluetooth: Option<&str>) -> String {
    let mut parametres = Vec::new();
    if let Some(numero) = whatsapp.and_then(crate::server::normalize_phone) {
        parametres.push(format!("w={numero}"));
    }
    if let Some(nom) = bluetooth.map(nom_bluetooth_pour_adresse).filter(|n| !n.is_empty()) {
        parametres.push(format!("b={nom}"));
    }
    if parametres.is_empty() {
        ADRESSE_APPLICATION.to_string()
    } else {
        format!("{ADRESSE_APPLICATION}?{}", parametres.join("&"))
    }
}

/// Lettres, chiffres, espace, tiret, point et souligné seulement (40 au plus),
/// encodés pour une adresse : rien d'autre ne peut passer par le QR.
fn nom_bluetooth_pour_adresse(nom: &str) -> String {
    nom.trim()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
        .take(40)
        .map(|c| match c {
            ' ' => "%20".to_string(),
            c if c.is_ascii() => c.to_string(),
            c => {
                let mut tampon = [0u8; 4];
                c.encode_utf8(&mut tampon).bytes().map(|o| format!("%{o:02X}")).collect()
            }
        })
        .collect()
}

#[tauri::command]
pub fn reception_directe_qr(state: tauri::State<crate::db::DbState>) -> Result<QrReception, String> {
    let (whatsapp, bluetooth) = {
        let conn = state.0.lock().map_err(|e| e.to_string())?;
        (
            crate::db::get_setting(&conn, "boutique_whatsapp"),
            crate::db::get_setting(&conn, "bluetooth_nom"),
        )
    };
    // Sans nom réglé par le gérant : celui que Windows montre réellement dans
    // la liste Bluetooth du téléphone, le nom de l'ordinateur.
    let bluetooth = bluetooth
        .filter(|n| !n.trim().is_empty())
        .or_else(|| std::env::var("COMPUTERNAME").ok());
    let application_url = adresse_application(whatsapp.as_deref(), bluetooth.as_deref());
    let fixe_url = adresse_fixe();
    let direct_url =
        adresse_actuelle().map(|a| format!("http://{a}:{}/", crate::server::PORT));
    Ok(QrReception {
        fixe: crate::qr::build_qr_data_uri(&fixe_url)?,
        direct: direct_url.as_deref().map(crate::qr::build_qr_data_uri).transpose()?,
        fixe_url,
        direct_url,
        application: crate::qr::build_qr_data_uri(&application_url)?,
        application_url,
    })
}

/// Prépare ce PC une fois pour toutes : ouvre le pare-feu (page d'envoi et
/// nom `kiosque.local`). Une seule fenêtre d'autorisation Windows.
#[tauri::command]
pub async fn reception_directe_preparer() -> Result<(), String> {
    let resultat = tauri::async_runtime::spawn_blocking(crate::pare_feu::autoriser)
        .await
        .map_err(|e| e.to_string())?;
    match &resultat {
        Ok(()) => noter("🛡 Pare-feu préparé (page d'envoi et kiosque.local)."),
        Err(e) => noter(format!("❌ Préparation du pare-feu : {e}")),
    }
    resultat
}

/// Ouvre la page Windows « Localisation » : sur Windows 11 récent, sans
/// elle, Windows refuse de montrer les Wi-Fi autour aux applications.
#[tauri::command]
pub fn reception_directe_ouvrir_localisation() -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("cmd")
            .args(["/C", "start", "", "ms-settings:privacy-location"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    Err("Disponible uniquement sur Windows".to_string())
}

// ───────────────────────────── Boucle ─────────────────────────────

fn adresses_ipv4() -> Vec<Ipv4Addr> {
    local_ip_address::list_afinet_netifas()
        .map(|l| {
            l.into_iter()
                .filter_map(|(_, ip)| match ip {
                    std::net::IpAddr::V4(v4) => Some(v4),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Lance la boucle en arrière-plan, pour toute la durée de l'application.
/// Elle ne fait rien tant que la réception directe n'est pas activée.
pub fn demarrer(app: AppHandle) {
    std::thread::spawn(move || {
        let mut mis_a_l_ecart: HashMap<String, Instant> = HashMap::new();
        // Réseaux du voisinage, appris au premier balayage (voir `choisir`).
        let mut fond: Option<HashSet<String>> = None;
        let mut empeche_veille = false;
        etat("Arrêtée");
        loop {
            let reglages = lire_reglages(&app);
            if reglages.active != empeche_veille {
                garder_eveille(reglages.active);
                empeche_veille = reglages.active;
            }
            if reglages.active && wifi_boutique_prioritaire(&app) {
                // Les deux méthodes se disputent la même carte Wi-Fi : une
                // carte ne peut pas à la fois créer un réseau et rejoindre
                // sans cesse ceux des téléphones. Le Wi-Fi de la boutique a
                // la priorité (demande du terrain : il doit être allumé en
                // permanence, comme une box). La réception directe attend
                // donc, sans rien couper, que le gérant l'éteigne.
                etat("En pause : le Wi-Fi de la boutique est allumé et a la priorité.");
            } else if reglages.active {
                un_tour(&app, &reglages, &mut mis_a_l_ecart, &mut fond);
            } else {
                etat("Arrêtée");
            }
            std::thread::sleep(PAUSE_ENTRE_TOURS);
        }
    });
}

/// Le gérant garde le Wi-Fi de la boutique allumé (ou il l'est déjà) :
/// la réception directe ne touche pas à la carte Wi-Fi.
fn wifi_boutique_prioritaire(app: &AppHandle) -> bool {
    // Routeur à part : le PC y est sans doute relié par sa carte Wi-Fi ;
    // partir rejoindre un téléphone couperait tous les autres clients.
    let routeur = {
        let state = app.state::<crate::db::DbState>();
        let conn = state.0.lock();
        conn.ok().and_then(|c| crate::db::get_setting(&c, "wifi_type_reseau")).as_deref() == Some("routeur_externe")
    };
    // Sinon, seul un Wi-Fi de boutique qui TOURNE (ou que le gardien est en
    // train de rallumer) a la priorité. Sur un PC qui ne sait pas en créer
    // — le cas pour lequel la réception directe existe —, la carte est
    // libre : avant, il suffisait que le gérant VEUILLE un Wi-Fi pour
    // qu'elle ne serve jamais.
    routeur || crate::hotspot::point_acces_actif() || crate::gardien_wifi::relance_en_cours()
}

/// Empêche le PC de se mettre en veille tout seul tant que la réception
/// directe est active (l'écran, lui, peut s'éteindre). En veille, plus rien
/// ne tourne : aucun client ne pourrait être servi.
#[cfg(windows)]
pub(crate) fn garder_eveille(oui: bool) {
    use windows::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED};
    // SAFETY : appel sans pointeur ; l'état reste attaché à ce fil, qui vit
    // aussi longtemps que l'application.
    unsafe {
        SetThreadExecutionState(if oui { ES_CONTINUOUS | ES_SYSTEM_REQUIRED } else { ES_CONTINUOUS });
    }
}

#[cfg(not(windows))]
pub(crate) fn garder_eveille(_oui: bool) {}

fn un_tour(
    app: &AppHandle,
    reglages: &Reglages,
    mis_a_l_ecart: &mut HashMap<String, Instant>,
    fond: &mut Option<HashSet<String>>,
) {
    mis_a_l_ecart.retain(|_, jusqu_a| *jusqu_a > Instant::now());

    let client = match wlan::Client::ouvrir() {
        Ok(c) => c,
        Err(e) => {
            // Carte désactivée, service arrêté, radio coupée : on répare
            // (au plus une fois toutes les 10 minutes, pour ne pas harceler
            // le gérant de fenêtres « Oui/Non »), puis on réessaie aussitôt.
            // Seulement si un client APPELLE : la réception directe tourne
            // désormais sur tous les PC, et un PC fixe sans carte Wi-Fi ne
            // doit pas recevoir une fenêtre « Oui » toutes les 10 minutes.
            let numero = numero_kiosque(app);
            let client_attend = crate::appel_ble::appel_a_servir(&numero, &|_: &str| false).is_some();
            if !client_attend {
                etat(format!("En veille : {e}"));
                noter_une_fois("carte-wifi-indisponible", format!("⚠️ Carte Wi-Fi indisponible : {e}"));
                std::thread::sleep(Duration::from_secs(10));
                return;
            }
            if let Some(message) = reparer_wifi() {
                noter(message);
                std::thread::sleep(Duration::from_secs(3));
                return;
            }
            etat(format!("Bloquée : {e}"));
            noter(format!("❌ {e}"));
            std::thread::sleep(Duration::from_secs(20));
            return;
        }
    };

    // Le Wi-Fi que ce PC crée peut-être lui-même (ancien système) : ne
    // jamais essayer de le rejoindre.
    let ignorer: Vec<String> = {
        let state = app.state::<crate::db::DbState>();
        let conn = state.0.lock();
        conn.ok()
            .and_then(|c| crate::db::get_setting(&c, "wifi_ssid"))
            .into_iter()
            .collect()
    };
    let fond = fond.get_or_insert_with(HashSet::new);
    let mon_numero = numero_kiosque(app);

    // 1. L'appel Bluetooth du téléphone (voir `appel_ble.rs`) : le PC sait
    //    tout de suite quel réseau rejoindre, sans le chercher.
    let ecoute = crate::appel_ble::ecoute_active();
    etat(if ecoute {
        "En attente d'un téléphone (écoute Bluetooth)…"
    } else {
        "En attente d'un téléphone…"
    });
    let appel = crate::appel_ble::attendre_appel(
        if ecoute { ATTENTE_APPEL } else { Duration::ZERO },
        &mon_numero,
        &|ssid: &str| mis_a_l_ecart.contains_key(ssid),
    );
    if let Some(appel) = appel {
        let cible = Reseau {
            ssid: appel.ssid.clone(),
            signal: signal_en_pourcentage(appel.signal),
            wpa3: false,
            protege: true,
            connu: false,
            securite: format!("appel Bluetooth, {} dBm", appel.signal),
        };
        // Une recherche du SEUL réseau annoncé : rapide, et elle aide les
        // cartes qui refusent de rejoindre un réseau jamais vu.
        client.scanner_cible(&cible.ssid);
        servir(app, &client, &cible, true, reglages, mis_a_l_ecart, &ignorer, &mon_numero);
        return;
    }

    // 2. Secours : la liste des Wi-Fi autour (PC sans Bluetooth, téléphone
    //    qui ne sait pas appeler).
    // Attendre la FIN de la recherche : lire la liste trop tôt rendait
    // celle d'avant, où le téléphone qui vient d'arriver n'est pas encore.
    client.scanner();
    let reseaux = match client.reseaux() {
        Ok(r) => r,
        Err(code) => {
            let message = if code == 5 {
                "Windows refuse de montrer les Wi-Fi autour : autorisez la LOCALISATION pour \
                 cette application (Paramètres → Confidentialité → Localisation)."
                    .to_string()
            } else {
                format!("Lecture des Wi-Fi impossible (code {code}).")
            };
            if ecoute {
                // L'appel Bluetooth suffit : la liste n'était qu'un secours.
                noter_une_fois("liste-wifi-refusee", format!("⚠️ {message} L'appel Bluetooth des téléphones reste entendu."));
                return;
            }
            etat(format!("Bloquée : {message}"));
            noter(format!("❌ {message}"));
            std::thread::sleep(Duration::from_secs(20));
            return;
        }
    };

    let a_l_ecart = |ssid: &str| mis_a_l_ecart.contains_key(ssid);
    // Chaque téléphone de l'application vu, une fois, avec ce qu'on en fait :
    // si le PC ne le rejoint pas, le journal dit pourquoi.
    for r in reseaux.iter().filter(|r| est_envoyeur(&r.ssid)) {
        let pourquoi = match kiosque_du_reseau(&r.ssid) {
            Some(n) if n != mon_numero => Some(format!("envoie au kiosque {n}, pas à celui-ci ({mon_numero})")),
            None if r.signal < reglages.seuil.min(SEUIL_SANS_NUMERO) => {
                Some(format!("signal trop faible ({} %) et sans numéro de kiosque", r.signal))
            }
            _ if r.connu => Some("réseau déjà connu de ce PC".to_string()),
            _ if a_l_ecart(&r.ssid) => Some("déjà servi ou en échec, attente".to_string()),
            _ => None,
        };
        if let Some(pourquoi) = pourquoi {
            noter_une_fois(&r.ssid, format!("👀 « {} » vu (signal {} %) mais pas rejoint : {pourquoi}.", r.ssid, r.signal));
        }
    }
    let Some(cible) = choisir(&reseaux, reglages.seuil, &a_l_ecart, &ignorer, fond, &mon_numero).cloned() else {
        return;
    };
    servir(app, &client, &cible, false, reglages, mis_a_l_ecart, &ignorer, &mon_numero);
}

/// Rejoint le téléphone du client, attend ses envois, puis le libère.
/// `par_appel` : le téléphone a appelé par Bluetooth (il est donc là, et
/// réessayer vite a du sens s'il y a un raté).
#[allow(clippy::too_many_arguments)]
fn servir(
    app: &AppHandle,
    client: &wlan::Client,
    cible: &Reseau,
    par_appel: bool,
    reglages: &Reglages,
    mis_a_l_ecart: &mut HashMap<String, Instant>,
    ignorer: &[String],
    mon_numero: &str,
) {
    let _occupation = Occupation::prendre();
    let cible = cible.clone();
    noter(format!(
        "📶 Téléphone {} : « {} » (signal {} %, {})",
        if par_appel { "appelle" } else { "vu" },
        cible.ssid,
        cible.signal,
        cible.securite
    ));
    etat(format!("Connexion à « {} »…", cible.ssid));
    if let Ok(mut d) = DERNIER_ENVOI.lock() {
        *d = None;
    }
    if let Ok(mut a) = DERNIERE_ACTIVITE.lock() {
        *a = None;
    }
    let avant = adresses_ipv4();
    let mot_de_passe = if kiosque_du_reseau(&cible.ssid).is_some() {
        mot_de_passe_kiosque(mon_numero)
    } else {
        reglages.mot_de_passe.clone()
    };
    if let Err(e) = client.rejoindre(&cible.ssid, &mot_de_passe, cible.wpa3) {
        noter(format!("❌ Connexion refusée à « {} » : {e}", cible.ssid));
        client.oublier(&cible.ssid);
        mis_a_l_ecart.insert(cible.ssid.clone(), Instant::now() + ecart_echec(&cible.ssid, par_appel));
        return;
    }

    // Connexion, puis adresse donnée par le téléphone. Windows raconte
    // chaque étape (voir `Evenement`) : en cas d'échec, le journal dit
    // POURQUOI, au lieu d'un simple « pas de connexion ».
    let debut = Instant::now();
    let mut adresse = None;
    let mut etapes: Vec<&'static str> = Vec::new();
    let mut connecte = false;
    let mut echec: Option<String> = None;
    while debut.elapsed() < if connecte { Duration::from_secs(45) } else { attente_connexion(&cible.ssid) } {
        std::thread::sleep(Duration::from_millis(500));
        for evenement in client.evenements(&cible.ssid) {
            match evenement {
                Evenement::Etape(e) => {
                    if e == "connecté" {
                        connecte = true;
                    }
                    if etapes.last() != Some(&e) {
                        etapes.push(e);
                    }
                }
                Evenement::Reussie => connecte = true,
                Evenement::Echec { code, texte } => {
                    echec = Some(format!("Windows refuse : {texte} (code {code})"))
                }
                Evenement::Deconnecte { code, texte } if code != 0 => {
                    echec = Some(format!("déconnecté par Windows : {texte} (code {code})"))
                }
                Evenement::Deconnecte { .. } => {}
            }
        }
        // Le téléphone a déjà envoyé : le PC était donc bien joignable, même
        // s'il n'a pas vu son adresse.
        if echec.is_some() || envoi_recu() {
            break;
        }
        if connecte || client.connecte_a().as_deref() == Some(cible.ssid.as_str()) {
            connecte = true;
            adresse = nouvelle_adresse(&avant, &adresses_ipv4());
            if adresse.is_some() {
                break;
            }
        }
    }
    let Some(adresse) = adresse else {
        let parcours_court = etapes.join(" → ");
        if envoi_recu() {
            noter(format!(
                "📄 « {} » : document reçu ({:.0} s). Étapes : {parcours_court}.",
                cible.ssid,
                debut.elapsed().as_secs_f32()
            ));
            etat(format!("Document reçu de « {} »", cible.ssid));
            // Laisser un instant pour d'autres fichiers, puis libérer.
            let fin = Instant::now() + CALME_APRES_ENVOI;
            while Instant::now() < fin
                && !client.evenements(&cible.ssid).iter().any(|e| matches!(e, Evenement::Deconnecte { .. }))
            {
                std::thread::sleep(Duration::from_secs(1));
            }
            client.deconnecter();
            client.oublier(&cible.ssid);
            mis_a_l_ecart.insert(cible.ssid.clone(), Instant::now() + MISE_A_L_ECART);
            return;
        }
        let parcours = if etapes.is_empty() { "aucune".to_string() } else { etapes.join(" → ") };
        let pourquoi = match echec {
            Some(e) => e,
            None if connecte => "connecté, mais aucune adresse reçue du téléphone en 45 s".to_string(),
            None => format!("aucune réponse du réseau en {} s", attente_connexion(&cible.ssid).as_secs()),
        };
        let pourquoi = if connecte {
            let liste = |l: &[Ipv4Addr]| l.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", ");
            format!("{pourquoi}. Adresses du PC avant : {} ; après : {}", liste(&avant), liste(&adresses_ipv4()))
        } else {
            pourquoi
        };
        noter(format!(
            "❌ « {} » : {pourquoi}. Étapes : {parcours}. {:.0} s.",
            cible.ssid,
            debut.elapsed().as_secs_f32()
        ));
        client.deconnecter();
        client.oublier(&cible.ssid);
        mis_a_l_ecart.insert(cible.ssid.clone(), Instant::now() + ecart_echec(&cible.ssid, par_appel));
        return;
    };

    let secondes = debut.elapsed().as_secs_f32();
    if let Ok(mut a) = ADRESSE.lock() {
        *a = Some(adresse);
    }
    annoncer_presence(adresse);
    noter(format!(
        "✅ Connecté à « {} » en {secondes:.1} s. Page : http://kiosque.local:{p} ou http://{adresse}:{p}",
        cible.ssid,
        p = crate::server::PORT
    ));
    etat(format!(
        "Connecté à « {} » — le client peut ouvrir http://kiosque.local:{}",
        cible.ssid,
        crate::server::PORT
    ));

    // Attendre l'envoi, puis un moment de calme.
    let connecte_depuis = Instant::now();
    let mut dernier_regard = Instant::now();
    let raison = loop {
        std::thread::sleep(Duration::from_secs(1));
        let coupe = client.evenements(&cible.ssid).iter().any(|e| matches!(e, Evenement::Deconnecte { .. }));
        if coupe || client.connecte_a().as_deref() != Some(cible.ssid.as_str()) {
            break "le téléphone a fermé son réseau";
        }
        // Un envoi en cours (même long) ou un client qui choisit encore ses
        // documents garde le PC ; il ne part qu'après un vrai silence.
        let activite = derniere_activite();
        let actif = activite.is_some_and(|t| t.elapsed() < ACTIVITE_RECENTE);
        let silencieux_depuis = activite.map_or(connecte_depuis.elapsed(), |t| {
            t.elapsed().min(connecte_depuis.elapsed())
        });
        let dernier = DERNIER_ENVOI.lock().ok().and_then(|d| *d);
        let calme = match dernier {
            Some(t) => t.elapsed() >= CALME_APRES_ENVOI && !actif,
            None => !actif && silencieux_depuis >= ATTENTE_PREMIER_ENVOI,
        };
        // Constaté sur le terrain : le PC quittait le téléphone 20 s après
        // un envoi et l'ignorait ensuite 3 minutes, alors que le client
        // touchait « Nouvelle commande » — son téléphone se croyait relié
        // et rien n'arrivait. Le PC reste donc tant que le téléphone garde
        // son réseau, et ne part que si un AUTRE client attend (ou après
        // 10 minutes sans rien).
        // L'application garde son réseau tant qu'elle est à l'écran : sans
        // cette règle, un premier client garderait le PC pour lui seul. Le
        // PC cède donc sa place à un client qui attend dès que le premier
        // n'envoie rien (jamais au milieu d'un envoi), après lui avoir
        // laissé au moins 20 secondes.
        let peut_ceder = crate::server::envois_en_cours() == 0
            && connecte_depuis.elapsed() >= Duration::from_secs(20)
            && dernier.is_none_or(|t| t.elapsed() >= Duration::from_secs(10));
        if calme && silencieux_depuis >= Duration::from_secs(10 * 60) {
            break "aucune activité depuis 10 minutes";
        }
        if calme || peut_ceder {
            let deja = |x: &str| x == cible.ssid || mis_a_l_ecart.contains_key(x);
            // Un autre téléphone appelle par Bluetooth : rien à chercher.
            let autre_appel = crate::appel_ble::appel_a_servir(mon_numero, &deja).is_some();
            if autre_appel {
                break if dernier.is_some() { "client servi, un autre client appelle" } else { "un autre client appelle" };
            }
            // Sans écoute Bluetooth seulement : une recherche Wi-Fi pendant
            // la connexion ralentit le téléphone relié. On la fait rarement.
            if !crate::appel_ble::ecoute_active() && dernier_regard.elapsed() >= Duration::from_secs(6) {
                dernier_regard = Instant::now();
                client.scanner();
                if let Ok(reseaux) = client.reseaux() {
                    if choisir(&reseaux, reglages.seuil, &deja, ignorer, &HashSet::new(), mon_numero).is_some() {
                        break if dernier.is_some() { "client servi, un autre client attend" } else { "un autre client attend" };
                    }
                }
            }
        }
        if !lire_reglages(app).active {
            break "réception directe arrêtée";
        }
    };
    let servi = DERNIER_ENVOI.lock().ok().and_then(|d| *d).is_some();
    noter(format!(
        "{} « {} » libéré : {raison}.",
        if servi { "📄" } else { "⏹" },
        cible.ssid
    ));
    client.deconnecter();
    client.oublier(&cible.ssid);
    if let Ok(mut a) = ADRESSE.lock() {
        *a = None;
    }
    mis_a_l_ecart.insert(cible.ssid, Instant::now() + MISE_A_L_ECART);
}

// ───────────────────────────── API Wi-Fi de Windows ─────────────────────────────

#[cfg(windows)]
mod wlan {
    use super::{Evenement, Reseau};
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::{BOOL, HANDLE};
    use windows::Win32::NetworkManagement::WiFi::*;

    /// Messages reçus de Windows (sur un de ses fils à lui), lus par la boucle,
    /// chacun avec le réseau qu'il concerne.
    ///
    /// Constaté sur le terrain : sans ce nom, l'« opération annulée » d'un
    /// partage de connexion du voisinage, tenté juste avant, était lue comme
    /// l'échec du téléphone du client — et le PC coupait lui-même une
    /// connexion réussie au bout d'une seconde, à chaque essai.
    static EVENEMENTS: Mutex<VecDeque<(String, Evenement)>> = Mutex::new(VecDeque::new());

    /// Windows a fini la recherche demandée par `scanner`.
    static RECHERCHE_FINIE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    fn pousser(ssid: String, e: Evenement) {
        if let Ok(mut liste) = EVENEMENTS.lock() {
            liste.push_back((ssid, e));
            while liste.len() > 100 {
                liste.pop_front();
            }
        }
    }

    /// Texte de Windows pour un code de raison (dans la langue du PC).
    pub fn raison_en_texte(code: u32) -> String {
        // Déclarée ici avec un tampon MODIFIABLE : la version du paquet
        // `windows` le prend en lecture seule, alors que Windows y écrit.
        #[link(name = "wlanapi")]
        extern "system" {
            fn WlanReasonCodeToString(
                code: u32,
                taille: u32,
                tampon: *mut u16,
                reserve: *const core::ffi::c_void,
            ) -> u32;
        }
        let mut tampon = [0u16; 512];
        // SAFETY : tampon local de 512 caractères, taille transmise à Windows.
        let r = unsafe {
            WlanReasonCodeToString(code, tampon.len() as u32, tampon.as_mut_ptr(), std::ptr::null())
        };
        if r != 0 {
            return format!("raison {code}");
        }
        let fin = tampon.iter().position(|&c| c == 0).unwrap_or(tampon.len());
        String::from_utf16_lossy(&tampon[..fin]).trim().to_string()
    }

    /// Nom du réseau porté par un message de Windows (vide s'il n'en dit rien).
    fn nom_du_reseau(ssid: &DOT11_SSID) -> String {
        let longueur = (ssid.uSSIDLength as usize).min(32);
        String::from_utf8_lossy(&ssid.ucSSID[..longueur]).to_string()
    }

    unsafe extern "system" fn rappel(donnees: *mut L2_NOTIFICATION_DATA, _contexte: *mut core::ffi::c_void) {
        let Some(d) = donnees.as_ref() else { return };
        let code = d.NotificationCode as i32;
        if d.NotificationSource == WLAN_NOTIFICATION_SOURCE_ACM
            && (code == wlan_notification_acm_scan_complete.0 || code == wlan_notification_acm_scan_fail.0)
        {
            RECHERCHE_FINIE.store(true, std::sync::atomic::Ordering::SeqCst);
            return;
        }
        if d.NotificationSource == WLAN_NOTIFICATION_SOURCE_ACM {
            let ssid = if !d.pData.is_null()
                && d.dwDataSize as usize
                    >= std::mem::offset_of!(WLAN_CONNECTION_NOTIFICATION_DATA, dot11Ssid) + std::mem::size_of::<DOT11_SSID>()
            {
                nom_du_reseau(&(*(d.pData as *const WLAN_CONNECTION_NOTIFICATION_DATA)).dot11Ssid)
            } else {
                String::new()
            };
            let raison = if !d.pData.is_null()
                && d.dwDataSize as usize >= std::mem::offset_of!(WLAN_CONNECTION_NOTIFICATION_DATA, dwFlags)
            {
                (*(d.pData as *const WLAN_CONNECTION_NOTIFICATION_DATA)).wlanReasonCode
            } else {
                0
            };
            if code == wlan_notification_acm_connection_complete.0 {
                pousser(ssid, if raison == 0 {
                    Evenement::Reussie
                } else {
                    Evenement::Echec { code: raison, texte: raison_en_texte(raison) }
                });
            } else if code == wlan_notification_acm_connection_attempt_fail.0 {
                pousser(ssid, Evenement::Echec { code: raison, texte: raison_en_texte(raison) });
            }
        } else if d.NotificationSource == WLAN_NOTIFICATION_SOURCE_MSM {
            let ssid = if !d.pData.is_null()
                && d.dwDataSize as usize
                    >= std::mem::offset_of!(WLAN_MSM_NOTIFICATION_DATA, dot11Ssid) + std::mem::size_of::<DOT11_SSID>()
            {
                nom_du_reseau(&(*(d.pData as *const WLAN_MSM_NOTIFICATION_DATA)).dot11Ssid)
            } else {
                String::new()
            };
            match code {
                c if c == wlan_notification_msm_associating.0 => pousser(ssid, Evenement::Etape("association")),
                c if c == wlan_notification_msm_authenticating.0 => pousser(ssid, Evenement::Etape("authentification")),
                c if c == wlan_notification_msm_connected.0 => pousser(ssid, Evenement::Etape("connecté")),
                c if c == wlan_notification_msm_disconnected.0 => {
                    let raison = if !d.pData.is_null()
                        && d.dwDataSize as usize >= std::mem::size_of::<WLAN_MSM_NOTIFICATION_DATA>()
                    {
                        (*(d.pData as *const WLAN_MSM_NOTIFICATION_DATA)).wlanReasonCode
                    } else {
                        0
                    };
                    pousser(ssid, Evenement::Deconnecte { code: raison, texte: raison_en_texte(raison) });
                }
                _ => {}
            }
        }
    }

    pub struct Client {
        poignee: HANDLE,
        interface: windows::core::GUID,
    }

    impl Client {
        pub fn ouvrir() -> Result<Self, String> {
            let mut version = 0u32;
            let mut poignee = HANDLE::default();
            // SAFETY : pointeurs vers des variables locales valides.
            let code = unsafe { WlanOpenHandle(2, None, &mut version, &mut poignee) };
            if code != 0 {
                return Err(format!(
                    "le service Wi-Fi de Windows ne répond pas (code {code}). Ce PC a-t-il une carte Wi-Fi ?"
                ));
            }
            let mut liste: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
            // SAFETY : `liste` est rempli par Windows et libéré plus bas.
            let code = unsafe { WlanEnumInterfaces(poignee, None, &mut liste) };
            if code != 0 || liste.is_null() {
                unsafe { WlanCloseHandle(poignee, None) };
                return Err(format!("impossible de lister les cartes Wi-Fi (code {code})."));
            }
            // SAFETY : `liste` non nul, structure fournie par Windows.
            let (nombre, interface) =
                unsafe { ((*liste).dwNumberOfItems, (*liste).InterfaceInfo[0].InterfaceGuid) };
            unsafe { WlanFreeMemory(liste as *const _) };
            if nombre == 0 {
                unsafe { WlanCloseHandle(poignee, None) };
                return Err("ce PC n'a aucune carte Wi-Fi active.".to_string());
            }
            // Être prévenu de chaque étape d'une connexion et de la raison
            // d'un refus. Sans cela, un échec n'est qu'un délai dépassé.
            // SAFETY : `rappel` est une fonction statique, sans contexte.
            unsafe {
                WlanRegisterNotification(
                    poignee,
                    WLAN_NOTIFICATION_SOURCES(WLAN_NOTIFICATION_SOURCE_ACM.0 | WLAN_NOTIFICATION_SOURCE_MSM.0),
                    BOOL(1),
                    Some(rappel),
                    None,
                    None,
                    None,
                );
            }
            if let Ok(mut liste) = EVENEMENTS.lock() {
                liste.clear();
            }
            Ok(Client { poignee, interface })
        }

        /// Les messages de Windows reçus depuis le dernier appel, pour CE
        /// réseau seulement : ceux d'un autre réseau (tenté avant, ou voisin)
        /// ne doivent jamais faire échouer la connexion en cours.
        pub fn evenements(&self, ssid: &str) -> Vec<Evenement> {
            EVENEMENTS
                .lock()
                .map(|mut l| l.drain(..).filter(|(s, _)| s == ssid).map(|(_, e)| e).collect())
                .unwrap_or_default()
        }

        /// Lance une recherche et attend sa fin (4 s au plus).
        pub fn scanner(&self) {
            use std::sync::atomic::Ordering;
            RECHERCHE_FINIE.store(false, Ordering::SeqCst);
            // SAFETY : poignée et GUID valides ; les autres paramètres sont facultatifs.
            let code = unsafe { WlanScan(self.poignee, &self.interface, None, None, None) };
            let debut = std::time::Instant::now();
            while code == 0
                && !RECHERCHE_FINIE.load(Ordering::SeqCst)
                && debut.elapsed() < std::time::Duration::from_secs(4)
            {
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
            if code != 0 {
                // Recherche refusée (déjà en cours) : laisser Windows finir.
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        }

        /// Les réseaux autour. Erreur = code Windows (5 = accès refusé :
        /// autorisation de localisation manquante, Windows 11 24H2).
        pub fn reseaux(&self) -> Result<Vec<Reseau>, u32> {
            let mut liste: *mut WLAN_AVAILABLE_NETWORK_LIST = std::ptr::null_mut();
            // SAFETY : `liste` est rempli par Windows et libéré plus bas.
            let code = unsafe {
                WlanGetAvailableNetworkList(self.poignee, &self.interface, 0, None, &mut liste)
            };
            if code != 0 || liste.is_null() {
                return Err(code);
            }
            let mut reseaux = Vec::new();
            // SAFETY : `dwNumberOfItems` entrées contiguës suivent l'en-tête.
            unsafe {
                let nombre = (*liste).dwNumberOfItems as usize;
                let premier = (*liste).Network.as_ptr();
                for i in 0..nombre {
                    let n = &*premier.add(i);
                    let longueur = (n.dot11Ssid.uSSIDLength as usize).min(32);
                    let ssid = String::from_utf8_lossy(&n.dot11Ssid.ucSSID[..longueur]).to_string();
                    let auth = n.dot11DefaultAuthAlgorithm;
                    let fin = n.strProfileName.iter().position(|&c| c == 0).unwrap_or(0);
                    let profil = String::from_utf16_lossy(&n.strProfileName[..fin]);
                    // Connu = un profil existe et ce n'est pas l'un des
                    // nôtres (« PB-client-… »), ou le PC y est connecté.
                    let connu = n.dwFlags & WLAN_AVAILABLE_NETWORK_CONNECTED != 0
                        || (n.dwFlags & WLAN_AVAILABLE_NETWORK_HAS_PROFILE != 0
                            && !profil.starts_with("PB-client-"));
                    reseaux.push(Reseau {
                        ssid,
                        signal: n.wlanSignalQuality,
                        wpa3: auth == DOT11_AUTH_ALGO_WPA3_SAE,
                        protege: n.bSecurityEnabled.as_bool()
                            && (auth == DOT11_AUTH_ALGO_RSNA_PSK || auth == DOT11_AUTH_ALGO_WPA3_SAE),
                        connu,
                        securite: format!(
                            "sécurité {}/{}",
                            auth.0, n.dot11DefaultCipherAlgorithm.0
                        ),
                    });
                }
                WlanFreeMemory(liste as *const _);
            }
            // Windows liste parfois deux fois le même réseau (avec et sans
            // profil) : connu pour l'un, connu pour tous.
            let connus: Vec<String> =
                reseaux.iter().filter(|r| r.connu).map(|r| r.ssid.clone()).collect();
            for r in &mut reseaux {
                r.connu |= connus.contains(&r.ssid);
            }
            Ok(reseaux)
        }

        pub fn rejoindre(&self, ssid: &str, mot_de_passe: &str, wpa3: bool) -> Result<(), String> {
            let xml = HSTRING::from(super::profil_xml(ssid, mot_de_passe, wpa3));
            let mut raison = 0u32;
            // Profil propre à l'utilisateur : aucun droit administrateur requis.
            // SAFETY : chaînes et pointeurs valides pendant l'appel.
            let mut code = unsafe {
                WlanSetProfile(
                    self.poignee,
                    &self.interface,
                    WLAN_PROFILE_USER,
                    &xml,
                    PCWSTR::null(),
                    BOOL(1),
                    None,
                    &mut raison,
                )
            };
            if code != 0 {
                // Certaines configurations refusent les profils par
                // utilisateur : on tente le profil commun.
                code = unsafe {
                    WlanSetProfile(self.poignee, &self.interface, 0, &xml, PCWSTR::null(), BOOL(1), None, &mut raison)
                };
            }
            if code != 0 {
                return Err(format!("profil refusé par Windows (code {code}, raison {raison})"));
            }
            let nom = HSTRING::from(super::nom_profil(ssid));
            let parametres = WLAN_CONNECTION_PARAMETERS {
                wlanConnectionMode: wlan_connection_mode_profile,
                strProfile: PCWSTR(nom.as_ptr()),
                pDot11Ssid: std::ptr::null_mut(),
                pDesiredBssidList: std::ptr::null_mut(),
                dot11BssType: dot11_BSS_type_infrastructure,
                dwFlags: 0,
            };
            // SAFETY : `nom` vit jusqu'à la fin de la fonction.
            let code = unsafe { WlanConnect(self.poignee, &self.interface, &parametres, None) };
            if code != 0 {
                return Err(format!("connexion refusée par Windows (code {code})"));
            }
            Ok(())
        }

        /// Le réseau auquel la carte est connectée en ce moment.
        pub fn connecte_a(&self) -> Option<String> {
            let mut taille = 0u32;
            let mut donnees: *mut core::ffi::c_void = std::ptr::null_mut();
            // SAFETY : Windows alloue `donnees`, libéré plus bas.
            let code = unsafe {
                WlanQueryInterface(
                    self.poignee,
                    &self.interface,
                    wlan_intf_opcode_current_connection,
                    None,
                    &mut taille,
                    &mut donnees,
                    None,
                )
            };
            if code != 0 || donnees.is_null() {
                return None;
            }
            // SAFETY : pour ce code, Windows renvoie WLAN_CONNECTION_ATTRIBUTES.
            let resultat = unsafe {
                let attributs = &*(donnees as *const WLAN_CONNECTION_ATTRIBUTES);
                let ssid = &attributs.wlanAssociationAttributes.dot11Ssid;
                let longueur = (ssid.uSSIDLength as usize).min(32);
                (attributs.isState == wlan_interface_state_connected)
                    .then(|| String::from_utf8_lossy(&ssid.ucSSID[..longueur]).to_string())
            };
            unsafe { WlanFreeMemory(donnees as *const _) };
            resultat
        }

        pub fn deconnecter(&self) {
            // SAFETY : poignée et GUID valides.
            unsafe { WlanDisconnect(self.poignee, &self.interface, None) };
        }

        /// Recherche du SEUL réseau nommé, et attend sa fin (3 s au plus).
        pub fn scanner_cible(&self, ssid: &str) {
            use std::sync::atomic::Ordering;
            let octets = ssid.as_bytes();
            let mut cible = DOT11_SSID { uSSIDLength: octets.len().min(32) as u32, ucSSID: [0; 32] };
            cible.ucSSID[..octets.len().min(32)].copy_from_slice(&octets[..octets.len().min(32)]);
            RECHERCHE_FINIE.store(false, Ordering::SeqCst);
            // SAFETY : `cible` vit pendant l'appel ; les autres paramètres sont facultatifs.
            let code = unsafe { WlanScan(self.poignee, &self.interface, Some(&cible as *const DOT11_SSID), None, None) };
            let debut = std::time::Instant::now();
            while code == 0
                && !RECHERCHE_FINIE.load(Ordering::SeqCst)
                && debut.elapsed() < std::time::Duration::from_secs(3)
            {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }

        /// Efface le profil créé pour ce téléphone : un nouveau nom à chaque
        /// envoi, et Windows les garderait tous, des milliers en un an.
        pub fn oublier(&self, ssid: &str) {
            let nom = HSTRING::from(super::nom_profil(ssid));
            // SAFETY : `nom` vit pendant l'appel.
            unsafe { WlanDeleteProfile(self.poignee, &self.interface, &nom, None) };
        }
    }

    impl Drop for Client {
        fn drop(&mut self) {
            // SAFETY : poignée ouverte dans `ouvrir`, fermée une seule fois.
            unsafe { WlanCloseHandle(self.poignee, None) };
        }
    }
}

#[cfg(not(windows))]
mod wlan {
    use super::{Evenement, Reseau};
    pub struct Client;
    impl Client {
        pub fn evenements(&self, _ssid: &str) -> Vec<Evenement> {
            Vec::new()
        }
        pub fn ouvrir() -> Result<Self, String> {
            Err("disponible uniquement sur Windows".to_string())
        }
        pub fn scanner(&self) {}
        pub fn reseaux(&self) -> Result<Vec<Reseau>, u32> {
            Ok(Vec::new())
        }
        pub fn rejoindre(&self, _: &str, _: &str, _: bool) -> Result<(), String> {
            Err("disponible uniquement sur Windows".to_string())
        }
        pub fn connecte_a(&self) -> Option<String> {
            None
        }
        pub fn deconnecter(&self) {}
        pub fn scanner_cible(&self, _ssid: &str) {}
        pub fn oublier(&self, _ssid: &str) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(ssid: &str, signal: u32) -> Reseau {
        Reseau { ssid: ssid.into(), signal, wpa3: false, protege: true, connu: false, securite: String::new() }
    }

    /// Constaté à l'essai : le PC tentait la box Celtiis de la maison.
    #[test]
    fn ne_touche_jamais_un_reseau_deja_connu_du_pc() {
        let mut box_maison = r("Celtiis_bfbd", 99);
        box_maison.connu = true;
        let reseaux = vec![box_maison, r("DIRECT-KQ-K4G7", 80)];
        assert_eq!(choisir(&reseaux, 60, &|_| false, &[], &HashSet::new(), "3FA92C").unwrap().ssid, "DIRECT-KQ-K4G7");
        let mut seul = r("Celtiis_bfbd", 99);
        seul.connu = true;
        assert!(choisir(&[seul], 60, &|_| false, &[], &HashSet::new(), "3FA92C").is_none());
    }

    #[test]
    fn prend_l_envoyeur_le_plus_fort_au_dessus_du_seuil() {
        let reseaux = vec![r("DIRECT-KQ-AAAA", 55), r("DIRECT-KQ-BBBB", 92), r("DIRECT-KQ-CCCC", 71)];
        let choisi = choisir(&reseaux, 60, &|_| false, &[], &HashSet::new(), "3FA92C").unwrap();
        assert_eq!(choisi.ssid, "DIRECT-KQ-BBBB");
    }

    #[test]
    fn ignore_les_faibles_les_ouverts_les_mis_a_l_ecart_et_notre_propre_wifi() {
        let mut ouvert = r("Ouvert", 99);
        ouvert.protege = false;
        ouvert.ssid = "DIRECT-KQ-OUVE".into();
        let reseaux = vec![ouvert, r("DIRECT-KQ-FAIB", 20), r("DIRECT-KQ-DEJA", 95), r("DIRECT-KQ-NOUS", 98)];
        let a_l_ecart = |s: &str| s == "DIRECT-KQ-DEJA";
        assert!(choisir(&reseaux, 60, &a_l_ecart, &["DIRECT-KQ-NOUS".to_string()], &HashSet::new(), "3FA92C").is_none());
    }

    /// Constaté à l'essai : le PC perdait son temps sur les box et partages
    /// du voisinage pendant que le vrai client attendait.
    #[test]
    fn ne_tente_jamais_un_reseau_du_voisinage() {
        let fond: HashSet<String> = ["TECNO SPARK 6 Go", "HUAWEI-5G-2WdT", "CPE_R0516_3F92"]
            .into_iter()
            .map(String::from)
            .collect();
        let voisins = vec![r("TECNO SPARK 6 Go", 95), r("HUAWEI-5G-2WdT", 89), r("CPE_R0516_3F92", 82)];
        assert!(choisir(&voisins, 60, &|_| false, &[], &fond, "3FA92C").is_none());
        // L'envoyeur Android passe toujours, même devant un partage plus fort.
        let reseaux = vec![r("iPhone de Koffi", 95), r("DIRECT-KQ-7H2M", 70)];
        assert_eq!(choisir(&reseaux, 60, &|_| false, &[], &fond, "3FA92C").unwrap().ssid, "DIRECT-KQ-7H2M");
        // Plus aucun partage de connexion tenté, même apparu au guichet :
        // seuls les réseaux de l'application.
        let reseaux = vec![r("TECNO SPARK 6 Go", 95), r("iPhone de Koffi", 80)];
        assert!(choisir(&reseaux, 60, &|_| false, &[], &HashSet::new(), "3FA92C").is_none());
        // Le réseau qui porte le numéro de ce kiosque passe même au signal faible.
        assert_eq!(
            choisir(&[r("DIRECT-KQ-3FA92C-QW2E", 12)], 60, &|_| false, &[], &fond, "3FA92C").unwrap().ssid,
            "DIRECT-KQ-3FA92C-QW2E"
        );
    }

    /// Deux kiosques voisins : chacun ne rejoint que les téléphones qui lui
    /// envoient, et ceux qui n'ont entendu aucune balise (réseau sans numéro).
    #[test]
    fn ignore_les_autres_reseaux_wifi_direct() {
        let reseaux = vec![r("DIRECT-KM-3FA92C", 99), r("DIRECT-5C-HP LaserJet", 95), r("DIRECT-KQ-K4G7", 60)];
        let choisi = choisir(&reseaux, 50, &|_| false, &[], &HashSet::new(), "3FA92C").unwrap();
        assert_eq!(choisi.ssid, "DIRECT-KQ-K4G7");
        let sans_client = vec![r("DIRECT-KM-3FA92C", 99)];
        assert!(choisir(&sans_client, 50, &|_| false, &[], &HashSet::new(), "3FA92C").is_none());
    }

    #[test]
    fn chaque_kiosque_ne_rejoint_que_ses_clients() {
        let reseaux = vec![r("DIRECT-KQ-A1B2C3-XY7K", 95), r("DIRECT-KQ-3FA92C-QW2E", 60)];
        let choisi = choisir(&reseaux, 50, &|_| false, &[], &HashSet::new(), "3FA92C").unwrap();
        assert_eq!(choisi.ssid, "DIRECT-KQ-3FA92C-QW2E");
        let autre = vec![r("DIRECT-KQ-A1B2C3-XY7K", 95)];
        assert!(choisir(&autre, 50, &|_| false, &[], &HashSet::new(), "3FA92C").is_none());
        // Réseau sans numéro (téléphone sans balise) : accepté, mais après le sien.
        let mixte = vec![r("DIRECT-KQ-9DKT", 99), r("DIRECT-KQ-3FA92C-QW2E", 60)];
        assert_eq!(choisir(&mixte, 50, &|_| false, &[], &HashSet::new(), "3FA92C").unwrap().ssid, "DIRECT-KQ-3FA92C-QW2E");
        assert_eq!(choisir(&[r("DIRECT-KQ-9DKT", 80)], 50, &|_| false, &[], &HashSet::new(), "3FA92C").unwrap().ssid, "DIRECT-KQ-9DKT");
    }

    #[test]
    fn numero_et_mot_de_passe_du_kiosque() {
        assert_eq!(kiosque_du_reseau("DIRECT-KQ-3FA92C-QW2E"), Some("3FA92C"));
        assert_eq!(kiosque_du_reseau("DIRECT-KQ-9DKT"), None);
        assert_eq!(kiosque_du_reseau("DIRECT-KQ-3fa92c-QW2E"), None);
        // Même calcul que l'application (Reglages.kt, motDePasseKiosque).
        assert_eq!(mot_de_passe_kiosque("3FA92C"), "b52e4f70d5eddab7");
        let kotlin = include_str!("../../android/app/src/main/java/bj/photocopie/envoyeur/Reglages.kt");
        assert!(kotlin.contains(&format!("SECRET_KIOSQUE = \"{SECRET_KIOSQUE}\"")));
    }

    #[test]
    fn un_reseau_etranger_est_attendu_moins_et_ecarte_plus_longtemps() {
        assert!(attente_connexion("iPhone de Koffi") < attente_connexion("DIRECT-KQ-9DKT"));
        assert!(ecart_apres_echec("iPhone de Koffi") > ecart_apres_echec("DIRECT-KQ-9DKT"));
    }

    #[test]
    fn le_profil_echappe_le_nom_et_reste_manuel() {
        let xml = profil_xml("Awa & <Co>", "kiosque2026", false);
        assert!(xml.contains("<name>Awa &amp; &lt;Co&gt;</name>"));
        assert!(xml.contains("<hex>4177612026203C436F3E</hex>"));
        assert!(xml.contains("<connectionMode>manual</connectionMode>"));
        assert!(xml.contains("</SSID><nonBroadcast>true</nonBroadcast></SSIDConfig>"));
        assert!(xml.contains("WPA2PSK"));
        assert!(profil_xml("x", "12345678", true).contains("WPA3SAE"));
    }

    #[test]
    fn trouve_l_adresse_donnee_par_le_telephone() {
        let avant = vec![Ipv4Addr::new(127, 0, 0, 1), Ipv4Addr::new(192, 168, 73, 1)];
        let apres = vec![
            Ipv4Addr::new(127, 0, 0, 1),
            Ipv4Addr::new(192, 168, 73, 1),
            Ipv4Addr::new(169, 254, 3, 4),
            Ipv4Addr::new(172, 20, 10, 3),
        ];
        assert_eq!(nouvelle_adresse(&avant, &apres), Some(Ipv4Addr::new(172, 20, 10, 3)));
    }

    #[test]
    fn le_qr_de_l_affiche_porte_le_whatsapp_de_la_boutique() {
        assert_eq!(adresse_application(None, None), ADRESSE_APPLICATION);
        assert_eq!(
            adresse_application(Some("01 51 22 67 41"), None),
            format!("{ADRESSE_APPLICATION}?w=2290151226741")
        );
        assert_eq!(
            adresse_application(Some("01 51 22 67 41"), Some(" PC Photocopie-Rapide ")),
            format!("{ADRESSE_APPLICATION}?w=2290151226741&b=PC%20Photocopie-Rapide")
        );
        // Rien d'autre ne passe : ni &, ni guillemets, ni balises.
        assert_eq!(
            adresse_application(None, Some("Kiosque\"&x=<b>é")),
            format!("{ADRESSE_APPLICATION}?b=Kiosquexb%C3%A9")
        );
        assert_eq!(adresse_application(None, Some("   ")), ADRESSE_APPLICATION);
    }

    #[test]
    fn reconnait_le_reseau_wifi_direct_meme_deja_liste() {
        // Essai réel : le téléphone joignait le PC à 192.168.49.96, mais
        // cette adresse ne paraissait pas « nouvelle » au PC.
        let pc = Ipv4Addr::new(192, 168, 49, 96);
        let liste = vec![Ipv4Addr::new(127, 0, 0, 1), Ipv4Addr::new(192, 168, 1, 20), pc];
        assert_eq!(nouvelle_adresse(&liste, &liste), Some(pc));
        // Le téléphone lui-même (.1) n'est jamais l'adresse du PC.
        let tel = vec![Ipv4Addr::new(192, 168, 49, 1)];
        assert_eq!(nouvelle_adresse(&tel, &tel), None);
    }
}
