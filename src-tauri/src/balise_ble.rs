//! Balise Bluetooth du kiosque : c'est elle qui « pousse » la proposition
//! d'envoi sur les téléphones Android qui ont l'envoyeur.
//!
//! Le PC émet en continu un petit signal Bluetooth basse consommation
//! (quelques octets, sans connexion ni appairage). Le téléphone du client,
//! application fermée, le reconnaît grâce à un filtre confié à Android
//! (voir `android/.../Balayage.kt`) ; si le signal est fort — le téléphone
//! est au guichet, pas dans la cage voisine —, il affiche « Kiosque à
//! côté : envoyer un document ? ». Le client touche, choisit, c'est parti.
//!
//! Contenu : « données fabricant » avec l'identifiant 0xFFFF, réservé par
//! la norme Bluetooth aux essais, suivi de « KQ ».
//!
//! Tous les PC ne savent pas émettre : il faut une carte Bluetooth 4.0 ou
//! plus récente, et un pilote qui accepte ce rôle. Le journal de la
//! réception directe le dit dès le démarrage.

pub const FABRICANT: u16 = 0xFFFF;
pub const DONNEES: &[u8] = b"KQ";

/// Ce que la balise émet : « KQ » puis le numéro du kiosque (3 octets),
/// pour que le téléphone crée son réseau pour CE kiosque.
pub fn donnees_balise(numero: &str) -> Vec<u8> {
    let mut donnees = DONNEES.to_vec();
    donnees.extend((0..3).filter_map(|i| u8::from_str_radix(numero.get(i * 2..i * 2 + 2)?, 16).ok()));
    donnees
}

#[cfg(windows)]
pub fn demarrer(app: tauri::AppHandle) {
    use std::time::Duration;
    use windows::Devices::Bluetooth::Advertisement::{
        BluetoothLEAdvertisementPublisher, BluetoothLEAdvertisementPublisherStatus as Statut,
        BluetoothLEManufacturerData,
    };
    use windows::Storage::Streams::DataWriter;

    fn creer(numero: &str) -> windows::core::Result<BluetoothLEAdvertisementPublisher> {
        let emetteur = BluetoothLEAdvertisementPublisher::new()?;
        let ecrivain = DataWriter::new()?;
        ecrivain.WriteBytes(&donnees_balise(numero))?;
        let donnees = BluetoothLEManufacturerData::Create(FABRICANT, &ecrivain.DetachBuffer()?)?;
        emetteur.Advertisement()?.ManufacturerData()?.Append(&donnees)?;
        Ok(emetteur)
    }

    fn nom(statut: Statut) -> &'static str {
        match statut {
            Statut::Created => "prête",
            Statut::Waiting => "en attente du Bluetooth",
            Statut::Started => "émise",
            Statut::Stopping | Statut::Stopped => "arrêtée",
            Statut::Aborted => "impossible",
            _ => "état inconnu",
        }
    }

    std::thread::spawn(move || {
        let noter = crate::reception_directe::noter;
        let numero = crate::reception_directe::numero_kiosque(&app);
        noter(format!("🏷 Numéro de ce kiosque : {numero}."));
        let emetteur = match creer(&numero) {
            Ok(e) => e,
            Err(e) => {
                noter(format!("❌ Balise Bluetooth indisponible sur ce PC ({e})."));
                return;
            }
        };
        let mut dernier: Option<Statut> = None;
        loop {
            let active = crate::reception_directe::est_active(&app);
            let statut = emetteur.Status().unwrap_or(Statut::Aborted);
            if active && matches!(statut, Statut::Created | Statut::Stopped | Statut::Aborted) {
                if let Err(e) = emetteur.Start() {
                    if dernier != Some(Statut::Aborted) {
                        noter(format!("❌ Balise Bluetooth : démarrage refusé ({e})."));
                        dernier = Some(Statut::Aborted);
                    }
                }
            } else if !active && matches!(statut, Statut::Started | Statut::Waiting) {
                let _ = emetteur.Stop();
            }
            std::thread::sleep(Duration::from_secs(3));
            let statut = emetteur.Status().unwrap_or(Statut::Aborted);
            if active && dernier != Some(statut) {
                let detail = match statut {
                    Statut::Started => " — les téléphones Android avec l'envoyeur l'entendent au guichet.",
                    Statut::Aborted => " — Bluetooth éteint, ou la carte de ce PC ne sait pas émettre.",
                    Statut::Waiting => " — allumez le Bluetooth du PC.",
                    _ => ".",
                };
                noter(format!("📡 Balise Bluetooth {}{detail}", nom(statut)));
                dernier = Some(statut);
            }
            if !active {
                dernier = None;
            }
            std::thread::sleep(Duration::from_secs(7));
        }
    });
}

#[cfg(not(windows))]
pub fn demarrer(_app: tauri::AppHandle) {}

#[cfg(test)]
mod tests {
    /// Doit rester identique à `Reglages.kt` de l'envoyeur Android : sinon
    /// le téléphone n'entend plus le kiosque, sans aucune erreur visible.
    #[test]
    fn meme_balise_que_l_envoyeur_android() {
        let kotlin = include_str!("../../android/app/src/main/java/bj/photocopie/envoyeur/Reglages.kt");
        assert!(kotlin.contains("FABRICANT_BLE = 0xFFFF"));
        assert_eq!(super::FABRICANT, 0xFFFF);
        assert!(kotlin.contains("byteArrayOf('K'.code.toByte(), 'Q'.code.toByte())"));
        assert_eq!(super::DONNEES, b"KQ");
        assert_eq!(super::donnees_balise("3FA92C"), vec![b'K', b'Q', 0x3F, 0xA9, 0x2C]);
        assert!(kotlin.contains(&format!("PREFIXE_RESEAU = \"{}\"", crate::reception_directe::PREFIXE_ENVOYEUR)));
        assert!(kotlin.contains(&format!("PORT_BALISE = {}", crate::reception_directe::PORT_ANNONCE)));
        assert!(kotlin.contains(&format!(
            "MOT_DE_PASSE_PAR_DEFAUT = \"{}\"",
            crate::reception_directe::MOT_DE_PASSE_PAR_DEFAUT
        )));
    }
}
