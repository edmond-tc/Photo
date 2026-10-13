// Relais Internet : le client envoie depuis ses données mobiles, le PC de la
// boutique vient chercher. Aucun des deux n'est joignable directement sur
// Internet (box, réseau mobile) : ce relais est le point de rendez-vous, et
// rien de plus. Il ne garde rien : chaque fichier est effacé dès que le PC
// l'a récupéré, et au plus tard au bout de 24 heures.
//
// Écrit avec les seules interfaces web standard (Request, Response,
// ReadableStream, crypto.subtle) et deux appuis : une base SQL (`env.DB`) et
// un stockage de fichiers (`env.TELECHARGEMENTS`, préfixe `relais/`). Changer
// d'hébergeur revient à fournir ces deux appuis ailleurs.

import SOURCE_PAGE_CLIENT from "../../client-web/index.html";
import { alleger } from "./alleger.js";

const PAGE_CLIENT = alleger(SOURCE_PAGE_CLIENT);
// Dictionnaire anglais de la page, chargé seulement par les téléphones en anglais.
import LANGUE_EN from "../../client-web/langue-en.json";
const LANGUE_EN_TEXTE = JSON.stringify(LANGUE_EN);

/** Un morceau : 1 Mo pour la page, un peu de marge. */
const MORCEAU_MAX = 2 * 1024 * 1024;
/** Un document : de quoi passer un mémoire scanné, pas un film. */
const FICHIER_MAX = 300 * 1024 * 1024;
/** Fichier joint directement (message vocal, document vide). */
const DIRECT_MAX = 8 * 1024 * 1024;
/** En attente, par boutique : au-delà, la boutique doit d'abord récupérer. */
const ATTENTE_MAX_PAR_BOUTIQUE = 2 * 1024 * 1024 * 1024;
const FICHIERS_MAX = 20;
/** Commandes d'un même numéro qui attendent encore la boutique. */
const COMMANDES_MAX_PAR_NUMERO = 10;
const ENVOI_MAX = 12 * 1024 * 1024; // une commande sans ses documents
const DUREE_DE_VIE_MS = 24 * 3600 * 1000;

/** Par téléphone et par boutique, sur une heure : de quoi servir un vrai
 * client (plusieurs gros documents), pas de quoi remplir l'attente d'une
 * boutique pour la bloquer. Tenu en mémoire de chaque instance du relais :
 * une protection de bon sens, pas une garantie absolue. */
// Large : au Bénin, les clients d'un même opérateur mobile sortent souvent
// par la même adresse Internet, et se partagent donc cette limite.
const DEBIT_MAX_PAR_HEURE = 2 * 1024 * 1024 * 1024;
const debits = new Map();
function debitPermis(request, code, octets) {
  const ip = request.headers.get("cf-connecting-ip") || "local";
  const cle = `${code}|${ip}`;
  const maintenant = Date.now();
  let d = debits.get(cle);
  if (!d || maintenant - d.depuis > 3600 * 1000) d = { depuis: maintenant, octets: 0 };
  d.octets += octets;
  debits.set(cle, d);
  if (debits.size > 5000) debits.clear();
  return d.octets <= DEBIT_MAX_PAR_HEURE;
}

const CODE_VALIDE = /^[0-9A-HJKMNP-TV-Z]{6,10}$/;
const ID_VALIDE = /^[0-9a-f]{12,40}$/;

const json = (objet, statut = 200) =>
  new Response(JSON.stringify(objet), {
    status: statut,
    headers: {
      "content-type": "application/json; charset=utf-8",
      "cache-control": "no-store",
      "access-control-allow-origin": "*",
    },
  });

function hasard(octets = 16) {
  const t = new Uint8Array(octets);
  crypto.getRandomValues(t);
  return [...t].map((o) => o.toString(16).padStart(2, "0")).join("");
}

async function empreinte(texte) {
  const brut = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(texte));
  return [...new Uint8Array(brut)].map((o) => o.toString(16).padStart(2, "0")).join("");
}

function egalConstant(a, b) {
  if (a.length !== b.length) return false;
  let d = 0;
  for (let i = 0; i < a.length; i++) d |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return d === 0;
}

const prefixeMorceaux = (code, id) => `relais/${code}/m/${id}/`;
const cleMorceau = (code, id, debut) => `${prefixeMorceaux(code, id)}${String(debut).padStart(12, "0")}`;
const cleDirecte = (code, commande, champ) => `relais/${code}/c/${commande}/${champ}`;

async function boutique(env, code) {
  if (!CODE_VALIDE.test(code)) return null;
  return env.DB.prepare("SELECT * FROM relais_boutiques WHERE code = ? AND actif = 1").bind(code).first();
}

/** Les morceaux d'un fichier, dans l'ordre, et la longueur contiguë reçue. */
async function morceaux(env, code, id) {
  const liste = [];
  let curseur;
  do {
    const r = await env.TELECHARGEMENTS.list({ prefix: prefixeMorceaux(code, id), cursor: curseur });
    liste.push(...r.objects);
    curseur = r.truncated ? r.cursor : undefined;
  } while (curseur);
  liste.sort((a, b) => (a.key < b.key ? -1 : 1));
  let recu = 0;
  const utiles = [];
  for (const o of liste) {
    const debut = Number(o.key.slice(-12));
    if (debut !== recu) break;
    utiles.push(o);
    recu += o.size;
  }
  return { recu, utiles, tous: liste };
}

async function compterAttente(env, code, delta) {
  await env.DB.prepare(
    "UPDATE relais_boutiques SET octets_attente = MAX(0, octets_attente + ?) WHERE code = ?"
  ).bind(delta, code).run();
}

async function effacerCommande(env, code, commande) {
  let fichiers = [];
  try { fichiers = JSON.parse(commande.fichiers || "[]"); } catch {}
  let libere = 0;
  for (const f of fichiers) {
    if (f.type === "morceaux") {
      const { tous } = await morceaux(env, code, f.id);
      for (const o of tous) { libere += o.size; await env.TELECHARGEMENTS.delete(o.key); }
    } else if (f.cle) {
      libere += f.taille || 0;
      await env.TELECHARGEMENTS.delete(f.cle);
    }
  }
  await compterAttente(env, code, -libere);
}

// ───────────────────────────── Côté client ─────────────────────────────

async function pageClient(b) {
  // La même page que dans la boutique ; elle sait qu'elle passe par
  // Internet grâce à son adresse (/b/CODE). Le nom de la boutique est posé
  // comme texte, jamais comme code.
  const nom = String(b.nom || "Photocopie").replace(/[<>&"]/g, "");
  return new Response(PAGE_CLIENT.replace("<title>", `<title>${nom} · `), {
    headers: {
      "content-type": "text/html; charset=utf-8",
      "cache-control": "no-store",
      "x-robots-tag": "noindex",
    },
  });
}

async function recevoirMorceau(request, env, b, id, debut) {
  const longueur = Number(request.headers.get("content-length") || 0);
  if (!longueur || longueur > MORCEAU_MAX) return json({ erreur: "Morceau trop grand." }, 413);
  if (debut + longueur > FICHIER_MAX) return new Response("Document trop lourd pour l'envoi par Internet (300 Mo au plus).", { status: 413 });
  if ((b.octets_attente || 0) + longueur > ATTENTE_MAX_PAR_BOUTIQUE) {
    return new Response("La boutique a trop de documents en attente. Réessayez dans un moment.", { status: 507 });
  }
  if (!debitPermis(request, b.code, longueur)) {
    return new Response("Trop d'envois depuis ce téléphone en peu de temps. Réessayez dans quelques minutes.", { status: 429 });
  }
  const { recu } = await morceaux(env, b.code, id);
  if (debut !== recu) return json({ recu }, 409);
  const cle = cleMorceau(b.code, id, debut);
  // Le même morceau peut arriver deux fois en même temps (le téléphone
  // renvoie quand le réseau hésite) : il n'est compté qu'une fois. Trouvé à
  // l'essai de charge : le compteur d'attente dérivait de plusieurs Mo par
  // vingt clients, jusqu'à bloquer à terme la boutique pour rien.
  const deja = await env.TELECHARGEMENTS.head(cle);
  await env.TELECHARGEMENTS.put(cle, request.body, {
    httpMetadata: { contentType: "application/octet-stream" },
  });
  if (!deja) await compterAttente(env, b.code, longueur);
  return json({ recu: recu + longueur });
}

async function recevoirCommande(request, env, b) {
  // La commande elle-même est petite (réglages et messages vocaux ; les
  // documents arrivent avant, par morceaux). Lue d'un bloc en mémoire, une
  // commande énorme pourrait faire tomber le relais pour toutes les autres
  // boutiques en même temps : refusée avant d'être lue.
  const longueur = Number(request.headers.get("content-length") || 0);
  if (!longueur || longueur > ENVOI_MAX) return new Response("Envoi trop lourd. Réessayez.", { status: 413 });
  let formulaire;
  try { formulaire = await request.formData(); } catch { return new Response("Envoi illisible. Réessayez.", { status: 400 }); }
  // Par Internet, le client n'est pas au guichet : la boutique doit pouvoir
  // le prévenir. Et une boutique peut bloquer un numéro (fausses commandes).
  // Vérifié avant de stocker quoi que ce soit.
  const numero = finDeNumero(formulaire.get("telephone"));
  if (numero.length < 8) {
    return new Response("Écrivez votre numéro WhatsApp : la boutique vous prévient quand c'est prêt.", { status: 400 });
  }
  let bloques = [];
  try { bloques = JSON.parse((await env.DB.prepare("SELECT bloques FROM relais_fiches WHERE code = ?").bind(b.code).first())?.bloques || "[]"); } catch {}
  if (bloques.includes(numero)) {
    return new Response("Cette boutique n'accepte pas vos commandes par Internet. Passez la voir directement.", { status: 403 });
  }
  const champs = {};
  const fichiers = [];
  const commande = hasard(12);
  const noms = {};
  for (const [cle, valeur] of formulaire.entries()) {
    if (cle.startsWith("nomreprise_")) noms[cle.slice(11)] = String(valeur).slice(0, 200);
  }
  for (const [cle, valeur] of formulaire.entries()) {
    if (fichiers.length >= FICHIERS_MAX * 2) break;
    if (cle.startsWith("reprise_")) {
      const indice = cle.slice(8);
      const id = String(valeur);
      if (!ID_VALIDE.test(id)) continue;
      const { recu } = await morceaux(env, b.code, id);
      if (!recu) continue;
      fichiers.push({ champ: `fichier_${indice}`, nom: noms[indice] || "document", type: "morceaux", id, taille: recu });
    } else if (typeof valeur === "object" && valeur && "arrayBuffer" in valeur) {
      if (!/^(fichier|vocal)_\d{1,2}$/.test(cle) || valeur.size > DIRECT_MAX) continue;
      const cleR2 = cleDirecte(b.code, commande, cle);
      await env.TELECHARGEMENTS.put(cleR2, await valeur.arrayBuffer());
      await compterAttente(env, b.code, valeur.size);
      fichiers.push({ champ: cle, nom: String(valeur.name || "document").slice(0, 200), type: "direct", cle: cleR2, taille: valeur.size });
    } else if (!cle.startsWith("nomreprise_") && Object.keys(champs).length < 400) {
      champs[cle.slice(0, 40)] = String(valeur).slice(0, 600);
    }
  }
  if (!fichiers.some((f) => f.champ.startsWith("fichier_"))) {
    return new Response("Aucun document n'est arrivé. Réessayez.", { status: 400 });
  }
  // La liaison a coupé juste après l'enregistrement et le client a touché
  // « Réessayer » : même document envoyé par morceaux = même commande. On
  // rend la première au lieu d'en créer une seconde (sinon la boutique
  // imprimerait deux fois).
  const premier = fichiers.find((f) => f.type === "morceaux");
  if (premier) {
    const deja = await env.DB.prepare(
      "SELECT jeton FROM relais_commandes WHERE code = ? AND fichiers LIKE ? LIMIT 1"
    ).bind(b.code, `%"id":"${premier.id}"%`).first();
    if (deja) {
      for (const f of fichiers) if (f.type === "direct") {
        await env.TELECHARGEMENTS.delete(f.cle);
        await compterAttente(env, b.code, -f.taille);
      }
      return json({ numero: null, jetons: [deja.jeton], en_ligne: true });
    }
  }
  champs.numero_client = String(champs.telephone || "").replace(/[^0-9]/g, "").slice(0, 20);
  const enCours = await env.DB.prepare(
    "SELECT COUNT(*) AS n FROM relais_commandes WHERE code = ? AND etape = 'en_ligne' AND champs LIKE ?"
  ).bind(b.code, `%"numero_client":"%${numero}"%`).first();
  if ((enCours?.n || 0) >= COMMANDES_MAX_PAR_NUMERO) {
    return new Response("Vous avez déjà plusieurs commandes qui attendent la boutique. Patientez qu'elle les reçoive.", { status: 429 });
  }
  const jeton = hasard(16);
  await env.DB.prepare(
    `INSERT INTO relais_commandes (id, code, jeton, champs, fichiers, etape, cree)
     VALUES (?, ?, ?, ?, ?, 'en_ligne', ?)`
  ).bind(commande, b.code, jeton, JSON.stringify(champs), JSON.stringify(fichiers), new Date().toISOString()).run();
  // Le numéro est celui du guichet : il arrive quand le PC a récupéré la
  // commande, et la page l'affiche alors d'elle-même (voir /statut).
  return json({ numero: null, jetons: [jeton], en_ligne: true });
}

async function statutClient(env, b, jeton) {
  if (!/^[0-9a-f]{32}$/.test(jeton)) return json({}, 404);
  const c = await env.DB.prepare("SELECT numero, etape, paye, message FROM relais_commandes WHERE code = ? AND jeton = ?")
    .bind(b.code, jeton).first();
  if (!c) return json({ traite: false, paye: false });
  return json({
    numero: c.numero || null,
    etape: c.etape === "en_ligne" ? "en_ligne" : c.etape,
    paye: !!c.paye,
    message: c.message || null,
    traite: c.etape !== "en_ligne",
  });
}

// ───────────────────────────── Côté PC ─────────────────────────────

async function pcAutorise(request, env, code) {
  const b = await boutique(env, code);
  const secret = request.headers.get("x-secret") || "";
  if (!b || secret.length < 32) return null;
  return egalConstant(await empreinte(secret), b.empreinte_secret) ? b : null;
}

async function inscrire(request, env) {
  let corps;
  try { corps = await request.json(); } catch { return json({ erreur: "illisible" }, 400); }
  const code = String(corps.code || "");
  const secret = String(corps.secret || "");
  if (!CODE_VALIDE.test(code) || !/^[0-9a-f]{64}$/.test(secret)) return json({ erreur: "code ou secret invalide" }, 400);
  const nom = String(corps.nom || "").slice(0, 80);
  const tarifs = JSON.stringify(corps.tarifs && typeof corps.tarifs === "object" ? corps.tarifs : {}).slice(0, 4000);
  const e = await empreinte(secret);
  const existant = await env.DB.prepare("SELECT empreinte_secret FROM relais_boutiques WHERE code = ?").bind(code).first();
  const maintenant = new Date().toISOString();
  if (!existant) {
    await env.DB.prepare(
      `INSERT INTO relais_boutiques (code, empreinte_secret, nom, tarifs, cree, vu, actif, octets_attente)
       VALUES (?, ?, ?, ?, ?, ?, 1, 0)`
    ).bind(code, e, nom, tarifs, maintenant, maintenant).run();
  } else if (!egalConstant(e, existant.empreinte_secret)) {
    // Code déjà pris par une autre boutique : le PC en tire un autre.
    return json({ erreur: "code pris" }, 409);
  } else {
    await env.DB.prepare("UPDATE relais_boutiques SET nom = ?, tarifs = ?, vu = ? WHERE code = ?")
      .bind(nom, tarifs, maintenant, code).run();
  }
  await env.DB.prepare(
    `INSERT INTO relais_fiches (code, fiche, bloques) VALUES (?, ?, ?)
     ON CONFLICT(code) DO UPDATE SET fiche = excluded.fiche, bloques = excluded.bloques`
  ).bind(code, JSON.stringify(ficheNettoyee(corps.fiche)), JSON.stringify(bloquesNettoyes(corps.bloques))).run();
  // Le jour où le service a son nom de domaine (wrangler.site.toml, [vars]),
  // les PC le mettent dans leurs QR ; les anciennes adresses restent valables.
  return json({
    ok: true,
    code,
    adresse_publique: /^https:\/\/[a-z0-9.-]+$/i.test(env.ADRESSE_PUBLIQUE || "") ? env.ADRESSE_PUBLIQUE : null,
    relais: /^https:\/\/[a-z0-9.-]+$/i.test(env.ADRESSE_RELAIS || "") ? env.ADRESSE_RELAIS : null,
  });
}

/** Fiche publique de la boutique : seulement du texte court, et un lien de
 * plan en https. Rien de ce qui vient du PC n'est servi tel quel. */
function ficheNettoyee(f) {
  if (!f || typeof f !== "object") return {};
  const texte = (v, n) => String(v || "").replace(/[\u0000-\u001f<>]/g, " ").trim().slice(0, n);
  const plan = texte(f.plan, 300);
  return {
    adresse: texte(f.adresse, 150),
    horaires: texte(f.horaires, 150),
    telephone: texte(f.telephone, 30).replace(/[^0-9+ ]/g, ""),
    plan: /^https:\/\/[^\s"'<>]+$/.test(plan) ? plan : "",
    devise: texte(f.devise, 8),
  };
}

/** Les 8 derniers chiffres suffisent à reconnaître un numéro, qu'il soit
 * écrit avec ou sans indicatif du pays. */
const finDeNumero = (t) => String(t || "").replace(/[^0-9]/g, "").slice(-8);
function bloquesNettoyes(l) {
  return [...new Set((Array.isArray(l) ? l : []).map(finDeNumero).filter((n) => n.length === 8))].slice(0, 1000);
}

async function enAttente(env, b) {
  // Une écriture à chaque visite (toutes les 15 s par PC) aurait épuisé le
  // quota gratuit d'écritures (100 000 par jour) vers 17 boutiques. Au plus
  // une toutes les 2 minutes : assez pour dire au client si le PC est là.
  const maintenant = Date.now();
  if (!b.vu || maintenant - Date.parse(b.vu) > 120 * 1000) {
    await env.DB.prepare("UPDATE relais_boutiques SET vu = ? WHERE code = ?").bind(new Date(maintenant).toISOString(), b.code).run();
  }
  const r = await env.DB.prepare(
    "SELECT id, champs, fichiers, cree FROM relais_commandes WHERE code = ? AND etape = 'en_ligne' ORDER BY cree LIMIT 10"
  ).bind(b.code).all();
  return json({
    commandes: (r.results || []).map((c) => ({
      id: c.id,
      cree: c.cree,
      champs: JSON.parse(c.champs || "{}"),
      fichiers: JSON.parse(c.fichiers || "[]").map((f) => ({ champ: f.champ, nom: f.nom, taille: f.taille })),
    })),
  });
}

async function fichierPourPc(env, b, id, champ) {
  const c = await env.DB.prepare("SELECT fichiers FROM relais_commandes WHERE code = ? AND id = ?").bind(b.code, id).first();
  if (!c) return new Response("introuvable", { status: 404 });
  const f = JSON.parse(c.fichiers || "[]").find((x) => x.champ === champ);
  if (!f) return new Response("introuvable", { status: 404 });
  if (f.type === "direct") {
    const o = await env.TELECHARGEMENTS.get(f.cle);
    if (!o) return new Response("introuvable", { status: 404 });
    return new Response(o.body, { headers: { "content-type": "application/octet-stream", "content-length": String(o.size) } });
  }
  const { utiles, recu } = await morceaux(env, b.code, f.id);
  // Les morceaux mis bout à bout, sans tout charger en mémoire.
  const { readable, writable } = new FixedLengthStream(recu);
  (async () => {
    for (const o of utiles) {
      const objet = await env.TELECHARGEMENTS.get(o.key);
      if (!objet) break;
      await objet.body.pipeTo(writable, { preventClose: true });
    }
    await writable.close();
  })().catch(() => writable.abort?.());
  return new Response(readable, { headers: { "content-type": "application/octet-stream", "content-length": String(recu) } });
}

async function accuser(request, env, b, id) {
  let corps = {};
  try { corps = await request.json(); } catch {}
  const c = await env.DB.prepare("SELECT * FROM relais_commandes WHERE code = ? AND id = ?").bind(b.code, id).first();
  if (!c) return json({ ok: true });
  await effacerCommande(env, b.code, c);
  await env.DB.prepare(
    "UPDATE relais_commandes SET etape = 'recu', numero = ?, recupere = ?, fichiers = '[]' WHERE id = ?"
  ).bind(String(corps.numero || "").slice(0, 20) || null, new Date().toISOString(), id).run();
  return json({ ok: true });
}

async function majEtat(request, env, b, id) {
  let corps = {};
  try { corps = await request.json(); } catch {}
  const etape = ["recu", "impression", "pret"].includes(corps.etape) ? corps.etape : null;
  await env.DB.prepare(
    "UPDATE relais_commandes SET etape = COALESCE(?, etape), paye = ?, message = ? WHERE code = ? AND id = ? AND etape != 'en_ligne'"
  ).bind(etape, corps.paye ? 1 : 0, corps.message ? String(corps.message).slice(0, 300) : null, b.code, id).run();
  return json({ ok: true });
}

// ───────────────────────────── Nettoyage ─────────────────────────────

/**
 * Au plus une fois par heure, déclenché par la visite d'un PC (pas de tâche
 * planifiée : le compte a atteint la limite gratuite de 5). Les PC passent
 * toutes les 15 s : l'heure est tenue à quelques secondes près dès qu'une
 * boutique est allumée.
 */
export async function nettoyerSiBesoin(env) {
  const maintenant = Date.now();
  const ligne = await env.DB.prepare("SELECT valeur FROM parametres WHERE cle = 'relais_dernier_nettoyage'").first();
  if (ligne && maintenant - Number(ligne.valeur) < 3600 * 1000) return;
  // Noté AVANT de nettoyer : deux PC qui arrivent ensemble ne lancent pas
  // deux nettoyages.
  await env.DB.prepare(
    "INSERT INTO parametres (cle, valeur) VALUES ('relais_dernier_nettoyage', ?) ON CONFLICT(cle) DO UPDATE SET valeur = excluded.valeur"
  ).bind(String(maintenant)).run();
  await nettoyer(env);
}

/** Rien ne reste plus de 24 heures. */
export async function nettoyer(env) {
  const limite = new Date(Date.now() - DUREE_DE_VIE_MS).toISOString();
  const vieilles = await env.DB.prepare(
    "SELECT * FROM relais_commandes WHERE etape = 'en_ligne' AND cree < ? LIMIT 200"
  ).bind(limite).all();
  for (const c of vieilles.results || []) {
    await effacerCommande(env, c.code, c);
    await env.DB.prepare("UPDATE relais_commandes SET etape = 'expire', fichiers = '[]' WHERE id = ?").bind(c.id).run();
  }
  // Morceaux envoyés sans commande (client parti en route).
  let curseur;
  const limiteDate = new Date(Date.now() - DUREE_DE_VIE_MS);
  do {
    const r = await env.TELECHARGEMENTS.list({ prefix: "relais/", cursor: curseur });
    for (const o of r.objects) {
      if (o.uploaded < limiteDate) {
        await env.TELECHARGEMENTS.delete(o.key);
        const code = o.key.split("/")[1];
        if (CODE_VALIDE.test(code)) await compterAttente(env, code, -o.size);
      }
    }
    curseur = r.truncated ? r.cursor : undefined;
  } while (curseur);
  // Le compteur d'attente de chaque boutique, recalculé d'après ce qui est
  // réellement stocké : aucune dérive ne peut durer plus d'une heure.
  const reel = new Map();
  let suite;
  do {
    const r = await env.TELECHARGEMENTS.list({ prefix: "relais/", cursor: suite });
    for (const o of r.objects) {
      const code = o.key.split("/")[1];
      reel.set(code, (reel.get(code) || 0) + o.size);
    }
    suite = r.truncated ? r.cursor : undefined;
  } while (suite);
  await env.DB.prepare("UPDATE relais_boutiques SET octets_attente = 0 WHERE octets_attente != 0").run();
  for (const [code, octets] of reel) {
    if (CODE_VALIDE.test(code)) {
      await env.DB.prepare("UPDATE relais_boutiques SET octets_attente = ? WHERE code = ?").bind(octets, code).run();
    }
  }
  // Les boutiques fictives de l'essai automatique (essai-relais.yml).
  await env.DB.prepare(
    "DELETE FROM relais_boutiques WHERE ((nom = 'Boutique d''essai Internet' AND code LIKE 'E%') OR (nom LIKE 'Charge %' AND code LIKE 'CHG%' AND length(code) = 9)) AND cree < ?"
  ).bind(new Date(Date.now() - 24 * 3600 * 1000).toISOString()).run();
  // Les commandes traitées depuis plus de 7 jours : plus rien à suivre.
  await env.DB.prepare("DELETE FROM relais_commandes WHERE cree < ?")
    .bind(new Date(Date.now() - 7 * 24 * 3600 * 1000).toISOString()).run();
}

// ───────────────────────────── Aiguillage ─────────────────────────────

/** Rend une réponse si l'adresse appartient au relais, sinon null. */
export async function relais(request, env) {
  const url = new URL(request.url);
  const p = url.pathname;
  const m = request.method;
  if (m === "OPTIONS") {
    return new Response(null, {
      status: 204,
      headers: {
        "access-control-allow-origin": "*",
        "access-control-allow-methods": "GET, POST, OPTIONS",
        "access-control-allow-headers": "*",
      },
    });
  }

  let r = p.match(/^\/b\/([0-9A-Za-z]{6,10})(\/.*)?$/);
  if (r) {
    const b = await boutique(env, r[1].toUpperCase());
    if (!b) {
      return new Response(
        "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width'><title>Boutique introuvable</title><body style='font-family:sans-serif;padding:2em;text-align:center'><h1>Boutique introuvable</h1><p>Ce code ne correspond à aucune boutique en service. Demandez au guichet de réafficher son code.</p>",
        { status: 404, headers: { "content-type": "text/html; charset=utf-8" } }
      );
    }
    const reste = r[2] || "/";
    if (r[1] !== b.code) return Response.redirect(`${url.origin}/b/${b.code}${reste}`, 301);
    if (m === "GET" && (reste === "/" || reste === "")) return pageClient(b);
    if (m === "GET" && reste === "/langue-en.json") {
      return new Response(LANGUE_EN_TEXTE, { headers: { "content-type": "application/json; charset=utf-8", "cache-control": "public, max-age=3600" } });
    }
    if (m === "GET" && reste === "/infos") {
      let tarifs = {};
      try { tarifs = JSON.parse(b.tarifs || "{}"); } catch {}
      const pcEnLigne = !!b.vu && Date.now() - Date.parse(b.vu) < 6 * 60 * 1000;
      let fiche = {};
      try { fiche = JSON.parse((await env.DB.prepare("SELECT fiche FROM relais_fiches WHERE code = ?").bind(b.code).first())?.fiche || "{}"); } catch {}
      return json({ boutique: b.nom || "", tarifs, fiche, en_ligne: true, pc_en_ligne: pcEnLigne });
    }
    let s = reste.match(/^\/morceau\/([0-9a-f]{12,40})$/);
    if (s && m === "GET") return json({ recu: (await morceaux(env, b.code, s[1])).recu });
    if (s && m === "POST") {
      const debut = Number(url.searchParams.get("debut") || -1);
      if (!Number.isSafeInteger(debut) || debut < 0) return json({ erreur: "début invalide" }, 400);
      return recevoirMorceau(request, env, b, s[1], debut);
    }
    if (reste === "/envoyer" && m === "POST") return recevoirCommande(request, env, b);
    s = reste.match(/^\/statut\/([0-9a-f]+)$/);
    if (s && m === "GET") return statutClient(env, b, s[1]);
    return json({}, 404);
  }

  if (p === "/api/relais/inscrire" && m === "POST") return inscrire(request, env);
  r = p.match(/^\/api\/relais\/([0-9A-Z]{6,10})\/(attente|fichier|accuse|etat)(?:\/([0-9a-f]{12,40}))?(?:\/([a-z]+_\d{1,2}))?$/);
  if (r) {
    const b = await pcAutorise(request, env, r[1]);
    if (!b) return json({ erreur: "non autorisé" }, 401);
    if (r[2] === "attente" && m === "GET") return enAttente(env, b);
    if (r[2] === "fichier" && m === "GET" && r[3] && r[4]) return fichierPourPc(env, b, r[3], r[4]);
    if (r[2] === "accuse" && m === "POST" && r[3]) return accuser(request, env, b, r[3]);
    if (r[2] === "etat" && m === "POST" && r[3]) return majEtat(request, env, b, r[3]);
    return json({}, 404);
  }
  return null;
}
