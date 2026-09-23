//! Décodage d'OBEX, la langue que parle un téléphone qui envoie un fichier
//! par Bluetooth ("Object Push").
//!
//! Le dialogue tient en trois temps : le téléphone se connecte (CONNECT),
//! envoie le fichier — le nom d'abord, puis le contenu DÉCOUPÉ en morceaux
//! de quelques kilo-octets (PUT, répété), le dernier morceau étant marqué
//! comme tel — puis se déconnecte (DISCONNECT). À chaque morceau reçu, le
//! PC doit répondre "continue", et "c'est bon" au dernier : sans réponse,
//! le téléphone abandonne l'envoi.
//!
//! Tout ce fichier est de la logique pure, volontairement séparée du
//! Bluetooth lui-même (voir `bluetooth.rs`) : c'est ce qui permet de le
//! tester entièrement ici, sans PC Windows ni téléphone, en lui présentant
//! des paquets fabriqués à la main. Un défaut à cet endroit donnerait un
//! fichier reçu silencieusement corrompu — le genre de panne qu'un gérant
//! ne verrait qu'en imprimant devant son client.

/// Codes d'opération OBEX (le premier octet de chaque paquet). Le bit de
/// poids fort (0x80) signale "dernier paquet de la requête".
const OP_CONNECT: u8 = 0x80;
const OP_DISCONNECT: u8 = 0x81;
const OP_PUT: u8 = 0x02;
const OP_PUT_FINAL: u8 = 0x82;
const OP_ABORT: u8 = 0xFF;

/// Identifiants d'en-tête OBEX utilisés par l'envoi de fichier.
const ENTETE_NOM: u8 = 0x01;
const ENTETE_CORPS: u8 = 0x48;
const ENTETE_FIN_DE_CORPS: u8 = 0x49;

/// Réponses OBEX.
const REPONSE_CONTINUER: u8 = 0x90;
const REPONSE_SUCCES: u8 = 0xA0;

/// Taille de paquet que l'on annonce au téléphone. Volontairement modeste :
/// les piles Bluetooth de certains téléphones se comportent mal avec de
/// grandes valeurs, et le débit réel est de toute façon limité par la radio.
pub const TAILLE_PAQUET_MAX: u16 = 4096;

/// Un fichier plus gros que ça n'a rien à faire en Bluetooth (ce serait des
/// heures de transfert) et, surtout, un envoi sans fin remplirait la mémoire
/// du PC de la boutique.
pub const TAILLE_FICHIER_MAX: usize = 100 * 1024 * 1024;

#[derive(Debug, PartialEq)]
pub enum Requete {
    Connexion,
    /// Un morceau du fichier. `nom` n'accompagne en général que le premier.
    Morceau {
        nom: Option<String>,
        donnees: Vec<u8>,
        dernier: bool,
    },
    Deconnexion,
    /// Le téléphone renonce (client qui annule, sortie de portée…).
    Abandon,
}

/// Découpe un paquet OBEX. `None` quand le paquet est illisible ou tronqué :
/// on préfère ignorer que deviner, un fichier à moitié inventé étant pire
/// qu'un envoi échoué.
pub fn decoder_requete(paquet: &[u8]) -> Option<Requete> {
    if paquet.len() < 3 {
        return None;
    }
    let operation = paquet[0];
    let longueur_annoncee = u16::from_be_bytes([paquet[1], paquet[2]]) as usize;
    // Un paquet qui annonce une longueur différente de sa taille réelle est
    // soit tronqué, soit forgé : dans les deux cas on ne le traite pas.
    if longueur_annoncee != paquet.len() {
        return None;
    }

    match operation {
        OP_CONNECT => Some(Requete::Connexion),
        OP_DISCONNECT => Some(Requete::Deconnexion),
        OP_ABORT => Some(Requete::Abandon),
        OP_PUT | OP_PUT_FINAL => {
            let entetes = lire_entetes(&paquet[3..])?;
            let mut nom = None;
            let mut donnees = Vec::new();
            let mut fin_de_corps_vue = false;

            for (identifiant, contenu) in entetes {
                match identifiant {
                    ENTETE_NOM => nom = decoder_texte_utf16(&contenu),
                    ENTETE_CORPS => donnees.extend_from_slice(&contenu),
                    ENTETE_FIN_DE_CORPS => {
                        donnees.extend_from_slice(&contenu);
                        fin_de_corps_vue = true;
                    }
                    _ => {}
                }
            }

            Some(Requete::Morceau {
                nom,
                donnees,
                // Le téléphone signale la fin de deux façons selon les
                // modèles : le bit "final" sur l'opération, ou l'en-tête
                // "fin de corps". Accepter les deux évite de rester bloqué à
                // attendre une suite qui ne viendra jamais.
                dernier: operation == OP_PUT_FINAL || fin_de_corps_vue,
            })
        }
        _ => None,
    }
}

/// Les en-têtes OBEX s'enchaînent ; les deux bits de poids fort de
/// l'identifiant disent comment lire la valeur qui suit.
fn lire_entetes(mut reste: &[u8]) -> Option<Vec<(u8, Vec<u8>)>> {
    let mut entetes = Vec::new();

    while !reste.is_empty() {
        let identifiant = reste[0];
        let (valeur, consomme) = match identifiant & 0xC0 {
            // Texte ou suite d'octets : longueur sur 2 octets, en-tête inclus.
            0x00 | 0x40 => {
                if reste.len() < 3 {
                    return None;
                }
                let longueur = u16::from_be_bytes([reste[1], reste[2]]) as usize;
                if longueur < 3 || longueur > reste.len() {
                    return None;
                }
                (reste[3..longueur].to_vec(), longueur)
            }
            // Valeur sur 1 octet.
            0x80 => {
                if reste.len() < 2 {
                    return None;
                }
                (vec![reste[1]], 2)
            }
            // Valeur sur 4 octets.
            _ => {
                if reste.len() < 5 {
                    return None;
                }
                (reste[1..5].to_vec(), 5)
            }
        };

        entetes.push((identifiant, valeur));
        reste = &reste[consomme..];
    }

    Some(entetes)
}

/// Le nom de fichier voyage en UTF-16 gros-boutien, terminé par un zéro.
fn decoder_texte_utf16(octets: &[u8]) -> Option<String> {
    if octets.len() < 2 {
        return None;
    }
    let unites: Vec<u16> = octets
        .chunks_exact(2)
        .map(|paire| u16::from_be_bytes([paire[0], paire[1]]))
        .take_while(|unite| *unite != 0)
        .collect();
    let texte = String::from_utf16(&unites).ok()?;
    if texte.is_empty() {
        None
    } else {
        Some(texte)
    }
}

/// Ramène le nom envoyé par le téléphone à un simple nom de fichier.
///
/// Rien n'empêche un téléphone — ou quelqu'un qui en imite un — d'annoncer
/// un nom comme `..\..\Windows\System32\quelquechose.exe`, qui sortirait du
/// dossier surveillé, ou `CON.pdf`, un nom de périphérique réservé sur
/// lequel l'écriture échoue en silence et fait disparaître le document du
/// client sans que personne ne s'en aperçoive.
///
/// On réutilise volontairement la protection déjà écrite et éprouvée pour
/// les envois par le Wi-Fi (`server::nom_fichier_sans_chemin`) : en écrire
/// une seconde ici, c'était garantir qu'elles divergent, et que le canal le
/// moins bien protégé devienne la porte d'entrée.
pub fn nom_de_fichier_sur(nom: Option<&str>) -> String {
    crate::server::nom_fichier_sans_chemin(nom.unwrap_or(""))
}

/// Réponse à CONNECT : accepte la connexion et annonce la taille de paquet.
pub fn reponse_connexion() -> Vec<u8> {
    let mut paquet = vec![REPONSE_SUCCES, 0, 7, 0x10, 0x00];
    paquet.extend_from_slice(&TAILLE_PAQUET_MAX.to_be_bytes());
    paquet
}

/// "Morceau reçu, envoie la suite."
pub fn reponse_continuer() -> Vec<u8> {
    vec![REPONSE_CONTINUER, 0, 3]
}

/// "Terminé." Sert aussi de réponse à DISCONNECT.
pub fn reponse_succes() -> Vec<u8> {
    vec![REPONSE_SUCCES, 0, 3]
}

/// Rassemble les morceaux d'un même envoi.
#[derive(Default)]
pub struct FichierEnCours {
    pub nom: Option<String>,
    pub donnees: Vec<u8>,
}

impl FichierEnCours {
    /// Ajoute un morceau. Erreur si l'envoi dépasse la taille maximale —
    /// sans cette borne, un envoi sans fin épuiserait la mémoire du PC.
    pub fn ajouter(&mut self, nom: Option<String>, donnees: &[u8]) -> Result<(), String> {
        if self.nom.is_none() {
            self.nom = nom;
        }
        if self.donnees.len() + donnees.len() > TAILLE_FICHIER_MAX {
            return Err("Fichier trop volumineux pour un envoi Bluetooth.".to_string());
        }
        self.donnees.extend_from_slice(donnees);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fabrique un en-tête "suite d'octets" (corps ou fin de corps).
    fn entete_octets(identifiant: u8, contenu: &[u8]) -> Vec<u8> {
        let longueur = (contenu.len() + 3) as u16;
        let mut entete = vec![identifiant];
        entete.extend_from_slice(&longueur.to_be_bytes());
        entete.extend_from_slice(contenu);
        entete
    }

    fn entete_nom(nom: &str) -> Vec<u8> {
        let mut contenu: Vec<u8> = nom
            .encode_utf16()
            .flat_map(|unite| unite.to_be_bytes())
            .collect();
        contenu.extend_from_slice(&[0, 0]);
        entete_octets(ENTETE_NOM, &contenu)
    }

    fn paquet(operation: u8, corps: Vec<u8>) -> Vec<u8> {
        let longueur = (corps.len() + 3) as u16;
        let mut paquet = vec![operation];
        paquet.extend_from_slice(&longueur.to_be_bytes());
        paquet.extend_from_slice(&corps);
        paquet
    }

    #[test]
    fn lit_un_envoi_tenant_dans_un_seul_paquet() {
        let mut corps = entete_nom("facture.pdf");
        corps.extend(entete_octets(ENTETE_FIN_DE_CORPS, b"PDF-CONTENU"));

        let requete = decoder_requete(&paquet(OP_PUT_FINAL, corps)).expect("paquet valide");

        assert_eq!(
            requete,
            Requete::Morceau {
                nom: Some("facture.pdf".to_string()),
                donnees: b"PDF-CONTENU".to_vec(),
                dernier: true,
            }
        );
    }

    #[test]
    fn rassemble_un_fichier_envoye_en_plusieurs_morceaux() {
        // Le cas normal dès que le fichier dépasse quelques kilo-octets :
        // le nom n'arrive qu'avec le premier morceau, et seul le dernier
        // porte "fin de corps".
        let mut premier = entete_nom("photo.jpg");
        premier.extend(entete_octets(ENTETE_CORPS, b"AAAA"));
        let deuxieme = entete_octets(ENTETE_CORPS, b"BBBB");
        let troisieme = entete_octets(ENTETE_FIN_DE_CORPS, b"CCCC");

        let mut fichier = FichierEnCours::default();
        let mut termine = false;

        for (operation, corps) in [
            (OP_PUT, premier),
            (OP_PUT, deuxieme),
            (OP_PUT_FINAL, troisieme),
        ] {
            let Some(Requete::Morceau {
                nom,
                donnees,
                dernier,
            }) = decoder_requete(&paquet(operation, corps))
            else {
                panic!("morceau illisible");
            };
            fichier.ajouter(nom, &donnees).expect("taille raisonnable");
            termine = dernier;
        }

        assert!(termine, "le dernier morceau doit clore l'envoi");
        assert_eq!(fichier.nom.as_deref(), Some("photo.jpg"));
        assert_eq!(fichier.donnees, b"AAAABBBBCCCC");
    }

    #[test]
    fn lit_un_nom_de_fichier_accentue() {
        // Les noms viennent de téléphones francophones : "Reçu août.pdf" ne
        // doit pas arriver déformé dans la file d'attente du gérant.
        let corps = entete_nom("Reçu août.pdf");
        let Some(Requete::Morceau { nom, .. }) = decoder_requete(&paquet(OP_PUT_FINAL, corps))
        else {
            panic!("paquet illisible");
        };
        assert_eq!(nom.as_deref(), Some("Reçu août.pdf"));
    }

    #[test]
    fn refuse_un_nom_qui_sortirait_du_dossier_surveille() {
        // Un nom forgé ne doit jamais permettre d'écrire ailleurs que dans
        // le dossier prévu.
        assert_eq!(
            nom_de_fichier_sur(Some(r"..\..\Windows\System32\piege.exe")),
            "piege.exe"
        );
        assert_eq!(nom_de_fichier_sur(Some("/etc/passwd")), "passwd");
        assert_eq!(
            nom_de_fichier_sur(Some("../../../secret.txt")),
            "secret.txt"
        );
        assert_eq!(nom_de_fichier_sur(Some("do:cu*ment?.pdf")), "document.pdf");
    }

    #[test]
    fn invente_un_nom_quand_le_telephone_n_en_donne_pas() {
        // Certains téléphones envoient sans nom : un fichier sans nom ne
        // doit pas faire échouer la réception.
        assert_eq!(nom_de_fichier_sur(None), "fichier_recu");
        assert_eq!(nom_de_fichier_sur(Some("   ")), "fichier_recu");
        assert_eq!(nom_de_fichier_sur(Some("..")), "fichier_recu");
    }

    #[test]
    fn le_bluetooth_est_protege_des_noms_reserves_de_windows() {
        // Trouvé à l'audit : cette protection existait pour les envois par
        // le Wi-Fi mais pas pour le Bluetooth. Écrire dans "CON.pdf" ou
        // "nul" échoue en silence sous Windows — le document du client
        // disparaîtrait sans message d'erreur.
        assert_eq!(nom_de_fichier_sur(Some("CON.pdf")), "fichier_recu");
        assert_eq!(nom_de_fichier_sur(Some("nul")), "fichier_recu");
        assert_eq!(nom_de_fichier_sur(Some("lpt1.txt")), "fichier_recu");
    }

    #[test]
    fn ignore_un_paquet_tronque_ou_incoherent_sans_planter() {
        assert_eq!(decoder_requete(&[]), None);
        assert_eq!(decoder_requete(&[OP_PUT]), None);
        // Annonce 200 octets mais n'en contient que 3.
        assert_eq!(decoder_requete(&[OP_PUT_FINAL, 0, 200]), None);
        // En-tête dont la longueur déborde du paquet.
        assert_eq!(
            decoder_requete(&paquet(OP_PUT_FINAL, vec![ENTETE_CORPS, 0xFF, 0xFF])),
            None
        );
        // En-tête annonçant une longueur plus petite que lui-même.
        assert_eq!(
            decoder_requete(&paquet(OP_PUT_FINAL, vec![ENTETE_CORPS, 0, 1])),
            None
        );
    }

    #[test]
    fn reconnait_la_connexion_et_la_deconnexion() {
        assert_eq!(
            decoder_requete(&[OP_CONNECT, 0, 7, 0x10, 0x00, 0x10, 0x00]),
            Some(Requete::Connexion)
        );
        assert_eq!(
            decoder_requete(&[OP_DISCONNECT, 0, 3]),
            Some(Requete::Deconnexion)
        );
        assert_eq!(decoder_requete(&[OP_ABORT, 0, 3]), Some(Requete::Abandon));
    }

    #[test]
    fn la_reponse_de_connexion_annonce_la_taille_de_paquet() {
        let reponse = reponse_connexion();
        assert_eq!(reponse[0], REPONSE_SUCCES);
        assert_eq!(
            u16::from_be_bytes([reponse[1], reponse[2]]) as usize,
            reponse.len()
        );
        assert_eq!(
            u16::from_be_bytes([reponse[5], reponse[6]]),
            TAILLE_PAQUET_MAX
        );
    }

    #[test]
    fn borne_la_taille_d_un_envoi() {
        let mut fichier = FichierEnCours::default();
        let gros = vec![0u8; TAILLE_FICHIER_MAX];
        assert!(fichier.ajouter(None, &gros).is_ok());
        assert!(
            fichier.ajouter(None, b"un octet de trop").is_err(),
            "un envoi sans fin remplirait la mémoire du PC"
        );
    }
}
