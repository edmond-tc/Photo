use crate::watcher::{enqueue_file_avec_options, OptionsImpression};
use axum::extract::{DefaultBodyLimit, Multipart, Path, State};
use axum::http::{StatusCode, Uri};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

pub const PORT: u16 = 4173;
pub const PORT_PORTAIL_CAPTIF: u16 = 80;

/// Limite haute pour un fichier envoyé par un client (au-delà, on refuse
/// proprement plutôt que de laisser le serveur consommer toute la mémoire).
const TAILLE_MAX_ENVOI: usize = 200 * 1024 * 1024; // 200 Mo

/// Envois traités en même temps. Chaque fichier est entièrement chargé en
/// mémoire avant d'être écrit sur le disque : sans cette limite, quelques
/// envois simultanés de 200 Mo suffisent à épuiser la mémoire d'un PC de
/// boutique — et c'est alors Windows entier qui rame, pas seulement
/// l'application. Les envois au-delà attendent leur tour (la connexion
/// reste ouverte) au lieu d'être refusés : un client ne doit jamais voir
/// "échec" simplement parce qu'un autre envoyait au même moment.
const ENVOIS_SIMULTANES_MAX: usize = 3;

static ENVOIS_EN_COURS: std::sync::LazyLock<tokio::sync::Semaphore> =
    std::sync::LazyLock::new(|| tokio::sync::Semaphore::new(ENVOIS_SIMULTANES_MAX));

/// Au-delà, un envoi est abandonné. Large exprès (un gros PDF sur un Wi-Fi
/// de téléphone peut être lent), mais borné : sans délai, trois connexions
/// laissées ouvertes volontairement bloqueraient la réception pour tout le
/// monde.
const DELAI_MAX_ENVOI: std::time::Duration = std::time::Duration::from_secs(600);

/// Nombre de documents acceptés en un seul envoi. Le formulaire n'en propose
/// jamais autant ; la limite vise une requête forgée à la main qui
/// contiendrait des milliers de fichiers minuscules — chacun crée une ligne
/// en base, un fichier sur le disque et une notification à l'écran.
const FICHIERS_MAX_PAR_ENVOI: usize = 20;

const LONGUEUR_MAX_NOM: usize = 120;
const LONGUEUR_MAX_TELEPHONE: usize = 30;
const LONGUEUR_MAX_PLAGE_PAGES: usize = 60;

/// Les seuls formats que la facturation sait traiter. Le menu déroulant de
/// la page client ne propose que ceux-là, mais une requête forgée peut
/// contenir n'importe quoi : ce qui n'est pas reconnu retombe sur A4 plutôt
/// que d'entrer tel quel dans la base.
const FORMATS_ACCEPTES: [&str; 3] = ["A4", "A3", "A5"];

/// Tronque un texte envoyé par un client. Les champs du formulaire n'ont
/// aucune limite côté navigateur : sans cela, un "nom" de plusieurs méga-
/// octets se retrouverait tel quel en base et dans l'écran du gérant.
fn borner_texte(valeur: &str, longueur_max: usize) -> String {
    valeur.trim().chars().take(longueur_max).collect()
}

fn format_papier_valide(valeur: &str) -> String {
    let valeur = valeur.trim().to_uppercase();
    if FORMATS_ACCEPTES.contains(&valeur.as_str()) {
        valeur
    } else {
        "A4".to_string()
    }
}

/// Reflète si le serveur local a effectivement réussi à démarrer. Sans ça,
/// on pourrait afficher un QR code qui pointe vers un serveur mort (ex: port
/// déjà utilisé) sans jamais prévenir le gérant.
#[derive(Default)]
pub struct EtatServeur(pub AtomicBool);

/// Normalise un numéro béninois selon la réforme du 30/11/2024 : le préfixe
/// "01" fait partie intégrante du numéro à 10 chiffres, ce n'est pas un
/// indicatif de tronc à retirer.
pub fn normalize_phone(raw: &str) -> Option<String> {
    let mut digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    if let Some(rest) = digits.strip_prefix("229") {
        digits = rest.to_string();
    }
    if digits.len() == 8 {
        digits = format!("01{digits}");
    }
    Some(format!("229{digits}"))
}

/// Démarre le serveur HTTP local (page de réception QR) dans une tâche
/// asynchrone. Sert la boutique en Wi-Fi local, sans passer par internet.
pub fn start(app: AppHandle) {
    demarrer_portail_captif(app.clone());

    tauri::async_runtime::spawn(async move {
        let app_pour_etat = app.clone();
        let router = construire_router(app);

        let addr = format!("0.0.0.0:{PORT}");
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => {
                app_pour_etat
                    .state::<EtatServeur>()
                    .0
                    .store(true, Ordering::SeqCst);
                if let Err(e) = axum::serve(listener, router).await {
                    eprintln!("Serveur local arrêté avec une erreur : {e}");
                }
                app_pour_etat
                    .state::<EtatServeur>()
                    .0
                    .store(false, Ordering::SeqCst);
            }
            Err(e) => {
                eprintln!(
                    "Impossible de démarrer le serveur local sur le port {PORT} : {e}. \
                     La réception par QR/Wi-Fi local est indisponible, les autres canaux \
                     (dossier surveillé, clé USB) continuent de fonctionner."
                );
            }
        }
    });
}

/// Toute adresse inconnue reçoit LA PAGE elle-même, et non une redirection.
///
/// La différence est décisive, et c'est elle qui manquait. Quand un
/// téléphone rejoint le réseau, il va chercher une page de contrôle
/// (`captive.apple.com` pour iPhone, `connectivitycheck.gstatic.com` pour
/// Android). Il n'attend pas une adresse où aller : il attend un CONTENU.
/// Recevoir une redirection — de surcroît vers un autre port — le laisse
/// hésitant : l'iPhone n'ouvre alors rien, et attend qu'on entre dans les
/// réglages Wi-Fi pour refaire sa vérification. C'est exactement ce que le
/// terrain a constaté : « il faut que j'ouvre mes paramètres Wi-Fi avant que
/// ça ne s'ouvre ».
///
/// Recevoir directement une page qui n'est pas celle attendue lui fait
/// conclure immédiatement « ce réseau demande une connexion » et ouvrir sa
/// fenêtre de portail, sans rien demander à personne.
///
/// Ce comportement était déjà décrit dans le commentaire de
/// `demarrer_portail_captif`, mais le code, lui, redirigeait toujours.
///
/// Le même routeur sert les deux ports : la page doit répondre pareil, quel
/// que soit le chemin par lequel le téléphone arrive.
/// Chemin annoncé aux téléphones par l'option DHCP du portail (voir
/// `dhcp.rs`). Fixe, car il voyage dans une offre DHCP et doit rester le
/// même d'une version à l'autre.
pub const CHEMIN_API_PORTAIL: &str = "/api-portail";

/// Réponse normalisée d'un portail captif (RFC 8908).
///
/// Indispensable en complément de l'option DHCP : celle-ci annonce une
/// ADRESSE, et un téléphone récent ne s'attend pas à y trouver une page
/// web — il attend cette réponse-ci, dans un format précis, qui lui dit en
/// clair « oui, ce réseau est fermé » et « voici la page à ouvrir ».
/// Envoyer l'adresse sans servir cette réponse revient à lui parler une
/// langue qu'il ne comprend qu'à moitié : les clients stricts l'ignorent.
///
/// Le type de contenu compte autant que le contenu : `application/captive+json`
/// est ce à quoi iOS et Android reconnaissent une réponse de portail.
async fn api_portail(State(app): State<AppHandle>) -> impl IntoResponse {
    let _ = app;
    let adresse = adresse_locale();
    (
        [(axum::http::header::CONTENT_TYPE, "application/captive+json")],
        format!(
            r#"{{"captive":true,"user-portal-url":"http://{adresse}/","can-extend-session":true}}"#
        ),
    )
}

fn construire_router(app: AppHandle) -> Router {
    Router::new()
        .route("/envoyer", post(recevoir_fichier))
        .route("/statut/:jeton", get(statut_fichier))
        .route(CHEMIN_API_PORTAIL, get(api_portail))
        // Toute autre adresse, `/` comprise : la page d'envoi, en 200.
        .fallback(page_accueil)
        .layer(DefaultBodyLimit::max(TAILLE_MAX_ENVOI))
        .with_state(app)
}

/// Portail captif, façon Wi-Fi d'hôtel : le téléphone qui rejoint un réseau
/// interroge tout seul une adresse de contrôle connue (Apple pour iPhone,
/// Google pour Android) pour savoir s'il a vraiment internet. Le serveur DNS
/// local (voir `dns.rs`) dirige cette question vers ce PC, et c'est ce
/// serveur-ci qui répond.
///
/// Ce qui compte, c'est de répondre AUTRE CHOSE que la réponse attendue :
/// iOS attend une page contenant "Success" et Android un code 204 vide ;
/// recevoir la page d'envoi à la place est précisément ce qui leur fait
/// conclure "ce réseau demande une connexion" et ouvrir un navigateur.
///
/// On sert donc ici la VRAIE page d'envoi, et non une redirection vers le
/// port {PORT} : la fenêtre qu'ouvre l'iPhone est un navigateur réduit et
/// cloisonné, où une redirection vers un port inhabituel est un risque
/// inutile. Servir directement la page supprime ce détour. Elle pèse ~19 Ko,
/// bien en dessous des ~128 Ko au-delà desquels iOS refuse d'afficher un
/// portail.
///
/// Ce paragraphe décrivait l'intention depuis le début, mais le code
/// redirigeait quand même — l'écart n'a été vu que lorsque le terrain a
/// rapporté qu'il fallait ouvrir les réglages Wi-Fi pour déclencher
/// l'ouverture. Voir `construire_router_portail`.
/// Pourquoi le portail captif ne tourne pas, s'il ne tourne pas.
///
/// Ce défaut s'écrivait jusqu'ici dans un `eprintln!` — c'est-à-dire dans
/// une console qui n'existe pas dans l'application installée. Or c'est
/// exactement le genre de panne qui ne se voit pas : le Wi-Fi s'allume, le
/// téléphone se connecte, tout a l'air normal, et la page ne s'ouvre
/// simplement jamais. Trois pannes de ce projet ont déjà eu cette forme.
static PROBLEME_PORTAIL_CAPTIF: Mutex<Option<String>> = Mutex::new(None);

/// Le portail répond-il VRAIMENT à la question que pose un téléphone ?
///
/// Même raison que pour le serveur de noms : « démarré » ne veut pas dire
/// « répond ». On envoie ici la requête exacte d'un Android rejoignant un
/// réseau — le chemin de contrôle de Google, avec son nom d'hôte — et on
/// exige un 200 portant notre page. C'est cette réponse-là, et pas une
/// autre, qui fait conclure au téléphone « ce réseau demande une connexion ».
pub async fn portail_repond(adresse: Ipv4Addr) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let connexion = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio::net::TcpStream::connect((adresse, PORT_PORTAIL_CAPTIF)),
    )
    .await;
    let Ok(Ok(mut flux)) = connexion else {
        return false;
    };

    let requete = b"GET /generate_204 HTTP/1.0\r\nHost: connectivitycheck.gstatic.com\r\n\r\n";
    if flux.write_all(requete).await.is_err() {
        return false;
    }

    let mut tampon = vec![0u8; 2048];
    let lecture =
        tokio::time::timeout(std::time::Duration::from_secs(2), flux.read(&mut tampon)).await;
    let Ok(Ok(taille)) = lecture else {
        return false;
    };

    let debut = String::from_utf8_lossy(&tampon[..taille]);
    debut.starts_with("HTTP/1.1 200") || debut.starts_with("HTTP/1.0 200")
}

/// La réponse normalisée du portail (RFC 8908) est-elle bien servie, et
/// avec le bon type de contenu ?
///
/// C'est le mécanisme sur lequel repose désormais l'ouverture automatique,
/// et il ne suffit pas qu'il existe : un téléphone n'y reconnaît une
/// réponse de portail que si le type de contenu est exactement celui
/// attendu. Autant le vérifier ici plutôt que sur le téléphone du gérant.
pub async fn api_portail_repond(adresse: Ipv4Addr) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let connexion = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio::net::TcpStream::connect((adresse, PORT_PORTAIL_CAPTIF)),
    )
    .await;
    let Ok(Ok(mut flux)) = connexion else {
        return false;
    };

    let requete = format!("GET {CHEMIN_API_PORTAIL} HTTP/1.0\r\nHost: {adresse}\r\n\r\n");
    if flux.write_all(requete.as_bytes()).await.is_err() {
        return false;
    }

    let mut tampon = vec![0u8; 2048];
    let lecture =
        tokio::time::timeout(std::time::Duration::from_secs(2), flux.read(&mut tampon)).await;
    let Ok(Ok(taille)) = lecture else {
        return false;
    };

    let reponse = String::from_utf8_lossy(&tampon[..taille]);
    reponse.contains(" 200")
        && reponse.contains("application/captive+json")
        && reponse.contains("\"captive\":true")
}

/// Le portail captif a-t-il échoué à démarrer ? Remonté au gérant parmi les
/// avertissements d'activation du Wi-Fi.
pub fn probleme_portail_captif() -> Option<String> {
    PROBLEME_PORTAIL_CAPTIF.lock().ok().and_then(|g| g.clone())
}

fn demarrer_portail_captif(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let router = construire_router(app);
        let addr = format!("0.0.0.0:{PORT_PORTAIL_CAPTIF}");
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => {
                if let Ok(mut garde) = PROBLEME_PORTAIL_CAPTIF.lock() {
                    *garde = None;
                }
                let _ = axum::serve(listener, router).await;
            }
            Err(e) => {
                if let Ok(mut garde) = PROBLEME_PORTAIL_CAPTIF.lock() {
                    *garde = Some(format!(
                        "L'ouverture automatique de la page est indisponible (port \
                         {PORT_PORTAIL_CAPTIF} : {e}). Le Wi-Fi et l'envoi fonctionnent, mais le \
                         client devra scanner le petit second QR pour ouvrir la page. Cause \
                         habituelle : un autre logiciel occupe déjà ce port sur ce PC (serveur \
                         web, Skype ancien, outil de développement)."
                    ));
                }
            }
        }
    });
}

/// L'adresse réelle du PC sur le réseau local, résolue à chaque requête.
///
/// Trouvé sur le terrain : cette fonction devinait autrefois l'adresse de
/// son côté (via `local_ip_address::local_ip()`, qui regarde par quelle
/// carte partirait une connexion vers internet), tandis que `hotspot.rs`
/// déterminait la sienne séparément pour configurer le point d'accès — deux
/// suppositions indépendantes, capables de se contredire. Sur un PC réel
/// équipé d'une carte VPN ou virtuelle, elles se sont effectivement
/// contredites : le QR affichait une adresse à laquelle aucun téléphone ne
/// pouvait jamais arriver, alors que le Wi-Fi Direct venait de démarrer sans
/// problème sur une tout autre adresse.
///
/// Il n'y a plus qu'une seule vérité : si l'application a activé un point
/// d'accès (réseau hébergé ou Wi-Fi Direct), on relit l'adresse qu'elle a
/// elle-même constatée et retenue (`hotspot::adresse_point_acces_active`) —
/// c'est exactement celle que les serveurs DHCP/DNS locaux annoncent aux
/// téléphones. On ne retombe sur la détection générique que si aucun point
/// d'accès de l'application ne tourne (PC directement sur le réseau de la
/// boutique).
pub fn adresse_locale() -> String {
    if let Some(adresse) = crate::hotspot::adresse_point_acces_active() {
        return adresse.to_string();
    }
    // Avant toute autre piste : un point d'accès que Windows fait réellement
    // tourner. Photographié en boutique — le réseau existait, le téléphone y
    // était connecté, et le QR annonçait pourtant l'adresse d'une carte VPN
    // fantôme (`10.10.10.1`), injoignable. La carte du point d'accès, elle,
    // est reconnaissable à coup sûr (voir `hotspot::choisir_adresse_point_acces`).
    if let Some(adresse) = adresse_detectee_en_cache() {
        return adresse.to_string();
    }
    local_ip_address::local_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| "192.168.137.1".to_string())
}

/// Combien de temps une adresse détectée reste valable sans réinterroger
/// Windows. Assez court pour suivre un changement de réseau, assez long
/// pour qu'une même action n'interroge pas le système dix fois.
const DUREE_CACHE_ADRESSE: std::time::Duration = std::time::Duration::from_secs(15);

static ADRESSE_DETECTEE: Mutex<Option<(std::time::Instant, Option<Ipv4Addr>)>> = Mutex::new(None);

/// Trouver l'adresse de ce PC quand aucun point d'accès ne tourne demande
/// d'interroger Windows — deux fois, et chaque interrogation lance un
/// programme système qui met une à trois secondes à répondre.
///
/// Signalé sur le terrain : en ouvrant la fenêtre du QR, l'écran restait
/// figé plusieurs secondes sans rien afficher, au point de croire à un
/// plantage. C'était cette attente. Elle se payait aussi à CHAQUE page
/// servie à un téléphone, ce qui est bien pire : une page lente est une
/// page qu'un portail captif peut abandonner avant qu'elle n'arrive.
///
/// La réponse est donc gardée quelques secondes. Un point d'accès actif,
/// lui, n'a jamais besoin de ce détour : son adresse est connue d'avance
/// (voir `hotspot::adresse_point_acces_active`).
fn adresse_detectee_en_cache() -> Option<Ipv4Addr> {
    if let Ok(garde) = ADRESSE_DETECTEE.lock() {
        if let Some((mesure_le, adresse)) = *garde {
            if mesure_le.elapsed() < DUREE_CACHE_ADRESSE {
                return adresse;
            }
        }
    }

    let adresse =
        crate::hotspot::adresse_point_acces_detectee().or_else(adresse_du_reseau_connecte);
    if let Ok(mut garde) = ADRESSE_DETECTEE.lock() {
        *garde = Some((std::time::Instant::now(), adresse));
    }
    adresse
}

/// L'adresse de ce PC sur le réseau auquel il est RÉELLEMENT relié.
///
/// `local_ip_address::local_ip()` rend la première adresse non-boucle qu'il
/// trouve, sans se demander si elle mène quelque part. Constaté sur le
/// terrain : sur un PC portant une carte VPN ou de machine virtuelle
/// laissée par un ancien logiciel, il a rendu `10.10.10.1` — une adresse
/// que le téléphone du client ne peut évidemment jamais joindre. Le QR
/// annonçait donc une page inaccessible, sans que rien ne le signale.
///
/// On demande plutôt à Windows quelle carte porte une vraie passerelle ET
/// est réellement en service : c'est celle par laquelle le téléphone du
/// client arrivera, que le réseau vienne d'une box, d'un routeur ou du
/// partage de connexion d'un téléphone.
#[cfg(windows)]
pub fn adresse_du_reseau_connecte() -> Option<Ipv4Addr> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let sortie = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            "(Get-NetIPConfiguration -ErrorAction SilentlyContinue | Where-Object { \
              $_.IPv4DefaultGateway -and $_.NetAdapter.Status -eq 'Up' } | \
              Select-Object -First 1).IPv4Address.IPAddress",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;

    premiere_adresse_utilisable(&String::from_utf8_lossy(&sortie.stdout))
}

#[cfg(not(windows))]
pub fn adresse_du_reseau_connecte() -> Option<Ipv4Addr> {
    None
}

/// Première adresse IPv4 exploitable d'une sortie PowerShell. Écarte ce qui
/// ne désigne aucun réseau joignable par un téléphone : la boucle locale
/// (127.x), l'absence d'adresse (0.0.0.0) et les adresses d'auto-attribution
/// (169.254.x) que Windows donne à une carte branchée sur rien.
fn premiere_adresse_utilisable(sortie: &str) -> Option<Ipv4Addr> {
    sortie.lines().find_map(|ligne| {
        ligne
            .trim()
            .parse::<Ipv4Addr>()
            .ok()
            .filter(|ip| !ip.is_loopback() && !ip.is_unspecified() && !ip.is_link_local())
    })
}

/// Le serveur local a-t-il réussi à démarrer ? Utilisé avant d'afficher le
/// QR code pour ne jamais présenter un lien mort au gérant.
pub fn est_actif(app: &AppHandle) -> bool {
    app.state::<EtatServeur>().0.load(Ordering::SeqCst)
}

/// Le nom Bluetooth est saisi par le gérant (Réglages) puis injecté tel
/// quel dans la page HTML servie au client — sans échappement, un caractère
/// comme `<` casserait la page, ou pire, permettrait d'y injecter du HTML.
fn echapper_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Ce que les téléphones ont RÉELLEMENT demandé au serveur de pages.
///
/// Dernier maillon qui manquait. On savait qu'un téléphone recevait son
/// adresse, puis qu'il posait ses questions de noms — mais pas s'il venait
/// ensuite frapper à la porte du portail. Or c'est cette visite-là, et elle
/// seule, qui décide de l'ouverture de la page.
///
/// La signature du navigateur est notée avec : celui d'un iPhone qui vient
/// vérifier son réseau s'annonce « CaptiveNetworkSupport », celui d'Android
/// autrement. On sait donc QUI a frappé, sans avoir à le deviner.
static JOURNAL_PAGES: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn journal_pages() -> Vec<String> {
    JOURNAL_PAGES.lock().map(|j| j.clone()).unwrap_or_default()
}

/// Assez grand pour que deux téléphones tiennent côte à côte.
///
/// Même défaut que celui découvert sur le journal des noms, et il aurait
/// produit la même conclusion fausse : un Android qui se connecte réclame
/// aussitôt une dizaine de pages, et une limite trop courte aurait effacé
/// la visite de l'iPhone — c'est-à-dire précisément la ligne qu'on vient
/// lire.
const MAX_JOURNAL_PAGES: usize = 60;

fn noter_visite(hote: &str, chemin: &str, navigateur: &str) {
    if let Ok(mut journal) = JOURNAL_PAGES.lock() {
        // Le nom du navigateur est long : on n'en garde que le début, qui
        // suffit à distinguer un iPhone d'un Android.
        let court: String = navigateur.chars().take(40).collect();
        let marque = if est_sonde_de_reseau(chemin, hote) {
            "  ⭐ VÉRIFICATION DE RÉSEAU"
        } else {
            ""
        };
        let ligne = format!("{hote}{chemin}  [{court}]{marque}");
        // Une même visite n'apparaît qu'une fois, où qu'elle soit déjà dans
        // la liste. Ne comparer qu'à la dernière ligne laissait repasser la
        // même visite dès qu'une autre s'était glissée entre deux.
        if journal.iter().any(|existante| existante == &ligne) {
            return;
        }
        if journal.len() >= MAX_JOURNAL_PAGES {
            journal.remove(0);
        }
        journal.push(ligne);
    }
}

/// Les machines que les téléphones appellent pour savoir s'ils ont internet.
///
/// Reconnaître le CHEMIN ne suffit pas, et c'est un trou qu'on ne pouvait
/// pas deviner : iOS n'interroge pas seulement
/// `captive.apple.com/hotspot-detect.html`, il interroge aussi
/// `netcts.cdn-apple.com/` — à la racine. Un test par chemin laisse donc
/// passer cette seconde vérification, qui reçoit alors la page d'envoi
/// entière. C'est exactement l'écart rapporté par nodogsplash : leur
/// portail s'affichait pour la première adresse et jamais pour la seconde
/// (nodogsplash/nodogsplash#472).
///
/// La machine appelée, elle, est toujours l'une de celles-ci, quel que
/// soit le chemin.
const MACHINES_DE_CONTROLE: &[&str] = &[
    "captive.apple.com",             // iPhone, iPad, Mac
    "netcts.cdn-apple.com",          // iOS récent, interrogé à la racine
    "connectivitycheck.gstatic.com", // Android
    "connectivitycheck.android.com", // Android
    "clients3.google.com",           // Android, ancienne adresse
    "www.msftconnecttest.com",       // Windows 10 et 11
    "www.msftncsi.com",              // Windows, ancienne adresse
    "detectportal.firefox.com",      // Firefox
    "connectivity-check.ubuntu.com", // Ubuntu
];

/// Les chemins connus, gardés en plus des machines : un téléphone peut
/// appeler par adresse IP, sans nom, et la machine est alors la nôtre.
const CHEMINS_DE_CONTROLE: &[&str] = &[
    "/hotspot-detect.html",
    "/library/test/success.html",
    "/generate_204",
    "/gen_204",
    "/connecttest.txt",
    "/ncsi.txt",
    "/success.txt",
    "/canonical.html",
];

/// Ces visites reçoivent une réponse à part (voir `page_de_controle`) et
/// sont marquées dans le journal, pour qu'on les repère au milieu des
/// autres.
fn est_sonde_de_reseau(chemin: &str, hote: &str) -> bool {
    // Le port fait partie de l'en-tête « Host » et n'appartient pas au nom.
    let hote = hote.to_ascii_lowercase();
    let hote = hote.split(':').next().unwrap_or("");
    if MACHINES_DE_CONTROLE.contains(&hote) {
        return true;
    }
    let chemin = chemin.to_ascii_lowercase();
    CHEMINS_DE_CONTROLE.contains(&chemin.as_str())
}

/// La page servie au téléphone qui vient vérifier son réseau — et à lui
/// seul.
///
/// Elle tient en moins d'un kilo-octet, sans JavaScript, sans feuille de
/// style, sans image. La page d'envoi, elle, en fait vingt mille avec du
/// CSS et du JavaScript : c'est une vraie application, et c'est ce qu'on
/// servait jusqu'ici à la fenêtre de contrôle du téléphone.
///
/// Or cette fenêtre n'est pas un navigateur ordinaire. C'est un affichage
/// réduit, et une page trop lourde ou dont le style le gêne est une cause
/// connue de fenêtre qui ne s'affiche pas du tout. Le téléphone conclut
/// alors « pas d'internet » au lieu de « ce réseau demande une connexion »,
/// et plus rien ne s'ouvre. Tout portail d'hôtel sert une page minuscule à
/// cet instant précis ; nous servions une application entière.
///
/// Le `meta refresh` fait passer le téléphone à la vraie page tout seul.
/// Le lien en dessous reste là pour le cas où il serait ignoré : une page
/// de portail dont on ne peut pas sortir vaut une page qui ne s'ouvre pas.
fn page_de_controle(adresse: &str) -> String {
    format!(
        "<!DOCTYPE html><html lang=\"fr\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<title>Envoyer vos documents</title>\
<meta http-equiv=\"refresh\" content=\"0; url=http://{adresse}/\"></head>\
<body style=\"font-family:sans-serif;text-align:center;padding:2em\">\
<h1 style=\"font-size:1.3em\">Photocopie</h1>\
<p><a href=\"http://{adresse}/\" style=\"font-size:1.2em\">Envoyer vos documents</a></p>\
</body></html>"
    )
}

async fn page_accueil(
    State(app): State<AppHandle>,
    methode: axum::http::Method,
    uri: Uri,
    entetes: axum::http::HeaderMap,
) -> Html<String> {
    let lire = |nom: axum::http::HeaderName| {
        entetes
            .get(nom)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("?")
            .to_string()
    };
    let _ = methode;
    noter_visite(
        &lire(axum::http::header::HOST),
        uri.path(),
        &lire(axum::http::header::USER_AGENT),
    );

    // Le téléphone qui vient vérifier son réseau reçoit la page minuscule,
    // pas l'application entière. On ne touche même pas à la base de
    // données : cette réponse doit partir tout de suite, une fenêtre de
    // contrôle n'attend pas.
    if est_sonde_de_reseau(uri.path(), &lire(axum::http::header::HOST)) {
        return Html(page_de_controle(&adresse_locale()));
    }

    let (whatsapp, bluetooth_nom) = {
        let state = app.state::<crate::db::DbState>();
        let Ok(conn) = state.0.lock() else {
            return Html("<p>Service temporairement indisponible, réessayez.</p>".to_string());
        };
        (
            crate::db::get_setting(&conn, "boutique_whatsapp"),
            crate::db::get_setting(&conn, "bluetooth_nom"),
        )
    };
    Html(construire_page_accueil(whatsapp, bluetooth_nom))
}

/// Séparée de `page_accueil` pour être vérifiable sans base ni serveur : la
/// page change de forme selon les réglages de la boutique, et c'est
/// justement une de ces variantes qui a cassé l'envoi sur le terrain.
fn construire_page_accueil(whatsapp: Option<String>, bluetooth_nom: Option<String>) -> String {
    // Les deux autres façons d'envoyer sont présentées comme des cartes à
    // part entière, en bas de page — et non comme des liens en petit au pied
    // de la page d'envoi. Un client qui n'arrive pas à passer par le Wi-Fi
    // doit voir tout de suite qu'il lui reste deux chemins, pas déchiffrer
    // une ligne grise.
    //
    // WhatsApp ouvre directement la discussion avec le gérant, sans que le
    // client ait à retenir ou recopier un numéro.
    let carte_whatsapp = match whatsapp.as_deref().and_then(normalize_phone) {
        Some(numero) => format!(
            r#"<section class="carte carte-alternative">
      <h2><span>📱</span> Envoyer par WhatsApp</h2>
      <p class="aide">
        Pour envoyer vos documents directement au gérant sur WhatsApp, avec vos
        propres données mobiles.
      </p>
      <p class="avertissement-reseau">
        ⚠️ Le Wi-Fi de la boutique n'a pas internet : WhatsApp ne pourra pas
        envoyer tant que votre téléphone y reste connecté.
      </p>
      <ol class="etapes">
        <li><strong>Coupez le Wi-Fi</strong> de votre téléphone (ou oubliez ce réseau).</li>
        <li>Vérifiez que vos <strong>données mobiles</strong> sont activées.</li>
        <li>Touchez le bouton ci-dessous, puis joignez vos documents.</li>
      </ol>
      <a class="bouton bouton-secondaire" href="whatsapp://send?phone={numero}">
        Ouvrir la discussion WhatsApp
      </a>
      <p class="rappel">
        Si rien ne s'ouvre, WhatsApp n'est pas installé sur ce téléphone :
        <a href="https://wa.me/{numero}" target="_blank" rel="noopener">essayez ce lien</a>.
      </p>
    </section>"#
        ),
        None => String::new(),
    };

    // Sans nom configuré, on reste sur une instruction générique plutôt que
    // de dire au client de chercher un appareil "vide" — mieux vaut ne rien
    // promettre de précis que d'induire en erreur.
    // Le Bluetooth ne consomme aucune donnée et ne dépend d'aucun réseau :
    // c'est le vrai secours quand le Wi-Fi de la boutique ne veut pas.
    //
    // La carte EXPLIQUE au lieu d'agir, et c'est une limite du navigateur,
    // pas un choix : le partage de fichiers depuis une page web n'est
    // autorisé qu'en `https://`, impossible à obtenir pour une adresse
    // locale sans coller un avertissement de sécurité sous les yeux du
    // client. Le bouton natif (voir `#btn-partager-bluetooth`) reste présent
    // et apparaîtra tout seul le jour où la page sera servie autrement.
    //
    // Sans nom configuré, on reste sur une consigne générique plutôt que
    // d'envoyer le client chercher un appareil "vide".
    let carte_bluetooth = match bluetooth_nom.filter(|n| !n.trim().is_empty()) {
        Some(nom) => format!(
            r#"<section class="carte carte-alternative">
      <h2><span>📶</span> Envoyer par Bluetooth</h2>
      <p class="aide">
        Sans internet et sans consommer vos données. Vos documents arrivent
        directement sur l'ordinateur du gérant.
      </p>
      <ol class="etapes">
        <li>Activez le <strong>Bluetooth</strong> sur votre téléphone.</li>
        <li>Ouvrez vos documents, sélectionnez-en un ou plusieurs.</li>
        <li>Appuyez sur <strong>Partager</strong>, puis <strong>Bluetooth</strong>.</li>
        <li>Choisissez l'appareil nommé :</li>
      </ol>
      <p class="bluetooth-nom">{nom}</p>
      <button type="button" id="btn-partager-bluetooth" class="bouton bouton-secondaire" hidden>
        Partager maintenant par Bluetooth
      </button>
      <p class="rappel">
        Si le gérant vous demande d'accepter la connexion sur son écran, c'est normal :
        il doit autoriser votre téléphone une première fois.
      </p>
    </section>"#,
            nom = echapper_html(&nom)
        ),
        None => r#"<section class="carte carte-alternative">
      <h2><span>📶</span> Envoyer par Bluetooth</h2>
      <p class="aide">
        Activez le Bluetooth sur votre téléphone, puis Partager → Bluetooth, et
        choisissez l'ordinateur de la boutique. Demandez son nom au gérant.
      </p>
    </section>"#
            .to_string(),
    };

    format!(
        r#"<!doctype html>
<html lang="fr">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1.0" />
<title>Envoyer un fichier à la boutique</title>
<style>
  :root {{
    --bleu: #2b579a;
    --bleu-clair: #eef3fa;
    --gris-fond: #f3f2f1;
    --gris-bord: #e1dfdd;
    --gris-texte: #605e5c;
    --texte: #252423;
    --espace: 1rem;
  }}
  * {{ box-sizing: border-box; }}
  body {{
    font-family: "Segoe UI", Calibri, Arial, sans-serif;
    background: var(--gris-fond);
    margin: 0;
    padding: var(--espace);
    color: var(--texte);
    line-height: 1.55;
  }}
  .page {{ max-width: 460px; margin: 0 auto; }}

  /* Chaque bloc est une carte distincte, séparée des autres par un vrai
     espace. Constaté sur une photo du terrain : tout était collé, et le
     bouton d'envoi passait par-dessus le texte qui le suivait. */
  .carte {{
    background: #fff;
    border-radius: 10px;
    padding: 1.25rem;
    box-shadow: 0 1px 3px rgba(0,0,0,0.08);
    margin-bottom: var(--espace);
  }}
  .carte:last-child {{ margin-bottom: 0; }}

  h1 {{ font-size: 1.25rem; color: var(--bleu); margin: 0 0 0.35rem; }}
  h2 {{ font-size: 1rem; color: var(--bleu); margin: 0 0 0.5rem; display:flex; align-items:center; gap:0.5rem; }}
  .sous-titre {{ font-size: 0.85rem; color: var(--gris-texte); margin: 0 0 1.25rem; }}

  .champ {{ margin-bottom: 1.1rem; }}
  .champ > label {{ display:block; font-size:0.9rem; font-weight:600; margin-bottom:0.15rem; }}
  .aide {{ font-size: 0.78rem; color: var(--gris-texte); margin: 0 0 0.45rem; }}

  input[type=text], input[type=tel] {{
    width: 100%;
    padding: 0.7rem 0.75rem;
    border: 1px solid var(--gris-bord);
    border-radius: 6px;
    font-size: 1rem;
    background: #fff;
  }}
  input[type=text]:focus, input[type=tel]:focus {{ outline: 2px solid var(--bleu); border-color: var(--bleu); }}
  input[type=file] {{ width: 100%; font-size: 0.9rem; }}

  button, .bouton {{
    display: block;
    width: 100%;
    padding: 0.9rem;
    background: var(--bleu);
    color: #fff;
    border: none;
    border-radius: 6px;
    font-size: 1.05rem;
    font-weight: 600;
    cursor: pointer;
    text-align: center;
    text-decoration: none;
    font-family: inherit;
  }}
  .bouton-secondaire {{
    background: #fff;
    color: var(--bleu);
    border: 1.5px solid var(--bleu);
  }}

  /* Plus aucune marge négative ici : c'est elle qui faisait remonter ce
     texte SOUS le bouton d'envoi. */
  .note-prix {{ font-size: 0.78rem; color: var(--gris-texte); text-align: center; margin: 0.75rem 0 0; }}
  .note-confidentialite {{
    font-size: 0.78rem; color: var(--gris-texte);
    background: var(--gris-fond); padding: 0.85rem; border-radius: 8px; margin: 1.1rem 0 0;
  }}

  #confirmation {{ display:none; text-align:center; color:#107c10; font-weight:600; margin: 1rem 0 0; }}
  #statut-fidelite {{ display:none; text-align:center; background:#dff6dd; color:#107c10; font-weight:600; padding:0.8rem; border-radius:8px; margin: 0.8rem 0 0; }}
  #progression {{ display:none; height:8px; background:var(--gris-bord); border-radius:4px; overflow:hidden; margin: 0 0 0.5rem; }}
  #progression > div {{ height:100%; width:0%; background:var(--bleu); transition:width .15s; }}
  #texte-progression {{ display:none; text-align:center; font-size:0.8rem; color:var(--gris-texte); margin: 0 0 0.75rem; }}

  .fichier {{ border:1px solid var(--gris-bord); border-radius:8px; padding:0.75rem; margin-bottom:0.65rem; }}
  .fichier-entete {{ display:flex; align-items:center; gap:0.5rem; }}
  .fichier-nom {{ flex:1; font-size:0.9rem; font-weight:600; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }}
  .fichier-retirer {{
    width:auto; flex:0 0 auto; background:none; border:none; color:var(--gris-texte);
    font-size:1.1rem; line-height:1; padding:0.2rem 0.35rem; cursor:pointer;
  }}
  .fichier-toggle {{ background:none; border:none; color:var(--bleu); font-size:0.82rem; padding:0.35rem 0 0; cursor:pointer; width:auto; text-align:left; font-weight:500; }}
  .fichier-options {{ display:none; margin-top:0.6rem; font-size:0.85rem; }}
  .fichier-options.ouvert {{ display:block; }}
  .fichier-options label {{ display:flex; align-items:center; gap:0.5rem; margin-bottom:0.5rem; }}
  .fichier-options select, .fichier-options input[type=number], .fichier-options input[type=text] {{
    width:auto; flex:1; padding:0.4rem; margin:0; border:1px solid var(--gris-bord); border-radius:5px;
  }}

  /* Les deux autres façons d'envoyer, en bas, chacune dans sa carte. */
  .carte-alternative {{ background:#fff; border:1px solid var(--gris-bord); box-shadow:none; }}
  .etapes {{ margin: 0.5rem 0 0.9rem; padding-left: 1.15rem; font-size: 0.85rem; color: var(--gris-texte); }}
  .etapes li {{ margin-bottom: 0.35rem; }}
  .bluetooth-nom {{
    text-align:center; font-size:1.3rem; font-weight:700; color:var(--bleu);
    background:var(--bleu-clair); border:2px dashed var(--bleu); border-radius:8px;
    padding:0.85rem 0.6rem; margin:0 0 0.9rem; word-break:break-word; letter-spacing:0.02em;
  }}
  .rappel {{ font-size:0.78rem; color:var(--gris-texte); margin:0.75rem 0 0; }}
  .avertissement-reseau {{
    font-size:0.8rem; background:#fff4ce; border-left:3px solid #d29200;
    padding:0.6rem 0.75rem; border-radius:0 6px 6px 0; margin:0 0 0.5rem;
  }}
</style>
</head>
<body>
  <div class="page">
    <section class="carte">
      <h1>Envoyer vos documents</h1>
      <p class="sous-titre">Choisissez vos fichiers, le gérant les reçoit aussitôt.</p>

      <form id="form-envoi">
        <div class="champ">
          <label for="champ-nom">Votre nom <span style="font-weight:400; color:var(--gris-texte)">(facultatif)</span></label>
          <p class="aide">Aide la boutique à savoir à qui appartient votre fichier, surtout si plusieurs personnes envoient en même temps.</p>
          <input type="text" id="champ-nom" name="nom" placeholder="Votre nom" />
        </div>

        <div class="champ">
          <label for="champ-tel">Votre numéro <span style="font-weight:400; color:var(--gris-texte)">(facultatif)</span></label>
          <p class="aide">Vous permet de profiter d'une réduction après plusieurs commandes chez cette boutique.</p>
          <input type="tel" id="champ-tel" name="telephone" placeholder="Votre numéro" />
        </div>

        <div class="champ">
          <label for="champ-fichiers">Vos documents</label>
          <p class="aide">
            Vous pouvez les ajouter <strong>un par un</strong> : chaque choix s'ajoute
            à la liste, rien ne remplace ce que vous avez déjà mis.
          </p>
          <input type="file" id="champ-fichiers" multiple />
        </div>

        <div id="liste-fichiers"></div>
        <div id="progression"><div></div></div>
        <p id="texte-progression"></p>

        <button type="submit">Envoyer à la boutique</button>
        <p class="note-prix">Le prix est à régler directement avec le gérant, sur place.</p>
      </form>

      <p id="confirmation">Fichier(s) envoyé(s), merci ! Le gérant a été prévenu.</p>
      <p id="statut-fidelite"></p>

      <p class="note-confidentialite">
        🔒 Votre document reste sur l'ordinateur de la boutique : il ne passe
        par aucun site internet et n'est envoyé à personne d'autre. Il est
        effacé automatiquement quelque temps après votre commande. Votre nom
        et votre numéro ne servent qu'à retrouver votre document et à votre
        réduction fidélité ; demandez au gérant si vous voulez qu'ils soient
        effacés.
      </p>
    </section>

    {carte_whatsapp}
    {carte_bluetooth}
  </div>
  <script>
    const champFichiers = document.getElementById('champ-fichiers');
    const listeFichiers = document.getElementById('liste-fichiers');

    // La sélection est CUMULÉE ici, au lieu de vivre dans le champ.
    //
    // Sur iPhone, choisir des documents ne permet souvent d'en prendre qu'un
    // seul à la fois — c'est le sélecteur d'Apple qui l'impose, pas notre
    // page, et la fenêtre réduite qu'ouvre un portail captif est encore plus
    // limitée. Tant que la liste vivait dans le champ, un second choix
    // EFFAÇAIT le premier : le client ne pouvait donc envoyer qu'un seul
    // document, sans que rien ne le prévienne.
    //
    // En gardant la liste de notre côté, ajouter se fait autant de fois
    // qu'on veut — un par un s'il le faut — et l'on peut aussi retirer une
    // ligne. Le champ est vidé après chaque choix, sinon reprendre le même
    // fichier ne déclencherait aucun événement.
    //
    // Chaque ligne garde SES options (Noir & Blanc, A4, 1 copie par défaut,
    // le client ne les voit qu'en cliquant "Personnaliser") : elles vivent
    // dans l'objet et non dans le HTML, pour survivre au réaffichage
    // provoqué par un ajout ou un retrait.
    let fichiersChoisis = [];

    function memeFichier(a, b) {{
      return a.name === b.name && a.size === b.size && a.lastModified === b.lastModified;
    }}

    function afficherFichiers() {{
      listeFichiers.innerHTML = '';
      fichiersChoisis.forEach((choix, i) => {{
        const bloc = document.createElement('div');
        bloc.className = 'fichier';
        bloc.innerHTML = `
          <div class="fichier-entete">
            <div class="fichier-nom">${{choix.fichier.name}}</div>
            <button type="button" class="fichier-retirer" aria-label="Retirer">✕</button>
          </div>
          <button type="button" class="fichier-toggle">Personnaliser (couleur, format, copies)…</button>
          <div class="fichier-options">
            <label><input type="checkbox" class="opt-couleur" /> Couleur (sinon Noir &amp; Blanc)</label>
            <label>Format
              <select class="opt-format">
                <option value="A4">A4</option>
                <option value="A3">A3</option>
                <option value="A5">A5</option>
              </select>
            </label>
            <label>Copies <input type="number" class="opt-copies" min="1" value="1" /></label>
            <label>Pages (ex: 1-5) <input type="text" class="opt-pages" placeholder="toutes" /></label>
          </div>
        `;
        const couleur = bloc.querySelector('.opt-couleur');
        const format = bloc.querySelector('.opt-format');
        const copies = bloc.querySelector('.opt-copies');
        const pages = bloc.querySelector('.opt-pages');
        couleur.checked = choix.couleur;
        format.value = choix.format;
        copies.value = choix.copies;
        pages.value = choix.pages;
        couleur.addEventListener('change', () => {{ choix.couleur = couleur.checked; }});
        format.addEventListener('change', () => {{ choix.format = format.value; }});
        copies.addEventListener('change', () => {{ choix.copies = copies.value || '1'; }});
        pages.addEventListener('input', () => {{ choix.pages = pages.value; }});

        bloc.querySelector('.fichier-toggle').addEventListener('click', () => {{
          bloc.querySelector('.fichier-options').classList.toggle('ouvert');
        }});
        bloc.querySelector('.fichier-retirer').addEventListener('click', () => {{
          fichiersChoisis.splice(i, 1);
          afficherFichiers();
        }});
        listeFichiers.appendChild(bloc);
      }});
      majBoutonBluetooth();
    }}

    champFichiers.addEventListener('change', () => {{
      [...champFichiers.files].forEach((fichier) => {{
        if (!fichiersChoisis.some((c) => memeFichier(c.fichier, fichier))) {{
          fichiersChoisis.push({{ fichier, couleur: false, format: 'A4', copies: '1', pages: '' }});
        }}
      }});
      // Vidé pour que rechoisir le même fichier déclenche bien un événement.
      champFichiers.value = '';
      afficherFichiers();
    }});

    // Le bouton Bluetooth n'apparaît que si le téléphone sait le faire
    // (surtout Android) et que des fichiers sont bien sélectionnés —
    // sinon les instructions manuelles restent le seul recours.
    // Ce bouton N'EXISTE PAS quand aucun nom Bluetooth n'est configuré : sa
    // carte affiche alors une consigne générale, sans bouton.
    //
    // Sans les gardes ci-dessous, le script levait une erreur ici même, et
    // s'arrêtait AVANT d'installer l'interception du formulaire. Le
    // navigateur retombait alors sur l'envoi classique d'un formulaire HTML :
    // la page se rechargeait, vide, et les fichiers ne partaient jamais.
    // Constaté sur le terrain, sur les deux téléphones à la fois.
    //
    // Règle qui en découle : tout élément facultatif de cette page doit être
    // lu avec une garde. Un détail absent ne doit jamais pouvoir emporter
    // l'envoi lui-même, qui est la seule chose indispensable ici.
    const btnBluetooth = document.getElementById('btn-partager-bluetooth');
    function majBoutonBluetooth() {{
      if (!btnBluetooth) return;
      const fichiers = fichiersChoisis.map((c) => c.fichier);
      const peutPartager =
        fichiers.length > 0 &&
        typeof navigator.canShare === 'function' &&
        navigator.canShare({{ files: fichiers }});
      btnBluetooth.hidden = !peutPartager;
    }}
    if (btnBluetooth) {{
      btnBluetooth.addEventListener('click', async () => {{
        try {{
          await navigator.share({{ files: fichiersChoisis.map((c) => c.fichier) }});
        }} catch {{
          // Annulé par le client, ou échec — pas grave, il peut toujours
          // utiliser "Envoyer à la boutique" ou les instructions manuelles.
        }}
      }});
    }}

    // Créé ici, pendant le clic (geste utilisateur) — les téléphones
    // bloquent le son créé plus tard par du code, mais celui-ci reste
    // utilisable pour le petit bip joué à la fin, une fois débloqué ainsi.
    let audioClient = null;

    document.getElementById('form-envoi').addEventListener('submit', (e) => {{
      e.preventDefault();
      try {{
        audioClient = new (window.AudioContext || window.webkitAudioContext)();
      }} catch {{}}
      const form = e.target;
      if (fichiersChoisis.length === 0) {{
        alert('Choisissez au moins un document avant d\'envoyer.');
        return;
      }}
      const donnees = new FormData();
      donnees.append('nom', form.nom.value);
      donnees.append('telephone', form.telephone.value);
      fichiersChoisis.forEach((choix, i) => {{
        donnees.append(`fichier_${{i}}`, choix.fichier);
        donnees.append(`couleur_${{i}}`, choix.couleur ? '1' : '0');
        donnees.append(`format_${{i}}`, choix.format || 'A4');
        donnees.append(`copies_${{i}}`, choix.copies || '1');
        donnees.append(`pages_${{i}}`, choix.pages || '');
      }});
      donnees.append('nombre_fichiers', String(fichiersChoisis.length));

      const bouton = form.querySelector('button');
      const barre = document.querySelector('#progression');
      const remplissage = barre.querySelector('div');
      const texte = document.querySelector('#texte-progression');
      bouton.disabled = true;
      bouton.textContent = 'Envoi en cours…';
      barre.style.display = 'block';
      texte.style.display = 'block';

      const xhr = new XMLHttpRequest();
      xhr.open('POST', '/envoyer');
      xhr.upload.addEventListener('progress', (ev) => {{
        if (!ev.lengthComputable) return;
        const pourcent = Math.round((ev.loaded / ev.total) * 100);
        remplissage.style.width = pourcent + '%';
        texte.textContent = `Envoi… ${{pourcent}}%  (patientez si le fichier est volumineux)`;
      }});
      xhr.addEventListener('load', () => {{
        if (xhr.status >= 200 && xhr.status < 300) {{
          form.hidden = true;
          barre.style.display = 'none';
          texte.style.display = 'none';
          document.getElementById('confirmation').style.display = 'block';
          try {{
            const reponse = JSON.parse(xhr.responseText);
            if (reponse.jetons && reponse.jetons.length) surveillerStatut(reponse.jetons);
          }} catch {{}}
        }} else {{
          bouton.disabled = false;
          bouton.textContent = 'Envoyer à la boutique';
          alert("L'envoi a échoué, réessayez.");
        }}
      }});
      xhr.addEventListener('error', () => {{
        bouton.disabled = false;
        bouton.textContent = 'Envoyer à la boutique';
        alert("L'envoi a échoué, réessayez.");
      }});
      xhr.send(donnees);
    }});

    // Tant que le client reste sur le Wi-Fi de la boutique (donc pas encore
    // parti), on regarde discrètement si le gérant a encaissé — pour lui
    // montrer un mot de remerciement en direct, sans imprimer de reçu ni
    // passer par internet.
    function jouerSonClient() {{
      try {{
        const ctx = audioClient || new (window.AudioContext || window.webkitAudioContext)();
        const osc = ctx.createOscillator();
        const gain = ctx.createGain();
        osc.type = 'sine';
        osc.frequency.value = 880;
        gain.gain.setValueAtTime(0.18, ctx.currentTime);
        gain.gain.exponentialRampToValueAtTime(0.001, ctx.currentTime + 0.35);
        osc.connect(gain).connect(ctx.destination);
        osc.start();
        osc.stop(ctx.currentTime + 0.35);
      }} catch {{}}
      if (navigator.vibrate) navigator.vibrate(200);
    }}

    function surveillerStatut(jetons) {{
      const statutEl = document.getElementById('statut-fidelite');
      let tentatives = 0;
      const maxTentatives = 200; // ~15 minutes, le temps d'un passage en boutique
      const minuteur = setInterval(async () => {{
        tentatives++;
        if (tentatives > maxTentatives) {{
          clearInterval(minuteur);
          return;
        }}
        for (const jeton of jetons) {{
          try {{
            const r = await fetch(`/statut/${{jeton}}`);
            const data = await r.json();
            if (data.paye && data.message) {{
              statutEl.textContent = data.message;
              statutEl.style.display = 'block';
              jouerSonClient();
              clearInterval(minuteur);
              return;
            }}
          }} catch {{}}
        }}
      }}, 4500);
    }}
  </script>
</body>
</html>"#
    )
}

struct FichierRecu {
    original_name: String,
    bytes: Vec<u8>,
}

/// Marge sous laquelle on refuse d'écrire un nouveau fichier reçu.
const ESPACE_DISQUE_MINIMUM: u64 = 500 * 1024 * 1024; // 500 Mo

fn espace_disque_insuffisant(data_dir: &std::path::Path) -> bool {
    let disques = sysinfo::Disks::new_with_refreshed_list();
    // On retient le disque dont le point de montage correspond le plus
    // précisément au dossier de données (sur Windows : la bonne lettre de
    // lecteur). Si on n'arrive pas à le déterminer, on laisse passer plutôt
    // que de bloquer à tort un client qui attend son document.
    disques
        .iter()
        .filter(|d| data_dir.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .is_some_and(|d| d.available_space() < ESPACE_DISQUE_MINIMUM)
}

/// Ne garde que le nom de fichier, sans le chemin — un client malveillant
/// pourrait sinon envoyer un nom du type "../../Windows/Startup/x.exe" pour
/// écrire en dehors du dossier de réception (faille de traversée de chemin).
///
/// Publique parce que TOUS les canaux d'arrivée doivent s'en servir, pas
/// seulement l'envoi par le Wi-Fi : la réception Bluetooth (voir `obex.rs`)
/// reçoit elle aussi un nom choisi par l'appareil d'en face. Deux fonctions
/// séparées finiraient par diverger, et la protection la plus faible
/// deviendrait la porte d'entrée.
pub fn nom_fichier_sans_chemin(nom_brut: &str) -> String {
    let nom = nom_brut
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(nom_brut)
        .trim();
    // Retire aussi les caractères interdits dans un nom de fichier Windows et
    // les caractères de contrôle, sinon std::fs::write échoue silencieusement
    // plus loin et le fichier reçu disparaît sans que personne ne s'en rende
    // compte (contredit la garantie de traçabilité de l'appli).
    let nom: String = nom
        .chars()
        .filter(|c| !c.is_control() && !r#":*?"<>|"#.contains(*c))
        .take(150)
        .collect();
    let nom = nom.trim();

    const NOMS_RESERVES_WINDOWS: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let base = nom.split('.').next().unwrap_or(nom).to_uppercase();

    if nom.is_empty() || nom == "." || nom == ".." || NOMS_RESERVES_WINDOWS.contains(&base.as_str())
    {
        "fichier_recu".to_string()
    } else {
        nom.to_string()
    }
}

async fn recevoir_fichier(State(app): State<AppHandle>, multipart: Multipart) -> impl IntoResponse {
    // Attend son tour si trois envois sont déjà en cours (voir
    // ENVOIS_SIMULTANES_MAX) : le permis est pris AVANT de lire le corps de
    // la requête, sinon la mémoire serait déjà consommée au moment où on
    // voudrait la limiter.
    let _permis = match ENVOIS_EN_COURS.acquire().await {
        Ok(p) => p,
        Err(_) => return (StatusCode::SERVICE_UNAVAILABLE, "erreur serveur").into_response(),
    };

    let data_dir = match app.path().app_data_dir() {
        Ok(d) => d,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "erreur serveur").into_response(),
    };

    // Vérifié AVANT de charger le moindre octet en mémoire : inutile de
    // recevoir 200 Mo pour découvrir ensuite qu'il n'y a plus la place de
    // les écrire. La page d'envoi étant ouverte à tout le Wi-Fi de la
    // boutique, quelqu'un peut sinon remplir le disque — et une fois le
    // disque plein, c'est Windows entier qui devient inutilisable.
    if espace_disque_insuffisant(&data_dir) {
        return (
            StatusCode::INSUFFICIENT_STORAGE,
            "L'ordinateur de la boutique n'a plus assez d'espace. Prévenez le gérant.",
        )
            .into_response();
    }

    match tokio::time::timeout(DELAI_MAX_ENVOI, lire_envoi(multipart)).await {
        Ok(Some(envoi)) => enregistrer_envoi(&app, &data_dir, envoi).await,
        Ok(None) => (StatusCode::BAD_REQUEST, "aucun fichier reçu").into_response(),
        Err(_) => (
            StatusCode::REQUEST_TIMEOUT,
            "L'envoi a pris trop de temps, réessayez.",
        )
            .into_response(),
    }
}

struct EnvoiClient {
    nom: Option<String>,
    telephone: Option<String>,
    fichiers: std::collections::HashMap<usize, FichierRecu>,
    options: std::collections::HashMap<usize, OptionsImpression>,
}

/// Lit la requête du client. Renvoie `None` si elle ne contient aucun
/// fichier exploitable. Tout ce qui vient d'ici est saisi par un inconnu
/// connecté au Wi-Fi de la boutique : chaque champ est borné, jamais repris
/// tel quel.
async fn lire_envoi(mut multipart: Multipart) -> Option<EnvoiClient> {
    let mut nom: Option<String> = None;
    let mut telephone: Option<String> = None;
    let mut fichiers: std::collections::HashMap<usize, FichierRecu> =
        std::collections::HashMap::new();
    let mut options: std::collections::HashMap<usize, OptionsImpression> =
        std::collections::HashMap::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();

        if name == "nom" {
            if let Ok(v) = field.text().await {
                let v = borner_texte(&v, LONGUEUR_MAX_NOM);
                if !v.is_empty() {
                    nom = Some(v);
                }
            }
            continue;
        }
        if name == "telephone" {
            if let Ok(v) = field.text().await {
                telephone = normalize_phone(&borner_texte(&v, LONGUEUR_MAX_TELEPHONE));
            }
            continue;
        }
        if name == "nombre_fichiers" {
            let _ = field.text().await;
            continue;
        }

        let Some((prefixe, indice)) = name.rsplit_once('_') else {
            continue;
        };
        let Ok(indice) = indice.parse::<usize>() else {
            continue;
        };

        match prefixe {
            "fichier" => {
                // Au-delà de la limite, les fichiers suivants sont ignorés
                // en silence plutôt que de faire échouer tout l'envoi : les
                // premiers documents du client sont bien reçus.
                if fichiers.len() >= FICHIERS_MAX_PAR_ENVOI {
                    continue;
                }
                let original_name = field
                    .file_name()
                    .map(nom_fichier_sans_chemin)
                    .unwrap_or_else(|| "fichier_recu".to_string());
                if let Ok(bytes) = field.bytes().await {
                    if !bytes.is_empty() {
                        fichiers.insert(
                            indice,
                            FichierRecu {
                                original_name,
                                bytes: bytes.to_vec(),
                            },
                        );
                    }
                }
            }
            "couleur" => {
                if let Ok(v) = field.text().await {
                    options.entry(indice).or_default().couleur = v == "1";
                }
            }
            "format" => {
                if let Ok(v) = field.text().await {
                    options.entry(indice).or_default().format_papier =
                        Some(format_papier_valide(&v));
                }
            }
            "copies" => {
                if let Ok(v) = field.text().await {
                    if let Ok(n) = v.parse::<i64>() {
                        // Le "min=1" du formulaire HTML est côté client, donc
                        // contournable par une requête forgée ; on borne ici
                        // pour éviter un débordement lors du calcul du prix.
                        options.entry(indice).or_default().copies = Some(n.clamp(1, 500));
                    }
                }
            }
            "pages" => {
                if let Ok(v) = field.text().await {
                    let v = borner_texte(&v, LONGUEUR_MAX_PLAGE_PAGES);
                    if !v.is_empty() {
                        options.entry(indice).or_default().plage_pages = Some(v);
                    }
                }
            }
            _ => {}
        }
    }

    if fichiers.is_empty() {
        return None;
    }
    Some(EnvoiClient {
        nom,
        telephone,
        fichiers,
        options,
    })
}

async fn enregistrer_envoi(
    app: &AppHandle,
    data_dir: &std::path::Path,
    envoi: EnvoiClient,
) -> axum::response::Response {
    let EnvoiClient {
        nom,
        telephone,
        fichiers,
        mut options,
    } = envoi;

    let recus_dir = data_dir.join("recus");
    if std::fs::create_dir_all(&recus_dir).is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, "erreur serveur").into_response();
    }

    let nombre_recus = fichiers.len();
    let mut nombre_enregistres = 0usize;
    let mut jetons: Vec<String> = Vec::new();
    for (indice, fichier) in fichiers {
        let horodatage = chrono::Local::now().format("%Y%m%d-%H%M%S%3f");
        let nom_fichier_sur_disque = format!("{horodatage}_{}_{}", indice, fichier.original_name);
        let chemin = recus_dir.join(&nom_fichier_sur_disque);
        if std::fs::write(&chemin, &fichier.bytes).is_err() {
            continue;
        }
        let opts = options.remove(&indice).unwrap_or_default();
        if let Some((_id, jeton)) = enqueue_file_avec_options(
            &app,
            &chemin,
            "qr",
            nom.as_deref(),
            telephone.as_deref(),
            opts,
        ) {
            nombre_enregistres += 1;
            jetons.push(jeton);
        }
    }

    // Ne jamais répondre "ok" si rien n'a pu être enregistré : le client
    // verrait "Fichier envoyé, merci !" alors que la boutique n'a rien reçu,
    // ce qui contredit la garantie de traçabilité (tout ce qui arrive doit
    // être compté).
    if nombre_recus > 0 && nombre_enregistres == 0 {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "L'envoi a échoué, réessayez.",
        )
            .into_response();
    }

    // Ces jetons permettent à la page du client de surveiller elle-même (en
    // interrogeant /statut/:jeton) le moment où le gérant encaisse, pour
    // afficher un message de remerciement en direct sans jamais passer par
    // internet (SMS/WhatsApp) — le téléphone reste sur le Wi-Fi local tant
    // que le client n'a pas quitté la boutique. Chaque jeton n'ouvre que sur
    // la commande qu'il a lui-même envoyée.
    (
        StatusCode::OK,
        Json(serde_json::json!({ "jetons": jetons })),
    )
        .into_response()
}

#[derive(serde::Serialize)]
struct StatutFichier {
    traite: bool,
    paye: bool,
    message: Option<String>,
}

/// Interrogée par la page du client (en boucle discrète, tant qu'il est
/// encore sur le Wi-Fi de la boutique) pour savoir si sa commande a été
/// encaissée — et dans ce cas, afficher un mot de remerciement avec son
/// compteur de fidélité, en direct, sans rien devoir imprimer ni envoyer.
async fn statut_fichier(
    State(app): State<AppHandle>,
    Path(jeton): Path<String>,
) -> impl IntoResponse {
    let inconnu = || {
        Json(StatutFichier {
            traite: false,
            paye: false,
            message: None,
        })
    };

    let state = app.state::<crate::db::DbState>();
    let Ok(conn) = state.0.lock() else {
        return inconnu();
    };

    // Le jeton, et lui seul, désigne la commande : impossible de consulter
    // celle d'un autre client en faisant défiler des numéros.
    let Ok(id) = conn.query_row(
        "SELECT id FROM files_queue WHERE jeton = ?1",
        rusqlite::params![jeton],
        |r| r.get::<_, i64>(0),
    ) else {
        return inconnu();
    };

    let statut_transaction: Option<String> = conn
        .query_row(
            "SELECT statut FROM transactions WHERE file_queue_id = ?1 ORDER BY id DESC LIMIT 1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .ok();

    let Some(statut) = statut_transaction else {
        return inconnu();
    };

    let paye = statut == "paye";
    let message = if paye {
        let telephone: Option<String> = conn
            .query_row(
                "SELECT client_telephone FROM files_queue WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .unwrap_or(None);

        let (seuil, remise_pourcent) = crate::gestion::parametres_fidelite(&conn);
        Some(match telephone.filter(|t| !t.is_empty()) {
            Some(tel) => {
                let visites: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM transactions t
                         JOIN files_queue f ON f.id = t.file_queue_id
                         WHERE f.client_telephone = ?1 AND t.statut = 'paye'",
                        rusqlite::params![tel],
                        |r| r.get(0),
                    )
                    .unwrap_or(1);
                if remise_pourcent > 0 && visites >= seuil {
                    format!(
                        "🎉 Merci pour votre fidélité ! Une réduction de {remise_pourcent}% a été appliquée."
                    )
                } else if remise_pourcent > 0 {
                    let restantes = seuil - visites;
                    format!(
                        "✅ Commande traitée, merci ! C'est votre {visites}ᵉ commande chez nous — \
                         encore {restantes} avant votre réduction fidélité."
                    )
                } else {
                    format!(
                        "✅ Commande traitée, merci ! C'est votre {visites}ᵉ commande chez nous."
                    )
                }
            }
            None => "✅ Commande traitée, merci pour votre confiance !".to_string(),
        })
    } else {
        None
    };

    Json(StatutFichier {
        traite: true,
        paye,
        message,
    })
}

#[cfg(test)]
mod tests {
    /// Le bug qui a fait perdre une journée de terrain : en déplaçant le
    /// bouton « Partager par Bluetooth » dans une carte qui n'existe que
    /// lorsqu'un nom Bluetooth est configuré, son élément a disparu de la
    /// page — mais le script continuait de le lire sans garde. Il levait
    /// donc une erreur AVANT d'installer l'interception du formulaire, et le
    /// navigateur retombait sur l'envoi HTML classique : page rechargée,
    /// vide, aucun fichier parti. Rien ne le signalait.
    ///
    /// Ce test parcourt CHAQUE variante de la page et exige que tout élément
    /// lu par le script y soit présent — ou, s'il est facultatif, qu'il soit
    /// lu derrière une garde. La liste ci-dessous n'est pas à rallonger à la
    /// légère : y ajouter un identifiant, c'est accepter qu'il puisse
    /// manquer un jour.
    #[test]
    fn le_script_ne_lit_aucun_element_absent_de_la_page() {
        use super::construire_page_accueil;

        // Facultatifs par construction, donc lus avec une garde dans le
        // script (`if (btnBluetooth)`).
        const FACULTATIFS: &[&str] = &["btn-partager-bluetooth"];

        let variantes = [
            ("sans réglage", None, None),
            ("WhatsApp seul", Some("+22997000000".to_string()), None),
            ("Bluetooth seul", None, Some("PC-BOUTIQUE".to_string())),
            (
                "les deux",
                Some("+22997000000".to_string()),
                Some("PC-BOUTIQUE".to_string()),
            ),
        ];

        for (nom_variante, whatsapp, bluetooth) in variantes {
            let page = construire_page_accueil(whatsapp, bluetooth);

            // Tous les identifiants que le script va chercher.
            let mut reste = page.as_str();
            while let Some(depart) = reste.find("getElementById('") {
                reste = &reste[depart + "getElementById('".len()..];
                let fin = reste.find('\'').expect("appel getElementById mal formé");
                let identifiant = &reste[..fin];
                if FACULTATIFS.contains(&identifiant) {
                    continue;
                }
                assert!(
                    page.contains(&format!("id=\"{identifiant}\"")),
                    "variante « {nom_variante} » : le script lit l'élément \
                     « {identifiant} », qui n'existe pas dans cette page. Un élément \
                     facultatif doit être lu derrière une garde, sinon l'erreur \
                     emporte tout le script — et l'envoi avec."
                );
            }
        }
    }

    /// Le seul élément facultatif doit rester lu derrière sa garde : la
    /// retirer reproduirait exactement la panne du terrain.
    #[test]
    fn le_bouton_bluetooth_facultatif_reste_lu_derriere_une_garde() {
        use super::construire_page_accueil;

        let sans_bluetooth = construire_page_accueil(None, None);
        assert!(
            !sans_bluetooth.contains("id=\"btn-partager-bluetooth\""),
            "sans nom configuré, ce bouton n'a pas lieu d'être"
        );
        assert!(
            sans_bluetooth.contains("if (btnBluetooth)"),
            "il est donc lu derrière une garde, faute de quoi le script s'arrête \
             avant d'installer l'envoi"
        );

        let avec_bluetooth = construire_page_accueil(None, Some("PC-BOUTIQUE".to_string()));
        assert!(avec_bluetooth.contains("id=\"btn-partager-bluetooth\""));
        assert!(avec_bluetooth.contains("PC-BOUTIQUE"));
    }

    /// Le bug exact venu du terrain : une carte VPN ou de machine virtuelle
    /// laissée par un ancien logiciel portait `10.10.10.1`, et le QR
    /// annonçait cette adresse au client — qui ne pouvait évidemment rien
    /// y joindre. On ne retient donc que ce qui désigne un réseau réel.
    #[test]
    fn ne_retient_que_les_adresses_joignables_par_un_telephone() {
        use super::premiere_adresse_utilisable;
        use std::net::Ipv4Addr;

        assert_eq!(
            premiere_adresse_utilisable("192.168.43.137\n"),
            Some(Ipv4Addr::new(192, 168, 43, 137))
        );
        // Carte branchée sur rien : Windows lui donne une adresse
        // d'auto-attribution qui ne mène nulle part.
        assert_eq!(premiere_adresse_utilisable("169.254.12.9"), None);
        assert_eq!(premiere_adresse_utilisable("127.0.0.1"), None);
        assert_eq!(premiere_adresse_utilisable("0.0.0.0"), None);
        // Sortie vide (PowerShell absent, aucune carte connectée).
        assert_eq!(premiere_adresse_utilisable(""), None);
        // La première ligne inutilisable ne doit pas masquer la bonne.
        assert_eq!(
            premiere_adresse_utilisable("169.254.1.1\n192.168.1.20"),
            Some(Ipv4Addr::new(192, 168, 1, 20))
        );
    }

    use super::*;

    // ── Noms de fichiers : tout vient d'un inconnu sur le Wi-Fi ──

    #[test]
    fn retire_le_chemin_pour_empecher_d_ecrire_ailleurs() {
        // Sans cela, un nom forgé écrirait hors du dossier de réception —
        // par exemple dans le démarrage de Windows.
        assert_eq!(
            nom_fichier_sans_chemin(r"..\..\Windows\Start Menu\virus.exe"),
            "virus.exe"
        );
        assert_eq!(nom_fichier_sans_chemin("../../etc/passwd"), "passwd");
        assert_eq!(
            nom_fichier_sans_chemin("/absolu/document.pdf"),
            "document.pdf"
        );
    }

    #[test]
    fn refuse_les_noms_qui_ne_designent_aucun_fichier() {
        assert_eq!(nom_fichier_sans_chemin(""), "fichier_recu");
        assert_eq!(nom_fichier_sans_chemin("."), "fichier_recu");
        assert_eq!(nom_fichier_sans_chemin(".."), "fichier_recu");
        assert_eq!(nom_fichier_sans_chemin("   "), "fichier_recu");
    }

    #[test]
    fn refuse_les_noms_reserves_de_windows() {
        // "CON.pdf" ou "LPT1.txt" : Windows les traite comme des
        // périphériques, l'écriture échouerait silencieusement.
        assert_eq!(nom_fichier_sans_chemin("CON.pdf"), "fichier_recu");
        assert_eq!(nom_fichier_sans_chemin("lpt1.txt"), "fichier_recu");
        assert_eq!(nom_fichier_sans_chemin("nul"), "fichier_recu");
    }

    #[test]
    fn nettoie_les_caracteres_interdits_sans_perdre_le_document() {
        assert_eq!(
            nom_fichier_sans_chemin(r#"fac<ture>:"a|b?.pdf"#),
            "factureab.pdf"
        );
        assert_eq!(
            nom_fichier_sans_chemin("rapport\u{0}\u{7}.pdf"),
            "rapport.pdf"
        );
    }

    #[test]
    fn garde_un_nom_normal_intact() {
        assert_eq!(
            nom_fichier_sans_chemin("Mémoire chapitre 3.pdf"),
            "Mémoire chapitre 3.pdf"
        );
    }

    #[test]
    fn borne_la_longueur_du_nom_de_fichier() {
        let tres_long = format!("{}.pdf", "a".repeat(500));
        assert!(nom_fichier_sans_chemin(&tres_long).chars().count() <= 150);
    }

    // ── Champs texte du formulaire : aucune limite côté navigateur ──

    #[test]
    fn borne_les_textes_envoyes_par_le_client() {
        let enorme = "x".repeat(5_000_000);
        assert_eq!(
            borner_texte(&enorme, LONGUEUR_MAX_NOM).chars().count(),
            LONGUEUR_MAX_NOM
        );
    }

    #[test]
    fn borner_texte_enleve_les_espaces_inutiles() {
        assert_eq!(borner_texte("  Fatou N.  ", LONGUEUR_MAX_NOM), "Fatou N.");
        assert_eq!(borner_texte("   ", LONGUEUR_MAX_NOM), "");
    }

    #[test]
    fn borner_texte_ne_coupe_pas_au_milieu_d_un_caractere_accentue() {
        // Compte des caractères, pas des octets : couper des octets sur un
        // "é" produirait une chaîne invalide.
        let accents = "é".repeat(200);
        let borne = borner_texte(&accents, 10);
        assert_eq!(borne.chars().count(), 10);
    }

    // ── Format papier : le menu déroulant est contournable ──

    #[test]
    fn accepte_les_formats_connus_quelle_que_soit_la_casse() {
        assert_eq!(format_papier_valide("A4"), "A4");
        assert_eq!(format_papier_valide("a3"), "A3");
        assert_eq!(format_papier_valide(" a5 "), "A5");
    }

    #[test]
    fn un_format_inconnu_retombe_sur_a4_au_lieu_d_entrer_en_base() {
        assert_eq!(format_papier_valide("A0"), "A4");
        assert_eq!(format_papier_valide(""), "A4");
        assert_eq!(format_papier_valide(&"x".repeat(100_000)), "A4");
        assert_eq!(format_papier_valide("<script>alert(1)</script>"), "A4");
    }

    // ── Numéro de téléphone ──

    #[test]
    fn normalise_les_numeros_beninois() {
        assert_eq!(
            normalize_phone("0197000000").as_deref(),
            Some("2290197000000")
        );
        assert_eq!(
            normalize_phone("97000000").as_deref(),
            Some("2290197000000")
        );
        assert_eq!(
            normalize_phone("+229 01 97 00 00 00").as_deref(),
            Some("2290197000000")
        );
        assert_eq!(normalize_phone("pas de chiffres"), None);
    }

    // ── Échappement de la page servie au client ──

    #[test]
    fn echappe_le_nom_bluetooth_saisi_par_le_gerant() {
        assert_eq!(
            echapper_html(r#"<script>alert("x")</script>"#),
            "&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;"
        );
        // L'esperluette doit être traitée en premier, sinon les entités
        // produites par les remplacements suivants seraient ré-échappées.
        assert_eq!(echapper_html("Tom & Jerry"), "Tom &amp; Jerry");
    }

    /// Ces adresses sont figées dans les téléphones et servent à marquer le
    /// journal : une faute de frappe ne se signalerait nulle part ailleurs.
    #[test]
    fn reconnait_les_sondes_de_chaque_systeme() {
        for chemin in [
            "/hotspot-detect.html",
            "/library/test/success.html",
            "/generate_204",
            "/gen_204",
            "/connecttest.txt",
            "/ncsi.txt",
            "/success.txt",
            "/canonical.html",
        ] {
            assert!(
                est_sonde_de_reseau(chemin, "192.168.73.1"),
                "sonde non reconnue : {chemin}"
            );
        }
        assert!(
            est_sonde_de_reseau("/GENERATE_204", "192.168.73.1"),
            "la casse vient du téléphone"
        );
    }

    /// Le trou que la documentation a révélé : iOS interroge aussi
    /// `netcts.cdn-apple.com/`, à la RACINE. Un test par chemin seul le
    /// laisse passer, et cette seconde vérification reçoit alors la page
    /// entière — le cas rapporté par nodogsplash, où le portail
    /// s'affichait pour une adresse et jamais pour l'autre.
    #[test]
    fn reconnait_les_machines_de_controle_meme_a_la_racine() {
        for hote in [
            "netcts.cdn-apple.com",
            "captive.apple.com",
            "connectivitycheck.gstatic.com",
            "www.msftconnecttest.com",
            "detectportal.firefox.com",
        ] {
            assert!(
                est_sonde_de_reseau("/", hote),
                "machine de contrôle non reconnue à la racine : {hote}"
            );
        }
        // Le port fait partie de l'en-tête « Host » et n'appartient pas au nom.
        assert!(est_sonde_de_reseau("/", "NETCTS.CDN-APPLE.COM:80"));
    }

    /// Le poids est ici la fonctionnalité : c'est précisément parce que la
    /// page d'envoi en fait vingt mille que la fenêtre de contrôle du
    /// téléphone pouvait refuser de s'afficher. Un ajout innocent — une
    /// image, une feuille de style — ramènerait la panne sans rien casser
    /// de visible.
    #[test]
    fn la_page_de_controle_reste_minuscule() {
        let page = page_de_controle("192.168.73.1");
        assert!(
            page.len() < 1024,
            "la page de contrôle fait {} octets, elle doit rester sous 1 Ko",
            page.len()
        );
        assert!(!page.contains("<script"), "aucun JavaScript");
        assert!(!page.contains("<img"), "aucune image");
        assert!(!page.contains("<link"), "aucune ressource externe");
    }

    /// Sans ces deux chemins, le téléphone affiche une page dont on ne peut
    /// pas sortir : il aurait détecté le portail sans pouvoir l'utiliser.
    #[test]
    fn la_page_de_controle_mene_a_la_page_d_envoi() {
        let page = page_de_controle("192.168.73.1");
        assert!(
            page.contains("http-equiv=\"refresh\""),
            "passage automatique"
        );
        assert!(
            page.contains("href=\"http://192.168.73.1/\""),
            "lien de repli si le passage automatique est ignoré"
        );
    }

    /// La page du client n'est pas une sonde : elle ne doit jamais porter
    /// la marque, sinon le journal devient illisible.
    #[test]
    fn la_page_du_client_n_est_pas_une_sonde() {
        for chemin in ["/", "/envoyer", "/statut/abc", "/api-portail"] {
            assert!(
                !est_sonde_de_reseau(chemin, "192.168.73.1"),
                "pris à tort pour une sonde : {chemin}"
            );
        }
    }
}
