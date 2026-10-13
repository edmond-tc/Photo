// Essai de charge du relais Internet : plusieurs boutiques, beaucoup de
// clients en même temps. Vérifie que chaque boutique reçoit exactement SES
// commandes, intactes, une seule fois, et compte les demandes faites au
// relais (pour les quotas).
//
//   RELAIS=http://127.0.0.1:8787 node banc-telephones/charge-relais.mjs
import crypto from "node:crypto";

const RELAIS = process.env.RELAIS || "http://127.0.0.1:8787";
const BOUTIQUES = Number(process.env.BOUTIQUES || 3);
const CLIENTS = Number(process.env.CLIENTS || 20);
const MO = 1024 * 1024;
let demandes = 0;
const reprises = { n: 0 };
// Une limite de temps sur chaque demande, comme le vrai PC et la vraie page.
const f = (url, opts = {}) => { demandes += 1; return fetch(url, { signal: AbortSignal.timeout(60000), ...opts }); };
const attendre = (ms) => new Promise((r) => setTimeout(r, ms));
const sha = (b) => crypto.createHash("sha256").update(b).digest("hex");
const alphabet = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
// 9 caractères, « CHG » en tête : jamais un code de vraie boutique (6 caractères),
// et reconnu par le nettoyage du relais.
const code = () => "CHG" + Array.from(crypto.randomBytes(6), (o) => alphabet[o % 32]).join("");

async function client(boutique, n, attendus) {
  const taille = Math.floor(MO / 2 + Math.random() * 3.5 * MO);
  const doc = crypto.randomBytes(taille);
  const nom = `${boutique.code}-client${n}.pdf`;
  const id = crypto.randomBytes(12).toString("hex");
  const doublon = n % 5 === 0; // un client sur cinq : le réseau renvoie le même morceau
  // Comme la vraie page : après une coupure, on demande au relais où il en
  // est et on reprend de là (jusqu'à 8 essais de suite).
  let debut = 0, echecs = 0;
  const position = async () => (await (await f(`${RELAIS}/b/${boutique.code}/morceau/${id}`)).json()).recu;
  while (debut < taille) {
    const morceau = doc.subarray(debut, Math.min(debut + MO, taille));
    const envoi = () => f(`${RELAIS}/b/${boutique.code}/morceau/${id}?debut=${debut}`, { method: "POST", body: morceau }).catch(() => ({ ok: false }));
    const reponses = await Promise.all(doublon ? [envoi(), envoi()] : [envoi()]);
    const bonne = reponses.find((r) => r.ok);
    if (bonne) { debut = (await bonne.json()).recu; echecs = 0; continue; }
    if (++echecs > 8) throw new Error(`morceau perdu (${nom})`);
    if (reponses.some((r) => r.status && r.status !== 409)) reprises.n += 1;
    await attendre(500 * echecs);
    debut = await position().catch(() => debut);
  }
  // La commande, avec reprise après une coupure, comme le bouton « Réessayer ».
  const commander = async () => {
    for (let essai = 1; ; essai++) {
      const form = new FormData();
      form.append("nom", `Client ${n}`);
      form.append("telephone", `+229 01 ${String(boutique.code.length * 1e6 + n * 7919).padStart(8, "0").slice(-8)}`);
      form.append("nombre_fichiers", "1");
      form.append("nomreprise_0", nom);
      form.append("reprise_0", id);
      const r = await f(`${RELAIS}/b/${boutique.code}/envoyer`, { method: "POST", body: form }).catch(() => ({ ok: false }));
      if (r.ok) return (await r.json()).jetons[0];
      if (essai >= 4) throw new Error(`commande refusée ${r.status} : ${r.text ? (await r.text()).slice(0, 80) : "coupure"}`);
      reprises.n += 1;
      await attendre(1000 * essai);
    }
  };
  const jeton = await commander();
  // Un client sur cinq retouche « Réessayer » : la même commande, pas une seconde.
  if (doublon && (await commander()) !== jeton) throw new Error(`commande en double (${nom})`);
  attendus.set(nom, { empreinte: sha(doc), jeton });
}

async function pc(boutique, recus, fin) {
  const h = { "x-secret": boutique.secret };
  while (!fin.fait || recus.size < boutique.attendus.size) {
    const a = await (await f(`${RELAIS}/api/relais/${boutique.code}/attente`, { headers: h })).json();
    for (const c of a.commandes) {
      for (const fi of c.fichiers) {
        const corps = Buffer.from(await (await f(`${RELAIS}/api/relais/${boutique.code}/fichier/${c.id}/${fi.champ}`, { headers: h })).arrayBuffer());
        const deja = recus.get(fi.nom);
        recus.set(fi.nom, { empreinte: sha(corps), fois: (deja?.fois || 0) + 1 });
      }
      await f(`${RELAIS}/api/relais/${boutique.code}/accuse/${c.id}`, {
        method: "POST", headers: { ...h, "content-type": "application/json" }, body: JSON.stringify({ numero: `X-${recus.size}` }),
      });
    }
    if (!a.commandes.length) await attendre(300);
  }
}

const debut = Date.now();
const boutiques = [];
for (let i = 0; i < BOUTIQUES; i++) {
  const b = { code: code(), secret: crypto.randomBytes(32).toString("hex"), attendus: new Map(), recus: new Map() };
  const r = await fetch(`${RELAIS}/api/relais/inscrire`, {
    method: "POST", headers: { "content-type": "application/json" },
    body: JSON.stringify({ code: b.code, secret: b.secret, nom: `Charge ${i}` }),
  });
  if (!r.ok) throw new Error("inscription " + r.status);
  boutiques.push(b);
}
const fin = { fait: false };
const pcs = boutiques.map((b) => pc(b, b.recus, fin));
const erreurs = [];
await Promise.all(
  boutiques.flatMap((b) =>
    Array.from({ length: CLIENTS }, (_, n) => client(b, n, b.attendus).catch((e) => erreurs.push(e.message)))
  )
);
fin.fait = true;
await Promise.race([Promise.all(pcs), attendre(120000)]);

let ok = erreurs.length === 0;
for (const b of boutiques) {
  let bons = 0, abimes = 0, doubles = 0, etrangers = 0;
  for (const [nom, att] of b.attendus) {
    const r = b.recus.get(nom);
    if (r && r.empreinte === att.empreinte) bons++; else abimes++;
    if (r && r.fois > 1) doubles++;
  }
  for (const nom of b.recus.keys()) if (!nom.startsWith(b.code)) etrangers++;
  // Le client voit-il le numéro ?
  const un = [...b.attendus.values()][0];
  const st = un ? await (await fetch(`${RELAIS}/b/${b.code}/statut/${un.jeton}`)).json() : {};
  const ligneOk = bons === CLIENTS && !abimes && !doubles && !etrangers && /^X-/.test(st.numero || "");
  ok &&= ligneOk;
  console.log(`${ligneOk ? "✅" : "❌"} boutique ${b.code} : ${bons}/${CLIENTS} commandes intactes, ${abimes} abîmées ou manquantes, ${doubles} reçues deux fois, ${etrangers} d'une autre boutique, numéro vu par le client : ${st.numero}`);
}
if (erreurs.length) console.log("Erreurs clients :", [...new Set(erreurs)].slice(0, 5).join(" | "));
const total = BOUTIQUES * CLIENTS;
console.log(`\n${demandes} demandes au relais pour ${total} commandes (${(demandes / total).toFixed(1)} par commande), en ${((Date.now() - debut) / 1000).toFixed(1)} s.`);
console.log(`${reprises.n} reprises après une coupure.`);
console.log(`BILAN CHARGE : ${ok ? "RÉUSSI" : "ÉCHEC"}`);
console.log("CODES " + boutiques.map((b) => b.code).join(" "));
process.exit(ok ? 0 : 1);
