use crate::files;
use crate::watcher::enqueue_file;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use sysinfo::Disks;
use tauri::AppHandle;

const PROFONDEUR_MAX: u32 = 2;

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
                scanner_dossier(&app, mount, 0);
            }

            deja_vus = amovibles;
            thread::sleep(Duration::from_secs(2));
        }
    });
}

fn scanner_dossier(app: &AppHandle, dossier: &Path, profondeur: u32) {
    let Ok(entries) = std::fs::read_dir(dossier) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if profondeur < PROFONDEUR_MAX {
                scanner_dossier(app, &path, profondeur + 1);
            }
            continue;
        }
        if files::classify(&path) == "inconnu" {
            continue;
        }
        enqueue_file(app, &path, "usb", None, None);
    }
}
