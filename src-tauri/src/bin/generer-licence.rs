//! Outil de secours du porteur du projet : génère une clé de licence sans
//! internet (en visite dans une boutique hors réseau, par exemple).
//!
//! En temps normal, les clés se génèrent depuis le tableau de bord en ligne.
//!
//! IMPORTANT — cet exécutable ne contient aucun secret. Pour signer, il faut
//! lui fournir la clé privée, qui reste la propriété du porteur du projet :
//!   - soit dans un fichier `cle-privee.txt` placé à côté de l'exe,
//!   - soit dans la variable d'environnement `CLE_PRIVEE_LICENCE`,
//!   - soit collée quand l'outil la demande.
//! Ce fichier ne doit jamais être laissé sur le PC d'un gérant, ni copié
//! dans le dépôt : qui l'a peut fabriquer des licences à vie.
//!
//! Usage : double-clic (détecte l'identifiant de ce PC), ou en ligne de
//! commande : generer-licence.exe <ID_MACHINE> [jours_valables=30]

use chrono::{Local, NaiveDate};
use ed25519_dalek::SigningKey;
use photocopie_benin_lib::license::{generer_cle, machine_id};
use std::io::{self, Write};

fn lire_ligne(invite: &str) -> String {
    print!("{invite}");
    let _ = io::stdout().flush();
    let mut ligne = String::new();
    let _ = io::stdin().read_line(&mut ligne);
    ligne.trim().to_string()
}

fn decoder_cle_privee(brut: &str) -> Option<SigningKey> {
    let brut: String = brut.chars().filter(|c| !c.is_whitespace()).collect();
    if brut.len() != 64 {
        return None;
    }
    let mut octets = [0u8; 32];
    for (i, morceau) in brut.as_bytes().chunks(2).enumerate() {
        let paire = std::str::from_utf8(morceau).ok()?;
        octets[i] = u8::from_str_radix(paire, 16).ok()?;
    }
    Some(SigningKey::from_bytes(&octets))
}

fn charger_cle_privee() -> Option<SigningKey> {
    let a_cote = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("cle-privee.txt")))
        .and_then(|p| std::fs::read_to_string(p).ok());
    if let Some(cle) = a_cote.as_deref().and_then(decoder_cle_privee) {
        return Some(cle);
    }

    if let Some(cle) = std::env::var("CLE_PRIVEE_LICENCE")
        .ok()
        .as_deref()
        .and_then(decoder_cle_privee)
    {
        return Some(cle);
    }

    println!("\nClé privée introuvable (ni cle-privee.txt à côté de cet outil,");
    println!("ni variable CLE_PRIVEE_LICENCE).");
    decoder_cle_privee(&lire_ligne("Collez la clé privée (64 caractères) : "))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let (id_machine, jours) = if let Some(id) = args.get(1) {
        let jours: i64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
        (id.clone(), jours)
    } else {
        println!("=== Génération d'une clé de licence — Gestion Photocopie ===\n");
        let id = machine_id();
        println!("Identifiant de ce PC : {id}");
        let reponse = lire_ligne("\nNombre de jours de validité (Entrée pour 30) : ");
        let jours: i64 = reponse.parse().unwrap_or(30);
        (id, jours)
    };

    let Some(cle_privee) = charger_cle_privee() else {
        println!("\nClé privée invalide : aucune clé de licence n'a été générée.");
        lire_ligne("Appuyez sur Entrée pour fermer...");
        return;
    };

    let expiration: NaiveDate = Local::now().date_naive() + chrono::Duration::days(jours);
    let cle = generer_cle(&cle_privee, &id_machine, expiration);

    println!("\nClé de licence : {cle}");
    println!("Valable jusqu'au : {}", expiration.format("%d/%m/%Y"));

    // Sans ça, une fenêtre ouverte par double-clic se fermerait aussitôt le
    // résultat affiché, avant que le porteur du projet ait pu le lire.
    println!();
    lire_ligne("Appuyez sur Entrée pour fermer...");
}
