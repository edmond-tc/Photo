//! Réception Bluetooth permanente : l'application s'enregistre auprès de
//! Windows comme destinataire de fichiers ("OBEX Object Push"), et reste à
//! l'écoute tant qu'elle tourne.
//!
//! C'est ce qui rend le Bluetooth utilisable en boutique. Le mécanisme
//! intégré à Windows (`fsquirt.exe`) exige que le gérant clique "Recevoir un
//! fichier" AVANT chaque envoi, faute de quoi le téléphone est refusé — donc
//! qu'il devine le moment où un client va envoyer. En tenant nous-mêmes le
//! rôle de destinataire, il n'y a plus rien à cliquer : le client envoie
//! quand il veut, le fichier tombe dans le dossier surveillé, et la suite
//! (file d'attente, aperçu, facturation) se déroule comme pour n'importe
//! quel autre canal.
//!
//! Le dialogue OBEX lui-même est décodé par `obex.rs`, volontairement séparé
//! pour être testable sans Bluetooth ni Windows. Ici ne reste que la
//! plomberie : ouvrir l'oreille, lire les octets, écrire le fichier.
//!
//! Limites, sues d'avance : un iPhone ne peut pas envoyer de fichier en
//! Bluetooth à un PC (verrou d'Apple, sans contournement), et un PC sans
//! radio Bluetooth ne peut évidemment rien recevoir.

use std::path::PathBuf;

/// Où déposer ce qui arrive : le dossier surveillé du gérant, puisque c'est
/// lui que le reste de l'application regarde déjà.
fn dossier_de_reception(app: &tauri::AppHandle) -> Option<PathBuf> {
    use tauri::Manager;

    let state = app.state::<crate::db::DbState>();
    let conn = state.0.lock().ok()?;
    crate::db::get_setting(&conn, "dossier_surveille")
        .filter(|d| !d.trim().is_empty())
        .map(PathBuf::from)
}

/// Écrit le fichier reçu sans jamais en écraser un autre : deux clients qui
/// envoient "document.pdf" le même jour ne doivent pas se recouvrir.
pub fn enregistrer_fichier(
    dossier: &std::path::Path,
    nom: &str,
    donnees: &[u8],
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dossier)?;

    let mut chemin = dossier.join(nom);
    let mut suffixe = 1;
    while chemin.exists() {
        let (base, extension) = match nom.rsplit_once('.') {
            Some((base, extension)) => (base, format!(".{extension}")),
            None => (nom, String::new()),
        };
        chemin = dossier.join(format!("{base} ({suffixe}){extension}"));
        suffixe += 1;
    }

    std::fs::write(&chemin, donnees)?;
    Ok(chemin)
}

#[cfg(windows)]
mod implementation {
    use super::*;
    use std::sync::Mutex;
    use windows::Devices::Bluetooth::Rfcomm::{RfcommServiceId, RfcommServiceProvider};
    use windows::Foundation::TypedEventHandler;
    use windows::Networking::Sockets::{
        SocketProtectionLevel, StreamSocket, StreamSocketListener,
        StreamSocketListenerConnectionReceivedEventArgs,
    };
    use windows::Storage::Streams::{DataReader, DataWriter};

    /// L'annonce Bluetooth vit aussi longtemps que ces deux objets : les
    /// relâcher retirerait le PC de la liste des destinataires possibles.
    static SERVICE: Mutex<Option<(RfcommServiceProvider, StreamSocketListener)>> = Mutex::new(None);

    pub fn demarrer(app: tauri::AppHandle) {
        // Sur un PC sans radio Bluetooth, l'enregistrement échoue : c'est
        // normal et sans gravité, les autres canaux (Wi-Fi, clé USB, dossier
        // surveillé) continuent. On le signale sans jamais bloquer.
        if let Err(e) = enregistrer_service(app) {
            eprintln!(
                "Réception Bluetooth indisponible sur ce PC ({e}). Les autres moyens \
                 d'envoi (Wi-Fi local, clé USB, dossier surveillé) restent utilisables."
            );
        }
    }

    fn enregistrer_service(app: tauri::AppHandle) -> windows::core::Result<()> {
        let fournisseur =
            RfcommServiceProvider::CreateAsync(&RfcommServiceId::ObexObjectPush()?)?.get()?;
        let ecouteur = StreamSocketListener::new()?;

        ecouteur.ConnectionReceived(&TypedEventHandler::<
            StreamSocketListener,
            StreamSocketListenerConnectionReceivedEventArgs,
        >::new(move |_, arguments| {
            let Some(arguments) = arguments.as_ref() else {
                return Ok(());
            };
            let socket = arguments.Socket()?;
            let app = app.clone();
            // Un envoi peut durer plusieurs minutes : le traiter à l'écart
            // pour ne pas retenir la pile Bluetooth de Windows, et pour que
            // deux clients puissent envoyer en même temps.
            std::thread::spawn(move || {
                if let Err(e) = traiter_connexion(&socket, &app) {
                    eprintln!("Envoi Bluetooth interrompu : {e}");
                }
            });
            Ok(())
        }))?;

        ecouteur
            .BindServiceNameWithProtectionLevelAsync(
                &fournisseur.ServiceId()?.AsString()?,
                SocketProtectionLevel::PlainSocket,
            )?
            .get()?;

        fournisseur.StartAdvertising(&ecouteur)?;

        if let Ok(mut garde) = SERVICE.lock() {
            *garde = Some((fournisseur, ecouteur));
        }
        Ok(())
    }

    /// Lit exactement `combien` octets, ou moins si le téléphone a raccroché.
    fn lire_exactement(lecteur: &DataReader, combien: u32) -> windows::core::Result<Vec<u8>> {
        let recus = lecteur.LoadAsync(combien)?.get()?;
        if recus == 0 {
            return Ok(Vec::new());
        }
        let mut tampon = vec![0u8; recus as usize];
        lecteur.ReadBytes(&mut tampon)?;
        Ok(tampon)
    }

    fn repondre(ecrivain: &DataWriter, reponse: &[u8]) -> windows::core::Result<()> {
        ecrivain.WriteBytes(reponse)?;
        ecrivain.StoreAsync()?.get()?;
        Ok(())
    }

    fn traiter_connexion(
        socket: &StreamSocket,
        app: &tauri::AppHandle,
    ) -> windows::core::Result<()> {
        let lecteur = DataReader::CreateDataReader(&socket.InputStream()?)?;
        let ecrivain = DataWriter::CreateDataWriter(&socket.OutputStream()?)?;
        let mut fichier = crate::obex::FichierEnCours::default();

        loop {
            // Chaque paquet OBEX commence par son code d'opération et sa
            // longueur totale sur trois octets : on lit d'abord ça pour
            // savoir combien d'octets réclamer ensuite.
            let entete = lire_exactement(&lecteur, 3)?;
            if entete.len() < 3 {
                break;
            }
            let longueur = u16::from_be_bytes([entete[1], entete[2]]) as usize;
            let mut paquet = entete;
            if longueur > 3 {
                let suite = lire_exactement(&lecteur, (longueur - 3) as u32)?;
                if suite.len() < longueur - 3 {
                    break;
                }
                paquet.extend_from_slice(&suite);
            }

            match crate::obex::decoder_requete(&paquet) {
                Some(crate::obex::Requete::Connexion) => {
                    repondre(&ecrivain, &crate::obex::reponse_connexion())?
                }
                Some(crate::obex::Requete::Morceau {
                    nom,
                    donnees,
                    dernier,
                }) => {
                    if fichier.ajouter(nom, &donnees).is_err() {
                        // Envoi trop volumineux : on coupe plutôt que de
                        // laisser la mémoire du PC se remplir.
                        break;
                    }
                    if dernier {
                        deposer(app, &mut fichier);
                        repondre(&ecrivain, &crate::obex::reponse_succes())?;
                    } else {
                        repondre(&ecrivain, &crate::obex::reponse_continuer())?;
                    }
                }
                Some(crate::obex::Requete::Deconnexion) => {
                    repondre(&ecrivain, &crate::obex::reponse_succes())?;
                    break;
                }
                Some(crate::obex::Requete::Abandon) => break,
                None => break,
            }
        }

        Ok(())
    }

    fn deposer(app: &tauri::AppHandle, fichier: &mut crate::obex::FichierEnCours) {
        let Some(dossier) = dossier_de_reception(app) else {
            eprintln!(
                "Fichier reçu par Bluetooth mais aucun dossier surveillé n'est configuré : \
                 renseignez-le dans Réglages, sinon les envois Bluetooth sont perdus."
            );
            return;
        };

        let nom = crate::obex::nom_de_fichier_sur(fichier.nom.as_deref());
        match enregistrer_fichier(&dossier, &nom, &fichier.donnees) {
            // Mis en file tout de suite, sans attendre que la surveillance
            // du dossier le remarque : c'est un envoi Bluetooth, et la file
            // doit le dire.
            Ok(chemin) => {
                crate::watcher::enqueue_file(app, &chemin, "bluetooth", None, None);
            }
            Err(e) => eprintln!("Impossible d'enregistrer le fichier reçu par Bluetooth : {e}"),
        }

        // Le même téléphone peut enchaîner plusieurs fichiers sur une seule
        // connexion : repartir à vide, sinon le deuxième contiendrait le
        // premier.
        *fichier = crate::obex::FichierEnCours::default();
    }
}

#[cfg(not(windows))]
mod implementation {
    pub fn demarrer(_app: tauri::AppHandle) {}
}

pub use implementation::demarrer;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn n_ecrase_jamais_un_fichier_deja_recu() {
        // Deux clients qui envoient "document.pdf" dans la même journée :
        // le second ne doit pas effacer le premier, qui n'a peut-être pas
        // encore été imprimé.
        let dossier = std::env::temp_dir().join(format!(
            "test-bluetooth-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        let premier = enregistrer_fichier(&dossier, "document.pdf", b"AAA").unwrap();
        let second = enregistrer_fichier(&dossier, "document.pdf", b"BBB").unwrap();

        assert_ne!(premier, second);
        assert_eq!(std::fs::read(&premier).unwrap(), b"AAA");
        assert_eq!(std::fs::read(&second).unwrap(), b"BBB");
        assert_eq!(
            second.file_name().unwrap().to_string_lossy(),
            "document (1).pdf",
            "le doublon garde son extension, sinon le fichier n'est plus reconnu"
        );

        let _ = std::fs::remove_dir_all(&dossier);
    }

    #[test]
    fn cree_le_dossier_s_il_n_existe_pas_encore() {
        let dossier = std::env::temp_dir().join(format!(
            "test-bluetooth-absent-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        let chemin = enregistrer_fichier(&dossier, "recu.jpg", b"XYZ").unwrap();
        assert!(chemin.exists());

        let _ = std::fs::remove_dir_all(&dossier);
    }
}
