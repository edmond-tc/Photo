//! Ouverture du pare-feu Windows pour la réception des documents.
//!
//! Chaînon manquant découvert en reprenant l'enquête de terrain : même quand
//! le Wi-Fi de la boutique existe vraiment et que le téléphone le rejoint,
//! Windows peut refuser TOUTES les connexions entrantes, et la page d'envoi
//! reste injoignable — exactement le symptôme décrit en boutique ("ça ne
//! rejoint jamais le PC"), sans qu'aucun message ne l'explique.
//!
//! La raison est le profil réseau : un réseau que Windows ne connaît pas est
//! classé "public", et sur un profil public le pare-feu bloque par défaut
//! tout ce qui arrive de l'extérieur. Or c'est précisément le cas de chacun
//! de nos réseaux — celui que le PC crée lui-même, comme le partage de
//! connexion d'un téléphone.
//!
//! Quatre ports sont nécessaires, et aucun ne peut être laissé de côté :
//!
//! - TCP 4173 : la page d'envoi elle-même (voir `server.rs`).
//! - UDP 67 : l'attribution d'adresse aux téléphones (voir `dhcp.rs`) —
//!   sans elle, le téléphone rejoint le Wi-Fi puis reste sans adresse.
//! - UDP 53 : la question « quelle est l'adresse de ce nom ? » (voir
//!   `dns.rs`).
//! - TCP 80 : la question suivante, et c'est celle qui OUVRE la page.
//!   Constaté sur le terrain : avec les trois premiers ports seulement, le
//!   téléphone rejoignait le réseau, recevait son adresse... et rien ne
//!   s'ouvrait. Un téléphone qui rejoint un Wi-Fi demande toujours
//!   « est-ce que ce réseau a vraiment internet ? » en allant chercher une
//!   page de contrôle — sur le port 80, jamais sur un autre. Sans réponse à
//!   cette question précise, il en conclut « réseau sans internet » au lieu
//!   de « réseau qui demande une connexion », et n'ouvre donc rien du tout.
//!
//! Les règles portent un nom fixe, sont recréées à l'identique à chaque
//! appel (donc jamais en double), et n'ouvrent que ces trois ports — rien
//! d'autre de ce PC n'est exposé.

use crate::server::PORT;

/// Préfixe commun des règles créées par l'application. Sert aussi à les
/// reconnaître : tout ce qui commence par ce nom nous appartient, et peut
/// donc être remplacé sans risque d'effacer une règle du gérant.
pub const PREFIXE_REGLE: &str = "Photocopie Benin";

const REGLE_PAGE: &str = "Photocopie Benin - page envoi";
const REGLE_DHCP: &str = "Photocopie Benin - adresses DHCP";
const REGLE_DNS: &str = "Photocopie Benin - noms de domaine";
const REGLE_PORTAIL: &str = "Photocopie Benin - ouverture automatique";

/// Les commandes à insérer dans un script déjà élevé.
///
/// Rendues séparément du script qui les exécute pour qu'un seul et même
/// accord administrateur suffise : elles sont ajoutées à la fin du script
/// d'activation du Wi-Fi (voir `hotspot::script_activation`) plutôt que de
/// déclencher une deuxième fenêtre d'autorisation Windows.
///
/// Chaque règle est supprimée avant d'être recréée : relancer l'application
/// dix fois ne laisse donc jamais dix règles derrière elle.
pub fn commandes_powershell() -> String {
    let regles = [
        (REGLE_PAGE, "TCP", u32::from(PORT)),
        (REGLE_DHCP, "UDP", 67),
        (REGLE_DNS, "UDP", 53),
        (REGLE_PORTAIL, "TCP", u32::from(crate::server::PORT_PORTAIL_CAPTIF)),
    ];

    regles
        .iter()
        .map(|(nom, protocole, port)| {
            format!(
                "$sortie += (netsh advfirewall firewall delete rule name=\"{nom}\" 2>&1 | Out-String)\n\
                 $sortie += (netsh advfirewall firewall add rule name=\"{nom}\" dir=in action=allow \
                 protocol={protocole} localport={port} profile=any 2>&1 | Out-String)"
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Script autonome, pour les installations où l'application ne crée pas le
/// réseau elle-même (box, routeur, partage de connexion d'un téléphone) :
/// il n'y a alors aucun script élevé auquel se greffer, mais le pare-feu
/// bloque tout autant.
#[cfg(windows)]
fn script_autonome(resultat: &std::path::Path) -> String {
    format!(
        "$ErrorActionPreference = 'Continue'\n\
         $sortie = @()\n\
         {commandes}\n\
         $sortie -join \"`n\" | Out-File -FilePath \"{res}\" -Encoding utf8\n",
        commandes = commandes_powershell(),
        res = resultat.display(),
    )
}

/// Les trois règles sont-elles déjà en place ? Lecture seule, sans droits
/// administrateur : sert à ne demander l'autorisation Windows que lorsque
/// c'est réellement nécessaire.
#[cfg(windows)]
pub fn regles_presentes() -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    [REGLE_PAGE, REGLE_DHCP, REGLE_DNS, REGLE_PORTAIL].iter().all(|nom| {
        std::process::Command::new("netsh")
            .args([
                "advfirewall",
                "firewall",
                "show",
                "rule",
                &format!("name={nom}"),
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map(|sortie| {
                regle_trouvee(&String::from_utf8_lossy(&sortie.stdout))
            })
            .unwrap_or(false)
    })
}

#[cfg(not(windows))]
pub fn regles_presentes() -> bool {
    false
}

/// `netsh` annonce explicitement l'absence de règle ; toute autre réponse
/// non vide décrit une règle existante.
///
/// La comparaison ignore les accents, pour la même raison que le reste du
/// diagnostic : la console française ne parle pas UTF-8, et chercher
/// "Aucune règle" tel quel échouerait sur presque tous les PC visés.
fn regle_trouvee(sortie: &str) -> bool {
    let normalisee: String = sortie
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii())
        .collect();

    if normalisee.trim().is_empty() {
        return false;
    }
    // "No rules match the specified criteria." / "Aucune rgle ne correspond
    // aux critres spcifis." une fois les accents retirs.
    !(normalisee.contains("no rules match") || normalisee.contains("aucune rgle"))
}

/// Crée les trois règles, en demandant une fois l'autorisation Windows.
#[cfg(windows)]
pub fn autoriser() -> Result<(), String> {
    let fichier_resultat = std::env::temp_dir().join("photocopie-benin-pare-feu-resultat.txt");
    crate::hotspot::executer_script_eleve(&script_autonome(&fichier_resultat))?;
    Ok(())
}

#[cfg(not(windows))]
pub fn autoriser() -> Result<(), String> {
    Err("Disponible uniquement sur Windows".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Les quatre ports doivent y être : en oublier un ne se verrait pas à
    /// l'écran (le Wi-Fi s'allume, le bandeau est vert) mais casserait la
    /// réception — sans adresse pour le téléphone, ou sans page qui s'ouvre.
    ///
    /// Le port 80 a justement été oublié une première fois, et le terrain
    /// l'a payé : le téléphone rejoignait le réseau, recevait son adresse,
    /// et aucune page ne s'ouvrait jamais.
    #[test]
    fn ouvre_les_quatre_ports_necessaires_et_rien_d_autre() {
        let commandes = commandes_powershell();
        assert!(commandes.contains("protocol=TCP localport=4173"));
        assert!(commandes.contains("protocol=UDP localport=67"));
        assert!(commandes.contains("protocol=UDP localport=53"));
        assert!(
            commandes.contains("protocol=TCP localport=80"),
            "sans le port 80, le téléphone ne peut pas poser la question qui \
             déclenche l'ouverture de la page"
        );
        assert_eq!(
            commandes.matches("add rule").count(),
            4,
            "exactement quatre règles, pas une de plus : rien d'autre de ce PC \
             ne doit être exposé"
        );
        // Entrant seulement, et jamais "autoriser tout".
        assert_eq!(commandes.matches("dir=in action=allow").count(), 4);
        assert!(!commandes.contains("dir=out"));
    }

    /// Relancer l'application ne doit pas empiler les règles : chacune est
    /// supprimée avant d'être recréée.
    #[test]
    fn remplace_les_regles_au_lieu_de_les_empiler() {
        let commandes = commandes_powershell();
        assert_eq!(commandes.matches("delete rule").count(), 4);
        for regle in [REGLE_PAGE, REGLE_DHCP, REGLE_DNS, REGLE_PORTAIL] {
            let suppression = commandes
                .find(&format!("delete rule name=\"{regle}\""))
                .expect("suppression attendue");
            let creation = commandes
                .find(&format!("add rule name=\"{regle}\""))
                .expect("création attendue");
            assert!(suppression < creation, "supprimer AVANT de recréer");
        }
    }

    /// Toutes les règles portent le même préfixe : c'est ce qui permet de
    /// les reconnaître comme les nôtres.
    #[test]
    fn toutes_les_regles_sont_identifiables_comme_les_notres() {
        for regle in [REGLE_PAGE, REGLE_DHCP, REGLE_DNS, REGLE_PORTAIL] {
            assert!(regle.starts_with(PREFIXE_REGLE), "{regle}");
        }
        // Des noms distincts : deux règles de même nom se remplaceraient
        // l'une l'autre, et un port resterait fermé sans que rien ne le dise.
        let noms = [REGLE_PAGE, REGLE_DHCP, REGLE_DNS, REGLE_PORTAIL];
        for (i, a) in noms.iter().enumerate() {
            for b in noms.iter().skip(i + 1) {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn distingue_une_regle_existante_d_une_absence_dans_les_deux_langues() {
        assert!(!regle_trouvee("No rules match the specified criteria."));
        assert!(!regle_trouvee(
            "Aucune r\u{FFFD}gle ne correspond aux crit\u{FFFD}res sp\u{FFFD}cifi\u{FFFD}s."
        ));
        assert!(!regle_trouvee("Aucune règle ne correspond aux critères spécifiés."));
        assert!(!regle_trouvee(""));
        assert!(regle_trouvee(
            "Nom de la règle : Photocopie Benin - page envoi\nActivée : Oui"
        ));

        // Aucun nom de règle ne doit contenir d'apostrophe : il traverse
        // PowerShell puis netsh, et s'y ferait découper.
        for regle in [REGLE_PAGE, REGLE_DHCP, REGLE_DNS, REGLE_PORTAIL] {
            assert!(!regle.contains('\''), "{regle}");
        }
    }
}
