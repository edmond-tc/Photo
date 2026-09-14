use crate::server::PORT;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use image::{ImageBuffer, Luma};
use qrcode::QrCode;
use std::io::Cursor;

#[derive(serde::Serialize)]
pub struct ServerInfo {
    pub url: String,
    /// QR unique à afficher/imprimer : rejoint le Wi-Fi automatiquement.
    /// La page d'envoi s'ouvre ensuite toute seule sur le plus de téléphones
    /// possible (redirection "portail captif" sur le port 80), et au pire le
    /// client n'a qu'à ouvrir son navigateur une fois connecté.
    pub qr_data_uri: String,
    pub wifi_configure: bool,
}

/// Construit le QR Wi-Fi (SSID + mot de passe configurés dans Réglages) et
/// l'URL de secours de la page d'envoi, à afficher/imprimer pour les
/// clients. Un seul QR : on privilégie la connexion automatique au Wi-Fi
/// plutôt qu'un deuxième code pour l'URL (retour du porteur du projet).
pub fn build_server_info(
    wifi_ssid: Option<String>,
    wifi_mot_de_passe: Option<String>,
) -> Result<ServerInfo, String> {
    let ip = local_ip_address::local_ip().map_err(|_| {
        "Impossible de déterminer l'adresse Wi-Fi locale du PC. Vérifiez que le partage de \
         connexion (Mobile Hotspot) est actif dans les paramètres Windows (bouton ci-dessous). \
         Si ça ne s'active toujours pas : certaines cartes Wi-Fi ne peuvent pas être à la fois \
         connectées à internet ET créer un point d'accès — désactivez temporairement le Wi-Fi \
         internet du PC, ou utilisez le dossier surveillé/la clé USB en attendant."
            .to_string()
    })?;
    let url = format!("http://{ip}:{PORT}/");

    match wifi_ssid.filter(|s| !s.is_empty()) {
        Some(ssid) => {
            let qr_data_uri =
                build_qr_data_uri(&wifi_qr_payload(&ssid, wifi_mot_de_passe.as_deref()))?;
            Ok(ServerInfo {
                url,
                qr_data_uri,
                wifi_configure: true,
            })
        }
        None => {
            // Pas encore configuré : on retombe sur un QR classique (ouvre
            // la page) en attendant que le gérant renseigne le Wi-Fi dans
            // Réglages > Wi-Fi local.
            let qr_data_uri = build_qr_data_uri(&url)?;
            Ok(ServerInfo {
                url,
                qr_data_uri,
                wifi_configure: false,
            })
        }
    }
}

/// Format standard "WIFI:" reconnu nativement par les appareils photo
/// Android et iPhone pour proposer "Rejoindre le réseau" en un geste.
fn wifi_qr_payload(ssid: &str, mot_de_passe: Option<&str>) -> String {
    let echapper = |s: &str| {
        s.chars()
            .flat_map(|c| {
                if matches!(c, '\\' | ';' | ',' | ':' | '"') {
                    vec!['\\', c]
                } else {
                    vec![c]
                }
            })
            .collect::<String>()
    };

    match mot_de_passe.filter(|p| !p.is_empty()) {
        Some(mdp) => format!("WIFI:T:WPA;S:{};P:{};;", echapper(ssid), echapper(mdp)),
        None => format!("WIFI:T:nopass;S:{};;", echapper(ssid)),
    }
}

fn build_qr_data_uri(data: &str) -> Result<String, String> {
    let code = QrCode::new(data.as_bytes()).map_err(|e| e.to_string())?;
    let image: ImageBuffer<Luma<u8>, Vec<u8>> = code.render::<Luma<u8>>().quiet_zone(true).build();

    let mut png_bytes = Cursor::new(Vec::new());
    image
        .write_to(&mut png_bytes, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;

    Ok(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(png_bytes.into_inner())
    ))
}
