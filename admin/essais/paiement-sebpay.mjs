// Banc d'essai du paiement par SebPay, de bout en bout, contre le tableau de
// bord local (wrangler dev, avec SEBPAY_ADRESSE_ESSAI) et un faux SebPay :
//   formulaire → collecte (demande sur le téléphone) → avis signé →
//   vérification → clé ou code d'installation lus par le logiciel.
//
//   ADMIN=http://127.0.0.1:8788 node admin/essais/paiement-sebpay.mjs
import crypto from "node:crypto";
import { demarrerFauxSebpay } from "./faux-sebpay.mjs";

const ADMIN = process.env.ADMIN || "http://127.0.0.1:8788";
const faux = await demarrerFauxSebpay(8097);
let ok = true;
const verifier = (condition, texte) => { console.log(`${condition ? "✅" : "❌"} ${texte}`); ok &&= condition; };
const nouvelleMachine = () => "SEB-" + crypto.randomBytes(4).toString("hex").toUpperCase();
const derniere = () => [...faux.collectes.values()].at(-1);
const cle = async (m) => { const r = await fetch(`${ADMIN}/api/licence/${m}`); return r.ok ? (await r.json()).cle : null; };
const code = async (m) => { const r = await fetch(`${ADMIN}/api/installation/${m}`); return r.ok ? (await r.json()).code : null; };

async function payer(machine, { jours = 30, pays = "BJ", operateur = "mtn", numero = "01 97 00 00 01", otp = "" } = {}) {
  const f = new FormData();
  f.append("nom_boutique", "Boutique banc SebPay");
  f.append("machine_id", machine);
  f.append("telephone", "+229 01 97 00 00 01");
  f.append("jours_demandes", String(jours));
  f.append("pays", pays);
  if (operateur) f.append("operateur", operateur);
  f.append("numero_paiement", numero);
  if (otp) f.append("otp", otp);
  // Une adresse différente à chaque fois : la limite de 8 demandes par quart
  // d'heure et par adresse ne gêne pas le banc (wrangler dev la transmet).
  const ip = `10.9.${crypto.randomInt(255)}.${crypto.randomInt(255)}`;
  return fetch(`${ADMIN}/payer`, { method: "POST", body: f, redirect: "manual", headers: { "CF-Connecting-IP": ip } });
}

// 1. Le formulaire propose les Mobile Money du pays
const page = await (await fetch(`${ADMIN}/renouveler`)).text();
verifier(page.includes('value="mtn"') && page.includes('value="moov"') && page.includes('name="numero_paiement"'), "formulaire : MTN et Moov proposés, numéro qui paie demandé");

// 2. Saisie incomplète : refusée, sans rien demander à SebPay
const avant = faux.collectes.size;
let r = await payer(nouvelleMachine(), { numero: "12" });
verifier(r.status === 400 && (await r.text()).includes("pas complet") && faux.collectes.size === avant, "numéro incomplet : refusé, rien envoyé à SebPay");

// 3. Paiement normal
const m1 = nouvelleMachine();
r = await payer(m1);
const attente = r.headers.get("location") || "";
const c1 = derniere();
verifier(r.status === 303 && /^\/paiement\/retour\?d=\d+$/.test(attente), "« Payer » → page d'attente");
verifier(c1 && c1.amount === 5000 && c1.currency === "XOF" && c1.phone === undefined && c1.customer_phone === "2290197000001" && c1.operator === "mtn" && /\/paiement\/sebpay$/.test(c1.callback_url),
  `collecte de 5 000 FCFA envoyée au 2290197000001 (MTN), avis attendu sur /paiement/sebpay`);
let rp = await fetch(ADMIN + attente);
verifier((await rp.text()).includes("Validez le paiement sur votre téléphone") && /^10; url=/.test(rp.headers.get("refresh") || ""), "page d'attente : « Validez le paiement sur votre téléphone », se recharge seule");
verifier(!(await cle(m1)), "pas de clé avant validation");

// 4. Faux avis (mauvaise signature) : refusé
const faussesCles = await faux.conclure(c1.external_reference, { statut: "pending", secret: "pas_le_bon" });
verifier(faussesCles.status === 400, "avis mal signé refusé");
faux.collectes.get(c1.external_reference).status = "pending";

// 5. Le gérant valide : avis signé → clé
const a1 = await faux.conclure(c1.external_reference);
verifier(a1.status === 200, "avis signé accepté (200)");
const k1 = await cle(m1);
verifier(!!k1 && /-\d{8}$/.test(k1), "clé délivrée, lue par le logiciel");
rp = await fetch(ADMIN + attente);
verifier((await rp.text()).includes("Paiement reçu") && !rp.headers.get("refresh"), "page d'attente : « Paiement reçu », ne se recharge plus");

// 6. Avis reçu deux fois : pas de seconde clé
await faux.conclure(c1.external_reference);
verifier((await cle(m1)) === k1, "avis en double : une seule clé");

// 7. Payé moins que le prix : pas de clé
const m2 = nouvelleMachine();
await payer(m2);
await faux.conclure(derniere().external_reference, { montant: 100 });
verifier(!(await cle(m2)), "payé 100 FCFA au lieu de 5 000 : pas de clé");

// 8. Refusé par le gérant : la page le dit, rien ne se recharge
const m3 = nouvelleMachine();
r = await payer(m3, { operateur: "moov", numero: "+229 01 66 00 00 02" });
const attente3 = r.headers.get("location");
verifier(derniere().operator === "moov" && derniere().customer_phone === "2290166000002", "Moov et numéro au format international (+229 accepté)");
await faux.conclure(derniere().external_reference, { statut: "rejected" });
rp = await fetch(ADMIN + attente3);
verifier((await rp.text()).includes("Paiement refusé ou annulé") && !rp.headers.get("refresh"), "paiement refusé : la page le dit, lien pour recommencer");
verifier(!(await cle(m3)), "paiement refusé : pas de clé");

// 9. Avis perdu : le logiciel, en venant chercher sa clé, la fait délivrer
const m4 = nouvelleMachine();
await payer(m4);
await faux.conclure(derniere().external_reference, { avis: false });
verifier(!!(await cle(m4)), "avis perdu : le logiciel reçoit quand même sa clé");

// 10. Frais d'activation : code d'installation
const m5 = "A1B2-C3D4-" + crypto.randomBytes(2).toString("hex").toUpperCase() + "-E5F6-0719";
const pageActiver = await (await fetch(`${ADMIN}/activer?m=${m5.toLowerCase()}`)).text();
verifier(pageActiver.includes(`value="${m5}"`) && pageActiver.includes('value="mtn"'), "page d'activation : identifiant rempli, Mobile Money proposés");
verifier(!(await code(m5)), "activation : pas de code avant paiement");
await payer(m5, { jours: 0 });
verifier(derniere().amount === 10000, "collecte d'activation de 10 000 FCFA");
await faux.conclure(derniere().external_reference);
verifier(/^INST-/.test((await code(m5)) || ""), "activation payée : code d'installation délivré");

// 11. Opérateur à code OTP (Orange Côte d'Ivoire)
const pageCI = await (await fetch(`${ADMIN}/renouveler?pays=CI`)).text();
verifier(pageCI.includes('value="orange"') && pageCI.includes("#144*82#") && pageCI.includes('name="otp"'), "Côte d'Ivoire : Orange proposé avec son code USSD et la case OTP");
r = await payer(nouvelleMachine(), { pays: "CI", operateur: "orange", numero: "07 00 00 00 00" });
verifier(r.status === 400 && (await r.text()).includes("#144*82#"), "Orange sans code OTP : refusé, la page dit quel code composer");
r = await payer(nouvelleMachine(), { pays: "CI", operateur: "orange", numero: "07 00 00 00 00", otp: "123456" });
verifier(r.status === 303 && derniere().customer_phone === "2250700000000", "Orange avec code OTP : collecte lancée (2250700000000)");

// 12. SebPay ne sait pas relire une collecte : l'avis signé fait foi
faux.etat.relectureAbsente = true;
const m6 = nouvelleMachine();
await payer(m6);
const ref6 = derniere().external_reference;
await faux.conclure(ref6, { secret: "pas_le_bon" });
verifier(!(await cle(m6)), "sans relecture : un avis mal signé ne donne rien");
await faux.conclure(ref6);
verifier(!!(await cle(m6)), "sans relecture : l'avis signé suffit");
faux.etat.relectureAbsente = false;

faux.fermer();
console.log(`\nBILAN SEBPAY : ${ok ? "RÉUSSI" : "ÉCHEC"}`);
process.exit(ok ? 0 : 1);
