//! Lecture du compteur de pages d'une imprimante ou d'un photocopieur
//! branché en RÉSEAU, par le protocole standard SNMP (version 1,
//! communauté « public », activée d'usine sur presque tous les modèles).
//!
//! Le compteur lu est `prtMarkerLifeCount` de la norme Printer-MIB
//! (RFC 3805) : le nombre total de pages produites par la machine depuis
//! sa fabrication, photocopies comprises. C'est lui qui permet de voir les
//! copies faites directement sur la vitre, qui ne passent jamais par le PC.
//!
//! Écrit à la main (une requête GET, une réponse) plutôt qu'avec une
//! bibliothèque : c'est une trentaine d'octets, et chaque dépendance en
//! plus est une chose de plus qui peut casser la compilation Windows.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

/// 1.3.6.1.2.1.43.10.2.1.4.1.1 : compteur à vie du premier « marqueur »
/// (moteur d'impression) de la machine.
const OID_COMPTEUR_PAGES: [u32; 13] = [1, 3, 6, 1, 2, 1, 43, 10, 2, 1, 4, 1, 1];

fn encoder_longueur(longueur: usize, sortie: &mut Vec<u8>) {
    if longueur < 0x80 {
        sortie.push(longueur as u8);
    } else if longueur <= 0xFF {
        sortie.extend([0x81, longueur as u8]);
    } else {
        sortie.extend([0x82, (longueur >> 8) as u8, longueur as u8]);
    }
}

fn tlv(type_: u8, contenu: &[u8]) -> Vec<u8> {
    let mut sortie = vec![type_];
    encoder_longueur(contenu.len(), &mut sortie);
    sortie.extend_from_slice(contenu);
    sortie
}

fn entier(valeur: u32) -> Vec<u8> {
    let mut octets = valeur.to_be_bytes().to_vec();
    // Forme la plus courte, sans rendre le nombre négatif (bit de poids
    // fort d'un premier octet non nul).
    while octets.len() > 1 && octets[0] == 0 && octets[1] & 0x80 == 0 {
        octets.remove(0);
    }
    tlv(0x02, &octets)
}

fn oid(arcs: &[u32]) -> Vec<u8> {
    let mut contenu = vec![(arcs[0] * 40 + arcs[1]) as u8];
    for &arc in &arcs[2..] {
        let mut morceaux = vec![(arc & 0x7F) as u8];
        let mut reste = arc >> 7;
        while reste > 0 {
            morceaux.push(((reste & 0x7F) as u8) | 0x80);
            reste >>= 7;
        }
        morceaux.reverse();
        contenu.extend(morceaux);
    }
    tlv(0x06, &contenu)
}

/// La requête GET complète.
fn requete(identifiant: u32) -> Vec<u8> {
    let liaison = tlv(0x30, &[oid(&OID_COMPTEUR_PAGES), vec![0x05, 0x00]].concat());
    let liaisons = tlv(0x30, &liaison);
    let pdu = tlv(
        0xA0,
        &[entier(identifiant), entier(0), entier(0), liaisons].concat(),
    );
    tlv(0x30, &[entier(0), tlv(0x04, b"public"), pdu].concat())
}

/// Lit un en-tête type + longueur ; rend (type, début du contenu, fin).
fn lire_tlv(octets: &[u8], position: usize) -> Option<(u8, usize, usize)> {
    let type_ = *octets.get(position)?;
    let premier = *octets.get(position + 1)? as usize;
    let (longueur, debut) = if premier < 0x80 {
        (premier, position + 2)
    } else {
        let nombre = premier & 0x7F;
        if nombre == 0 || nombre > 2 {
            return None;
        }
        let mut longueur = 0usize;
        for i in 0..nombre {
            longueur = (longueur << 8) | *octets.get(position + 2 + i)? as usize;
        }
        (longueur, position + 2 + nombre)
    };
    let fin = debut.checked_add(longueur)?;
    (fin <= octets.len()).then_some((type_, debut, fin))
}

/// Extrait le compteur de la réponse. `None` si la réponse est une erreur,
/// ne porte pas sur ce compteur, ou est illisible.
fn lire_reponse(octets: &[u8], identifiant: u32) -> Option<u64> {
    let (t, debut, _) = lire_tlv(octets, 0)?; // message
    (t == 0x30).then_some(())?;
    let (_, _, apres_version) = lire_tlv(octets, debut)?;
    let (_, _, apres_communaute) = lire_tlv(octets, apres_version)?;
    let (t, debut_pdu, _) = lire_tlv(octets, apres_communaute)?;
    (t == 0xA2).then_some(())?; // GetResponse
    let (_, d, apres_id) = lire_tlv(octets, debut_pdu)?;
    let id_recu = octets[d..apres_id].iter().fold(0u64, |a, &o| (a << 8) | o as u64);
    (id_recu == identifiant as u64).then_some(())?;
    let (_, d, apres_erreur) = lire_tlv(octets, apres_id)?;
    (octets[d..apres_erreur].iter().all(|&o| o == 0)).then_some(())?; // error-status = 0
    let (_, _, apres_index) = lire_tlv(octets, apres_erreur)?;
    let (_, debut_liste, _) = lire_tlv(octets, apres_index)?;
    let (_, debut_liaison, _) = lire_tlv(octets, debut_liste)?;
    let (t, d, apres_oid) = lire_tlv(octets, debut_liaison)?;
    (t == 0x06 && octets[d - 2..apres_oid] == oid(&OID_COMPTEUR_PAGES)[..]).then_some(())?;
    let (t, d, fin) = lire_tlv(octets, apres_oid)?;
    // INTEGER, Counter32, Gauge32, Counter64 : un entier non signé.
    if ![0x02, 0x41, 0x42, 0x46].contains(&t) || fin - d > 9 || fin == d {
        return None;
    }
    Some(octets[d..fin].iter().fold(0u64, |a, &o| (a << 8) | o as u64))
}

/// Le compteur de pages de la machine à cette adresse, ou `None` si elle
/// ne répond pas (éteinte, SNMP désactivé, ce n'est pas une imprimante).
pub fn compteur_pages(adresse: Ipv4Addr) -> Option<u64> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.set_read_timeout(Some(Duration::from_millis(1500))).ok()?;
    let identifiant = rand::random::<u32>() & 0x7FFF_FFFF;
    socket
        .send_to(&requete(identifiant), SocketAddr::from((adresse, 161)))
        .ok()?;
    let mut tampon = [0u8; 1500];
    let (taille, _) = socket.recv_from(&mut tampon).ok()?;
    lire_reponse(&tampon[..taille], identifiant)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_requete_est_celle_de_la_norme() {
        // Octets d'une requête GET SNMPv1 « public » réelle pour cet OID,
        // identifiant 1.
        let attendu: Vec<u8> = vec![
            0x30, 0x2A, 0x02, 0x01, 0x00, 0x04, 0x06, b'p', b'u', b'b', b'l', b'i', b'c', 0xA0,
            0x1D, 0x02, 0x01, 0x01, 0x02, 0x01, 0x00, 0x02, 0x01, 0x00, 0x30, 0x12, 0x30, 0x10,
            0x06, 0x0C, 0x2B, 0x06, 0x01, 0x02, 0x01, 0x2B, 0x0A, 0x02, 0x01, 0x04, 0x01, 0x01,
            0x05, 0x00,
        ];
        assert_eq!(requete(1), attendu);
    }

    /// Réponse fabriquée comme la ferait une machine : compteur à 123 456.
    fn reponse(identifiant: u32, erreur: u32, valeur: &[u8], type_valeur: u8) -> Vec<u8> {
        let liaison = tlv(0x30, &[oid(&OID_COMPTEUR_PAGES), tlv(type_valeur, valeur)].concat());
        let pdu = tlv(
            0xA2,
            &[entier(identifiant), entier(erreur), entier(0), tlv(0x30, &liaison)].concat(),
        );
        tlv(0x30, &[entier(0), tlv(0x04, b"public"), pdu].concat())
    }

    #[test]
    fn lit_le_compteur_d_une_reponse() {
        let r = reponse(77, 0, &[0x01, 0xE2, 0x40], 0x41);
        assert_eq!(lire_reponse(&r, 77), Some(123_456));
        let r = reponse(77, 0, &[0x00, 0xE2, 0x40], 0x02);
        assert_eq!(lire_reponse(&r, 77), Some(57_920));
    }

    #[test]
    fn refuse_une_reponse_qui_n_est_pas_la_sienne_ou_en_erreur() {
        assert_eq!(lire_reponse(&reponse(78, 0, &[0x05], 0x41), 77), None);
        assert_eq!(lire_reponse(&reponse(77, 2, &[0x05], 0x41), 77), None);
        assert_eq!(lire_reponse(&[0x30, 0x05, 0x02], 77), None);
        assert_eq!(lire_reponse(&[], 77), None);
    }

    #[test]
    fn les_grands_identifiants_restent_positifs() {
        let r = requete(0x0080_0000);
        assert!(r.windows(6).any(|w| w == [0x02, 0x04, 0x00, 0x80, 0x00, 0x00]));
    }
}
