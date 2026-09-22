//! Réception locale via un routeur Wi-Fi dédié — plutôt qu'un réseau créé
//! par le PC lui-même (voir `hotspot.rs`, `wifi_direct.rs`).
//!
//! Sur un PC sans carte Wi-Fi, ou dont le pilote refuse les deux méthodes de
//! `hotspot.rs`, aucun logiciel ne peut créer de réseau : il faut une vraie
//! radio Wi-Fi. Un petit routeur bon marché (aucun abonnement, aucun accès
//! internet nécessaire — juste du courant) en fournit une, fiable sur
//! n'importe quel PC puisqu'elle ne dépend d'aucun pilote Windows.
//!
//! Contrairement au point d'accès du PC, ce n'est PAS notre application qui
//! crée le réseau : le routeur s'en charge, y compris la distribution des
//! adresses (son propre serveur DHCP, plus mature que le nôtre). Il ne
//! manque qu'une chose pour que la promesse "le client ne tape jamais rien"
//! tienne quand même : le tour de passe-passe qui ouvre la page toute seule
//! (voir `dns.rs`) exige que CE PC, et non le routeur, réponde aux
//! questions DNS des téléphones. D'où l'étape unique à faire une fois dans
//! les réglages du routeur (guide d'installation) : désigner l'adresse de
//! ce PC comme serveur DNS distribué par le routeur.

use std::net::Ipv4Addr;

/// L'adresse de ce PC sur le réseau du routeur.
///
/// Contrairement à `server::adresse_locale()` pour un PC 100% hors ligne
/// (où cette détection standard n'a aucune passerelle fiable pour se
/// repérer, et se trompe), un routeur présente ici une vraie passerelle :
/// `local_ip_address::local_ip()` la retrouve correctement.
pub fn adresse_sur_le_routeur() -> Option<Ipv4Addr> {
    // Même détection que pour le QR (voir `server::adresse_du_reseau_connecte`) :
    // les deux DOIVENT désigner la même carte, sinon le QR annonce une
    // adresse pendant que le serveur DNS en sert une autre — une variante du
    // bug d'adresses contradictoires déjà corrigé côté point d'accès.
    if let Some(adresse) = crate::server::adresse_du_reseau_connecte() {
        return Some(adresse);
    }
    match local_ip_address::local_ip() {
        Ok(std::net::IpAddr::V4(v4)) => Some(v4),
        _ => None,
    }
}

/// Démarre uniquement le serveur DNS local. Pas de DHCP ici : le routeur
/// s'en occupe déjà, bien mieux que notre implémentation minimale. Pas de
/// création de réseau non plus : il tourne déjà, indépendamment de nous.
pub async fn activer() -> Result<(Ipv4Addr, tauri::async_runtime::JoinHandle<()>), String> {
    let adresse = adresse_sur_le_routeur().ok_or_else(|| {
        "Ce PC ne semble pas relié au routeur (câble réseau débranché, ou routeur éteint)."
            .to_string()
    })?;
    let tache = crate::dns::demarrer(adresse).await?;
    Ok((adresse, tache))
}
