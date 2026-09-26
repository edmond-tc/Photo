//! Ce que font les machines de la boutique, à chaque instant.
//!
//! Deux sources, parce qu'aucune ne voit tout :
//! - le journal des impressions de Windows (voir `controle_impressions.rs`)
//!   voit TOUT ce qui part du PC — Word, PDF, l'application — vers
//!   n'importe quelle imprimante, en USB comme en réseau ;
//! - le compteur de pages des machines branchées en RÉSEAU (voir
//!   `snmp.rs`) voit aussi ce qui ne passe jamais par le PC : les
//!   photocopies faites sur la vitre.
//!
//! Quand le compteur d'une machine avance de plus que ce que le PC lui a
//! envoyé, la différence a été faite directement sur la machine.
//!
//! Une machine branchée en USB n'a pas de compteur lisible par Windows :
//! ses photocopies restent invisibles, et l'écran le dit.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Mutex;

use rusqlite::params;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::db::DbState;

/// Toutes les 15 secondes : assez pour suivre la boutique « en direct »,
/// assez peu pour ne jamais ralentir le PC.
const INTERVALLE: std::time::Duration = std::time::Duration::from_secs(15);

const CLE_DERNIER_ENREGISTREMENT: &str = "activite_dernier_enregistrement";

#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct EtatMachine {
    pub nom: String,
    /// « réseau », « USB » ou « autre ».
    pub branchement: String,
    pub adresse: Option<String>,
    /// Compteur total de la machine, quand elle le donne.
    pub compteur: Option<u64>,
}

#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct Evenement {
    pub heure: String,
    pub machine: String,
    /// `impression_pc` ou `sur_la_machine`.
    pub genre: String,
    pub document: Option<String>,
    pub pages: i64,
}

#[derive(serde::Serialize)]
pub struct ActiviteJour {
    pub journal_actif: bool,
    pub machines: Vec<EtatMachine>,
    pub evenements: Vec<Evenement>,
    pub pages_depuis_pc: i64,
    pub pages_sur_machines: i64,
}

static MACHINES: Mutex<Vec<EtatMachine>> = Mutex::new(Vec::new());
static JOURNAL_ACTIF: Mutex<bool> = Mutex::new(false);

#[derive(Debug, PartialEq)]
struct MachineWindows {
    nom: String,
    adresse: Option<Ipv4Addr>,
    port: String,
}

#[derive(Debug, PartialEq)]
struct ImpressionPc {
    enregistrement: i64,
    heure: String,
    document: String,
    machine: String,
    pages: i64,
}

#[derive(Debug, PartialEq, Default)]
struct Lecture {
    machines: Vec<MachineWindows>,
    journal_actif: bool,
    /// Point de départ, donné une seule fois au tout premier passage.
    dernier: Option<i64>,
    impressions: Vec<ImpressionPc>,
}

const JOURNAL: &str = "Microsoft-Windows-PrintService/Operational";

/// `dernier` vaut -1 au tout premier passage : on part alors de
/// MAINTENANT, plutôt que de rejouer tout l'historique comme « nouveau ».
fn script(dernier: i64) -> String {
    format!(
        "$ErrorActionPreference = 'SilentlyContinue'\n\
         [Console]::OutputEncoding = [System.Text.Encoding]::UTF8\n\
         foreach ($p in Get-Printer) {{\n\
             $port = Get-PrinterPort -Name $p.PortName\n\
             \"MACHINE`t$($p.Name)`t$($port.PrinterHostAddress)`t$($p.PortName)\"\n\
         }}\n\
         $j = Get-WinEvent -ListLog '{JOURNAL}'\n\
         \"ACTIF`t$($j.IsEnabled)\"\n\
         $dernier = {dernier}\n\
         if ($dernier -lt 0) {{\n\
             $e = Get-WinEvent -LogName '{JOURNAL}' -MaxEvents 1\n\
             if ($e) {{ \"DERNIER`t$($e.RecordId)\" }} else {{ \"DERNIER`t0\" }}\n\
         }} else {{\n\
             Get-WinEvent -FilterHashtable @{{LogName='{JOURNAL}'; Id=307; StartTime=(Get-Date).AddDays(-2)}} |\n\
             Where-Object {{ $_.RecordId -gt $dernier }} | Sort-Object RecordId |\n\
             ForEach-Object {{\n\
                 $d = ([xml]$_.ToXml()).Event.UserData.DocumentPrinted\n\
                 $nom = \"$($d.Param2)\" -replace \"`t|`r|`n\", ' '\n\
                 \"IMPRESSION`t$($_.RecordId)`t$($_.TimeCreated.ToString('yyyy-MM-ddTHH:mm:ss'))`t$nom`t$($d.Param5)`t$($d.Param8)\"\n\
             }}\n\
         }}\n"
    )
}

fn lire_sortie(sortie: &str) -> Lecture {
    let mut lecture = Lecture::default();
    for ligne in sortie.lines() {
        let champs: Vec<&str> = ligne.trim_end_matches('\r').split('\t').collect();
        match champs.as_slice() {
            ["MACHINE", nom, adresse, port] => lecture.machines.push(MachineWindows {
                nom: nom.to_string(),
                adresse: adresse.trim().parse().ok(),
                port: port.to_string(),
            }),
            ["ACTIF", valeur] => lecture.journal_actif = valeur.trim().eq_ignore_ascii_case("true"),
            ["DERNIER", valeur] => lecture.dernier = valeur.trim().parse().ok(),
            ["IMPRESSION", enregistrement, heure, document, machine, pages] => {
                if let Ok(enregistrement) = enregistrement.trim().parse() {
                    lecture.impressions.push(ImpressionPc {
                        enregistrement,
                        heure: heure.to_string(),
                        document: document.to_string(),
                        machine: machine.to_string(),
                        pages: pages.trim().parse().unwrap_or(0),
                    });
                }
            }
            _ => {}
        }
    }
    lecture
}

fn branchement(machine: &MachineWindows) -> &'static str {
    let port = machine.port.to_uppercase();
    if machine.adresse.is_some() || port.starts_with("WSD") || port.starts_with("IP_") {
        "réseau"
    } else if port.starts_with("USB") || port.starts_with("DOT4") {
        "USB"
    } else {
        "autre"
    }
}

/// Pages faites DIRECTEMENT sur la machine depuis la lecture précédente.
///
/// `credit` : pages envoyées par le PC à cette machine et pas encore
/// retrouvées dans son compteur. Elles sont retirées de l'avance du
/// compteur ; ce qui reste a été fait sur la machine elle-même. Un compteur
/// qui recule (machine remplacée, remise à zéro) ne compte rien.
fn pages_sur_la_machine(precedent: Option<i64>, actuel: i64, credit: &mut i64) -> i64 {
    let Some(precedent) = precedent else { return 0 };
    if actuel < precedent {
        *credit = 0;
        return 0;
    }
    let avance = actuel - precedent;
    let depuis_pc = (*credit).min(avance);
    *credit -= depuis_pc;
    avance - depuis_pc
}

fn maintenant() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// Un passage de surveillance. Rend les nouveaux événements.
fn un_passage(app: &AppHandle, dernier: &mut i64, credit: &mut HashMap<String, i64>) -> Vec<Evenement> {
    let Ok(sortie) = crate::telephone_usb::executer_powershell(&script(*dernier)) else {
        return Vec::new();
    };
    let lecture = lire_sortie(&sortie);
    if let Ok(mut actif) = JOURNAL_ACTIF.lock() {
        *actif = lecture.journal_actif;
    }
    if let Some(depart) = lecture.dernier {
        *dernier = depart;
    }

    let mut evenements = Vec::new();
    for impression in lecture.impressions {
        *dernier = (*dernier).max(impression.enregistrement);
        if crate::controle_impressions::imprimante_virtuelle(&impression.machine) {
            continue;
        }
        *credit.entry(impression.machine.clone()).or_default() += impression.pages;
        evenements.push(Evenement {
            heure: impression.heure,
            machine: impression.machine,
            genre: "impression_pc".to_string(),
            document: Some(impression.document),
            pages: impression.pages,
        });
    }

    let state = app.state::<DbState>();
    let mut etats = Vec::new();
    for machine in lecture.machines {
        if crate::controle_impressions::imprimante_virtuelle(&machine.nom)
            || crate::controle_impressions::imprimante_virtuelle(&machine.port)
        {
            continue;
        }
        let compteur = machine.adresse.and_then(crate::snmp::compteur_pages);
        if let (Some(valeur), Ok(conn)) = (compteur, state.0.lock()) {
            let precedent: Option<(i64, String)> = conn
                .query_row(
                    "SELECT valeur, lu_le FROM compteurs_machines WHERE machine = ?1",
                    params![machine.nom],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .ok();
            let credit_machine = credit.entry(machine.nom.clone()).or_default();
            let pages = pages_sur_la_machine(precedent.as_ref().map(|p| p.0), valeur as i64, credit_machine);
            if pages > 0 {
                // Lecture précédente vieille de plus de 2 minutes :
                // l'application était fermée pendant ce temps.
                let pendant_fermeture = precedent
                    .as_ref()
                    .and_then(|p| chrono::NaiveDateTime::parse_from_str(&p.1, "%Y-%m-%dT%H:%M:%S").ok())
                    .is_some_and(|lu| chrono::Local::now().naive_local() - lu > chrono::Duration::minutes(2));
                evenements.push(Evenement {
                    heure: maintenant(),
                    machine: machine.nom.clone(),
                    genre: "sur_la_machine".to_string(),
                    document: Some(
                        if pendant_fermeture {
                            "Fait sur la machine pendant que l'application était fermée"
                        } else {
                            "Photocopies ou impressions faites sur la machine"
                        }
                        .to_string(),
                    ),
                    pages,
                });
            }
            let _ = conn.execute(
                "INSERT INTO compteurs_machines (machine, valeur, lu_le) VALUES (?1, ?2, ?3)
                 ON CONFLICT(machine) DO UPDATE SET valeur = excluded.valeur, lu_le = excluded.lu_le",
                params![machine.nom, valeur as i64, maintenant()],
            );
        }
        etats.push(EtatMachine {
            branchement: branchement(&machine).to_string(),
            adresse: machine.adresse.map(|a| a.to_string()),
            nom: machine.nom,
            compteur,
        });
    }
    if let Ok(mut machines) = MACHINES.lock() {
        *machines = etats;
    }

    if let Ok(conn) = state.0.lock() {
        for e in &evenements {
            let _ = conn.execute(
                "INSERT INTO activite (heure, machine, genre, document, pages) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![e.heure, e.machine, e.genre, e.document, e.pages],
            );
        }
        let _ = crate::db::set_setting(&conn, CLE_DERNIER_ENREGISTREMENT, &dernier.to_string());
    }
    evenements
}

/// Lance la surveillance en arrière-plan, pour toute la durée de
/// l'application.
pub fn surveiller(app: AppHandle) {
    if !cfg!(windows) {
        return;
    }
    std::thread::spawn(move || {
        let mut dernier: i64 = {
            let state = app.state::<DbState>();
            let conn = state.0.lock();
            conn.ok()
                .and_then(|c| crate::db::get_setting(&c, CLE_DERNIER_ENREGISTREMENT))
                .and_then(|v| v.parse().ok())
                .unwrap_or(-1)
        };
        let mut credit = HashMap::new();
        loop {
            let nouveaux = un_passage(&app, &mut dernier, &mut credit);
            if !nouveaux.is_empty() {
                let _ = app.emit("activite-nouvelle", nouveaux);
            }
            std::thread::sleep(INTERVALLE);
        }
    });
}

/// L'activité d'aujourd'hui, la plus récente en haut.
#[tauri::command]
pub fn activite_du_jour(state: State<DbState>) -> Result<ActiviteJour, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let motif = format!("{}%", chrono::Local::now().format("%Y-%m-%d"));
    let mut requete = conn
        .prepare(
            "SELECT heure, machine, genre, document, pages FROM activite
             WHERE heure LIKE ?1 ORDER BY heure DESC, id DESC LIMIT 300",
        )
        .map_err(|e| e.to_string())?;
    let evenements: Vec<Evenement> = requete
        .query_map(params![motif], |r| {
            Ok(Evenement {
                heure: r.get(0)?,
                machine: r.get(1)?,
                genre: r.get(2)?,
                document: r.get(3)?,
                pages: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    let total = |genre: &str| -> i64 {
        conn.query_row(
            "SELECT COALESCE(SUM(pages), 0) FROM activite WHERE heure LIKE ?1 AND genre = ?2",
            params![motif, genre],
            |r| r.get(0),
        )
        .unwrap_or(0)
    };
    Ok(ActiviteJour {
        journal_actif: JOURNAL_ACTIF.lock().map(|a| *a).unwrap_or(false),
        machines: MACHINES.lock().map(|m| m.clone()).unwrap_or_default(),
        pages_depuis_pc: total("impression_pc"),
        pages_sur_machines: total("sur_la_machine"),
        evenements,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lit_machines_et_impressions() {
        let lecture = lire_sortie(
            "MACHINE\tCanon iR2520\t192.168.1.50\tIP_192.168.1.50\r\n\
             MACHINE\tHP LaserJet P1102\t\tUSB001\r\n\
             ACTIF\tTrue\r\n\
             IMPRESSION\t812\t2026-09-26T14:05:10\tMémoire Koffi.docx\tHP LaserJet P1102\t30\r\n",
        );
        assert!(lecture.journal_actif);
        assert_eq!(lecture.machines.len(), 2);
        assert_eq!(branchement(&lecture.machines[0]), "réseau");
        assert_eq!(branchement(&lecture.machines[1]), "USB");
        assert_eq!(lecture.impressions[0].pages, 30);
        assert_eq!(lecture.impressions[0].enregistrement, 812);
    }

    #[test]
    fn les_photocopies_sont_ce_que_le_pc_n_a_pas_envoye() {
        let mut credit = 0;
        // Première lecture : rien à comparer.
        assert_eq!(pages_sur_la_machine(None, 10_000, &mut credit), 0);
        // 12 photocopies sur la vitre.
        assert_eq!(pages_sur_la_machine(Some(10_000), 10_012, &mut credit), 12);
        // Le PC envoie 30 pages ; le compteur avance de 30 : rien de plus.
        credit += 30;
        assert_eq!(pages_sur_la_machine(Some(10_012), 10_042, &mut credit), 0);
        assert_eq!(credit, 0);
        // Le PC envoie 10 pages, 5 sortent avant la lecture, puis 5 plus 8 copies.
        credit += 10;
        assert_eq!(pages_sur_la_machine(Some(10_042), 10_047, &mut credit), 0);
        assert_eq!(pages_sur_la_machine(Some(10_047), 10_060, &mut credit), 8);
    }

    #[test]
    fn un_compteur_qui_recule_ne_compte_rien() {
        let mut credit = 4;
        assert_eq!(pages_sur_la_machine(Some(50_000), 12, &mut credit), 0);
        assert_eq!(credit, 0);
    }

    #[test]
    fn premier_passage_part_de_maintenant() {
        assert!(script(-1).contains("-MaxEvents 1"));
        assert!(script(812).contains("$_.RecordId -gt $dernier"));
        assert_eq!(lire_sortie("DERNIER\t812\n").dernier, Some(812));
    }
}
