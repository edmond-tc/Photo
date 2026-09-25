use crate::files;
use crate::watcher::enqueue_file;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use sysinfo::Disks;
use tauri::{AppHandle, Emitter, Manager};

const PROFONDEUR_MAX: u32 = 2;

/// Nombre maximum de documents PROPOSÉS pour une même clé.
///
/// Rien n'entre plus dans la file sans qu'on l'ait choisi, donc cette limite
/// ne protège plus le gérant d'une invasion : elle empêche seulement une
/// liste interminable à faire défiler. Assez large pour qu'un client
/// retrouve son document, assez courte pour rester lisible.
const DOCUMENTS_LISTES_MAX: usize = 300;

/// Au-delà, on ne recopie pas le fichier localement. Elle était de 100 Mo :
/// trop bas — un mémoire ou un PDF plein de photos dépasse souvent 200 Mo,
/// et la même limite a fait échouer l'envoi par QR sur le terrain. La copie
/// se fait sur le disque, sans passer par la mémoire : seule une vidéo ou
/// une image disque dépasse 4 Go.
const TAILLE_MAX_COPIE: u64 = 4 * 1024 * 1024 * 1024;

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

/// Nom du fichier reconnu comme une clé de licence à activer automatiquement.
/// Évite au gérant de retaper à la main une clé de plus de 100 caractères :
/// le porteur du projet écrit la clé dans ce fichier (avec le Bloc-notes,
/// depuis n'importe quel PC connecté) et la remet sur une clé USB.
const NOM_FICHIER_LICENCE: &str = "licence.txt";

/// Un document trouvé sur la clé, PROPOSÉ et non importé.
#[derive(serde::Serialize, Clone, Debug)]
pub struct DocumentUsb {
    pub chemin: String,
    pub nom: String,
    /// Le dossier d'où il vient, pour distinguer deux fichiers de même nom.
    pub dossier: String,
    pub taille_ko: u64,
    pub type_doc: String,
}

/// Ce que la clé actuellement branchée contient, en attente du choix.
static DOCUMENTS_PROPOSES: std::sync::Mutex<Vec<DocumentUsb>> = std::sync::Mutex::new(Vec::new());

/// Les documents trouvés sur la dernière clé branchée.
#[tauri::command]
pub fn documents_cle_usb() -> Vec<DocumentUsb> {
    DOCUMENTS_PROPOSES
        .lock()
        .map(|d| d.clone())
        .unwrap_or_default()
}

/// Met en file d'attente UNIQUEMENT les documents choisis, et rien d'autre.
#[tauri::command]
pub fn importer_documents_usb(app: AppHandle, chemins: Vec<String>) -> usize {
    // On ne prend que des chemins qui étaient réellement proposés : sans
    // cette vérification, cette commande permettrait de faire lire n'importe
    // quel fichier du PC depuis la page.
    let proposes: HashSet<String> = documents_cle_usb().into_iter().map(|d| d.chemin).collect();

    let mut importes = 0;
    for chemin in chemins {
        if !proposes.contains(&chemin) {
            continue;
        }
        let source = PathBuf::from(&chemin);
        let chemin_a_enregistrer = copier_en_local(&app, &source).unwrap_or(source);
        if enqueue_file(&app, &chemin_a_enregistrer, "usb", None, None).is_some() {
            importes += 1;
        }
    }
    importes
}

/// Surveille l'apparition de clés USB et PROPOSE ce qu'elles contiennent.
///
/// Elle importait tout automatiquement, dans la limite de quarante fichiers.
/// Le terrain a montré ce que cela donne : un client tend sa clé, et les
/// centaines de documents qu'elle contient — ses papiers, ses photos —
/// s'affichent dans l'application de la boutique. Ce n'était pas seulement
/// encombrant, c'était une atteinte à sa vie privée, dans un logiciel qui
/// promet par ailleurs que ses documents ne sont vus par personne.
///
/// La clé est donc lue, mais rien n'entre dans la file tant que quelqu'un
/// n'a pas choisi. Le gérant tend l'écran au client, ou choisit avec lui.
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
                let mut trouves = Vec::new();
                scanner_dossier(&app, mount, 0, &mut trouves);

                if let Ok(mut proposes) = DOCUMENTS_PROPOSES.lock() {
                    *proposes = trouves.clone();
                }
                // L'interface ouvre la liste : le gérant n'a pas à deviner
                // qu'il s'est passé quelque chose ni à aller la chercher.
                let _ = app.emit("cle-usb-inseree", trouves.len());
            }

            // Clé retirée : on oublie ce qu'elle proposait, sinon le gérant
            // pourrait importer plus tard depuis une clé qui n'est plus là.
            if !deja_vus.is_empty() && amovibles.is_empty() {
                if let Ok(mut proposes) = DOCUMENTS_PROPOSES.lock() {
                    proposes.clear();
                }
                let _ = app.emit("cle-usb-retiree", ());
            }

            deja_vus = amovibles;
            thread::sleep(Duration::from_secs(2));
        }
    });
}

/// Parcourt la clé et DRESSE LA LISTE, sans rien mettre en file.
fn scanner_dossier(
    app: &AppHandle,
    dossier: &Path,
    profondeur: u32,
    trouves: &mut Vec<DocumentUsb>,
) {
    let Ok(entries) = std::fs::read_dir(dossier) else {
        return;
    };
    for entry in entries.flatten() {
        if trouves.len() >= DOCUMENTS_LISTES_MAX {
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
                scanner_dossier(app, &path, profondeur + 1, trouves);
            }
            continue;
        }
        let nom = entry.file_name();
        if nom
            .to_string_lossy()
            .eq_ignore_ascii_case(NOM_FICHIER_LICENCE)
        {
            // Seule exception qui agit toute seule : ce n'est pas un
            // document de client mais une clé de licence que le porteur du
            // projet a déposée lui-même. La faire choisir n'aurait pas de
            // sens, et le gérant ne saurait pas de quoi il s'agit.
            if let Ok(contenu) = std::fs::read_to_string(&path) {
                crate::license::tenter_activation_depuis_usb(app, contenu.trim());
            }
            continue;
        }
        let type_doc = files::classify(&path);
        if type_doc == "inconnu" {
            continue;
        }
        let taille_ko = std::fs::metadata(&path)
            .map(|m| m.len() / 1024)
            .unwrap_or(0);
        trouves.push(DocumentUsb {
            chemin: path.to_string_lossy().to_string(),
            nom: nom.to_string_lossy().to_string(),
            dossier: dossier.to_string_lossy().to_string(),
            taille_ko,
            type_doc: type_doc.to_string(),
        });
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
