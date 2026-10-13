// Banc d'essai du paiement en ligne, de bout en bout, contre le tableau de
// bord local (wrangler dev) et un faux FedaPay :
//   formulaire → transaction → avis signé → vérification → clé → /api/licence
//
//   ADMIN=http://127.0.0.1:8788 node admin/essais/paiement.mjs
import crypto from "node:crypto";
import { demarrerFauxFedapay } from "./faux-fedapay.mjs";

const ADMIN = process.env.ADMIN || "http://127.0.0.1:8788";
const SECRET_AVIS = process.env.SECRET_AVIS || "whsec_banc";
const faux = await demarrerFauxFedapay(8098);
let ok = true;
const verifier = (condition, texte) => { console.log(`${condition ? "✅" : "❌"} ${texte}`); ok &&= condition; };

async function avis(t, secret = SECRET_AVIS, decalage = 0) {
  const corps = JSON.stringify({ name: "transaction.approved", entity: { id: t.id, custom_metadata: t.custom_metadata, status: t.status, amount: t.amount } });
  const horodatage = Math.floor(Date.now() / 1000) + decalage;
  const s = crypto.createHmac("sha256", secret).update(`${horodatage}.${corps}`).digest("hex");
  return fetch(`${ADMIN}/paiement/fedapay`, { method: "POST", headers: { "x-fedapay-signature": `t=${horodatage},s=${s}`, "content-type": "application/json" }, body: corps });
}
async function payer(machine, jours = 30) {
  const f = new FormData();
  f.append("nom_boutique", "Boutique banc paiement");
  f.append("machine_id", machine);
  f.append("telephone", "+229 01 97 00 00 01");
  f.append("jours_demandes", String(jours));
  const r = await fetch(`${ADMIN}/payer`, { method: "POST", body: f, redirect: "manual" });
  return r;
}
const cle = async (machine) => { const r = await fetch(`${ADMIN}/api/licence/${machine}`); return r.ok ? (await r.json()).cle : null; };

// 1. Paiement normal
const machine = "BANC-" + crypto.randomBytes(4).toString("hex").toUpperCase();
let r = await payer(machine);
verifier(r.status === 303 && /\/payer\/\d+$/.test(r.headers.get("location") || ""), `formulaire → page de paiement FedaPay (${r.status})`);
const idT = Number((r.headers.get("location") || "").split("/").pop());
const t = faux.transactions.get(idT);
verifier(t && t.amount === 5000 && t.currency?.iso === "XOF", "transaction de 5 000 FCFA créée chez FedaPay");

// 2. Avis avant paiement : aucune clé
await avis(t);
verifier(!(await cle(machine)), "avis reçu avant le paiement : pas de clé (FedaPay redemandé, statut « pending »)");

// 3. Avis mal signé, ou trop vieux : refusés
verifier((await avis({ ...t, status: "approved" }, "mauvais_secret")).status === 400, "avis mal signé refusé");
verifier((await avis({ ...t, status: "approved" }, SECRET_AVIS, -3600)).status === 400, "avis vieux d'une heure refusé");

// 4. Le client paie, revient sur le site : clé délivrée
const retour = await fetch(`http://127.0.0.1:8098/payer/${idT}`, { redirect: "manual" });
const page = await (await fetch(retour.headers.get("location"))).text();
verifier(page.includes("Paiement reçu"), "retour du client : « Paiement reçu »");
const c1 = await cle(machine);
verifier(!!c1 && /-\d{8}$/.test(c1), "clé délivrée, lisible par le logiciel (/api/licence)");

// 5. Avis tardif après coup : pas de seconde clé
await avis({ ...t, status: "approved" });
verifier((await cle(machine)) === c1, "avis tardif : pas de seconde clé");

// 6. Payé moins que le prix : pas de clé
const machine2 = "BANC-" + crypto.randomBytes(4).toString("hex").toUpperCase();
r = await payer(machine2);
const id2 = Number((r.headers.get("location") || "").split("/").pop());
await fetch(`http://127.0.0.1:8098/payer/${id2}?montant=100`, { redirect: "manual" });
await avis(faux.transactions.get(id2));
verifier(!(await cle(machine2)), "payé 100 FCFA au lieu de 5 000 : pas de clé");

// 7. Paiement en avance : la nouvelle période s'ajoute à la fin de l'actuelle
r = await payer(machine);
const id3 = Number((r.headers.get("location") || "").split("/").pop());
await fetch(`http://127.0.0.1:8098/payer/${id3}`, { redirect: "manual" });
await avis(faux.transactions.get(id3));
const c2 = await cle(machine);
const fin = (c) => c && c.slice(-8);
verifier(c2 && Number(fin(c2)) > Number(fin(c1)), `renouvelé en avance : fin repoussée (${fin(c1)} → ${fin(c2)})`);

// 8. Frais d'activation : pas de code avant paiement, code après
const machine3 = "A1B2-C3D4-" + crypto.randomBytes(2).toString("hex").toUpperCase() + "-E5F6-0718";
const code = async (m) => { const r = await fetch(`${ADMIN}/api/installation/${m}`); return r.ok ? (await r.json()).code : null; };
verifier(!(await code(machine3)), "activation : aucun code avant paiement (pas d'activation gratuite)");
const offre = await (await fetch(`${ADMIN}/api/offre`)).json();
verifier(offre.activation === 10000 && offre.abonnement.some((o) => o.jours === 30), "offre lue par le logiciel : activation 10 000 FCFA, abonnement 30 jours");
const pageActiver = await (await fetch(`${ADMIN}/activer?m=${machine3.toLowerCase()}`)).text();
verifier(pageActiver.includes(`value="${machine3}"`) && pageActiver.includes("Payer l'activation"), "page d'activation ouverte par le QR : identifiant rempli (remis en majuscules)");
r = await payer(machine3, 0);
const id4 = Number((r.headers.get("location") || "").split("/").pop());
verifier(faux.transactions.get(id4)?.amount === 10000, "transaction d'activation de 10 000 FCFA créée");
const retour4 = await fetch(`http://127.0.0.1:8098/payer/${id4}`, { redirect: "manual" });
const page4 = await (await fetch(retour4.headers.get("location"))).text();
const c4 = await code(machine3);
verifier(/^INST-/.test(c4 || "") && page4.includes("activé"), "activation payée : code délivré, le logiciel le lit tout seul");
verifier(!(await cle(machine3)), "l'activation ne donne pas de licence (l'essai gratuit commence)");

faux.fermer();
console.log(`\nBILAN PAIEMENT : ${ok ? "RÉUSSI" : "ÉCHEC"}`);
process.exit(ok ? 0 : 1);
