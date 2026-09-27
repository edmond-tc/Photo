//! Le nom fixe `kiosque.local`, pour la réception directe.
//!
//! Quand le PC rejoint le partage de connexion d'un client, c'est le
//! téléphone qui lui donne son adresse (172.20.10.x sur iPhone, 192.168.x.y
//! sur Android) : elle change d'un client à l'autre, et le client ne peut
//! pas la deviner. Le téléphone ouvre donc toujours la même adresse,
//! `http://kiosque.local:4173`, et demande au réseau « qui est
//! kiosque.local ? ». Ce module répond : « moi, à telle adresse ».
//!
//! C'est le protocole mDNS (RFC 6762), celui des imprimantes et de
//! l'AirPlay : iPhone et Android récents le parlent sans rien installer, et
//! sans internet. La question part en multidiffusion sur le seul réseau du
//! téléphone : un PC d'un kiosque voisin, qui n'est pas sur ce réseau, ne
//! l'entend jamais.
//!
//! Écrit à la main, comme `snmp.rs` : un paquet de question, un paquet de
//! réponse, quelques dizaines d'octets.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::Duration;

pub const NOM: &str = "kiosque.local";
const PORT_MDNS: u16 = 5353;
const GROUPE: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
/// Durée de validité annoncée : courte, puisque l'adresse change au client
/// suivant.
const TTL: u32 = 60;
/// Pour les questions « à l'ancienne » (port source autre que 5353), la
/// norme demande une durée d'au plus 10 s.
const TTL_ANCIENNE: u32 = 10;

const TYPE_A: u16 = 1;
const TYPE_TOUT: u16 = 255;
const CLASSE_IN: u16 = 1;
/// Bit « remplace ce que tu avais en mémoire » : l'adresse d'hier soir
/// (autre client) ne doit pas rester dans le téléphone.
const VIDER_CACHE: u16 = 0x8000;

// ───────────────────────────── Paquets (logique pure) ─────────────────────────────

/// Lit un nom de domaine à partir de `position`, en suivant les pointeurs
/// de compression. Rend (nom, position après le nom).
fn lire_nom(paquet: &[u8], mut position: usize) -> Option<(String, usize)> {
    let mut morceaux: Vec<String> = Vec::new();
    let mut apres: Option<usize> = None;
    let mut sauts = 0;
    loop {
        let longueur = *paquet.get(position)? as usize;
        if longueur == 0 {
            position += 1;
            break;
        }
        if longueur & 0xC0 == 0xC0 {
            let suite = *paquet.get(position + 1)? as usize;
            apres.get_or_insert(position + 2);
            position = ((longueur & 0x3F) << 8) | suite;
            sauts += 1;
            if sauts > 16 {
                return None; // boucle de pointeurs : paquet piégé
            }
            continue;
        }
        if longueur > 63 {
            return None;
        }
        let etiquette = paquet.get(position + 1..position + 1 + longueur)?;
        morceaux.push(String::from_utf8_lossy(etiquette).to_string());
        position += 1 + longueur;
    }
    Some((morceaux.join("."), apres.unwrap_or(position)))
}

fn ecrire_nom(nom: &str, sortie: &mut Vec<u8>) {
    for etiquette in nom.split('.').filter(|e| !e.is_empty()) {
        sortie.push(etiquette.len() as u8);
        sortie.extend_from_slice(etiquette.as_bytes());
    }
    sortie.push(0);
}

/// Une question lue dans le paquet : (nom, type, classe brute).
fn questions(paquet: &[u8]) -> Option<Vec<(String, u16, u16)>> {
    if paquet.len() < 12 {
        return None;
    }
    let drapeaux = u16::from_be_bytes([paquet[2], paquet[3]]);
    if drapeaux & 0x8000 != 0 {
        return None; // c'est une réponse, pas une question
    }
    let nombre = u16::from_be_bytes([paquet[4], paquet[5]]).min(32);
    let mut position = 12;
    let mut liste = Vec::new();
    for _ in 0..nombre {
        let (nom, apres) = lire_nom(paquet, position)?;
        let champs = paquet.get(apres..apres + 4)?;
        liste.push((
            nom,
            u16::from_be_bytes([champs[0], champs[1]]),
            u16::from_be_bytes([champs[2], champs[3]]),
        ));
        position = apres + 4;
    }
    Some(liste)
}

/// L'enregistrement « kiosque.local = adresse ».
fn enregistrement(adresse: Ipv4Addr, ttl: u32, vider_cache: bool, sortie: &mut Vec<u8>) {
    ecrire_nom(NOM, sortie);
    sortie.extend(TYPE_A.to_be_bytes());
    let classe = if vider_cache { CLASSE_IN | VIDER_CACHE } else { CLASSE_IN };
    sortie.extend(classe.to_be_bytes());
    sortie.extend(ttl.to_be_bytes());
    sortie.extend(4u16.to_be_bytes());
    sortie.extend(adresse.octets());
}

/// Réponse multidiffusée (ou annonce spontanée) : identifiant 0, pas de
/// question recopiée, comme le veut la norme.
pub fn annonce(adresse: Ipv4Addr) -> Vec<u8> {
    let mut paquet = vec![0, 0, 0x84, 0x00, 0, 0, 0, 1, 0, 0, 0, 0];
    enregistrement(adresse, TTL, true, &mut paquet);
    paquet
}

/// La réponse à un paquet reçu, s'il demande `kiosque.local`.
/// `ancienne` : question venue d'un autre port que 5353 — on répond alors
/// directement, avec le même identifiant et la question recopiée.
pub fn repondre(paquet: &[u8], adresse: Ipv4Addr, ancienne: bool) -> Option<Vec<u8>> {
    let liste = questions(paquet)?;
    let (nom, type_, classe) = liste.into_iter().find(|(nom, type_, classe)| {
        nom.eq_ignore_ascii_case(NOM)
            && (*type_ == TYPE_A || *type_ == TYPE_TOUT)
            && (classe & !VIDER_CACHE) == CLASSE_IN
    })?;
    if !ancienne {
        return Some(annonce(adresse));
    }
    let mut reponse = vec![paquet[0], paquet[1], 0x84, 0x00, 0, 1, 0, 1, 0, 0, 0, 0];
    ecrire_nom(&nom, &mut reponse);
    reponse.extend(type_.to_be_bytes());
    reponse.extend((classe & !VIDER_CACHE).to_be_bytes());
    enregistrement(adresse, TTL_ANCIENNE, false, &mut reponse);
    Some(reponse)
}

// ───────────────────────────── Répondeur ─────────────────────────────

fn ouvrir() -> std::io::Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    // Windows écoute déjà ce port pour ses propres noms : on le partage.
    socket.set_reuse_address(true)?;
    socket.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, PORT_MDNS)).into())?;
    socket.set_multicast_ttl_v4(255)?;
    socket.set_read_timeout(Some(Duration::from_secs(1)))?;
    Ok(socket.into())
}

/// Lance le répondeur, pour toute la durée de l'application. Il ne répond
/// que lorsque le PC est sur le réseau d'un client (adresse connue de
/// `reception_directe`).
pub fn demarrer() {
    std::thread::spawn(|| {
        let socket = loop {
            match ouvrir() {
                Ok(s) => break s,
                Err(e) => {
                    crate::reception_directe::noter(format!(
                        "❌ Nom kiosque.local indisponible (port 5353 : {e}). Nouvel essai dans 1 min."
                    ));
                    std::thread::sleep(Duration::from_secs(60));
                }
            }
        };
        let groupe = SocketAddr::V4(SocketAddrV4::new(GROUPE, PORT_MDNS));
        let mut rejoint: Option<Ipv4Addr> = None;
        let mut tampon = [0u8; 1500];
        loop {
            let actuelle = crate::reception_directe::adresse_actuelle();
            if actuelle != rejoint {
                if let Some(ancienne) = rejoint.take() {
                    let _ = socket.leave_multicast_v4(&GROUPE, &ancienne);
                }
                if let Some(adresse) = actuelle {
                    let rejointe = socket.join_multicast_v4(&GROUPE, &adresse);
                    let _ = socket2::SockRef::from(&socket).set_multicast_if_v4(&adresse);
                    match rejointe {
                        Ok(()) => {
                            // Deux annonces, comme le prévoit la norme : le
                            // téléphone connaît l'adresse avant même de
                            // demander.
                            let _ = socket.send_to(&annonce(adresse), groupe);
                            std::thread::sleep(Duration::from_millis(1000));
                            let _ = socket.send_to(&annonce(adresse), groupe);
                            crate::reception_directe::noter(format!(
                                "📣 kiosque.local annoncé à l'adresse {adresse}"
                            ));
                        }
                        Err(e) => crate::reception_directe::noter(format!(
                            "❌ kiosque.local : écoute impossible sur {adresse} ({e})"
                        )),
                    }
                    rejoint = Some(adresse);
                }
            }
            let Ok((taille, expediteur)) = socket.recv_from(&mut tampon) else {
                continue;
            };
            let Some(adresse) = rejoint else { continue };
            let SocketAddr::V4(expediteur) = expediteur else { continue };
            if *expediteur.ip() == adresse {
                continue; // notre propre annonce
            }
            let ancienne = expediteur.port() != PORT_MDNS;
            if let Some(reponse) = repondre(&tampon[..taille], adresse, ancienne) {
                if ancienne {
                    let _ = socket.send_to(&reponse, expediteur);
                } else {
                    // Multidiffusion (la norme), plus un envoi direct : si
                    // le réseau du téléphone filtre la multidiffusion, la
                    // réponse directe passe quand même.
                    let _ = socket.send_to(&reponse, groupe);
                    let _ = socket.send_to(&reponse, expediteur);
                }
                crate::reception_directe::noter(format!(
                    "🔎 {} a demandé kiosque.local → {adresse}",
                    expediteur.ip()
                ));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Question telle qu'un téléphone l'envoie.
    fn question(identifiant: u16, nom: &str, type_: u16, classe: u16) -> Vec<u8> {
        let mut p = identifiant.to_be_bytes().to_vec();
        p.extend([0, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
        ecrire_nom(nom, &mut p);
        p.extend(type_.to_be_bytes());
        p.extend(classe.to_be_bytes());
        p
    }

    const ICI: Ipv4Addr = Ipv4Addr::new(172, 20, 10, 2);

    #[test]
    fn repond_a_kiosque_local_en_multidiffusion() {
        let r = repondre(&question(0, "kiosque.local", 1, 0x8001), ICI, false).unwrap();
        assert_eq!(&r[..12], &[0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 0]);
        assert!(r.ends_with(&[0, 4, 172, 20, 10, 2]));
        // Le nom est bien écrit, avec le bit « vider le cache ».
        let (nom, apres) = lire_nom(&r, 12).unwrap();
        assert_eq!(nom, "kiosque.local");
        assert_eq!(&r[apres..apres + 4], &[0, 1, 0x80, 1]);
    }

    #[test]
    fn repond_directement_aux_questions_a_l_ancienne() {
        let q = question(0xBEEF, "KIOSQUE.local", 1, 1);
        let r = repondre(&q, ICI, true).unwrap();
        assert_eq!(&r[..2], &[0xBE, 0xEF], "même identifiant");
        assert_eq!(&r[4..8], &[0, 1, 0, 1], "question recopiée, une réponse");
        assert!(r.ends_with(&[0, 0, 0, TTL_ANCIENNE as u8, 0, 4, 172, 20, 10, 2]));
    }

    #[test]
    fn ignore_les_autres_noms_types_et_les_reponses() {
        assert!(repondre(&question(0, "imprimante.local", 1, 1), ICI, false).is_none());
        assert!(repondre(&question(0, "kiosque.local", 28, 1), ICI, false).is_none());
        let mut reponse = question(0, "kiosque.local", 1, 1);
        reponse[2] = 0x84;
        assert!(repondre(&reponse, ICI, false).is_none());
        assert!(repondre(&[0, 1, 2], ICI, false).is_none());
    }

    #[test]
    fn suit_la_compression_et_resiste_aux_paquets_pieges() {
        // Deux questions ; la seconde pointe vers « local » de la première.
        let mut p = vec![0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0];
        ecrire_nom("autre.local", &mut p); // « local » commence à l'octet 18
        p.extend([0, 1, 0, 1]);
        p.push(7);
        p.extend(b"kiosque");
        p.extend([0xC0, 18, 0, 1, 0, 1]);
        assert!(repondre(&p, ICI, false).is_some());

        // Pointeur qui se désigne lui-même : refus, pas de boucle infinie.
        let piege = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0xC0, 12, 0, 1, 0, 1];
        assert!(repondre(&piege, ICI, false).is_none());
    }
}
