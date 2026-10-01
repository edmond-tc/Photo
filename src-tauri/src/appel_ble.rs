//! L'appel du téléphone, par Bluetooth : « je suis le réseau DIRECT-KQ-…,
//! viens ».
//!
//! C'est ce qui manquait à la réception directe pour être fiable. Avant,
//! le PC cherchait le téléphone en regardant les Wi-Fi autour, encore et
//! encore : un réseau qui venait de naître n'était souvent pas encore dans
//! la liste de Windows, la recherche prenait 4 secondes, et Windows 11 la
//! refuse sans l'autorisation « Localisation ». Quick Share ne cherche pas :
//! les deux appareils se disent par Bluetooth où se retrouver. Ici pareil.
//!
//! Le téléphone (application Envoyeur) émet un petit signal Bluetooth basse
//! consommation, sans connexion ni appairage : données fabricant 0xFFFF,
//! « KC », le numéro du kiosque (3 octets, zéros s'il ne le connaît pas),
//! puis les 4 lettres qui terminent le nom de son réseau. Le PC l'entend,
//! en déduit le nom exact du réseau et son mot de passe, et le rejoint tout
//! de suite, sans recherche.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Deux lettres qui distinguent l'appel d'un téléphone de la balise du PC
/// (« KQ », voir `balise_ble.rs`).
pub const APPEL: &[u8; 2] = b"KC";

/// Lettres possibles à la fin du nom de réseau (voir `Liaison.kt`).
const LETTRES: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// Un appel plus vieux que ça n'est plus servi : le téléphone a abandonné
/// ou s'est déjà relié. Il répète son appel plusieurs fois par seconde.
const FRAICHEUR: Duration = Duration::from_secs(15);

/// Un téléphone qui ne connaît pas le numéro du kiosque doit être tout près :
/// dans un marché, la cage voisine a peut-être son propre kiosque.
pub const SIGNAL_MINIMUM_SANS_NUMERO: i16 = -80;

/// Ce que le téléphone met dans son appel.
pub fn donnees_appel(kiosque: Option<&str>, suffixe: &str) -> Option<Vec<u8>> {
    if suffixe.len() != 4 || !suffixe.bytes().all(|b| LETTRES.contains(&b)) {
        return None;
    }
    let mut d = APPEL.to_vec();
    match kiosque {
        Some(n) => {
            if n.len() != 6 {
                return None;
            }
            for i in 0..3 {
                d.push(u8::from_str_radix(n.get(i * 2..i * 2 + 2)?, 16).ok()?);
            }
        }
        None => d.extend([0, 0, 0]),
    }
    d.extend(suffixe.bytes());
    Some(d)
}

/// Lit un appel : (numéro du kiosque s'il y en a un, fin du nom du réseau).
pub fn lire_appel(d: &[u8]) -> Option<(Option<String>, String)> {
    if d.len() < 9 || &d[..2] != APPEL {
        return None;
    }
    let suffixe = &d[5..9];
    if !suffixe.iter().all(|b| LETTRES.contains(b)) {
        return None;
    }
    let kiosque = (d[2..5] != [0, 0, 0]).then(|| d[2..5].iter().map(|o| format!("{o:02X}")).collect());
    Some((kiosque, String::from_utf8_lossy(suffixe).to_string()))
}

/// Nom du réseau créé par le téléphone (le même calcul que `Liaison.kt`).
pub fn ssid_de_l_appel(kiosque: Option<&str>, suffixe: &str) -> String {
    match kiosque {
        Some(n) => format!("{}{n}-{suffixe}", crate::reception_directe::PREFIXE_ENVOYEUR),
        None => format!("{}{suffixe}", crate::reception_directe::PREFIXE_ENVOYEUR),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Appel {
    pub ssid: String,
    pub kiosque: Option<String>,
    /// Force du signal, en dBm (−40 : dans la main ; −90 : très loin).
    pub signal: i16,
    pub entendu: Instant,
    /// Mot de passe donné par le téléphone (canal Bluetooth, voir
    /// `canal_bt.rs`) : n'importe quel réseau, même au nom choisi par Android.
    pub mot_de_passe: Option<String>,
    /// Demande faite par le canal Bluetooth relié à CE PC : forcément pour
    /// lui, quel que soit le signal.
    pub direct: bool,
}

static APPELS: Mutex<Vec<Appel>> = Mutex::new(Vec::new());

/// L'oreille Bluetooth tourne-t-elle ? Sinon, la réception directe cherche
/// les téléphones à l'ancienne (liste des Wi-Fi).
static ECOUTE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn ecoute_active() -> bool {
    ECOUTE.load(std::sync::atomic::Ordering::SeqCst)
}

/// Note un appel entendu (le plus fort signal récent l'emporte).
pub fn entendre(donnees: &[u8], signal: i16) {
    let Some((kiosque, suffixe)) = lire_appel(donnees) else { return };
    let ssid = ssid_de_l_appel(kiosque.as_deref(), &suffixe);
    let Ok(mut appels) = APPELS.lock() else { return };
    let nouveau = !appels.iter().any(|a| a.ssid == ssid);
    appels.retain(|a| a.entendu.elapsed() < FRAICHEUR && a.ssid != ssid);
    if appels.iter().any(|a| a.ssid == ssid && a.direct) {
        return;
    }
    appels.push(Appel { ssid: ssid.clone(), kiosque, signal, entendu: Instant::now(), mot_de_passe: None, direct: false });
    drop(appels);
    if nouveau {
        crate::reception_directe::noter(format!("📣 Appel Bluetooth du téléphone « {ssid} » (signal {signal} dBm)."));
    }
}

/// Demande arrivée par le canal Bluetooth : nom ET mot de passe du réseau.
pub fn demande_directe(ssid: &str, mot_de_passe: &str) {
    let Ok(mut appels) = APPELS.lock() else { return };
    appels.retain(|a| a.entendu.elapsed() < FRAICHEUR && a.ssid != ssid);
    appels.push(Appel {
        ssid: ssid.to_string(),
        kiosque: None,
        signal: 0,
        entendu: Instant::now(),
        mot_de_passe: Some(mot_de_passe.to_string()),
        direct: true,
    });
}

/// L'appel à servir maintenant, s'il y en a un : pour CE kiosque (ou sans
/// numéro mais tout près), pas mis à l'écart, le plus fort.
pub fn choisir_appel(
    appels: &[Appel],
    mon_numero: &str,
    a_l_ecart: &dyn Fn(&str) -> bool,
) -> Option<Appel> {
    appels
        .iter()
        .filter(|a| a.entendu.elapsed() < FRAICHEUR)
        .filter(|a| {
            a.direct
                || match &a.kiosque {
                    Some(n) => n == mon_numero,
                    None => a.signal >= SIGNAL_MINIMUM_SANS_NUMERO,
                }
        })
        .filter(|a| !a_l_ecart(&a.ssid))
        .max_by_key(|a| (a.direct, a.kiosque.is_some(), a.signal))
        .cloned()
}

pub fn appel_a_servir(mon_numero: &str, a_l_ecart: &dyn Fn(&str) -> bool) -> Option<Appel> {
    let appels = APPELS.lock().ok()?;
    choisir_appel(&appels, mon_numero, a_l_ecart)
}

/// Attend un appel jusqu'à `delai`. Rend aussitôt dès qu'il y en a un.
pub fn attendre_appel(delai: Duration, mon_numero: &str, a_l_ecart: &dyn Fn(&str) -> bool) -> Option<Appel> {
    let debut = Instant::now();
    loop {
        if let Some(a) = appel_a_servir(mon_numero, a_l_ecart) {
            return Some(a);
        }
        if debut.elapsed() >= delai {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(windows)]
pub fn demarrer() {
    use windows::Devices::Bluetooth::Advertisement::{
        BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher,
        BluetoothLEAdvertisementWatcherStatus as Statut, BluetoothLEScanningMode,
    };
    use windows::Foundation::TypedEventHandler;
    use windows::Storage::Streams::DataReader;

    fn lire(args: &BluetoothLEAdvertisementReceivedEventArgs) -> windows::core::Result<()> {
        let signal = args.RawSignalStrengthInDBm()?;
        let vue = args.Advertisement()?.GetManufacturerDataByCompanyId(crate::balise_ble::FABRICANT)?;
        for i in 0..vue.Size()? {
            let tampon = vue.GetAt(i)?.Data()?;
            let mut octets = vec![0u8; tampon.Length()? as usize];
            DataReader::FromBuffer(&tampon)?.ReadBytes(&mut octets)?;
            entendre(&octets, signal);
        }
        Ok(())
    }

    fn creer() -> windows::core::Result<BluetoothLEAdvertisementWatcher> {
        let oreille = BluetoothLEAdvertisementWatcher::new()?;
        // Actif : Windows écoute plus souvent, l'appel est entendu plus vite.
        oreille.SetScanningMode(BluetoothLEScanningMode::Active)?;
        oreille.Received(&TypedEventHandler::<
            BluetoothLEAdvertisementWatcher,
            BluetoothLEAdvertisementReceivedEventArgs,
        >::new(|_, args| {
            if let Some(args) = args.as_ref() {
                let _ = lire(args);
            }
            Ok(())
        }))?;
        Ok(oreille)
    }

    std::thread::spawn(|| {
        let noter = crate::reception_directe::noter;
        let oreille = match creer() {
            Ok(o) => o,
            Err(e) => {
                noter(format!(
                    "❌ Écoute Bluetooth impossible sur ce PC ({e}) : les téléphones seront cherchés \
                     dans la liste des Wi-Fi (plus lent)."
                ));
                return;
            }
        };
        let mut dernier: Option<Statut> = None;
        loop {
            // Un téléphone envoie par le canal Bluetooth : on lui laisse la radio.
            if crate::canal_bt::canal_ouvert() {
                if oreille.Status().ok() == Some(Statut::Started) {
                    let _ = oreille.Stop();
                }
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
            let statut = oreille.Status().unwrap_or(Statut::Aborted);
            // Bluetooth éteint puis rallumé, pilote réinitialisé : Windows
            // arrête l'écoute (« Aborted ») et ne la reprend jamais seul.
            if matches!(statut, Statut::Created | Statut::Stopped | Statut::Aborted) {
                let _ = oreille.Start();
                std::thread::sleep(Duration::from_secs(1));
            }
            let statut = oreille.Status().unwrap_or(Statut::Aborted);
            ECOUTE.store(statut == Statut::Started, std::sync::atomic::Ordering::SeqCst);
            if dernier != Some(statut) {
                noter(match statut {
                    Statut::Started => "👂 Écoute Bluetooth en marche : le PC entend l'appel des téléphones.".to_string(),
                    Statut::Aborted => {
                        "⚠️ Écoute Bluetooth arrêtée : Bluetooth du PC éteint ? Les téléphones seront \
                         cherchés dans la liste des Wi-Fi (plus lent)."
                            .to_string()
                    }
                    autre => format!("Écoute Bluetooth : état {}.", autre.0),
                });
                dernier = Some(statut);
            }
            std::thread::sleep(Duration::from_secs(5));
        }
    });
}

#[cfg(not(windows))]
pub fn demarrer() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appel_aller_retour() {
        let d = donnees_appel(Some("3FA92C"), "AB2Z").unwrap();
        assert_eq!(d, vec![b'K', b'C', 0x3F, 0xA9, 0x2C, b'A', b'B', b'2', b'Z']);
        assert_eq!(lire_appel(&d), Some((Some("3FA92C".to_string()), "AB2Z".to_string())));
        assert_eq!(ssid_de_l_appel(Some("3FA92C"), "AB2Z"), "DIRECT-KQ-3FA92C-AB2Z");

        let d = donnees_appel(None, "QQQQ").unwrap();
        assert_eq!(lire_appel(&d), Some((None, "QQQQ".to_string())));
        assert_eq!(ssid_de_l_appel(None, "QQQQ"), "DIRECT-KQ-QQQQ");
    }

    #[test]
    fn refuse_ce_qui_n_est_pas_un_appel() {
        // La balise du PC lui-même.
        assert_eq!(lire_appel(b"KQ\x3F\xA9\x2CABCD"), None);
        assert_eq!(lire_appel(b"KC\x00\x00"), None);
        // Lettres impossibles (0, O, 1, I exclus ; minuscules).
        assert_eq!(lire_appel(b"KC\x00\x00\x00AB0D"), None);
        assert_eq!(lire_appel(b"KC\x00\x00\x00abcd"), None);
        assert_eq!(donnees_appel(None, "ABC"), None);
        assert_eq!(donnees_appel(Some("XYZ"), "ABCD"), None);
    }

    #[test]
    fn choisit_le_bon_appel() {
        let a = |ssid: &str, kiosque: Option<&str>, signal: i16| Appel {
            ssid: ssid.to_string(),
            kiosque: kiosque.map(String::from),
            signal,
            entendu: Instant::now(),
            mot_de_passe: None,
            direct: false,
        };
        let jamais = |_: &str| false;
        // Le kiosque voisin n'est jamais servi, même tout près.
        assert_eq!(choisir_appel(&[a("v", Some("111111"), -30)], "3FA92C", &jamais), None);
        // Le sien, même au signal faible (PC à petite antenne).
        assert_eq!(choisir_appel(&[a("m", Some("3FA92C"), -95)], "3FA92C", &jamais).unwrap().ssid, "m");
        // Sans numéro : seulement tout près.
        assert_eq!(choisir_appel(&[a("s", None, -90)], "3FA92C", &jamais), None);
        assert_eq!(choisir_appel(&[a("s", None, -60)], "3FA92C", &jamais).unwrap().ssid, "s");
        // Le numéroté passe avant, puis le plus fort.
        let liste = [a("s", None, -40), a("m", Some("3FA92C"), -70)];
        assert_eq!(choisir_appel(&liste, "3FA92C", &jamais).unwrap().ssid, "m");
        // Mis à l'écart : ignoré.
        assert_eq!(choisir_appel(&liste, "3FA92C", &|s: &str| s == "m").unwrap().ssid, "s");
        // Trop vieux : ignoré.
        let mut vieux = a("m", Some("3FA92C"), -50);
        vieux.entendu = Instant::now() - Duration::from_secs(60);
        assert_eq!(choisir_appel(&[vieux], "3FA92C", &jamais), None);
        // Demande par le canal Bluetooth : servie d'abord, même sans signal.
        let mut direct = a("AndroidShare_1234", None, 0);
        direct.direct = true;
        let liste = [a("m", Some("3FA92C"), -40), direct];
        assert_eq!(choisir_appel(&liste, "3FA92C", &jamais).unwrap().ssid, "AndroidShare_1234");
    }

    /// Doit rester identique à `Reglages.kt` de l'envoyeur Android : sinon le
    /// PC n'entend plus l'appel, sans aucune erreur visible.
    #[test]
    fn meme_appel_que_l_envoyeur_android() {
        let kotlin = include_str!("../../android/app/src/main/java/bj/photocopie/envoyeur/Reglages.kt");
        assert!(kotlin.contains("byteArrayOf('K'.code.toByte(), 'C'.code.toByte())"));
        let liaison = include_str!("../../android/app/src/main/java/bj/photocopie/envoyeur/Liaison.kt");
        assert!(liaison.contains(&format!("\"{}\"", std::str::from_utf8(LETTRES).unwrap())));
    }
}
