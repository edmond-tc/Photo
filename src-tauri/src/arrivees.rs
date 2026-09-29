//! Un téléphone vient de rejoindre le Wi-Fi du PC, mais sa page d'envoi ne
//! s'est pas ouverte : le PC le remarque et prévient le gérant, qui tourne
//! l'écran vers le client (code de secours).
//!
//! Aucun réseau ne peut obliger un téléphone à ouvrir une page : selon le
//! modèle, elle s'ouvre seule, ou via la notification « Se connecter au
//! réseau », ou pas du tout. Ce dernier cas laissait le client bloqué sans
//! que personne le sache. Le PC le voit maintenant : le téléphone lui parle
//! (adresse demandée, questions de réseau) mais ne charge jamais la page.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Le temps laissé au téléphone pour ouvrir la page tout seul, ou au client
/// pour toucher la notification.
const ATTENTE: Duration = Duration::from_secs(20);

/// Un téléphone oublié depuis longtemps est un nouveau client s'il revient.
const OUBLI: Duration = Duration::from_secs(15 * 60);

#[derive(Debug)]
struct Arrivee {
    depuis: Instant,
    vu_la_derniere_fois: Instant,
    page: bool,
    signale: bool,
}

static ARRIVEES: Mutex<Option<HashMap<Ipv4Addr, Arrivee>>> = Mutex::new(None);

fn avec<R>(f: impl FnOnce(&mut HashMap<Ipv4Addr, Arrivee>) -> R) -> Option<R> {
    let mut garde = ARRIVEES.lock().ok()?;
    Some(f(garde.get_or_insert_with(HashMap::new)))
}

fn ecarte(ip: Ipv4Addr) -> bool {
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_broadcast()
        || crate::hotspot::adresse_point_acces_active() == Some(ip)
}

/// Le téléphone parle au PC (adresse demandée, question de réseau…).
pub fn vu(ip: Ipv4Addr) {
    vu_a(ip, Instant::now());
}

fn vu_a(ip: Ipv4Addr, maintenant: Instant) {
    if ecarte(ip) {
        return;
    }
    avec(|m| {
        let a = m.entry(ip).or_insert(Arrivee {
            depuis: maintenant,
            vu_la_derniere_fois: maintenant,
            page: false,
            signale: false,
        });
        a.vu_la_derniere_fois = maintenant;
    });
}

/// Sa page d'envoi est ouverte (ou l'application l'a trouvé) : rien à faire.
pub fn page_ouverte(ip: Ipv4Addr) {
    if ecarte(ip) {
        return;
    }
    let maintenant = Instant::now();
    avec(|m| {
        let a = m.entry(ip).or_insert(Arrivee {
            depuis: maintenant,
            vu_la_derniere_fois: maintenant,
            page: true,
            signale: false,
        });
        a.page = true;
        a.vu_la_derniere_fois = maintenant;
    });
}

/// Les téléphones à signaler maintenant, une seule fois chacun.
fn a_signaler(maintenant: Instant) -> Vec<Ipv4Addr> {
    avec(|m| {
        m.retain(|_, a| maintenant.duration_since(a.vu_la_derniere_fois) < OUBLI);
        let mut liste = Vec::new();
        for (ip, a) in m.iter_mut() {
            if !a.page && !a.signale && maintenant.duration_since(a.depuis) >= ATTENTE {
                a.signale = true;
                liste.push(*ip);
            }
        }
        liste
    })
    .unwrap_or_default()
}

/// Surveille en arrière-plan et prévient l'écran du gérant.
pub fn demarrer(app: tauri::AppHandle) {
    use tauri::Emitter;
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(3));
        if !crate::hotspot::point_acces_actif() {
            continue;
        }
        for ip in a_signaler(Instant::now()) {
            let _ = app.emit("telephone-sans-page", ip.to_string());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signale_une_fois_le_telephone_qui_n_ouvre_pas_la_page() {
        let t0 = Instant::now();
        let ip = Ipv4Addr::new(192, 168, 137, 41);
        let autre = Ipv4Addr::new(192, 168, 137, 42);
        vu_a(ip, t0);
        vu_a(autre, t0);
        page_ouverte(autre);
        assert!(a_signaler(t0 + Duration::from_secs(5)).is_empty());
        let liste = a_signaler(t0 + ATTENTE + Duration::from_secs(1));
        assert!(liste.contains(&ip));
        assert!(!liste.contains(&autre));
        assert!(!a_signaler(t0 + ATTENTE + Duration::from_secs(4)).contains(&ip));
    }
}
