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
                 et le second QR restent utilisables.\n\nCAUSE LA PLUS FRÉQUENTE : une AUTRE \
                 COPIE de cette application tourne encore sur ce PC et retient le port. Regardez \
                 la liste « Qui écoute sur les ports » plus bas : si « photocopie-benin » y \
                 apparaît, c'est le cas. Remède : redémarrez le PC, puis rouvrez l'application \
                 UNE SEULE FOIS."
            )
        })?;

    Ok(tauri::async_runtime::spawn(servir(socket, adresse)))
}

/// Les noms que les téléphones ont RÉELLEMENT demandés à ce serveur.
///
/// Même raison que pour les adresses : c'est la seule preuve qu'un
/// téléphone nous parle. Voir un téléphone demander « captive.apple.com »
/// ou « connectivitycheck.gstatic.com » prouve que la chaîne tient jusque
/// là — et que ce qui suit ne dépend plus de nous.
static JOURNAL: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Assez grand pour qu'un téléphone bavard n'efface pas un téléphone
/// discret.
///
/// Défaut constaté aussitôt après la mise en service, et il a produit une
/// conclusion fausse : un Android pose une trentaine de questions
/// différentes en quelques secondes (ses applications se réveillent toutes
/// en même temps), là où un iPhone en pose deux ou trois. Avec une limite
/// de 25 lignes, l'Android chassait entièrement l'iPhone du journal — et on
/// en déduisait qu'il ne demandait rien, alors qu'il avait demandé et été
/// servi. La mesure disait le contraire de la vérité.
const MAX_JOURNAL: usize = 80;

/// Les noms demandés, le plus ancien d'abord.
pub fn journal() -> Vec<String> {
    JOURNAL.lock().map(|j| j.clone()).unwrap_or_default()
}

/// Un téléphone qui nous parle depuis un réseau que nous refusons.
///
/// C'est le contraire d'un détail : ce téléphone A BIEN rejoint notre
/// Wi-Fi et nous pose bien ses questions, mais il tient son adresse d'un
/// autre serveur — le partage de connexion de Windows, le plus souvent, qui
/// distribue du 192.168.137.x. Nous ne lui répondons pas, il conclut
/// « pas d'internet », et la page ne s'ouvre jamais d'elle-même.
///
/// Jusqu'ici ce rejet ne laissait aucune trace. Le journal montrait un
/// serveur de noms qui « ne reçoit rien », alors qu'il recevait et jetait.
#[cfg(test)]
fn vider_journal() {
    if let Ok(mut journal) = JOURNAL.lock() {
        journal.clear();
    }
}

fn noter_refus(expediteur: Ipv4Addr, nom: &str, notre_adresse: Ipv4Addr) {
    if let Ok(mut journal) = JOURNAL.lock() {
        let ligne = format!(
            "⛔ {expediteur} demande {nom} — REFUSÉ : ce téléphone a reçu son adresse \
             d'un AUTRE serveur (le nôtre donne du {}.x). Coupez le partage de \
             connexion Windows, puis réactivez le Wi-Fi de la boutique.",
            notre_adresse
                .octets()
                .iter()
                .take(3)
                .map(|o| o.to_string())
                .collect::<Vec<_>>()
                .join(".")
        );
        if journal.iter().any(|existante| existante == &ligne) {
            return;
        }
        if journal.len() >= MAX_JOURNAL {
            journal.remove(0);
        }
        journal.push(ligne);
    }
}

fn noter(expediteur: Ipv4Addr, nom: &str) {
    if let Ok(mut journal) = JOURNAL.lock() {
        // Les questions de vérification de réseau sont signalées : ce sont
        // les seules qui décident de l'ouverture de la page, et elles se
        // perdent sinon au milieu des dizaines d'autres.
        let marque = if nom.contains("captive.apple")
            || nom.contains("connectivitycheck")
            || nom.contains("gstatic")
            || nom.contains("msftconnecttest")
        {
            "  ⭐ VÉRIFICATION DE RÉSEAU"
        } else {
            ""
        };
        let ligne = format!("{expediteur} demande {nom}{marque}");
        // Chaque couple (téléphone, nom) n'apparaît qu'UNE fois, où qu'il
        // soit déjà dans la liste — et non plus seulement s'il vient d'être
        // noté. Un téléphone réessaie le même nom des dizaines de fois : ne
        // comparer qu'à la dernière ligne laissait repasser la même question
        // dès qu'une autre s'était glissée entre deux, et le journal se
        // remplissait de répétitions au détriment des autres appareils.
        if journal.iter().any(|existante| existante == &ligne) {
            return;
        }
        if journal.len() >= MAX_JOURNAL {
            journal.remove(0);
        }
        journal.push(ligne);
    }
}

/// Le serveur répond-il VRAIMENT ?
///
/// « Démarré » n'est pas « répond » : le port peut être pris, la réponse
/// peut être rejetée, le filtre de sous-réseau peut écarter l'appelant. Sur
/// le terrain, l'écran affichait « ✅ noms de domaine » pendant que les
/// téléphones n'obtenaient rien — et plusieurs déplacements ont été perdus
/// à chercher ailleurs.
///
/// On pose donc au serveur la question exacte que pose un iPhone en
/// rejoignant un réseau, depuis l'adresse du point d'accès lui-même pour
/// passer le même filtre qu'un téléphone du réseau, et on vérifie que la
/// réponse désigne bien ce PC.
pub async fn repond(adresse: Ipv4Addr) -> bool {
    use hickory_proto::op::{Message, OpCode, Query};
    use hickory_proto::rr::Name;
    use std::str::FromStr;

    let Ok(socket) = UdpSocket::bind(std::net::SocketAddr::from((adresse, 0))).await else {
        return false;
    };

    let mut requete = Message::query();
    requete.metadata.id = 0x4242;
    requete.metadata.op_code = OpCode::Query;
    requete.metadata.recursion_desired = true;
    let Ok(nom) = Name::from_str("captive.apple.com.") else {
        return false;
    };
    requete.add_query(Query::query(nom, RecordType::A));
    let Ok(brut) = requete.to_vec() else {
        return false;
    };

    if socket.send_to(&brut, (adresse, PORT_DNS)).await.is_err() {
        return false;
    }

    let mut tampon = [0u8; 512];
    let attente =
        tokio::time::timeout(std::time::Duration::from_secs(2), socket.recv(&mut tampon)).await;
    let Ok(Ok(taille)) = attente else {
        return false;
    };

    Message::from_vec(&tampon[..taille])
        .ok()
        .is_some_and(|reponse| {
            reponse.answers.iter().any(
                |enregistrement| matches!(&enregistrement.data, RData::A(A(ip)) if *ip == adresse),
            )
        })
}

/// L'expéditeur a-t-il droit à une réponse ?
///
/// Le filtre d'origine n'acceptait que le même réseau /24 que notre point
/// d'accès, pour ne pas détourner les noms de toute la boutique quand le PC
/// est aussi branché au réseau du patron. La règle reste, mais elle avait
/// deux angles morts, et tous deux produisent la panne constatée : rien ne
/// se déclenche à l'arrivée sur le réseau, alors que tout marche si l'on
/// entre ensuite dans les réglages Wi-Fi.
///
/// 1. Un téléphone qui n'a pas ENCORE son adresse. Entre le moment où il
///    rejoint le réseau et celui où le bail lui parvient, il peut émettre
///    depuis 0.0.0.0 ou depuis une adresse qu'il s'est attribuée lui-même
///    (169.254.x.x). Sa toute première vérification de réseau tombe
///    précisément dans cette fenêtre — et c'est celle qui décide de
///    l'ouverture de la page.
///
/// 2. Un téléphone servi par un AUTRE serveur d'adresses. Le partage de
///    connexion de Windows distribue du 192.168.137.x, et il a été vu en
///    train de tourner sur la machine d'essai. Un téléphone qui reçoit son
///    adresse de lui parle bien à notre serveur de noms, mais depuis un
///    réseau que nous rejetons.
///
/// Le cas 2 reste refusé — y répondre reviendrait à détourner les noms
/// d'un réseau qui n'est pas le nôtre. Mais il n'est plus refusé EN
/// SILENCE : il s'écrit dans le journal, parce qu'une panne qu'on ne voit
/// pas est une panne qu'on cherche ailleurs pendant des jours.
fn dans_le_bon_reseau(expediteur: Ipv4Addr, adresse: Ipv4Addr) -> bool {
    if expediteur.octets()[..3] == adresse.octets()[..3] {
        return true;
    }
    // Sans adresse encore attribuée : c'est forcément un téléphone qui
    // vient de rejoindre NOTRE réseau, puisque le paquet est arrivé sur
    // cette carte.
    expediteur.is_unspecified() || expediteur.octets()[..2] == [169, 254]
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
        let admis = dans_le_bon_reseau(*expediteur.ip(), adresse);
        if let Ok(demande) = hickory_proto::op::Message::from_vec(&tampon[..taille]) {
            if let Some(question) = demande.queries.first() {
                if admis {
                    noter(*expediteur.ip(), &question.name().to_string());
                } else {
                    noter_refus(*expediteur.ip(), &question.name().to_string(), adresse);
                }
            }
        }
        if !admis {
            continue;
        }
        if let Some(reponse) = construire_reponse(&tampon[..taille], adresse) {
            let _ = socket.send_to(&reponse, expediteur).await;
        }
    }
}

/// Les adresses du Relais privé iCloud, les seules auxquelles ce serveur
/// refuse de répondre.
///
/// Tout le reste reçoit notre adresse : c'est ainsi qu'un téléphone
/// découvre le portail. Mais pour celles-ci, répondre est précisément ce
/// qui casse tout.
///
/// Le Relais privé fait passer TOUT le trafic de l'iPhone par deux relais
/// d'Apple, sur internet. Sur un réseau sans internet — le nôtre — il ne
/// peut pas les joindre. Or comme notre serveur répondait « 192.168.73.1 »
/// à ces noms comme à tous les autres, l'iPhone croyait le relais
/// disponible, tentait d'y monter son tunnel, recevait notre page HTML à la
/// place, et Safari restait bloqué. Constaté en boutique : le second QR
/// n'ouvre rien sur iPhone, alors qu'il marche sur Android — qui n'a pas de
/// relais privé.
///
/// Refuser le nom (NXDOMAIN) est la méthode qu'Apple prescrit elle-même aux
/// gestionnaires de réseau : l'iPhone en conclut que le relais n'est pas
/// disponible ici, le désactive pour ce réseau, et parle de nouveau
/// directement — donc à nous.
const RELAIS_PRIVE_APPLE: &[&str] = &[
    "mask.icloud.com",
    "mask-h2.icloud.com",
    "mask-api.icloud.com",
];

fn est_relais_prive_apple(nom: &str) -> bool {
    let nom = nom.trim_end_matches('.').to_ascii_lowercase();
    RELAIS_PRIVE_APPLE.contains(&nom.as_str())
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
    // Drapeaux recopiés de la demande : un téléphone qui a demandé la
    // récursion attend qu'on le lui confirme. Une réponse sans ces marques
    // est jetée par les résolveurs stricts — Android en tête — et le
    // téléphone conclut « pas d'internet » au lieu de « portail à ouvrir ».
    reponse.metadata.recursion_desired = requete.metadata.recursion_desired;
    reponse.metadata.recursion_available = true;
    reponse.add_query(question.clone());

    // Le Relais privé d'Apple est la seule chose à qui l'on REFUSE une
    // adresse. Voir `EST_RELAIS_PRIVE_APPLE`.
    if est_relais_prive_apple(&question.name().to_string()) {
        reponse.metadata.response_code = hickory_proto::op::ResponseCode::NXDomain;
        return reponse.to_vec().ok();
    }

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
        // Ce que pose tout téléphone réel : « je veux que tu cherches pour
        // moi ». Ne pas le reproduire ici masquerait justement le défaut de
        // drapeaux que ce fichier vient de corriger.
        requete.metadata.recursion_desired = true;
        requete.add_query(Query::query(Name::from_str(nom).unwrap(), type_question));
        requete.to_vec().unwrap()
    }

    /// La toute première vérification de réseau d'un téléphone peut partir
    /// AVANT que le bail lui soit parvenu — depuis 0.0.0.0 ou depuis une
    /// adresse qu'il s'est attribuée lui-même. C'est elle qui décide de
    /// l'ouverture de la page : la refuser, c'est garantir que rien ne
    /// s'ouvrira à l'arrivée sur le réseau.
    #[test]
    fn repond_au_telephone_qui_n_a_pas_encore_son_adresse() {
        let nous = Ipv4Addr::new(192, 168, 73, 1);
        assert!(dans_le_bon_reseau(Ipv4Addr::new(0, 0, 0, 0), nous));
        assert!(dans_le_bon_reseau(Ipv4Addr::new(169, 254, 12, 34), nous));
    }

    /// Un téléphone servi par un AUTRE serveur d'adresses reste refusé —
    /// lui répondre détournerait les noms d'un réseau qui n'est pas le
    /// nôtre. Mais ce refus doit laisser une trace : sans elle, le journal
    /// montre un serveur qui « ne reçoit rien » alors qu'il reçoit et jette.
    #[test]
    fn refuse_un_autre_reseau_mais_le_consigne() {
        let nous = Ipv4Addr::new(192, 168, 73, 1);
        let intrus = Ipv4Addr::new(192, 168, 137, 45);
        assert!(!dans_le_bon_reseau(intrus, nous));

        vider_journal();
        noter_refus(intrus, "captive.apple.com.", nous);
        let journal = journal();
        assert_eq!(journal.len(), 1);
        assert!(journal[0].contains("REFUSÉ"), "{}", journal[0]);
        assert!(journal[0].contains("192.168.137.45"), "{}", journal[0]);
        assert!(journal[0].contains("192.168.73.x"), "{}", journal[0]);
        vider_journal();
    }

    /// Les trois seuls noms auxquels ce serveur doit REFUSER une adresse.
    ///
    /// C'est le remède prescrit par Apple aux gestionnaires de réseau, et
    /// il répare un symptôme constaté en boutique : le second QR n'ouvre
    /// rien sur iPhone alors qu'il fonctionne sur Android. Répondre à ces
    /// noms fait croire à l'iPhone que le Relais privé est utilisable ici,
    /// il tente d'y faire passer tout son trafic, et Safari se bloque.
    #[test]
    fn refuse_le_relais_prive_apple() {
        let adresse = Ipv4Addr::new(192, 168, 73, 1);
        for nom in [
            "mask.icloud.com.",
            "mask-h2.icloud.com.",
            "mask-api.icloud.com.",
        ] {
            let requete = fabriquer_requete(nom, RecordType::A);
            let brut = construire_reponse(&requete, adresse).expect("une réponse est attendue");
            let reponse = Message::from_vec(&brut).unwrap();
            assert_eq!(
                reponse.metadata.response_code,
                hickory_proto::op::ResponseCode::NXDomain,
                "{nom} doit être refusé"
            );
            assert!(
                reponse.answers.is_empty(),
                "{nom} ne doit porter aucune adresse"
            );
        }
    }

    /// La casse et le point final viennent du téléphone, pas de nous.
    #[test]
    fn reconnait_le_relais_prive_quelle_que_soit_l_ecriture() {
        assert!(est_relais_prive_apple("MASK.ICLOUD.COM."));
        assert!(est_relais_prive_apple("mask-h2.icloud.com"));
        // Et surtout : rien d'autre ne doit être refusé, sans quoi le
        // téléphone ne découvrirait plus le portail.
        assert!(!est_relais_prive_apple("captive.apple.com."));
        assert!(!est_relais_prive_apple("gateway.icloud.com."));
        assert!(!est_relais_prive_apple("icloud.com."));
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

    /// Un téléphone n'accepte une réponse DNS que si elle lui ressemble :
    /// l'identifiant recopié, la question renvoyée, et surtout les drapeaux
    /// cohérents avec sa demande. Android est particulièrement strict —
    /// une réponse mal pavoisée est jetée en silence, et le téléphone
    /// conclut alors « ce réseau n'a pas internet » au lieu de « ce réseau
    /// demande une connexion ». Aucune page ne s'ouvre, et rien ne le dit.
    #[test]
    fn la_reponse_porte_les_drapeaux_attendus_par_un_telephone() {
        let adresse = Ipv4Addr::new(192, 168, 73, 1);
        let requete = fabriquer_requete("captive.apple.com.", RecordType::A);
        let brut = construire_reponse(&requete, adresse).expect("une réponse est attendue");
        let reponse = Message::from_vec(&brut).expect("réponse mal formée");

        assert_eq!(reponse.metadata.message_type, MessageType::Response);
        assert_eq!(reponse.metadata.id, 4242);
        assert_eq!(
            reponse.metadata.response_code,
            hickory_proto::op::ResponseCode::NoError,
            "un code d'erreur ferait conclure au téléphone que le nom n'existe pas"
        );
        assert!(
            reponse.metadata.recursion_desired,
            "le drapeau « récursion demandée » du client doit être recopié tel quel"
        );
        assert!(
            reponse.metadata.recursion_available,
            "sans « récursion disponible », un résolveur strict jette la réponse"
        );
        assert_eq!(reponse.queries.len(), 1, "la question doit être renvoyée");
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
        assert!(dans_le_bon_reseau(
            Ipv4Addr::new(192, 168, 73, 42),
            point_acces
        ));

        // Un appareil du réseau de la boutique (box sur un tout autre
        // sous-réseau) : jamais de réponse, jamais d'interférence.
        assert!(!dans_le_bon_reseau(Ipv4Addr::new(10, 0, 0, 5), point_acces));
        assert!(!dans_le_bon_reseau(
            Ipv4Addr::new(192, 168, 1, 5),
            point_acces
        ));
    }
}
