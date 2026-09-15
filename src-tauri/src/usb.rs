use crate::files;
use crate::watcher::enqueue_file;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use sysinfo::Disks;
use tauri::{AppHandle, Manager};

const PROFONDEUR_MAX: u32 = 2;

/// Nombre maximum de fichiers pris sur une même clé. Une clé de client peut
/// contenir des centaines de photos personnelles : sans cette limite, elles
/// rempliraient la file d'attente du gérant (et feraient sonner l'appli à
/// chaque fichier), noyant les vraies commandes.
const FICHIERS_MAX_PAR_CLE: usize = 40;

/// Au-delà, on ne recopie pas le fichier localement : ce n'est de toute
/// façon pas un document à photocopier.
const TAILLE_MAX_COPIE: u64 = 100 * 1024 * 1024;

/// Dossiers système présents sur presque toutes les clés : les scanner ne
/// donne jamais un document de client.
const DOSSIERS_IGNORES: [&str; 6] = [
    "System Volume Information",
    "$RECYCLE.BIN",
    "found.000",
    ".Trashes",
    ".Spotlight-V100",
    ".fseventsd",
];

/// Surveille en continu l'apparition de clés USB (disques amovibles) et met en
/// file d'attente tout fichier de format reconnu trouvé dessus. Simple par
/// design : on ne demande rien au gérant, on scanne juste le contenu.
pub fn watch_usb_drives(app: AppHandle) {
    thread::spawn(move || {
        let mut deja_vus: HashSet<PathBuf> = HashSet::new();

        loop {
            let disks = Disks::new_with_refreshed_list();
            let amovibles: HashSet<PathBuf> = disks
                .iter()
                .filter(|d| d.is_removable())
                .map(|d| d.mount_point().to_path_buf())
                .collect();

            for mount in amovibles.difference(&deja_vus) {
                let mut restants = FICHIERS_MAX_PAR_CLE;
                scanner_dossier(&app, mount, 0, &mut restants);
            }

            deja_vus = amovibles;
            thread::sleep(Duration::from_secs(2));
        }
    });
}

fn scanner_dossier(app: &AppHandle, dossier: &Path, profondeur: u32, restants: &mut usize) {
    let Ok(entries) = std::fs::read_dir(dossier) else {
        return;
    };
    for entry in entries.flatten() {
        if *restants == 0 {
            return;
        }
        let path = entry.path();
        if path.is_dir() {
            let nom = entry.file_name();
            let nom = nom.to_string_lossy();
            if DOSSIERS_IGNORES.iter().any(|d| nom.eq_ignore_ascii_case(d)) {
                continue;
            }
            if profondeur < PROFONDEUR_MAX {
                scanner_dossier(app, &path, profondeur + 1, restants);
            }
            continue;
        }
        if files::classify(&path) == "inconnu" {
            continue;
        }
        let chemin_a_enregistrer = copier_en_local(app, &path).unwrap_or(path);
        if enqueue_file(app, &chemin_a_enregistrer, "usb", None, None).is_some() {
            *restants -= 1;
        }
    }
}

/// Recopie le fichier dans les données de l'application avant de le mettre en
/// file d'attente. Sans cela, la ligne de la file pointerait sur la clé : dès
/// que le client la reprend — souvent juste après l'avoir tendue — le document
/// devient impossible à ouvrir ou à imprimer, et la commande est perdue alors
/// que l'appli affiche qu'elle est bien là.
fn copier_en_local(app: &AppHandle, source: &Path) -> Option<PathBuf> {
    let taille = std::fs::metadata(source).ok()?.len();
    if taille > TAILLE_MAX_COPIE {
        return None;
    }

    let dossier = app.path().app_data_dir().ok()?.join("recus_usb");
    std::fs::create_dir_all(&dossier).ok()?;

    let nom = source.file_name()?.to_string_lossy().to_string();
    let horodatage = chrono::Local::now().format("%Y%m%d-%H%M%S%3f");
    let destination = dossier.join(format!("{horodatage}_{nom}"));

    std::fs::copy(source, &destination).ok()?;
    Some(destination)
}
