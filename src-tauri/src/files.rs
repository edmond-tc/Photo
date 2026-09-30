use std::path::Path;

/// Taille au-delà de laquelle on prévient le gérant qu'un fichier est
/// volumineux (section 4 : "indicateur de progression clair").
pub const SEUIL_FICHIER_VOLUMINEUX: u64 = 20 * 1024 * 1024; // 20 Mo

/// Limite de lecture pour les diagnostics PDF ci-dessous : au-delà, on ne
/// scanne pas le fichier (documents de boutique de photocopie rarement
/// aussi gros) plutôt que de ralentir la réception.
const LIMITE_DIAGNOSTIC_PDF: u64 = 25 * 1024 * 1024;

/// Anciens formats Office capables de porter des macros (les .docx, .xlsx
/// et .pptx ne le peuvent pas ; les .docm & co. restent « inconnus »).
const FORMATS_A_MACROS: [&str; 4] = ["doc", "xls", "ppt", "rtf"];

/// Marque un document de client comme « venu d'internet » (la marque que
/// Windows pose sur un téléchargement). Office l'ouvre alors en mode protégé
/// et bloque toute macro, sans bouton pour l'activer : un .doc piégé envoyé
/// par un client ne peut rien lancer sur le PC de la boutique. Le gérant
/// touche « Activer la modification » pour éditer, comme pour une pièce
/// jointe de courriel.
pub fn marquer_si_macros_possibles(path: &Path) {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !FORMATS_A_MACROS.contains(&ext.as_str()) {
        return;
    }
    #[cfg(windows)]
    {
        let mut flux = path.as_os_str().to_owned();
        flux.push(":Zone.Identifier");
        let _ = std::fs::write(flux, "[ZoneTransfer]\r\nZoneId=3\r\n");
    }
}

/// Classe un fichier reçu selon le routage décrit dans le cahier des charges :
/// PDF/image -> impression directe ; bureautique -> ouverture dans l'éditeur natif.
pub fn classify(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "exe" | "msi" => "installateur",
        "pdf" | "jpg" | "jpeg" | "png" | "bmp" | "tif" | "tiff" | "gif" | "webp" => "imprimable",
        "doc" | "docx" | "ppt" | "pptx" | "xls" | "xlsx" | "txt" | "rtf" | "odt" | "odp"
        | "ods" | "csv" => "editable",
        _ => "inconnu",
    }
}

/// Génère une petite vignette base64 pour les fichiers image (aperçu dans
/// la file d'attente). Renvoie None pour les autres formats — l'interface
/// affiche alors une icône générique par type, pas la peine de réinventer
/// un moteur de rendu PDF pour une simple vignette (section 3 : router vers
/// les outils déjà connus plutôt que tout réinventer).
pub fn miniature_base64(path: &Path) -> Option<String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !matches!(
        ext.as_str(),
        "jpg" | "jpeg" | "png" | "bmp" | "gif" | "webp"
    ) {
        return None;
    }

    let img = image::open(path).ok()?;
    let vignette = img.thumbnail(96, 96);
    let mut buffer = std::io::Cursor::new(Vec::new());
    vignette
        .write_to(&mut buffer, image::ImageFormat::Png)
        .ok()?;

    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    Some(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(buffer.into_inner())
    ))
}

/// Limite pour l'aperçu intégré (au-delà, le data URI deviendrait trop
/// volumineux pour la vue web — le gérant utilise alors directement le
/// bouton Imprimer, qui fonctionne quelle que soit la taille du fichier).
const LIMITE_APERCU: u64 = 15 * 1024 * 1024;

fn type_mime(ext: &str) -> Option<&'static str> {
    match ext {
        "pdf" => Some("application/pdf"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "bmp" => Some("image/bmp"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "tif" | "tiff" => Some("image/tiff"),
        _ => None,
    }
}

/// Lit un fichier imprimable (PDF/image) et le renvoie en data URI, pour un
/// aperçu affiché directement dans l'application — le gérant voit le
/// document et imprime depuis le même écran, sans ouvrir une autre
/// application entre les deux (pas de va-et-vient).
pub fn apercu_data_uri(path: &Path) -> Result<String, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mime = type_mime(&ext).ok_or_else(|| "Aperçu non disponible pour ce format".to_string())?;

    let taille = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if taille > LIMITE_APERCU {
        return Err(format!(
            "Fichier trop volumineux pour l'aperçu intégré ({:.1} Mo) — imprimez directement.",
            taille as f64 / 1024.0 / 1024.0
        ));
    }

    let contenu = std::fs::read(path).map_err(|e| e.to_string())?;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    Ok(format!("data:{mime};base64,{}", STANDARD.encode(contenu)))
}

/// Diagnostics légers sur un PDF, par lecture directe des octets (pas un
/// vrai analyseur PDF — cf. limites documentées dans le README). Renvoie
/// (protégé_par_mot_de_passe, format_papier_detecte).
pub fn diagnostiquer_pdf(path: &Path) -> (bool, Option<&'static str>) {
    let est_pdf = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false);
    if !est_pdf {
        return (false, None);
    }
    let Ok(meta) = std::fs::metadata(path) else {
        return (false, None);
    };
    if meta.len() > LIMITE_DIAGNOSTIC_PDF {
        return (false, None);
    }
    let Ok(contenu) = std::fs::read(path) else {
        return (false, None);
    };

    let protege = contient_motif(&contenu, b"/Encrypt");
    let format = detecter_format_papier(&contenu);
    (protege, format)
}

fn contient_motif(hay: &[u8], motif: &[u8]) -> bool {
    hay.windows(motif.len()).any(|fenetre| fenetre == motif)
}

/// Cherche `/MediaBox [x0 y0 x1 y1]` (en points, 1/72 pouce) et compare aux
/// dimensions standard. Best-effort : peut manquer les PDF dont les objets
/// de page sont dans un flux compressé (PDF 1.5+, "object streams").
fn detecter_format_papier(contenu: &[u8]) -> Option<&'static str> {
    let texte = String::from_utf8_lossy(contenu);
    let debut = texte.find("/MediaBox")?;
    let apres = &texte[debut + "/MediaBox".len()..];
    let ouverture = apres.find('[')?;
    let fermeture = apres.find(']')?;
    if fermeture < ouverture {
        return None;
    }
    let nombres: Vec<f64> = apres[ouverture + 1..fermeture]
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();
    let [x0, y0, x1, y1] = nombres[..].try_into().ok()?;
    let largeur = (x1 - x0).abs();
    let hauteur = (y1 - y0).abs();
    let (petit, grand) = if largeur < hauteur {
        (largeur, hauteur)
    } else {
        (hauteur, largeur)
    };

    const TOLERANCE: f64 = 5.0;
    const A4: (f64, f64) = (595.0, 842.0);
    const LETTER: (f64, f64) = (612.0, 792.0);

    if (petit - A4.0).abs() < TOLERANCE && (grand - A4.1).abs() < TOLERANCE {
        None // déjà au format A4, rien à signaler
    } else if (petit - LETTER.0).abs() < TOLERANCE && (grand - LETTER.1).abs() < TOLERANCE {
        Some("US Letter")
    } else {
        None
    }
}

/// Ouvre le fichier avec le programme associé par défaut sur Windows (Word,
/// LibreOffice, etc. selon ce que le gérant a déjà installé).
#[cfg(windows)]
pub fn shell_open(path: &Path, verb: &str) -> Result<(), String> {
    shell_executer(path, verb, None)
}

/// Envoie le fichier à l'impression sur UNE imprimante précise plutôt que
/// celle par défaut — utilisé quand le gérant a choisi une imprimante dans
/// la liste déroulante (voir `commands::print_file`). Le verbe "printto" du
/// Shell Windows attend le nom de l'imprimante entre guillemets.
#[cfg(windows)]
pub fn shell_print_vers(path: &Path, imprimante: &str) -> Result<(), String> {
    shell_executer(path, "printto", Some(&format!("\"{imprimante}\"")))
}

#[cfg(windows)]
fn shell_executer(path: &Path, verb: &str, parametres: Option<&str>) -> Result<(), String> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::ShellExecuteW;

    let path_h = HSTRING::from(path.as_os_str());
    let verb_h = HSTRING::from(verb);
    let parametres_h = parametres.map(HSTRING::from);
    let parametres_ptr = parametres_h
        .as_ref()
        .map(|h| PCWSTR(h.as_ptr()))
        .unwrap_or(PCWSTR::null());

    // SAFETY: appel FFI standard vers l'API Shell de Windows, aucune mémoire
    // n'est retenue au-delà de l'appel (les HSTRING restent en vie jusque-là).
    let result = unsafe {
        ShellExecuteW(
            HWND(std::ptr::null_mut()),
            PCWSTR(verb_h.as_ptr()),
            PCWSTR(path_h.as_ptr()),
            parametres_ptr,
            PCWSTR::null(),
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        )
    };

    // ShellExecuteW renvoie un HINSTANCE ; une valeur <= 32 signale une erreur.
    if (result.0 as isize) <= 32 {
        return Err(format!(
            "Impossible d'ouvrir le fichier (code {})",
            result.0 as isize
        ));
    }
    Ok(())
}

/// Les documents qu'une boutique ouvre tous les jours, et eux seuls, sont
/// ouverts directement. Tout autre fichier reçu — script, raccourci, page
/// web, archive, type inconnu — pourrait exécuter quelque chose au simple
/// « Ouvrir » : il est montré dans son dossier, jamais lancé. Une liste de
/// ce qui est permis, pas de ce qui est interdit : un type dangereux oublié
/// reste ainsi fermé par défaut.
pub fn ouvrable_sans_risque(path: &Path) -> bool {
    const DOCUMENTS: [&str; 25] = [
        "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "pub", "odt", "ods", "odp", "rtf",
        "txt", "csv", "jpg", "jpeg", "png", "gif", "bmp", "webp", "heic", "heif", "tif", "tiff",
        "xps",
    ];
    crate::signature_maj::extension_reelle(path).is_some_and(|e| DOCUMENTS.contains(&e.as_str()))
}

/// Ouvre l'Explorateur sur le dossier du fichier, le fichier sélectionné,
/// sans rien lancer.
pub fn montrer_dans_dossier(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut argument = std::ffi::OsString::from("/select,");
        argument.push(path.as_os_str());
        std::process::Command::new("explorer.exe")
            .raw_arg(argument)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Impossible d'ouvrir le dossier : {e}"))
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("disponible seulement sous Windows".to_string())
    }
}

/// Le message rendu au gérant quand un fichier n'est pas ouvert : il
/// commence par 🔒, que l'interface affiche tel quel.
pub fn refus_type_inconnu(path: &Path) -> String {
    let extension = crate::signature_maj::extension_reelle(path)
        .map(|e| format!(".{e}"))
        .unwrap_or_else(|| "sans extension".to_string());
    format!(
        "🔒 Par sécurité, ce type de fichier ({extension}) n'est pas ouvert directement : il pourrait \
         lancer un programme. Son dossier vient de s'ouvrir. N'y touchez que si vous connaissez \
         le client et le fichier."
    )
}

#[cfg(test)]
mod tests_types_ouvrables {
    use std::path::Path;
    #[test]
    fn seuls_les_documents_courants_s_ouvrent() {
        assert!(super::ouvrable_sans_risque(Path::new("CV Koffi.PDF")));
        assert!(super::ouvrable_sans_risque(Path::new("photo.jpeg")));
        assert!(!super::ouvrable_sans_risque(Path::new("facture.pdf.url")));
        assert!(!super::ouvrable_sans_risque(Path::new("script.wsf")));
        assert!(!super::ouvrable_sans_risque(Path::new("page.html")));
        assert!(!super::ouvrable_sans_risque(Path::new("macro.docm")));
        assert!(!super::ouvrable_sans_risque(Path::new("sans_extension")));
        assert!(!super::ouvrable_sans_risque(Path::new("virus.exe.")));
    }
}

/// Ouvre un document comme le gérant en a l'habitude. Pour un PDF : dans le
/// vrai lecteur PDF installé sur le PC (Adobe, Foxit, Sumatra…), même si
/// Windows a mis Edge par défaut — demandé sur le terrain : la fenêtre
/// d'impression d'Edge n'offre pas les « Propriétés » de l'imprimante.
pub fn ouvrir_document(path: &Path) -> Result<(), String> {
    let est_pdf = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
    if est_pdf {
        if let Some(lecteur) = lecteur_pdf_installe() {
            if std::process::Command::new(&lecteur).arg(path).spawn().is_ok() {
                return Ok(());
            }
        }
    }
    shell_open(path, "open")
}

/// Le premier lecteur PDF « de bureau » trouvé sur ce PC.
#[cfg(windows)]
pub fn lecteur_pdf_installe() -> Option<std::path::PathBuf> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const PROGRAMMES: [&str; 10] = [
        "AcroRd32.exe", "Acrobat.exe", "FoxitPDFReader.exe", "FoxitReader.exe",
        "FoxitPDFEditor.exe", "SumatraPDF.exe", "PDFXEdit.exe", "PDFXCview.exe",
        "NitroPDF.exe", "NitroPDFReader.exe",
    ];
    const RACINES: [&str; 3] = [
        r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths",
        r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths",
        r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths",
    ];
    for programme in PROGRAMMES {
        for racine in RACINES {
            let Ok(sortie) = std::process::Command::new("reg")
                .args(["query", &format!("{racine}\\{programme}"), "/ve"])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
            else {
                continue;
            };
            if let Some(chemin) = lire_valeur_reg(&String::from_utf8_lossy(&sortie.stdout)) {
                let chemin = std::path::PathBuf::from(chemin);
                if chemin.is_file() {
                    return Some(chemin);
                }
            }
        }
    }
    // Emplacements habituels, pour une installation sans « App Paths ».
    for base in [std::env::var("ProgramFiles").ok(), std::env::var("ProgramFiles(x86)").ok()].into_iter().flatten() {
        for relatif in [
            r"Adobe\Acrobat DC\Acrobat\Acrobat.exe",
            r"Adobe\Acrobat Reader DC\Reader\AcroRd32.exe",
            r"Adobe\Acrobat Reader\Reader\AcroRd32.exe",
            r"Foxit Software\Foxit PDF Reader\FoxitPDFReader.exe",
            r"SumatraPDF\SumatraPDF.exe",
        ] {
            let chemin = std::path::Path::new(&base).join(relatif);
            if chemin.is_file() {
                return Some(chemin);
            }
        }
    }
    None
}

#[cfg(not(windows))]
pub fn lecteur_pdf_installe() -> Option<std::path::PathBuf> {
    None
}

/// Lit la valeur d'une ligne `reg query … /ve` : « (par défaut) REG_SZ C:\… ».
fn lire_valeur_reg(sortie: &str) -> Option<String> {
    sortie.lines().find_map(|ligne| {
        let (_, valeur) = ligne.split_once("REG_SZ").or_else(|| ligne.split_once("REG_EXPAND_SZ"))?;
        let valeur = valeur.trim().trim_matches('"').to_string();
        (!valeur.is_empty()).then_some(valeur)
    })
}

#[cfg(test)]
mod tests_lecteur_pdf {
    #[test]
    fn lit_le_chemin_du_lecteur_dans_la_reponse_du_registre() {
        let sortie = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\...\\AcroRd32.exe\r\n    (par défaut)    REG_SZ    \"C:\\Program Files\\Adobe\\AcroRd32.exe\"\r\n";
        assert_eq!(super::lire_valeur_reg(sortie).as_deref(), Some("C:\\Program Files\\Adobe\\AcroRd32.exe"));
        assert_eq!(super::lire_valeur_reg("ERREUR : clé introuvable"), None);
    }
}

#[cfg(not(windows))]
pub fn shell_open(path: &Path, _verb: &str) -> Result<(), String> {
    // Environnement non-Windows (utilisé seulement pour `cargo check` en CI/dev) :
    // l'application cible exclusivement Windows en production, voir shell_open ci-dessus.
    Err(format!(
        "shell_open non supporté hors Windows (chemin: {})",
        path.display()
    ))
}

#[cfg(not(windows))]
pub fn shell_print_vers(path: &Path, _imprimante: &str) -> Result<(), String> {
    Err(format!(
        "shell_print_vers non supporté hors Windows (chemin: {})",
        path.display()
    ))
}
