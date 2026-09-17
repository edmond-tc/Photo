//! Confirmation réelle de l'impression, en observant le spouleur Windows —
//! jusqu'ici, cliquer "Imprimer" ne faisait que transmettre l'ordre à
//! Windows (voir `files::shell_open`), sans jamais savoir si l'impression
//! avait vraiment réussi. Ce module regarde la file d'impression du système
//! (la même que celle qu'affiche l'icône imprimante de Windows) pour
//! retrouver la tâche correspondante et suivre son état jusqu'au bout.
//!
//! Volontairement séparé en deux parties : la logique de correspondance et
//! d'interprétation ci-dessous est du Rust ordinaire, testable sur
//! n'importe quelle machine (voir les tests en bas du fichier) ; l'appel
//! réel au spouleur Windows, à la fin du fichier, ne peut être vérifié que
//! sur une vraie machine Windows — d'où le soin à isoler l'un de l'autre.

#[cfg(windows)]
use crate::db::DbState;

/// Résultat, une fois qu'on a fini de suivre une tâche d'impression (ou
/// renoncé à la retrouver).
#[derive(Debug, Clone, PartialEq)]
pub enum EtatFinal {
    /// Le spouleur confirme que la tâche est sortie ; `pages` est le
    /// nombre de pages réellement imprimées (ou le total prévu, si le
    /// pilote de l'imprimante ne remonte pas le nombre imprimé en détail).
    Terminee { pages: u32 },
    /// Le spouleur signale un problème (bourrage, imprimante hors ligne,
    /// tâche annulée par le gérant lui-même…).
    Erreur(String),
    /// Aucune tâche correspondante trouvée dans le délai imparti — ce n'est
    /// pas forcément un échec : certains lecteurs PDF n'utilisent pas le
    /// spouleur Windows de façon standard, ou l'impression a été trop
    /// rapide pour être observée. On ne bloque jamais le gérant là-dessus.
    Introuvable,
}

/// Une tâche du spouleur telle qu'on la lit depuis `JOB_INFO_2W` (voir la
/// fin du fichier) — dupliquée ici en type Rust ordinaire pour rester
/// testable sans dépendre de la structure Windows brute.
#[derive(Debug, Clone)]
pub struct TacheSpouleur {
    pub id: u32,
    pub nom_document: String,
    /// Combinaison de drapeaux `JOB_STATUS_*` telle que renvoyée par Windows.
    pub statut: u32,
    pub pages_totales: u32,
    pub pages_imprimees: u32,
}

// Drapeaux JOB_STATUS_* (winspool.h) — dupliqués ici en constantes plutôt
// que dépendre du crate `windows` (non disponible hors compilation Windows),
// pour que cette logique reste testable sur n'importe quelle machine.
const JOB_STATUS_ERROR: u32 = 0x0002;
const JOB_STATUS_DELETING: u32 = 0x0004;
const JOB_STATUS_PRINTED: u32 = 0x0080;
const JOB_STATUS_DELETED: u32 = 0x0100;
const JOB_STATUS_COMPLETE: u32 = 0x1000;
const JOB_STATUS_PAPEROUT: u32 = 0x0040;
const JOB_STATUS_OFFLINE: u32 = 0x0020;
const JOB_STATUS_BLOCKED_DEVQ: u32 = 0x0200;

/// Une tâche du spouleur correspond-elle au fichier qu'on vient d'envoyer à
/// imprimer ? Le nom affiché par le spouleur varie selon le logiciel qui a
/// reçu l'ordre "print" (Adobe, la visionneuse Windows, un navigateur…) :
/// certains reprennent le nom du fichier tel quel, d'autres l'entourent de
/// guillemets ou y ajoutent le nom de l'application — on compare donc sans
/// tenir compte de la casse ni de l'extension, en cherchant l'un dans
/// l'autre plutôt qu'une égalité stricte.
pub fn tache_correspond(nom_document_spouleur: &str, nom_fichier_original: &str) -> bool {
    let nettoyer = |s: &str| -> String {
        let sans_extension = s.rsplit_once('.').map(|(base, _)| base).unwrap_or(s);
        sans_extension
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_lowercase()
    };

    let a = nettoyer(nom_document_spouleur);
    let b = nettoyer(nom_fichier_original);

    if a.is_empty() || b.is_empty() {
        return false;
    }
    a.contains(&b) || b.contains(&a)
}

/// Traduit l'état brut d'une tâche du spouleur en résultat compréhensible.
/// Renvoie `None` tant que la tâche est encore en cours — c'est à
/// l'appelant de continuer à interroger le spouleur dans ce cas.
pub fn interpreter(tache: &TacheSpouleur) -> Option<EtatFinal> {
    if tache.statut & JOB_STATUS_ERROR != 0 {
        return Some(EtatFinal::Erreur("Erreur d'impression signalée par Windows.".to_string()));
    }
    if tache.statut & JOB_STATUS_PAPEROUT != 0 {
        return Some(EtatFinal::Erreur("Imprimante à court de papier.".to_string()));
    }
    if tache.statut & JOB_STATUS_OFFLINE != 0 {
        return Some(EtatFinal::Erreur("Imprimante hors ligne.".to_string()));
    }
    if tache.statut & JOB_STATUS_BLOCKED_DEVQ != 0 {
        return Some(EtatFinal::Erreur("Impression bloquée (vérifier l'imprimante).".to_string()));
    }
    if tache.statut & (JOB_STATUS_DELETING | JOB_STATUS_DELETED) != 0 {
        return Some(EtatFinal::Erreur("Impression annulée.".to_string()));
    }
    if tache.statut & (JOB_STATUS_PRINTED | JOB_STATUS_COMPLETE) != 0 {
        // Certains pilotes ne remplissent jamais pages_imprimees : le total
        // prévu reste alors la meilleure estimation disponible.
        let pages = if tache.pages_imprimees > 0 {
            tache.pages_imprimees
        } else {
            tache.pages_totales
        };
        return Some(EtatFinal::Terminee { pages });
    }
    None
}

/// Enregistre le résultat en base, une fois la surveillance terminée
/// (succès, erreur, ou abandon après le délai). Séparé de l'appel Windows
/// pour rester testable avec une vraie connexion SQLite, sans dépendre du
/// spouleur.
pub fn enregistrer_resultat(
    conn: &rusqlite::Connection,
    id_queue: i64,
    etat: &EtatFinal,
) -> rusqlite::Result<()> {
    match etat {
        EtatFinal::Terminee { pages } => conn.execute(
            "UPDATE files_queue SET impression_confirmee = 1, pages_imprimees = ?1, impression_erreur = NULL WHERE id = ?2",
            rusqlite::params![pages, id_queue],
        ),
        EtatFinal::Erreur(message) => conn.execute(
            "UPDATE files_queue SET impression_confirmee = 0, impression_erreur = ?1 WHERE id = ?2",
            rusqlite::params![message, id_queue],
        ),
        EtatFinal::Introuvable => {
            // Rien à corriger : ce n'est pas un échec, juste une absence
            // d'information. On ne veut pas alarmer le gérant pour ça.
            Ok(0)
        }
    }?;
    Ok(())
}

// ─────────────────────── Partie spécifique à Windows ───────────────────────
// Non testable ici (pas de machine Windows dans cet environnement) —
// s'appuie entièrement sur la logique ci-dessus, déjà vérifiée. Toute
// modification de cette section doit être confirmée par le test de fumée
// GitHub Actions (build-windows.yml) puis par un usage réel sur le terrain.
#[cfg(windows)]
mod windows_impl {
    use super::*;
    use std::time::{Duration, Instant};
    use windows::core::PWSTR;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Graphics::Printing::{
        ClosePrinter, EnumJobsW, GetDefaultPrinterW, GetJobW, OpenPrinterW, JOB_INFO_2W,
    };

    /// Nom de l'imprimante actuellement définie par défaut sur ce PC.
    fn imprimante_par_defaut() -> Option<String> {
        let mut taille: u32 = 0;
        // Premier appel : Windows renvoie la taille de tampon nécessaire
        // sans rien écrire (comportement documenté de cette API).
        unsafe {
            let _ = GetDefaultPrinterW(PWSTR::null(), &mut taille);
        }
        if taille == 0 {
            return None;
        }
        let mut tampon: Vec<u16> = vec![0; taille as usize];
        let ok = unsafe {
            GetDefaultPrinterW(PWSTR(tampon.as_mut_ptr()), &mut taille).as_bool()
        };
        if !ok {
            return None;
        }
        let fin = tampon.iter().position(|&c| c == 0).unwrap_or(tampon.len());
        Some(String::from_utf16_lossy(&tampon[..fin]))
    }

    /// Convertit les octets bruts renvoyés par `EnumJobsW`/`GetJobW`
    /// (niveau 2) en `TacheSpouleur`, en lisant les chaînes pointées par la
    /// structure — elles restent valides tant que le tampon d'origine
    /// n'est pas libéré, donc converties immédiatement.
    unsafe fn lire_job_info_2(info: &JOB_INFO_2W) -> TacheSpouleur {
        let nom_document = if info.pDocument.is_null() {
            String::new()
        } else {
            info.pDocument.to_string().unwrap_or_default()
        };
        TacheSpouleur {
            id: info.JobId,
            nom_document,
            statut: info.Status,
            pages_totales: info.TotalPages,
            pages_imprimees: info.PagesPrinted,
        }
    }

    fn lister_taches(hprinter: HANDLE) -> Vec<TacheSpouleur> {
        let mut octets_necessaires: u32 = 0;
        let mut nb_taches: u32 = 0;
        // Premier appel à vide pour connaître la taille réelle du tampon —
        // le nombre de tâches en file n'est jamais connu à l'avance.
        unsafe {
            let _ = EnumJobsW(hprinter, 0, 500, 2, None, &mut octets_necessaires, &mut nb_taches);
        }
        if octets_necessaires == 0 {
            return Vec::new();
        }
        let mut tampon: Vec<u8> = vec![0; octets_necessaires as usize];
        let mut renvoyees: u32 = 0;
        let ok = unsafe {
            EnumJobsW(hprinter, 0, 500, 2, Some(&mut tampon), &mut octets_necessaires, &mut renvoyees)
        };
        if ok.is_err() {
            return Vec::new();
        }
        let ptr = tampon.as_ptr() as *const JOB_INFO_2W;
        (0..renvoyees as usize)
            .map(|i| unsafe { lire_job_info_2(&*ptr.add(i)) })
            .collect()
    }

    fn tache_par_id(hprinter: HANDLE, job_id: u32) -> Option<TacheSpouleur> {
        let mut octets_necessaires: u32 = 0;
        unsafe {
            let _ = GetJobW(hprinter, job_id, 2, None, &mut octets_necessaires);
        }
        if octets_necessaires == 0 {
            return None;
        }
        let mut tampon: Vec<u8> = vec![0; octets_necessaires as usize];
        let ok = unsafe {
            GetJobW(hprinter, job_id, 2, Some(&mut tampon), &mut octets_necessaires).as_bool()
        };
        if !ok {
            return None;
        }
        let info = tampon.as_ptr() as *const JOB_INFO_2W;
        Some(unsafe { lire_job_info_2(&*info) })
    }

    /// Cherche puis suit une tâche d'impression jusqu'à sa conclusion (ou
    /// jusqu'au délai maximal), en interrogeant le spouleur à intervalles
    /// réguliers. Tourne dans son propre thread — n'a jamais à bloquer
    /// l'application le temps qu'un document sorte de l'imprimante.
    pub fn surveiller(nom_fichier_original: &str, delai_max: Duration) -> EtatFinal {
        let Some(nom_imprimante) = imprimante_par_defaut() else {
            return EtatFinal::Introuvable;
        };
        let mut nom_imprimante_wide: Vec<u16> = nom_imprimante.encode_utf16().chain(std::iter::once(0)).collect();
        let mut hprinter = HANDLE(std::ptr::null_mut());
        let ouvert = unsafe {
            OpenPrinterW(PWSTR(nom_imprimante_wide.as_mut_ptr()), &mut hprinter, None)
        };
        if ouvert.is_err() {
            return EtatFinal::Introuvable;
        }

        let debut = Instant::now();
        let mut job_id_suivi: Option<u32> = None;

        let resultat = loop {
            if debut.elapsed() > delai_max {
                break EtatFinal::Introuvable;
            }

            let tache = match job_id_suivi {
                Some(id) => tache_par_id(hprinter, id),
                None => {
                    let trouvee = lister_taches(hprinter)
                        .into_iter()
                        .find(|t| tache_correspond(&t.nom_document, nom_fichier_original));
                    if let Some(t) = &trouvee {
                        job_id_suivi = Some(t.id);
                    }
                    trouvee
                }
            };

            if let Some(t) = tache {
                if let Some(etat) = interpreter(&t) {
                    break etat;
                }
            }

            std::thread::sleep(Duration::from_millis(800));
        };

        unsafe {
            let _ = ClosePrinter(hprinter);
        }
        resultat
    }
}

/// Lance la surveillance en arrière-plan et enregistre le résultat en base
/// une fois connu — appelée juste après avoir transmis l'ordre d'impression
/// à Windows (voir `commands::print_file`). N'affecte jamais la rapidité du
/// clic "Imprimer" lui-même : tout se passe dans un thread séparé.
#[cfg(windows)]
pub fn confirmer_en_arriere_plan(app: tauri::AppHandle, id_queue: i64, nom_fichier_original: String) {
    use tauri::{Emitter, Manager};
    std::thread::spawn(move || {
        let etat = windows_impl::surveiller(&nom_fichier_original, std::time::Duration::from_secs(45));

        let state = app.state::<DbState>();
        if let Ok(conn) = state.0.lock() {
            let _ = enregistrer_resultat(&conn, id_queue, &etat);
        }

        let _ = app.emit(
            "impression-confirmee",
            serde_json::json!({
                "id": id_queue,
                "confirmee": matches!(etat, EtatFinal::Terminee { .. }),
                "pages_imprimees": match &etat {
                    EtatFinal::Terminee { pages } => Some(*pages),
                    _ => None,
                },
                "erreur": match &etat {
                    EtatFinal::Erreur(m) => Some(m.clone()),
                    _ => None,
                },
            }),
        );
    });
}

#[cfg(not(windows))]
pub fn confirmer_en_arriere_plan(_app: tauri::AppHandle, _id_queue: i64, _nom_fichier_original: String) {
    // Rien à surveiller hors Windows (cargo check en CI/dev uniquement) —
    // voir windows_impl pour l'implémentation réelle.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnait_le_meme_nom_de_fichier() {
        assert!(tache_correspond("facture-client.pdf", "facture-client.pdf"));
    }

    #[test]
    fn ignore_la_casse_et_l_extension() {
        assert!(tache_correspond("Facture-Client", "facture-client.pdf"));
    }

    #[test]
    fn accepte_un_nom_entoure_par_l_application_d_impression() {
        // Adobe et d'autres lecteurs ajoutent parfois un préfixe/suffixe
        // ("Microsoft Print to PDF - facture-client.pdf", des guillemets…).
        assert!(tache_correspond("Adobe Acrobat - facture-client.pdf", "facture-client.pdf"));
    }

    #[test]
    fn refuse_un_nom_totalement_different() {
        assert!(!tache_correspond("rapport-mensuel.docx", "facture-client.pdf"));
    }

    #[test]
    fn refuse_les_noms_vides() {
        assert!(!tache_correspond("", "facture-client.pdf"));
        assert!(!tache_correspond("facture-client.pdf", ""));
    }

    fn tache(statut: u32, pages_totales: u32, pages_imprimees: u32) -> TacheSpouleur {
        TacheSpouleur { id: 1, nom_document: "test.pdf".to_string(), statut, pages_totales, pages_imprimees }
    }

    #[test]
    fn tache_en_cours_ne_renvoie_rien() {
        assert_eq!(interpreter(&tache(JOB_STATUS_SPOOLING_TEST, 3, 0)), None);
    }
    const JOB_STATUS_SPOOLING_TEST: u32 = 0x0008;

    #[test]
    fn tache_imprimee_renvoie_le_nombre_de_pages_reel() {
        assert_eq!(
            interpreter(&tache(JOB_STATUS_PRINTED, 5, 5)),
            Some(EtatFinal::Terminee { pages: 5 })
        );
    }

    #[test]
    fn tache_imprimee_sans_detail_utilise_le_total_prevu() {
        // Certains pilotes ne renseignent jamais pages_imprimees : il faut
        // quand même donner un chiffre utile plutôt que zéro.
        assert_eq!(
            interpreter(&tache(JOB_STATUS_PRINTED, 5, 0)),
            Some(EtatFinal::Terminee { pages: 5 })
        );
    }

    #[test]
    fn tache_terminee_via_complete_compte_aussi() {
        assert_eq!(
            interpreter(&tache(JOB_STATUS_COMPLETE, 2, 2)),
            Some(EtatFinal::Terminee { pages: 2 })
        );
    }

    #[test]
    fn erreur_imprimante_est_signalee() {
        assert!(matches!(interpreter(&tache(JOB_STATUS_ERROR, 1, 0)), Some(EtatFinal::Erreur(_))));
    }

    #[test]
    fn panne_de_papier_est_signalee_distinctement() {
        match interpreter(&tache(JOB_STATUS_PAPEROUT, 1, 0)) {
            Some(EtatFinal::Erreur(msg)) => assert!(msg.to_lowercase().contains("papier")),
            other => panic!("attendu une erreur de papier, obtenu {other:?}"),
        }
    }

    #[test]
    fn tache_supprimee_est_une_erreur_pas_un_succes() {
        assert!(matches!(interpreter(&tache(JOB_STATUS_DELETED, 3, 1)), Some(EtatFinal::Erreur(_))));
    }

    #[test]
    fn statuts_combines_priorisent_l_erreur_sur_l_impression() {
        // Une tâche peut porter plusieurs drapeaux à la fois (ex: imprimée
        // ET supprimée juste après) — l'échec doit l'emporter, jamais le
        // faux positif "terminée avec succès".
        assert!(matches!(
            interpreter(&tache(JOB_STATUS_PRINTED | JOB_STATUS_ERROR, 3, 3)),
            Some(EtatFinal::Erreur(_))
        ));
    }

    #[test]
    fn enregistre_un_succes_en_base() {
        let (_dossier, conn) = base_de_test();
        inserer_ligne_test(&conn, 1);
        enregistrer_resultat(&conn, 1, &EtatFinal::Terminee { pages: 4 }).unwrap();
        let (confirmee, pages): (i64, Option<i64>) = conn
            .query_row(
                "SELECT impression_confirmee, pages_imprimees FROM files_queue WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(confirmee, 1);
        assert_eq!(pages, Some(4));
    }

    #[test]
    fn enregistre_une_erreur_en_base_sans_marquer_confirmee() {
        let (_dossier, conn) = base_de_test();
        inserer_ligne_test(&conn, 2);
        enregistrer_resultat(&conn, 2, &EtatFinal::Erreur("bourrage".to_string())).unwrap();
        let (confirmee, erreur): (i64, Option<String>) = conn
            .query_row(
                "SELECT impression_confirmee, impression_erreur FROM files_queue WHERE id = 2",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(confirmee, 0);
        assert_eq!(erreur.as_deref(), Some("bourrage"));
    }

    #[test]
    fn introuvable_ne_touche_pas_la_ligne_existante() {
        let (_dossier, conn) = base_de_test();
        inserer_ligne_test(&conn, 3);
        // On ne doit jamais planter, même quand il n'y a rien à enregistrer.
        enregistrer_resultat(&conn, 3, &EtatFinal::Introuvable).unwrap();
    }

    fn base_de_test() -> (tempfile::TempDir, rusqlite::Connection) {
        let dossier = tempfile::tempdir().expect("dossier temporaire");
        let conn = crate::db::open(dossier.path()).expect("ouverture de la base de test");
        (dossier, conn)
    }

    fn inserer_ligne_test(conn: &rusqlite::Connection, id: i64) {
        conn.execute(
            "INSERT INTO files_queue (id, original_name, path, source, kind, status, received_at, jeton)
             VALUES (?1, 'test.pdf', 'C:\\test.pdf', 'usb', 'imprimable', 'en_attente', '2026-01-01T00:00:00+01:00', 'jeton-test')",
            rusqlite::params![id],
        )
        .expect("insertion de test");
    }
}
