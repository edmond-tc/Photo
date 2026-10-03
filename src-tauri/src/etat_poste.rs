//! L'état du poste, en mots simples, pour le gérant.
//!
//! Une pastille en haut de l'écran dit « Tout va bien » ou ce qui ne va pas,
//! sans jamais montrer de terme technique. Le bouton « Réparer » refait ce
//! qu'on referait à la main, et « Envoyer le rapport » met l'état du PC dans
//! un QR code : le gérant le scanne avec son téléphone, WhatsApp s'ouvre
//! vers le support, message déjà écrit. Le PC n'a pas besoin d'internet.

use crate::db::{self, DbState};
use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, Manager, State};

/// Numéro d'assistance par défaut (celui que les écrans de blocage
/// affichent déjà). Un réglage `assistance_whatsapp` le remplace.
pub const NUMERO_ASSISTANCE: &str = "0151226741";

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Element {
    pub cle: String,
    pub titre: String,
    pub ok: bool,
    /// Un problème grave empêche les clients d'envoyer.
    pub grave: bool,
    /// Le poste est en train de le rétablir tout seul : inutile d'intervenir.
    pub en_cours: bool,
    pub detail: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct EtatPoste {
    /// `ok`, `attention` ou `grave`.
    pub niveau: String,
    /// La phrase de la pastille.
    pub phrase: String,
    pub elements: Vec<Element>,
}

/// Ce qu'on a observé sur le poste (séparé de l'analyse pour la tester).
#[derive(Debug, Clone, Default)]
pub struct Observations {
    pub serveur: bool,
    pub wifi: bool,
    pub wifi_en_relance: bool,
    pub page_auto_probleme: bool,
    pub dossier_ok: bool,
    pub licence_ok: bool,
    pub disque_ok: bool,
}

fn element(cle: &str, titre: &str, ok: bool, grave: bool, detail_ok: &str, detail_ko: &str) -> Element {
    Element {
        cle: cle.to_string(),
        titre: titre.to_string(),
        ok,
        grave: !ok && grave,
        en_cours: false,
        detail: if ok { detail_ok } else { detail_ko }.to_string(),
    }
}

pub fn composer(o: &Observations) -> EtatPoste {
    let wifi_detail = if o.wifi_en_relance {
        "Le Wi-Fi se rallume, patientez un instant."
    } else {
        "Le Wi-Fi de la boutique est éteint. Touchez « Réparer »."
    };
    let mut wifi = element("wifi", "Wi-Fi de la boutique", o.wifi, false, "Allumé.", wifi_detail);
    if o.wifi_en_relance {
        wifi.ok = false;
        wifi.en_cours = true;
    }
    let elements = vec![
        element(
            "licence",
            "Abonnement",
            o.licence_ok,
            true,
            "Actif.",
            "L'abonnement est terminé : les clients ne peuvent plus envoyer. Appelez le support.",
        ),
        element(
            "reception",
            "Réception des documents",
            o.serveur,
            true,
            "Les clients peuvent envoyer leurs documents.",
            "Le logiciel ne reçoit plus les documents. Touchez « Réparer ».",
        ),
        wifi,
        element(
            "page",
            "Page d'envoi sur les téléphones",
            !o.page_auto_probleme,
            false,
            "Elle s'ouvre toute seule.",
            "Elle ne s'ouvre pas toute seule sur certains téléphones : les clients doivent scanner le deuxième code de l'affiche.",
        ),
        element(
            "disque",
            "Place sur le disque",
            o.disque_ok,
            true,
            "Assez de place.",
            "Le disque est presque plein : libérez de la place, sinon les clients ne pourront plus envoyer.",
        ),
        element(
            "dossier",
            "Dossier de réception",
            o.dossier_ok,
            false,
            "Choisi.",
            "Le dossier de réception n'existe plus. Choisissez-en un autre dans le menu ⋮.",
        ),
    ];

    let grave = elements.iter().find(|e| e.grave);
    let autre = elements.iter().find(|e| !e.ok);
    let (niveau, phrase) = match (grave, autre) {
        (Some(e), _) => ("grave", e.detail.clone()),
        (None, Some(e)) => ("attention", e.detail.clone()),
        (None, None) => ("ok", "Tout va bien : les clients peuvent envoyer.".to_string()),
    };
    EtatPoste { niveau: niveau.to_string(), phrase, elements }
}

fn serveur_repond() -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], crate::server::PORT)),
        Duration::from_millis(400),
    )
    .is_ok()
}

fn observer(app: &AppHandle) -> Observations {
    let (routeur, dossier, licence_ok) = {
        let state = app.state::<DbState>();
        let lu = match state.0.lock() {
            Ok(conn) => (
                db::get_setting(&conn, "wifi_type_reseau").as_deref() == Some("routeur_externe"),
                db::get_setting(&conn, "dossier_surveille").filter(|d| !d.trim().is_empty()),
                crate::license::utilisation_autorisee(&conn),
            ),
            Err(_) => (false, None, true),
        };
        lu
    };
    let disque_ok = app
        .path()
        .app_data_dir()
        .map(|d| !crate::server::espace_disque_insuffisant(&d))
        .unwrap_or(true);
    Observations {
        serveur: serveur_repond(),
        wifi: routeur || crate::hotspot::point_acces_actif(),
        wifi_en_relance: crate::gardien_wifi::relance_en_cours(),
        page_auto_probleme: crate::server::probleme_portail_captif().is_some(),
        dossier_ok: dossier.map(|d| std::path::Path::new(&d).is_dir()).unwrap_or(true),
        licence_ok,
        disque_ok,
    }
}

#[tauri::command]
pub async fn etat_du_poste(app: AppHandle) -> Result<EtatPoste, String> {
    tauri::async_runtime::spawn_blocking(move || composer(&observer(&app)))
        .await
        .map_err(|e| e.to_string())
}

/// Ce qu'on refait à la main quand « ça ne marche plus » : le script de
/// démarrage du Wi-Fi, l'ouverture avec Windows et la veille. Si le logiciel
/// ne reçoit plus, il redémarre — c'est ce que tout le monde essaie de toute
/// façon. Le Wi-Fi lui-même est rallumé par l'écran (il demande l'accord de
/// Windows, que seul le gérant peut donner).
#[tauri::command]
pub async fn reparer_poste(app: AppHandle) -> Result<(), String> {
    crate::hotspot::rafraichir_script_demarrage();
    let actif = {
        let state = app.state::<DbState>();
        let conn = state.0.lock().map_err(|e| e.to_string())?;
        crate::permanence::est_active(&conn)
    };
    if actif {
        crate::permanence::appliquer(true);
    }
    let serveur = tauri::async_runtime::spawn_blocking(serveur_repond)
        .await
        .map_err(|e| e.to_string())?;
    if !serveur {
        crate::reception_directe::noter("🔧 Réparation : le logiciel redémarre.");
        app.restart();
    }
    Ok(())
}

// ─────────────────────────── Rapport pour le support ───────────────────────────

#[derive(Serialize)]
pub struct RapportAssistance {
    /// Le rapport complet, à copier.
    pub texte: String,
    pub lien: String,
    /// Image (data URI) du QR code qui ouvre WhatsApp, vide si trop long.
    pub qr: String,
}

fn oui_non(valeur: Option<bool>) -> &'static str {
    match valeur {
        Some(true) => "oui",
        Some(false) => "non",
        None => "?",
    }
}

fn tronquer(texte: &str, max: usize) -> String {
    if texte.chars().count() <= max {
        texte.to_string()
    } else {
        let mut court: String = texte.chars().take(max.saturating_sub(1)).collect();
        court.push('…');
        court
    }
}

/// Encodage des caractères spéciaux d'une adresse (octets UTF-8).
pub fn pourcent(texte: &str) -> String {
    let mut sortie = String::with_capacity(texte.len() * 2);
    for octet in texte.bytes() {
        if octet.is_ascii_alphanumeric() || matches!(octet, b'-' | b'_' | b'.' | b'~') {
            sortie.push(octet as char);
        } else {
            sortie.push_str(&format!("%{octet:02X}"));
        }
    }
    sortie
}

pub fn lien_whatsapp(numero: &str, texte: &str) -> Option<String> {
    let international = crate::server::normalize_phone(numero)?;
    Some(format!("https://wa.me/{international}?text={}", pourcent(texte)))
}

fn numero_assistance(conn: &rusqlite::Connection) -> String {
    db::get_setting(conn, "assistance_whatsapp")
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| NUMERO_ASSISTANCE.to_string())
}

pub struct InfosRapport {
    pub version: String,
    pub systeme: String,
    pub boutique: String,
    pub identifiant: String,
    pub etat: EtatPoste,
    pub carte_wifi: Option<bool>,
    pub wifi_creable: Option<bool>,
    pub bluetooth: Option<bool>,
    pub journal: Vec<String>,
}

/// Court, pour tenir dans un QR code que l'appareil photo d'un téléphone
/// lit sur un écran d'ordinateur.
pub fn message_compact(i: &InfosRapport) -> String {
    let mut message = format!(
        "Rapport Gestion Photocopie v{} | {} | {} | ID {} | {} | carte Wi-Fi {}, création Wi-Fi {}, Bluetooth {}",
        i.version,
        tronquer(&i.systeme, 30),
        tronquer(&i.boutique, 30),
        i.identifiant,
        tronquer(&i.etat.phrase, 110),
        oui_non(i.carte_wifi),
        oui_non(i.wifi_creable),
        oui_non(i.bluetooth),
    );
    if let Some(derniere) = i.journal.first() {
        message.push_str(&format!(" | {}", tronquer(derniere, 80)));
    }
    tronquer(&message, 480)
}

pub fn message_complet(i: &InfosRapport) -> String {
    let mut lignes = vec![
        "Rapport Gestion Photocopie".to_string(),
        format!("Version : {}", i.version),
        format!("Ordinateur : {}", i.systeme),
        format!("Boutique : {}", i.boutique),
        format!("Identifiant : {}", i.identifiant),
        format!("État : {}", i.etat.phrase),
    ];
    for e in &i.etat.elements {
        lignes.push(format!("- {} : {} ({})", e.titre, if e.ok { "OK" } else { "PROBLÈME" }, e.detail));
    }
    lignes.push(format!(
        "Ce PC : carte Wi-Fi {}, création de Wi-Fi {}, Bluetooth {}",
        oui_non(i.carte_wifi),
        oui_non(i.wifi_creable),
        oui_non(i.bluetooth)
    ));
    if !i.journal.is_empty() {
        lignes.push("Dernières lignes du journal :".to_string());
        lignes.extend(i.journal.iter().map(|l| format!("  {l}")));
    }
    lignes.join("\n")
}

#[tauri::command]
pub async fn rapport_assistance(app: AppHandle) -> Result<RapportAssistance, String> {
    let app2 = app.clone();
    let (etat, diag) = tauri::async_runtime::spawn_blocking(move || {
        (composer(&observer(&app2)), crate::hotspot::diagnostiquer())
    })
    .await
    .map_err(|e| e.to_string())?;

    let (boutique, identifiant, numero) = {
        let state = app.state::<DbState>();
        let boutique = {
            let conn = state.0.lock().map_err(|e| e.to_string())?;
            (db::get_setting(&conn, "boutique_nom").unwrap_or_default(), numero_assistance(&conn))
        };
        let identifiant = crate::license::get_license_status(state.clone())
            .map(|l| l.machine_id)
            .unwrap_or_default();
        (boutique.0, identifiant, boutique.1)
    };

    let infos = InfosRapport {
        version: crate::updates::version_actuelle(),
        systeme: sysinfo::System::long_os_version().unwrap_or_else(|| "Windows".to_string()),
        boutique,
        identifiant,
        etat,
        carte_wifi: diag.carte_wifi_presente,
        wifi_creable: diag.reseau_heberge_supporte,
        bluetooth: diag.bluetooth_present,
        journal: crate::reception_directe::derniers_messages(8),
    };
    let lien = lien_whatsapp(&numero, &message_compact(&infos)).unwrap_or_default();
    let qr = crate::qr::build_qr_data_uri(&lien).unwrap_or_default();
    Ok(RapportAssistance { texte: message_complet(&infos), lien, qr })
}

/// Un QR code qui ouvre WhatsApp vers le support avec ce message déjà écrit
/// (écrans de blocage : le gérant n'a qu'à scanner et envoyer).
#[tauri::command]
pub fn qr_whatsapp_support(state: State<DbState>, texte: String) -> Result<String, String> {
    let numero = {
        let conn = state.0.lock().map_err(|e| e.to_string())?;
        numero_assistance(&conn)
    };
    let lien = lien_whatsapp(&numero, &tronquer(&texte, 500)).ok_or("numéro d'assistance invalide")?;
    crate::qr::build_qr_data_uri(&lien)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tout_va_bien() -> Observations {
        Observations {
            serveur: true,
            wifi: true,
            wifi_en_relance: false,
            page_auto_probleme: false,
            dossier_ok: true,
            licence_ok: true,
            disque_ok: true,
        }
    }

    #[test]
    fn tout_va_bien_donne_une_pastille_verte() {
        let e = composer(&tout_va_bien());
        assert_eq!(e.niveau, "ok");
        assert_eq!(e.phrase, "Tout va bien : les clients peuvent envoyer.");
        assert!(e.elements.iter().all(|x| x.ok));
    }

    #[test]
    fn un_serveur_muet_est_grave_et_dit_quoi_faire() {
        let e = composer(&Observations { serveur: false, ..tout_va_bien() });
        assert_eq!(e.niveau, "grave");
        assert!(e.phrase.contains("Réparer"));
    }

    #[test]
    fn un_wifi_eteint_n_est_qu_une_attention() {
        let e = composer(&Observations { wifi: false, ..tout_va_bien() });
        assert_eq!(e.niveau, "attention");
        assert!(e.phrase.contains("Wi-Fi"));
    }

    #[test]
    fn un_wifi_qui_se_rallume_n_alarme_pas() {
        let e = composer(&Observations { wifi: false, wifi_en_relance: true, ..tout_va_bien() });
        assert_eq!(e.niveau, "attention");
        assert!(e.phrase.contains("se rallume"));
    }

    #[test]
    fn le_plus_grave_passe_devant() {
        let e = composer(&Observations { wifi: false, licence_ok: false, ..tout_va_bien() });
        assert_eq!(e.niveau, "grave");
        assert!(e.phrase.contains("abonnement"));
    }

    #[test]
    fn aucun_mot_technique_dans_les_phrases() {
        let tout_casse = Observations::default();
        let e = composer(&tout_casse);
        let texte: String = e.elements.iter().map(|x| format!("{} {}", x.titre, x.detail)).collect::<Vec<_>>().join(" ").to_lowercase();
        for mot in ["ssid", "dhcp", "dns", "hébergé", "port ", "http", "serveur", "portail", "point d'accès"] {
            assert!(!texte.contains(mot), "mot technique : {mot}");
        }
    }

    /// Le gérant ne doit jamais lire un mot de technicien dans ses écrans.
    /// Les lignes de commentaires sont écartées : seul ce qui s'affiche compte.
    #[test]
    fn aucun_mot_de_technicien_dans_les_ecrans_du_gerant() {
        const MAIN_JS: &str = include_str!("../../src/main.js");
        const INDEX_HTML: &str = include_str!("../../src/index.html");
        let affiche = |texte: &str| -> String {
            let mut sans_html = String::new();
            let mut reste = texte;
            while let Some(debut) = reste.find("<!--") {
                sans_html.push_str(&reste[..debut]);
                reste = match reste[debut..].find("-->") {
                    Some(fin) => &reste[debut + fin + 3..],
                    None => "",
                };
            }
            sans_html.push_str(reste);
            sans_html
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
                })
                .collect::<Vec<_>>()
                .join("\n")
                .to_lowercase()
        };
        let interdits = [
            "(ssid)",
            "méthode 1",
            "méthode 2",
            "méthode 3",
            "méthode retenue",
            "méthode essayée",
            "adaptateur",
            "dhcp",
            " dns ",
        ];
        for (nom, texte) in [("main.js", MAIN_JS), ("index.html", INDEX_HTML)] {
            let visible = affiche(texte);
            for mot in interdits {
                assert!(!visible.contains(mot), "{nom} affiche encore « {mot} »");
            }
        }
    }

    #[test]
    fn le_lien_whatsapp_vise_le_bon_numero_et_encode_le_texte() {
        let lien = lien_whatsapp("0151226741", "Bonjour é & 1+1").unwrap();
        assert_eq!(lien, "https://wa.me/2290151226741?text=Bonjour%20%C3%A9%20%26%201%2B1");
    }

    fn infos() -> InfosRapport {
        InfosRapport {
            version: "0.5.35".into(),
            systeme: "Windows 11 Famille".into(),
            boutique: "Cyber Espoir".into(),
            identifiant: "ABCD-1234-EFGH".into(),
            etat: composer(&Observations { wifi: false, ..tout_va_bien() }),
            carte_wifi: Some(true),
            wifi_creable: Some(false),
            bluetooth: None,
            journal: vec!["10:02:11 Canal Bluetooth fermé.".repeat(20)],
        }
    }

    #[test]
    fn le_message_du_qr_reste_court_et_lisible() {
        let m = message_compact(&infos());
        assert!(m.chars().count() <= 480);
        assert!(m.contains("v0.5.35") && m.contains("ABCD-1234-EFGH"));
        assert!(m.contains("création Wi-Fi non") && m.contains("Bluetooth ?"));
        // Et le QR correspondant se fabrique bel et bien.
        let lien = lien_whatsapp(NUMERO_ASSISTANCE, &m).unwrap();
        assert!(crate::qr::build_qr_data_uri(&lien).is_ok());
    }

    #[test]
    fn le_rapport_complet_liste_tout() {
        let m = message_complet(&infos());
        assert!(m.contains("- Wi-Fi de la boutique : PROBLÈME"));
        assert!(m.contains("- Abonnement : OK"));
        assert!(m.contains("Dernières lignes du journal"));
    }
}
