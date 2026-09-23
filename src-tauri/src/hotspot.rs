//! Point d'accès Wi-Fi local, sans passer par le "Point d'accès mobile" des
//! paramètres Windows — celui-ci refuse de s'activer si le PC n'a AUCUNE
//! connexion (Ethernet, Wi-Fi, données) à partager, ce qui est précisément
//! le cas normal d'une boutique 100% hors ligne. Vérifié sur le terrain
//! (HP ProBook, septembre 2026) : "Nous ne pouvons pas configurer de point
//! d'accès sans fil mobile, car votre ordinateur ne possède aucune connexion
//! Ethernet, Wi-Fi ou de données cellulaires."
//!
//! `netsh wlan hostednetwork` est l'API Windows plus ancienne qui a
//! remplacé ça : elle crée un point d'accès Wi-Fi autonome (la carte Wi-Fi
//! sert de borne), sans jamais exiger de connexion internet à partager.
//! Contrepartie : elle exige les droits administrateur, d'où la demande
//! d'autorisation Windows (UAC) à chaque activation.
//!
//! Ne s'arrête pas à la création du réseau : un téléphone qui s'y connecte
//! a aussi besoin d'une adresse IP (voir `dhcp.rs`) et, pour que la page
//! d'envoi s'ouvre toute seule, d'un serveur DNS qui l'amène jusqu'à elle
//! (voir `dns.rs`). Les deux visent une adresse FIXE et connue à l'avance
//! (`ADRESSE_POINT_ACCES`) — pas question de la deviner : c'est cette même
//! valeur qu'on assigne ici à la carte Wi-Fi.

use std::net::Ipv4Addr;
use std::sync::Mutex;
use tauri::async_runtime::JoinHandle;

/// Adresse fixe du PC sur son propre point d'accès Wi-Fi local. Choisie en
/// dehors de la plage 192.168.137.0/24 qu'utilise le "Point d'accès mobile"
/// classique de Windows, pour ne jamais se marcher dessus si le gérant
/// bascule de l'un à l'autre.
pub const ADRESSE_POINT_ACCES: Ipv4Addr = Ipv4Addr::new(192, 168, 73, 1);

/// Les deux serveurs (DHCP, DNS) tournent tant que le point d'accès est
/// actif : leurs tâches sont gardées ici pour pouvoir les arrêter net à la
/// désactivation, plutôt que de les laisser tourner indéfiniment en fond
/// après que le Wi-Fi lui-même a été coupé.
/// DHCP et DNS peuvent chacun réussir ou échouer à s'installer
/// indépendamment (Windows peut occuper l'un des deux ports sans l'autre) :
/// une liste plutôt qu'un couple figé, pour ne garder que les tâches qui
/// ont réellement démarré.
#[derive(Default)]
pub struct EtatPointAcces(pub Mutex<Vec<JoinHandle<()>>>);

/// L'adresse EFFECTIVEMENT en service, quand un point d'accès tourne.
///
/// Trouvé sur le terrain : `adresse_locale()` (server.rs) et le choix de
/// l'adresse Wi-Fi Direct devinaient chacun de leur côté, par des chemins
/// différents, quelle adresse utiliser — sur un PC réel, la première a
/// renvoyé `10.10.10.1` (une carte VPN ou virtuelle sans rapport) pendant
/// que le vrai réseau Wi-Fi Direct tournait ailleurs. Le QR affichait une
/// adresse à laquelle aucun téléphone ne pouvait jamais arriver.
///
/// Cette variable est désormais la SEULE source : elle est écrite une fois,
/// au moment où `activer_par_tous_les_moyens` détermine l'adresse réelle du
/// réseau qu'il vient de créer, et relue partout ailleurs (QR, redirection
/// du portail captif, statut affiché) — plutôt que d'avoir plusieurs
/// tentatives de deviner qui peuvent se contredire.
static ADRESSE_ACTIVE: Mutex<Option<Ipv4Addr>> = Mutex::new(None);

/// L'adresse du point d'accès actuellement actif, ou `None` si aucun des
/// deux n'a été activé par l'application (PC directement sur le réseau de
/// la boutique, ou Wi-Fi non configuré).
pub fn adresse_point_acces_active() -> Option<Ipv4Addr> {
    ADRESSE_ACTIVE.lock().ok().and_then(|garde| *garde)
}

/// Enregistre l'adresse en service — pour un réseau que ce module n'a pas
/// lui-même créé. Utilisé par `routeur_externe.rs` : un routeur dédié crée
/// son propre réseau indépendamment de nous, mais `adresse_locale()`
/// (server.rs) et le QR ont quand même besoin de savoir, par la même source
/// unique que pour nos deux méthodes, quelle adresse est en service.
pub fn definir_adresse_active(adresse: Option<Ipv4Addr>) {
    if let Ok(mut garde) = ADRESSE_ACTIVE.lock() {
        *garde = adresse;
    }
}

/// Nom de la tâche Windows qui rallume le Wi-Fi de la boutique à chaque
/// démarrage du PC. Fixe : la recréer sous le même nom remplace l'ancienne
/// au lieu d'en empiler une seconde.
pub const NOM_TACHE_DEMARRAGE: &str = "Photocopie Benin - Wi-Fi boutique";

/// Ce que la tâche exécute à l'ouverture de session.
///
/// Le réseau hébergé de Windows ne survit pas à un redémarrage : sans cette
/// tâche, le gérant doit rappuyer sur « Activer le Wi-Fi local » chaque
/// matin — et s'il oublie, le premier client de la journée repart avec ses
/// documents sous le bras, sans que personne ne comprenne pourquoi.
///
/// La tâche ne refait PAS la configuration du réseau : Windows garde le nom
/// et le mot de passe de son côté depuis l'activation. Elle se contente de
/// le rallumer et de lui redonner son adresse fixe — la même que partout
/// ailleurs dans ce module, jamais devinée.
///
/// Tout y est silencieux : si le réseau ne démarre pas, rien ne doit
/// s'afficher au visage du gérant à l'ouverture de sa session. Le bouton
/// reste là, et lui dira ce qui ne va pas.
const SCRIPT_DEMARRAGE: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
netsh wlan start hostednetwork | Out-Null
$adaptateur = Get-NetAdapter | Where-Object { $_.InterfaceDescription -like '*Hosted Network Virtual Adapter*' } | Select-Object -First 1
if ($adaptateur) {
    Remove-NetIPAddress -InterfaceIndex $adaptateur.InterfaceIndex -Confirm:$false -ErrorAction SilentlyContinue
    New-NetIPAddress -InterfaceIndex $adaptateur.InterfaceIndex -IPAddress "__ADRESSE__" -PrefixLength 24 -ErrorAction SilentlyContinue | Out-Null
}
"#;

/// Écrit le script de démarrage à un endroit stable, et rend son chemin.
///
/// Dans le dossier de l'utilisateur et non dans `%TEMP%` : Windows efface le
/// contenu de `%TEMP%`, et une tâche qui pointe vers un fichier disparu
/// échoue en silence tous les matins.
#[cfg(windows)]
fn ecrire_script_demarrage() -> Result<std::path::PathBuf, String> {
    let dossier = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("photocopie-benin");
    std::fs::create_dir_all(&dossier)
        .map_err(|e| format!("Impossible de préparer le démarrage automatique ({e})."))?;

    let fichier = dossier.join("demarrage-wifi.ps1");
    let contenu = SCRIPT_DEMARRAGE.replace("__ADRESSE__", &ADRESSE_POINT_ACCES.to_string());
    // Même marque UTF-8 que le script d'activation, pour la même raison.
    std::fs::write(&fichier, format!("\u{FEFF}{contenu}"))
        .map_err(|e| format!("Impossible d'écrire le démarrage automatique ({e})."))?;
    Ok(fichier)
}

/// Mot de passe posé d'office sur une installation neuve. Huit caractères
/// au minimum : c'est la règle du Wi-Fi lui-même, pas la nôtre, et Windows
/// refuse le réseau en dessous avec une erreur incompréhensible.
///
/// Il n'a pas à être secret : le QR que le client scanne le contient, par
/// construction. Ce qui compte, c'est que le gérant puisse le lire dans
/// Réglages et le changer s'il le souhaite.
pub const MOT_DE_PASSE_PAR_DEFAUT: &str = "photocopie";

/// Nom de réseau posé d'office sur une installation neuve, tiré du nom de la
/// boutique quand il y en a un — le client reconnaît alors l'endroit où il
/// se trouve dans la liste des Wi-Fi de son téléphone.
///
/// Réduit à ce qu'un SSID accepte sans histoires : lettres, chiffres et
/// tirets, sans accent (tous les téléphones ne les affichent pas pareil), et
/// 32 caractères au plus, la limite de la norme Wi-Fi.
pub fn nom_reseau_par_defaut(nom_boutique: Option<&str>) -> String {
    let propre: String = nom_boutique
        .unwrap_or("")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|morceau| !morceau.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    if propre.is_empty() {
        return "PHOTOCOPIE".to_string();
    }
    propre.chars().take(32).collect()
}

/// Ce que ce PC-ci sait faire, tel que Windows le déclare. Établi SANS
/// demander les droits administrateur et sans rien activer : le gérant (ou
/// le revendeur, avant même une vente) peut donc le lancer sur n'importe
/// quel PC en quelques secondes.
///
/// Le but n'est pas d'afficher des capacités techniques, mais de répondre à
/// la seule question qui se pose en boutique : **sur CE poste, qu'est-ce que
/// je fais ?** D'où le `verdict`, écrit pour être lu tel quel et suivi sans
/// rien connaître du sujet.
#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct DiagnosticPoste {
    /// `None` quand Windows n'a pas répondu du tout (netsh absent, bloqué…).
    pub carte_wifi_presente: Option<bool>,
    /// Support du point d'accès autonome (`netsh wlan hostednetwork`), la
    /// méthode qu'utilise `activer`. `None` = Windows ne l'indique pas (les
    /// versions récentes ont retiré cette ligne).
    pub reseau_heberge_supporte: Option<bool>,
    /// Support du rôle "propriétaire de groupe" Wi-Fi Direct, celui qui crée
    /// le point d'accès dans la méthode 2 (voir `wifi_direct.rs`). C'est la
    /// capacité que l'exemple officiel Microsoft dit de vérifier avant
    /// d'utiliser cette API.
    pub wifi_direct_go_supporte: Option<bool>,
    /// Le pilote Wi-Fi expose-t-il l'interface WDI ? C'est le modèle de
    /// pilote que Microsoft a imposé à partir de Windows 10 et sur lequel
    /// reposent les API Wi-Fi Direct modernes (méthode 2, voir
    /// `wifi_direct.rs`). Un pilote plus ancien (constaté sur le terrain :
    /// Broadcom 802.11n daté de 2011) répond ici "interface WDI non prise en
    /// charge" : ses seules capacités réellement utilisables sont les
    /// anciennes, donc le réseau hébergé — la méthode 1.
    ///
    /// Compte double ici : ce même défaut explique pourquoi les lignes
    /// "Wi-Fi Direct : pris en charge" de `wirelesscapabilities` peuvent
    /// annoncer des capacités que le pilote ne tient pas.
    pub wdi_supporte: Option<bool>,
    /// Ce PC a-t-il une radio Bluetooth ? Wi-Fi et Bluetooth partageant la
    /// même puce sur l'immense majorité des machines, un PC sans Wi-Fi n'en
    /// a presque jamais — c'est justement ce qu'il faut vérifier plutôt que
    /// de le supposer.
    pub bluetooth_present: Option<bool>,
    /// Ce PC est-il déjà joignable sur un réseau (câble Ethernet vers la box
    /// de la boutique, ou Wi-Fi existant) ? Si oui, le téléphone du client
    /// peut l'atteindre directement : c'est le parcours le plus simple, sans
    /// point d'accès ni portail captif.
    pub reseau_utilisable: bool,
    /// Phrase directement affichable au gérant.
    pub verdict: String,
    /// Sortie brute de Windows, à copier/transmettre au support : c'est elle
    /// qui fait foi quand l'analyse automatique ci-dessus reste indécise.
    pub details_bruts: String,
}

/// Rend une sortie de `netsh` comparable quelle que soit la langue ET
/// l'encodage de Windows : la console française renvoie du CP850, pas de
/// l'UTF-8, donc les accents arrivent ici en caractères de remplacement.
/// En ne gardant que l'ASCII, "réseau hébergé" et "r?seau h?berg?" se
/// réduisent tous deux à "rseau hberg", sur quoi on peut chercher.
fn normaliser(sortie: &str) -> String {
    sortie
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii())
        .collect()
}

/// Cherche la ligne "Prise en charge du réseau hébergé / Hosted network
/// supported" et en lit la valeur (Oui/Non/Yes/No).
fn lire_prise_en_charge_reseau_heberge(sortie: &str) -> Option<bool> {
    let normalisee = normaliser(sortie);
    let ligne = normalisee.lines().find(|ligne| {
        ligne.contains("hosted network") || ligne.contains("rseau hberg")
    })?;
    let valeur = ligne.rsplit(':').next()?.trim();
    if valeur.contains("oui") || valeur.contains("yes") {
        Some(true)
    } else if valeur.contains("non") || valeur.contains("no") {
        Some(false)
    } else {
        None
    }
}

/// Lit la ligne "Wi-Fi Direct GO / GO Wi-Fi Direct" de
/// `netsh wlan show wirelesscapabilities`.
fn lire_prise_en_charge_wifi_direct_go(sortie: &str) -> Option<bool> {
    let normalisee = normaliser(sortie);
    let ligne = normalisee
        .lines()
        .find(|ligne| ligne.contains("wi-fi direct go") || ligne.contains("go wi-fi direct"))?;
    let valeur = ligne.rsplit(':').next()?.trim();
    // "not supported" contient "supported", et "non pris en charge" contient
    // "pris en charge" : la négation doit donc être testée en premier, sans
    // quoi tout PC incapable serait déclaré capable.
    if valeur.contains("not supported") || valeur.contains("non pris en charge") {
        Some(false)
    } else if valeur.contains("supported") || valeur.contains("pris en charge") {
        Some(true)
    } else {
        None
    }
}

/// Lit la ligne "Version WDI (fabricant de matériel)" / "WDI Version (IHV)"
/// de `netsh wlan show wirelesscapabilities`.
///
/// Windows y écrit soit un numéro de version (pilote WDI moderne), soit
/// "interface WDI non prise en charge" / "WDI not supported" (pilote
/// ancien). Cette ligne est la plus fiable des trois : contrairement aux
/// lignes "Wi-Fi Direct ... : pris en charge", elle décrit le pilote
/// lui-même et non des capacités annoncées.
fn lire_prise_en_charge_wdi(sortie: &str) -> Option<bool> {
    let normalisee = normaliser(sortie);
    let ligne = normalisee.lines().find(|ligne| ligne.contains("wdi"))?;
    let valeur = ligne.rsplit(':').next()?.trim().to_string();
    // La négation d'abord : "non prise en charge" contient "pris en charge",
    // et "not supported" contient "supported".
    if valeur.contains("non pris") || valeur.contains("not supported") {
        Some(false)
    } else if valeur.chars().any(|c| c.is_ascii_digit()) {
        // Un numéro de version : le pilote expose bien l'interface WDI.
        Some(true)
    } else {
        None
    }
}

/// `netsh wlan show interfaces` annonce explicitement l'absence de carte
/// sans fil ; toute autre réponse non vide signifie qu'il y en a une.
fn lire_presence_carte_wifi(sortie: &str) -> Option<bool> {
    let normalisee = normaliser(sortie);
    if normalisee.trim().is_empty() {
        return None;
    }
    if normalisee.contains("no wireless interface")
        || normalisee.contains("aucune interface sans fil")
    {
        return Some(false);
    }
    Some(true)
}

/// L'ancienne détection se contentait de regarder si UNE adresse IP
/// "ressemblait" à un réseau (ni boucle locale, ni 169.254.x.x, ni notre
/// propre point d'accès). Trouvé sur le terrain : ça s'est trompé sur un PC
/// sans aucun routeur ni box branché — le diagnostic a dit "ce PC est déjà
/// sur un réseau" alors que rien n'était branché, très probablement à cause
/// d'un adaptateur fantôme (VPN, machine virtuelle, commutateur virtuel)
/// dont l'adresse ressemble à un vrai réseau sans jamais mener nulle part.
///
/// Le bon signal, c'est la présence d'une PASSERELLE PAR DÉFAUT : un vrai
/// routeur ou une vraie box en annonce toujours une (c'est elle qui rend
/// le réseau "utilisable" au sens où un appareil peut en sortir), alors
/// qu'un adaptateur fantôme n'en a généralement aucune.
///
/// Renforcé après un second faux positif signalé sur le terrain : certains
/// adaptateurs fantômes (VPN, machine virtuelle) conservent bel et bien une
/// passerelle mémorisée. On n'accepte donc désormais que les passerelles
/// portées par une carte dont Windows dit qu'un câble ou un Wi-Fi est
/// RÉELLEMENT branché dessus (`MediaConnectionState = Connected`).
fn a_une_passerelle_valide(sortie: &str) -> bool {
    sortie.lines().any(|ligne| {
        ligne
            .trim()
            .parse::<Ipv4Addr>()
            .is_ok_and(|ip| !ip.is_unspecified())
    })
}

/// Ce PC est-il déjà sur un réseau que le téléphone d'un client pourrait
/// rejoindre ? Voir `a_une_passerelle_valide` pour le signal utilisé.
#[cfg(windows)]
fn reseau_utilisable() -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            "(Get-NetRoute -DestinationPrefix '0.0.0.0/0' -ErrorAction SilentlyContinue | \
              Where-Object { (Get-NetAdapter -InterfaceIndex $_.InterfaceIndex \
              -ErrorAction SilentlyContinue).MediaConnectionState -eq 'Connected' }).NextHop",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|sortie| a_une_passerelle_valide(&String::from_utf8_lossy(&sortie.stdout)))
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn reseau_utilisable() -> bool {
    false
}

/// Traduit les capacités constatées en une consigne que le gérant peut
/// suivre tel quel, sans rien connaître au sujet.
///
/// L'ordre n'est pas celui des capacités techniques mais celui du confort du
/// client : un PC déjà sur le réseau de la boutique offre le parcours le
/// plus simple qui existe (un scan, la page s'ouvre dans le vrai
/// navigateur), donc il passe avant la création d'un point d'accès, qui
/// impose au téléphone de changer de réseau puis de passer par le portail
/// captif.
fn composer_verdict(
    carte_wifi_presente: Option<bool>,
    reseau_heberge_supporte: Option<bool>,
    wifi_direct_go_supporte: Option<bool>,
    wdi_supporte: Option<bool>,
    bluetooth_present: Option<bool>,
    reseau_utilisable: bool,
) -> String {
    // La méthode 2 passe par les API Wi-Fi Direct de Windows, qui s'appuient
    // sur le modèle de pilote WDI : un pilote qui ne l'expose pas a beau
    // annoncer "Wi-Fi Direct : pris en charge", il ne tiendra pas la
    // promesse. Ne pas en tenir compte ici reviendrait à promettre au gérant
    // un Wi-Fi que ce PC ne créera jamais par cette voie.
    let wifi_direct_utilisable =
        wifi_direct_go_supporte != Some(false) && wdi_supporte != Some(false);
    // Une seule des deux méthodes suffit à créer le réseau : exiger les deux
    // déclarerait incapable un PC parfaitement capable.
    let sait_creer_un_wifi = carte_wifi_presente != Some(false)
        && (reseau_heberge_supporte != Some(false) || wifi_direct_utilisable);

    if reseau_utilisable {
        let complement = if sait_creer_un_wifi {
            " Ce PC sait aussi créer son propre Wi-Fi, mais ce n'est pas nécessaire ici."
        } else {
            ""
        };
        return format!(
            "✅ Ce PC est déjà sur un réseau. Montrez simplement le QR : la page d'envoi \
             s'ouvrira directement sur le téléphone du client, à condition qu'il soit connecté \
             au même Wi-Fi (celui de la box ou du routeur de la boutique). C'est le cas le plus \
             simple, rien d'autre à faire.{complement}"
        );
    }

    if sait_creer_un_wifi {
        return "✅ Ce PC sait créer le Wi-Fi de la boutique. Appuyez sur « Activer le Wi-Fi \
                local de la boutique », puis montrez le QR au client."
            .to_string();
    }

    if bluetooth_present == Some(true) {
        return "⚠️ Ce PC ne peut pas créer de Wi-Fi et n'est branché à aucun réseau, mais il a \
                le Bluetooth. Les clients Android peuvent envoyer par Bluetooth (les iPhone ne \
                le peuvent pas). Mieux : branchez un câble réseau entre ce PC et la box de la \
                boutique, et tout redevient simple."
            .to_string();
    }

    "❌ Ce PC n'a ni Wi-Fi utilisable, ni réseau branché, ni Bluetooth. L'envoi par QR est \
     impossible en l'état. Deux solutions gratuites : brancher un câble réseau entre ce PC et \
     la box de la boutique, ou passer par une clé USB. Sinon, une clé Wi-Fi USB règle le \
     problème définitivement."
        .to_string()
}

#[cfg(windows)]
fn executer_netsh(arguments: &[&str]) -> String {
    use std::os::windows::process::CommandExt;
    // Sans ce drapeau, chaque appel fait clignoter une fenêtre noire de
    // console devant le gérant.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    std::process::Command::new("netsh")
        .args(arguments)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|sortie| {
            let mut texte = String::from_utf8_lossy(&sortie.stdout).into_owned();
            texte.push_str(&String::from_utf8_lossy(&sortie.stderr));
            texte
        })
        .unwrap_or_default()
}

/// Présence d'une radio Bluetooth. `None` si Windows ne sait pas répondre,
/// ce qui ne vaut pas "absente".
#[cfg(windows)]
fn bluetooth_present() -> Option<bool> {
    use windows::Devices::Bluetooth::BluetoothAdapter;

    let Ok(operation) = BluetoothAdapter::GetDefaultAsync() else {
        // L'API elle-même est absente : on ne sait pas, et "on ne sait pas"
        // ne doit pas être présenté comme "il n'y en a pas".
        return None;
    };

    // Sans radio, Windows ne rend pas d'erreur mais un adaptateur vide, dont
    // l'adresse matérielle vaut zéro. C'est cette adresse qui tranche.
    match operation.get() {
        Ok(adaptateur) => Some(adaptateur.BluetoothAddress().unwrap_or(0) != 0),
        Err(_) => Some(false),
    }
}

/// Interroge Windows sur les capacités de CE PC. Aucun droit
/// administrateur, aucune activation, aucun effet de bord.
#[cfg(windows)]
pub fn diagnostiquer() -> DiagnosticPoste {
    let pilotes = executer_netsh(&["wlan", "show", "drivers"]);
    let interfaces = executer_netsh(&["wlan", "show", "interfaces"]);
    let capacites = executer_netsh(&["wlan", "show", "wirelesscapabilities"]);

    let carte_wifi_presente = lire_presence_carte_wifi(&interfaces);
    let reseau_heberge_supporte = lire_prise_en_charge_reseau_heberge(&pilotes);
    let wifi_direct_go_supporte = lire_prise_en_charge_wifi_direct_go(&capacites);
    let wdi_supporte = lire_prise_en_charge_wdi(&capacites);
    let bluetooth_present = bluetooth_present();
    let reseau_utilisable = reseau_utilisable();

    DiagnosticPoste {
        carte_wifi_presente,
        reseau_heberge_supporte,
        wifi_direct_go_supporte,
        wdi_supporte,
        bluetooth_present,
        reseau_utilisable,
        verdict: composer_verdict(
            carte_wifi_presente,
            reseau_heberge_supporte,
            wifi_direct_go_supporte,
            wdi_supporte,
            bluetooth_present,
            reseau_utilisable,
        ),
        details_bruts: format!(
            "--- pare-feu Windows ---\nRègles de réception en place : {}\
             \n--- netsh wlan show interfaces ---\n{}\n--- netsh wlan show drivers ---\n{}\
             \n--- netsh wlan show wirelesscapabilities ---\n{}",
            if crate::pare_feu::regles_presentes() { "oui" } else { "non (créées à l'activation)" },
            interfaces.trim(),
            pilotes.trim(),
            capacites.trim()
        ),
    }
}

#[cfg(not(windows))]
pub fn diagnostiquer() -> DiagnosticPoste {
    DiagnosticPoste {
        carte_wifi_presente: None,
        reseau_heberge_supporte: None,
        wifi_direct_go_supporte: None,
        wdi_supporte: None,
        bluetooth_present: None,
        reseau_utilisable: false,
        verdict: "Diagnostic disponible uniquement sur Windows.".to_string(),
        details_bruts: String::new(),
    }
}

#[cfg(windows)]
fn echapper_powershell(valeur: &str) -> String {
    // Dans une chaîne PowerShell entre guillemets doubles, ces trois
    // caractères ont un sens spécial (interpolation de variable, échappement,
    // fin de chaîne) : un SSID ou mot de passe qui les contient casserait
    // sinon le script généré.
    valeur
        .replace('`', "``")
        .replace('$', "`$")
        .replace('"', "`\"")
}

#[cfg(windows)]
pub fn executer_script_eleve(script: &str) -> Result<String, String> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{WaitForSingleObject, INFINITE};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

    let dossier_temp = std::env::temp_dir();
    let fichier_script = dossier_temp.join("photocopie-benin-hotspot.ps1");
    let fichier_resultat = dossier_temp.join("photocopie-benin-hotspot-resultat.txt");
    let _ = std::fs::remove_file(&fichier_resultat);

    // PowerShell 5.1 (celui de Windows 10) lit un .ps1 SANS marque d'ordre
    // des octets comme de l'ANSI, pas de l'UTF-8 : un SSID ou un mot de
    // passe accentué y arriverait déformé, et le Wi-Fi créé porterait un
    // autre nom que celui affiché dans le QR. La marque (BOM) lève
    // l'ambiguïté.
    let mut contenu = String::from("\u{FEFF}");
    contenu.push_str(script);
    std::fs::write(&fichier_script, contenu)
        .map_err(|e| format!("Impossible de préparer la commande ({e})."))?;

    let parametres = format!(
        "-NoProfile -ExecutionPolicy Bypass -File \"{}\"",
        fichier_script.display()
    );

    let verbe = HSTRING::from("runas");
    let fichier = HSTRING::from("powershell.exe");
    let parametres_h = HSTRING::from(parametres);

    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: windows::Win32::UI::Shell::SEE_MASK_NOCLOSEPROCESS,
        lpVerb: PCWSTR(verbe.as_ptr()),
        lpFile: PCWSTR(fichier.as_ptr()),
        lpParameters: PCWSTR(parametres_h.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };

    // SAFETY : appel FFI standard vers l'API Shell de Windows. Les HSTRING
    // référencées par `info` restent en vie jusqu'à la fin de la fonction,
    // après l'appel bloquant (WaitForSingleObject) qui les utilise encore.
    let lance = unsafe { ShellExecuteExW(&mut info) };
    if lance.is_err() || info.hProcess.is_invalid() {
        let _ = std::fs::remove_file(&fichier_script);
        return Err(
            "La demande d'autorisation administrateur a été refusée ou annulée. \
             Réessayez et cliquez \"Oui\" sur la fenêtre Windows qui apparaît."
                .to_string(),
        );
    }

    // SAFETY : hProcess vient d'être obtenu valide ci-dessus (SEE_MASK_NOCLOSEPROCESS),
    // et est fermé juste après l'attente, une seule fois.
    unsafe {
        WaitForSingleObject(info.hProcess, INFINITE);
        let _ = CloseHandle(info.hProcess);
    }

    // LE bug qui a fait échouer la méthode 1 pendant tout le projet, et qui
    // ne se voyait pas parce qu'il ne laissait aucune trace.
    //
    // `ShellExecuteExW` rend la main dès que le processus est CRÉÉ — pas
    // quand il a fini, ni même quand il a ouvert son fichier. Le script
    // était effacé juste après cette ligne, donc pendant que PowerShell
    // démarrait encore. PowerShell trouvait alors un `-File` qui n'existait
    // plus, s'arrêtait sans rien écrire, et l'application n'avait pour seule
    // information qu'un fichier de résultat vide : « Windows n'a rien
    // répondu. Vérifiez que vous avez bien cliqué Oui ». Message
    // doublement trompeur, puisque le gérant AVAIT cliqué "Oui" et que la
    // commande n'avait jamais été lue.
    //
    // C'est une course : selon la vitesse du disque et le temps passé sur la
    // fenêtre d'autorisation, la suppression gagnait ou perdait. D'où des
    // échecs qui semblaient aléatoires, et une méthode 1 réputée
    // "incompatible avec ce PC" alors qu'elle n'avait jamais été exécutée.
    //
    // Le fichier n'est donc effacé qu'ICI, une fois le processus terminé.
    let _ = std::fs::remove_file(&fichier_script);

    let resultat = std::fs::read_to_string(&fichier_resultat).unwrap_or_default();
    let _ = std::fs::remove_file(&fichier_resultat);
    Ok(resultat)
}

/// Délimitent, dans la sortie du script élevé, la réponse de la SEULE
/// commande qui dise si le réseau existe vraiment (`netsh wlan start
/// hostednetwork`). Le script en exécute désormais plusieurs autres avant
/// elle (arrêt, remise à zéro, réactivation de la carte virtuelle) : sans
/// ces marqueurs, leurs messages se mélangeraient au sien, et c'est
/// justement le sien qu'il faut lire mot pour mot pour réparer.
const MARQUEUR_DEBUT_DEMARRAGE: &str = "===DEBUT_DEMARRAGE===";
const MARQUEUR_FIN_DEMARRAGE: &str = "===FIN_DEMARRAGE===";

/// Isole ce qui se trouve entre deux marqueurs. Un script interrompu en
/// plein milieu n'écrit pas le marqueur de fin : on rend alors tout ce qui
/// suit le début, plutôt que rien — c'est précisément dans ce cas que le
/// message manquant compte le plus.
fn extraire_section<'a>(sortie: &'a str, debut: &str, fin: &str) -> Option<&'a str> {
    let apres = sortie.split_once(debut)?.1;
    Some(match apres.split_once(fin) {
        Some((interieur, _)) => interieur,
        None => apres,
    })
}

/// Script complet : crée le point d'accès Wi-Fi, PUIS retrouve la carte
/// virtuelle que Windows vient de créer pour lui assigner une adresse fixe
/// et connue (`ADRESSE_POINT_ACCES`) — sans quoi le serveur DHCP (voir
/// `dhcp.rs`) annoncerait une adresse que la carte n'a pas vraiment, et
/// aucun téléphone ne pourrait jamais la joindre.
///
/// Ne se contente plus de lancer la commande : il remet d'abord le réseau
/// hébergé à zéro et répare ce qui peut l'être, parce que ces trois pannes
/// sont les plus fréquentes et qu'aucune ne devrait obliger le gérant à
/// ouvrir le Gestionnaire de périphériques en pleine boutique :
///
/// 1. Un réseau resté ouvert par un essai précédent occupe la carte Wi-Fi
///    et fait échouer le démarrage suivant → on arrête d'abord.
/// 2. La carte virtuelle du réseau hébergé est dans un état bancal →
///    `mode=disallow` puis `mode=allow` force Windows à la refaire. C'est la
///    manipulation qui règle l'erreur "le groupe ou la ressource n'est pas
///    dans l'état approprié".
/// 3. Cette même carte est simplement DÉSACTIVÉE (elle est cachée, donc
///    invisible sans l'option "Afficher les périphériques cachés") → ce
///    script tourne déjà en administrateur, il la réactive lui-même.
#[cfg(windows)]
fn script_activation(
    ssid: &str,
    mot_de_passe: &str,
    resultat: &std::path::Path,
    script_demarrage: &std::path::Path,
) -> String {
    let ssid = echapper_powershell(ssid);
    let mot_de_passe = echapper_powershell(mot_de_passe);
    let ip = ADRESSE_POINT_ACCES.to_string();
    format!(
        r#"
$ErrorActionPreference = 'Continue'
$sortie = @()
try {{
    $sortie += (netsh wlan stop hostednetwork 2>&1 | Out-String)
    $sortie += (netsh wlan set hostednetwork mode=disallow 2>&1 | Out-String)
    $sortie += (netsh wlan set hostednetwork mode=allow ssid="{ssid}" key="{mot_de_passe}" 2>&1 | Out-String)

    $sortie += "===CARTES_VIRTUELLES==="
    $cartes = @(Get-NetAdapter -IncludeHidden -ErrorAction SilentlyContinue | Where-Object {{ $_.InterfaceDescription -like '*Hosted Network Virtual Adapter*' }})
    foreach ($carte in $cartes) {{
        $sortie += ("carte: " + $carte.Name + " / " + $carte.Status)
        if ($carte.Status -ne 'Up') {{
            $sortie += (Enable-NetAdapter -Name $carte.Name -Confirm:$false -ErrorAction SilentlyContinue 2>&1 | Out-String)
        }}
    }}

    $sortie += "{marqueur_debut}"
    $sortie += (netsh wlan start hostednetwork 2>&1 | Out-String)
    $sortie += "{marqueur_fin}"

    # 5. Ouvrir le pare-feu Windows. Greffé ici plutôt que dans sa propre
    #    fenêtre d'autorisation : un seul "Oui" du gérant pour tout.
    $sortie += "===PARE_FEU==="
{pare_feu}

    # 6. Rallumer ce réseau à chaque démarrage du PC, sans rien demander.
    #    Créée dans le même accord administrateur : une tâche planifiée "au
    #    plus haut niveau de privilèges" ne redemande jamais d'autorisation
    #    par la suite.
    $sortie += "===DEMARRAGE_AUTO==="
    $commandeTache = 'powershell.exe -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File "{script_demarrage}"'
    $sortie += (schtasks /Create /TN "{nom_tache}" /TR $commandeTache /SC ONLOGON /RL HIGHEST /F 2>&1 | Out-String)

    $adaptateur = Get-NetAdapter | Where-Object {{ $_.InterfaceDescription -like '*Hosted Network Virtual Adapter*' }} | Select-Object -First 1
    if ($adaptateur) {{
        Remove-NetIPAddress -InterfaceIndex $adaptateur.InterfaceIndex -Confirm:$false -ErrorAction SilentlyContinue
        New-NetIPAddress -InterfaceIndex $adaptateur.InterfaceIndex -IPAddress "{ip}" -PrefixLength 24 -ErrorAction Stop | Out-Null
        $sortie += "ADRESSE_CONFIGUREE"
    }} else {{
        $sortie += "ADAPTATEUR_INTROUVABLE"
    }}
}} catch {{
    $sortie += "ERREUR_POWERSHELL: $_"
}}
$sortie -join "`n" | Out-File -FilePath "{res}" -Encoding utf8
"#,
        ssid = ssid,
        mot_de_passe = mot_de_passe,
        ip = ip,
        marqueur_debut = MARQUEUR_DEBUT_DEMARRAGE,
        marqueur_fin = MARQUEUR_FIN_DEMARRAGE,
        pare_feu = crate::pare_feu::commandes_powershell(),
        nom_tache = NOM_TACHE_DEMARRAGE,
        script_demarrage = script_demarrage.display(),
        res = resultat.display(),
    )
}

#[cfg(windows)]
fn script_desactivation(resultat: &std::path::Path) -> String {
    format!(
        r#"
$sortie = (netsh wlan stop hostednetwork 2>&1 | Out-String)
$sortie | Out-File -FilePath "{res}" -Encoding utf8
"#,
        res = resultat.display(),
    )
}

/// `netsh` répond dans la langue de Windows : on cherche des morceaux de
/// phrase plutôt qu'un message exact, pour rester correct même si la
/// formulation précise varie d'une version de Windows à l'autre.
///
/// La comparaison passe par `normaliser` — donc sans accents — pour la même
/// raison que le diagnostic : selon le chemin qu'a pris la sortie de `netsh`
/// (console CP850, fichier UTF-8), les accents arrivent ici intacts ou
/// abîmés. Les chercher tels quels ferait prendre une réussite pour un
/// échec sur un Windows français, c'est-à-dire sur presque tous les PC visés.
fn contient_confirmation_demarrage(sortie: &str) -> bool {
    let normalisee = normaliser(sortie);
    normalisee.contains("hosted network started")
        // "le mode hébergé a démarré" / "le réseau hébergé a démarré", une
        // fois les accents retirés.
        || normalisee.contains("hberg a dmarr")
}

/// Active le point d'accès Wi-Fi local avec le SSID/mot de passe fournis, et
/// lui assigne son adresse fixe. Ne dépend d'aucune connexion internet ni
/// Ethernet. Bloquant (attend la réponse de la fenêtre d'autorisation
/// Windows) — à appeler depuis un thread dédié, jamais directement dans une
/// tâche asynchrone.
#[cfg(windows)]
pub fn activer(ssid: &str, mot_de_passe: &str) -> Result<(), String> {
    let fichier_resultat = std::env::temp_dir().join("photocopie-benin-hotspot-resultat.txt");
    // Écrit avant l'élévation : ce dossier appartient à l'utilisateur, aucun
    // droit administrateur n'y est nécessaire. En cas d'échec on active quand
    // même — le démarrage automatique est un confort, pas une condition.
    let script_demarrage = ecrire_script_demarrage().unwrap_or_default();
    let sortie = executer_script_eleve(&script_activation(
        ssid,
        mot_de_passe,
        &fichier_resultat,
        &script_demarrage,
    ))?;

    if sortie.trim().is_empty() {
        return Err(
            "Windows n'a pas exécuté la commande d'activation (aucune réponse). Si la fenêtre \
             d'autorisation Windows est bien apparue et que vous avez cliqué \"Oui\", \
             réessayez une fois : ce PC a peut-être mis plus de temps que prévu à démarrer la \
             commande."
                .to_string(),
        );
    }
    // Seule la réponse de `netsh wlan start hostednetwork` fait foi : les
    // commandes de remise à zéro qui la précèdent produisent elles aussi des
    // messages, dont certains ressemblent à des erreurs alors qu'ils sont
    // normaux (arrêter un réseau qui ne tournait pas, par exemple).
    let demarrage = extraire_section(&sortie, MARQUEUR_DEBUT_DEMARRAGE, MARQUEUR_FIN_DEMARRAGE)
        .unwrap_or(sortie.as_str())
        .trim()
        .to_string();
    if !contient_confirmation_demarrage(&demarrage) {
        let normalise = normaliser(&demarrage);
        if normalise.contains("non pris en charge") || normalise.contains("not supported") {
            return Err(
                "La carte Wi-Fi de ce PC ne supporte pas la création d'un point d'accès \
                 autonome. Utilisez plutôt la solution de secours (paramètres Windows), qui \
                 nécessite une connexion internet ou Ethernet active."
                    .to_string(),
            );
        }
        if demarrage.is_empty() {
            return Err(format!(
                "Windows n'a rien répondu à la commande de démarrage du réseau. Détail \
                 technique complet : {}",
                sortie.trim()
            ));
        }
        return Err(format!(
            "Windows a refusé d'activer le point d'accès. Message exact de Windows : {demarrage}"
        ));
    }
    if sortie.contains("ADAPTATEUR_INTROUVABLE") {
        return Err(
            "Le réseau Wi-Fi a démarré, mais son adresse n'a pas pu être configurée : les \
             téléphones connectés risquent de ne rien pouvoir envoyer. Réessayez, ou signalez \
             ce problème."
                .to_string(),
        );
    }
    if !sortie.contains("ADRESSE_CONFIGUREE") {
        return Err(format!(
            "Le réseau Wi-Fi a démarré, mais son adresse n'a pas pu être vérifiée. Détail : {}",
            sortie.trim()
        ));
    }
    Ok(())
}

/// Coupe le point d'accès Wi-Fi local activé par `activer`. Échoue en
/// silence si rien n'était démarré : le gérant peut cliquer "Désactiver" par
/// précaution, sans savoir si c'était déjà arrêté.
#[cfg(windows)]
pub fn desactiver() -> Result<(), String> {
    let fichier_resultat = std::env::temp_dir().join("photocopie-benin-hotspot-resultat.txt");
    let _ = executer_script_eleve(&script_desactivation(&fichier_resultat))?;
    Ok(())
}

#[cfg(not(windows))]
pub fn activer(_ssid: &str, _mot_de_passe: &str) -> Result<(), String> {
    Err("Disponible uniquement sur Windows".to_string())
}

/// Ce qui a effectivement marché, une fois le réseau créé.
pub struct Activation {
    /// Nom de la méthode, repris tel quel dans l'interface : sur le terrain,
    /// savoir laquelle des deux a fonctionné est la première information
    /// utile quand quelque chose cloche ensuite.
    pub methode: &'static str,
    /// Adresse du PC sur ce réseau — pas la même selon la méthode, et c'est
    /// elle que les serveurs DHCP/DNS et le QR code doivent annoncer.
    pub adresse: Ipv4Addr,
    /// Tout ce que le gérant doit savoir malgré la réussite : les méthodes
    /// essayées avant celle qui a marché et pourquoi elles ont échoué, et
    /// les réglages système qui n'ont pas pu être appliqués (pare-feu).
    ///
    /// Trouvé sur le terrain : jusqu'ici, dès qu'une méthode réussissait,
    /// l'échec des précédentes était jeté. Or c'est exactement l'inverse
    /// qu'il faut : sur ce PC, la méthode 1 (réseau hébergé) est la SEULE
    /// adaptée à son pilote Wi-Fi de 2011 — la méthode 2 y est officiellement
    /// dépréciée. Elle "réussissait" donc en apparence tout en ne marchant
    /// jamais vraiment, pendant que la vraie erreur, celle qui aurait permis
    /// de réparer, restait invisible.
    pub avertissements: Vec<String>,
}

/// Traduit l'erreur brute de `netsh wlan start hostednetwork` en une phrase
/// avec la manipulation qui la règle.
///
/// "Le groupe ou la ressource n'est pas dans l'état approprié" est de loin
/// la panne la plus fréquente de cette méthode, et elle a une cause unique
/// et une solution en trente secondes : la carte virtuelle que Windows crée
/// pour le réseau hébergé est désactivée dans le Gestionnaire de
/// périphériques. Laisser passer le message d'origine, incompréhensible,
/// revenait à condamner un PC parfaitement capable.
fn expliquer_echec_reseau_heberge(erreur: &str) -> String {
    let normalise = normaliser(erreur);
    let ressource_mal_en_point = normalise.contains("tat appropri")
        || normalise.contains("not in the correct state")
        || normalise.contains("group or resource");

    if ressource_mal_en_point {
        return format!(
            "{erreur}\n\n➜ Cette erreur précise a presque toujours la même cause : la carte \
             virtuelle du réseau hébergé est DÉSACTIVÉE sur ce PC. Pour la réactiver : clic \
             droit sur le menu Démarrer → « Gestionnaire de périphériques » → menu « Affichage » \
             → « Afficher les périphériques cachés » → ouvrir « Cartes réseau » → clic droit sur \
             « Microsoft Hosted Network Virtual Adapter » (ou « Carte virtuelle hébergée ») → \
             « Activer ». Puis réessayez d'activer le Wi-Fi local."
        );
    }

    erreur.to_string()
}

/// LE bug de terrain, resté invisible le plus longtemps : quand la méthode 2
/// annonçait une réussite, on prenait l'adresse trouvée sur la carte Wi-Fi
/// Direct — et, quand il n'y en avait aucune, on se rabattait sur une
/// adresse DEVINÉE (192.168.137.1). Or "aucune adresse" ne veut pas dire
/// "adresse inconnue" : ça veut dire que le réseau n'a jamais existé.
///
/// Les conséquences observées en boutique s'expliquent toutes par là : le QR
/// affichait une adresse que rien ne portait, le serveur DNS refusait de
/// s'y attacher (erreur Windows 10049, "l'adresse demandée n'est pas valide
/// dans son contexte"), et le téléphone du client ne rejoignait jamais le PC
/// — le tout sous un bandeau vert "Wi-Fi activé". Pire encore : cette fausse
/// réussite écartait la méthode 1, la seule qui pouvait marcher sur ce PC,
/// et jetait au passage son message d'erreur.
///
/// Une adresse absente est donc désormais un ÉCHEC, dit comme tel.
fn echec_wifi_direct_sans_adresse(wdi_supporte: Option<bool>) -> String {
    let constat = "Windows a annoncé avoir créé le réseau, mais aucune adresse n'est apparue \
                   sur la carte Wi-Fi Direct : le réseau n'existe donc pas réellement, et \
                   aucun téléphone n'aurait pu le rejoindre.";

    if wdi_supporte == Some(false) {
        return format!(
            "{constat} La cause est connue pour ce PC : son pilote Wi-Fi n'expose pas \
             l'interface WDI, dont cette méthode dépend (Windows l'écrit lui-même dans le \
             diagnostic : « interface WDI non prise en charge »). Sur ce poste, seule la \
             méthode 1 (réseau hébergé) peut fonctionner — c'est son message d'erreur, \
             ci-dessus, qu'il faut traiter."
        );
    }

    constat.to_string()
}

/// Essaie TOUTES les façons connues de créer un Wi-Fi depuis ce PC, dans
/// l'ordre de ce qui a le plus de chances de marcher, et ne renonce qu'après
/// les avoir toutes épuisées.
///
/// Les deux méthodes ne couvrent pas le même parc : `hostednetwork` marche
/// sur les PC plus anciens, Wi-Fi Direct sur les plus récents. Les essayer
/// l'une après l'autre couvre donc bien plus de machines que n'importe
/// laquelle seule — et le gérant, lui, ne voit qu'un seul bouton.
pub fn activer_par_tous_les_moyens(
    ssid: &str,
    mot_de_passe: &str,
) -> Result<Activation, String> {
    let mut activation = tenter_toutes_les_methodes(ssid, mot_de_passe)?;
    if let Err(e) = garantir_pare_feu() {
        activation.avertissements.push(e);
    }
    Ok(activation)
}

/// Le réseau peut exister et rester parfaitement injoignable : sur un réseau
/// que Windows classe "public" — ce que sont tous les nôtres — le pare-feu
/// bloque par défaut tout ce qui arrive de l'extérieur. Voir `pare_feu.rs`.
///
/// Quand la méthode 1 a tourné, ses règles ont déjà été créées dans le même
/// script élevé : la vérification passe et le gérant n'a rien de plus à
/// accepter. Sinon seulement, une autorisation est demandée.
fn garantir_pare_feu() -> Result<(), String> {
    if crate::pare_feu::regles_presentes() {
        return Ok(());
    }
    crate::pare_feu::autoriser().map_err(|e| {
        format!(
            "Le pare-feu Windows n'a pas pu être ouvert ({e}). Le Wi-Fi fonctionne, mais \
             Windows risque de bloquer les téléphones avant qu'ils n'atteignent la page \
             d'envoi. Réessayez d'activer le Wi-Fi local et acceptez la fenêtre \
             d'autorisation Windows."
        )
    })
}

fn tenter_toutes_les_methodes(
    ssid: &str,
    mot_de_passe: &str,
) -> Result<Activation, String> {
    let diagnostic = diagnostiquer();
    if diagnostic.carte_wifi_presente == Some(false) {
        return Err(diagnostic.verdict);
    }

    let mut echecs = Vec::new();

    // Méthode 1 : sautée seulement quand Windows affirme qu'elle est
    // impossible — en cas de doute (`None`), on essaie quand même.
    if diagnostic.reseau_heberge_supporte == Some(false) {
        echecs.push(
            "Méthode 1 (réseau hébergé) : la carte Wi-Fi de ce PC déclare ne pas la supporter."
                .to_string(),
        );
    } else {
        match activer(ssid, mot_de_passe) {
            Ok(()) => {
                if let Ok(mut garde) = ADRESSE_ACTIVE.lock() {
                    *garde = Some(ADRESSE_POINT_ACCES);
                }
                return Ok(Activation {
                    methode: "réseau hébergé",
                    adresse: ADRESSE_POINT_ACCES,
                    avertissements: echecs,
                });
            }
            Err(e) => echecs.push(format!(
                "Méthode 1 (réseau hébergé) : {}",
                expliquer_echec_reseau_heberge(&e)
            )),
        }
    }

    if diagnostic.wifi_direct_go_supporte == Some(false) {
        echecs.push(
            "Méthode 2 (Wi-Fi Direct) : la carte Wi-Fi de ce PC déclare ne pas savoir créer de \
             groupe Wi-Fi Direct."
                .to_string(),
        );
    } else {
        match crate::wifi_direct::activer(ssid, mot_de_passe) {
            // Windows dit avoir démarré le réseau. Ça ne suffit pas : c'est
            // l'adresse réellement apparue sur la carte Wi-Fi Direct qui
            // prouve qu'il existe (voir `echec_wifi_direct_sans_adresse`).
            Ok(()) => match adresse_adaptateur_wifi_direct() {
                Some(adresse) => {
                    if let Ok(mut garde) = ADRESSE_ACTIVE.lock() {
                        *garde = Some(adresse);
                    }
                    return Ok(Activation {
                        methode: "Wi-Fi Direct",
                        adresse,
                        avertissements: echecs,
                    });
                }
                None => {
                    // Ne pas laisser tourner une annonce Wi-Fi Direct qui ne
                    // mène à rien : elle occuperait la carte pour rien.
                    let _ = crate::wifi_direct::desactiver();
                    let cartes = adresses_par_carte();
                    echecs.push(format!(
                        "Méthode 2 (Wi-Fi Direct) : {}\n\nAdresses présentes sur ce PC au \
                         moment de l'échec :\n{}",
                        echec_wifi_direct_sans_adresse(diagnostic.wdi_supporte),
                        if cartes.trim().is_empty() {
                            "(aucune)".to_string()
                        } else {
                            cartes.trim().to_string()
                        }
                    ));
                }
            },
            Err(e) => echecs.push(format!("Méthode 2 (Wi-Fi Direct) : {e}")),
        }
    }

    Err(format!(
        "Aucune des méthodes disponibles n'a pu créer le réseau Wi-Fi sur ce PC.\n\n{}",
        echecs.join("\n")
    ))
}

/// Un réseau créé par l'application est-il en train de tourner ? Sert à
/// choisir le bon QR code : proposer de rejoindre un réseau qui n'existe pas
/// enverrait le client dans le vide.
pub fn point_acces_actif() -> bool {
    adresse_point_acces_active().is_some()
}

/// Séparateur entre l'adresse et la description de la carte qui la porte,
/// dans la sortie de `adresses_par_carte`.
const SEPARATEUR_CARTE: char = '|';

/// Choisit, parmi toutes les adresses en service de ce PC, celle du réseau
/// que l'application vient de créer.
///
/// Constaté sur le terrain, photo à l'appui : le réseau Wi-Fi existait
/// vraiment (le téléphone du gérant s'y était connecté, pleine réception),
/// mais le QR annonçait `10.10.10.1` — l'adresse d'une carte VPN fantôme
/// sans rapport, qui n'a évidemment jamais répondu
/// ("ERR_ADDRESS_UNREACHABLE"). Deviner "la première adresse locale venue"
/// ne pouvait pas marcher sur un PC qui en porte plusieurs.
///
/// On choisit donc sur un critère vérifiable, dans cet ordre :
///
/// 1. l'adresse portée par une carte virtuelle de point d'accès — Windows
///    les nomme toujours pareil, et ce nom n'est pas traduit ;
/// 2. à défaut, une adresse dans `192.168.137.0/24`, la plage que Windows
///    réserve depuis toujours à ses points d'accès logiciels ;
/// 3. sinon rien — plutôt qu'une adresse choisie au hasard, qui enverrait
///    de nouveau le client dans le vide.
fn choisir_adresse_point_acces(lignes: &str) -> Option<Ipv4Addr> {
    let entrees: Vec<(Ipv4Addr, String)> = lignes
        .lines()
        .filter_map(|ligne| {
            let (adresse, carte) = ligne.trim().split_once(SEPARATEUR_CARTE)?;
            let adresse = adresse.trim().parse::<Ipv4Addr>().ok()?;
            if adresse.is_loopback() || adresse.is_unspecified() || adresse.is_link_local() {
                return None;
            }
            Some((adresse, normaliser(carte)))
        })
        .collect();

    entrees
        .iter()
        .find(|(_, carte)| {
            carte.contains("wi-fi direct virtual adapter")
                || carte.contains("hosted network virtual adapter")
        })
        .or_else(|| {
            entrees
                .iter()
                .find(|(adresse, _)| adresse.octets()[..3] == [192, 168, 137])
        })
        .map(|(adresse, _)| *adresse)
}

/// Toutes les adresses IPv4 réellement en service, chacune suivie du nom de
/// la carte qui la porte. Sert à choisir la bonne (voir
/// `choisir_adresse_point_acces`) et, quand aucune ne convient, à montrer au
/// support ce que ce PC présentait vraiment au moment de l'échec.
#[cfg(windows)]
fn adresses_par_carte() -> String {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            "Get-NetIPAddress -AddressFamily IPv4 -AddressState Preferred \
             -ErrorAction SilentlyContinue | ForEach-Object { \
             $d = (Get-NetAdapter -InterfaceIndex $_.InterfaceIndex \
             -ErrorAction SilentlyContinue).InterfaceDescription; \
             \"$($_.IPAddress)|$d\" }",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|sortie| String::from_utf8_lossy(&sortie.stdout).into_owned())
        .unwrap_or_default()
}

#[cfg(not(windows))]
fn adresses_par_carte() -> String {
    String::new()
}

/// L'adresse d'un point d'accès que Windows fait tourner en ce moment, s'il
/// y en a un. Contrairement à `adresse_point_acces_active`, ne s'appuie pas
/// sur ce que l'application croit avoir activé mais sur ce que Windows
/// présente réellement : dernier filet quand notre propre comptabilité s'est
/// perdue (activation partielle, application redémarrée, réseau créé lors
/// d'un essai précédent).
pub fn adresse_point_acces_detectee() -> Option<Ipv4Addr> {
    choisir_adresse_point_acces(&adresses_par_carte())
}

/// Retrouve l'adresse du point d'accès que Windows vient de créer.
///
/// L'attribution n'est pas instantanée : quelques tentatives espacées
/// laissent le temps à l'adresse d'apparaître avant de renoncer.
fn adresse_adaptateur_wifi_direct() -> Option<Ipv4Addr> {
    const TENTATIVES: u32 = 10;
    const DELAI_ENTRE_TENTATIVES: std::time::Duration = std::time::Duration::from_millis(500);

    for tentative in 0..TENTATIVES {
        if tentative > 0 {
            std::thread::sleep(DELAI_ENTRE_TENTATIVES);
        }
        if let Some(adresse) = choisir_adresse_point_acces(&adresses_par_carte()) {
            return Some(adresse);
        }
    }
    None
}

/// Au lancement de l'application : adopte un point d'accès que Windows fait
/// déjà tourner, et remet en service ce qui va avec.
///
/// C'est la seconde moitié du démarrage automatique. La tâche planifiée
/// (voir `NOM_TACHE_DEMARRAGE`) rallume le réseau Wi-Fi à l'ouverture de
/// session, mais un réseau seul ne sert à rien : sans les serveurs DHCP et
/// DNS — qui vivent dans cette application, pas dans Windows — les
/// téléphones rejoignent un Wi-Fi qui ne leur donne aucune adresse et
/// n'ouvre aucune page. Exactement le « Connexion… » sans fin du terrain.
///
/// Les deux moitiés ne démarrent pas au même rythme : Windows peut mettre
/// une minute à sortir le réseau, et l'application s'ouvre souvent avant.
/// D'où des tentatives espacées plutôt qu'un seul regard au démarrage.
///
/// N'active RIEN par elle-même et ne demande aucune autorisation : elle se
/// contente de constater. Si aucun réseau n'a été allumé, il ne se passe
/// rien et le bouton « Activer le Wi-Fi local » garde son rôle habituel.
pub fn reprendre_point_acces_existant(app: tauri::AppHandle) {
    use tauri::Manager;

    const TENTATIVES: u32 = 12;
    const DELAI: std::time::Duration = std::time::Duration::from_secs(5);

    tauri::async_runtime::spawn(async move {
        for tentative in 0..TENTATIVES {
            if tentative > 0 {
                tokio::time::sleep(DELAI).await;
            }
            // Le gérant a pu appuyer sur le bouton entre-temps : ne pas lui
            // passer devant, ni faire tourner deux DHCP sur le même port.
            if point_acces_actif() {
                return;
            }

            // Interroger Windows est bloquant : jamais directement dans une
            // tâche asynchrone, sous peine de figer le reste.
            let trouvee = tauri::async_runtime::spawn_blocking(adresse_point_acces_detectee)
                .await
                .ok()
                .flatten();
            let Some(adresse) = trouvee else { continue };

            let mut taches = Vec::new();
            if let Ok(tache) = crate::dhcp::demarrer(adresse).await {
                taches.push(tache);
            }
            if let Ok(tache) = crate::dns::demarrer(adresse).await {
                taches.push(tache);
            }
            // Sans DHCP ni DNS, se déclarer actif ferait afficher un QR pour
            // un réseau où rien ne répondrait : mieux vaut laisser le gérant
            // appuyer sur le bouton, qui lui dira ce qui bloque.
            if taches.is_empty() {
                return;
            }

            definir_adresse_active(Some(adresse));
            if let Some(etat) = app.try_state::<EtatPointAcces>() {
                if let Ok(mut garde) = etat.0.lock() {
                    let anciennes = std::mem::replace(&mut *garde, taches);
                    for ancienne in anciennes {
                        ancienne.abort();
                    }
                }
            }
            return;
        }
    });
}

/// Coupe le réseau quelle que soit la méthode qui l'a créé.
pub fn desactiver_par_tous_les_moyens() -> Result<(), String> {
    if let Ok(mut garde) = ADRESSE_ACTIVE.lock() {
        *garde = None;
    }
    let arret_wifi_direct = crate::wifi_direct::desactiver();
    let arret_reseau_heberge = desactiver();
    // Sur un PC donné, une seule des deux était active : l'échec de l'autre
    // est normal et ne doit pas être remonté comme une erreur.
    if arret_reseau_heberge.is_ok() || arret_wifi_direct.is_ok() {
        Ok(())
    } else {
        arret_reseau_heberge
    }
}

#[cfg(not(windows))]
pub fn desactiver() -> Result<(), String> {
    Err("Disponible uniquement sur Windows".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reproduit le bug exact trouvé sur le terrain : avant ce correctif,
    /// `adresse_locale()` (server.rs) et le choix de l'adresse Wi-Fi Direct
    /// devinaient chacune séparément, et pouvaient se contredire — un QR
    /// affichant une adresse que le point d'accès réel n'utilisait pas.
    /// Ici, on vérifie que la source unique (`ADRESSE_ACTIVE`) est bien ce
    /// que `point_acces_actif` et `adresse_point_acces_active` relisent, et
    /// que la désactivation l'efface bien — sinon un ancien point d'accès
    /// coupé continuerait d'apparaître comme actif.
    #[test]
    fn point_acces_actif_reflete_uniquement_la_source_unique() {
        assert!(!point_acces_actif(), "rien d'activé au départ");
        assert_eq!(adresse_point_acces_active(), None);

        {
            let mut garde = ADRESSE_ACTIVE.lock().unwrap();
            *garde = Some(Ipv4Addr::new(10, 10, 10, 1));
        }
        assert!(point_acces_actif());
        assert_eq!(
            adresse_point_acces_active(),
            Some(Ipv4Addr::new(10, 10, 10, 1))
        );

        let _ = desactiver_par_tous_les_moyens();
        assert!(
            !point_acces_actif(),
            "la désactivation doit effacer la source unique, pas seulement tenter d'arrêter \
             les deux méthodes"
        );
        assert_eq!(adresse_point_acces_active(), None);
    }

    /// La console Windows française ne parle pas UTF-8 : les accents
    /// arrivent ici en caractères de remplacement. Le diagnostic doit rester
    /// juste dans ce cas, sinon il déclarerait "indéterminé" sur tous les
    /// Windows français — c'est-à-dire sur presque tous les PC visés.
    #[test]
    fn lit_la_prise_en_charge_meme_avec_les_accents_abimes() {
        let francais_abime = "Prise en charge du r\u{FFFD}seau h\u{FFFD}berg\u{FFFD} : Non";
        assert_eq!(
            lire_prise_en_charge_reseau_heberge(francais_abime),
            Some(false)
        );

        let francais_propre = "Prise en charge du réseau hébergé : Oui";
        assert_eq!(
            lire_prise_en_charge_reseau_heberge(francais_propre),
            Some(true)
        );

        let anglais = "    Hosted network supported  : Yes";
        assert_eq!(lire_prise_en_charge_reseau_heberge(anglais), Some(true));
    }

    #[test]
    fn ne_devine_pas_quand_windows_ne_dit_rien() {
        assert_eq!(
            lire_prise_en_charge_reseau_heberge("Interface name: Wi-Fi\nDriver: Intel"),
            None
        );
        assert_eq!(lire_presence_carte_wifi(""), None);
    }

    #[test]
    fn detecte_l_absence_de_carte_wifi_dans_les_deux_langues() {
        assert_eq!(
            lire_presence_carte_wifi("There is no wireless interface on the system."),
            Some(false)
        );
        assert_eq!(
            lire_presence_carte_wifi("Il n'y a aucune interface sans fil sur le système."),
            Some(false)
        );
        assert_eq!(
            lire_presence_carte_wifi("Nom : Wi-Fi\n    État : connecté"),
            Some(true)
        );
    }

    #[test]
    fn lit_la_capacite_wifi_direct_sans_confondre_supported_et_not_supported() {
        // "not supported" contient "supported" : c'est le piège exact que
        // cette lecture doit éviter, sous peine de déclarer capable un PC
        // qui ne l'est pas.
        assert_eq!(
            lire_prise_en_charge_wifi_direct_go("    Wi-Fi Direct GO         : Not supported"),
            Some(false)
        );
        assert_eq!(
            lire_prise_en_charge_wifi_direct_go("    Wi-Fi Direct GO         : Supported"),
            Some(true)
        );
        assert_eq!(
            lire_prise_en_charge_wifi_direct_go("    GO Wi-Fi Direct : Non pris en charge"),
            Some(false)
        );
        assert_eq!(
            lire_prise_en_charge_wifi_direct_go("Nombre d'antennes connectées : 1"),
            None
        );
    }

    #[test]
    fn ne_prend_pas_une_absence_de_passerelle_pour_un_reseau() {
        // Le bug exact trouvé sur le terrain : un PC sans routeur ni box
        // branché, où le diagnostic a quand même affirmé "ce PC est déjà
        // sur un réseau" — la sortie vide de `Get-NetRoute` (aucune
        // passerelle par défaut) doit donner `false`, pas `true`.
        assert!(!a_une_passerelle_valide(""));
        assert!(!a_une_passerelle_valide("\n\n"));
        // `Get-NetRoute` peut renvoyer 0.0.0.0 pour une route sans
        // passerelle réelle (interface locale) : ça ne compte pas non plus.
        assert!(!a_une_passerelle_valide("0.0.0.0"));
    }

    /// L'erreur la plus fréquente du réseau hébergé, dans les deux langues
    /// de Windows : le message brut ne dit rien au gérant, alors que la
    /// cause est connue et la solution tient en une manipulation.
    #[test]
    fn explique_la_carte_virtuelle_desactivee() {
        let francais = "Le groupe ou la ressource n'est pas dans l'état approprié pour \
                        exécuter l'opération demandée.";
        let explique = expliquer_echec_reseau_heberge(francais);
        assert!(explique.contains("Gestionnaire de périphériques"));
        assert!(
            explique.contains(francais),
            "le message d'origine doit rester visible pour le support"
        );

        let anglais = "The group or resource is not in the correct state to perform the \
                       requested operation.";
        assert!(expliquer_echec_reseau_heberge(anglais).contains("Gestionnaire de périphériques"));
    }

    #[test]
    fn ne_deforme_pas_une_erreur_qu_on_ne_sait_pas_expliquer() {
        // Inventer une explication pour une panne inconnue enverrait le
        // gérant sur une fausse piste : mieux vaut transmettre tel quel.
        let inconnue = "Erreur inattendue du pilote Wi-Fi (code 0x8007139F).";
        assert_eq!(expliquer_echec_reseau_heberge(inconnue), inconnue);
    }

    #[test]
    fn reconnait_une_vraie_passerelle() {
        assert!(a_une_passerelle_valide("192.168.1.1"));
        // Plusieurs cartes réseau peuvent chacune annoncer une route par
        // défaut : une seule vraie passerelle suffit.
        assert!(a_une_passerelle_valide("0.0.0.0\n192.168.137.1\n"));
    }

    /// Un cas par ligne du tableau des situations rencontrées en boutique.
    /// Ces phrases sont lues telles quelles par le gérant : une consigne qui
    /// ne correspond pas à son poste le laisse bloqué devant un client.
    #[test]
    fn chaque_situation_de_boutique_recoit_la_bonne_consigne() {
        // PC déjà sur le réseau de la boutique : le parcours le plus simple,
        // il passe avant tout le reste même si ce PC sait créer un Wi-Fi.
        let deja_en_reseau = composer_verdict(
            Some(true),
            Some(true),
            Some(true),
            Some(true),
            Some(true),
            true,
        );
        assert!(deja_en_reseau.contains("Montrez simplement le QR"));
        assert!(deja_en_reseau.contains("pas nécessaire ici"));

        // Portable capable de créer son Wi-Fi, hors de tout réseau.
        let cree_son_wifi = composer_verdict(
            Some(true),
            Some(true),
            Some(false),
            Some(true),
            Some(true),
            false,
        );
        assert!(cree_son_wifi.contains("Activer le Wi-Fi local"));

        // Portable dont le pilote refuse les deux méthodes : reste le
        // Bluetooth, et il faut dire qu'il ne couvre pas les iPhone.
        let bluetooth_seul = composer_verdict(
            Some(true),
            Some(false),
            Some(false),
            Some(true),
            Some(true),
            false,
        );
        assert!(bluetooth_seul.contains("Bluetooth"));
        assert!(bluetooth_seul.contains("iPhone"));

        // Poste sans rien : ne pas laisser le gérant chercher.
        let rien = composer_verdict(
            Some(false),
            Some(false),
            Some(false),
            Some(false),
            Some(false),
            false,
        );
        assert!(rien.contains("clé USB"));
        assert!(rien.contains("câble réseau"));
    }

    #[test]
    fn dans_le_doute_on_propose_d_essayer_plutot_que_de_renoncer() {
        // Windows ne dit rien de clair (`None` partout) : l'application sait
        // essayer les deux méthodes, donc le verdict ne doit pas envoyer le
        // gérant vers la clé USB par excès de prudence.
        let indetermine = composer_verdict(None, None, None, None, None, false);
        assert!(
            indetermine.contains("Activer le Wi-Fi local"),
            "obtenu : {indetermine}"
        );
    }

    /// La ligne exacte relevée sur le PC de terrain (Broadcom 802.11n,
    /// pilote de 2011), accents abîmés compris comme les envoie la console
    /// française.
    #[test]
    fn lit_la_ligne_wdi_du_pc_de_terrain() {
        let terrain = "    Version WDI (fabricant de mat\u{FFFD}riel) : interface WDI non prise \
                       en charge";
        assert_eq!(lire_prise_en_charge_wdi(terrain), Some(false));

        assert_eq!(
            lire_prise_en_charge_wdi("    WDI Version (IHV)     : WDI not supported"),
            Some(false)
        );
        // Pilote moderne : un numéro de version.
        assert_eq!(
            lire_prise_en_charge_wdi("    WDI Version (IHV)     : 0.0.0.20"),
            Some(true)
        );
        // Rien sur le WDI : on ne devine pas.
        assert_eq!(
            lire_prise_en_charge_wdi("    Wi-Fi Direct GO : Supported"),
            None
        );
    }

    /// Le piège du PC de terrain : `wirelesscapabilities` y annonce
    /// "Wi-Fi Direct GO : pris en charge" alors que le pilote ne gère pas
    /// l'interface WDI dont cette méthode dépend. Le verdict ne doit alors
    /// pas s'appuyer sur la méthode 2 — et surtout pas déclarer capable un
    /// PC dont le réseau hébergé, lui, est refusé.
    #[test]
    fn un_pilote_sans_wdi_ne_compte_pas_sur_le_wifi_direct() {
        let sans_wdi = composer_verdict(
            Some(true),
            Some(false), // réseau hébergé refusé
            Some(true),  // Wi-Fi Direct annoncé...
            Some(false), // ...mais pas d'interface WDI : promesse non tenue
            Some(true),
            false,
        );
        assert!(
            !sans_wdi.contains("Activer le Wi-Fi local"),
            "obtenu : {sans_wdi}"
        );
        assert!(sans_wdi.contains("Bluetooth"));

        // Contrôle : avec le WDI, la même machine reste bien capable.
        let avec_wdi = composer_verdict(
            Some(true),
            Some(false),
            Some(true),
            Some(true),
            Some(true),
            false,
        );
        assert!(avec_wdi.contains("Activer le Wi-Fi local"));
    }

    /// Le correctif central de cette version : une méthode 2 sans adresse
    /// n'est pas une réussite, et le message doit désigner la méthode 1
    /// comme la seule piste réelle quand le pilote n'a pas le WDI.
    #[test]
    fn une_methode_2_sans_adresse_est_dite_echouee_et_renvoie_vers_la_methode_1() {
        let sans_wdi = echec_wifi_direct_sans_adresse(Some(false));
        assert!(sans_wdi.contains("n'existe donc pas réellement"));
        assert!(sans_wdi.contains("WDI"));
        assert!(sans_wdi.contains("méthode 1"));

        // Sans information sur le WDI, on constate sans inventer de cause.
        let inconnu = echec_wifi_direct_sans_adresse(None);
        assert!(inconnu.contains("n'existe donc pas réellement"));
        assert!(!inconnu.contains("WDI"));
    }

    /// Le script élevé lance maintenant plusieurs commandes avant le vrai
    /// démarrage. Confondre leurs messages avec celui de `start` ferait
    /// afficher au gérant une phrase sans rapport avec la panne.
    #[test]
    fn isole_la_reponse_de_la_commande_de_demarrage() {
        let sortie = "Le mode hébergé est arrêté.\n===DEBUT_DEMARRAGE===\nLe groupe ou la \
                      ressource n'est pas dans l'état approprié.\n===FIN_DEMARRAGE===\n\
                      ADRESSE_CONFIGUREE";
        let section = extraire_section(&sortie, MARQUEUR_DEBUT_DEMARRAGE, MARQUEUR_FIN_DEMARRAGE)
            .expect("la section doit être trouvée");
        assert!(section.contains("état approprié"));
        assert!(!section.contains("est arrêté"));
        assert!(!section.contains("ADRESSE_CONFIGUREE"));
    }

    /// Un script interrompu (PowerShell tué, PC qui s'éteint) n'écrit pas le
    /// marqueur de fin. Rendre `None` ferait perdre exactement le message
    /// qu'on cherchait à capturer.
    #[test]
    fn rend_la_fin_de_sortie_quand_le_marqueur_de_fin_manque() {
        let tronquee = "préliminaires\n===DEBUT_DEMARRAGE===\nErreur brutale";
        assert_eq!(
            extraire_section(&tronquee, MARQUEUR_DEBUT_DEMARRAGE, MARQUEUR_FIN_DEMARRAGE),
            Some("\nErreur brutale")
        );
        assert_eq!(
            extraire_section("rien du tout", MARQUEUR_DEBUT_DEMARRAGE, MARQUEUR_FIN_DEMARRAGE),
            None
        );
    }

    /// Le cas exact photographié en boutique : le réseau Wi-Fi existait
    /// vraiment (le téléphone s'y était connecté, pleine réception), mais le
    /// QR annonçait `10.10.10.1` — une carte VPN fantôme — et le téléphone
    /// répondait "ERR_ADDRESS_UNREACHABLE". C'est la carte du point d'accès
    /// qu'il fallait retenir, pas la première venue.
    #[test]
    fn ignore_la_carte_fantome_et_retient_celle_du_point_d_acces() {
        let terrain = "10.10.10.1|TAP-Windows Adapter V9\n                       192.168.137.1|Microsoft Wi-Fi Direct Virtual Adapter #2\n                       169.254.4.9|Realtek PCIe GbE Family Controller";
        assert_eq!(
            choisir_adresse_point_acces(terrain),
            Some(Ipv4Addr::new(192, 168, 137, 1))
        );
    }

    /// L'ordre des lignes ne doit rien changer : c'est le NOM de la carte
    /// qui tranche, pas sa position dans la liste.
    #[test]
    fn le_choix_ne_depend_pas_de_l_ordre_des_cartes() {
        let carte_apres = "10.10.10.1|TAP-Windows Adapter V9\n                           192.168.73.1|Microsoft Hosted Network Virtual Adapter";
        let carte_avant = "192.168.73.1|Microsoft Hosted Network Virtual Adapter\n                           10.10.10.1|TAP-Windows Adapter V9";
        let attendue = Some(Ipv4Addr::new(192, 168, 73, 1));
        assert_eq!(choisir_adresse_point_acces(carte_apres), attendue);
        assert_eq!(choisir_adresse_point_acces(carte_avant), attendue);
    }

    /// Sans carte reconnaissable, la plage que Windows réserve à ses points
    /// d'accès logiciels reste un indice valable — mais elle seule.
    #[test]
    fn retombe_sur_la_plage_des_points_d_acces_windows_puis_renonce() {
        assert_eq!(
            choisir_adresse_point_acces("192.168.137.1|Carte au nom inconnu"),
            Some(Ipv4Addr::new(192, 168, 137, 1))
        );
        // Aucune carte de point d'accès, aucune adresse dans la plage
        // Windows : renoncer vaut mieux qu'annoncer au client une adresse
        // qui ne répondra pas — c'est exactement le bug qu'on corrige.
        assert_eq!(
            choisir_adresse_point_acces("10.10.10.1|TAP-Windows Adapter V9"),
            None
        );
        assert_eq!(choisir_adresse_point_acces(""), None);
        assert_eq!(choisir_adresse_point_acces("ligne sans separateur"), None);
    }

    /// Une carte branchée sur rien, ou la boucle locale, ne désignent aucun
    /// réseau joignable — même si leur nom ressemble à celui d'un point
    /// d'accès.
    #[test]
    fn ecarte_les_adresses_qui_ne_menent_nulle_part() {
        assert_eq!(
            choisir_adresse_point_acces(
                "169.254.1.1|Microsoft Wi-Fi Direct Virtual Adapter\n                 127.0.0.1|Loopback\n0.0.0.0|Carte inconnue"
            ),
            None
        );
    }

    /// Une installation neuve ne doit jamais bloquer le gérant devant un
    /// client : il faut un nom de réseau utilisable, quoi qu'il y ait dans
    /// les réglages.
    #[test]
    fn propose_toujours_un_nom_de_reseau_utilisable() {
        assert_eq!(nom_reseau_par_defaut(None), "PHOTOCOPIE");
        assert_eq!(nom_reseau_par_defaut(Some("")), "PHOTOCOPIE");
        assert_eq!(nom_reseau_par_defaut(Some("   ")), "PHOTOCOPIE");

        // Le nom de la boutique, nettoyé de ce qu'un SSID supporte mal :
        // accents et ponctuation. Le client reconnaît ainsi l'endroit dans
        // la liste des Wi-Fi de son téléphone.
        assert_eq!(
            nom_reseau_par_defaut(Some("Photocopie Tonanzé")),
            "Photocopie-Tonanz"
        );
        assert_eq!(
            nom_reseau_par_defaut(Some("Chez  Kofi & Fils")),
            "Chez-Kofi-Fils"
        );

        // Limite de la norme Wi-Fi : 32 caractères.
        let tres_long = "A".repeat(80);
        assert_eq!(nom_reseau_par_defaut(Some(&tres_long)).chars().count(), 32);
    }

    /// Le mot de passe par défaut doit passer la règle du Wi-Fi, sinon
    /// Windows refuse le réseau avec une erreur que personne ne comprend.
    #[test]
    fn le_mot_de_passe_par_defaut_respecte_la_regle_du_wifi() {
        assert!(MOT_DE_PASSE_PAR_DEFAUT.chars().count() >= 8);
        assert!(MOT_DE_PASSE_PAR_DEFAUT.is_ascii());
    }

    #[test]
    fn reconnait_la_confirmation_en_francais_et_en_anglais() {
        // Accents intacts (sortie lue en UTF-8)…
        assert!(contient_confirmation_demarrage(
            "Le mode hébergé a démarré."
        ));
        assert!(contient_confirmation_demarrage(
            "Le réseau hébergé a démarré."
        ));
        // …et accents abîmés (sortie passée par la console française) : le
        // même message doit être reconnu dans les deux cas.
        assert!(contient_confirmation_demarrage(
            "Le mode h\u{FFFD}berg\u{FFFD} a d\u{FFFD}marr\u{FFFD}."
        ));
        assert!(contient_confirmation_demarrage("The hosted network started."));
        assert!(!contient_confirmation_demarrage(
            "Accès refusé. Vous devez être administrateur."
        ));
    }

    #[cfg(windows)]
    #[test]
    fn echappe_les_caracteres_speciaux_powershell() {
        assert_eq!(echapper_powershell(r#"mot"de"passe"#), r#"mot`"de`"passe"#);
        assert_eq!(echapper_powershell("prix$100"), "prix`$100");
        assert_eq!(echapper_powershell("a`b"), "a``b");
    }

    #[cfg(windows)]
    #[test]
    fn le_script_d_activation_integre_le_ssid_et_le_mot_de_passe_echappes() {
        let script = script_activation(
            "Ma Boutique",
            "secret\"123",
            std::path::Path::new("C:\\r.txt"),
            std::path::Path::new("C:\\demarrage.ps1"),
        );
        assert!(script.contains(r#"ssid="Ma Boutique""#));
        assert!(script.contains(r#"key="secret`"123""#));
        assert!(script.contains("192.168.73.1"));
    }

    /// Les trois réparations automatiques ajoutées après les essais de
    /// terrain : sans elles, le gérant devait ouvrir le Gestionnaire de
    /// périphériques en pleine boutique, ou ne comprenait pas pourquoi un
    /// deuxième essai échouait toujours.
    #[cfg(windows)]
    #[test]
    fn le_script_repare_avant_de_demarrer() {
        let script = script_activation(
            "Boutique",
            "motdepasse",
            std::path::Path::new("C:\\r.txt"),
            std::path::Path::new("C:\\demarrage.ps1"),
        );
        let position = |aiguille: &str| script.find(aiguille).expect(aiguille);

        // 1. Arrêt d'un réseau resté ouvert, 2. remise à zéro de la carte
        // virtuelle, 3. réactivation si Windows l'a désactivée — puis
        // seulement le démarrage.
        assert!(position("stop hostednetwork") < position("mode=disallow"));
        assert!(position("mode=disallow") < position("mode=allow"));
        assert!(position("mode=allow") < position("Enable-NetAdapter"));
        assert!(position("Enable-NetAdapter") < position("start hostednetwork"));
        assert!(script.contains("-IncludeHidden"));

        // Le démarrage doit être encadré pour que son message soit isolable.
        assert!(position(MARQUEUR_DEBUT_DEMARRAGE) < position("start hostednetwork"));
        assert!(position("start hostednetwork") < position(MARQUEUR_FIN_DEMARRAGE));

        // Le réseau doit se rallumer tout seul au démarrage du PC, sinon le
        // gérant qui oublie de cliquer perd son premier client de la journée.
        assert!(script.contains("schtasks /Create"));
        assert!(script.contains(NOM_TACHE_DEMARRAGE));
        assert!(script.contains("/SC ONLOGON"));
        // "Au plus haut niveau de privilèges" : c'est ce qui évite que
        // Windows redemande une autorisation chaque matin.
        assert!(script.contains("/RL HIGHEST"));
        // /F remplace la tâche existante au lieu d'échouer, sinon une
        // deuxième activation ne mettrait jamais le chemin à jour.
        assert!(script.contains("/F"));
    }
}
