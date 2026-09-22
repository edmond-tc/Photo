//! Deuxième méthode pour créer le Wi-Fi de la boutique, quand
//! `netsh wlan hostednetwork` (voir `hotspot.rs`) n'est pas supporté.
//!
//! Les deux se complètent au lieu de se remplacer :
//!
//! - `hostednetwork` est l'ancienne méthode. Les pilotes Wi-Fi récents
//!   l'abandonnent de plus en plus (Microsoft a poussé le "Point d'accès
//!   mobile" à la place), mais elle reste présente sur les PC plus anciens —
//!   exactement ceux qu'on trouve en boutique.
//! - Wi-Fi Direct est la méthode moderne, imposée à tous les pilotes
//!   certifiés depuis Windows 8. Son mode "legacy" (`LegacySettings`) crée un
//!   vrai point d'accès avec SSID et mot de passe, que n'importe quel
//!   téléphone rejoint comme un Wi-Fi ordinaire — sans savoir que c'est du
//!   Wi-Fi Direct.
//!
//! Deux avantages décisifs de Wi-Fi Direct ici : il n'exige AUCUNE connexion
//! internet à partager (contrairement au "Point d'accès mobile" des
//! paramètres, qui refuse sur un PC hors ligne), et il ne demande pas les
//! droits administrateur — donc pas de fenêtre d'autorisation Windows.
//!
//! Le point d'accès vit aussi longtemps que l'objet `publisher` : il est donc
//! gardé en mémoire ici, et seul son abandon (ou `desactiver`) coupe le
//! réseau.

#[cfg(windows)]
mod implementation {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    use windows::core::HSTRING;
    use windows::Devices::WiFiDirect::{
        WiFiDirectAdvertisementListenStateDiscoverability, WiFiDirectAdvertisementPublisher,
        WiFiDirectAdvertisementPublisherStatus,
    };
    use windows::Security::Credentials::PasswordCredential;

    /// Tant que cet objet existe, le réseau Wi-Fi existe. Le relâcher le
    /// coupe — d'où la conservation ici plutôt qu'une variable locale.
    static PUBLICATEUR: Mutex<Option<WiFiDirectAdvertisementPublisher>> = Mutex::new(None);

    /// Windows ne démarre pas le point d'accès dans l'instant : il passe de
    /// `Created` à `Started` (ou `Aborted`) de façon asynchrone. On laisse ce
    /// délai avant de conclure.
    const DELAI_DEMARRAGE: Duration = Duration::from_secs(8);

    pub fn activer(ssid: &str, mot_de_passe: &str) -> Result<(), String> {
        // Un mot de passe trop court est refusé par Windows avec une erreur
        // opaque : autant le dire clairement ici.
        if mot_de_passe.chars().count() < 8 {
            return Err(
                "Le mot de passe du Wi-Fi doit faire au moins 8 caractères (règle du Wi-Fi \
                 lui-même, pas de l'application)."
                    .to_string(),
            );
        }

        arreter_publicateur_existant();

        let publicateur = WiFiDirectAdvertisementPublisher::new()
            .map_err(|e| format!("Wi-Fi Direct indisponible sur ce PC ({e})."))?;

        let annonce = publicateur
            .Advertisement()
            .map_err(|e| format!("Wi-Fi Direct inutilisable sur ce PC ({e})."))?;
        annonce
            .SetListenStateDiscoverability(WiFiDirectAdvertisementListenStateDiscoverability::Normal)
            .map_err(|e| format!("Wi-Fi Direct refuse d'être visible ({e})."))?;

        let parametres = annonce
            .LegacySettings()
            .map_err(|e| format!("Ce PC ne sait pas créer un Wi-Fi classique ({e})."))?;
        parametres
            .SetIsEnabled(true)
            .map_err(|e| format!("Impossible d'activer le mode Wi-Fi classique ({e})."))?;
        parametres
            .SetSsid(&HSTRING::from(ssid))
            .map_err(|e| format!("Nom de réseau refusé par Windows ({e})."))?;

        let identifiants = PasswordCredential::new()
            .map_err(|e| format!("Impossible de préparer le mot de passe ({e})."))?;
        identifiants
            .SetPassword(&HSTRING::from(mot_de_passe))
            .map_err(|e| format!("Mot de passe refusé par Windows ({e})."))?;
        parametres
            .SetPassphrase(&identifiants)
            .map_err(|e| format!("Mot de passe refusé par Windows ({e})."))?;

        publicateur
            .Start()
            .map_err(|e| format!("Windows a refusé de démarrer le réseau Wi-Fi ({e})."))?;

        let debut = Instant::now();
        loop {
            match publicateur.Status() {
                Ok(WiFiDirectAdvertisementPublisherStatus::Started) => break,
                Ok(WiFiDirectAdvertisementPublisherStatus::Aborted) => {
                    return Err(
                        "La carte Wi-Fi de ce PC a refusé de créer le réseau (Wi-Fi Direct \
                         interrompu par Windows). Elle est peut-être déjà connectée à un autre \
                         réseau Wi-Fi : déconnectez-la et réessayez."
                            .to_string(),
                    )
                }
                Ok(_) if debut.elapsed() < DELAI_DEMARRAGE => {
                    std::thread::sleep(Duration::from_millis(250))
                }
                Ok(_) => {
                    return Err(
                        "Le réseau Wi-Fi n'a pas démarré dans le délai prévu. Réessayez, ou \
                         vérifiez que le Wi-Fi du PC n'est pas désactivé (mode avion, \
                         interrupteur physique)."
                            .to_string(),
                    )
                }
                Err(e) => return Err(format!("État du réseau Wi-Fi illisible ({e}).")),
            }
        }

        if let Ok(mut garde) = PUBLICATEUR.lock() {
            *garde = Some(publicateur);
        }
        Ok(())
    }

    pub fn desactiver() -> Result<(), String> {
        arreter_publicateur_existant();
        Ok(())
    }

    /// Relâcher l'objet suffit à couper le réseau, mais on demande l'arrêt
    /// explicitement d'abord pour que Windows libère la carte proprement.
    fn arreter_publicateur_existant() {
        if let Ok(mut garde) = PUBLICATEUR.lock() {
            if let Some(publicateur) = garde.take() {
                let _ = publicateur.Stop();
            }
        }
    }

    /// Vrai quand un réseau créé par cette méthode est en cours — sert à
    /// savoir laquelle des deux méthodes a réussi, sans redemander à Windows.
    pub fn est_actif() -> bool {
        PUBLICATEUR
            .lock()
            .map(|garde| garde.is_some())
            .unwrap_or(false)
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
