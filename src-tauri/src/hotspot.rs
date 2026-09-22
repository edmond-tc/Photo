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

#[cfg(not(windows))]
pub fn desactiver() -> Result<(), String> {
    Err("Disponible uniquement sur Windows".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
