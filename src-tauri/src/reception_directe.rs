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

use std::collections::{HashMap, VecDeque};
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

/// Sans envoi dans ce délai après la connexion, on libère : le client est
/// peut-être parti, ou ce n'était pas un client.
const ATTENTE_PREMIER_ENVOI: Duration = Duration::from_secs(120);
/// Après le dernier envoi, on laisse ce délai au client pour un autre
/// fichier, puis on libère pour le suivant.
const CALME_APRES_ENVOI: Duration = Duration::from_secs(20);
/// Un réseau refusé (mauvais mot de passe : ce n'est pas un client) ou déjà
/// servi n'est pas retenté pendant ce délai.
const MISE_A_L_ECART: Duration = Duration::from_secs(180);
const PAUSE_ENTRE_TOURS: Duration = Duration::from_secs(4);

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
}

/// Le réseau à rejoindre : protégé, au-dessus du seuil, pas mis à l'écart,
/// et le plus fort. `ignorer` : nos propres réseaux (celui que le PC crée
/// éventuellement) et ceux qu'on ne doit jamais tenter.
pub fn choisir<'a>(
    reseaux: &'a [Reseau],
    seuil: u32,
    a_l_ecart: &dyn Fn(&str) -> bool,
    ignorer: &[String],
) -> Option<&'a Reseau> {
    reseaux
        .iter()
        .filter(|r| r.protege && r.signal >= seuil && !r.ssid.is_empty())
        .filter(|r| !a_l_ecart(&r.ssid) && !ignorer.iter().any(|i| i == &r.ssid))
        .max_by_key(|r| r.signal)
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
pub fn profil_xml(ssid: &str, mot_de_passe: &str, wpa3: bool) -> String {
    let hex: String = ssid.bytes().map(|o| format!("{o:02X}")).collect();
    let authentification = if wpa3 { "WPA3SAE" } else { "WPA2PSK" };
    format!(
        "<?xml version=\"1.0\"?>\
<WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\">\
<name>{nom}</name>\
<SSIDConfig><SSID><hex>{hex}</hex><name>{ssid}</name></SSID></SSIDConfig>\
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

/// L'adresse apparue après la connexion : celle que le téléphone du client
/// a donnée au PC.
pub fn nouvelle_adresse(avant: &[Ipv4Addr], apres: &[Ipv4Addr]) -> Option<Ipv4Addr> {
    apres
        .iter()
        .copied()
        .find(|ip| !avant.contains(ip) && !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified())
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
        active: lire("reception_directe_active").as_deref() == Some("oui"),
        mot_de_passe: lire("reception_directe_mdp")
            .filter(|m| m.chars().count() >= 8)
            .unwrap_or_else(|| MOT_DE_PASSE_PAR_DEFAUT.to_string()),
        seuil: lire("reception_directe_seuil")
            .and_then(|s| s.parse().ok())
            .map(|s: u32| s.clamp(1, 100))
            .unwrap_or(SEUIL_PAR_DEFAUT),
    }
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
}

#[tauri::command]
pub fn reception_directe_qr() -> Result<QrReception, String> {
    let fixe_url = adresse_fixe();
    let direct_url =
        adresse_actuelle().map(|a| format!("http://{a}:{}/", crate::server::PORT));
    Ok(QrReception {
        fixe: crate::qr::build_qr_data_uri(&fixe_url)?,
        direct: direct_url.as_deref().map(crate::qr::build_qr_data_uri).transpose()?,
        fixe_url,
        direct_url,
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
        let mut empeche_veille = false;
        etat("Arrêtée");
        loop {
            let reglages = lire_reglages(&app);
            if reglages.active != empeche_veille {
                garder_eveille(reglages.active);
                empeche_veille = reglages.active;
            }
            if reglages.active {
                un_tour(&app, &reglages, &mut mis_a_l_ecart);
            } else {
                etat("Arrêtée");
            }
            std::thread::sleep(PAUSE_ENTRE_TOURS);
        }
    });
}

/// Empêche le PC de se mettre en veille tout seul tant que la réception
/// directe est active (l'écran, lui, peut s'éteindre). En veille, plus rien
/// ne tourne : aucun client ne pourrait être servi.
#[cfg(windows)]
fn garder_eveille(oui: bool) {
    use windows::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED};
    // SAFETY : appel sans pointeur ; l'état reste attaché à ce fil, qui vit
    // aussi longtemps que l'application.
    unsafe {
        SetThreadExecutionState(if oui { ES_CONTINUOUS | ES_SYSTEM_REQUIRED } else { ES_CONTINUOUS });
    }
}

#[cfg(not(windows))]
fn garder_eveille(_oui: bool) {}

fn un_tour(app: &AppHandle, reglages: &Reglages, mis_a_l_ecart: &mut HashMap<String, Instant>) {
    mis_a_l_ecart.retain(|_, depuis| depuis.elapsed() < MISE_A_L_ECART);

    let client = match wlan::Client::ouvrir() {
        Ok(c) => c,
        Err(e) => {
            etat(format!("Bloquée : {e}"));
            noter(format!("❌ {e}"));
            std::thread::sleep(Duration::from_secs(20));
            return;
        }
    };

    etat("En attente d'un téléphone…");
    client.scanner();
    std::thread::sleep(Duration::from_secs(3));
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
            etat(format!("Bloquée : {message}"));
            noter(format!("❌ {message}"));
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
    let a_l_ecart = |ssid: &str| mis_a_l_ecart.contains_key(ssid);
    let Some(cible) = choisir(&reseaux, reglages.seuil, &a_l_ecart, &ignorer).cloned() else {
        return;
    };

    noter(format!("📶 Téléphone vu : « {} » (signal {} %)", cible.ssid, cible.signal));
    etat(format!("Connexion à « {} »…", cible.ssid));
    let avant = adresses_ipv4();
    if let Err(e) = client.rejoindre(&cible.ssid, &reglages.mot_de_passe, cible.wpa3) {
        noter(format!("❌ Connexion refusée à « {} » : {e}", cible.ssid));
        mis_a_l_ecart.insert(cible.ssid.clone(), Instant::now());
        return;
    }

    // Connexion, puis adresse donnée par le téléphone.
    let debut = Instant::now();
    let mut adresse = None;
    while debut.elapsed() < Duration::from_secs(25) {
        std::thread::sleep(Duration::from_millis(700));
        if client.connecte_a().as_deref() == Some(cible.ssid.as_str()) {
            adresse = nouvelle_adresse(&avant, &adresses_ipv4());
            if adresse.is_some() {
                break;
            }
        }
    }
    let Some(adresse) = adresse else {
        noter(format!(
            "❌ « {} » : pas de connexion en 25 s (mot de passe différent, ou ce n'est pas un client).",
            cible.ssid
        ));
        client.deconnecter();
        mis_a_l_ecart.insert(cible.ssid.clone(), Instant::now());
        return;
    };

    let secondes = debut.elapsed().as_secs_f32();
    if let Ok(mut a) = ADRESSE.lock() {
        *a = Some(adresse);
    }
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
    if let Ok(mut d) = DERNIER_ENVOI.lock() {
        *d = None;
    }

    // Attendre l'envoi, puis un moment de calme.
    let connecte_depuis = Instant::now();
    let raison = loop {
        std::thread::sleep(Duration::from_secs(1));
        if client.connecte_a().as_deref() != Some(cible.ssid.as_str()) {
            break "le téléphone a coupé son partage de connexion";
        }
        let dernier = DERNIER_ENVOI.lock().ok().and_then(|d| *d);
        match dernier {
            Some(t) if t.elapsed() >= CALME_APRES_ENVOI => break "client servi",
            None if connecte_depuis.elapsed() >= ATTENTE_PREMIER_ENVOI => {
                break "aucun envoi en 2 minutes"
            }
            _ => {}
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
    if let Ok(mut a) = ADRESSE.lock() {
        *a = None;
    }
    mis_a_l_ecart.insert(cible.ssid, Instant::now());
}

// ───────────────────────────── API Wi-Fi de Windows ─────────────────────────────

#[cfg(windows)]
mod wlan {
    use super::Reseau;
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::{BOOL, HANDLE};
    use windows::Win32::NetworkManagement::WiFi::*;

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
            Ok(Client { poignee, interface })
        }

        pub fn scanner(&self) {
            // SAFETY : poignée et GUID valides ; les autres paramètres sont facultatifs.
            unsafe { WlanScan(self.poignee, &self.interface, None, None, None) };
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
                    reseaux.push(Reseau {
                        ssid,
                        signal: n.wlanSignalQuality,
                        wpa3: auth == DOT11_AUTH_ALGO_WPA3_SAE,
                        protege: n.bSecurityEnabled.as_bool()
                            && (auth == DOT11_AUTH_ALGO_RSNA_PSK || auth == DOT11_AUTH_ALGO_WPA3_SAE),
                    });
                }
                WlanFreeMemory(liste as *const _);
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
    use super::Reseau;
    pub struct Client;
    impl Client {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(ssid: &str, signal: u32) -> Reseau {
        Reseau { ssid: ssid.into(), signal, wpa3: false, protege: true }
    }

    #[test]
    fn prend_le_telephone_le_plus_fort_au_dessus_du_seuil() {
        let reseaux = vec![r("iPhone de Koffi", 55), r("TECNO SPARK", 92), r("Voisin", 71)];
        let choisi = choisir(&reseaux, 60, &|_| false, &[]).unwrap();
        assert_eq!(choisi.ssid, "TECNO SPARK");
    }

    #[test]
    fn ignore_les_faibles_les_ouverts_les_mis_a_l_ecart_et_notre_propre_wifi() {
        let mut ouvert = r("Ouvert", 99);
        ouvert.protege = false;
        let reseaux = vec![ouvert, r("Faible", 40), r("Deja", 95), r("Photocopie-Awa", 98)];
        let a_l_ecart = |s: &str| s == "Deja";
        assert!(choisir(&reseaux, 60, &a_l_ecart, &["Photocopie-Awa".to_string()]).is_none());
    }

    #[test]
    fn le_profil_echappe_le_nom_et_reste_manuel() {
        let xml = profil_xml("Awa & <Co>", "kiosque2026", false);
        assert!(xml.contains("<name>Awa &amp; &lt;Co&gt;</name>"));
        assert!(xml.contains("<hex>4177612026203C436F3E</hex>"));
        assert!(xml.contains("<connectionMode>manual</connectionMode>"));
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
}
