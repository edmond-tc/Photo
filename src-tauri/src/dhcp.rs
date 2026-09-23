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
            // Le client n'a pas encore d'adresse : seule une diffusion peut
            // l'atteindre. Mais LAQUELLE compte, et c'est ce qui manquait.
            //
            // `255.255.255.255` ne désigne aucun réseau en particulier : sur
            // un PC qui a plusieurs cartes — et un PC de boutique en a
            // toujours plusieurs (Ethernet, Wi-Fi, cartes virtuelles) —
            // Windows choisit tout seul par où l'envoyer, en suivant sa
            // table de routage. Il l'envoie donc vers la carte de sortie
            // habituelle, PAS vers le point d'accès qu'on vient de créer.
            // Le téléphone attendait une réponse qui partait ailleurs :
            // « Connexion… » pour toujours.
            //
            // La diffusion du sous-réseau (192.168.73.255 pour un point
            // d'accès en 192.168.73.1) ne laisse aucun choix à Windows :
            // cette plage n'existe que sur la carte du point d'accès, la
            // réponse ne peut donc sortir que par là.
            let [a, b, c, _] = adresse_serveur.octets();
            let diffusion_du_reseau = Ipv4Addr::new(a, b, c, 255);
            let _ = socket
                .send_to(&reponse, (diffusion_du_reseau, PORT_CLIENT))
                .await;
            // Envoyée aussi à l'ancienne adresse, pour les rares appareils
            // qui n'écoutent que celle-là. Un DHCP reçu deux fois ne gêne
            // aucun client ; ne pas le recevoir du tout les bloque tous.
            let _ = socket
                .send_to(&reponse, (Ipv4Addr::BROADCAST, PORT_CLIENT))
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

/// L'adresse que le téléphone réclame : soit explicitement dans sa demande,
/// soit celle qu'il utilise déjà et cherche à prolonger.
fn adresse_demandee(requete: &DhcpMessage) -> Option<Ipv4Addr> {
    if let Some(DhcpOption::RequestedIpAddress(ip)) =
        requete.opts().get(OptionCode::RequestedIpAddress)
    {
        return Some(*ip);
    }
    Some(requete.ciaddr()).filter(|ip| !ip.is_unspecified())
}

/// Cette adresse a-t-elle un sens sur notre point d'accès ? L'adresse du PC
/// lui-même est exclue : la donner à un téléphone couperait tout.
fn dans_notre_reseau(adresse: Ipv4Addr, adresse_serveur: Ipv4Addr) -> bool {
    adresse.octets()[..3] == adresse_serveur.octets()[..3]
        && adresse.octets()[3] != adresse_serveur.octets()[3]
        && adresse.octets()[3] != 0
        && adresse.octets()[3] != 255
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

    let [a, b, c, _] = adresse_serveur.octets();
    let adresse_derivee = Ipv4Addr::new(a, b, c, adresse_pour(requete.chaddr()));

    // Ce que le téléphone réclame, s'il réclame quelque chose. Un téléphone
    // qui a déjà été connecté à un autre Wi-Fi redemande d'abord SON
    // ancienne adresse, apprise ailleurs.
    let demandee = adresse_demandee(&requete);

    let (type_reponse, adresse_client) = match type_demande {
        MessageType::Discover => (MessageType::Offer, adresse_derivee),
        MessageType::Request => match demandee {
            // Adresse cohérente avec notre réseau : on l'accorde telle
            // quelle, même si ce n'est pas celle qu'on aurait choisie.
            Some(ip) if dans_notre_reseau(ip, adresse_serveur) => (MessageType::Ack, ip),
            // Adresse d'un AUTRE réseau : il faut dire NON, explicitement.
            //
            // C'est le second défaut trouvé ici, et il expliquait à lui seul
            // un « Connexion… » sans fin. On répondait « d'accord » (Ack) en
            // y mettant une adresse DIFFÉRENTE de celle demandée. Pour le
            // téléphone, cette réponse est incohérente : il la jette,
            // redemande, la jette encore — sans jamais se connecter ni
            // afficher d'erreur. Un refus net (Nak), lui, le fait repartir
            // immédiatement de zéro et aboutir en une seconde.
            Some(_) => (MessageType::Nak, Ipv4Addr::UNSPECIFIED),
            None => (MessageType::Ack, adresse_derivee),
        },
        _ => return None,
    };

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

    // Un refus ne porte aucun réglage : y joindre un masque ou une
    // passerelle ferait douter le téléphone de ce qu'on lui refuse.
    if type_reponse == MessageType::Nak {
        opts.insert(DhcpOption::End);
        let mut octets = Vec::new();
        reponse.encode(&mut Encoder::new(&mut octets)).ok()?;
        return Some(octets);
    }

    opts.insert(DhcpOption::SubnetMask(Ipv4Addr::new(255, 255, 255, 0)));
    opts.insert(DhcpOption::Router(vec![adresse_serveur]));
    opts.insert(DhcpOption::DomainNameServer(vec![adresse_serveur]));
    opts.insert(DhcpOption::AddressLeaseTime(DUREE_BAIL_SECONDES));

    // DIRE au téléphone qu'il y a un portail, au lieu d'espérer qu'il le
    // devine.
    //
    // Jusqu'ici on comptait uniquement sur sa vérification automatique :
    // le téléphone interroge une page de contrôle chez Apple ou Google, on
    // détourne la question, et il est CENSÉ en conclure « ce réseau demande
    // une connexion ». Cette déduction est une heuristique, et sur le
    // terrain elle n'aboutissait pas : la page ne s'ouvrait qu'après être
    // entré à la main dans les réglages Wi-Fi, ce qui force une nouvelle
    // vérification.
    //
    // La norme RFC 8910 prévoit exactement le cas : une option DHCP qui
    // porte l'ADRESSE du portail. Plus de déduction — le téléphone
    // l'apprend au moment même où il reçoit son adresse, avant d'avoir
    // essayé quoi que ce soit. iOS la comprend depuis la version 14,
    // Android depuis la 11.
    //
    // Sur le port 80, pas 4173 : c'est là que répond le serveur du portail
    // (voir `server::PORT_PORTAIL_CAPTIF`), et une fenêtre de portail est un
    // navigateur réduit où une adresse à port inhabituel est un risque
    // inutile.
    opts.insert(DhcpOption::CaptivePortal(format!(
        "http://{adresse_serveur}/"
    )));
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

    fn requete_avec_adresse_demandee(mac: &[u8; 6], demandee: Ipv4Addr) -> Vec<u8> {
        let mut requete = DhcpMessage::new(
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::UNSPECIFIED,
            mac,
        );
        requete.set_opcode(Opcode::BootRequest);
        requete.set_xid(123456);
        requete
            .opts_mut()
            .insert(DhcpOption::MessageType(MessageType::Request));
        requete
            .opts_mut()
            .insert(DhcpOption::RequestedIpAddress(demandee));
        let mut octets = Vec::new();
        requete.encode(&mut Encoder::new(&mut octets)).unwrap();
        octets
    }

    fn type_de(brut: &[u8]) -> MessageType {
        match decoder_requete(brut)
            .unwrap()
            .opts()
            .get(OptionCode::MessageType)
        {
            Some(DhcpOption::MessageType(t)) => *t,
            autre => panic!("type de message illisible : {autre:?}"),
        }
    }

    /// Le cas exact vu sur le terrain : « Connexion… » qui ne finit jamais.
    ///
    /// Un téléphone déjà connecté ailleurs redemande d'abord SON ancienne
    /// adresse. On répondait « d'accord » en y mettant une adresse
    /// différente : réponse incohérente, que le téléphone jette et
    /// redemande, indéfiniment, sans jamais afficher d'erreur. Il faut
    /// refuser NET pour qu'il reparte de zéro et aboutisse.
    #[test]
    fn refuse_net_une_adresse_venue_d_un_autre_reseau() {
        let serveur = Ipv4Addr::new(192, 168, 73, 1);
        let ancienne = Ipv4Addr::new(192, 168, 1, 57); // le Wi-Fi de la maison
        let brut = construire_reponse(
            &requete_avec_adresse_demandee(&[1, 2, 3, 4, 5, 6], ancienne),
            serveur,
        )
        .expect("un refus est attendu, pas un silence");

        assert_eq!(type_de(&brut), MessageType::Nak);
        let reponse = decoder_requete(&brut).unwrap();
        assert!(
            reponse.yiaddr().is_unspecified(),
            "un refus n'attribue aucune adresse"
        );
        // Un refus ne porte aucun réglage : sinon le téléphone doute de ce
        // qu'on lui refuse.
        assert_eq!(reponse.opts().get(OptionCode::SubnetMask), None);
        assert_eq!(reponse.opts().get(OptionCode::Router), None);
    }

    /// À l'inverse, une adresse cohérente avec notre réseau doit être
    /// accordée TELLE QUELLE — même si ce n'est pas celle qu'on aurait
    /// choisie. Répondre autre chose relancerait la même boucle.
    #[test]
    fn accorde_telle_quelle_une_adresse_coherente_avec_notre_reseau() {
        let serveur = Ipv4Addr::new(192, 168, 73, 1);
        let demandee = Ipv4Addr::new(192, 168, 73, 44);
        let brut = construire_reponse(
            &requete_avec_adresse_demandee(&[1, 2, 3, 4, 5, 6], demandee),
            serveur,
        )
        .unwrap();

        assert_eq!(type_de(&brut), MessageType::Ack);
        assert_eq!(
            decoder_requete(&brut).unwrap().yiaddr(),
            demandee,
            "le téléphone doit recevoir EXACTEMENT l'adresse qu'il a demandée"
        );
    }

    /// Ni l'adresse du PC, ni les adresses réservées du réseau ne peuvent
    /// être attribuées à un téléphone : la première lui couperait l'accès au
    /// PC, les autres ne désignent aucun appareil.
    #[test]
    fn n_accorde_jamais_l_adresse_du_pc_ni_les_adresses_reservees() {
        let serveur = Ipv4Addr::new(192, 168, 73, 1);
        assert!(!dans_notre_reseau(serveur, serveur));
        assert!(!dans_notre_reseau(Ipv4Addr::new(192, 168, 73, 0), serveur));
        assert!(!dans_notre_reseau(Ipv4Addr::new(192, 168, 73, 255), serveur));
        assert!(dans_notre_reseau(Ipv4Addr::new(192, 168, 73, 44), serveur));

        // Demander l'adresse du PC lui-même se solde par un refus.
        let brut =
            construire_reponse(&requete_avec_adresse_demandee(&[7; 6], serveur), serveur).unwrap();
        assert_eq!(type_de(&brut), MessageType::Nak);
    }

    /// Une demande sans adresse précise reste servie normalement : c'est le
    /// cas du tout premier téléphone, qui n'a encore rien à réclamer.
    #[test]
    fn une_demande_sans_adresse_precise_recoit_toujours_une_adresse() {
        let serveur = Ipv4Addr::new(192, 168, 73, 1);
        let brut = construire_reponse(
            &fabriquer_requete(MessageType::Request, &[1, 2, 3, 4, 5, 6]),
            serveur,
        )
        .unwrap();
        assert_eq!(type_de(&brut), MessageType::Ack);
        assert!(dans_notre_reseau(
            decoder_requete(&brut).unwrap().yiaddr(),
            serveur
        ));
    }

    /// Sans cette option, le téléphone doit DEVINER qu'un portail existe.
    /// Avec elle, on le lui dit. C'est la différence entre une page qui
    /// s'ouvre et une page qui ne s'ouvre qu'après une manipulation dans les
    /// réglages Wi-Fi — exactement ce qui était constaté en boutique.
    #[test]
    fn annonce_l_adresse_du_portail_a_chaque_telephone() {
        let serveur = Ipv4Addr::new(192, 168, 73, 1);

        for type_demande in [MessageType::Discover, MessageType::Request] {
            let brut =
                construire_reponse(&fabriquer_requete(type_demande, &[1, 2, 3, 4, 5, 6]), serveur)
                    .expect("une réponse est attendue");
            let reponse = decoder_requete(&brut).unwrap();

            match reponse.opts().get(OptionCode::CaptivePortal) {
                Some(DhcpOption::CaptivePortal(adresse)) => assert_eq!(
                    adresse, "http://192.168.73.1/",
                    "l'adresse annoncée doit être celle du portail, sur le port 80"
                ),
                autre => panic!("option de portail attendue, obtenu {autre:?}"),
            }
        }
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
