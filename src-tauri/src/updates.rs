use crate::db::{self, DbState};
use tauri::State;

const VERSION_ACTUELLE: &str = env!("CARGO_PKG_VERSION");

#[derive(serde::Deserialize)]
struct ReponseVersion {
    version: String,
}

/// Vérification opportuniste : si une connexion internet est disponible ET
/// qu'une URL de vérification est configurée, compare la version distante à
/// la version locale. N'affiche qu'un badge discret, n'installe jamais rien
/// automatiquement — exactement ce que demande la section 6 du cahier des
/// charges. Échoue silencieusement si pas de connexion.
#[tauri::command]
pub async fn verifier_mise_a_jour(state: State<'_, DbState>) -> Result<Option<String>, String> {
    let url = {
        let conn = state.0.lock().map_err(|e| e.to_string())?;
        db::get_setting(&conn, "url_verification_maj")
    };
    let Some(url) = url.filter(|u| !u.is_empty()) else {
        return Ok(None);
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .build()
        .map_err(|e| e.to_string())?;

    let reponse = match client.get(&url).send().await {
        Ok(r) => r,
        Err(_) => return Ok(None), // pas de connexion : silencieux, pas d'erreur affichée
    };

    let Ok(donnees) = reponse.json::<ReponseVersion>().await else {
        return Ok(None);
    };

    if donnees.version.as_str() != VERSION_ACTUELLE {
        Ok(Some(donnees.version))
    } else {
        Ok(None)
    }
}

#[tauri::command]
pub fn version_actuelle() -> String {
    VERSION_ACTUELLE.to_string()
}
