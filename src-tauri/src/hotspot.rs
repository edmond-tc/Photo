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
#[derive(Default)]
pub struct EtatPointAcces(pub Mutex<Option<(JoinHandle<()>, JoinHandle<()>)>>);

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

/// Ce PC est-il déjà sur un réseau que le téléphone d'un client pourrait
/// rejoindre ?
///
/// On écarte trois familles d'adresses qui ressemblent à un réseau sans en
/// être un : la boucle locale (le PC qui se parle à lui-même), les adresses
/// 169.254.x.x que Windows s'attribue justement quand AUCUN réseau n'a
/// répondu, et l'adresse de notre propre point d'accès — sinon un PC ne
/// serait jamais que "déjà en réseau avec lui-même".
fn reseau_utilisable(adresses: &[Ipv4Addr]) -> bool {
    adresses.iter().any(|adresse| {
        !adresse.is_loopback()
            && !adresse.is_link_local()
            && *adresse != ADRESSE_POINT_ACCES
            && !adresse.is_unspecified()
    })
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
    bluetooth_present: Option<bool>,
    reseau_utilisable: bool,
) -> String {
    // Une seule des deux méthodes suffit à créer le réseau : exiger les deux
    // déclarerait incapable un PC parfaitement capable.
    let sait_creer_un_wifi = carte_wifi_presente != Some(false)
        && (reseau_heberge_supporte != Some(false) || wifi_direct_go_supporte != Some(false));

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

/// Adresses IPv4 réellement portées par les cartes de ce PC.
fn adresses_locales() -> Vec<Ipv4Addr> {
    local_ip_address::list_afinet_netifas()
        .map(|interfaces| {
            interfaces
                .into_iter()
                .filter_map(|(_, ip)| match ip {
                    std::net::IpAddr::V4(v4) => Some(v4),
                    _ => None,
                })
                .collect()
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
    let bluetooth_present = bluetooth_present();
    let reseau_utilisable = reseau_utilisable(&adresses_locales());

    DiagnosticPoste {
        carte_wifi_presente,
        reseau_heberge_supporte,
        wifi_direct_go_supporte,
        bluetooth_present,
        reseau_utilisable,
        verdict: composer_verdict(
            carte_wifi_presente,
            reseau_heberge_supporte,
            wifi_direct_go_supporte,
            bluetooth_present,
            reseau_utilisable,
        ),
        details_bruts: format!(
            "--- netsh wlan show interfaces ---\n{}\n--- netsh wlan show drivers ---\n{}\
             \n--- netsh wlan show wirelesscapabilities ---\n{}",
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
fn executer_script_eleve(script: &str) -> Result<String, String> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{WaitForSingleObject, INFINITE};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

    let dossier_temp = std::env::temp_dir();
    let fichier_script = dossier_temp.join("photocopie-benin-hotspot.ps1");
    let fichier_resultat = dossier_temp.join("photocopie-benin-hotspot-resultat.txt");
    let _ = std::fs::remove_file(&fichier_resultat);

    std::fs::write(&fichier_script, script)
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
    let _ = std::fs::remove_file(&fichier_script);
    if lance.is_err() || info.hProcess.is_invalid() {
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

    let resultat = std::fs::read_to_string(&fichier_resultat).unwrap_or_default();
    let _ = std::fs::remove_file(&fichier_resultat);
    Ok(resultat)
}

/// Script complet : crée le point d'accès Wi-Fi, PUIS retrouve la carte
/// virtuelle que Windows vient de créer pour lui assigner une adresse fixe
/// et connue (`ADRESSE_POINT_ACCES`) — sans quoi le serveur DHCP (voir
/// `dhcp.rs`) annoncerait une adresse que la carte n'a pas vraiment, et
/// aucun téléphone ne pourrait jamais la joindre.
#[cfg(windows)]
fn script_activation(ssid: &str, mot_de_passe: &str, resultat: &std::path::Path) -> String {
    let ssid = echapper_powershell(ssid);
    let mot_de_passe = echapper_powershell(mot_de_passe);
    let ip = ADRESSE_POINT_ACCES.to_string();
    format!(
        r#"
$ErrorActionPreference = 'Continue'
$sortie = @()
try {{
    $sortie += (netsh wlan set hostednetwork mode=allow ssid="{ssid}" key="{mot_de_passe}" 2>&1 | Out-String)
    $sortie += (netsh wlan start hostednetwork 2>&1 | Out-String)

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
fn contient_confirmation_demarrage(sortie_minuscule: &str) -> bool {
    sortie_minuscule.contains("hosted network started")
        || sortie_minuscule.contains("le mode hébergé a démarré")
        || sortie_minuscule.contains("réseau hébergé a démarré")
}

/// Active le point d'accès Wi-Fi local avec le SSID/mot de passe fournis, et
/// lui assigne son adresse fixe. Ne dépend d'aucune connexion internet ni
/// Ethernet. Bloquant (attend la réponse de la fenêtre d'autorisation
/// Windows) — à appeler depuis un thread dédié, jamais directement dans une
/// tâche asynchrone.
#[cfg(windows)]
pub fn activer(ssid: &str, mot_de_passe: &str) -> Result<(), String> {
    let fichier_resultat = std::env::temp_dir().join("photocopie-benin-hotspot-resultat.txt");
    let sortie = executer_script_eleve(&script_activation(ssid, mot_de_passe, &fichier_resultat))?;
    let sortie_minuscule = sortie.to_lowercase();

    if sortie.trim().is_empty() {
        return Err(
            "Windows n'a rien répondu. Vérifiez que vous avez bien cliqué \"Oui\" sur la \
             fenêtre d'autorisation qui devait apparaître."
                .to_string(),
        );
    }
    if !contient_confirmation_demarrage(&sortie_minuscule) {
        if sortie_minuscule.contains("non pris en charge") || sortie_minuscule.contains("not supported")
        {
            return Err(
                "La carte Wi-Fi de ce PC ne supporte pas la création d'un point d'accès \
                 autonome. Utilisez plutôt la solution de secours (paramètres Windows), qui \
                 nécessite une connexion internet ou Ethernet active."
                    .to_string(),
            );
        }
        return Err(format!(
            "Windows a refusé d'activer le point d'accès. Détail technique : {}",
            sortie.trim()
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
                return Ok(Activation {
                    methode: "réseau hébergé",
                    adresse: ADRESSE_POINT_ACCES,
                })
            }
            Err(e) => echecs.push(format!("Méthode 1 (réseau hébergé) : {e}")),
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
            Ok(()) => {
                return Ok(Activation {
                    methode: "Wi-Fi Direct",
                    adresse: adresse_wifi_direct(),
                })
            }
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
    if crate::wifi_direct::est_actif() {
        return true;
    }
    local_ip_address::list_afinet_netifas()
        .map(|interfaces| {
            interfaces
                .iter()
                .any(|(_, ip)| *ip == std::net::IpAddr::V4(ADRESSE_POINT_ACCES))
        })
        .unwrap_or(false)
}

/// Contrairement au réseau hébergé, c'est Windows qui choisit l'adresse de
/// l'interface Wi-Fi Direct — historiquement dans 192.168.137.0/24. On la
/// retrouve donc au lieu de l'imposer.
fn adresse_wifi_direct() -> Ipv4Addr {
    if let Ok(interfaces) = local_ip_address::list_afinet_netifas() {
        for (_, ip) in interfaces {
            if let std::net::IpAddr::V4(v4) = ip {
                let octets = v4.octets();
                if octets[0] == 192 && octets[1] == 168 && octets[2] == 137 {
                    return v4;
                }
            }
        }
    }
    Ipv4Addr::new(192, 168, 137, 1)
}

/// Coupe le réseau quelle que soit la méthode qui l'a créé.
pub fn desactiver_par_tous_les_moyens() -> Result<(), String> {
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
    fn ne_prend_pas_une_absence_de_reseau_pour_un_reseau() {
        // 169.254.x.x est précisément ce que Windows s'attribue quand AUCUN
        // réseau n'a répondu : le confondre avec un vrai réseau ferait dire
        // au gérant "montrez le QR" alors que rien ne fonctionnerait.
        assert!(!reseau_utilisable(&[Ipv4Addr::new(169, 254, 12, 34)]));
        assert!(!reseau_utilisable(&[Ipv4Addr::new(127, 0, 0, 1)]));
        assert!(!reseau_utilisable(&[ADRESSE_POINT_ACCES]));
        assert!(!reseau_utilisable(&[]));

        assert!(reseau_utilisable(&[Ipv4Addr::new(192, 168, 1, 25)]));
        assert!(reseau_utilisable(&[
            Ipv4Addr::new(127, 0, 0, 1),
            Ipv4Addr::new(10, 0, 0, 7)
        ]));
    }

    /// Un cas par ligne du tableau des situations rencontrées en boutique.
    /// Ces phrases sont lues telles quelles par le gérant : une consigne qui
    /// ne correspond pas à son poste le laisse bloqué devant un client.
    #[test]
    fn chaque_situation_de_boutique_recoit_la_bonne_consigne() {
        // PC déjà sur le réseau de la boutique : le parcours le plus simple,
        // il passe avant tout le reste même si ce PC sait créer un Wi-Fi.
        let deja_en_reseau = composer_verdict(Some(true), Some(true), Some(true), Some(true), true);
        assert!(deja_en_reseau.contains("Montrez simplement le QR"));
        assert!(deja_en_reseau.contains("pas nécessaire ici"));

        // Portable capable de créer son Wi-Fi, hors de tout réseau.
        let cree_son_wifi =
            composer_verdict(Some(true), Some(true), Some(false), Some(true), false);
        assert!(cree_son_wifi.contains("Activer le Wi-Fi local"));

        // Portable dont le pilote refuse les deux méthodes : reste le
        // Bluetooth, et il faut dire qu'il ne couvre pas les iPhone.
        let bluetooth_seul =
            composer_verdict(Some(true), Some(false), Some(false), Some(true), false);
        assert!(bluetooth_seul.contains("Bluetooth"));
        assert!(bluetooth_seul.contains("iPhone"));

        // Poste sans rien : ne pas laisser le gérant chercher.
        let rien = composer_verdict(Some(false), Some(false), Some(false), Some(false), false);
        assert!(rien.contains("clé USB"));
        assert!(rien.contains("câble réseau"));
    }

    #[test]
    fn dans_le_doute_on_propose_d_essayer_plutot_que_de_renoncer() {
        // Windows ne dit rien de clair (`None` partout) : l'application sait
        // essayer les deux méthodes, donc le verdict ne doit pas envoyer le
        // gérant vers la clé USB par excès de prudence.
        let indetermine = composer_verdict(None, None, None, None, false);
        assert!(
            indetermine.contains("Activer le Wi-Fi local"),
            "obtenu : {indetermine}"
        );
    }

    #[test]
    fn reconnait_la_confirmation_en_francais_et_en_anglais() {
        assert!(contient_confirmation_demarrage(
            "le mode hébergé a démarré."
        ));
        assert!(contient_confirmation_demarrage("the hosted network started."));
        assert!(!contient_confirmation_demarrage(
            "accès refusé. vous devez être administrateur."
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
        let script = script_activation("Ma Boutique", "secret\"123", std::path::Path::new("C:\\r.txt"));
        assert!(script.contains(r#"ssid="Ma Boutique""#));
        assert!(script.contains(r#"key="secret`"123""#));
        assert!(script.contains("192.168.73.1"));
    }
}
