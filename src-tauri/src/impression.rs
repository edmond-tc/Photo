//! Confirmation réelle de l'impression, en observant le spouleur Windows —
//! jusqu'ici, cliquer "Imprimer" ne faisait que transmettre l'ordre à
//! Windows (voir `files::shell_open`), sans jamais savoir si l'impression
//! avait vraiment réussi. Ce module regarde la file d'impression du système
//! (la même que celle qu'affiche l'icône imprimante de Windows) pour
//! retrouver la tâche correspondante et suivre son état jusqu'au bout — y
//! compris les détails réels (couleur, recto-verso, format, poste,
//! imprimante) pour les comparer à ce qui a été facturé au client.
//!
//! Volontairement séparé en deux parties : la logique de correspondance,
//! d'interprétation et de comparaison ci-dessous est du Rust ordinaire,
//! testable sur n'importe quelle machine (voir les tests en bas du
//! fichier) ; l'appel réel au spouleur Windows, à la fin du fichier, ne
//! peut être vérifié que sur une vraie machine Windows — d'où le soin à
//! isoler l'un de l'autre.

#[cfg(windows)]
use crate::db::DbState;

/// Détails d'une tâche d'impression tels que lus dans la boîte de dialogue
/// Windows (structure `DEVMODEW`) au moment où le document est sorti.
/// Chaque champ est `None` quand le pilote de l'imprimante ne remonte pas
/// l'information (cf. `dmFields`) — on ne devine jamais une valeur non
/// fournie, ça produirait de faux écarts avec la facturation.
///
/// `copies` est à titre strictement informatif (affiché dans les détails
/// techniques) : ce n'est PAS la même chose que la colonne `copies` de
/// `files_queue`, qui est le nombre TOTAL de feuilles facturées (pages du
/// document × nombre de copies demandées), alors que `copies` ici est
/// uniquement le "nombre de copies" tel que réglé dans la boîte de dialogue
/// d'impression. Comparer les deux directement produirait un faux écart à
/// chaque impression — c'est `pages_imprimees`/`pages_totales` (déjà comparé
/// à la colonne `copies` facturée) qui donne le vrai total de feuilles.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DetailsImpression {
    pub copies: Option<u32>,
    pub couleur: Option<bool>,
    pub recto_verso: Option<bool>,
    pub format_papier: Option<String>,
}

// Drapeaux DM_* (wingdi.h) indiquant quels champs de DEVMODEW le pilote a
// réellement renseignés — dupliqués ici en constantes plutôt que dépendre
// du crate `windows` (non disponible hors compilation Windows).
const DM_PAPERSIZE: u32 = 0x0002;
const DM_COPIES: u32 = 0x0100;
const DM_COLOR: u32 = 0x0800;
const DM_DUPLEX: u32 = 0x1000;

// Valeurs DMCOLOR_*/DMDUP_*/DMPAPER_* correspondantes.
const DMCOLOR_COLOR: i16 = 2;
const DMDUP_SIMPLEX: i16 = 1;
const DMPAPER_LETTER: i16 = 1;
const DMPAPER_A3: i16 = 8;
const DMPAPER_A4: i16 = 9;
const DMPAPER_A5: i16 = 11;

/// Valeurs brutes d'un `DEVMODEW`, extraites telles quelles (aucune logique
/// Windows ici) pour que leur interprétation reste testable sans machine
/// Windows. Voir `windows_impl::lire_devmode` pour la lecture réelle.
#[derive(Debug, Clone, Copy, Default)]
pub struct DevmodeBrut {
    pub champs_presents: u32,
    pub copies: i16,
    pub couleur: i16,
    pub recto_verso: i16,
    pub format_papier: i16,
}

/// Traduit les valeurs brutes d'un `DEVMODEW` en détails compréhensibles,
/// en ne gardant que les champs que le pilote a effectivement renseignés.
pub fn interpreter_devmode(brut: &DevmodeBrut) -> DetailsImpression {
    DetailsImpression {
        copies: (brut.champs_presents & DM_COPIES != 0 && brut.copies > 0)
            .then_some(brut.copies as u32),
        couleur: (brut.champs_presents & DM_COLOR != 0).then_some(brut.couleur == DMCOLOR_COLOR),
        recto_verso: (brut.champs_presents & DM_DUPLEX != 0)
            .then_some(brut.recto_verso != DMDUP_SIMPLEX),
        format_papier: (brut.champs_presents & DM_PAPERSIZE != 0)
            .then(|| format_papier_depuis_code(brut.format_papier))
            .flatten(),
    }
}

/// Ne couvre que les formats que la facturation propose déjà (A4/A3/A5) plus
/// le format US Letter (déjà signalé ailleurs, cf. `files::diagnostiquer_pdf`).
/// Un format non reconnu renvoie `None` plutôt qu'un texte technique
/// ("code 42") qui n'aiderait pas le gérant — mieux vaut ne rien afficher
/// qu'une valeur illisible.
fn format_papier_depuis_code(code: i16) -> Option<String> {
    match code {
        c if c == DMPAPER_A4 => Some("A4".to_string()),
        c if c == DMPAPER_A3 => Some("A3".to_string()),
        c if c == DMPAPER_A5 => Some("A5".to_string()),
        c if c == DMPAPER_LETTER => Some("US Letter".to_string()),
        _ => None,
    }
}

/// Résultat, une fois qu'on a fini de suivre une tâche d'impression (ou
/// renoncé à la retrouver).
#[derive(Debug, Clone, PartialEq)]
pub enum EtatFinal {
    /// Le spouleur confirme que la tâche est sortie ; `pages` est le nombre
    /// de feuilles réellement imprimées (ou le total prévu, si le pilote de
    /// l'imprimante ne remonte pas le nombre imprimé en détail).
    Terminee {
        pages: u32,
        details: DetailsImpression,
        poste_utilisateur: Option<String>,
        imprimante: Option<String>,
    },
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
#[derive(Debug, Clone, Default)]
pub struct TacheSpouleur {
    pub id: u32,
    pub nom_document: String,
    /// Combinaison de drapeaux `JOB_STATUS_*` telle que renvoyée par Windows.
    pub statut: u32,
    pub pages_totales: u32,
    pub pages_imprimees: u32,
    pub poste_utilisateur: Option<String>,
    pub imprimante: Option<String>,
    pub details: DetailsImpression,
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
        return Some(EtatFinal::Terminee {
            pages,
            details: tache.details.clone(),
            poste_utilisateur: tache.poste_utilisateur.clone(),
            imprimante: tache.imprimante.clone(),
        });
    }
    None
}

/// Un écart entre ce qui a été facturé au client et ce que l'imprimante a
/// réellement reçu comme ordre — jamais bloquant, jamais caché : juste
/// listé pour que le gérant (et surtout le propriétaire) puisse voir la
/// différence et en discuter avec l'employé si besoin.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Ecart {
    pub champ: String,
    pub facture: String,
    pub imprime: String,
}

/// Ce qui a été facturé au client pour ce document (lu depuis `files_queue`).
#[derive(Debug, Clone, PartialEq)]
pub struct Facturation {
    /// Total de feuilles facturées (pages du document × nombre de copies) —
    /// c'est la colonne `copies` de `files_queue`, comparable au total de
    /// pages réellement imprimées (`EtatFinal::Terminee.pages`), pas au
    /// "nombre de copies" de la boîte de dialogue Windows.
    pub feuilles: i64,
    pub couleur: bool,
    pub recto_verso: bool,
    pub format_papier: String,
}

/// Compare ce qui a été facturé à ce que le spouleur a réellement vu passer.
/// Un champ que le pilote de l'imprimante n'a pas renseigné (`None`) est
/// silencieusement ignoré : mieux vaut ne rien dire que signaler un écart
/// sur une information qu'on n'a jamais eue. Ne bloque jamais rien — cette
/// fonction ne fait qu'observer, jamais refuser une action.
pub fn comparer_a_la_facturation(facture: &Facturation, imprime_pages: u32, imprime: &DetailsImpression) -> Vec<Ecart> {
    let mut ecarts = Vec::new();

    if imprime_pages as i64 != facture.feuilles {
        ecarts.push(Ecart {
            champ: "feuilles".to_string(),
            facture: format!("{} feuille(s) facturée(s)", facture.feuilles),
            imprime: format!("{} feuille(s) réellement imprimée(s)", imprime_pages),
        });
    }
    if let Some(couleur) = imprime.couleur {
        if couleur != facture.couleur {
            ecarts.push(Ecart {
                champ: "couleur".to_string(),
                facture: libelle_couleur(facture.couleur),
                imprime: libelle_couleur(couleur),
            });
        }
    }
    if let Some(recto_verso) = imprime.recto_verso {
        if recto_verso != facture.recto_verso {
            ecarts.push(Ecart {
                champ: "recto_verso".to_string(),
                facture: libelle_recto_verso(facture.recto_verso),
                imprime: libelle_recto_verso(recto_verso),
            });
        }
    }
    if let Some(format) = &imprime.format_papier {
        if format != &facture.format_papier {
            ecarts.push(Ecart {
                champ: "format_papier".to_string(),
                facture: facture.format_papier.clone(),
                imprime: format.clone(),
            });
        }
    }

    ecarts
}

fn libelle_couleur(couleur: bool) -> String {
    if couleur { "Couleur".to_string() } else { "Noir & Blanc".to_string() }
}

fn libelle_recto_verso(recto_verso: bool) -> String {
    if recto_verso { "Recto-verso".to_string() } else { "Recto simple".to_string() }
}

/// Enregistre le résultat en base, une fois la surveillance terminée
/// (succès, erreur, ou abandon après le délai) — y compris, en cas de
/// succès, les détails réels et la comparaison avec ce qui a été facturé.
/// Séparé de l'appel Windows pour rester testable avec une vraie connexion
/// SQLite, sans dépendre du spouleur.
pub fn enregistrer_resultat(
    conn: &rusqlite::Connection,
    id_queue: i64,
    etat: &EtatFinal,
) -> rusqlite::Result<()> {
    match etat {
        EtatFinal::Terminee { pages, details, poste_utilisateur, imprimante } => {
            let facture: Option<(i64, bool, bool, String)> = conn
                .query_row(
                    "SELECT copies, couleur, recto_verso, format_papier FROM files_queue WHERE id = ?1",
                    rusqlite::params![id_queue],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .ok();

            let ecarts_json = facture.map(|(feuilles, couleur, recto_verso, format_papier)| {
                let facturation = Facturation { feuilles, couleur, recto_verso, format_papier };
                let ecarts = comparer_a_la_facturation(&facturation, *pages, details);
                serde_json::to_string(&ecarts).unwrap_or_else(|_| "[]".to_string())
            });

            conn.execute(
                "UPDATE files_queue SET
                    impression_confirmee = 1,
                    pages_imprimees = ?1,
                    impression_erreur = NULL,
                    impression_couleur_reelle = ?2,
                    impression_recto_verso_reelle = ?3,
                    impression_format_reel = ?4,
                    impression_copies_reelles = ?5,
                    impression_poste = ?6,
                    impression_imprimante_reelle = ?7,
                    impression_ecarts = ?8
                 WHERE id = ?9",
                rusqlite::params![
                    pages,
                    details.couleur,
                    details.recto_verso,
                    details.format_papier,
                    details.copies,
                    poste_utilisateur,
                    imprimante,
                    ecarts_json,
                    id_queue,
                ],
            )
        }
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
    use windows::Win32::Graphics::Gdi::DEVMODEW;
    use windows::Win32::Graphics::Printing::{
        ClosePrinter, EnumJobsW, EnumPrintersW, GetDefaultPrinterW, GetJobW, OpenPrinterW,
        JOB_INFO_2W, PRINTER_ENUM_CONNECTIONS, PRINTER_ENUM_LOCAL, PRINTER_INFO_4W,
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

    /// Liste les imprimantes installées sur ce PC (locales et connectées en
    /// réseau) — la même liste que celle visible dans les paramètres
    /// Windows, pour permettre au gérant d'en choisir une autre que celle
    /// par défaut sans quitter l'application.
    pub fn lister_imprimantes() -> Vec<String> {
        let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
        let mut octets_necessaires: u32 = 0;
        let mut nb_imprimantes: u32 = 0;
        unsafe {
            let _ = EnumPrintersW(flags, PWSTR::null(), 4, None, &mut octets_necessaires, &mut nb_imprimantes);
        }
        if octets_necessaires == 0 {
            return Vec::new();
        }
        let mut tampon: Vec<u8> = vec![0; octets_necessaires as usize];
        let mut renvoyees: u32 = 0;
        let ok = unsafe {
            EnumPrintersW(flags, PWSTR::null(), 4, Some(&mut tampon), &mut octets_necessaires, &mut renvoyees)
        };
        if ok.is_err() {
            return Vec::new();
        }
        let ptr = tampon.as_ptr() as *const PRINTER_INFO_4W;
        (0..renvoyees as usize)
            .filter_map(|i| {
                let info = unsafe { &*ptr.add(i) };
                if info.pPrinterName.is_null() {
                    None
                } else {
                    unsafe { info.pPrinterName.to_string().ok() }
                }
            })
            .collect()
    }

    /// Lit les champs `DEVMODEW` réellement renseignés par le pilote, sous
    /// forme de valeurs brutes — la traduction (`interpreter_devmode`) est
    /// du Rust ordinaire testable, voir plus haut dans ce fichier.
    unsafe fn lire_devmode(devmode: *mut DEVMODEW) -> DetailsImpression {
        if devmode.is_null() {
            return DetailsImpression::default();
        }
        let dm = &*devmode;
        let brut = DevmodeBrut {
            champs_presents: dm.dmFields.0,
            copies: dm.Anonymous1.Anonymous1.dmCopies,
            couleur: dm.dmColor.0,
            recto_verso: dm.dmDuplex.0,
            format_papier: dm.Anonymous1.Anonymous1.dmPaperSize,
        };
        interpreter_devmode(&brut)
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
        let poste_utilisateur = if info.pUserName.is_null() {
            None
        } else {
            info.pUserName.to_string().ok()
        };
        let imprimante = if info.pPrinterName.is_null() {
            None
        } else {
            info.pPrinterName.to_string().ok()
        };
        TacheSpouleur {
            id: info.JobId,
            nom_document,
            statut: info.Status,
            pages_totales: info.TotalPages,
            pages_imprimees: info.PagesPrinted,
            poste_utilisateur,
            imprimante,
            details: lire_devmode(info.pDevMode),
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
    ///
    /// `imprimante_cible` : quand le gérant a choisi une imprimante précise
    /// avant d'imprimer (voir la liste déroulante), on surveille CELLE-LÀ —
    /// sinon (`None`), l'imprimante par défaut du PC, comme avant.
    pub fn surveiller(
        nom_fichier_original: &str,
        imprimante_cible: Option<&str>,
        delai_max: Duration,
    ) -> EtatFinal {
        let nom_imprimante = match imprimante_cible {
            Some(nom) => nom.to_string(),
            None => match imprimante_par_defaut() {
                Some(nom) => nom,
                None => return EtatFinal::Introuvable,
            },
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

/// Liste les imprimantes installées sur ce PC, pour la liste déroulante à
/// côté du bouton "Imprimer" (voir `commands::lister_imprimantes`).
#[cfg(windows)]
pub fn imprimantes_disponibles() -> Vec<String> {
    windows_impl::lister_imprimantes()
}

#[cfg(not(windows))]
pub fn imprimantes_disponibles() -> Vec<String> {
    Vec::new()
}

/// Lance la surveillance en arrière-plan et enregistre le résultat en base
/// une fois connu — appelée juste après avoir transmis l'ordre d'impression
/// à Windows (voir `commands::print_file`). N'affecte jamais la rapidité du
/// clic "Imprimer" lui-même : tout se passe dans un thread séparé.
#[cfg(windows)]
pub fn confirmer_en_arriere_plan(
    app: tauri::AppHandle,
    id_queue: i64,
    nom_fichier_original: String,
    imprimante_cible: Option<String>,
) {
    use tauri::{Emitter, Manager};
    std::thread::spawn(move || {
        let etat = windows_impl::surveiller(
            &nom_fichier_original,
            imprimante_cible.as_deref(),
            std::time::Duration::from_secs(45),
        );

        let state = app.state::<DbState>();
        if let Ok(conn) = state.0.lock() {
            let _ = enregistrer_resultat(&conn, id_queue, &etat);
        }

        let (pages, ecarts) = match &etat {
            EtatFinal::Terminee { pages, .. } => {
                let ecarts = state
                    .0
                    .lock()
                    .ok()
                    .and_then(|conn| {
                        conn.query_row(
                            "SELECT impression_ecarts FROM files_queue WHERE id = ?1",
                            rusqlite::params![id_queue],
                            |r| r.get::<_, Option<String>>(0),
                        )
                        .ok()
                        .flatten()
                    })
                    .and_then(|json| serde_json::from_str::<Vec<Ecart>>(&json).ok())
                    .unwrap_or_default();
                (Some(*pages), ecarts)
            }
            _ => (None, Vec::new()),
        };

        let _ = app.emit(
            "impression-confirmee",
            serde_json::json!({
                "id": id_queue,
                "confirmee": matches!(etat, EtatFinal::Terminee { .. }),
                "pages_imprimees": pages,
                "ecarts": ecarts,
                "erreur": match &etat {
                    EtatFinal::Erreur(m) => Some(m.clone()),
                    _ => None,
                },
            }),
        );
    });
}

#[cfg(not(windows))]
pub fn confirmer_en_arriere_plan(
    _app: tauri::AppHandle,
    _id_queue: i64,
    _nom_fichier_original: String,
    _imprimante_cible: Option<String>,
) {
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
        TacheSpouleur {
            id: 1,
            nom_document: "test.pdf".to_string(),
            statut,
            pages_totales,
            pages_imprimees,
            ..Default::default()
        }
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
            Some(EtatFinal::Terminee {
                pages: 5,
                details: DetailsImpression::default(),
                poste_utilisateur: None,
                imprimante: None,
            })
        );
    }

    #[test]
    fn tache_imprimee_sans_detail_utilise_le_total_prevu() {
        // Certains pilotes ne renseignent jamais pages_imprimees : il faut
        // quand même donner un chiffre utile plutôt que zéro.
        assert_eq!(
            interpreter(&tache(JOB_STATUS_PRINTED, 5, 0)),
            Some(EtatFinal::Terminee {
                pages: 5,
                details: DetailsImpression::default(),
                poste_utilisateur: None,
                imprimante: None,
            })
        );
    }

    #[test]
    fn tache_terminee_via_complete_compte_aussi() {
        assert_eq!(
            interpreter(&tache(JOB_STATUS_COMPLETE, 2, 2)),
            Some(EtatFinal::Terminee {
                pages: 2,
                details: DetailsImpression::default(),
                poste_utilisateur: None,
                imprimante: None,
            })
        );
    }

    #[test]
    fn tache_terminee_transporte_les_details_et_le_poste() {
        let mut t = tache(JOB_STATUS_PRINTED, 4, 4);
        t.poste_utilisateur = Some("gerant-pc".to_string());
        t.imprimante = Some("HP LaserJet".to_string());
        t.details = DetailsImpression {
            copies: Some(2),
            couleur: Some(true),
            recto_verso: Some(false),
            format_papier: Some("A4".to_string()),
        };
        match interpreter(&t) {
            Some(EtatFinal::Terminee { poste_utilisateur, imprimante, details, .. }) => {
                assert_eq!(poste_utilisateur.as_deref(), Some("gerant-pc"));
                assert_eq!(imprimante.as_deref(), Some("HP LaserJet"));
                assert_eq!(details.couleur, Some(true));
            }
            other => panic!("attendu Terminee avec détails, obtenu {other:?}"),
        }
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

    // ───────────────────── interpreter_devmode ─────────────────────

    #[test]
    fn devmode_sans_champ_renseigne_ne_donne_aucun_detail() {
        let brut = DevmodeBrut { champs_presents: 0, ..Default::default() };
        assert_eq!(interpreter_devmode(&brut), DetailsImpression::default());
    }

    #[test]
    fn devmode_detecte_la_couleur() {
        let brut = DevmodeBrut { champs_presents: DM_COLOR, couleur: DMCOLOR_COLOR, ..Default::default() };
        assert_eq!(interpreter_devmode(&brut).couleur, Some(true));
    }

    #[test]
    fn devmode_detecte_le_noir_et_blanc() {
        let brut = DevmodeBrut { champs_presents: DM_COLOR, couleur: 1, ..Default::default() };
        assert_eq!(interpreter_devmode(&brut).couleur, Some(false));
    }

    #[test]
    fn devmode_detecte_le_recto_verso() {
        let brut = DevmodeBrut { champs_presents: DM_DUPLEX, recto_verso: 2, ..Default::default() };
        assert_eq!(interpreter_devmode(&brut).recto_verso, Some(true));
    }

    #[test]
    fn devmode_detecte_le_recto_simple() {
        let brut = DevmodeBrut { champs_presents: DM_DUPLEX, recto_verso: DMDUP_SIMPLEX, ..Default::default() };
        assert_eq!(interpreter_devmode(&brut).recto_verso, Some(false));
    }

    #[test]
    fn devmode_reconnait_le_format_a4() {
        let brut = DevmodeBrut { champs_presents: DM_PAPERSIZE, format_papier: DMPAPER_A4, ..Default::default() };
        assert_eq!(interpreter_devmode(&brut).format_papier.as_deref(), Some("A4"));
    }

    #[test]
    fn devmode_format_inconnu_n_affiche_rien_plutot_qu_un_code_illisible() {
        let brut = DevmodeBrut { champs_presents: DM_PAPERSIZE, format_papier: 200, ..Default::default() };
        assert_eq!(interpreter_devmode(&brut).format_papier, None);
    }

    #[test]
    fn devmode_copies_zero_est_ignore_meme_si_le_champ_est_marque_present() {
        // Un pilote qui renseigne le drapeau mais laisse 0 : pas une vraie
        // information, mieux vaut ne rien afficher qu'un "0 copie" absurde.
        let brut = DevmodeBrut { champs_presents: DM_COPIES, copies: 0, ..Default::default() };
        assert_eq!(interpreter_devmode(&brut).copies, None);
    }

    // ───────────────────── comparer_a_la_facturation ─────────────────────

    fn facturation(feuilles: i64, couleur: bool, recto_verso: bool, format: &str) -> Facturation {
        Facturation { feuilles, couleur, recto_verso, format_papier: format.to_string() }
    }

    #[test]
    fn aucun_ecart_quand_tout_correspond() {
        let f = facturation(5, false, false, "A4");
        let d = DetailsImpression {
            copies: Some(1),
            couleur: Some(false),
            recto_verso: Some(false),
            format_papier: Some("A4".to_string()),
        };
        assert!(comparer_a_la_facturation(&f, 5, &d).is_empty());
    }

    #[test]
    fn aucun_ecart_quand_le_pilote_ne_dit_rien() {
        // Rien de connu côté imprimante : on ne signale jamais un écart sur
        // une info qu'on n'a jamais eue.
        let f = facturation(5, true, true, "A3");
        assert!(comparer_a_la_facturation(&f, 5, &DetailsImpression::default()).is_empty());
    }

    #[test]
    fn detecte_un_ecart_de_nombre_de_feuilles() {
        let f = facturation(10, false, false, "A4");
        let ecarts = comparer_a_la_facturation(&f, 6, &DetailsImpression::default());
        assert_eq!(ecarts.len(), 1);
        assert_eq!(ecarts[0].champ, "feuilles");
    }

    #[test]
    fn detecte_un_ecart_de_couleur() {
        let f = facturation(3, false, false, "A4");
        let d = DetailsImpression { couleur: Some(true), ..Default::default() };
        let ecarts = comparer_a_la_facturation(&f, 3, &d);
        assert_eq!(ecarts.len(), 1);
        assert_eq!(ecarts[0].champ, "couleur");
        assert_eq!(ecarts[0].facture, "Noir & Blanc");
        assert_eq!(ecarts[0].imprime, "Couleur");
    }

    #[test]
    fn detecte_un_ecart_de_recto_verso() {
        let f = facturation(3, false, true, "A4");
        let d = DetailsImpression { recto_verso: Some(false), ..Default::default() };
        let ecarts = comparer_a_la_facturation(&f, 3, &d);
        assert_eq!(ecarts.len(), 1);
        assert_eq!(ecarts[0].champ, "recto_verso");
    }

    #[test]
    fn detecte_un_ecart_de_format() {
        let f = facturation(3, false, false, "A4");
        let d = DetailsImpression { format_papier: Some("A3".to_string()), ..Default::default() };
        let ecarts = comparer_a_la_facturation(&f, 3, &d);
        assert_eq!(ecarts.len(), 1);
        assert_eq!(ecarts[0].champ, "format_papier");
    }

    #[test]
    fn cumule_plusieurs_ecarts_a_la_fois() {
        let f = facturation(10, false, false, "A4");
        let d = DetailsImpression {
            copies: None,
            couleur: Some(true),
            recto_verso: Some(true),
            format_papier: Some("A3".to_string()),
        };
        let ecarts = comparer_a_la_facturation(&f, 4, &d);
        assert_eq!(ecarts.len(), 4);
    }

    // ───────────────────── enregistrer_resultat ─────────────────────

    #[test]
    fn enregistre_un_succes_en_base() {
        let (_dossier, conn) = base_de_test();
        inserer_ligne_test(&conn, 1);
        enregistrer_resultat(
            &conn,
            1,
            &EtatFinal::Terminee {
                pages: 4,
                details: DetailsImpression::default(),
                poste_utilisateur: None,
                imprimante: None,
            },
        )
        .unwrap();
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
    fn enregistre_les_details_reels_et_le_poste() {
        let (_dossier, conn) = base_de_test();
        inserer_ligne_test(&conn, 1);
        enregistrer_resultat(
            &conn,
            1,
            &EtatFinal::Terminee {
                pages: 1,
                details: DetailsImpression {
                    copies: Some(1),
                    couleur: Some(true),
                    recto_verso: Some(false),
                    format_papier: Some("A4".to_string()),
                },
                poste_utilisateur: Some("caisse-1".to_string()),
                imprimante: Some("HP LaserJet".to_string()),
            },
        )
        .unwrap();
        let (couleur, poste, imprimante): (Option<i64>, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT impression_couleur_reelle, impression_poste, impression_imprimante_reelle
                 FROM files_queue WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(couleur, Some(1));
        assert_eq!(poste.as_deref(), Some("caisse-1"));
        assert_eq!(imprimante.as_deref(), Some("HP LaserJet"));
    }

    #[test]
    fn enregistre_un_ecart_detecte_avec_la_facturation() {
        let (_dossier, conn) = base_de_test();
        // Facturé : 1 feuille, noir & blanc (valeurs par défaut de la ligne de test).
        inserer_ligne_test(&conn, 1);
        enregistrer_resultat(
            &conn,
            1,
            &EtatFinal::Terminee {
                pages: 1,
                details: DetailsImpression { couleur: Some(true), ..Default::default() },
                poste_utilisateur: None,
                imprimante: None,
            },
        )
        .unwrap();
        let ecarts_json: Option<String> = conn
            .query_row("SELECT impression_ecarts FROM files_queue WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        let ecarts: Vec<Ecart> = serde_json::from_str(&ecarts_json.unwrap()).unwrap();
        assert_eq!(ecarts.len(), 1);
        assert_eq!(ecarts[0].champ, "couleur");
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
