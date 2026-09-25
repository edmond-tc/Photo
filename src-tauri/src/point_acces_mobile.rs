//! Troisième méthode pour créer le Wi-Fi de la boutique : le « Point
//! d'accès mobile » de Windows — celui des Paramètres, qu'on allume d'un
//! interrupteur.
//!
//! Pourquoi elle existe. Les deux premières méthodes (`hotspot.rs` et
//! `wifi_direct.rs`) reposent sur une capacité des pilotes Wi-Fi appelée
//! SoftAP. Intel l'écrit officiellement : ses cartes récentes « doivent
//! implémenter un nouveau modèle de pilote et ne peuvent plus supporter le
//! SoftAP ; elles supportent à la place le Point d'accès mobile de
//! Windows ». La communauté Intel le confirme pour la carte AX201, l'une des
//! plus répandues depuis 2020 : réseau hébergé ET Wi-Fi Direct de type
//! SoftAP ne fonctionnent plus. D'où le constat du terrain : l'application
//! marchait sur le PC de mise au point — une carte Broadcom avec un pilote
//! de 2011, qui sait encore faire le SoftAP — et échouait partout ailleurs.
//! Cette méthode est l'adaptateur qui manquait pour les cartes récentes.
//!
//! Le seul obstacle : Windows refuse d'allumer ce point d'accès s'il n'a
//! aucune connexion à « partager ». Un PC de boutique n'en a aucune. La
//! parade, documentée et utilisée par ceux qui travaillent loin de tout
//! réseau : une carte réseau FACTICE (« Microsoft KM-TEST Loopback
//! Adapter », fournie avec Windows) qui sert de connexion à partager. Elle
//! ne mène nulle part et ne transporte rien ; elle donne seulement à
//! Windows la « source » qu'il exige. Sa création demande les droits
//! administrateur : c'est le rôle de `script_preparation`, exécuté une seule
//! fois par activation, et seulement quand les deux autres méthodes ont
//! déjà échoué.
//!
//! Ce que cette méthode ne fait PAS : ici c'est Windows qui distribue les
//! adresses et répond aux noms de domaine, pas nos serveurs. La page ne
//! s'ouvre donc pas toute seule ; le client scanne le second QR, comme sur
//! l'affiche.

use std::net::Ipv4Addr;
use std::path::Path;

/// Nom donné à la carte réseau factice, pour la retrouver d'une activation
/// à l'autre au lieu d'en créer une nouvelle à chaque fois.
pub const NOM_CARTE_FACTICE: &str = "Photocopie-Boucle";

/// Adresse fixe de la carte factice. Hors de toutes les plages utilisées
/// par Windows et par nos propres réseaux, sans passerelle : aucun trafic ne
/// peut s'y égarer, et rien ne la prendra pour un réseau connecté.
pub const ADRESSE_CARTE_FACTICE: Ipv4Addr = Ipv4Addr::new(10, 254, 254, 1);

/// Vrai pour l'adresse de la carte factice : elle ne doit JAMAIS être
/// annoncée à un client (elle ne mène nulle part).
pub fn est_adresse_factice(adresse: Ipv4Addr) -> bool {
    adresse.octets()[..3] == ADRESSE_CARTE_FACTICE.octets()[..3]
}

/// Marqueurs écrits par le script de préparation, relus pour savoir ce qui
/// s'est réellement passé.
pub const MARQUEUR_SERVICES: &str = "SERVICES_REACTIVES";
pub const MARQUEUR_CARTE_PRETE: &str = "CARTE_FACTICE_PRETE";
pub const MARQUEUR_CARTE_ABSENTE: &str = "CARTE_FACTICE_ABSENTE";

/// Le script à exécuter avec les droits administrateur, AVANT d'allumer le
/// point d'accès mobile.
///
/// 1. Il rend au point d'accès mobile les deux services Windows dont il a
///    besoin (`icssvc`, `SharedAccess`). La méthode 1 les désactive, pour de
///    bonnes raisons : ils volent les téléphones au serveur d'adresses de
///    notre réseau hébergé. Mais sans eux, cette méthode-ci est morte avant
///    d'avoir commencé.
/// 2. Il crée la carte factice si elle n'existe pas encore — par les mêmes
///    appels Windows que l'outil officiel `devcon`, qui n'est plus livré
///    avec Windows. `pnputil` seul ne suffit pas : il installe un pilote,
///    mais ne crée pas d'appareil pour une carte qui n'a pas de matériel.
/// 3. Il lui donne un nom et une adresse fixes, sans passerelle.
///
/// Il retire aussi la coupure automatique après 5 minutes sans téléphone,
/// et ouvre le pare-feu — dans la même autorisation, pour ne demander
/// « Oui » qu'une seule fois au gérant.
pub fn script_preparation(resultat: &Path) -> String {
    MODELE_SCRIPT_PREPARATION
        .replace("__RESULTAT__", &resultat.display().to_string())
        .replace("__NOM__", NOM_CARTE_FACTICE)
        .replace("__IP__", &ADRESSE_CARTE_FACTICE.to_string())
        .replace("__SERVICES__", MARQUEUR_SERVICES)
        .replace("__PRETE__", MARQUEUR_CARTE_PRETE)
        .replace("__ABSENTE__", MARQUEUR_CARTE_ABSENTE)
        // En dernier : les commandes du pare-feu ne doivent pas être
        // retouchées par les remplacements ci-dessus.
        .replace("__PARE_FEU__", &crate::pare_feu::commandes_powershell())
}

/// Écrit sans `format!` : le code C# embarqué est plein d'accolades, et les
/// doubler toutes rendrait le script illisible et facile à casser.
const MODELE_SCRIPT_PREPARATION: &str = r#"
$ErrorActionPreference = 'Continue'
$sortie = @()
try {
    foreach ($service in @('SharedAccess', 'icssvc')) {
        Set-Service -Name $service -StartupType Manual -ErrorAction SilentlyContinue
    }
    $sortie += "__SERVICES__"

    # Par défaut, Windows ÉTEINT le point d'accès mobile après 5 minutes sans
    # téléphone connecté : la boutique perdrait son Wi-Fi à chaque creux
    # entre deux clients. Ces deux valeurs retirent cette coupure.
    $cle = 'HKLM:\SYSTEM\CurrentControlSet\Services\icssvc\Settings'
    if (-not (Test-Path $cle)) { New-Item -Path $cle -Force | Out-Null }
    New-ItemProperty -Path $cle -Name 'PeerlessTimeoutEnabled' -PropertyType DWord -Value 0 -Force -ErrorAction SilentlyContinue | Out-Null
    New-ItemProperty -Path $cle -Name 'PeerlessTimeout' -PropertyType DWord -Value 120 -Force -ErrorAction SilentlyContinue | Out-Null

__PARE_FEU__

    $carte = Get-NetAdapter -Name "__NOM__" -ErrorAction SilentlyContinue
    if (-not $carte) {
        $carte = Get-NetAdapter -IncludeHidden -ErrorAction SilentlyContinue |
            Where-Object { $_.InterfaceDescription -like '*KM-TEST*' } |
            Select-Object -First 1
    }

    if (-not $carte) {
        Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class PhotocopieCarteFactice {
    [StructLayout(LayoutKind.Sequential)]
    struct SP_DEVINFO_DATA { public uint cbSize; public Guid ClassGuid; public uint DevInst; public IntPtr Reserved; }
    [DllImport("setupapi.dll", SetLastError = true)]
    static extern IntPtr SetupDiCreateDeviceInfoList(ref Guid classGuid, IntPtr parent);
    [DllImport("setupapi.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool SetupDiCreateDeviceInfoW(IntPtr set, string name, ref Guid classGuid, string description, IntPtr parent, uint flags, ref SP_DEVINFO_DATA data);
    [DllImport("setupapi.dll", SetLastError = true)]
    static extern bool SetupDiSetDeviceRegistryPropertyW(IntPtr set, ref SP_DEVINFO_DATA data, uint property, byte[] buffer, uint size);
    [DllImport("setupapi.dll", SetLastError = true)]
    static extern bool SetupDiCallClassInstaller(uint function, IntPtr set, ref SP_DEVINFO_DATA data);
    [DllImport("setupapi.dll", SetLastError = true)]
    static extern bool SetupDiDestroyDeviceInfoList(IntPtr set);
    [DllImport("newdev.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool UpdateDriverForPlugAndPlayDevicesW(IntPtr parent, string hardwareId, string infPath, uint flags, out bool reboot);

    public static string Creer(string inf) {
        Guid classeReseau = new Guid("4d36e972-e325-11ce-bfc1-08002be10318");
        IntPtr ensemble = SetupDiCreateDeviceInfoList(ref classeReseau, IntPtr.Zero);
        if (ensemble == new IntPtr(-1)) { return "liste " + Marshal.GetLastWin32Error(); }
        try {
            SP_DEVINFO_DATA donnees = new SP_DEVINFO_DATA();
            donnees.cbSize = (uint)Marshal.SizeOf(typeof(SP_DEVINFO_DATA));
            if (!SetupDiCreateDeviceInfoW(ensemble, "Net", ref classeReseau, null, IntPtr.Zero, 1, ref donnees)) {
                return "creation " + Marshal.GetLastWin32Error();
            }
            byte[] identifiant = System.Text.Encoding.Unicode.GetBytes("*MSLOOP\0\0");
            if (!SetupDiSetDeviceRegistryPropertyW(ensemble, ref donnees, 1, identifiant, (uint)identifiant.Length)) {
                return "identifiant " + Marshal.GetLastWin32Error();
            }
            if (!SetupDiCallClassInstaller(0x19, ensemble, ref donnees)) {
                return "enregistrement " + Marshal.GetLastWin32Error();
            }
        } finally {
            SetupDiDestroyDeviceInfoList(ensemble);
        }
        bool redemarrage;
        if (!UpdateDriverForPlugAndPlayDevicesW(IntPtr.Zero, "*MSLOOP", inf, 1, out redemarrage)) {
            return "pilote " + Marshal.GetLastWin32Error();
        }
        return "OK";
    }
}
"@
        $retour = [PhotocopieCarteFactice]::Creer("$env:windir\inf\netloop.inf")
        $sortie += "CREATION_CARTE: $retour"
        Start-Sleep -Seconds 4
        $carte = Get-NetAdapter -IncludeHidden -ErrorAction SilentlyContinue |
            Where-Object { $_.InterfaceDescription -like '*KM-TEST*' } |
            Select-Object -First 1
    }

    if ($carte) {
        if ($carte.Name -ne "__NOM__") {
            Rename-NetAdapter -Name $carte.Name -NewName "__NOM__" -ErrorAction SilentlyContinue
        }
        Enable-NetAdapter -Name "__NOM__" -Confirm:$false -ErrorAction SilentlyContinue
        $deja = Get-NetIPAddress -InterfaceAlias "__NOM__" -AddressFamily IPv4 -ErrorAction SilentlyContinue |
            Where-Object { $_.IPAddress -eq "__IP__" }
        if (-not $deja) {
            New-NetIPAddress -InterfaceAlias "__NOM__" -IPAddress "__IP__" -PrefixLength 24 -ErrorAction SilentlyContinue | Out-Null
        }
        $sortie += "__PRETE__"
    } else {
        $sortie += "__ABSENTE__"
    }
} catch {
    $sortie += "ERREUR_POWERSHELL: $_"
}
$sortie -join "`n" | Out-File -FilePath "__RESULTAT__" -Encoding utf8
"#;

/// Traduit la réponse du script de préparation en un verdict : `Ok` si la
/// carte factice est prête, sinon la raison lisible par le gérant.
pub fn lire_preparation(sortie: &str) -> Result<(), String> {
    if sortie.trim().is_empty() {
        return Err(
            "Windows n'a pas exécuté la préparation (aucune réponse). Si la fenêtre \
             d'autorisation est apparue, cliquez « Oui » et réessayez."
                .to_string(),
        );
    }
    if sortie.contains(MARQUEUR_CARTE_PRETE) {
        return Ok(());
    }
    let detail = sortie
        .lines()
        .find(|ligne| ligne.starts_with("CREATION_CARTE:") || ligne.starts_with("ERREUR_POWERSHELL:"))
        .unwrap_or("aucun détail")
        .trim();
    Err(format!(
        "la carte réseau factice dont Windows a besoin n'a pas pu être créée ({detail})."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_script_de_preparation_ne_garde_aucun_trou() {
        let script = script_preparation(Path::new("C:\\Temp\\resultat.txt"));
        for trou in ["__RESULTAT__", "__NOM__", "__IP__", "__SERVICES__", "__PRETE__", "__ABSENTE__", "__PARE_FEU__"] {
            assert!(!script.contains(trou), "{trou} n'a pas été remplacé");
        }
        assert!(script.contains("C:\\Temp\\resultat.txt"));
        assert!(script.contains("10.254.254.1"));
        assert!(script.contains("*MSLOOP"));
        assert!(script.contains("PeerlessTimeoutEnabled"));
        assert!(script.contains("localport=4173"), "le pare-feu doit être ouvert dans la même autorisation");
    }

    #[test]
    fn la_preparation_se_lit_honnetement() {
        assert!(lire_preparation("SERVICES_REACTIVES\nCARTE_FACTICE_PRETE").is_ok());
        assert!(lire_preparation("").is_err());
        let erreur = lire_preparation("SERVICES_REACTIVES\nCREATION_CARTE: pilote 2\nCARTE_FACTICE_ABSENTE")
            .unwrap_err();
        assert!(erreur.contains("pilote 2"));
    }

    #[test]
    fn l_adresse_factice_n_est_jamais_annoncee() {
        assert!(est_adresse_factice(ADRESSE_CARTE_FACTICE));
        assert!(!est_adresse_factice(Ipv4Addr::new(192, 168, 137, 1)));
    }
}

#[cfg(windows)]
mod implementation {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    use windows::core::HSTRING;
    use windows::Foundation::{AsyncStatus, IAsyncAction, IAsyncOperation};
    use windows::Networking::Connectivity::{ConnectionProfile, NetworkInformation};
    use windows::Networking::NetworkOperators::{
        NetworkOperatorTetheringManager, NetworkOperatorTetheringOperationResult, TetheringCapability,
        TetheringOperationStatus, TetheringOperationalState,
    };

    /// Vrai quand c'est l'application qui a allumé le point d'accès. Le
    /// gestionnaire Windows lui-même ne peut pas être gardé d'un fil
    /// d'exécution à l'autre : on le recrée pour l'éteindre.
    static ALLUME_PAR_NOUS: Mutex<bool> = Mutex::new(false);

    /// Délai maximal accordé à chaque opération Windows. Documenté par
    /// d'autres : `StartTetheringAsync` peut ne JAMAIS signaler sa fin. Sans
    /// délai, le bouton du gérant resterait bloqué pour toujours.
    const DELAI: Duration = Duration::from_secs(25);

    pub fn activer(ssid: &str, mot_de_passe: &str) -> Result<(), String> {
        if mot_de_passe.chars().count() < 8 {
            return Err(
                "le mot de passe du Wi-Fi doit faire au moins 8 caractères.".to_string(),
            );
        }
        let profils = profils_a_essayer();
        if profils.is_empty() {
            return Err(
                "Windows ne présente aucune connexion à partager, pas même la carte factice."
                    .to_string(),
            );
        }

        let mut erreurs = Vec::new();
        for (nom, profil) in profils {
            match allumer_depuis(&profil, ssid, mot_de_passe) {
                Ok(()) => {
                    if let Ok(mut garde) = ALLUME_PAR_NOUS.lock() {
                        *garde = true;
                    }
                    return Ok(());
                }
                Err(e) => erreurs.push(format!("« {nom} » : {e}")),
            }
        }
        Err(erreurs.join(" ; "))
    }

    pub fn desactiver() -> Result<(), String> {
        let allume = ALLUME_PAR_NOUS
            .lock()
            .map(|mut garde| std::mem::replace(&mut *garde, false))
            .unwrap_or(false);
        if !allume {
            return Err("aucun point d'accès mobile allumé par l'application".to_string());
        }
        for (_, profil) in profils_a_essayer() {
            let Ok(gestionnaire) =
                NetworkOperatorTetheringManager::CreateFromConnectionProfile(&profil)
            else {
                continue;
            };
            if gestionnaire.TetheringOperationalState() == Ok(TetheringOperationalState::On) {
                if let Ok(operation) = gestionnaire.StopTetheringAsync() {
                    let _ = attendre_resultat(&operation);
                }
            }
        }
        Ok(())
    }

    pub fn est_actif() -> bool {
        ALLUME_PAR_NOUS.lock().map(|garde| *garde).unwrap_or(false)
    }

    /// Les connexions que Windows accepterait de partager : d'abord celle
    /// qui mène à internet s'il y en a une, puis toutes les autres — dont
    /// la carte factice.
    fn profils_a_essayer() -> Vec<(String, ConnectionProfile)> {
        let mut profils = Vec::new();
        if let Ok(internet) = NetworkInformation::GetInternetConnectionProfile() {
            let nom = internet.ProfileName().map(|n| n.to_string()).unwrap_or_default();
            profils.push((nom, internet));
        }
        if let Ok(tous) = NetworkInformation::GetConnectionProfiles() {
            let nombre = tous.Size().unwrap_or(0);
            for indice in 0..nombre {
                if let Ok(profil) = tous.GetAt(indice) {
                    let nom = profil.ProfileName().map(|n| n.to_string()).unwrap_or_default();
                    if !profils.iter().any(|(deja, _)| *deja == nom) {
                        profils.push((nom, profil));
                    }
                }
            }
        }
        profils
    }

    fn allumer_depuis(
        profil: &ConnectionProfile,
        ssid: &str,
        mot_de_passe: &str,
    ) -> Result<(), String> {
        // Windows dit lui-même, avant toute tentative, si ce PC PEUT le
        // faire — et pourquoi il ne le peut pas. C'est la réponse franche que
        // le gérant mérite, au lieu d'une erreur obscure après coup.
        match NetworkOperatorTetheringManager::GetTetheringCapabilityFromConnectionProfile(profil) {
            Ok(TetheringCapability::Enabled) | Err(_) => {}
            Ok(TetheringCapability::DisabledByHardwareLimitation) => {
                return Err(
                    "la carte Wi-Fi de ce PC ne sait PAS créer de point d'accès (limite du \
                     matériel, dite par Windows). Une clé Wi-Fi USB règle ce cas."
                        .to_string(),
                )
            }
            Ok(TetheringCapability::DisabledByGroupPolicy) => {
                return Err(
                    "le point d'accès mobile est interdit sur ce PC par une stratégie de \
                     l'administrateur."
                        .to_string(),
                )
            }
            Ok(autre) => return Err(format!("Windows refuse le partage (raison {})", autre.0)),
        }

        let gestionnaire = NetworkOperatorTetheringManager::CreateFromConnectionProfile(profil)
            .map_err(|e| format!("Windows refuse de partager cette connexion ({e})"))?;

        // Déjà allumé par un essai précédent : l'éteindre, sinon nos réglages
        // ne seraient pas pris en compte.
        if gestionnaire.TetheringOperationalState() == Ok(TetheringOperationalState::On) {
            if let Ok(operation) = gestionnaire.StopTetheringAsync() {
                let _ = attendre_resultat(&operation);
            }
        }

        let configuration = gestionnaire
            .GetCurrentAccessPointConfiguration()
            .map_err(|e| format!("réglages du point d'accès illisibles ({e})"))?;
        configuration
            .SetSsid(&HSTRING::from(ssid))
            .map_err(|e| format!("nom de réseau refusé ({e})"))?;
        configuration
            .SetPassphrase(&HSTRING::from(mot_de_passe))
            .map_err(|e| format!("mot de passe refusé ({e})"))?;
        let action = gestionnaire
            .ConfigureAccessPointAsync(&configuration)
            .map_err(|e| format!("réglages refusés ({e})"))?;
        attendre_action(&action)?;

        let operation = gestionnaire
            .StartTetheringAsync()
            .map_err(|e| format!("démarrage refusé ({e})"))?;
        let resultat = attendre_resultat(&operation)?;
        match resultat.Status() {
            Ok(TetheringOperationStatus::Success) => Ok(()),
            Ok(TetheringOperationStatus::WiFiDeviceOff) => Err(
                "le Wi-Fi de ce PC est éteint (mode avion, ou interrupteur du clavier)."
                    .to_string(),
            ),
            Ok(statut) => {
                let detail = resultat
                    .AdditionalErrorMessage()
                    .map(|m| m.to_string())
                    .unwrap_or_default();
                Err(format!("Windows a refusé de l'allumer (code {}) {detail}", statut.0))
            }
            Err(e) => Err(format!("réponse de Windows illisible ({e})")),
        }
    }

    fn attendre_action(action: &IAsyncAction) -> Result<(), String> {
        let debut = Instant::now();
        loop {
            match action.Status() {
                Ok(AsyncStatus::Completed) => return action.GetResults().map_err(|e| e.to_string()),
                Ok(AsyncStatus::Error) => {
                    return Err(format!(
                        "erreur Windows {}",
                        action.ErrorCode().map(|c| format!("0x{:08X}", c.0)).unwrap_or_default()
                    ))
                }
                Ok(AsyncStatus::Canceled) => return Err("opération annulée".to_string()),
                Ok(_) if debut.elapsed() < DELAI => {
                    std::thread::sleep(Duration::from_millis(200))
                }
                Ok(_) => {
                    let _ = action.Cancel();
                    return Err("Windows n'a pas répondu à temps".to_string());
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    }

    fn attendre_resultat(
        operation: &IAsyncOperation<NetworkOperatorTetheringOperationResult>,
    ) -> Result<NetworkOperatorTetheringOperationResult, String> {
        let debut = Instant::now();
        loop {
            match operation.Status() {
                Ok(AsyncStatus::Completed) => {
                    return operation.GetResults().map_err(|e| e.to_string())
                }
                Ok(AsyncStatus::Error) => {
                    return Err(format!(
                        "erreur Windows {}",
                        operation.ErrorCode().map(|c| format!("0x{:08X}", c.0)).unwrap_or_default()
                    ))
                }
                Ok(AsyncStatus::Canceled) => return Err("opération annulée".to_string()),
                Ok(_) if debut.elapsed() < DELAI => std::thread::sleep(Duration::from_millis(200)),
                Ok(_) => {
                    let _ = operation.Cancel();
                    return Err("Windows n'a pas répondu à temps".to_string());
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    }
}

#[cfg(not(windows))]
mod implementation {
    pub fn activer(_ssid: &str, _mot_de_passe: &str) -> Result<(), String> {
        Err("Disponible uniquement sur Windows".to_string())
    }

    pub fn desactiver() -> Result<(), String> {
        Err("Disponible uniquement sur Windows".to_string())
    }

    pub fn est_actif() -> bool {
        false
    }
}

pub use implementation::{activer, desactiver, est_actif};
