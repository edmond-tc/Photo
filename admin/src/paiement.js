// Paiement en ligne de l'abonnement, par FedaPay (Mobile Money MTN, Moov,
// et cartes ; Bénin, Togo, Côte d'Ivoire, Sénégal…).
//
// Éteint tant que les clés ne sont pas posées sur le Worker (Settings →
// Variables and Secrets) : FEDAPAY_CLE_SECRETE et FEDAPAY_SECRET_WEBHOOK,
// plus FEDAPAY_MODE = "live" pour les vrais paiements (sinon : bac à sable,
// pour essayer sans argent). Guide : docs/paiements.md.
//
// Règle de sûreté : un avis de paiement ne suffit jamais. Avant de délivrer
// une clé, le tableau de bord redemande la transaction à FedaPay lui-même
// (statut « approved », bon montant, bonne demande).

// FEDAPAY_ADRESSE_ESSAI : seulement pour le banc d'essai (faux FedaPay local).
const base = (env) =>
  (env.FEDAPAY_ADRESSE_ESSAI || (env.FEDAPAY_MODE === "live" ? "https://api.fedapay.com" : "https://sandbox-api.fedapay.com")) + "/v1";

export const paiementActif = (env) => Boolean(env.FEDAPAY_CLE_SECRETE && env.FEDAPAY_SECRET_WEBHOOK);

async function appel(env, methode, chemin, corps) {
  const r = await fetch(base(env) + chemin, {
    method: methode,
    headers: {
      Authorization: `Bearer ${env.FEDAPAY_CLE_SECRETE}`,
      "content-type": "application/json",
      accept: "application/json",
    },
    body: corps ? JSON.stringify(corps) : undefined,
  });
  const texte = await r.text();
  let donnees = {};
  try {
    donnees = JSON.parse(texte);
  } catch {}
  if (!r.ok) throw new Error(`FedaPay ${r.status} : ${texte.slice(0, 200)}`);
  return donnees;
}

// FedaPay range l'objet sous « v1/transaction » ; on accepte aussi les
// formes plus simples, au cas où.
const transactionDe = (d) => d?.["v1/transaction"] || d?.transaction || d || {};

/** Crée la transaction et rend l'adresse de la page de paiement FedaPay. */
export async function creerPaiement(env, { montant, description, retour, demandeId }) {
  const t = transactionDe(
    await appel(env, "POST", "/transactions", {
      description,
      amount: montant,
      currency: { iso: "XOF" },
      callback_url: retour,
      custom_metadata: { demande_id: String(demandeId) },
    })
  );
  if (!t.id) throw new Error("FedaPay : transaction sans identifiant");
  const jeton = await appel(env, "POST", `/transactions/${t.id}/token`);
  if (!jeton.url) throw new Error("FedaPay : pas d'adresse de paiement");
  return { id: String(t.id), url: jeton.url };
}

export async function lireTransaction(env, id) {
  return transactionDe(await appel(env, "GET", `/transactions/${encodeURIComponent(id)}`));
}

/** La transaction est-elle payée, au bon montant, pour cette demande ? */
export function transactionPayee(t, { montant, demandeId }) {
  const payee = ["approved", "transferred"].includes(String(t.status || ""));
  const bonMontant = Number(t.amount) >= Number(montant);
  const meta = t.custom_metadata || {};
  const bonneDemande = String(meta.demande_id ?? "") === String(demandeId);
  return payee && bonMontant && bonneDemande;
}

/** En-tête « X-FEDAPAY-SIGNATURE: t=<horodatage>,s=<signature> » : HMAC-SHA256
 * de « <horodatage>.<corps brut> » avec le secret du webhook, 5 minutes de
 * tolérance (comme la bibliothèque officielle fedapay-node). */
export async function signatureValide(secret, entete, corps, maintenant = Date.now()) {
  if (!secret || !entete) return false;
  let horodatage = null;
  const signatures = [];
  for (const morceau of String(entete).split(",")) {
    const [cle, valeur] = morceau.split("=").map((x) => (x || "").trim());
    if (cle === "t") horodatage = valeur;
    else if (cle === "s" && valeur) signatures.push(valeur.toLowerCase());
  }
  if (!horodatage || !signatures.length || !/^\d+$/.test(horodatage)) return false;
  if (Math.abs(maintenant / 1000 - Number(horodatage)) > 300) return false;
  const cle = await crypto.subtle.importKey("raw", new TextEncoder().encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  const brut = await crypto.subtle.sign("HMAC", cle, new TextEncoder().encode(`${horodatage}.${corps}`));
  const attendue = [...new Uint8Array(brut)].map((o) => o.toString(16).padStart(2, "0")).join("");
  return signatures.some((s) => egalConstant(s, attendue));
}

function egalConstant(a, b) {
  if (a.length !== b.length) return false;
  let d = 0;
  for (let i = 0; i < a.length; i++) d |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return d === 0;
}

/** Prix de l'abonnement, en francs CFA, par durée (Paramètres du tableau de bord). */
export const DUREES = [30, 90, 365];
