// Vérifie qu'un dictionnaire couvre tous les textes français d'une
// interface : sort en erreur et liste ce qui manque.
//
//   node scripts/langue/verifier.mjs client-web/index.html client-web/langue-en.json
//   node scripts/langue/verifier.mjs src/index.html src/main.js src/langue-en.json
import fs from "node:fs";
import { execFileSync } from "node:child_process";

const fichiers = process.argv.slice(2);
const dico = JSON.parse(fs.readFileSync(fichiers.pop(), "utf8"));
const extrait = JSON.parse(execFileSync("node", [new URL("./extraire.mjs", import.meta.url).pathname, ...fichiers], { maxBuffer: 64 << 20 }).toString());
const ignores = new Set(dico._ignores || []);
const motifs = new Set((dico.motifs || []).map(([fr]) => fr));
const exacts = extrait.exacts.filter((t) => !(t in dico.textes) && !ignores.has(t));
const variables = extrait.motifs.filter((t) => !motifs.has(t) && !ignores.has(t));
// Les motifs : mêmes parties variables des deux côtés.
const desaccords = (dico.motifs || []).filter(([fr, en]) => (fr.match(/\{\d+\}/g) || []).length < (en.match(/\{\d+\}/g) || []).length);
for (const t of exacts) console.log("MANQUE  " + t);
for (const t of variables) console.log("MOTIF   " + t);
for (const [fr] of desaccords) console.log("VARIABLES  " + fr);
const total = extrait.exacts.length + extrait.motifs.length;
console.log(`${total - exacts.length - variables.length}/${total} textes traduits.`);
process.exit(exacts.length + variables.length + desaccords.length ? 1 : 0);
