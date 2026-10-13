// La page du client, allégée pour le téléphone : sans commentaires ni
// indentation (les iPhone refusent un portail de plus de ~128 Ko). Mêmes
// règles que `alleger()` dans src-tauri/build.rs (page servie par le PC).
export function alleger(texte) {
  // 1. Commentaires HTML.
  let sans = texte.replace(/<!--[\s\S]*?(-->|$)/g, "");
  // 2. Commentaires CSS, seulement dans <style> (ailleurs, « image/* » n'est
  //    pas un commentaire).
  sans = sans.replace(/<style>[\s\S]*?(<\/style>|$)/g, (bloc) => bloc.replace(/\/\*[\s\S]*?(\*\/|$)/g, ""));
  // 3. Lignes : sans indentation, sans lignes vides ni lignes de commentaire.
  return sans
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith("//"))
    .join("\n");
}
