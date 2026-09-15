use crate::watcher::{enqueue_file_avec_options, OptionsImpression};
use axum::extract::{DefaultBodyLimit, Multipart, Path, State};
use axum::http::{StatusCode, Uri};
use axum::response::{Html, IntoResponse, Redirect};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager};

pub const PORT: u16 = 4173;
const PORT_PORTAIL_CAPTIF: u16 = 80;

/// Limite haute pour un fichier envoyé par un client (au-delà, on refuse
/// proprement plutôt que de laisser le serveur consommer toute la mémoire).
const TAILLE_MAX_ENVOI: usize = 200 * 1024 * 1024; // 200 Mo

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
    tauri::async_runtime::spawn(async move {
        let app_pour_etat = app.clone();
        let router = Router::new()
            .route("/", get(page_accueil))
            .route("/envoyer", post(recevoir_fichier))
            .route("/statut/:jeton", get(statut_fichier))
            .layer(DefaultBodyLimit::max(TAILLE_MAX_ENVOI))
            .with_state(app);

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

    demarrer_portail_captif();
}

/// Tentative "portail captif" façon Wi-Fi d'hôtel/café : une fois le client
/// connecté au Wi-Fi local, on essaie de faire en sorte que son téléphone
/// ouvre automatiquement la page d'envoi, sans qu'il ait à toucher son
/// navigateur. Fonctionne sur une partie des téléphones seulement (dépend
/// de la détection de portail captif du système, non testable ici sans une
/// vraie machine Windows) — best-effort, jamais bloquant si ça échoue.
fn demarrer_portail_captif() {
    tauri::async_runtime::spawn(async move {
        let router = Router::new()
            .route("/*chemin", any(rediriger_vers_accueil))
            .route("/", any(rediriger_vers_accueil));
        let addr = format!("0.0.0.0:{PORT_PORTAIL_CAPTIF}");
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => {
                let _ = axum::serve(listener, router).await;
            }
            Err(e) => {
                eprintln!(
                    "Portail captif (port {PORT_PORTAIL_CAPTIF}) indisponible : {e}. \
                     Pas grave : le client ouvrira la page manuellement après connexion au Wi-Fi."
                );
            }
        }
    });
}

/// L'adresse réelle du PC sur le réseau local, résolue à chaque requête : le
/// partage de connexion Windows donne le plus souvent 192.168.137.1, mais pas
/// toujours (PC branché sur la box du quartier, autre configuration...). Une
/// adresse écrite en dur enverrait alors le client sur une page inexistante.
fn adresse_locale() -> String {
    local_ip_address::local_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| "192.168.137.1".to_string())
}

async fn rediriger_vers_accueil(_uri: Uri) -> impl IntoResponse {
    Redirect::to(&format!("http://{}:{PORT}/", adresse_locale()))
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

async fn page_accueil(State(app): State<AppHandle>) -> Html<String> {
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

    let lien_whatsapp = match whatsapp.as_deref().and_then(normalize_phone) {
        Some(numero) => format!(
            r#"<a class="lien-secondaire" href="https://wa.me/{numero}" target="_blank">Envoyer par WhatsApp à la place</a>"#
        ),
        None => String::new(),
    };

    // Sans nom configuré, on reste sur une instruction générique plutôt que
    // de dire au client de chercher un appareil "vide" — mieux vaut ne rien
    // promettre de précis que d'induire en erreur.
    let bloc_bluetooth = match bluetooth_nom.filter(|n| !n.trim().is_empty()) {
        Some(nom) => format!(
            r#"<div class="bluetooth-bloc">
                <p style="margin:0 0 0.5rem">
                    <strong>Envoyer par Bluetooth :</strong> activez le Bluetooth sur votre
                    téléphone, puis utilisez le bouton "Partager par Bluetooth" ci-dessus
                    si vous le voyez. Sinon : sélectionnez votre/vos fichier(s) dans vos
                    Photos ou Fichiers, appuyez sur "Partager", puis "Bluetooth", et
                    cherchez l'appareil ci-dessous.
                </p>
                <p class="bluetooth-nom-puce">📶 {nom}</p>
            </div>"#,
            nom = echapper_html(&nom)
        ),
        None => r#"<p>Bluetooth : depuis votre téléphone, activez le Bluetooth et cherchez l'ordinateur de la boutique.</p>"#.to_string(),
    };

    Html(format!(
        r#"<!doctype html>
<html lang="fr">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1.0" />
<title>Envoyer un fichier à la boutique</title>
<style>
  body {{ font-family: "Segoe UI", Calibri, Arial, sans-serif; background:#f3f2f1; margin:0; padding:1.5rem; color:#323130; }}
  .carte {{ max-width: 440px; margin: 0 auto; background:#fff; border-radius:8px; padding:1.5rem; box-shadow: 0 2px 8px rgba(0,0,0,0.08); }}
  h1 {{ font-size:1.1rem; color:#2b579a; margin-top:0; }}
  input[type=text], input[type=tel] {{ width:100%; padding:0.6rem; margin-bottom:0.75rem; border:1px solid #d6d4d1; border-radius:4px; font-size:1rem; box-sizing:border-box; }}
  input[type=file] {{ width:100%; margin-bottom:1rem; }}
  button {{ width:100%; padding:0.85rem; background:#2b579a; color:#fff; border:none; border-radius:4px; font-size:1.05rem; font-weight:600; cursor:pointer; }}
  .liens-secondaires {{ margin-top:1.5rem; text-align:center; font-size:0.8rem; }}
  .liens-secondaires a {{ color:#605e5c; text-decoration:underline; }}
  #confirmation {{ display:none; text-align:center; color:#107c10; font-weight:600; margin-top:1rem; }}
  #statut-fidelite {{ display:none; text-align:center; background:#dff6dd; color:#107c10; font-weight:600; padding:0.75rem; border-radius:6px; margin-top:0.75rem; }}
  #progression {{ display:none; height:8px; background:#e8e6e4; border-radius:4px; overflow:hidden; margin-bottom:1rem; }}
  #progression > div {{ height:100%; width:0%; background:#2b579a; transition:width .15s; }}
  #texte-progression {{ display:none; text-align:center; font-size:0.8rem; color:#605e5c; margin:-0.5rem 0 1rem; }}
  .fichier {{ border:1px solid #d6d4d1; border-radius:6px; padding:0.6rem 0.75rem; margin-bottom:0.6rem; }}
  .fichier-nom {{ font-size:0.9rem; font-weight:600; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }}
  .fichier-toggle {{ background:none; border:none; color:#2b579a; font-size:0.8rem; padding:0.2rem 0; cursor:pointer; width:auto; }}
  .fichier-options {{ display:none; margin-top:0.5rem; font-size:0.85rem; }}
  .fichier-options.ouvert {{ display:block; }}
  .fichier-options label {{ display:flex; align-items:center; gap:0.4rem; margin-bottom:0.4rem; }}
  .fichier-options select, .fichier-options input[type=number], .fichier-options input[type=text] {{
    width: auto; flex:1; padding:0.35rem; margin:0; border:1px solid #d6d4d1; border-radius:4px;
  }}
  .bluetooth-bloc {{ text-align:left; background:#f3f2f1; padding:0.75rem; border-radius:6px; line-height:1.6; }}
  .bluetooth-nom-puce {{ text-align:center; font-size:1.15rem; font-weight:700; color:#2b579a; background:#fff; border:2px dashed #2b579a; border-radius:6px; padding:0.6rem; margin:0; word-break:break-word; }}
  .btn-bluetooth {{ background:#fff; color:#2b579a; border:1px solid #2b579a; margin-bottom:1rem; }}
  .note-prix {{ font-size:0.72rem; color:#8a8886; text-align:center; margin:-0.5rem 0 1rem; }}
</style>
</head>
<body>
  <div class="carte">
    <h1>Envoyer un fichier à la boutique</h1>
    <form id="form-envoi">
      <p style="font-size:0.78rem; color:#605e5c; margin:0 0 0.3rem">
        Votre nom (facultatif) aide la boutique à savoir à qui appartient
        votre fichier, surtout si plusieurs personnes envoient en même temps.
      </p>
      <input type="text" name="nom" placeholder="Votre nom (optionnel)" />
      <p style="font-size:0.78rem; color:#605e5c; margin:0.75rem 0 0.3rem">
        Votre numéro (facultatif) vous permet de profiter d'une réduction
        après plusieurs commandes chez cette boutique.
      </p>
      <input type="tel" name="telephone" placeholder="Votre numéro (optionnel)" />
      <input type="file" id="champ-fichiers" multiple required />
      <div id="liste-fichiers"></div>
      <button type="button" id="btn-partager-bluetooth" class="btn-bluetooth" hidden>📤 Partager par Bluetooth à la place</button>
      <div id="progression"><div></div></div>
      <p id="texte-progression"></p>
      <button type="submit">Envoyer à la boutique</button>
      <p class="note-prix">Le prix est à régler directement avec le gérant, sur place.</p>
    </form>
    <p id="confirmation">Fichier(s) envoyé(s), merci ! Le gérant a été prévenu.</p>
    <p id="statut-fidelite"></p>
    <div class="liens-secondaires">
      {lien_whatsapp}
      {bloc_bluetooth}
    </div>
  </div>
  <script>
    const champFichiers = document.getElementById('champ-fichiers');
    const listeFichiers = document.getElementById('liste-fichiers');

    // Par défaut : Noir & Blanc, A4, 1 copie — le client ne voit ces
    // réglages que s'il clique "Personnaliser", pour ne jamais compliquer
    // l'envoi simple d'un fichier tel quel.
    champFichiers.addEventListener('change', () => {{
      listeFichiers.innerHTML = '';
      [...champFichiers.files].forEach((fichier, i) => {{
        const bloc = document.createElement('div');
        bloc.className = 'fichier';
        bloc.innerHTML = `
          <div class="fichier-nom">${{fichier.name}}</div>
          <button type="button" class="fichier-toggle">Personnaliser (couleur, format, copies)…</button>
          <div class="fichier-options">
            <label><input type="checkbox" name="couleur_${{i}}" /> Couleur (sinon Noir &amp; Blanc)</label>
            <label>Format
              <select name="format_${{i}}">
                <option value="A4">A4</option>
                <option value="A3">A3</option>
                <option value="A5">A5</option>
              </select>
            </label>
            <label>Copies <input type="number" name="copies_${{i}}" min="1" value="1" /></label>
            <label>Pages (ex: 1-5) <input type="text" name="pages_${{i}}" placeholder="toutes" /></label>
          </div>
        `;
        bloc.querySelector('.fichier-toggle').addEventListener('click', (e) => {{
          bloc.querySelector('.fichier-options').classList.toggle('ouvert');
        }});
        listeFichiers.appendChild(bloc);
      }});
      majBoutonBluetooth();
    }});

    // Le bouton Bluetooth n'apparaît que si le téléphone sait le faire
    // (surtout Android) et que des fichiers sont bien sélectionnés —
    // sinon les instructions manuelles restent le seul recours.
    const btnBluetooth = document.getElementById('btn-partager-bluetooth');
    function majBoutonBluetooth() {{
      const fichiers = [...champFichiers.files];
      const peutPartager =
        fichiers.length > 0 &&
        typeof navigator.canShare === 'function' &&
        navigator.canShare({{ files: fichiers }});
      btnBluetooth.hidden = !peutPartager;
    }}
    btnBluetooth.addEventListener('click', async () => {{
      try {{
        await navigator.share({{ files: [...champFichiers.files] }});
      }} catch {{
        // Annulé par le client, ou échec — pas grave, il peut toujours
        // utiliser "Envoyer à la boutique" ou les instructions manuelles.
      }}
    }});

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
      const donnees = new FormData();
      donnees.append('nom', form.nom.value);
      donnees.append('telephone', form.telephone.value);
      [...champFichiers.files].forEach((fichier, i) => {{
        donnees.append(`fichier_${{i}}`, fichier);
        donnees.append(`couleur_${{i}}`, form[`couleur_${{i}}`]?.checked ? '1' : '0');
        donnees.append(`format_${{i}}`, form[`format_${{i}}`]?.value || 'A4');
        donnees.append(`copies_${{i}}`, form[`copies_${{i}}`]?.value || '1');
        donnees.append(`pages_${{i}}`, form[`pages_${{i}}`]?.value || '');
      }});
      donnees.append('nombre_fichiers', String(champFichiers.files.length));

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
    ))
}

struct FichierRecu {
    original_name: String,
    bytes: Vec<u8>,
}

/// Ne garde que le nom de fichier, sans le chemin — un client malveillant
/// pourrait sinon envoyer un nom du type "../../Windows/Startup/x.exe" pour
/// écrire en dehors du dossier de réception (faille de traversée de chemin).
fn nom_fichier_sans_chemin(nom_brut: &str) -> String {
    let nom = nom_brut.rsplit(['/', '\\']).next().unwrap_or(nom_brut).trim();
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
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let base = nom.split('.').next().unwrap_or(nom).to_uppercase();

    if nom.is_empty() || nom == "." || nom == ".." || NOMS_RESERVES_WINDOWS.contains(&base.as_str()) {
        "fichier_recu".to_string()
    } else {
        nom.to_string()
    }
}

async fn recevoir_fichier(
    State(app): State<AppHandle>,
    mut multipart: Multipart,
) -> impl IntoResponse {
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
                if !v.trim().is_empty() {
                    nom = Some(v.trim().to_string());
                }
            }
            continue;
        }
        if name == "telephone" {
            if let Ok(v) = field.text().await {
                telephone = normalize_phone(&v);
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
                    if !v.is_empty() {
                        options.entry(indice).or_default().format_papier = Some(v);
                    }
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
                    if !v.trim().is_empty() {
                        options.entry(indice).or_default().plage_pages = Some(v.trim().to_string());
                    }
                }
            }
            _ => {}
        }
    }

    if fichiers.is_empty() {
        return (StatusCode::BAD_REQUEST, "aucun fichier reçu").into_response();
    }

    let data_dir = match app.path().app_data_dir() {
        Ok(d) => d,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "erreur serveur").into_response(),
    };
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
    (StatusCode::OK, Json(serde_json::json!({ "jetons": jetons }))).into_response()
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
                    format!("✅ Commande traitée, merci ! C'est votre {visites}ᵉ commande chez nous.")
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
