//! Serveur DHCP minimal : distribue une adresse à chaque téléphone qui
//! rejoint le point d'accès Wi-Fi local, et lui annonce notre serveur DNS
//! (voir `dns.rs`) comme seul serveur de noms.
//!
//! Sans lui, un téléphone connecté au Wi-Fi ne reçoit AUCUNE adresse IP —
//! il ne peut donc même pas essayer de joindre le PC, qu'il ait ou non un
//! nom à résoudre. C'est la pièce manquante qui empêchait jusqu'ici toute
//! ouverture automatique de la page d'envoi.
//!
//! Volontairement minimal : pas de suivi de baux, pas de table partagée —
//! l'adresse offerte est calculée directement à partir de l'adresse
//! matérielle (MAC) du téléphone, donc stable pour lui d'une requête à
//! l'autre sans avoir besoin de mémoriser quoi que ce soit entre deux
//! paquets. Largement suffisant pour quelques téléphones de passage dans
//! une boutique, le temps d'une visite.

use dhcproto::v4::{
    DecodeResult, Decodable, Decoder, DhcpOption, Encodable, Encoder, Message as DhcpMessage,
    MessageType, Opcode, OptionCode,
};
use std::net::Ipv4Addr;
use tokio::net::UdpSocket;

const PORT_SERVEUR: u16 = 67;
const PORT_CLIENT: u16 = 68;
const DUREE_BAIL_SECONDES: u32 = 3600;

/// Premier octet du dernier nombre de l'adresse distribuée : les 10
/// premières valeurs de la plage restent libres (10 à 19) pour une éventuelle
/// adresse fixe future, sans conflit avec ce qui est distribué ici.
const PREMIERE_ADRESSE: u8 = 20;
const NOMBRE_ADRESSES: u8 = 200;

/// Tente de s'installer sur le port DHCP. Rendu visible dans l'interface
/// (au lieu d'un simple `eprintln!` perdu dans une console qui n'existe pas
/// dans l'application installée) : Windows peut déjà faire tourner SON
/// PROPRE serveur DHCP sur la carte du point d'accès — par exemple via le
/// partage de connexion qu'il active parfois tout seul avec le Wi-Fi Direct
/// — et dans ce cas cette tentative échoue exactement comme l'a fait le
/// port 4173 quand l'application tournait deux fois : sans le dire, un
/// gérant croirait le Wi-Fi pleinement fonctionnel alors qu'aucun téléphone
/// ne recevrait d'adresse.
pub async fn demarrer(adresse_serveur: Ipv4Addr) -> Result<tauri::async_runtime::JoinHandle<()>, String> {
    let socket = UdpSocket::bind(format!("0.0.0.0:{PORT_SERVEUR}"))
        .await
        .map_err(|e| {
            format!(
                "Serveur DHCP local indisponible (port {PORT_SERVEUR} : {e}) — les téléphones \
                 connectés au Wi-Fi local n'obtiendront aucune adresse. Vérifiez qu'aucun autre \
                 logiciel (partage de connexion Windows, par exemple) n'utilise déjà ce port."
            )
        })?;
    socket
        .set_broadcast(true)
        .map_err(|e| format!("Impossible d'activer la diffusion DHCP : {e}"))?;

    Ok(tauri::async_runtime::spawn(servir(socket, adresse_serveur)))
}

/// Tourne indéfiniment (jusqu'à ce que le point d'accès soit désactivé —
/// voir `hotspot::desactiver`, qui annule cette tâche).
async fn servir(socket: UdpSocket, adresse_serveur: Ipv4Addr) {
    let mut tampon = [0u8; 576];
    loop {
        let taille = match socket.recv(&mut tampon).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(reponse) = construire_reponse(&tampon[..taille], adresse_serveur) {
            // Le client n'a pas encore d'adresse : seule la diffusion générale
            // du réseau local peut l'atteindre.
            let _ = socket
                .send_to(&reponse, ("255.255.255.255", PORT_CLIENT))
                .await;
        }
    }
}

fn decoder_requete(brut: &[u8]) -> DecodeResult<DhcpMessage> {
    DhcpMessage::decode(&mut Decoder::new(brut))
}

/// Adresse offerte à ce téléphone précis : dérivée de son adresse matérielle
/// (MAC), donc toujours la même pour lui d'une requête à l'autre, sans avoir
/// besoin de mémoriser de table de baux entre deux paquets séparés.
fn adresse_pour(chaddr: &[u8]) -> u8 {
    let empreinte = chaddr.iter().fold(0u32, |acc, o| acc.wrapping_mul(31).wrapping_add(*o as u32));
    PREMIERE_ADRESSE.wrapping_add((empreinte % NOMBRE_ADRESSES as u32) as u8)
}

fn construire_reponse(brut: &[u8], adresse_serveur: Ipv4Addr) -> Option<Vec<u8>> {
    let requete = decoder_requete(brut).ok()?;
    if requete.opcode() != Opcode::BootRequest {
        return None;
    }

    let Some(DhcpOption::MessageType(type_demande)) = requete.opts().get(OptionCode::MessageType)
    else {
        return None;
    };

    let type_reponse = match type_demande {
        MessageType::Discover => MessageType::Offer,
        // Une Request confirme simplement l'adresse déjà annoncée dans notre
        // Offer précédente : pas de vraie négociation possible (on n'a rien
        // mémorisé), donc toujours un Ack plutôt qu'un Nak — l'adresse
        // dérivée du MAC du téléphone est de toute façon stable.
        MessageType::Request => MessageType::Ack,
        _ => return None,
    };

    let [a, b, c, _] = adresse_serveur.octets();
    let adresse_client = Ipv4Addr::new(a, b, c, adresse_pour(requete.chaddr()));

    let mut reponse = DhcpMessage::new(
        Ipv4Addr::UNSPECIFIED,
        adresse_client,
        adresse_serveur,
        Ipv4Addr::UNSPECIFIED,
        requete.chaddr(),
    );
    reponse.set_opcode(Opcode::BootReply);
    reponse.set_htype(requete.htype());
    reponse.set_xid(requete.xid());
    reponse.set_flags(requete.flags());
    reponse.set_siaddr(adresse_serveur);

    let opts = reponse.opts_mut();
    opts.insert(DhcpOption::MessageType(type_reponse));
    opts.insert(DhcpOption::ServerIdentifier(adresse_serveur));
    opts.insert(DhcpOption::SubnetMask(Ipv4Addr::new(255, 255, 255, 0)));
    opts.insert(DhcpOption::Router(vec![adresse_serveur]));
    opts.insert(DhcpOption::DomainNameServer(vec![adresse_serveur]));
    opts.insert(DhcpOption::AddressLeaseTime(DUREE_BAIL_SECONDES));
    opts.insert(DhcpOption::End);

    let mut octets = Vec::new();
    reponse.encode(&mut Encoder::new(&mut octets)).ok()?;
    Some(octets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fabriquer_requete(type_message: MessageType, mac: &[u8; 6]) -> Vec<u8> {
        let mut requete = DhcpMessage::new(
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::UNSPECIFIED,
            mac,
        );
        requete.set_opcode(Opcode::BootRequest);
        requete.set_xid(123456);
        requete.opts_mut().insert(DhcpOption::MessageType(type_message));
        let mut octets = Vec::new();
        requete.encode(&mut Encoder::new(&mut octets)).unwrap();
        octets
    }

    #[test]
    fn offre_une_adresse_dans_la_plage_attendue() {
        let serveur = Ipv4Addr::new(192, 168, 73, 1);
        let requete = fabriquer_requete(MessageType::Discover, &[1, 2, 3, 4, 5, 6]);

        let brut = construire_reponse(&requete, serveur).expect("une offre est attendue");
        let reponse = decoder_requete(&brut).unwrap();

        assert_eq!(reponse.opcode(), Opcode::BootReply);
        assert_eq!(reponse.xid(), 123456, "le xid doit être recopié tel quel");
        let offerte = reponse.yiaddr();
        assert_eq!(offerte.octets()[..3], [192, 168, 73]);
        assert!((20..220).contains(&offerte.octets()[3]));

        match reponse.opts().get(OptionCode::MessageType) {
            Some(DhcpOption::MessageType(MessageType::Offer)) => {}
            autre => panic!("attendu Offer, obtenu {autre:?}"),
        }
        assert_eq!(
            reponse.opts().get(OptionCode::DomainNameServer),
            Some(&DhcpOption::DomainNameServer(vec![serveur])),
            "le téléphone doit recevoir notre PC comme unique serveur DNS"
        );
    }

    #[test]
    fn la_meme_adresse_mac_recoit_toujours_la_meme_adresse_ip() {
        let serveur = Ipv4Addr::new(192, 168, 73, 1);
        let mac = [0xAA, 0xBB, 0xCC, 0x11, 0x22, 0x33];

        let offre1 = decoder_requete(
            &construire_reponse(&fabriquer_requete(MessageType::Discover, &mac), serveur).unwrap(),
        )
        .unwrap();
        let offre2 = decoder_requete(
            &construire_reponse(&fabriquer_requete(MessageType::Request, &mac), serveur).unwrap(),
        )
        .unwrap();

        assert_eq!(
            offre1.yiaddr(),
            offre2.yiaddr(),
            "le même téléphone doit recevoir la même adresse à chaque requête"
        );
    }

    #[test]
    fn ignore_un_paquet_illisible_sans_planter() {
        assert!(construire_reponse(&[9, 9, 9], Ipv4Addr::new(192, 168, 73, 1)).is_none());
    }

    #[test]
    fn ignore_les_types_de_messages_qu_on_ne_traite_pas() {
        // RELEASE, DECLINE... : on n'a rien à répondre, pas de bail à tenir.
        let serveur = Ipv4Addr::new(192, 168, 73, 1);
        let requete = fabriquer_requete(MessageType::Release, &[1, 2, 3, 4, 5, 6]);
        assert!(construire_reponse(&requete, serveur).is_none());
    }
}
