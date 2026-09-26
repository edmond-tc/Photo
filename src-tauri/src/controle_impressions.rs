//! Contrôle des impressions : pages réellement imprimées par ce PC, contre
//! pages encaissées dans l'application.
//!
//! Le besoin n°1 d'un patron de boutique : il n'est pas toujours là.
//! L'employé peut imprimer directement depuis Word, encaisser et garder
//! l'argent — aucune trace, puisque l'application ne compte que ce qui
//! passe par sa file. Windows, lui, voit TOUTES les impressions : son
//! journal « PrintService/Operational » note pour chacune le document,
//! l'imprimante et le nombre de pages (événement 307). Ce module le lit,
//! sans internet ni droits administrateur, et le rapproche de la caisse.
//!
//! Ce journal est éteint par défaut dans Windows. Il faut l'allumer une
//! fois, avec les droits administrateur (`commandes_activation`) : c'est
//! greffé sur le script d'activation du Wi-Fi (un seul « Oui » pour tout),
//! et proposé aussi par un bouton dans Rapports.
//!
//! Limites, dites dans l'interface :
//! - les photocopies faites directement sur la machine ne passent pas par
//!   le PC : elles ne sont pas comptées ici ;
//! - selon le pilote, un document imprimé en plusieurs exemplaires peut
//!   n'être compté qu'une fois par Windows.

use rusqlite::params;
use tauri::State;

use crate::db::DbState;

const JOURNAL: &str = "Microsoft-Windows-PrintService/Operational";

/// À insérer dans un script déjà élevé. `/ms` : 32 Mo, des mois
/// d'impressions ; `/rt:false` : les plus anciennes sont remplacées quand
/// il est plein, il ne bloque jamais.
pub fn commandes_activation() -> String {
    format!(
        "$sortie += \"===CONTROLE_IMPRESSIONS===\"\n\
         $sortie += (wevtutil sl {JOURNAL} /e:true /ms:33554432 /rt:false 2>&1 | Out-String)\n"
    )
}

/// Une impression vue par Windows.
#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct ImpressionWindows {
    pub heure: String,
    pub document: String,
    pub imprimante: String,
    pub pages: i64,
    pub utilisateur: String,
    /// Correspond à un document de la file de l'application (ou à un reçu
    /// qu'elle a imprimé) : ce n'est pas une impression « hors caisse ».
    pub par_application: bool,
}

#[derive(serde::Serialize, Debug)]
pub struct ControleImpressions {
    pub date: String,
    /// Le journal de Windows est-il allumé ? Sinon, rien ne peut être compté.
    pub journal_actif: bool,
    pub pages_imprimees: i64,
    pub pages_encaissees: i64,
    /// Pages imprimées au-delà de ce qui a été encaissé (0 si tout va bien).
    pub pages_sans_paiement: i64,
    pub impressions: usize,
    /// Impressions qui ne correspondent à aucun document de l'application :
    /// celles à regarder en premier.
    pub hors_application: Vec<ImpressionWindows>,
}

fn script_lecture(date: &str) -> String {
    format!(
        "$ErrorActionPreference = 'SilentlyContinue'\n\
         [Console]::OutputEncoding = [System.Text.Encoding]::UTF8\n\
         $j = Get-WinEvent -ListLog '{JOURNAL}'\n\
         \"ACTIF`t$($j.IsEnabled)\"\n\
         $debut = [datetime]'{date}'\n\
         Get-WinEvent -FilterHashtable @{{LogName='{JOURNAL}'; Id=307; StartTime=$debut; EndTime=$debut.AddDays(1)}} |\n\
         ForEach-Object {{\n\
             $p = ([xml]$_.ToXml()).Event.UserData.DocumentPrinted\n\
             $nom = \"$($p.Param2)\" -replace \"`t|`r|`n\", ' '\n\
             \"IMPRESSION`t$($_.TimeCreated.ToString('HH:mm'))`t$nom`t$($p.Param5)`t$($p.Param8)`t$($p.Param3)\"\n\
         }}\n"
    )
}

/// Traduit la sortie du script. Séparée pour être testable sans Windows.
fn lire_sortie(sortie: &str) -> (bool, Vec<ImpressionWindows>) {
    let mut actif = false;
    let mut impressions = Vec::new();
    for ligne in sortie.lines() {
        let champs: Vec<&str> = ligne.trim_end_matches('\r').split('\t').collect();
        match champs.as_slice() {
            ["ACTIF", valeur] => actif = valeur.trim().eq_ignore_ascii_case("true"),
            ["IMPRESSION", heure, document, imprimante, pages, utilisateur] => {
                impressions.push(ImpressionWindows {
                    heure: heure.to_string(),
                    document: document.to_string(),
                    imprimante: imprimante.to_string(),
                    pages: pages.trim().parse().unwrap_or(0),
                    utilisateur: utilisateur.to_string(),
                    par_application: false,
                })
            }
            _ => {}
        }
    }
    (actif, impressions)
}

/// Les imprimantes « virtuelles » (PDF, OneNote, XPS, fax) ne consomment
/// ni papier ni encre : les compter créerait de faux écarts.
pub(crate) fn imprimante_virtuelle(nom: &str) -> bool {
    let nom = nom.to_lowercase();
    ["pdf", "onenote", "xps", "fax", "send to", "envoyer vers"]
        .iter()
        .any(|mot| nom.contains(mot))
}

/// Rapproche les impressions de Windows des documents de l'application et
/// fait les comptes. Séparée pour être testable sans Windows.
fn comparer(
    date: &str,
    journal_actif: bool,
    impressions: Vec<ImpressionWindows>,
    noms_application: &[String],
    pages_encaissees: i64,
) -> ControleImpressions {
    let mut impressions: Vec<ImpressionWindows> = impressions
        .into_iter()
        .filter(|i| !imprimante_virtuelle(&i.imprimante))
        .collect();
    for impression in &mut impressions {
        // Les reçus sont imprimés par l'application elle-même.
        let recu = impression.document.to_lowercase().starts_with("recu_");
        impression.par_application = recu
            || noms_application
                .iter()
                .any(|nom| crate::impression::tache_correspond(&impression.document, nom));
    }
    let pages_imprimees: i64 = impressions
        .iter()
        .filter(|i| !i.document.to_lowercase().starts_with("recu_"))
        .map(|i| i.pages)
        .sum();
    let nombre = impressions.len();
    ControleImpressions {
        date: date.to_string(),
        journal_actif,
        pages_imprimees,
        pages_encaissees,
        pages_sans_paiement: (pages_imprimees - pages_encaissees).max(0),
        impressions: nombre,
        hors_application: impressions.into_iter().filter(|i| !i.par_application).collect(),
    }
}

/// Le contrôle d'une journée (aujourd'hui par défaut).
#[tauri::command]
pub async fn controle_impressions(
    state: State<'_, DbState>,
    date: Option<String>,
) -> Result<ControleImpressions, String> {
    let date = date
        .filter(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok())
        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());

    let script = script_lecture(&date);
    let sortie = tauri::async_runtime::spawn_blocking(move || {
        crate::telephone_usb::executer_powershell(&script)
    })
    .await
    .map_err(|e| e.to_string())??;
    let (journal_actif, impressions) = lire_sortie(&sortie);

    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let motif = format!("{date}%");
    // Feuilles facturées (`copies` = pages × exemplaires) des commandes
    // encaissées ce jour-là, payées ou à crédit : dans les deux cas, le
    // travail a été enregistré.
    let pages_encaissees: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(f.copies), 0) FROM transactions t
             JOIN files_queue f ON f.id = t.file_queue_id
             WHERE t.created_at LIKE ?1 AND f.kind = 'imprimable'",
            params![motif],
            |r| r.get(0),
        )
        .unwrap_or(0);
    // Tous les documents passés par l'application ces derniers jours : une
    // commande reçue la veille peut être imprimée aujourd'hui.
    let veille = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d")
        .map(|d| (d - chrono::Duration::days(3)).format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    let mut requete = conn
        .prepare("SELECT original_name FROM files_queue WHERE received_at >= ?1")
        .map_err(|e| e.to_string())?;
    let noms: Vec<String> = requete
        .query_map(params![veille], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();

    Ok(comparer(&date, journal_actif, impressions, &noms, pages_encaissees))
}

/// Allume le journal des impressions de Windows (une seule fois, fenêtre
/// d'autorisation Windows).
#[tauri::command]
pub async fn activer_controle_impressions() -> Result<(), String> {
    #[cfg(not(windows))]
    return Err("Disponible uniquement sur Windows".to_string());
    #[cfg(windows)]
    tauri::async_runtime::spawn_blocking(|| -> Result<(), String> {
        let resultat = std::env::temp_dir().join("photocopie-benin-controle-resultat.txt");
        let script = format!(
            "$sortie = @()\n{}\n$sortie -join \"`n\" | Out-File -FilePath \"{}\" -Encoding utf8\n",
            commandes_activation(),
            resultat.display()
        );
        crate::hotspot::executer_script_eleve(&script).map(|_| ())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn impression(document: &str, imprimante: &str, pages: i64) -> ImpressionWindows {
        ImpressionWindows {
            heure: "10:00".into(),
            document: document.into(),
            imprimante: imprimante.into(),
            pages,
            utilisateur: "Caisse".into(),
            par_application: false,
        }
    }

    #[test]
    fn lit_le_journal_de_windows() {
        let (actif, liste) = lire_sortie(
            "ACTIF\tTrue\r\nIMPRESSION\t09:12\tMicrosoft Word - memoire.docx\tHP LaserJet P2035\t12\tCaisse\r\n",
        );
        assert!(actif);
        assert_eq!(liste, vec![ImpressionWindows {
            heure: "09:12".into(),
            document: "Microsoft Word - memoire.docx".into(),
            imprimante: "HP LaserJet P2035".into(),
            pages: 12,
            utilisateur: "Caisse".into(),
            par_application: false,
        }]);
        assert!(!lire_sortie("ACTIF\tFalse\n").0);
    }

    /// Le cas que le patron veut voir : 30 pages imprimées depuis Word, rien
    /// encaissé pour elles.
    #[test]
    fn une_impression_hors_application_est_signalee() {
        let controle = comparer(
            "2026-09-26",
            true,
            vec![
                impression("cv-awa.pdf", "HP LaserJet", 2),
                impression("Microsoft Word - mémoire Koffi", "HP LaserJet", 30),
                impression("recu_12.html", "HP LaserJet", 1),
                impression("brouillon", "Microsoft Print to PDF", 50),
            ],
            &["cv-awa.pdf".to_string()],
            2,
        );
        assert_eq!(controle.pages_imprimees, 32, "PDF virtuel et reçu exclus");
        assert_eq!(controle.pages_encaissees, 2);
        assert_eq!(controle.pages_sans_paiement, 30);
        assert_eq!(controle.hors_application.len(), 1);
        assert_eq!(controle.hors_application[0].pages, 30);
    }

    #[test]
    fn tout_encaisse_aucun_ecart() {
        let controle = comparer(
            "2026-09-26",
            true,
            vec![impression("cv.pdf", "Canon", 4)],
            &["cv.pdf".to_string()],
            4,
        );
        assert_eq!(controle.pages_sans_paiement, 0);
        assert!(controle.hors_application.is_empty());
    }

    #[test]
    fn l_activation_allume_le_bon_journal_sans_bloquer() {
        let commandes = commandes_activation();
        assert!(commandes.contains("wevtutil sl Microsoft-Windows-PrintService/Operational /e:true"));
        assert!(commandes.contains("/rt:false"));
    }
}
