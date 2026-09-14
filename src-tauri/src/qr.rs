use crate::server::PORT;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use image::{ImageBuffer, Luma};
use qrcode::QrCode;
use std::io::Cursor;

#[derive(serde::Serialize)]
pub struct ServerInfo {
    pub url: String,
    pub qr_data_uri: String,
}

/// Construit l'URL du serveur local (adresse LAN du PC) et le QR code
/// correspondant, à afficher à l'écran pour que les clients scannent.
pub fn build_server_info() -> Result<ServerInfo, String> {
    let ip = local_ip_address::local_ip().map_err(|_| {
        "Impossible de déterminer l'adresse Wi-Fi locale du PC. Vérifiez que le partage de \
         connexion (Mobile Hotspot) est actif dans les paramètres Windows (bouton ci-dessous). \
         Si ça ne s'active toujours pas : certaines cartes Wi-Fi ne peuvent pas être à la fois \
         connectées à internet ET créer un point d'accès — désactivez temporairement le Wi-Fi \
         internet du PC, ou utilisez le dossier surveillé/la clé USB en attendant."
            .to_string()
    })?;

    let url = format!("http://{ip}:{PORT}/");
    let qr_data_uri = build_qr_data_uri(&url)?;
    Ok(ServerInfo { url, qr_data_uri })
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
