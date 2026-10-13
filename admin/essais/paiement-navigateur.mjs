// Le bouton « Payer » dans un VRAI navigateur (Chromium) : la page de
// renouvellement doit pouvoir envoyer le gérant vers la page de paiement
// FedaPay. (Les règles de sécurité du navigateur — form-action — bloquent
// une redirection vers un site non autorisé, ce qu'un essai sans
// navigateur ne voit pas.)
//
//   ADMIN=http://127.0.0.1:8788 node admin/essais/paiement-navigateur.mjs
import { execSync } from "node:child_process";
process.env.ADRESSE_PAGE_PAIEMENT = "https://process.fedapay.com/paiement";
const { demarrerFauxFedapay } = await import("./faux-fedapay.mjs");
const racine = execSync("npm root -g").toString().trim();
const { chromium } = await import(`${racine}/playwright/index.mjs`);
const ADMIN = process.env.ADMIN || "http://127.0.0.1:8788";
const faux = await demarrerFauxFedapay(8098);
const n = await chromium.launch();
const p = await n.newPage();
// Le navigateur part-il vers FedaPay ? (Playwright n'intercepte pas une page
// atteinte par redirection : on regarde la demande partir, sans la servir.)
let arrive = false;
p.on("request", (r) => { if (r.url().startsWith("https://process.fedapay.com/")) arrive = true; });
const bloque = [];
p.on("console", (m) => { if (/Content Security Policy|form-action/i.test(m.text())) bloque.push(m.text()); });
let ok = true;
for (const [chemin, champs] of [["/renouveler?m=A1B2-C3D4-E5F6-0718-9A0B", {}], ["/activer?m=A1B2-C3D4-E5F6-0718-9A0B", {}]]) {
  arrive = false;
  bloque.length = 0;
  await p.goto(ADMIN + chemin);
  await p.fill('form[action="/payer"] input[name="nom_boutique"]', "Boutique navigateur");
  await p.fill('form[action="/payer"] input[name="telephone"]', "+229 0197000001");
  await p.click('form[action="/payer"] button[type="submit"]');
  await p.waitForTimeout(2500);
  const bon = arrive && !bloque.length;
  console.log(`${bon ? "✅" : "❌"} ${chemin.split("?")[0]} : bouton « Payer » → le navigateur part vers FedaPay${bon ? "" : " : BLOQUÉ (" + (bloque[0] || "aucune demande vers FedaPay") + ")"}`);
  ok &&= bon;
}
await n.close();
faux.fermer();
console.log(`BILAN NAVIGATEUR : ${ok ? "RÉUSSI" : "ÉCHEC"}`);
process.exit(ok ? 0 : 1);
