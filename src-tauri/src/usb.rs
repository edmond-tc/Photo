use crate::files;
use crate::watcher::enqueue_file;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use sysinfo::Disks;
use tauri::{AppHandle, Emitter, Manager};

/// Profondeur de recherche dans les dossiers de la clé. Elle était de 2 :
/// un document rangé dans « Documents › Cours › 2026 » n'était jamais vu.
const PROFONDEUR_MAX: u32 = 6;

/// Au-delà de ce nombre d'éléments parcourus, on s'arrête : une grosse clé
/// (disque de sauvegarde, milliers de photos) ne doit pas faire attendre.
const ELEMENTS_PARCOURUS_MAX: usize = 20_000;

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
    /// Date de modification (secondes) : les plus récents en haut de la
    /// liste, là où est presque toujours le document du client.
    #[serde(skip)]
    pub modifie: u64,
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
pub async fn importer_documents_usb(app: AppHandle, chemins: Vec<String>) -> usize {
    // Copier depuis une clé peut prendre longtemps (gros fichiers, clé
    // lente) : hors du fil de la fenêtre, qui sinon se figeait.
    tauri::async_runtime::spawn_blocking(move || importer(&app, chemins))
        .await
        .unwrap_or(0)
}

fn importer(app: &AppHandle, chemins: Vec<String>) -> usize {
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
        let chemin_a_enregistrer = copier_en_local(app, &source).unwrap_or(source);
        if enqueue_file(app, &chemin_a_enregistrer, "usb", None, None).is_some() {
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
        // Ce qui est déjà branché au lancement (disque USB de sauvegarde,
        // lecteur de cartes) n'ouvre pas la liste à chaque démarrage : seul
        // un BRANCHEMENT la fait apparaître. Le bouton 💾 relit à la demande.
        let mut deja_vus: HashSet<PathBuf> = lecteurs_amovibles();
        // Le lecteur dont la liste est affichée en ce moment.
        let mut propose: Option<PathBuf> = None;
        // Lecteurs apparus mais pas encore lisibles, avec le nombre d'essais.
        let mut en_attente: std::collections::HashMap<PathBuf, u32> = std::collections::HashMap::new();

        loop {
            let amovibles = lecteurs_amovibles();

            for mount in amovibles.difference(&deja_vus) {
                // Windows donne la lettre du lecteur une ou deux secondes
                // AVANT que son contenu soit lisible. Lue trop tôt, la clé
                // paraissait vide et n'était plus jamais relue : « la clé
                // n'est pas détectée ». On réessaie donc (20 s au plus).
                let essais = en_attente.entry(mount.clone()).or_insert(0);
                *essais += 1;
                let lisible = std::fs::read_dir(mount).is_ok();
                if !lisible && *essais < 10 {
                    continue;
                }
                let mut trouves = lire_cle(&app, mount);
                if trouves.is_empty() && lisible && *essais < 3 {
                    // Lisible mais vide : peut-être encore en train de se
                    // monter. Un dernier regard avant de conclure.
                    continue;
                }
                en_attente.remove(mount);
                trouves.truncate(DOCUMENTS_LISTES_MAX);

                if let Ok(mut proposes) = DOCUMENTS_PROPOSES.lock() {
                    *proposes = trouves.clone();
                }
                propose = Some(mount.clone());
                // L'interface ouvre la liste : le gérant n'a pas à deviner
                // qu'il s'est passé quelque chose ni à aller la chercher.
                let _ = app.emit("cle-usb-inseree", trouves.len());
            }
            en_attente.retain(|m, _| amovibles.contains(m));

            // Clé retirée : on oublie ce qu'elle proposait, sinon le gérant
            // pourrait importer plus tard depuis une clé qui n'est plus là.
            //
            // On regarde CE lecteur-là, pas « plus aucun lecteur amovible » :
            // avec un disque dur USB ou un lecteur de cartes branché en
            // permanence, il reste toujours un lecteur, et le retrait de la
            // clé du client n'était jamais vu.
            if propose.as_ref().is_some_and(|m| !amovibles.contains(m)) {
                propose = None;
                if let Ok(mut proposes) = DOCUMENTS_PROPOSES.lock() {
                    proposes.clear();
                }
                let _ = app.emit("cle-usb-retiree", ());
            }

            // Un lecteur encore illisible n'est pas « vu » : il sera relu.
            deja_vus = amovibles
                .into_iter()
                .filter(|m| !en_attente.contains_key(m))
                .collect();
            thread::sleep(Duration::from_secs(2));
        }
    });
}

/// Bouton 💾 : relit tout de suite les clés branchées (liste fermée par
/// erreur, clé branchée avant l'ouverture de l'application…). Rend le
/// nombre de documents proposés, ou `None` s'il n'y a aucune clé.
#[tauri::command]
pub async fn relire_cles_usb(app: AppHandle) -> Option<usize> {
    tauri::async_runtime::spawn_blocking(move || {
        let lecteurs = lecteurs_amovibles();
        if lecteurs.is_empty() {
            return None;
        }
        let mut trouves = Vec::new();
        for lecteur in &lecteurs {
            trouves.extend(lire_cle(&app, lecteur));
        }
        trouves.truncate(DOCUMENTS_LISTES_MAX);
        let nombre = trouves.len();
        if let Ok(mut proposes) = DOCUMENTS_PROPOSES.lock() {
            *proposes = trouves;
        }
        Some(nombre)
    })
    .await
    .ok()
    .flatten()
}

/// Tous les documents d'une clé, les plus récents d'abord.
fn lire_cle(app: &AppHandle, lecteur: &Path) -> Vec<DocumentUsb> {
    let mut trouves = Vec::new();
    let mut parcourus = 0usize;
    scanner_dossier(app, lecteur, 0, &mut trouves, &mut parcourus);
    trouves.sort_by_key(|d| std::cmp::Reverse(d.modifie));
    trouves
}

/// Les lecteurs d'une clé USB, d'une carte mémoire ou d'un disque USB.
///
/// Windows ne marque « amovibles » qu'une partie des clés : beaucoup de
/// clés récentes et TOUS les disques durs USB se déclarent « disque fixe »,
/// comme le disque du PC. Ils n'étaient jamais détectés. On demande donc
/// aussi à chaque lecteur par quel câble il est relié (USB, carte SD).
fn lecteurs_amovibles() -> HashSet<PathBuf> {
    let systeme = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
    Disks::new_with_refreshed_list()
        .iter()
        .filter(|d| {
            let point = d.mount_point();
            let lettre = point.to_string_lossy();
            !lettre.to_ascii_uppercase().starts_with(&systeme.to_ascii_uppercase())
                && (d.is_removable() || relie_par_usb(point))
        })
        .map(|d| d.mount_point().to_path_buf())
        .collect()
}

/// Le lecteur est-il relié par USB ou par un lecteur de cartes ?
#[cfg(windows)]
fn relie_par_usb(point: &Path) -> bool {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Storage::FileSystem::{
        BusTypeMmc, BusTypeSd, BusTypeUsb, CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Ioctl::{
        PropertyStandardQuery, StorageDeviceProperty, IOCTL_STORAGE_QUERY_PROPERTY,
        STORAGE_DEVICE_DESCRIPTOR, STORAGE_PROPERTY_QUERY,
    };
    use windows::Win32::System::IO::DeviceIoControl;

    let texte = point.to_string_lossy();
    let lettre = texte.trim_end_matches('\\');
    if lettre.len() != 2 || !lettre.ends_with(':') {
        return false;
    }
    // SAFETY : appels Windows classiques ; la poignée est fermée ci-dessous,
    // les tampons vivent jusqu'à la fin des appels. Accès « 0 » : on ne lit
    // que la description du lecteur, aucun droit administrateur requis.
    unsafe {
        let Ok(poignee) = CreateFileW(
            &HSTRING::from(format!("\\\\.\\{lettre}")),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        ) else {
            return false;
        };
        let question = STORAGE_PROPERTY_QUERY {
            PropertyId: StorageDeviceProperty,
            QueryType: PropertyStandardQuery,
            AdditionalParameters: [0],
        };
        let mut reponse = [0u8; 1024];
        let mut octets = 0u32;
        let ok = DeviceIoControl(
            poignee,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(&question as *const _ as *const core::ffi::c_void),
            std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            Some(reponse.as_mut_ptr() as *mut core::ffi::c_void),
            reponse.len() as u32,
            Some(&mut octets),
            None,
        )
        .is_ok();
        let _ = CloseHandle(poignee);
        if !ok || (octets as usize) < std::mem::size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
            return false;
        }
        let description = std::ptr::read_unaligned(reponse.as_ptr() as *const STORAGE_DEVICE_DESCRIPTOR);
        [BusTypeUsb, BusTypeSd, BusTypeMmc].contains(&description.BusType)
    }
}

#[cfg(not(windows))]
fn relie_par_usb(_point: &Path) -> bool {
    false
}

/// Parcourt la clé et DRESSE LA LISTE, sans rien mettre en file.
fn scanner_dossier(
    app: &AppHandle,
    dossier: &Path,
    profondeur: u32,
    trouves: &mut Vec<DocumentUsb>,
    parcourus: &mut usize,
) {
    let Ok(entries) = std::fs::read_dir(dossier) else {
        return;
    };
    for entry in entries.flatten() {
        *parcourus += 1;
        if *parcourus >= ELEMENTS_PARCOURUS_MAX || trouves.len() >= DOCUMENTS_LISTES_MAX * 4 {
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
                scanner_dossier(app, &path, profondeur + 1, trouves, parcourus);
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
        let meta = std::fs::metadata(&path).ok();
        let taille_ko = meta.as_ref().map(|m| m.len() / 1024).unwrap_or(0);
        let modifie = meta
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        trouves.push(DocumentUsb {
            modifie,
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
