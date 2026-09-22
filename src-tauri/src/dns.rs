//! Serveur DNS minimal : répond à TOUTE question par l'adresse du point
//! d'accès Wi-Fi local.
//!
//! C'est le mécanisme standard employé par le vrai Wi-Fi des hôtels et des
//! aéroports ("portail captif") : dès qu'un téléphone rejoint un Wi-Fi, il
//! essaie de joindre une adresse de contrôle connue (Google pour Android,
//! Apple pour iPhone) pour vérifier s'il a vraiment accès à internet. En
//! répondant à CETTE question — et à toutes les autres — par notre propre
//! adresse, le téléphone reçoit une page différente de celle attendue, en
//! déduit qu'il doit "se connecter" à ce réseau, et ouvre tout seul un
//! navigateur dessus. Sans ce serveur, le téléphone ne peut même pas tenter
//! cette vérification (aucun serveur ne répond au nom qu'il cherche à
//! joindre) et n'a alors aucune raison d'ouvrir quoi que ce soit lui-même.

use hickory_proto::op::MessageType;
use hickory_proto::rr::{rdata::A, RData, Record, RecordType};
use std::net::Ipv4Addr;
use tokio::net::UdpSocket;

const PORT_DNS: u16 = 53;

/// Tente de s'installer sur le port DNS. Rendu visible dans l'interface
/// plutôt que perdu dans un `eprintln!` : Windows peut déjà avoir son
/// propre relais DNS actif ailleurs sur la machine, auquel cas cette
/// tentative échoue en silence côté système — mais ne doit plus l'être côté
/// gérant, puisque c'est précisément ce serveur qui déclenche l'ouverture
/// automatique de la page.
///
/// Écoute sur TOUTES les cartes (`0.0.0.0`), comme le fait déjà `dhcp.rs` —
/// et pour la même raison : exiger une adresse précise (l'ancienne
/// approche) s'est montrée fragile sur le terrain. Windows peut mettre un
/// instant à rendre une adresse fraîchement attribuée réellement utilisable
/// pour un bind, et la détecter "trop tôt" fait échouer ce démarrage avec
/// une erreur système ("l'adresse demandée n'est pas valide dans son
/// contexte") — sans qu'aucun bug ne soit en cause côté adresse elle-même.
/// `0.0.0.0`, lui, n'a jamais échoué dans aucun essai sur le terrain,
/// exactement comme pour DHCP.
///
/// La restriction "ne répondre qu'aux téléphones du point d'accès" (pour ne
/// pas perturber le vrai réseau de la boutique si le PC y est aussi
/// branché) est maintenue — juste déplacée : ce n'est plus le système
/// d'exploitation qui la fait respecter au moment d'ouvrir le port, c'est
/// `servir` qui l'applique à chaque paquet reçu (voir `dans_le_bon_reseau`).
pub async fn demarrer(adresse: Ipv4Addr) -> Result<tauri::async_runtime::JoinHandle<()>, String> {
    let socket = UdpSocket::bind(format!("0.0.0.0:{PORT_DNS}"))
        .await
        .map_err(|e| {
            format!(
                "Serveur DNS local indisponible (port {PORT_DNS} : {e}) — l'ouverture \
                 automatique de la page chez le client ne fonctionnera pas, mais le Wi-Fi \
                 et l'ouverture manuelle du navigateur restent utilisables."
            )
        })?;

    Ok(tauri::async_runtime::spawn(servir(socket, adresse)))
}

/// L'expéditeur appartient-il au même réseau /24 que notre point d'accès ?
/// Seul un paquet dans ce cas obtient une réponse — les autres sont
/// ignorés en silence, comme s'ils avaient été reçus par un serveur DNS qui
/// n'écoutait que sur cette carte précise. Le résultat pour le reste du
/// réseau de la boutique est identique à l'ancienne approche (bind ciblé) ;
/// seule la façon de l'obtenir a changé.
fn dans_le_bon_reseau(expediteur: Ipv4Addr, adresse: Ipv4Addr) -> bool {
    expediteur.octets()[..3] == adresse.octets()[..3]
}

async fn servir(socket: UdpSocket, adresse: Ipv4Addr) {
    let mut tampon = [0u8; 512];
    loop {
        let (taille, expediteur) = match socket.recv_from(&mut tampon).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        let std::net::SocketAddr::V4(expediteur) = expediteur else {
            continue;
        };
        if !dans_le_bon_reseau(*expediteur.ip(), adresse) {
            continue;
        }
        if let Some(reponse) = construire_reponse(&tampon[..taille], adresse) {
            let _ = socket.send_to(&reponse, expediteur).await;
        }
    }
}

fn construire_reponse(requete_brute: &[u8], adresse: Ipv4Addr) -> Option<Vec<u8>> {
    use hickory_proto::op::Message;

    let requete = Message::from_vec(requete_brute).ok()?;
    if requete.metadata.message_type != MessageType::Query {
        return None;
    }
    let question = requete.queries.first()?.clone();

    let mut reponse = Message::response(requete.metadata.id, requete.metadata.op_code);
    reponse.metadata.authoritative = true;
    reponse.add_query(question.clone());

    // Seules les questions IPv4 (A) obtiennent une vraie réponse : renvoyer
    // "pas d'enregistrement" (plutôt que de fabriquer une fausse adresse
    // IPv6) aux questions AAAA fait basculer le téléphone sur IPv4 tout de
    // suite, sans attendre un délai d'expiration.
    if question.query_type() == RecordType::A {
        let enregistrement = Record::from_rdata(question.name().clone(), 60, RData::A(A(adresse)));
        reponse.add_answer(enregistrement);
    }

    reponse.to_vec().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::op::{Message, OpCode, Query};
    use hickory_proto::rr::Name;
    use std::str::FromStr;

    fn fabriquer_requete(nom: &str, type_question: RecordType) -> Vec<u8> {
        let mut requete = Message::query();
        requete.metadata.id = 4242;
        requete.metadata.op_code = OpCode::Query;
        requete.add_query(Query::query(Name::from_str(nom).unwrap(), type_question));
        requete.to_vec().unwrap()
    }

    #[test]
    fn repond_a_n_importe_quel_nom_de_domaine_par_notre_adresse() {
        // C'est tout l'intérêt : le téléphone demande un nom qu'on ne
        // connaît évidemment pas ("connectivitycheck.gstatic.com"), et doit
        // quand même recevoir notre adresse — jamais une absence de réponse.
        let adresse = Ipv4Addr::new(192, 168, 73, 1);
        let requete = fabriquer_requete("connectivitycheck.gstatic.com.", RecordType::A);

        let brut = construire_reponse(&requete, adresse).expect("une réponse est attendue");
        let reponse = Message::from_vec(&brut).expect("réponse mal formée");

        assert_eq!(reponse.metadata.id, 4242, "l'id doit être recopié tel quel");
        assert_eq!(reponse.metadata.message_type, MessageType::Response);
        assert_eq!(reponse.answers.len(), 1);
        match &reponse.answers[0].data {
            RData::A(A(ip)) => assert_eq!(*ip, adresse),
            autre => panic!("attendu un enregistrement A, obtenu {autre:?}"),
        }
    }

    #[test]
    fn ne_fabrique_pas_de_fausse_adresse_ipv6() {
        let adresse = Ipv4Addr::new(192, 168, 73, 1);
        let requete = fabriquer_requete("captive.apple.com.", RecordType::AAAA);

        let brut = construire_reponse(&requete, adresse).expect("une réponse est attendue");
        let reponse = Message::from_vec(&brut).expect("réponse mal formée");

        assert!(reponse.answers.is_empty(), "pas d'IPv6 disponible ici");
    }

    #[test]
    fn ignore_un_paquet_illisible_sans_planter() {
        assert!(construire_reponse(&[1, 2, 3], Ipv4Addr::new(192, 168, 73, 1)).is_none());
    }

    /// Le point qui remplace l'ancienne protection par bind ciblé : un
    /// appareil du VRAI réseau de la boutique (si le PC y est aussi
    /// branché par câble) ne doit jamais recevoir de réponse, sous peine de
    /// lui couper internet en détournant tous ses noms de domaine.
    #[test]
    fn ne_repond_qu_aux_appareils_du_meme_reseau_que_le_point_d_acces() {
        let point_acces = Ipv4Addr::new(192, 168, 73, 1);

        // Un téléphone qui a bien rejoint le point d'accès.
        assert!(dans_le_bon_reseau(Ipv4Addr::new(192, 168, 73, 42), point_acces));

        // Un appareil du réseau de la boutique (box sur un tout autre
        // sous-réseau) : jamais de réponse, jamais d'interférence.
        assert!(!dans_le_bon_reseau(Ipv4Addr::new(10, 0, 0, 5), point_acces));
        assert!(!dans_le_bon_reseau(
            Ipv4Addr::new(192, 168, 1, 5),
            point_acces
        ));
    }
}
