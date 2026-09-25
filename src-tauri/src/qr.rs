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
/// Réseau créé par un routeur Wi-Fi dédié (voir `routeur_externe.rs`), pas
/// par le PC : contrairement au point d'accès du PC, le Wi-Fi du routeur
/// fonctionne TOUJOURS, que l'application ait démarré son tour de
/// passe-passe DNS ou non — seule l'ouverture automatique en dépend.
pub const MODE_ROUTEUR_ACTIF: &str = "routeur_actif";
pub const MODE_ROUTEUR_INACTIF: &str = "routeur_inactif";

#[derive(serde::Serialize)]
pub struct ServerInfo {
    pub url: String,
    /// Premier QR. Selon le mode : soit il fait rejoindre le Wi-Fi de la
    /// boutique, soit il ouvre directement la page d'envoi.
    pub qr_data_uri: String,
    /// Second QR, présent UNIQUEMENT quand le premier sert à rejoindre un
    /// Wi-Fi : celui-ci ouvre la page d'envoi.
    ///
    /// Ajouté parce que l'ouverture automatique de la page (le portail
    /// captif, voir `dns.rs`) ne peut PAS être garantie partout : Android se
    /// contente d'une notification que le client peut manquer, et sur un
    /// réseau que nous ne fabriquons pas — partage de connexion d'un
    /// téléphone, routeur mal réglé — c'est ce réseau, pas nous, qui répond
    /// aux questions DNS : l'ouverture automatique n'a alors aucune chance.
    ///
    /// Sans ce second QR, il ne restait qu'une issue dans ces cas : faire
    /// TAPER l'adresse au client. C'est exactement la promesse qu'on refuse
    /// de casser. Deux scans, zéro saisie — et un seul scan dès la
    /// deuxième visite, puisque le téléphone retient le Wi-Fi.
    pub qr_page_data_uri: Option<String>,
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
    type_reseau: Option<String>,
) -> Result<ServerInfo, String> {
    // Même logique que le serveur HTTP lui-même (voir `server::adresse_locale`) :
    // la page servie doit être joignable à l'adresse annoncée dans le QR.
    let url = format!(
        "http://{}{}/",
        crate::server::adresse_locale(),
        suffixe_port()
    );

    let Some(ssid) = wifi_ssid.filter(|s| !s.is_empty()) else {
        return Ok(ServerInfo {
            qr_data_uri: build_qr_data_uri(&url)?,
            // Ce QR ouvre DÉJÀ la page : un second n'aurait rien à ajouter.
            qr_page_data_uri: None,
            url,
            mode: MODE_RESEAU_PARTAGE,
        });
    };

    let mode = choisir_mode(
        type_reseau.as_deref() == Some("routeur_externe"),
        crate::hotspot::point_acces_actif(),
    );

    // Quand le réseau vient d'ailleurs, ce PC ne crée AUCUN Wi-Fi. Le nom
    // enregistré dans les réglages est alors celui que l'application aurait
    // créé — un réseau qui n'existe nulle part. Proposer un QR pour le
    // rejoindre envoie le client dans le vide : constaté en boutique, les
    // codes se génèrent, aucun message d'erreur, et les téléphones
    // n'arrivent pas à rejoindre.
    //
    // On n'annonce donc que le réseau auquel ce PC est RÉELLEMENT connecté.
    // S'il est sur un câble, il n'y a aucun nom à donner : le client est
    // déjà censé être sur le réseau de la boutique, et un seul code — celui
    // de la page — suffit.
    if mode == MODE_ROUTEUR_ACTIF || mode == MODE_ROUTEUR_INACTIF {
        return Ok(match crate::hotspot::ssid_connecte() {
            Some(reel) => ServerInfo {
                qr_data_uri: build_qr_data_uri(&wifi_qr_payload(
                    &reel,
                    wifi_mot_de_passe.as_deref(),
                ))?,
                qr_page_data_uri: Some(build_qr_data_uri(&url)?),
                url,
                mode,
            },
            None => ServerInfo {
                qr_data_uri: build_qr_data_uri(&url)?,
                qr_page_data_uri: None,
                url,
                mode,
            },
        });
    }

    Ok(ServerInfo {
        qr_data_uri: build_qr_data_uri(&wifi_qr_payload(&ssid, wifi_mot_de_passe.as_deref()))?,
        qr_page_data_uri: Some(build_qr_data_uri(&url)?),
        url,
        mode,
    })
}

/// Le port à écrire dans l'adresse du QR : toujours {PORT}, jamais absent.
///
/// Il y a eu une version de cette fonction qui omettait le port quand le
/// portail répondait sur 80 au moment de la lecture. Défaut grave qu'elle
/// causait, découvert sur le terrain : `probleme_portail_captif()` reflète
/// le résultat d'UNE SEULE tentative de connexion au port 80, faite au
/// DÉMARRAGE du serveur — port notoirement disputé (IIS, un vieux Skype,
/// un outil de développement). Selon que cette tentative précise réussit ou
/// échoue, l'adresse écrite dans le QR change d'une activation à l'autre,
/// SUR LE MÊME PC. Un QR imprimé ou affiché à un instant devenait donc
/// injoignable après un simple redémarrage de l'application ou une
/// nouvelle activation du Wi-Fi — le pire défaut possible pour un code
/// destiné à rester affiché en boutique.
///
/// Le port 4173, lui, ne dépend d'aucune contention connue et sert la MÊME
/// page (voir `server::construire_router`). On perd quelques caractères de
/// raccourci ; on gagne la garantie qu'un QR généré une fois reste valide
/// tant que l'adresse du point d'accès ne change pas — et elle est fixe.
fn suffixe_port() -> String {
    format!(":{PORT}")
}

/// Séparée de `build_server_info` pour être testable sans toucher à l'état
/// global du point d'accès ni générer d'image QR.
///
/// Proposer de rejoindre un réseau qui n'est pas allumé enverrait le client
/// dans le vide : le mode le signale pour que l'interface rappelle d'abord
/// d'activer le Wi-Fi local. Distinction importante pour un routeur dédié :
/// son Wi-Fi fonctionne déjà (le routeur, pas nous, le fait tourner) même
/// quand `actif` est faux — "inactif" y signifie seulement que l'ouverture
/// automatique ne se déclenchera pas encore, pas que le réseau est absent
/// comme c'est le cas pour le point d'accès du PC.
fn choisir_mode(routeur_externe: bool, actif: bool) -> &'static str {
    match (routeur_externe, actif) {
        (true, true) => MODE_ROUTEUR_ACTIF,
        (true, false) => MODE_ROUTEUR_INACTIF,
        (false, true) => MODE_POINT_ACCES_ACTIF,
        (false, false) => MODE_POINT_ACCES_INACTIF,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Le cas qui compte le plus : un routeur dédié fait déjà tourner son
    /// Wi-Fi tout seul. Le confondre avec "réseau absent" (le message du
    /// point d'accès du PC) enverrait le gérant vérifier un routeur qui
    /// fonctionne très bien, ou pire, le pousserait à le débrancher.
    #[test]
    fn un_routeur_inactif_cote_application_n_est_pas_un_reseau_absent() {
        assert_eq!(choisir_mode(true, false), MODE_ROUTEUR_INACTIF);
        assert_ne!(choisir_mode(true, false), MODE_POINT_ACCES_INACTIF);
    }

    #[test]
    fn choisit_le_bon_mode_dans_les_quatre_situations() {
        assert_eq!(choisir_mode(false, true), MODE_POINT_ACCES_ACTIF);
        assert_eq!(choisir_mode(false, false), MODE_POINT_ACCES_INACTIF);
        assert_eq!(choisir_mode(true, true), MODE_ROUTEUR_ACTIF);
        assert_eq!(choisir_mode(true, false), MODE_ROUTEUR_INACTIF);
    }
}
