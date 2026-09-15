use crate::db::DbState;
use crate::files;
use crate::models::QueueItem;
use chrono::Local;
use notify::{Event, EventKind, RecursiveMode, Watcher};
use rusqlite::params;
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// Démarre la surveillance du dossier de réception dans un thread dédié.
/// À chaque fichier nouvellement créé, on l'enregistre en base et on
/// prévient l'interface via un événement Tauri — sans rechargement de page.
pub fn watch_folder(app: AppHandle, folder: PathBuf) {
    thread::spawn(move || {
        let (tx, rx) = channel::<notify::Result<Event>>();

        let mut watcher = match notify::recommended_watcher(tx) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("Impossible de démarrer la surveillance du dossier : {e}");
                return;
            }
        };

        if let Err(e) = watcher.watch(&folder, RecursiveMode::NonRecursive) {
            eprintln!(
                "Impossible de surveiller le dossier {} : {e}",
                folder.display()
            );
            return;
        }

        for res in rx {
            let Ok(event) = res else { continue };
            if !matches!(event.kind, EventKind::Create(_)) {
                continue;
            }
            for path in event.paths {
                if !path.is_file() {
                    continue;
                }
                // Laisse le temps à une copie de fichier de se terminer avant de la traiter.
                thread::sleep(Duration::from_millis(600));
                enqueue_file(&app, &path, "dossier_surveille", None, None);
            }
        }
    });
}

/// Préférences d'impression indiquées par le client lui-même (ex: via le
/// formulaire QR), pour que le gérant n'ait qu'à confirmer plutôt qu'à
/// redemander à chacun comment il veut son document.
#[derive(Default)]
pub struct OptionsImpression {
    pub couleur: bool,
    pub format_papier: Option<String>,
    pub copies: Option<i64>,
    pub plage_pages: Option<String>,
}

/// Enregistre un fichier reçu (quel que soit le canal) dans la file d'attente
/// et prévient l'interface. Ignoré silencieusement si le chemin est déjà connu.
pub fn enqueue_file(
    app: &AppHandle,
    path: &std::path::Path,
    source: &str,
    client_name: Option<&str>,
    client_telephone: Option<&str>,
) -> Option<i64> {
    enqueue_file_avec_options(
        app,
        path,
        source,
        client_name,
        client_telephone,
        OptionsImpression::default(),
    )
}

pub fn enqueue_file_avec_options(
    app: &AppHandle,
    path: &std::path::Path,
    source: &str,
    client_name: Option<&str>,
    client_telephone: Option<&str>,
    options: OptionsImpression,
) -> Option<i64> {
    let original_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "fichier".to_string());
    let kind = files::classify(path);
    // Un .exe/.msi classé "installateur" obtient un bouton "Installer la mise
    // à jour" qui l'exécute en un clic (voir files::shell_open). Le canal QR
    // est accessible à n'importe qui connecté au Wi-Fi de la boutique — sans
    // ce garde-fou, un client malveillant pourrait faire exécuter un fichier
    // exécutable arbitraire au gérant en le nommant "Mise_a_jour.exe". Seuls
    // les canaux qui exigent un accès physique au PC (clé USB, dossier
    // surveillé/Bluetooth) restent traités comme une vraie mise à jour.
    let kind = if kind == "installateur" && source == "qr" {
        "inconnu"
    } else {
        kind
    };
    let received_at = Local::now().to_rfc3339();
    let taille_octets = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let (protege, format_detecte) = files::diagnostiquer_pdf(path);
    let format_detecte = format_detecte.map(str::to_string);

    let state = app.state::<DbState>();
    let conn = state.0.lock().expect("verrou base de données corrompu");

    let already = conn
        .query_row(
            "SELECT 1 FROM files_queue WHERE path = ?1",
            params![path.to_string_lossy()],
            |_| Ok(()),
        )
        .is_ok();
    if already {
        return None;
    }

    // Borné aussi ici (pas seulement côté serveur HTTP) : ce point d'entrée
    // sert à tous les canaux de réception, pas seulement le QR.
    let copies = options.copies.unwrap_or(1).clamp(1, 500);
    let format_papier = options
        .format_papier
        .clone()
        .unwrap_or_else(|| "A4".to_string());

    let insert_result = conn.execute(
        "INSERT INTO files_queue
            (original_name, path, client_name, client_telephone, source, kind, status,
             received_at, taille_octets, protege, format_detecte, couleur, format_papier,
             copies, plage_pages)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'en_attente', ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            original_name,
            path.to_string_lossy(),
            client_name,
            client_telephone,
            source,
            kind,
            received_at,
            taille_octets as i64,
            protege,
            format_detecte,
            options.couleur,
            format_papier,
            copies,
            options.plage_pages
        ],
    );

    if let Err(e) = insert_result {
        eprintln!("Échec d'enregistrement du fichier reçu : {e}");
        return None;
    }
    let id = conn.last_insert_rowid();
    drop(conn);

    let item = QueueItem {
        id,
        original_name,
        path: path.to_string_lossy().to_string(),
        client_name: client_name.map(str::to_string),
        client_telephone: client_telephone.map(str::to_string),
        source: source.to_string(),
        kind: kind.to_string(),
        status: "en_attente".to_string(),
        received_at,
        taille_octets: taille_octets as i64,
        protege,
        format_detecte,
        copies,
        couleur: options.couleur,
        format_papier,
        plage_pages: options.plage_pages,
        finitions: vec![],
        prix: None,
        employe: None,
        raison_ignore: None,
    };

    let _ = app.emit("nouveau-fichier", item);
    Some(id)
}
