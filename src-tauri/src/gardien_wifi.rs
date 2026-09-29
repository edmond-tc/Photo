//! Garde le Wi-Fi de la boutique allumé, sans que le gérant y pense.
//!
//! Constaté sur le terrain, version après version : le client scanne le QR
//! et son téléphone ne trouve pas le réseau ; le gérant « rafraîchit »
//! (réappuie sur « Activer le Wi-Fi local ») et ça marche. Le QR n'était
//! pas en cause : c'est le Wi-Fi du PC qui s'était éteint. Windows l'arrête
//! sans prévenir (redémarrage du PC, veille, économie d'énergie de la
//! carte, pilote réinitialisé), et selon la méthode il ne revient pas seul.
//!
//! Le gardien fait ce geste à la place du gérant : toutes les 20 secondes,
//! il vérifie que le réseau tourne VRAIMENT, et sinon le rallume par la
//! même méthode que celle qui a marché sur ce PC.

use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

const PERIODE: Duration = Duration::from_secs(20);

/// Au démarrage, `hotspot::reprendre_point_acces_existant` et la tâche de
/// démarrage de Windows ont une minute pour faire leur travail d'abord.
const ATTENTE_DEMARRAGE: Duration = Duration::from_secs(75);

/// Entre deux relances : une relance prend jusqu'à une minute.
const ENTRE_RELANCES: Duration = Duration::from_secs(60);

fn reglage(app: &tauri::AppHandle, cle: &str) -> Option<String> {
    let state = app.state::<crate::db::DbState>();
    let conn = state.0.lock().ok()?;
    crate::db::get_setting(&conn, cle)
}

/// Le gérant a allumé le Wi-Fi de la boutique (et ne l'a pas coupé).
pub fn retenir_allume(app: &tauri::AppHandle, methode: Option<&str>) {
    let state = app.state::<crate::db::DbState>();
    let Ok(conn) = state.0.lock() else { return };
    let _ = crate::db::set_setting(&conn, "wifi_garder_allume", if methode.is_some() { "oui" } else { "non" });
    if let Some(m) = methode {
        let _ = crate::db::set_setting(&conn, "wifi_methode", m);
    }
}

/// Le réseau tourne-t-il vraiment, selon la méthode qui l'a créé ?
fn reseau_vivant(methode: &str) -> bool {
    if !crate::hotspot::point_acces_actif() {
        return false;
    }
    match methode {
        "Wi-Fi Direct" => crate::wifi_direct::en_marche(),
        m if m == crate::hotspot::METHODE_POINT_ACCES_MOBILE => {
            crate::point_acces_mobile::en_marche()
        }
        "réseau hébergé" => crate::hotspot::reseau_heberge_demarre().unwrap_or(true),
        // Méthode inconnue (ancienne version) : l'adresse suffit.
        _ => true,
    }
}

pub fn demarrer(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let debut = Instant::now();
        let mut derniere_relance: Option<Instant> = None;
        let mut echecs_suivis = 0u32;
        loop {
            std::thread::sleep(PERIODE);
            if debut.elapsed() < ATTENTE_DEMARRAGE {
                continue;
            }
            if reglage(&app, "wifi_garder_allume").as_deref() != Some("oui")
                || reglage(&app, "wifi_type_reseau").as_deref() == Some("routeur_externe")
                || crate::reception_directe::est_active(&app)
            {
                continue;
            }
            let methode = reglage(&app, "wifi_methode").unwrap_or_default();
            if reseau_vivant(&methode) {
                echecs_suivis = 0;
                continue;
            }
            // Relances espacées, et de plus en plus après des échecs : une
            // carte vraiment en panne ne doit pas faire tourner le PC en rond.
            let pause = ENTRE_RELANCES * (1 + echecs_suivis.min(9));
            if derniere_relance.is_some_and(|t| t.elapsed() < pause) {
                continue;
            }
            derniere_relance = Some(Instant::now());
            let _ = app.emit("wifi-relance", "en cours");
            if relancer(&app, &methode) {
                echecs_suivis = 0;
                let _ = app.emit("wifi-relance", "ok");
            } else {
                echecs_suivis += 1;
                let _ = app.emit("wifi-relance", "echec");
            }
        }
    });
}

/// Rallume le réseau. Méthode 1 (réseau hébergé) : par la tâche Windows de
/// démarrage, qui a déjà les droits — aucune fenêtre « Oui » au gérant.
/// Autres méthodes : la même activation que le bouton, sans la méthode 1.
fn relancer(app: &tauri::AppHandle, methode: &str) -> bool {
    if methode == "réseau hébergé" {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let _ = std::process::Command::new("schtasks")
                .args(["/run", "/tn", crate::hotspot::NOM_TACHE_DEMARRAGE])
                .creation_flags(CREATE_NO_WINDOW)
                .status();
        }
        std::thread::sleep(Duration::from_secs(20));
        // Après un redémarrage du PC, l'application ne connaît pas encore
        // l'adresse : la reprise la retrouve et relance DHCP et DNS.
        if !crate::hotspot::point_acces_actif() {
            crate::hotspot::reprendre_point_acces_existant(app.clone());
            std::thread::sleep(Duration::from_secs(30));
        }
        return reseau_vivant(methode);
    }

    crate::hotspot::SAUTER_RESEAU_HEBERGE.store(true, std::sync::atomic::Ordering::SeqCst);
    let app2 = app.clone();
    let resultat = tauri::async_runtime::block_on(async move {
        crate::commands::activer_point_acces_local(app2.state(), app2.state(), app2.clone()).await
    });
    crate::hotspot::SAUTER_RESEAU_HEBERGE.store(false, std::sync::atomic::Ordering::SeqCst);
    resultat.is_ok()
}
