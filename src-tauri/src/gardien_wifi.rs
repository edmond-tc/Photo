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

const PERIODE: Duration = Duration::from_secs(10);

/// Au démarrage, `hotspot::reprendre_point_acces_existant` et la tâche de
/// démarrage de Windows travaillent d'abord (une relance du gardien pendant
/// ce temps ne ferait que refaire la même chose).
const ATTENTE_DEMARRAGE: Duration = Duration::from_secs(30);

/// Posé quand le gérant ouvre la fenêtre du QR : le gardien vérifie tout de
/// suite, sans attendre son tour ni la pause entre deux relances. Un client
/// est sans doute devant lui, prêt à scanner.
static VERIFIER_MAINTENANT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Demande une vérification immédiate du Wi-Fi (fenêtre du QR ouverte).
#[tauri::command]
pub fn wifi_verifier_maintenant() {
    VERIFIER_MAINTENANT.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Attend la prochaine vérification : `PERIODE`, ou moins si le gérant
/// vient d'ouvrir le QR. Rend `true` dans ce second cas.
fn attendre_tour() -> bool {
    let debut = Instant::now();
    while debut.elapsed() < PERIODE {
        if VERIFIER_MAINTENANT.swap(false, std::sync::atomic::Ordering::SeqCst) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

/// Entre deux relances : une relance prend jusqu'à une minute.
const ENTRE_RELANCES: Duration = Duration::from_secs(60);

fn reglage(app: &tauri::AppHandle, cle: &str) -> Option<String> {
    let state = app.state::<crate::db::DbState>();
    let conn = state.0.lock().ok()?;
    crate::db::get_setting(&conn, cle)
}

/// Retient la méthode qui a marché (`Some`), ou que le gérant a coupé le
/// Wi-Fi exprès (`None`) : seul ce geste arrête le gardien.
pub fn retenir_allume(app: &tauri::AppHandle, methode: Option<&str>) {
    let state = app.state::<crate::db::DbState>();
    let Ok(conn) = state.0.lock() else { return };
    let _ = crate::db::set_setting(&conn, "wifi_coupe_par_gerant", if methode.is_some() { "non" } else { "oui" });
    if let Some(m) = methode {
        let _ = crate::db::set_setting(&conn, "wifi_methode", m);
    }
}

/// Le Wi-Fi de la boutique doit-il être allumé ? OUI, par défaut, dès que
/// l'application tourne — comme une box. Constaté sur le terrain : il
/// fallait appuyer sur « Activer le Wi-Fi local » avant que le QR marche.
/// L'ancien réglage (« wifi_garder_allume ») ne passait à « oui » qu'après
/// ce bouton, et la réception directe le remettait à « non » en coupant le
/// Wi-Fi : le gardien restait alors éteint pour toujours, sans le dire.
/// Désormais, seul un arrêt voulu par le gérant l'arrête (ou un routeur à
/// part, qui fait le Wi-Fi à notre place).
pub fn doit_rester_allume(app: &tauri::AppHandle) -> bool {
    reglage(app, "wifi_coupe_par_gerant").as_deref() != Some("oui")
        && reglage(app, "wifi_type_reseau").as_deref() != Some("routeur_externe")
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
        let mut derniere_facade: Option<Instant> = None;
        let mut empeche_veille = false;
        loop {
            let demande_du_gerant = attendre_tour();
            if debut.elapsed() < ATTENTE_DEMARRAGE && !demande_du_gerant {
                continue;
            }
            // La réception directe ne l'arrête plus : le Wi-Fi de la
            // boutique passe avant elle (voir reception_directe.rs).
            let garder = doit_rester_allume(&app);
            // Comme une box : le PC ne s'endort plus tant que le Wi-Fi de
            // la boutique doit rester allumé (en veille, plus de Wi-Fi du
            // tout). L'écran, lui, peut s'éteindre.
            if garder != empeche_veille {
                crate::reception_directe::garder_eveille(garder);
                empeche_veille = garder;
            }
            if !garder {
                continue;
            }
            let methode = reglage(&app, "wifi_methode").unwrap_or_default();
            if reseau_vivant(&methode) {
                echecs_suivis = 0;
                // Réseau hébergé sans l'adresse de façade (PC mis à jour sans
                // réactiver le Wi-Fi) : la tâche de démarrage, qui a déjà les
                // droits, la pose — sans fenêtre « Oui ».
                if methode == "réseau hébergé"
                    && !crate::hotspot::adresse_portail_en_place()
                    && derniere_facade.is_none_or(|t: Instant| t.elapsed() > Duration::from_secs(600))
                {
                    derniere_facade = Some(Instant::now());
                    lancer_tache_demarrage();
                }
                continue;
            }
            // Relances espacées, et de plus en plus après des échecs : une
            // carte vraiment en panne ne doit pas faire tourner le PC en rond.
            let pause = ENTRE_RELANCES * (1 + echecs_suivis.min(9));
            let pause = if demande_du_gerant { Duration::from_secs(15) } else { pause };
            if derniere_relance.is_some_and(|t| t.elapsed() < pause) {
                continue;
            }
            derniere_relance = Some(Instant::now());
            let _ = app.emit("wifi-relance", "en cours");
            if relancer(&app, &methode, echecs_suivis) {
                echecs_suivis = 0;
                let _ = app.emit("wifi-relance", "ok");
            } else {
                echecs_suivis += 1;
                let _ = app.emit("wifi-relance", "echec");
            }
        }
    });
}

/// La tâche Windows de démarrage (réseau hébergé, adresse fixe, adresse de
/// façade), lancée avec les droits qu'elle a déjà.
fn lancer_tache_demarrage() {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("schtasks")
            .args(["/run", "/tn", crate::hotspot::NOM_TACHE_DEMARRAGE])
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }
}

/// Rallume le réseau. Méthode 1 (réseau hébergé) : par la tâche Windows de
/// démarrage, qui a déjà les droits — aucune fenêtre « Oui » au gérant.
/// Autres méthodes : la même activation que le bouton, sans la méthode 1.
fn relancer(app: &tauri::AppHandle, methode: &str, echecs_suivis: u32) -> bool {
    // Après deux échecs par la tâche Windows (tâche effacée, pas encore
    // créée), on refait l'activation complète, comme le bouton : sinon le
    // gardien réessaierait le même chemin cassé indéfiniment.
    if methode == "réseau hébergé" && echecs_suivis < 2 {
        lancer_tache_demarrage();
        std::thread::sleep(Duration::from_secs(20));
        // Après un redémarrage du PC, l'application ne connaît pas encore
        // l'adresse : la reprise la retrouve et relance DHCP et DNS.
        if !crate::hotspot::point_acces_actif() {
            crate::hotspot::reprendre_point_acces_existant(app.clone());
            std::thread::sleep(Duration::from_secs(30));
        }
        return reseau_vivant(methode);
    }

    // Méthode 1 déjà connue pour marcher ici : on la retente en premier.
    // Une autre méthode a marché : on la saute (elle a échoué sur ce PC).
    // Aucune méthode connue (jamais activé) : on essaie tout, comme le bouton.
    let sauter = !methode.is_empty() && methode != "réseau hébergé";
    crate::hotspot::SAUTER_RESEAU_HEBERGE.store(sauter, std::sync::atomic::Ordering::SeqCst);
    let app2 = app.clone();
    let resultat = tauri::async_runtime::block_on(async move {
        crate::commands::activer_point_acces_local(app2.state(), app2.state(), app2.clone()).await
    });
    crate::hotspot::SAUTER_RESEAU_HEBERGE.store(false, std::sync::atomic::Ordering::SeqCst);
    resultat.is_ok()
}
