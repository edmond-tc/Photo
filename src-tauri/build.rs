use std::path::Path;

fn main() {
    allegee("../client-web/index.html", "client.html");
    println!("cargo:rerun-if-changed=../client-web/index.html");
    tauri_build::build()
}

/// La page du client, allégée pour le téléphone : sans commentaires ni
/// indentation (les iPhone refusent un portail de plus de ~128 Ko). La source
/// (client-web/index.html) garde ses explications. Mêmes règles que
/// `alleger()` dans admin/site/relais.js, pour la page servie par Internet.
fn allegee(source: &str, sortie: &str) {
    let texte = std::fs::read_to_string(source).expect("page du client");
    let dossier = std::env::var("OUT_DIR").unwrap();
    std::fs::write(Path::new(&dossier).join(sortie), alleger(&texte)).unwrap();
}

fn alleger(texte: &str) -> String {
    // 1. Commentaires HTML.
    let mut sans = String::with_capacity(texte.len());
    let mut reste = texte;
    while let Some(debut) = reste.find("<!--") {
        sans.push_str(&reste[..debut]);
        match reste[debut..].find("-->") {
            Some(fin) => reste = &reste[debut + fin + 3..],
            None => { reste = ""; }
        }
    }
    sans.push_str(reste);
    // 2. Commentaires CSS, seulement entre <style> et </style> (ailleurs,
    //    « image/* » n'est pas un commentaire).
    let mut css = String::with_capacity(sans.len());
    let mut reste = sans.as_str();
    while let Some(debut) = reste.find("<style>") {
        let fin = reste[debut..].find("</style>").map(|f| debut + f).unwrap_or(reste.len());
        css.push_str(&reste[..debut]);
        let mut bloc = &reste[debut..fin];
        while let Some(d) = bloc.find("/*") {
            css.push_str(&bloc[..d]);
            bloc = match bloc[d..].find("*/") { Some(f) => &bloc[d + f + 2..], None => "" };
        }
        css.push_str(bloc);
        reste = &reste[fin..];
    }
    css.push_str(reste);
    // 3. Lignes : sans indentation, sans lignes vides ni lignes de commentaire.
    css.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}
