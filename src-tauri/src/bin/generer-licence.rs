//! Outil pour le porteur du projet : génère une clé de licence.
//!
//! Usage sur le terrain (aucun terminal à ouvrir) : double-clic sur l'exe
//! depuis l'Explorateur Windows, sur le PC du gérant lui-même — l'outil
//! détecte tout seul l'identifiant de cette machine, demande juste le
//! nombre de jours (Entrée = 30 par défaut), et affiche la clé.
//!
//! Usage avancé en ligne de commande, pour générer la clé d'un PC dont on
//! connaît déjà l'identifiant (ex: communiqué par téléphone) :
//!   generer-licence.exe <ID_MACHINE> [jours_valables=30]

use chrono::{Local, NaiveDate};
use photocopie_benin_lib::license::{generer_cle, machine_id};
use std::io::{self, Write};

fn lire_ligne(invite: &str) -> String {
    print!("{invite}");
    let _ = io::stdout().flush();
    let mut ligne = String::new();
    let _ = io::stdin().read_line(&mut ligne);
    ligne.trim().to_string()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Deux modes : ID machine passé en argument (usage avancé), ou détecté
    // automatiquement sur cette machine si l'exe est lancé sans argument
    // (double-clic depuis l'Explorateur, cas normal sur le terrain).
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

    let expiration: NaiveDate = Local::now().date_naive() + chrono::Duration::days(jours);
    let cle = generer_cle(&id_machine, expiration);

    println!("\nClé de licence : {cle}");
    println!("Valable jusqu'au : {}", expiration.format("%d/%m/%Y"));

    // Sans ça, une fenêtre ouverte par double-clic se fermerait aussitôt le
    // résultat affiché, avant que le porteur du projet ait pu le lire.
    println!();
    lire_ligne("Appuyez sur Entrée pour fermer...");
}
