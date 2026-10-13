//! Pays de la boutique : monnaie affichée et indicatif téléphonique (pour
//! écrire au client sur WhatsApp). Le Bénin par défaut. Les prix restent
//! ceux que le gérant saisit, dans sa monnaie.

pub struct Pays {
    pub code: &'static str,
    pub nom: &'static str,
    pub name: &'static str,
    pub devise: &'static str,
    pub indicatif: &'static str,
}

const fn p(code: &'static str, nom: &'static str, name: &'static str, devise: &'static str, indicatif: &'static str) -> Pays {
    Pays { code, nom, name, devise, indicatif }
}

pub const PAYS: &[Pays] = &[
    p("BJ", "Bénin", "Benin", "FCFA", "229"),
    p("BF", "Burkina Faso", "Burkina Faso", "FCFA", "226"),
    p("CI", "Côte d'Ivoire", "Côte d'Ivoire", "FCFA", "225"),
    p("ML", "Mali", "Mali", "FCFA", "223"),
    p("NE", "Niger", "Niger", "FCFA", "227"),
    p("SN", "Sénégal", "Senegal", "FCFA", "221"),
    p("TG", "Togo", "Togo", "FCFA", "228"),
    p("CM", "Cameroun", "Cameroon", "FCFA", "237"),
    p("GA", "Gabon", "Gabon", "FCFA", "241"),
    p("CG", "Congo", "Congo", "FCFA", "242"),
    p("TD", "Tchad", "Chad", "FCFA", "235"),
    p("CF", "Centrafrique", "Central African Republic", "FCFA", "236"),
    p("GN", "Guinée", "Guinea", "GNF", "224"),
    p("CD", "RD Congo", "DR Congo", "FC", "243"),
    p("MA", "Maroc", "Morocco", "DH", "212"),
    p("DZ", "Algérie", "Algeria", "DA", "213"),
    p("TN", "Tunisie", "Tunisia", "DT", "216"),
    p("GH", "Ghana", "Ghana", "GH₵", "233"),
    p("NG", "Nigeria", "Nigeria", "₦", "234"),
    p("KE", "Kenya", "Kenya", "KSh", "254"),
    p("RW", "Rwanda", "Rwanda", "FRW", "250"),
    p("MG", "Madagascar", "Madagascar", "Ar", "261"),
    p("FR", "France", "France", "€", "33"),
];

pub fn pays(code: Option<&str>) -> &'static Pays {
    code.and_then(|c| PAYS.iter().find(|p| p.code.eq_ignore_ascii_case(c.trim()))).unwrap_or(&PAYS[0])
}

/// Numéro écrit par le client (« 01 97 12 34 56 », « +229 0197123456 »,
/// « 00229… ») → chiffres internationaux pour un lien WhatsApp
/// (« 2290197123456 »). `None` si ce n'est pas un numéro.
pub fn numero_international(saisi: &str, indicatif: &str) -> Option<String> {
    let t = saisi.trim();
    let chiffres: String = t.chars().filter(char::is_ascii_digit).collect();
    if chiffres.len() < 8 || chiffres.len() > 15 {
        return None;
    }
    let international = if t.starts_with('+') {
        chiffres
    } else if let Some(reste) = chiffres.strip_prefix("00") {
        reste.to_string()
    } else if chiffres.starts_with(indicatif) && chiffres.len() > indicatif.len() + 8 {
        chiffres
    } else if indicatif != "229" && chiffres.starts_with('0') {
        // Ailleurs qu'au Bénin, le 0 de tête ne se compose pas depuis
        // l'étranger (Côte d'Ivoire exceptée : il fait partie du numéro).
        if indicatif == "225" { format!("{indicatif}{chiffres}") } else { format!("{indicatif}{}", &chiffres[1..]) }
    } else {
        // Bénin : les numéros à 10 chiffres commencent par 01, qui se garde.
        format!("{indicatif}{chiffres}")
    };
    (international.len() <= 15).then_some(international)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benin_par_defaut_et_code_inconnu() {
        assert_eq!(pays(None).code, "BJ");
        assert_eq!(pays(Some("zz")).devise, "FCFA");
        assert_eq!(pays(Some("gh")).devise, "GH₵");
    }

    #[test]
    fn numeros_ecrits_de_toutes_les_facons() {
        assert_eq!(numero_international("01 97 12 34 56", "229").as_deref(), Some("2290197123456"));
        assert_eq!(numero_international("+229 01 97 12 34 56", "229").as_deref(), Some("2290197123456"));
        assert_eq!(numero_international("00229 0197123456", "229").as_deref(), Some("2290197123456"));
        assert_eq!(numero_international("2290197123456", "229").as_deref(), Some("2290197123456"));
        assert_eq!(numero_international("024 123 4567", "233").as_deref(), Some("233241234567"));
        assert_eq!(numero_international("0803 123 4567", "234").as_deref(), Some("2348031234567"));
        assert_eq!(numero_international("07 07 12 34 56", "225").as_deref(), Some("2250707123456"));
        assert_eq!(numero_international("77 123 45 67", "221").as_deref(), Some("221771234567"));
        assert_eq!(numero_international("06 12 34 56 78", "33").as_deref(), Some("33612345678"));
        assert_eq!(numero_international("123", "229"), None);
        assert_eq!(numero_international("Awa", "229"), None);
    }
}
