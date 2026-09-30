//! Reconnaître une mise à jour officielle de Gestion Photocopie.
//!
//! Le build signe chaque installateur (voir scripts/signer-mise-a-jour.mjs)
//! avec une clé que seul le porteur du projet détient (secret GitHub
//! CLE_SIGNATURE_MAJ). La signature est ajoutée à la fin du fichier, qui
//! s'installe exactement comme avant.
//!
//! C'est le CONTENU qui compte, plus le chemin : une vraie mise à jour
//! s'installe qu'elle arrive par WhatsApp, Bluetooth, câble ou clé USB, et
//! un programme non signé n'est lancé par aucun chemin, clé USB comprise.
//! Aucune connexion n'est nécessaire : la clé publique est dans le logiciel.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

/// Clé publique des mises à jour (la clé privée n'est jamais dans le dépôt).
const CLE_PUBLIQUE_MAJ: [u8; 32] = [
    0x0b, 0x99, 0x72, 0x34, 0x48, 0x67, 0xff, 0x43, 0x4a, 0x90, 0x9a, 0xe7, 0xa1, 0x19, 0xd3, 0x96,
    0x3b, 0x8f, 0x71, 0x9b, 0x70, 0x0a, 0xab, 0x9b, 0x1b, 0x86, 0x1e, 0x09, 0xf7, 0x2a, 0xaa, 0x71,
];

const MARQUE: &[u8; 8] = b"KQMAJ001";
/// Version (16) + signature (64) + marque (8).
const TAILLE_FIN: u64 = 16 + 64 + 8;

/// Une mise à jour officielle, avec la version qu'elle installe.
pub fn verifier(chemin: &Path) -> Option<String> {
    verifier_avec(&CLE_PUBLIQUE_MAJ, chemin)
}

fn verifier_avec(cle: &[u8; 32], chemin: &Path) -> Option<String> {
    let cle = VerifyingKey::from_bytes(cle).ok()?;
    let mut fichier = std::fs::File::open(chemin).ok()?;
    let taille = fichier.metadata().ok()?.len();
    if taille <= TAILLE_FIN {
        return None;
    }
    let corps = taille - TAILLE_FIN;

    let mut fin = [0u8; TAILLE_FIN as usize];
    fichier.seek(SeekFrom::Start(corps)).ok()?;
    fichier.read_exact(&mut fin).ok()?;
    if &fin[80..] != MARQUE {
        return None;
    }
    let version: String = fin[..16]
        .iter()
        .take_while(|&&o| o != 0)
        .map(|&o| o as char)
        .collect();
    if version.is_empty()
        || !version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return None;
    }
    let signature = Signature::from_bytes(fin[16..80].try_into().ok()?);

    // L'empreinte du fichier d'origine, lu par morceaux : l'installateur
    // complet pèse plus de 200 Mo.
    fichier.seek(SeekFrom::Start(0)).ok()?;
    let mut empreinte = Sha256::new();
    let mut reste = corps;
    let mut tampon = vec![0u8; 1 << 16];
    while reste > 0 {
        let n = (reste as usize).min(tampon.len());
        fichier.read_exact(&mut tampon[..n]).ok()?;
        empreinte.update(&tampon[..n]);
        reste -= n as u64;
    }
    let empreinte: String = empreinte
        .finalize()
        .iter()
        .map(|o| format!("{o:02x}"))
        .collect();

    let message = format!("GESTION-PHOTOCOPIE-MAJ|{version}|{empreinte}");
    cle.verify_strict(message.as_bytes(), &signature).ok()?;
    Some(version)
}

/// Extensions qu'un double-clic exécuterait : jamais lancées sans signature.
pub fn est_programme(chemin: &Path) -> bool {
    const PROGRAMMES: [&str; 14] = [
        "exe", "msi", "bat", "cmd", "com", "scr", "pif", "ps1", "vbs", "vbe", "js", "jse", "hta",
        "lnk",
    ];
    extension_reelle(chemin).is_some_and(|e| PROGRAMMES.contains(&e.as_str()))
}

/// L'extension que Windows retiendra : en minuscules, après avoir ôté les
/// points et espaces de fin qu'il efface lui-même (« virus.exe. » est un
/// .exe pour Windows, pas un fichier sans extension).
pub fn extension_reelle(chemin: &Path) -> Option<String> {
    let nom = chemin.file_name()?.to_string_lossy();
    let nom = nom.trim_end_matches(['.', ' ']);
    let (_, extension) = nom.rsplit_once('.')?;
    (!extension.is_empty()).then(|| extension.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    /// Fichier signé par scripts/signer-mise-a-jour.mjs avec la clé d'essai
    /// [7; 32] : prouve que le build et le PC calculent la même chose.
    const ESSAI: &[u8] = include_bytes!("essais/maj-signee-essai.bin");

    fn cle_essai() -> [u8; 32] {
        SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes()
    }

    fn ecrire(octets: &[u8]) -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(octets).unwrap();
        f
    }

    #[test]
    fn reconnait_un_installateur_signe_par_le_build() {
        let f = ecrire(ESSAI);
        assert_eq!(verifier_avec(&cle_essai(), f.path()).as_deref(), Some("9.9.9"));
    }

    #[test]
    fn refuse_une_autre_cle() {
        let f = ecrire(ESSAI);
        assert_eq!(verifier(f.path()), None);
    }

    #[test]
    fn refuse_un_installateur_modifie() {
        let mut octets = ESSAI.to_vec();
        octets[5] ^= 1;
        let f = ecrire(&octets);
        assert_eq!(verifier_avec(&cle_essai(), f.path()), None);
    }

    #[test]
    fn refuse_une_version_changee() {
        let mut octets = ESSAI.to_vec();
        let debut = octets.len() - TAILLE_FIN as usize;
        octets[debut] = b'8';
        let f = ecrire(&octets);
        assert_eq!(verifier_avec(&cle_essai(), f.path()), None);
    }

    #[test]
    fn refuse_un_programme_sans_signature() {
        let f = ecrire(&ESSAI[..ESSAI.len() - TAILLE_FIN as usize]);
        assert_eq!(verifier_avec(&cle_essai(), f.path()), None);
        let vide = ecrire(b"");
        assert_eq!(verifier_avec(&cle_essai(), vide.path()), None);
    }

    #[test]
    fn repere_les_programmes() {
        assert!(est_programme(Path::new("Mise_a_jour.EXE")));
        assert!(est_programme(Path::new("facture.pdf.lnk")));
        assert!(!est_programme(Path::new("cv.pdf")));
        assert!(!est_programme(Path::new("sans-extension")));
        assert!(est_programme(Path::new("virus.exe.")));
        assert!(est_programme(Path::new("virus.EXE . ")));
    }
}
