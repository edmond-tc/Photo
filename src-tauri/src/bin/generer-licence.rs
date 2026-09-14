//! Outil pour le porteur du projet : génère une clé de licence pour un
//! identifiant machine donné (affiché dans le panneau Réglages > Licence
//! de l'application côté gérant).
//!
//! Usage : cargo run --release --bin generer-licence -- <ID_MACHINE> [jours_valables]
//! Exemple : cargo run --release --bin generer-licence -- ABCD1234-... 30

use chrono::{Local, NaiveDate};
use photocopie_benin_lib::license::generer_cle;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(machine_id) = args.get(1) else {
        eprintln!("Usage: generer-licence <ID_MACHINE> [jours_valables=30]");
        std::process::exit(1);
    };

    let jours: i64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);

    let expiration: NaiveDate = Local::now().date_naive() + chrono::Duration::days(jours);
    let cle = generer_cle(machine_id, expiration);

    println!("Clé de licence : {cle}");
    println!("Valable jusqu'au : {}", expiration.format("%d/%m/%Y"));
}
