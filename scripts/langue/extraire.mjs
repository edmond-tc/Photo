// Extrait les textes français visibles d'une interface (HTML + JavaScript),
// pour fabriquer ou vérifier un dictionnaire de traduction.
//
//   node scripts/langue/extraire.mjs client-web/index.html > textes.json
//   node scripts/langue/extraire.mjs src/index.html src/main.js > textes.json
//
// Sortie : { exacts: [...], motifs: [...] } — un motif est un texte avec des
// parties variables, notées {1}, {2}… (ex. « {1} documents reçus »).
import fs from "node:fs";
import { execSync } from "node:child_process";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const ts = require(`${execSync("npm root -g").toString().trim()}/typescript`);

const exacts = new Set();
const motifs = new Set();
const VAR = "\u0000";

// Un texte « français visible » : des lettres, et une espace, un accent ou
// une majuscule en tête — pas une clé (« kiosque.profil »), un sélecteur,
// une adresse ou du code.
function estTexte(t) {
  const c = t.trim();
  if (c.length < 2 || !/[A-Za-zÀ-ÿ]/.test(c)) return false;
  if (/^(https?:|\/|#|\.|\[|data:|wa\.me|tel:|mailto:)/.test(c)) return false;
  if (/^[a-z0-9_\-.:/]+$/.test(c)) return false; // clé, classe, identifiant
  if (/^[a-z-]+(\s*[:;{]|\()/.test(c) && !/[àâçéèêëîïôûùü]/i.test(c)) return false; // CSS
  if (/^[\w$.]+\(/.test(c)) return false;
  if (/^(GET|POST|PUT|DELETE)$/.test(c)) return false;
  if (/^[A-Z_]+$/.test(c)) return false;
  return /[ àâçéèêëîïôûùüÀÂÇÉÈÊ«»'’]/.test(c) || /^[A-ZÀ-Ý][a-zà-ÿ]/.test(c);
}

// Un morceau de gabarit HTML (« <p>Bonjour ${nom}</p> ») : textes entre les
// balises, avec les parties variables.
function ajouterSquelette(squelette) {
  const sansBalises = squelette.replace(/<[^>]*>/g, "\u0001");
  for (const morceau of sansBalises.split("\u0001")) {
    const t = morceau.replace(/\s+/g, " ").trim();
    if (!t) continue;
    if (t.includes(VAR)) {
      const lettres = t.split(VAR).join("");
      if (!estTexte(lettres)) continue;
      let n = 0;
      motifs.add(t.replace(new RegExp(VAR, "g"), () => `{${++n}}`));
    } else if (estTexte(t)) {
      exacts.add(t);
    }
  }
}

function attributs(html) {
  for (const m of html.matchAll(/\s(?:placeholder|aria-label|title|alt)="([^"$]*)"/g)) {
    if (estTexte(m[1])) exacts.add(m[1].trim());
  }
}

function lireJs(source) {
  const f = ts.createSourceFile("x.js", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
  const visiter = (n) => {
    if (ts.isStringLiteral(n) || ts.isNoSubstitutionTemplateLiteral(n)) {
      const t = n.text;
      if (/<[a-z][^>]*>/i.test(t)) { ajouterSquelette(t); attributs(t); }
      else if (estTexte(t) && !isImportOuCle(n)) exacts.add(t.replace(/\s+/g, " ").trim());
    } else if (ts.isTemplateExpression(n)) {
      let s = n.head.text;
      for (const span of n.templateSpans) s += VAR + span.literal.text;
      ajouterSquelette(s);
      attributs(s);
    }
    ts.forEachChild(n, visiter);
  };
  visiter(f);
}

// Clés d'objets, sélecteurs passés à querySelector, noms d'événements : pas
// des textes à traduire.
function isImportOuCle(n) {
  const p = n.parent;
  if (!p) return false;
  if (ts.isPropertyAssignment(p) && p.name === n) return true;
  if (ts.isElementAccessExpression(p)) return true;
  if (ts.isCallExpression(p)) {
    const nom = p.expression.getText();
    if (/(querySelector(All)?|\$\$?|getElementById|addEventListener|setAttribute|getItem|setItem|removeItem|lire|ecrire|invoke|listen|matches|closest|startsWith|endsWith|includes|split|replace|test|toggle|add|remove|contains)$/.test(nom)) return true;
  }
  return false;
}

for (const chemin of process.argv.slice(2)) {
  const texte = fs.readFileSync(chemin, "utf8");
  if (chemin.endsWith(".html")) {
    const scripts = [...texte.matchAll(/<script>([\s\S]*?)<\/script>/g)].map((m) => m[1]);
    const html = texte.replace(/<script>[\s\S]*?<\/script>/g, "").replace(/<style>[\s\S]*?<\/style>/g, "").replace(/<!--[\s\S]*?-->/g, "");
    ajouterSquelette(html);
    attributs(html);
    const titre = html.match(/<title>([^<]*)<\/title>/);
    if (titre && estTexte(titre[1])) exacts.add(titre[1].trim());
    scripts.forEach(lireJs);
  } else {
    lireJs(texte);
  }
}
console.log(JSON.stringify({ exacts: [...exacts].sort(), motifs: [...motifs].sort() }, null, 1));
