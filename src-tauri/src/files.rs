use std::path::Path;

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
