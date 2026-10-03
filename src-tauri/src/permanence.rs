//! Le logiciel doit tourner en permanence.
//!
//! Le Wi-Fi de la boutique se rallume tout seul au démarrage de Windows
//! (voir `hotspot.rs`), mais le programme qui REÇOIT les documents est
//! celui-ci : s'il est fermé, les clients envoient dans le vide, sans que
//! personne ne le voie. Trois protections :
//!
//! 1. **Il s'ouvre avec Windows** (clé `Run` de l'utilisateur, sans droits
//!    administrateur), réglable par une case dans Réglages.
//! 2. **La croix de la fenêtre le réduit** au lieu de le fermer. On l'arrête
//!    vraiment par « Arrêter le logiciel », seul geste volontaire.
//! 3. **Une tâche de veille**, toutes les 5 minutes, le rouvre s'il a
//!    disparu (plantage). Elle respecte l'arrêt volontaire : tant que le
//!    gérant n'a pas rouvert le logiciel lui-même, elle ne le relance pas.

use crate::db::{self, DbState};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, State};

/// Lancement par Windows à l'ouverture de session.
pub const ARG_SESSION: &str = "--session";
/// Lancement par la tâche de veille.
pub const ARG_VEILLE: &str = "--veille";

const NOM_TACHE_VEILLE: &str = "Photocopie Benin - Veille du logiciel";
#[cfg(windows)]
const NOM_VALEUR_RUN: &str = "GestionPhotocopie";
const REGLAGE_OUVERTURE: &str = "ouvrir_avec_windows";
const REGLAGE_INFO_FERMETURE: &str = "info_fermeture_vue";

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Lancement {
    /// Double-clic du gérant, installeur, mise à jour.
    Normal,
    Session,
    Veille,
}

pub fn lancement(args: &[String]) -> Lancement {
    if args.iter().any(|a| a == ARG_VEILLE) {
        Lancement::Veille
    } else if args.iter().any(|a| a == ARG_SESSION) {
        Lancement::Session
    } else {
        Lancement::Normal
    }
}

fn dossier() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("photocopie-benin")
}

/// Présent quand le gérant a arrêté le logiciel exprès.
pub fn marqueur_arret() -> PathBuf {
    dossier().join("arret-volontaire")
}

/// À appeler tout au début : `true` = ne pas démarrer du tout.
///
/// La veille ne relance rien après un arrêt volontaire. Tout autre
/// lancement (le gérant rouvre le logiciel, Windows l'ouvre au matin)
/// efface l'arrêt volontaire : la veille reprend son travail.
pub fn doit_sortir(l: Lancement) -> bool {
    match l {
        Lancement::Veille => marqueur_arret().exists(),
        Lancement::Normal | Lancement::Session => {
            let _ = std::fs::remove_file(marqueur_arret());
            false
        }
    }
}

/// Les réglages sont appliqués à chaque démarrage : le chemin du programme
/// change quand on le réinstalle ailleurs, la clé doit suivre.
pub fn est_active(conn: &rusqlite::Connection) -> bool {
    db::get_setting(conn, REGLAGE_OUVERTURE).as_deref() != Some("0")
}

/// Au démarrage : applique le réglage, et met la fenêtre de côté quand c'est
/// la veille qui vient de rouvrir le logiciel (elle ne doit pas surgir
/// devant un client).
pub fn demarrer(app: &AppHandle, l: Lancement) {
    if l == Lancement::Veille {
        if let Some(fenetre) = app.get_webview_window("main") {
            let _ = fenetre.minimize();
        }
    }
    let actif = {
        let state = app.state::<DbState>();
        let conn = state.0.lock();
        conn.map(|c| est_active(&c)).unwrap_or(true)
    };
    appliquer(actif);
}

/// Écrit ou retire l'ouverture automatique et la tâche de veille.
pub fn appliquer(actif: bool) {
    #[cfg(windows)]
    std::thread::spawn(move || {
        let Ok(exe) = std::env::current_exe() else { return };
        ecrire_cle_run(actif, &exe);
        if actif {
            creer_tache_veille(&exe);
        } else {
            supprimer_tache_veille();
        }
    });
    #[cfg(not(windows))]
    let _ = actif;
}

#[cfg(windows)]
fn ecrire_cle_run(actif: bool, exe: &Path) {
    use winreg::enums::*;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok((cle, _)) = hkcu.create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run") else {
        return;
    };
    if actif {
        let _ = cle.set_value(NOM_VALEUR_RUN, &format!("\"{}\" {ARG_SESSION}", exe.display()));
    } else {
        let _ = cle.delete_value(NOM_VALEUR_RUN);
    }
}

/// Le texte du fichier à importer dans le planificateur de tâches. Un XML
/// plutôt que des options en ligne de commande : seul le XML permet de
/// dire « même sur batterie » (un portable débranché ne rouvrirait sinon
/// jamais le logiciel) et « pas de limite de durée » (la tâche EST le
/// logiciel : une limite le tuerait).
pub fn xml_tache_veille(exe: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers>
    <TimeTrigger>
      <Repetition>
        <Interval>PT5M</Interval>
        <StopAtDurationEnd>false</StopAtDurationEnd>
      </Repetition>
      <StartBoundary>2020-01-01T00:00:00</StartBoundary>
      <Enabled>true</Enabled>
    </TimeTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <StartWhenAvailable>true</StartWhenAvailable>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <Arguments>{ARG_VEILLE}</Arguments>
    </Exec>
  </Actions>
</Task>
"#,
        exe = echapper_xml(&exe.display().to_string())
    )
}

fn echapper_xml(texte: &str) -> String {
    texte
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// `schtasks /Create /XML` veut de l'UTF-16 avec marque d'ordre des octets.
pub fn en_utf16_avec_marque(texte: &str) -> Vec<u8> {
    let mut octets = vec![0xFF, 0xFE];
    for unite in texte.encode_utf16() {
        octets.extend_from_slice(&unite.to_le_bytes());
    }
    octets
}

/// La tâche existante (sortie de `schtasks /Query /XML`) lance-t-elle déjà
/// CE programme ? Évite de la recréer — donc de toucher à la tâche qui est
/// peut-être en train de faire tourner le logiciel — à chaque démarrage.
pub fn tache_pointe_vers(xml: &str, exe: &Path) -> bool {
    let nettoye = xml.replace('\0', "").to_lowercase();
    nettoye.contains(&echapper_xml(&exe.display().to_string()).to_lowercase())
}

#[cfg(windows)]
fn creer_tache_veille(exe: &Path) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    if let Ok(sortie) = std::process::Command::new("schtasks")
        .args(["/Query", "/TN", NOM_TACHE_VEILLE, "/XML"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        if sortie.status.success() && tache_pointe_vers(&String::from_utf8_lossy(&sortie.stdout), exe) {
            return;
        }
    }
    let fichier = std::env::temp_dir().join("photocopie-benin-veille.xml");
    if std::fs::write(&fichier, en_utf16_avec_marque(&xml_tache_veille(exe))).is_err() {
        return;
    }
    let _ = std::process::Command::new("schtasks")
        .args(["/Create", "/TN", NOM_TACHE_VEILLE, "/XML"])
        .arg(&fichier)
        .arg("/F")
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    let _ = std::fs::remove_file(&fichier);
}

#[cfg(windows)]
fn supprimer_tache_veille() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new("schtasks")
        .args(["/Delete", "/TN", NOM_TACHE_VEILLE, "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

/// La croix de la fenêtre : le logiciel est réduit, pas fermé. Le tout
/// premier jour, une phrase explique pourquoi la fenêtre n'a pas disparu.
pub fn croix_cliquee(app: &AppHandle) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

    let premiere_fois = {
        let state = app.state::<DbState>();
        let Ok(conn) = state.0.lock() else { return };
        if db::get_setting(&conn, REGLAGE_INFO_FERMETURE).as_deref() == Some("1") {
            false
        } else {
            let _ = db::set_setting(&conn, REGLAGE_INFO_FERMETURE, "1");
            true
        }
    };
    if premiere_fois {
        app.dialog()
            .message(
                "Le logiciel reste ouvert en bas de l'écran pour continuer à recevoir les documents des clients.\n\n\
                 Pour l'arrêter vraiment : menu ⋮ puis « Arrêter le logiciel ».",
            )
            .title("Gestion Photocopie")
            .kind(MessageDialogKind::Info)
            .show(|_| {});
    }
}

#[tauri::command]
pub fn get_ouvrir_avec_windows(state: State<DbState>) -> bool {
    state.0.lock().map(|c| est_active(&c)).unwrap_or(true)
}

#[tauri::command]
pub fn set_ouvrir_avec_windows(state: State<DbState>, actif: bool) -> Result<(), String> {
    {
        let conn = state.0.lock().map_err(|e| e.to_string())?;
        db::set_setting(&conn, REGLAGE_OUVERTURE, if actif { "1" } else { "0" }).map_err(|e| e.to_string())?;
    }
    appliquer(actif);
    Ok(())
}

/// Le seul vrai arrêt. La veille ne rouvre rien tant que le gérant n'a pas
/// relancé le logiciel lui-même.
#[tauri::command]
pub fn arreter_le_logiciel(app: AppHandle) {
    let _ = std::fs::create_dir_all(dossier());
    let _ = std::fs::write(marqueur_arret(), b"1");
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(liste: &[&str]) -> Vec<String> {
        liste.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn reconnait_qui_a_lance_le_logiciel() {
        assert_eq!(lancement(&args(&["app.exe"])), Lancement::Normal);
        assert_eq!(lancement(&args(&["app.exe", "--session"])), Lancement::Session);
        assert_eq!(lancement(&args(&["app.exe", "--veille"])), Lancement::Veille);
    }

    #[test]
    fn la_tache_de_veille_ne_tue_pas_le_logiciel_et_marche_sur_batterie() {
        let xml = xml_tache_veille(Path::new(r"C:\Program Files\Gestion Photocopie\app.exe"));
        assert!(xml.contains("<Interval>PT5M</Interval>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(xml.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
        assert!(xml.contains("<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>"));
        assert!(xml.contains("<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>"));
        assert!(xml.contains("<RunLevel>LeastPrivilege</RunLevel>"));
        assert!(xml.contains(r"<Command>C:\Program Files\Gestion Photocopie\app.exe</Command>"));
        assert!(xml.contains("<Arguments>--veille</Arguments>"));
    }

    #[test]
    fn un_chemin_avec_des_caracteres_speciaux_reste_du_xml_valide() {
        let xml = xml_tache_veille(Path::new(r"C:\Dupont & Fils\<x>\app.exe"));
        assert!(xml.contains(r"C:\Dupont &amp; Fils\&lt;x&gt;\app.exe"));
    }

    #[test]
    fn le_fichier_de_tache_est_en_utf16_avec_marque() {
        let o = en_utf16_avec_marque("é<");
        assert_eq!(&o[..2], &[0xFF, 0xFE]);
        assert_eq!(&o[2..], &[0xE9, 0x00, 0x3C, 0x00]);
    }

    #[test]
    fn reconnait_une_tache_deja_bonne_meme_lue_en_utf16() {
        let exe = Path::new(r"C:\Gestion\app.exe");
        let xml = "<Command>C:\\Gestion\\app.exe</Command>";
        assert!(tache_pointe_vers(xml, exe));
        let utf16: String = xml.chars().flat_map(|c| [c, '\0']).collect();
        assert!(tache_pointe_vers(&utf16, exe));
        assert!(tache_pointe_vers(&xml.to_uppercase(), exe));
        assert!(!tache_pointe_vers("<Command>C:\\Autre\\app.exe</Command>", exe));
    }

    #[test]
    fn la_veille_respecte_l_arret_volontaire() {
        // Un seul test : les deux cas partagent le même fichier marqueur.
        let temp = tempfile::tempdir().unwrap();
        std::env::set_var("LOCALAPPDATA", temp.path());
        std::fs::create_dir_all(dossier()).unwrap();
        assert!(!doit_sortir(Lancement::Veille));
        std::fs::write(marqueur_arret(), b"1").unwrap();
        assert!(doit_sortir(Lancement::Veille), "arrêt volontaire : la veille ne relance pas");
        assert!(!doit_sortir(Lancement::Session), "l'ouverture de session efface l'arrêt");
        assert!(!marqueur_arret().exists());
        assert!(!doit_sortir(Lancement::Veille));
    }
}
