//! Fichiers WhatsApp d'un téléphone branché par câble USB.
//!
//! Le cas du terrain : un client envoie son document au gérant par
//! WhatsApp. Le PC de la boutique n'a pas internet, le fichier est sur le
//! téléphone du gérant. Jusqu'ici, il fallait le brancher, ouvrir
//! l'Explorateur, descendre dans
//! `Android › media › com.whatsapp › WhatsApp › Media › WhatsApp Documents`,
//! retrouver le bon fichier parmi des centaines, le copier dans le dossier
//! surveillé. Beaucoup trop long avec un client qui attend.
//!
//! Ici : un bouton, et la liste des DERNIERS fichiers reçus sur WhatsApp
//! (documents et photos, le plus récent en haut). Le gérant coche, ils
//! entrent dans la file.
//!
//! Un téléphone branché n'a pas de lettre de lecteur (C:, D:…) : Windows le
//! présente par le protocole MTP, que seul l'Explorateur sait parcourir. On
//! passe donc par le même chemin que l'Explorateur (l'objet COM
//! `Shell.Application`), depuis PowerShell, sans droits administrateur et
//! sans rien installer.
//!
//! Limites, dites au gérant dans l'interface :
//! - le téléphone doit être DÉVERROUILLÉ et en mode « Transfert de
//!   fichiers » (Android le propose dans la notification USB ; par défaut
//!   il ne fait que charger) ;
//! - un iPhone ne montre au PC que ses photos, jamais les documents
//!   WhatsApp.

use std::sync::Mutex;

/// Un fichier trouvé sur le téléphone, PROPOSÉ et pas encore copié.
#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct DocumentTelephone {
    /// Identifiant opaque renvoyé par l'interface pour le choisir.
    pub id: String,
    pub nom: String,
    /// « Document » ou « Photo ».
    pub genre: String,
    pub taille_ko: u64,
    /// Date de réception telle que le téléphone la donne (peut être vide).
    pub date: String,
    /// Arrivé depuis le dernier import fait depuis ce téléphone.
    pub nouveau: bool,
    /// Déjà mis en file une fois : ne pas l'imprimer deux fois par erreur.
    pub deja_ajoute: bool,
    /// Coché d'office dans la liste.
    pub coche: bool,
    #[serde(skip)]
    emplacement: Emplacement,
}

/// Ce que l'application retient d'un branchement à l'autre, pour que le
/// gérant n'ait plus qu'à appuyer sur « Ajouter » :
/// - `importes` : les fichiers déjà mis en file ;
/// - `vus` : pour chaque téléphone, la date du plus récent fichier déjà
///   présenté lors d'un import. Tout fichier plus récent est « nouveau ».
///
/// On compare aux dates DU TÉLÉPHONE, jamais à l'horloge du PC : les deux
/// sont souvent décalées, et un fichier reçu il y a une minute paraîtrait
/// sinon ancien.
#[derive(serde::Serialize, serde::Deserialize, Default, Debug, PartialEq)]
struct Memoire {
    importes: Vec<String>,
    vus: std::collections::HashMap<String, String>,
}

/// Au-delà, les plus anciennes entrées sont oubliées : le fichier reste
/// petit, et un document vieux de plusieurs milliers d'envois ne reviendra
/// pas au comptoir.
const IMPORTES_MAX: usize = 3000;

/// Empreinte d'un fichier, et non son nom : la mémoire posée sur le disque
/// du PC ne doit pas devenir une liste lisible des documents des clients.
fn cle(e: &Emplacement, date: &str) -> String {
    use sha2::{Digest, Sha256};
    let empreinte = Sha256::digest(format!("{}|{}|{}|{}", e.appareil, e.nom, e.taille, date));
    empreinte.iter().map(|o| format!("{o:02x}")).collect()
}

/// Marque nouveaux, déjà ajoutés et cochés. Séparée pour être testable.
fn marquer(lecture: &mut LectureTelephone, memoire: &Memoire) {
    let mut premier_coche_par_appareil = std::collections::HashSet::new();
    for document in &mut lecture.documents {
        let appareil = &document.emplacement.appareil;
        document.deja_ajoute = memoire
            .importes
            .contains(&cle(&document.emplacement, &document.date));
        match memoire.vus.get(appareil) {
            Some(deja_vu) => {
                document.nouveau = !document.deja_ajoute && document.date > *deja_vu;
                document.coche = document.nouveau;
            }
            // Premier branchement de ce téléphone : rien n'est « nouveau »,
            // mais le plus récent est presque toujours celui du client qui
            // attend. Les documents sont déjà triés du plus récent au plus
            // ancien.
            None => {
                document.coche = !document.deja_ajoute
                    && premier_coche_par_appareil.insert(appareil.clone());
            }
        }
    }
    // Les nouveaux en haut, puis le reste, chacun du plus récent au plus
    // ancien (le tri est stable).
    lecture.documents.sort_by_key(|d| !d.nouveau);
}

/// Après un import : retient ce qui a été ajouté, et tout ce qui a été
/// présenté devient « déjà vu ».
fn retenir(memoire: &mut Memoire, presentes: &[DocumentTelephone], ajoutes: &[&DocumentTelephone]) {
    for document in ajoutes {
        memoire.importes.push(cle(&document.emplacement, &document.date));
    }
    if memoire.importes.len() > IMPORTES_MAX {
        let surplus = memoire.importes.len() - IMPORTES_MAX;
        memoire.importes.drain(..surplus);
    }
    for document in presentes {
        let entree = memoire
            .vus
            .entry(document.emplacement.appareil.clone())
            .or_default();
        if document.date > *entree {
            *entree = document.date.clone();
        }
    }
}

fn chemin_memoire(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join("telephone_memoire.json"))
}

fn charger_memoire(app: &tauri::AppHandle) -> Memoire {
    chemin_memoire(app)
        .and_then(|c| std::fs::read_to_string(c).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn enregistrer_memoire(app: &tauri::AppHandle, memoire: &Memoire) {
    if let (Some(chemin), Ok(texte)) = (chemin_memoire(app), serde_json::to_string(memoire)) {
        let _ = std::fs::write(chemin, texte);
    }
}

/// De quoi retrouver le fichier sur le téléphone au moment de le copier.
#[derive(Clone, Debug, PartialEq, Default)]
struct Emplacement {
    appareil: String,
    stockage: String,
    dossier: String,
    nom: String,
    taille: u64,
}

/// Ce que la dernière lecture a trouvé.
#[derive(serde::Serialize, Clone, Debug, Default)]
pub struct LectureTelephone {
    /// Nombre de téléphones vus par Windows.
    pub telephones: usize,
    /// Nombre de dossiers WhatsApp trouvés dessus.
    pub dossiers_whatsapp: usize,
    /// Un import a déjà été fait depuis ce téléphone : c'est celui du
    /// gérant. Seul un téléphone connu ouvre la liste tout seul — celui d'un
    /// client branché pour se recharger ne doit JAMAIS afficher ses fichiers
    /// à l'écran de la boutique.
    pub connu: bool,
    pub documents: Vec<DocumentTelephone>,
}

static PROPOSES: Mutex<Vec<DocumentTelephone>> = Mutex::new(Vec::new());

/// Au-delà, la liste deviendrait aussi longue que le dossier lui-même :
/// le document du client qui attend est forcément parmi les plus récents.
const DOCUMENTS_MAX: usize = 60;

/// Où WhatsApp range ce qu'il reçoit. Depuis Android 11, sous
/// `Android/media/…` ; avant, à la racine. WhatsApp Business a ses propres
/// dossiers. Les sous-dossiers (`Sent`, `Private`) sont ignorés : ce sont
/// les fichiers ENVOYÉS par le gérant, pas reçus.
const DOSSIERS_WHATSAPP: [(&str, &str); 9] = [
    // Les téléchargements du téléphone : un fichier reçu par un autre
    // moyen (navigateur, Telegram « Enregistrer », Bluetooth sur certains
    // modèles) arrive souvent là. Le PC n'a toujours besoin d'aucune
    // connexion : tout passe par le câble.
    ("Download", "Téléchargement"),
    (r"Android\media\com.whatsapp\WhatsApp\Media\WhatsApp Documents", "Document"),
    (r"Android\media\com.whatsapp\WhatsApp\Media\WhatsApp Images", "Photo"),
    (
        r"Android\media\com.whatsapp.w4b\WhatsApp Business\Media\WhatsApp Business Documents",
        "Document",
    ),
    (
        r"Android\media\com.whatsapp.w4b\WhatsApp Business\Media\WhatsApp Business Images",
        "Photo",
    ),
    (r"WhatsApp\Media\WhatsApp Documents", "Document"),
    (r"WhatsApp\Media\WhatsApp Images", "Photo"),
    (r"WhatsApp Business\Media\WhatsApp Business Documents", "Document"),
    (r"WhatsApp Business\Media\WhatsApp Business Images", "Photo"),
];

/// Protège un texte pour une chaîne PowerShell entre apostrophes : seule
/// l'apostrophe y a un sens, et elle se double.
fn entre_apostrophes(texte: &str) -> String {
    format!("'{}'", texte.replace('\'', "''"))
}

/// Fonctions communes aux deux scripts : trouver un sous-dossier par son
/// nom, et descendre un chemin `a\b\c` depuis un stockage.
const OUTILS_POWERSHELL: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$shell = New-Object -ComObject Shell.Application
function Enfant($dossier, $nom) {
    foreach ($i in $dossier.Items()) { if ($i.IsFolder -and $i.Name -eq $nom) { return $i.GetFolder } }
    return $null
}
function Descendre($dossier, $chemin) {
    $d = $dossier
    foreach ($morceau in $chemin.Split('\')) { if ($d) { $d = Enfant $d $morceau } }
    return $d
}
function Telephones() {
    # « Ce PC » : les disques y sont des dossiers du système de fichiers,
    # les téléphones (MTP) non.
    foreach ($a in $shell.NameSpace(17).Items()) {
        if (-not $a.IsFileSystem -and $a.IsFolder) { $a }
    }
}
"#;

fn script_lecture() -> String {
    let dossiers = DOSSIERS_WHATSAPP
        .iter()
        .map(|(chemin, genre)| format!("@({}, {})", entre_apostrophes(chemin), entre_apostrophes(genre)))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{OUTILS_POWERSHELL}\n\
         $dossiers = @({dossiers})\n\
         foreach ($tel in Telephones) {{\n\
             $racine = $tel.GetFolder\n\
             if (-not $racine) {{ continue }}\n\
             $stockages = @($racine.Items() | Where-Object {{ $_.IsFolder }})\n\
             \"TELEPHONE`t$($tel.Name)`t$($stockages.Count)\"\n\
             foreach ($stockage in $stockages) {{\n\
                 foreach ($paire in $dossiers) {{\n\
                     $d = Descendre $stockage.GetFolder $paire[0]\n\
                     if (-not $d) {{ continue }}\n\
                     \"DOSSIER`t$($paire[0])\"\n\
                     foreach ($f in $d.Items()) {{\n\
                         if ($f.IsFolder) {{ continue }}\n\
                         $date = ''\n\
                         try {{ $date = $f.ModifyDate.ToString('yyyy-MM-dd HH:mm') }} catch {{}}\n\
                         \"FICHIER`t$($tel.Name)`t$($stockage.Name)`t$($paire[0])`t$($paire[1])`t$($f.Name)`t$($f.Size)`t$date\"\n\
                     }}\n\
                 }}\n\
             }}\n\
         }}\n"
    )
}

/// Traduit la sortie du script de lecture. Séparée pour être testable sans
/// téléphone ni Windows.
fn lire_sortie(sortie: &str) -> LectureTelephone {
    let mut lecture = LectureTelephone::default();
    for ligne in sortie.lines() {
        let champs: Vec<&str> = ligne.trim_end_matches('\r').split('\t').collect();
        match champs.as_slice() {
            ["TELEPHONE", ..] => lecture.telephones += 1,
            ["DOSSIER", ..] => lecture.dossiers_whatsapp += 1,
            ["FICHIER", appareil, stockage, dossier, genre, nom, taille, date] => {
                // Un nom avec un caractère de contrôle ne peut pas venir de
                // WhatsApp et casserait le script de copie.
                if nom.is_empty() || nom.chars().any(char::is_control) {
                    continue;
                }
                let taille: u64 = taille.trim().parse().unwrap_or(0);
                let emplacement = Emplacement {
                    appareil: appareil.to_string(),
                    stockage: stockage.to_string(),
                    dossier: dossier.to_string(),
                    nom: nom.to_string(),
                    taille,
                };
                lecture.documents.push(DocumentTelephone {
                    id: String::new(),
                    nom: nom.to_string(),
                    genre: genre.to_string(),
                    taille_ko: taille.div_ceil(1024),
                    date: date.trim().to_string(),
                    nouveau: false,
                    deja_ajoute: false,
                    coche: false,
                    emplacement,
                });
            }
            _ => {}
        }
    }
    // Le plus récent en haut : c'est celui du client qui attend. Dates au
    // format année-mois-jour : l'ordre du texte est celui du temps.
    lecture.documents.sort_by(|a, b| b.date.cmp(&a.date));
    lecture.documents.truncate(DOCUMENTS_MAX);
    for (indice, document) in lecture.documents.iter_mut().enumerate() {
        document.id = indice.to_string();
    }
    lecture
}

fn script_copie(documents: &[DocumentTelephone], destination: &std::path::Path) -> String {
    let mut script = String::from(OUTILS_POWERSHELL);
    let dossier = entre_apostrophes(&destination.display().to_string());
    script.push_str(&format!(
        "$dossierCible = {dossier}\n\
         $destination = $shell.NameSpace($dossierCible)\n\
         # La copie depuis un téléphone se poursuit en arrière-plan, DANS ce\n\
         # processus : s'il se termine trop tôt, elle est interrompue. On\n\
         # attend donc que le fichier soit complet (3 minutes au plus).\n\
         function Attendre($cible, $taille) {{\n\
             $precedente = -1; $stable = 0\n\
             for ($i = 0; $i -lt 360; $i++) {{\n\
                 Start-Sleep -Milliseconds 500\n\
                 if (-not (Test-Path -LiteralPath $cible)) {{ continue }}\n\
                 $t = (Get-Item -LiteralPath $cible).Length\n\
                 if ($taille -gt 0 -and $t -ge $taille) {{ return $true }}\n\
                 if ($t -gt 0 -and $t -eq $precedente) {{ $stable++ }} else {{ $stable = 0 }}\n\
                 if ($taille -le 0 -and $stable -ge 4) {{ return $true }}\n\
                 $precedente = $t\n\
             }}\n\
             return $false\n\
         }}\n"
    ));
    for document in documents {
        let e = &document.emplacement;
        script.push_str(&format!(
            "$ok = $false\n\
             foreach ($tel in Telephones) {{\n\
                 if ($ok -or $tel.Name -ne {appareil}) {{ continue }}\n\
                 $stockage = Enfant $tel.GetFolder {stockage}\n\
                 if (-not $stockage) {{ continue }}\n\
                 $d = Descendre $stockage {dossier}\n\
                 if (-not $d) {{ continue }}\n\
                 foreach ($f in $d.Items()) {{\n\
                     if ($f.Name -ne {nom}) {{ continue }}\n\
                     # 4 : sans fenêtre de progression ; 16 : oui à tout ;\n\
                     # 1024 : sans message d'erreur à l'écran.\n\
                     $destination.CopyHere($f, 1044)\n\
                     $ok = Attendre (Join-Path $dossierCible $f.Name) $f.Size\n\
                     break\n\
                 }}\n\
             }}\n\
             \"COPIE`t{id}`t$ok\"\n",
            appareil = entre_apostrophes(&e.appareil),
            stockage = entre_apostrophes(&e.stockage),
            dossier = entre_apostrophes(&e.dossier),
            nom = entre_apostrophes(&e.nom),
            id = document.id,
        ));
    }
    script
}

#[cfg(windows)]
fn executer_powershell(script: &str) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    // Un fichier plutôt que `-Command` : le script est long, et un nom de
    // fichier accentué passerait mal par la ligne de commande. La marque
    // UTF-8 (BOM) évite que PowerShell 5.1 le lise comme de l'ANSI.
    let fichier = std::env::temp_dir().join(format!(
        "photocopie-telephone-{}.ps1",
        rand::random::<u32>()
    ));
    std::fs::write(&fichier, format!("\u{FEFF}{script}"))
        .map_err(|e| format!("Impossible de préparer la lecture du téléphone ({e})."))?;
    let sortie = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-STA", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&fichier)
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let _ = std::fs::remove_file(&fichier);
    sortie
        .map(|s| String::from_utf8_lossy(&s.stdout).into_owned())
        .map_err(|e| format!("Windows n'a pas pu lire le téléphone ({e})."))
}

#[cfg(not(windows))]
fn executer_powershell(_script: &str) -> Result<String, String> {
    Err("Disponible uniquement sur Windows".to_string())
}

/// Lit le téléphone branché et PROPOSE ses derniers fichiers WhatsApp.
#[tauri::command]
pub async fn documents_whatsapp_telephone(
    app: tauri::AppHandle,
) -> Result<LectureTelephone, String> {
    let sortie = tauri::async_runtime::spawn_blocking(|| executer_powershell(&script_lecture()))
        .await
        .map_err(|e| e.to_string())??;
    let mut lecture = lire_sortie(&sortie);
    let memoire = charger_memoire(&app);
    lecture.connu = lecture
        .documents
        .iter()
        .any(|d| memoire.vus.contains_key(&d.emplacement.appareil));
    marquer(&mut lecture, &memoire);
    if let Ok(mut proposes) = PROPOSES.lock() {
        *proposes = lecture.documents.clone();
    }
    Ok(lecture)
}

/// Copie les fichiers choisis sur le PC et les met en file. Rend le nombre
/// de fichiers réellement ajoutés.
#[tauri::command]
pub async fn importer_documents_telephone(
    app: tauri::AppHandle,
    ids: Vec<String>,
) -> Result<usize, String> {
    use tauri::Manager;

    // Seulement des fichiers réellement proposés par la dernière lecture.
    let presentes: Vec<DocumentTelephone> =
        PROPOSES.lock().map(|p| p.clone()).unwrap_or_default();
    let choisis: Vec<DocumentTelephone> = presentes
        .iter()
        .filter(|d| ids.contains(&d.id))
        .cloned()
        .collect();
    if choisis.is_empty() {
        return Ok(0);
    }

    // Un dossier neuf par import : deux clients peuvent avoir envoyé
    // « Document.pdf », et aucun ne doit écraser l'autre.
    let destination = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("recus_telephone")
        .join(chrono::Local::now().format("%Y%m%d-%H%M%S%3f").to_string());
    std::fs::create_dir_all(&destination).map_err(|e| e.to_string())?;

    let script = script_copie(&choisis, &destination);
    tauri::async_runtime::spawn_blocking(move || executer_powershell(&script))
        .await
        .map_err(|e| e.to_string())??;

    // La copie depuis un téléphone continue APRÈS la fin du script (Windows
    // la fait en arrière-plan). On attend que chaque fichier soit là, à sa
    // taille complète — sans quoi on mettrait en file un fichier tronqué.
    let mut ajoutes = Vec::new();
    for document in &choisis {
        let chemin = destination.join(&document.emplacement.nom);
        if attendre_copie_complete(&chemin, document.emplacement.taille).await
            && crate::watcher::enqueue_file(&app, &chemin, "telephone", None, None).is_some()
        {
            ajoutes.push(document);
        }
    }
    let mut memoire = charger_memoire(&app);
    retenir(&mut memoire, &presentes, &ajoutes);
    enregistrer_memoire(&app, &memoire);
    Ok(ajoutes.len())
}

/// Surveille le branchement des téléphones (mode « Transfert de fichiers »)
/// et prévient l'interface, qui ouvre la liste toute seule : le gérant n'a
/// même plus à appuyer sur 📱.
///
/// Une clé USB apparaît aussi comme « appareil portable » pour Windows :
/// l'interface ne dit rien quand la lecture ne trouve aucun téléphone.
pub fn surveiller_telephones(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        use tauri::Emitter;
        let mut connus = identifiants_telephones().unwrap_or_default();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3));
            let Some(actuels) = identifiants_telephones() else { continue };
            if actuels.difference(&connus).next().is_some() {
                let _ = app.emit("telephone-branche", ());
            }
            connus = actuels;
        }
    });
}

#[cfg(windows)]
fn identifiants_telephones() -> Option<std::collections::HashSet<String>> {
    use windows::Devices::Enumeration::DeviceInformation;
    use windows::Devices::Portable::StorageDevice;

    let selecteur = StorageDevice::GetDeviceSelector().ok()?;
    let appareils = DeviceInformation::FindAllAsyncAqsFilter(&selecteur)
        .ok()?
        .get()
        .ok()?;
    let mut ids = std::collections::HashSet::new();
    for indice in 0..appareils.Size().unwrap_or(0) {
        if let Ok(id) = appareils.GetAt(indice).and_then(|a| a.Id()) {
            ids.insert(id.to_string());
        }
    }
    Some(ids)
}

#[cfg(not(windows))]
fn identifiants_telephones() -> Option<std::collections::HashSet<String>> {
    None
}

/// Vrai quand le fichier est arrivé en entier. Si le téléphone n'a pas
/// donné la taille, on attend qu'elle ne bouge plus.
async fn attendre_copie_complete(chemin: &std::path::Path, taille_attendue: u64) -> bool {
    const DELAI_MAX: std::time::Duration = std::time::Duration::from_secs(180);
    let debut = std::time::Instant::now();
    let mut precedente = None;
    let mut stable_depuis = 0u32;
    while debut.elapsed() < DELAI_MAX {
        if let Ok(meta) = std::fs::metadata(chemin) {
            let taille = meta.len();
            if taille_attendue > 0 && taille >= taille_attendue {
                return true;
            }
            if taille_attendue == 0 && taille > 0 {
                stable_depuis = if precedente == Some(taille) { stable_depuis + 1 } else { 0 };
                if stable_depuis >= 4 {
                    return true;
                }
            }
            precedente = Some(taille);
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lit_les_fichiers_du_plus_recent_au_plus_ancien() {
        let sortie = "TELEPHONE\tGalaxy A12\t1\r\n\
                      DOSSIER\tAndroid\\media\\com.whatsapp\\WhatsApp\\Media\\WhatsApp Documents\r\n\
                      FICHIER\tGalaxy A12\tStockage interne partagé\tAndroid\\media\\com.whatsapp\\WhatsApp\\Media\\WhatsApp Documents\tDocument\tmemoire.pdf\t230686720\t2026-09-24 10:02\r\n\
                      FICHIER\tGalaxy A12\tStockage interne partagé\tAndroid\\media\\com.whatsapp\\WhatsApp\\Media\\WhatsApp Documents\tDocument\tcv d'Awa.docx\t20480\t2026-09-25 16:40\r\n";
        let lecture = lire_sortie(sortie);
        assert_eq!(lecture.telephones, 1);
        assert_eq!(lecture.dossiers_whatsapp, 1);
        assert_eq!(lecture.documents.len(), 2);
        assert_eq!(lecture.documents[0].nom, "cv d'Awa.docx");
        assert_eq!(lecture.documents[0].id, "0");
        assert_eq!(lecture.documents[1].taille_ko, 225_280);
    }

    fn fichier(nom: &str, date: &str) -> String {
        format!("FICHIER\tTel\tInterne\tWhatsApp\\Media\\WhatsApp Documents\tDocument\t{nom}\t100\t{date}\n")
    }

    #[test]
    fn premier_branchement_seul_le_plus_recent_est_coche() {
        let mut lecture = lire_sortie(&(fichier("ancien.pdf", "2026-09-20 10:00")
            + &fichier("client.pdf", "2026-09-26 09:00")));
        marquer(&mut lecture, &Memoire::default());
        assert_eq!(lecture.documents[0].nom, "client.pdf");
        assert!(lecture.documents[0].coche);
        assert!(!lecture.documents[1].coche);
        assert!(lecture.documents.iter().all(|d| !d.nouveau));
    }

    #[test]
    fn les_fichiers_arrives_depuis_le_dernier_import_sont_nouveaux_et_coches() {
        // Premier passage : on importe client1.pdf.
        let sortie1 = fichier("ancien.pdf", "2026-09-20 10:00") + &fichier("client1.pdf", "2026-09-26 09:00");
        let mut lecture1 = lire_sortie(&sortie1);
        marquer(&mut lecture1, &Memoire::default());
        let mut memoire = Memoire::default();
        let importe = lecture1.documents.iter().find(|d| d.nom == "client1.pdf").unwrap();
        retenir(&mut memoire, &lecture1.documents, &[importe]);

        // Deuxième branchement : deux fichiers d'un nouveau client.
        let sortie2 = sortie1 + &fichier("client2-a.pdf", "2026-09-26 11:02")
            + &fichier("client2-b.jpg", "2026-09-26 11:03");
        let mut lecture2 = lire_sortie(&sortie2);
        marquer(&mut lecture2, &memoire);
        let noms_coches: Vec<&str> = lecture2
            .documents
            .iter()
            .filter(|d| d.coche)
            .map(|d| d.nom.as_str())
            .collect();
        assert_eq!(noms_coches, vec!["client2-b.jpg", "client2-a.pdf"]);
        assert!(lecture2.documents[0].nouveau && lecture2.documents[1].nouveau);
        let deja = lecture2.documents.iter().find(|d| d.nom == "client1.pdf").unwrap();
        assert!(deja.deja_ajoute && !deja.coche && !deja.nouveau);
    }

    #[test]
    fn la_memoire_se_relit_telle_quelle() {
        let mut memoire = Memoire::default();
        memoire.importes.push("0f3a".to_string());
        memoire.vus.insert("Tel".to_string(), "2026-09-26 09:00".to_string());
        let texte = serde_json::to_string(&memoire).unwrap();
        assert_eq!(serde_json::from_str::<Memoire>(&texte).unwrap(), memoire);
    }

    #[test]
    fn telephone_verrouille_ou_en_simple_charge() {
        // Windows voit le téléphone mais aucun stockage : il est verrouillé
        // ou en mode « charge uniquement ».
        let lecture = lire_sortie("TELEPHONE\tRedmi 9\t0\n");
        assert_eq!(lecture.telephones, 1);
        assert_eq!(lecture.dossiers_whatsapp, 0);
        assert!(lecture.documents.is_empty());
    }

    #[test]
    fn un_nom_avec_apostrophe_ne_casse_pas_le_script_de_copie() {
        let lecture = lire_sortie(
            "FICHIER\tTel\tInterne\tWhatsApp\\Media\\WhatsApp Documents\tDocument\tcv d'Awa.pdf\t10\t\n",
        );
        let script = script_copie(&lecture.documents, std::path::Path::new("C:\\x"));
        assert!(script.contains("'cv d''Awa.pdf'"));
    }

    #[test]
    fn la_liste_reste_courte() {
        let mut sortie = String::new();
        for i in 0..200 {
            sortie.push_str(&format!(
                "FICHIER\tTel\tInterne\tWhatsApp\\Media\\WhatsApp Images\tPhoto\tIMG-{i:03}.jpg\t1000\t2026-09-{:02} 10:00\n",
                1 + i % 28
            ));
        }
        assert_eq!(lire_sortie(&sortie).documents.len(), DOCUMENTS_MAX);
    }

    #[test]
    fn le_script_de_lecture_cherche_aussi_whatsapp_business_et_les_anciens_android() {
        let script = script_lecture();
        assert!(script.contains(r"Android\media\com.whatsapp\WhatsApp\Media\WhatsApp Documents"));
        assert!(script.contains("com.whatsapp.w4b"));
        assert!(script.contains(r"'WhatsApp\Media\WhatsApp Documents'"));
    }
}
