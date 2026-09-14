use crate::watcher::enqueue_file;
use axum::extract::{Multipart, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::Router;
use tauri::{AppHandle, Manager};

pub const PORT: u16 = 4173;

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
        let router = Router::new()
            .route("/", get(page_accueil))
            .route("/envoyer", post(recevoir_fichier))
            .with_state(app);

        let addr = format!("0.0.0.0:{PORT}");
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => {
                if let Err(e) = axum::serve(listener, router).await {
                    eprintln!("Serveur local arrêté avec une erreur : {e}");
                }
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

async fn page_accueil(State(app): State<AppHandle>) -> Html<String> {
    let whatsapp = {
        let state = app.state::<crate::db::DbState>();
        let conn = state.0.lock().unwrap();
        crate::db::get_setting(&conn, "boutique_whatsapp")
    };

    let lien_whatsapp = match whatsapp.as_deref().and_then(normalize_phone) {
        Some(numero) => format!(
            r#"<a class="lien-secondaire" href="https://wa.me/{numero}" target="_blank">Envoyer par WhatsApp à la place</a>"#
        ),
        None => String::new(),
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
  .carte {{ max-width: 420px; margin: 0 auto; background:#fff; border-radius:8px; padding:1.5rem; box-shadow: 0 2px 8px rgba(0,0,0,0.08); }}
  h1 {{ font-size:1.1rem; color:#2b579a; margin-top:0; }}
  input[type=text], input[type=tel] {{ width:100%; padding:0.6rem; margin-bottom:0.75rem; border:1px solid #d6d4d1; border-radius:4px; font-size:1rem; }}
  input[type=file] {{ width:100%; margin-bottom:1rem; }}
  button {{ width:100%; padding:0.85rem; background:#2b579a; color:#fff; border:none; border-radius:4px; font-size:1.05rem; font-weight:600; cursor:pointer; }}
  .liens-secondaires {{ margin-top:1.5rem; text-align:center; font-size:0.8rem; }}
  .liens-secondaires a {{ color:#605e5c; text-decoration:underline; }}
  #confirmation {{ display:none; text-align:center; color:#107c10; font-weight:600; margin-top:1rem; }}
</style>
</head>
<body>
  <div class="carte">
    <h1>Envoyer un fichier à la boutique</h1>
    <form id="form-envoi" method="post" action="/envoyer" enctype="multipart/form-data">
      <input type="text" name="nom" placeholder="Votre nom (optionnel)" />
      <input type="tel" name="telephone" placeholder="Votre numéro (optionnel)" />
      <input type="file" name="fichier" required />
      <button type="submit">Envoyer à la boutique</button>
    </form>
    <p id="confirmation">Fichier envoyé, merci ! Le gérant a été prévenu.</p>
    <div class="liens-secondaires">
      {lien_whatsapp}
      <p>Bluetooth : depuis votre téléphone, activez le Bluetooth et cherchez l'ordinateur de la boutique.</p>
    </div>
  </div>
  <script>
    document.getElementById('form-envoi').addEventListener('submit', async (e) => {{
      e.preventDefault();
      const form = e.target;
      const donnees = new FormData(form);
      const bouton = form.querySelector('button');
      bouton.disabled = true;
      bouton.textContent = 'Envoi en cours…';
      try {{
        const reponse = await fetch('/envoyer', {{ method: 'POST', body: donnees }});
        if (!reponse.ok) throw new Error('echec');
        form.hidden = true;
        document.getElementById('confirmation').style.display = 'block';
      }} catch (err) {{
        bouton.disabled = false;
        bouton.textContent = 'Envoyer à la boutique';
        alert("L'envoi a échoué, réessayez.");
      }}
    }});
  </script>
</body>
</html>"#
    ))
}

async fn recevoir_fichier(
    State(app): State<AppHandle>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let mut nom: Option<String> = None;
    let mut telephone: Option<String> = None;
    let mut fichier_sauve = false;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "nom" => {
                if let Ok(v) = field.text().await {
                    if !v.trim().is_empty() {
                        nom = Some(v.trim().to_string());
                    }
                }
            }
            "telephone" => {
                if let Ok(v) = field.text().await {
                    telephone = normalize_phone(&v);
                }
            }
            "fichier" => {
                let original_name = field
                    .file_name()
                    .map(str::to_string)
                    .unwrap_or_else(|| "fichier_recu".to_string());
                let Ok(bytes) = field.bytes().await else {
                    continue;
                };
                if bytes.is_empty() {
                    continue;
                }

                let data_dir = app
                    .path()
                    .app_data_dir()
                    .expect("dossier de données introuvable");
                let recus_dir = data_dir.join("recus");
                if std::fs::create_dir_all(&recus_dir).is_err() {
                    return (StatusCode::INTERNAL_SERVER_ERROR, "erreur serveur").into_response();
                }

                let horodatage = chrono::Local::now().format("%Y%m%d-%H%M%S%3f");
                let nom_fichier_sur_disque = format!("{horodatage}_{original_name}");
                let chemin = recus_dir.join(&nom_fichier_sur_disque);

                if std::fs::write(&chemin, &bytes).is_err() {
                    return (StatusCode::INTERNAL_SERVER_ERROR, "erreur serveur").into_response();
                }

                enqueue_file(&app, &chemin, "qr", nom.as_deref(), telephone.as_deref());
                fichier_sauve = true;
            }
            _ => {}
        }
    }

    if fichier_sauve {
        (StatusCode::OK, [(header::CONTENT_TYPE, "text/plain")], "ok").into_response()
    } else {
        (StatusCode::BAD_REQUEST, "aucun fichier reçu").into_response()
    }
}
