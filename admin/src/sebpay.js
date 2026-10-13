// Paiement en ligne de l'activation et de la licence par SebPay (Mobile Money,
// Bénin et Afrique de l'Ouest). Documentation : https://new.sebpay.bj
//
// Pas de page de paiement : le tableau de bord demande la collecte, et le
// gérant reçoit la demande directement sur son téléphone (il la valide avec
// son code secret Mobile Money). Certains opérateurs (Orange CI, BF…)
// demandent d'abord un code OTP obtenu en composant un code USSD.
//
// Éteint tant que les clés ne sont pas posées sur le Worker (Settings →
// Variables and Secrets) : SEBPAY_CLE_PUBLIQUE et SEBPAY_CLE_SECRETE.
// Guide : docs/paiements.md.
//
// Règle de sûreté : un avis de paiement ne suffit pas tout seul. Il doit
// porter la signature HMAC de SebPay, et le tableau de bord redemande la
// collecte à SebPay (statut « approved », montant, référence) avant de
// délivrer une clé.

// SEBPAY_ADRESSE_ESSAI : seulement pour le banc d'essai (faux SebPay local).
const base = (env) => (env.SEBPAY_ADRESSE_ESSAI || "https://newapi.sebpay.bj") + "/api/v1";

export const sebpayActif = (env) => Boolean(env.SEBPAY_CLE_PUBLIQUE && env.SEBPAY_CLE_SECRETE);

/** Pays de la zone franc CFA (XOF), où les prix en FCFA s'appliquent tels quels. */
export const PAYS_SEBPAY = [
  ["BJ", "Bénin", "229"],
  ["TG", "Togo", "228"],
  ["CI", "Côte d'Ivoire", "225"],
  ["SN", "Sénégal", "221"],
  ["BF", "Burkina Faso", "226"],
  ["ML", "Mali", "223"],
  ["NE", "Niger", "227"],
];

export class ErreurSebpay extends Error {
  constructor(statut, message) {
    super(message);
    this.statut = statut;
  }
}

async function appel(env, methode, chemin, corps) {
  const r = await fetch(base(env) + chemin, {
    method: methode,
    headers: {
      "X-Public-Key": env.SEBPAY_CLE_PUBLIQUE,
      "X-Secret-Key": env.SEBPAY_CLE_SECRETE,
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
  if (!r.ok) {
    const message = donnees?.message || donnees?.error || texte.slice(0, 200);
    throw new ErreurSebpay(r.status, `SebPay ${r.status} : ${typeof message === "string" ? message : JSON.stringify(message)}`);
  }
  return donnees;
}

// Les réponses peuvent être enveloppées (« data ») ou non : on accepte les deux.
const contenu = (d) => (d && typeof d === "object" && d.data && typeof d.data === "object" ? d.data : d) || {};

const operateursEnCache = new Map();

/** Opérateurs Mobile Money du pays, lus chez SebPay (gardés 10 minutes). */
export async function operateurs(env, pays) {
  const enCache = operateursEnCache.get(pays);
  if (enCache && Date.now() - enCache.quand < 10 * 60 * 1000) return enCache.liste;
  const d = contenu(await appel(env, "GET", `/operators?country=${encodeURIComponent(pays)}`));
  const brut = Array.isArray(d) ? d : Array.isArray(d.operators) ? d.operators : [];
  const liste = brut
    .map((o) => ({
      slug: String(o.slug || o.code || "").trim(),
      nom: String(o.name || o.nom || o.label || o.slug || "").trim(),
      pays: String(o.country || o.country_code || pays).toUpperCase(),
      otp: o.otp_required === true || o.otp_required === 1 || o.otp_required === "1",
      ussd: String(o.ussd_code || "").trim(),
    }))
    .filter((o) => o.slug && o.pays === pays);
  operateursEnCache.set(pays, { quand: Date.now(), liste });
  return liste;
}

/** Numéro au format international sans « + » (ex. 2290197000000). */
export function numeroInternational(saisi, indicatif) {
  let n = String(saisi || "").replace(/[^0-9]/g, "");
  if (n.startsWith("00")) n = n.slice(2);
  if (!n.startsWith(indicatif)) n = indicatif + n;
  return n.length >= 10 && n.length <= 15 ? n : "";
}

/** Lance la collecte : le gérant reçoit la demande sur son téléphone. */
export async function creerCollecte(env, { montant, telephone, operateur, pays, reference, otp, retour, description }) {
  const corps = {
    amount: montant,
    currency: "XOF",
    phone: telephone,
    operator: operateur,
    country: pays,
    external_reference: reference,
    callback_url: retour,
    description,
  };
  if (otp) corps.otp_code = otp;
  const t = contenu(await appel(env, "POST", "/collections", corps));
  return { id: String(t.transaction_id || t.id || ""), statut: String(t.status || "pending") };
}

export async function lireCollecte(env, reference) {
  return contenu(await appel(env, "GET", `/collections/${encodeURIComponent(reference)}`));
}

/** La collecte est-elle payée, au bon montant, pour cette référence ? */
export function collectePayee(t, { montant, reference }) {
  return (
    String(t?.status || "") === "approved" &&
    Number(t?.amount) >= Number(montant) &&
    String(t?.external_reference ?? "") === String(reference)
  );
}

export const collecteRefusee = (t) => String(t?.status || "") === "rejected";

/** En-tête « X-SebPay-Signature » : HMAC-SHA256 du corps brut avec la clé
 * secrète. Hexadécimal attendu ; base64 et préfixe « sha256= » acceptés. */
export async function signatureSebpayValide(secret, entete, corps) {
  if (!secret || !entete) return false;
  const recue = String(entete).trim().replace(/^sha256=/i, "");
  const cle = await crypto.subtle.importKey("raw", new TextEncoder().encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  const brut = new Uint8Array(await crypto.subtle.sign("HMAC", cle, new TextEncoder().encode(corps)));
  const hex = [...brut].map((o) => o.toString(16).padStart(2, "0")).join("");
  const b64 = btoa(String.fromCharCode(...brut));
  return egalConstant(recue.toLowerCase(), hex) || egalConstant(recue, b64);
}

function egalConstant(a, b) {
  if (a.length !== b.length) return false;
  let d = 0;
  for (let i = 0; i < a.length; i++) d |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return d === 0;
}
