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

/// Marque la version actuelle comme vue SANS renvoyer la liste des
/// nouveautés — appelé à la fin de l'assistant de premier démarrage, pour
/// qu'un gérant qui vient d'installer ne se voie jamais présenter ce qu'il
/// utilise depuis le premier jour comme une nouveauté fraîche.
#[tauri::command]
pub fn marquer_version_actuelle_vue(state: State<'_, DbState>) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    db::set_setting(&conn, "derniere_version_vue", VERSION_ACTUELLE).map_err(|e| e.to_string())
}

// ───────────────────────────── Nouveautés ─────────────────────────────
// L'appli n'étant jamais connectée à internet, ce contenu doit être livré
// avec le logiciel lui-même (pas de liste récupérée en ligne) : ajouter une
// entrée ici demande une nouvelle version, comme tout le reste.

#[derive(serde::Serialize)]
pub struct Nouveaute {
    pub titre: String,
    pub description: String,
    pub ou_trouver: String,
}

/// (version, titre, description, où la trouver). La description explique
/// ce que ça change concrètement pour le gérant — jamais le "comment" technique.
const NOUVEAUTES: &[(&str, &str, &str, &str)] = &[
    (
        "0.3.0",
        "L'imprimante confirme elle-même",
        "Avant, « Imprimer » envoyait l'ordre sans jamais savoir si le papier était vraiment sorti. Maintenant l'application vous le dit — et vous prévient si l'imprimante bourre, n'a plus de papier ou n'est pas allumée.",
        "Automatique, sous chaque commande",
    ),
    (
        "0.3.0",
        "Écart entre ce qui est facturé et ce qui est imprimé",
        "Si une commande est facturée en noir & blanc mais sortie en couleur, l'écart s'affiche en clair. Rien n'est bloqué : c'est là pour être vu et éclairci.",
        "Automatique, sous chaque commande",
    ),
    (
        "0.3.0",
        "Recto-verso à facturer",
        "Une case en plus dans les détails de facturation : le recto-verso était géré par Windows à l'impression, mais n'était jamais suivi côté prix.",
        "Bouton Détails d'une commande",
    ),
    (
        "0.3.0",
        "Choisir son imprimante sans quitter l'application",
        "Pour les boutiques qui changent d'imprimante en cours de journée : une liste déroulante au-dessus des commandes, plutôt qu'un passage par les réglages de Windows.",
        "En haut de l'écran principal",
    ),
    (
        "0.3.0",
        "Rapport imprimable du jour, de la semaine ou du mois",
        "Les chiffres, et le détail de chaque impression réussie avec son heure et le temps d'attente du client. Sur papier, ou en PDF à garder.",
        "Rapports",
    ),
    (
        "0.3.0",
        "Une aide sur chaque écran",
        "Un bouton « ? » en haut explique à quoi sert chaque bouton de l'écran où vous êtes. Avec une visite guidée à revoir quand vous voulez, et un guide d'une page à imprimer et poser près du PC.",
        "Bouton ? en haut de l'écran",
    ),
    (
        "0.3.0",
        "Votre licence survit à une réinstallation de Windows",
        "Si l'ordinateur doit être reformaté, votre abonnement continue de fonctionner sans qu'on ait à vous refabriquer une clé.",
        "Automatique, rien à faire",
    ),
    (
        "0.2.0",
        "Supprimer un document sur demande",
        "Un client vous demande d'effacer son document ? Un seul bouton suffit, sans mot de passe technique.",
        "Historique",
    ),
    (
        "0.2.0",
        "Conservation automatique des documents",
        "Choisissez après combien de jours les documents de vos clients s'effacent tout seuls du disque.",
        "Réglages → Conservation des documents clients",
    ),
    (
        "0.2.0",
        "Restaurer une sauvegarde",
        "En cas de souci sur l'ordinateur, retrouvez vos données à un moment précis, sans aide extérieure.",
        "Réglages → outils techniques",
    ),
    (
        "0.2.0",
        "Fidélité client réglable",
        "Choisissez vous-même après combien de visites une réduction s'applique, et de combien — plus une valeur imposée.",
        "Réglages → Fidélité client",
    ),
    (
        "0.2.0",
        "Message de fidélité en direct",
        "Le client voit un mot de remerciement et son compteur de fidélité sur son propre téléphone, dès l'encaissement — sans imprimer de reçu.",
        "Automatique, rien à faire",
    ),
    (
        "0.2.0",
        "Bluetooth pour les clients sans Wi-Fi",
        "Vos clients peuvent maintenant envoyer leurs fichiers même sans réseau, directement en Bluetooth.",
        "Réglages → Bluetooth",
    ),
    (
        "0.2.0",
        "Résumés du matin et du soir",
        "Un petit mot d'accueil avec les chiffres d'hier, et un résumé encourageant en fin de journée bien remplie.",
        "Automatique, rien à faire",
    ),
    (
        "0.2.0",
        "Sons et confirmations plus clairs",
        "Un son différent pour chaque action (nouvelle commande, impression, encaissement), et des confirmations qui ne bloquent plus l'écran.",
        "Automatique, rien à faire",
    ),
];

fn parse_version(s: &str) -> Vec<u32> {
    s.split('.').filter_map(|p| p.parse().ok()).collect()
}

/// Renvoie les nouveautés parues entre la dernière version vue par ce
/// gérant et la version installée, puis marque la version actuelle comme
/// vue — pour ne jamais montrer deux fois la même liste. Appelé juste après
/// une activation de licence réussie (voir main.js).
#[tauri::command]
pub fn recuperer_nouveautes_et_marquer_vues(
    state: State<'_, DbState>,
) -> Result<Vec<Nouveaute>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;

    let derniere_vue =
        db::get_setting(&conn, "derniere_version_vue").unwrap_or_else(|| "0.0.0".to_string());
    let derniere_vue = parse_version(&derniere_vue);
    let actuelle = parse_version(VERSION_ACTUELLE);

    let nouveautes = NOUVEAUTES
        .iter()
        .filter(|(v, ..)| {
            let v = parse_version(v);
            v > derniere_vue && v <= actuelle
        })
        .map(|(_, titre, description, ou_trouver)| Nouveaute {
            titre: titre.to_string(),
            description: description.to_string(),
            ou_trouver: ou_trouver.to_string(),
        })
        .collect();

    db::set_setting(&conn, "derniere_version_vue", VERSION_ACTUELLE).map_err(|e| e.to_string())?;
    Ok(nouveautes)
}
