use std::path::Path;

/// Taille au-delà de laquelle on prévient le gérant qu'un fichier est
/// volumineux (section 4 : "indicateur de progression clair").
pub const SEUIL_FICHIER_VOLUMINEUX: u64 = 20 * 1024 * 1024; // 20 Mo

/// Limite de lecture pour les diagnostics PDF ci-dessous : au-delà, on ne
/// scanne pas le fichier (documents de boutique de photocopie rarement
/// aussi gros) plutôt que de ralentir la réception.
const LIMITE_DIAGNOSTIC_PDF: u64 = 25 * 1024 * 1024;

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
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::ShellExecuteW;

    let path_h = HSTRING::from(path.as_os_str());
    let verb_h = HSTRING::from(verb);

    // SAFETY: appel FFI standard vers l'API Shell de Windows, aucune mémoire
    // n'est retenue au-delà de l'appel (les HSTRING restent en vie jusque-là).
    let result = unsafe {
        ShellExecuteW(
            HWND(std::ptr::null_mut()),
            PCWSTR(verb_h.as_ptr()),
            PCWSTR(path_h.as_ptr()),
            PCWSTR::null(),
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

#[cfg(not(windows))]
pub fn shell_open(path: &Path, _verb: &str) -> Result<(), String> {
    // Environnement non-Windows (utilisé seulement pour `cargo check` en CI/dev) :
    // l'application cible exclusivement Windows en production, voir shell_open ci-dessus.
    Err(format!(
        "shell_open non supporté hors Windows (chemin: {})",
        path.display()
    ))
}
