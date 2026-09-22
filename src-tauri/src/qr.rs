use crate::server::PORT;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use image::{ImageBuffer, Luma};
use qrcode::QrCode;
use std::io::Cursor;

/// Le QR ne veut pas dire la même chose selon l'installation de la boutique,
/// et l'interface doit dire au gérant la phrase qui correspond à SA
/// situation — pas une phrase qui suppose un matériel qu'il n'a pas.
pub const MODE_POINT_ACCES_ACTIF: &str = "point_acces_actif";
pub const MODE_POINT_ACCES_INACTIF: &str = "point_acces_inactif";
pub const MODE_RESEAU_PARTAGE: &str = "reseau_partage";

#[derive(serde::Serialize)]
pub struct ServerInfo {
    pub url: String,
    /// QR unique à afficher/imprimer. Selon le mode : soit il fait rejoindre
    /// le Wi-Fi de la boutique, soit il ouvre directement la page d'envoi.
    pub qr_data_uri: String,
    pub mode: &'static str,
}

/// Construit le QR à montrer aux clients, en fonction de ce qui tourne
/// vraiment sur ce PC.
///
/// Deux installations coexistent sur le terrain, et elles n'appellent pas le
/// même QR :
///
/// - Le PC crée lui-même le Wi-Fi de la boutique (voir `hotspot.rs`) : le
///   client doit d'abord rejoindre ce réseau, donc le QR porte le SSID et le
///   mot de passe, et la page s'ouvre ensuite via le portail captif.
/// - Le PC n'a pas de carte Wi-Fi — fréquent sur les tours de bureau — et
///   il est branché en Ethernet à la box ou au routeur déjà présent dans la
///   boutique. Le téléphone est alors DÉJÀ sur le même réseau : le QR doit
///   porter l'adresse de la page, qui s'ouvre d'un seul scan, dans le vrai
///   navigateur. C'est le parcours le plus simple des deux, pas un repli
///   dégradé.
pub fn build_server_info(
    wifi_ssid: Option<String>,
    wifi_mot_de_passe: Option<String>,
) -> Result<ServerInfo, String> {
    // Même logique que le serveur HTTP lui-même (voir `server::adresse_locale`) :
    // la page servie doit être joignable à l'adresse annoncée dans le QR.
    let url = format!("http://{}:{PORT}/", crate::server::adresse_locale());

    let Some(ssid) = wifi_ssid.filter(|s| !s.is_empty()) else {
        return Ok(ServerInfo {
            qr_data_uri: build_qr_data_uri(&url)?,
            url,
            mode: MODE_RESEAU_PARTAGE,
        });
    };

    // Proposer de rejoindre un réseau qui n'est pas allumé enverrait le
    // client dans le vide : le mode le signale pour que l'interface rappelle
    // d'abord d'activer le Wi-Fi local.
    let mode = if crate::hotspot::point_acces_actif() {
        MODE_POINT_ACCES_ACTIF
    } else {
        MODE_POINT_ACCES_INACTIF
    };

    Ok(ServerInfo {
        qr_data_uri: build_qr_data_uri(&wifi_qr_payload(&ssid, wifi_mot_de_passe.as_deref()))?,
        url,
        mode,
    })
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
