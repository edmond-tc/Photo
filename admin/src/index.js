// Tableau de bord administrateur pour le porteur du projet "Gestion
// Photocopie" — suivi des boutiques déployées et génération des clés de
// licence. Ne reçoit jamais de données des PC des boutiques (ceux-ci ne
// sont jamais connectés à internet) : tout ce qui est ici est saisi par
// le porteur du projet lui-même, depuis son téléphone ou son PC.

const ALPHABET_CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

// ───────────────────────── Licence : doit produire EXACTEMENT les mêmes
// clés que src-tauri/src/license.rs (même secret, même algorithme), sinon
// l'application du gérant refuserait une clé générée ici. ─────────────────

function crockfordBase32(octets) {
  let bits = 0;
  let valeur = 0;
  let sortie = "";
  for (const octet of octets) {
    valeur = (valeur << 8) | octet;
    bits += 8;
    while (bits >= 5) {
      sortie += ALPHABET_CROCKFORD[(valeur >>> (bits - 5)) & 31];
      bits -= 5;
    }
  }
  if (bits > 0) {
    sortie += ALPHABET_CROCKFORD[(valeur << (5 - bits)) & 31];
  }
  return sortie;
}

async function hmacSha256(secret, texte) {
  const enc = new TextEncoder();
  const cle = await crypto.subtle.importKey(
    "raw",
    enc.encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"]
  );
  const signatureBrute = await crypto.subtle.sign("HMAC", cle, enc.encode(texte));
  return new Uint8Array(signatureBrute);
}

function formatDateCompacte(date) {
  const y = date.getUTCFullYear();
  const m = String(date.getUTCMonth() + 1).padStart(2, "0");
  const d = String(date.getUTCDate()).padStart(2, "0");
  return `${y}${m}${d}`;
}

/// La clé privée (64 caractères hexadécimaux) est un secret du Worker. Elle
/// seule permet de fabriquer une licence ; l'application, elle, ne contient
/// que la clé publique correspondante, qui ne sait que vérifier. C'est ce qui
/// empêche désormais quiconque possède l'exe de se fabriquer des licences.
function decoderHex(hex) {
  const propre = String(hex ?? "").trim();
  if (!/^[0-9a-fA-F]{64}$/.test(propre)) {
    throw new Error("CLE_PRIVEE_LICENCE absente ou mal formée (64 caractères hexadécimaux attendus).");
  }
  const octets = new Uint8Array(32);
  for (let i = 0; i < 32; i++) octets[i] = parseInt(propre.slice(i * 2, i * 2 + 2), 16);
  return octets;
}

/// Ed25519 attend une clé privée au format PKCS#8 : on enveloppe les 32
/// octets bruts dans l'en-tête ASN.1 fixe correspondant.
const PREFIXE_PKCS8_ED25519 = new Uint8Array([
  0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
]);

async function signerEd25519(cleHex, message) {
  const graine = decoderHex(cleHex);
  const pkcs8 = new Uint8Array(PREFIXE_PKCS8_ED25519.length + graine.length);
  pkcs8.set(PREFIXE_PKCS8_ED25519, 0);
  pkcs8.set(graine, PREFIXE_PKCS8_ED25519.length);

  const cle = await crypto.subtle.importKey("pkcs8", pkcs8, { name: "Ed25519" }, false, ["sign"]);
  const signature = await crypto.subtle.sign({ name: "Ed25519" }, cle, new TextEncoder().encode(message));
  return new Uint8Array(signature);
}

async function genererCle(env, machineId, jours) {
  const expiration = new Date();
  expiration.setUTCDate(expiration.getUTCDate() + Number(jours));
  const expirationCompacte = formatDateCompacte(expiration);
  const signature = await signerEd25519(env.CLE_PRIVEE_LICENCE, `${machineId}|${expirationCompacte}`);
  return {
    cle: `${crockfordBase32(signature)}-${expirationCompacte}`,
    dateExpiration: expirationCompacte,
  };
}

// ───────────────────────────── Session admin ─────────────────────────────
// Pas de table de sessions : le cookie porte lui-même sa date d'expiration
// et un numéro tiré au hasard, le tout signé avec le mot de passe admin.
// Seul quelqu'un qui connaît déjà le mot de passe peut en fabriquer un.
//
// La date d'expiration et le tirage au sort comptent : un cookie qui vaut
// toujours la même chose ne peut jamais être révoqué, et s'il fuite une
// seule fois (téléphone prêté, historique de navigateur, sauvegarde) il
// donne un accès administrateur permanent.

const DUREE_SESSION_MS = 30 * 24 * 60 * 60 * 1000;

/// Comparaison à durée constante : comparer deux chaînes avec === s'arrête
/// au premier caractère différent, ce qui laisse deviner un secret à la
/// vitesse de réponse.
function comparaisonConstante(a, b) {
  if (typeof a !== "string" || typeof b !== "string" || a.length !== b.length) return false;
  let difference = 0;
  for (let i = 0; i < a.length; i++) difference |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return difference === 0;
}

async function signer(env, texte) {
  return crockfordBase32(await hmacSha256(env.ADMIN_PASSWORD, texte));
}

/// Compare deux mots de passe par leurs empreintes : de longueurs toujours
/// identiques, elles ne trahissent donc même pas la longueur du vrai mot de
/// passe.
async function motDePasseValide(env, fourni) {
  const attendu = crockfordBase32(await hmacSha256(env.ADMIN_PASSWORD, "verification"));
  const propose = crockfordBase32(await hmacSha256(String(fourni ?? ""), "verification"));
  return comparaisonConstante(attendu, propose);
}

async function creerCookieSession(env) {
  const expiration = Date.now() + DUREE_SESSION_MS;
  const alea = crypto.randomUUID().replace(/-/g, "");
  const charge = `${expiration}.${alea}`;
  return `${charge}.${await signer(env, charge)}`;
}

function lireCookie(request, nom) {
  const cookies = request.headers.get("Cookie") || "";
  const match = cookies.match(new RegExp(`(?:^|;\\s*)${nom}=([^;]+)`));
  return match ? match[1] : null;
}

async function estConnecte(request, env) {
  const cookie = lireCookie(request, "session");
  if (!cookie) return false;
  const parties = cookie.split(".");
  if (parties.length !== 3) return false;
  const [expiration, alea, signature] = parties;
  if (!/^\d+$/.test(expiration) || Number(expiration) < Date.now()) return false;
  return comparaisonConstante(signature, await signer(env, `${expiration}.${alea}`));
}

// ───────────────── Protection contre les essais de mot de passe ──────────
// Sans limite, le mot de passe qui protège la génération des licences peut
// être essayé des milliers de fois par minute depuis n'importe où.

const TENTATIVES_MAX = 8;
const FENETRE_TENTATIVES_MS = 15 * 60 * 1000;

function adresseAppelant(request) {
  return request.headers.get("CF-Connecting-IP") || "inconnue";
}

async function tentativesRecentes(env, ip) {
  try {
    const brut = await obtenirParametre(env, `tentatives:${ip}`, "");
    if (!brut) return 0;
    const { n, debut } = JSON.parse(brut);
    if (Date.now() - debut > FENETRE_TENTATIVES_MS) return 0;
    return n;
  } catch {
    // Compteur illisible ou base indisponible : on ne bloque pas le porteur
    // du projet hors de son propre tableau de bord pour autant.
    return 0;
  }
}

async function enregistrerEchec(env, ip) {
  try {
    const brut = await obtenirParametre(env, `tentatives:${ip}`, "");
    let compteur = { n: 0, debut: Date.now() };
    if (brut) {
      const precedent = JSON.parse(brut);
      if (Date.now() - precedent.debut <= FENETRE_TENTATIVES_MS) compteur = precedent;
    }
    compteur.n += 1;
    await definirParametre(env, `tentatives:${ip}`, JSON.stringify(compteur));
  } catch {
    // Rien à faire de plus : l'échec de comptage ne doit pas casser la page.
  }
}

async function oublierTentatives(env, ip) {
  try {
    await env.DB.prepare(`DELETE FROM parametres WHERE cle = ?`).bind(`tentatives:${ip}`).run();
  } catch {
    // Sans importance : le compteur expirera de lui-même.
  }
}

// ─────────────────────────────── Paramètres ──────────────────────────────

async function obtenirParametre(env, cle, defaut = "") {
  const ligne = await env.DB.prepare(`SELECT valeur FROM parametres WHERE cle = ?`).bind(cle).first();
  return ligne?.valeur ?? defaut;
}

async function definirParametre(env, cle, valeur) {
  await env.DB.prepare(
    `INSERT INTO parametres (cle, valeur) VALUES (?, ?) ON CONFLICT(cle) DO UPDATE SET valeur = excluded.valeur`
  )
    .bind(cle, valeur)
    .run();
}

// ───────────────────────────────── HTML ──────────────────────────────────

function echapper(valeur) {
  return String(valeur ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function page(titre, corps, { connecte = true } = {}) {
  return `<!doctype html>
<html lang="fr">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1.0" />
<title>${echapper(titre)} · Admin Gestion Photocopie</title>
<style>
  :root { color-scheme: light; }
  body { font-family: "Segoe UI", Calibri, Arial, sans-serif; background:#f3f2f1; margin:0; padding:1.25rem; color:#323130; }
  .carte { max-width: 560px; margin: 0 auto 1rem; background:#fff; border-radius:8px; padding:1.25rem; box-shadow: 0 1px 4px rgba(0,0,0,0.08); }
  h1 { font-size:1.2rem; color:#2b579a; margin:0 0 1rem; }
  h2 { font-size:1rem; margin: 0 0 0.5rem; }
  a { color:#2b579a; }
  label { display:block; font-size:0.85rem; margin-bottom:0.75rem; }
  input, select, textarea { width:100%; padding:0.55rem; margin-top:0.25rem; border:1px solid #d6d4d1; border-radius:4px; font-size:1rem; box-sizing:border-box; font-family:inherit; }
  button, .btn { display:inline-block; padding:0.6rem 1rem; background:#2b579a; color:#fff; border:none; border-radius:4px; font-size:0.95rem; font-weight:600; cursor:pointer; text-decoration:none; text-align:center; }
  button.secondaire, .btn.secondaire { background:#fff; color:#2b579a; border:1px solid #2b579a; }
  button.danger { background:#a4262c; }
  table { width:100%; border-collapse:collapse; font-size:0.9rem; }
  th, td { text-align:left; padding:0.5rem 0.4rem; border-bottom:1px solid #eee; }
  .badge { font-size:0.75rem; padding:0.2rem 0.5rem; border-radius:999px; display:inline-block; }
  .badge-actif { background:#dff6dd; color:#0e5c1f; }
  .badge-bientot { background:#fff4ce; color:#614300; }
  .badge-expire { background:#fde7e9; color:#a4262c; }
  .badge-desactive { background:#e8e6e4; color:#605e5c; }
  .topbar { max-width:560px; margin:0 auto 1rem; display:flex; justify-content:space-between; align-items:center; }
  .topbar a { font-size:0.85rem; }
  .cle-resultat { font-family: "Consolas", monospace; font-size:1.1rem; background:#f3f2f1; padding:0.75rem; border-radius:4px; word-break:break-all; margin:0.5rem 0; }
</style>
</head>
<body>
${connecte ? `<div class="topbar"><strong>Admin Gestion Photocopie</strong><a href="/deconnexion">Se déconnecter</a></div>` : ""}
${corps}
</body>
</html>`;
}

function badgeStatut(dateExpirationCompacte, desactive) {
  if (desactive) return `<span class="badge badge-desactive">Désactivé (note)</span>`;
  if (!dateExpirationCompacte) return `<span class="badge badge-expire">Aucune licence</span>`;
  const aujourdhui = formatDateCompacte(new Date());
  const dansSeptJours = formatDateCompacte(new Date(Date.now() + 7 * 86400000));
  if (dateExpirationCompacte < aujourdhui) return `<span class="badge badge-expire">Expiré</span>`;
  if (dateExpirationCompacte <= dansSeptJours) return `<span class="badge badge-bientot">Expire bientôt</span>`;
  return `<span class="badge badge-actif">Actif</span>`;
}

function formatDateLisible(compacte) {
  if (!compacte || compacte.length !== 8) return "—";
  return `${compacte.slice(6, 8)}/${compacte.slice(4, 6)}/${compacte.slice(0, 4)}`;
}

// ───────────────────────────────── Pages ─────────────────────────────────

async function pageConnexion(erreur) {
  return page(
    "Connexion",
    `<div class="carte" style="margin-top:3rem">
      <h1>Admin Gestion Photocopie</h1>
      ${erreur ? `<p style="color:#a4262c">${echapper(erreur)}</p>` : ""}
      <form method="POST" action="/connexion">
        <label>Mot de passe <input type="password" name="mot_de_passe" required autofocus /></label>
        <button type="submit">Se connecter</button>
      </form>
    </div>`,
    { connecte: false }
  );
}

async function pageAccueil(env) {
  const { results: boutiques } = await env.DB.prepare(
    `SELECT b.id, b.nom, b.gerant_nom, b.telephone, b.machine_id, b.abonnement_desactive,
            l.date_expiration
     FROM boutiques b
     LEFT JOIN licences l ON l.id = (
       SELECT id FROM licences WHERE boutique_id = b.id ORDER BY date_expiration DESC LIMIT 1
     )
     ORDER BY (l.date_expiration IS NULL) DESC, l.date_expiration ASC`
  ).all();

  const aujourdhui = formatDateCompacte(new Date());
  const dansSeptJours = formatDateCompacte(new Date(Date.now() + 7 * 86400000));
  const alertes = boutiques.filter(
    (b) =>
      !b.abonnement_desactive &&
      b.date_expiration &&
      b.date_expiration <= dansSeptJours
  );

  const lignesAlertes = alertes
    .map(
      (b) => `<li>
        <a href="/boutiques/${b.id}">${echapper(b.nom)}</a>
        — ${b.date_expiration < aujourdhui ? "expiré" : "expire"} le ${formatDateLisible(b.date_expiration)}
        ${b.telephone ? ` · <a href="tel:${echapper(b.telephone)}">${echapper(b.telephone)}</a>` : ""}
      </li>`
    )
    .join("");

  const lignesBoutiques = boutiques
    .map(
      (b) => `<tr>
        <td><a href="/boutiques/${b.id}">${echapper(b.nom)}</a><br><span style="font-size:0.8rem; color:#605e5c">${echapper(b.gerant_nom)}</span></td>
        <td>${badgeStatut(b.date_expiration, b.abonnement_desactive)}</td>
        <td>${formatDateLisible(b.date_expiration)}</td>
      </tr>`
    )
    .join("");

  const { results: demandes } = await env.DB.prepare(
    `SELECT * FROM demandes WHERE statut = 'en_attente' ORDER BY created_at ASC`
  ).all();

  const lignesDemandes = demandes
    .map(
      (d) => `<li style="margin-bottom:0.5rem">
        <strong>${echapper(d.nom_boutique)}</strong> — ${echapper(d.jours_demandes)} jours,
        payé au ${echapper(d.numero_paiement)}${d.telephone ? ` (tél. ${echapper(d.telephone)})` : ""}
        <br><span style="font-size:0.75rem; color:#605e5c">Machine : <code>${echapper(d.machine_id)}</code></span>
        ${d.commentaire ? `<br><span style="font-size:0.8rem; color:#605e5c">${echapper(d.commentaire)}</span>` : ""}
        <br>
        <form method="POST" action="/demandes/${d.id}/confirmer" style="display:inline">
          <button type="submit" style="font-size:0.8rem; padding:0.3rem 0.6rem">✅ Confirmer et générer la clé</button>
        </form>
        <form method="POST" action="/demandes/${d.id}/rejeter" style="display:inline">
          <button type="submit" class="secondaire" style="font-size:0.8rem; padding:0.3rem 0.6rem">Rejeter</button>
        </form>
      </li>`
    )
    .join("");

  return page(
    "Boutiques",
    `<div class="carte">
      <h1>Boutiques (${boutiques.length})</h1>
      <a class="btn" href="/boutiques/nouvelle">+ Ajouter une boutique</a>
      <a class="btn secondaire" href="/parametres">Paramètres</a>
      <a class="btn secondaire" href="/telecharger" target="_blank">Page de téléchargement ↗</a>
      <a class="btn secondaire" href="/nouveautes">Nouveautés</a>
    </div>
    ${
      demandes.length
        ? `<div class="carte" style="border-left:4px solid #2b579a">
            <h2>📩 Demandes de renouvellement en attente (${demandes.length})</h2>
            <ul style="margin:0; padding-left:1.2rem; font-size:0.9rem">${lignesDemandes}</ul>
          </div>`
        : ""
    }
    ${
      alertes.length
        ? `<div class="carte" style="border-left:4px solid #a4262c">
            <h2>⚠️ À relancer bientôt</h2>
            <ul style="margin:0; padding-left:1.2rem; font-size:0.9rem">${lignesAlertes}</ul>
          </div>`
        : ""
    }
    <div class="carte">
      <table>
        <thead><tr><th>Boutique</th><th>Statut</th><th>Expire le</th></tr></thead>
        <tbody>${lignesBoutiques || `<tr><td colspan="3">Aucune boutique enregistrée pour l'instant.</td></tr>`}</tbody>
      </table>
    </div>`
  );
}

async function pageNouvelleBoutique(erreur) {
  return page(
    "Ajouter une boutique",
    `<div class="carte">
      <h1>Ajouter une boutique</h1>
      ${erreur ? `<p style="color:#a4262c">${echapper(erreur)}</p>` : ""}
      <form method="POST" action="/boutiques/nouvelle">
        <label>Nom de la boutique <input type="text" name="nom" required /></label>
        <label>Nom du gérant <input type="text" name="gerant_nom" /></label>
        <label>Téléphone <input type="tel" name="telephone" /></label>
        <label>Identifiant machine (lu dans Réglages > Licence sur le PC du gérant) <input type="text" name="machine_id" required /></label>
        <label>Notes <textarea name="notes" rows="2"></textarea></label>
        <button type="submit">Enregistrer</button>
        <a class="btn secondaire" href="/">Annuler</a>
      </form>
    </div>`
  );
}

async function pageBoutique(env, id, { cleGeneree } = {}) {
  const boutique = await env.DB.prepare(`SELECT * FROM boutiques WHERE id = ?`).bind(id).first();
  if (!boutique) return null;

  const { results: licences } = await env.DB.prepare(
    `SELECT * FROM licences WHERE boutique_id = ? ORDER BY created_at DESC`
  )
    .bind(id)
    .all();

  const derniereExpiration = licences[0]?.date_expiration;

  const { results: rapports } = await env.DB.prepare(
    `SELECT * FROM rapports WHERE boutique_id = ? ORDER BY created_at DESC LIMIT 10`
  )
    .bind(id)
    .all();

  const blocsRapports = rapports
    .map((r) => {
      let contenu;
      try {
        contenu = JSON.parse(r.contenu_json);
      } catch {
        contenu = null;
      }
      if (!contenu) return `<div class="carte" style="padding:0.75rem"><p>Rapport illisible.</p></div>`;
      return `<div class="carte" style="padding:0.75rem; margin-bottom:0.5rem">
        <p style="font-size:0.8rem; color:#605e5c; margin:0 0 0.4rem">Reçu le ${new Date(r.created_at).toLocaleDateString("fr-FR")}</p>
        <table>
          <tr><td>Version appli</td><td>${echapper(contenu.version)}</td></tr>
          <tr><td>Licence</td><td>${echapper(contenu.statut_licence)} (${echapper(contenu.jours_restants)} j restants)</td></tr>
          <tr><td>Dernière sauvegarde</td><td>${contenu.derniere_sauvegarde ? new Date(contenu.derniere_sauvegarde).toLocaleString("fr-FR") : "aucune"}</td></tr>
          <tr><td>Dernier fichier reçu</td><td>${contenu.dernier_fichier_recu ? new Date(contenu.dernier_fichier_recu).toLocaleString("fr-FR") : "aucun"}</td></tr>
          <tr><td>Transactions au total</td><td>${echapper(contenu.nombre_transactions_total ?? "—")}</td></tr>
          <tr><td>Taille de la base</td><td>${contenu.taille_base_octets ? Math.round(contenu.taille_base_octets / 1024) + " Ko" : "—"}</td></tr>
        </table>
      </div>`;
    })
    .join("");

  const lignesLicences = licences
    .map(
      (l) => `<tr>
        <td class="cle-resultat" style="font-size:0.8rem; padding:0.3rem">${echapper(l.cle)}</td>
        <td>${l.jours} j</td>
        <td>${formatDateLisible(l.date_expiration)}</td>
      </tr>`
    )
    .join("");

  return page(
    boutique.nom,
    `<div class="carte">
      <p><a href="/">&larr; Toutes les boutiques</a></p>
      <h1>${echapper(boutique.nom)} ${badgeStatut(derniereExpiration, boutique.abonnement_desactive)}</h1>
      <p style="font-size:0.9rem; color:#605e5c">
        ${echapper(boutique.gerant_nom) || "—"}
        ${boutique.telephone ? ` · <a href="tel:${echapper(boutique.telephone)}">${echapper(boutique.telephone)}</a>` : ""}
      </p>
      <p style="font-size:0.85rem"><strong>Identifiant machine :</strong> ${echapper(boutique.machine_id)}</p>
      ${boutique.notes ? `<p style="font-size:0.85rem">${echapper(boutique.notes)}</p>` : ""}
      <form method="POST" action="/boutiques/${id}/desactiver" style="display:inline">
        <button type="submit" class="${boutique.abonnement_desactive ? "secondaire" : "danger"}">
          ${boutique.abonnement_desactive ? "Marquer comme actif (note)" : "Marquer comme désactivé (note)"}
        </button>
      </form>
      <p style="font-size:0.75rem; color:#605e5c; margin-top:0.4rem">
        Rappel : ceci est juste une note pour toi. Le PC de la boutique n'étant jamais connecté à
        internet, rien ne peut être coupé à distance — l'appli continue de fonctionner jusqu'à la
        date déjà écrite dans sa dernière clé.
      </p>
    </div>

    <div class="carte">
      <h2>Générer une nouvelle clé</h2>
      ${
        cleGeneree
          ? `<p style="color:#0e5c1f">Clé générée, valable jusqu'au ${formatDateLisible(cleGeneree.dateExpiration)} :</p>
             <p class="cle-resultat">${echapper(cleGeneree.cle)}</p>
             <p style="font-size:0.85rem">Copie-la et transmets-la au gérant (WhatsApp, SMS...) pour qu'il la colle dans Réglages &gt; Licence.</p>`
          : ""
      }
      <form method="POST" action="/boutiques/${id}/licence">
        <label>Durée
          <select name="jours">
            <option value="30">30 jours (essai / mensuel)</option>
            <option value="90">90 jours (trimestriel)</option>
            <option value="365">365 jours (annuel)</option>
          </select>
        </label>
        <button type="submit">Générer</button>
      </form>
    </div>

    <div class="carte">
      <h2>Historique des clés</h2>
      <table>
        <thead><tr><th>Clé</th><th>Durée</th><th>Expire le</th></tr></thead>
        <tbody>${lignesLicences || `<tr><td colspan="3">Aucune clé générée pour l'instant.</td></tr>`}</tbody>
      </table>
    </div>

    <div class="carte">
      <h2>Rapports de visite</h2>
      ${blocsRapports || `<p style="font-size:0.85rem; color:#605e5c">Aucun rapport importé pour l'instant.</p>`}
      <details>
        <summary style="cursor:pointer; font-size:0.9rem">+ Importer un nouveau rapport</summary>
        <p style="font-size:0.8rem; color:#605e5c">
          Colle ici le texte généré par le bouton "Générer le rapport" dans
          Réglages &gt; Rapport pour le porteur du projet, sur le PC de cette
          boutique.
        </p>
        <form method="POST" action="/boutiques/${id}/rapport">
          <textarea name="contenu_json" rows="6" placeholder='{"version": "0.1.0", ...}' required></textarea>
          <button type="submit" class="secondaire">Importer</button>
        </form>
      </details>
    </div>`
  );
}

async function pageRenouveler(env, { envoye, erreur } = {}) {
  const numeros = await obtenirParametre(env, "numeros_paiement", "");
  const contact = await obtenirParametre(env, "contact_whatsapp", "");
  const montant = await obtenirParametre(env, "montant_indicatif", "");

  const { results: nouveautes } = await env.DB.prepare(
    `SELECT * FROM nouveautes ORDER BY created_at DESC LIMIT 3`
  ).all();
  const blocNouveautes = nouveautes.length
    ? `<div class="carte" style="border-left:4px solid #0e5c1f">
        <h2>🎁 Ce que vous obtenez en renouvelant</h2>
        ${nouveautes
          .map(
            (n) => `<div style="margin-bottom:0.75rem">
              <strong>${echapper(n.titre)}</strong> <span style="font-size:0.75rem; color:#605e5c">(v${echapper(n.version)})</span>
              ${n.description ? `<p style="font-size:0.85rem; margin:0.2rem 0 0">${echapper(n.description)}</p>` : ""}
            </div>`
          )
          .join("")}
      </div>`
    : "";

  return page(
    "Renouveler mon abonnement",
    `${blocNouveautes}
    <div class="carte">
      <h1>Renouveler l'abonnement</h1>
      ${
        envoye
          ? `<p style="color:#0e5c1f">Demande envoyée. Le porteur du projet la vérifie et vous enverra
             votre nouvelle clé (WhatsApp ou appel) — patientez quelques jours si besoin.</p>
             ${contact ? `<p>En cas d'urgence : <a href="https://wa.me/${echapper(contact.replace(/[^0-9]/g, ""))}">WhatsApp</a> ou appelez le ${echapper(contact)}.</p>` : ""}`
          : `
      ${numeros ? `<p><strong>1. Payez</strong> ${montant ? `(${echapper(montant)})` : ""} au numéro Mobile Money :<br>${echapper(numeros).replace(/\n/g, "<br>")}</p>` : ""}
      ${contact ? `<p><strong>2. Besoin d'aide ?</strong> <a href="https://wa.me/${echapper(contact.replace(/[^0-9]/g, ""))}">WhatsApp</a> ou appelez le ${echapper(contact)}.</p>` : ""}
      <p><strong>3. Remplissez ce formulaire</strong> pour indiquer votre paiement :</p>
      ${erreur ? `<p style="color:#a4262c">${echapper(erreur)}</p>` : ""}
      <form method="POST" action="/renouveler">
        <label>Nom de la boutique <input type="text" name="nom_boutique" required /></label>
        <label>Identifiant machine (Réglages &gt; Licence, dans l'appli) <input type="text" name="machine_id" required /></label>
        <label>Votre téléphone <input type="tel" name="telephone" required /></label>
        <label>Numéro utilisé pour payer <input type="tel" name="numero_paiement" required /></label>
        <label>Durée souhaitée
          <select name="jours_demandes">
            <option value="30">30 jours</option>
            <option value="90">90 jours</option>
            <option value="365">365 jours</option>
          </select>
        </label>
        <label>Commentaire (optionnel) <textarea name="commentaire" rows="2"></textarea></label>
        <button type="submit">Envoyer ma demande</button>
      </form>`
      }
    </div>`,
    { connecte: false }
  );
}

async function pageNouveautes(env) {
  const { results: nouveautes } = await env.DB.prepare(
    `SELECT * FROM nouveautes ORDER BY created_at DESC`
  ).all();
  const lignes = nouveautes
    .map(
      (n) => `<div class="carte" style="padding:0.75rem">
        <strong>${echapper(n.titre)}</strong> <span style="font-size:0.75rem; color:#605e5c">v${echapper(n.version)} — ${new Date(n.created_at).toLocaleDateString("fr-FR")}</span>
        ${n.description ? `<p style="font-size:0.85rem; margin:0.3rem 0 0">${echapper(n.description)}</p>` : ""}
        <form method="POST" action="/nouveautes/${n.id}/supprimer" style="margin-top:0.4rem">
          <button type="submit" class="secondaire" style="font-size:0.8rem; padding:0.3rem 0.6rem">Supprimer</button>
        </form>
      </div>`
    )
    .join("");

  return page(
    "Nouveautés",
    `<div class="carte">
      <p><a href="/">&larr; Retour</a></p>
      <h1>Nouveautés</h1>
      <p style="font-size:0.85rem; color:#605e5c">
        Affichées aux gérants sur la page de renouvellement, pour qu'ils
        voient ce qu'ils gagnent en payant. Écris en langage simple, orienté
        bénéfice ("vous pouvez maintenant...") plutôt que technique.
        Seules les 3 plus récentes sont montrées.
      </p>
      <form method="POST" action="/nouveautes">
        <label>Version (ex: 0.2.0) <input type="text" name="version" required /></label>
        <label>Titre court <input type="text" name="titre" required placeholder="Ex: Aperçu avant impression" /></label>
        <label>Description (optionnel) <textarea name="description" rows="2" placeholder="Ex: Vous voyez maintenant le document avant de l'imprimer, sans ouvrir un autre logiciel."></textarea></label>
        <button type="submit">Ajouter</button>
      </form>
    </div>
    ${lignes}`
  );
}

async function pageParametres(env) {
  const numeros = await obtenirParametre(env, "numeros_paiement", "");
  const contact = await obtenirParametre(env, "contact_whatsapp", "");
  const montant = await obtenirParametre(env, "montant_indicatif", "");
  return page(
    "Paramètres",
    `<div class="carte">
      <p><a href="/">&larr; Retour</a></p>
      <h1>Paramètres de la page de renouvellement</h1>
      <p style="font-size:0.85rem; color:#605e5c">
        Ce que voient les gérants sur la page publique <code>/renouveler</code> quand ils veulent payer.
      </p>
      <form method="POST" action="/parametres">
        <label>Numéro(s) Mobile Money (un par ligne) <textarea name="numeros_paiement" rows="2">${echapper(numeros)}</textarea></label>
        <label>Montant indicatif à afficher (optionnel) <input type="text" name="montant_indicatif" value="${echapper(montant)}" /></label>
        <label>Ton contact (WhatsApp/téléphone) <input type="text" name="contact_whatsapp" value="${echapper(contact)}" /></label>
        <button type="submit">Enregistrer</button>
      </form>
    </div>`
  );
}

const CLE_INSTALLATEUR = "GestionPhotocopie-Installateur.exe";
const CLE_EMPREINTE = "empreinte-sha256.txt";

function pageTelecharger(disponible, empreinte) {
  // L'empreinte remplace ce qu'aurait apporté un certificat de signature :
  // elle ne supprime pas l'avertissement de Windows, mais elle permet de
  // vérifier que le fichier téléchargé est bien celui qui a été compilé, et
  // pas une version modifiée en route.
  const blocEmpreinte = empreinte
    ? `<div style="text-align:left; background:#f3f2f1; padding:0.8rem; border-radius:6px; margin-top:1rem">
         <p style="font-size:0.8rem; margin:0 0 0.4rem"><strong>Vérifier que le fichier est authentique</strong> (recommandé)</p>
         <p style="font-size:0.78rem; margin:0 0 0.4rem; color:#605e5c">
           Empreinte officielle de cette version :
         </p>
         <p class="cle-resultat" style="font-size:0.72rem; margin:0 0 0.5rem">${echapper(empreinte)}</p>
         <p style="font-size:0.78rem; margin:0; color:#605e5c">
           Sur le PC, ouvrez l'invite de commande dans le dossier du fichier
           téléchargé et tapez :<br>
           <code style="font-size:0.75rem">certutil -hashfile ${echapper(CLE_INSTALLATEUR)} SHA256</code><br>
           Le résultat doit être identique, caractère pour caractère. S'il
           diffère, n'installez pas le fichier.
         </p>
       </div>`
    : "";

  return page(
    "Télécharger Gestion Photocopie",
    `<div class="carte" style="margin-top:2rem; text-align:center">
      <h1>Gestion Photocopie</h1>
      <p style="font-size:0.9rem; color:#605e5c">
        Logiciel de gestion pour boutiques de photocopie — 100% hors ligne,
        pour Windows.
      </p>
      ${
        disponible
          ? `<a class="btn" href="/telecharger/exe" style="display:block; margin:1rem 0; padding:1rem;">⬇️ Télécharger pour Windows</a>
             <p style="font-size:0.8rem; color:#605e5c; text-align:left">
               Au premier lancement, Windows affichera un avertissement
               "éditeur inconnu". Il apparaît sur tout logiciel dont l'auteur
               n'a pas payé de certificat, y compris celui-ci — mais il
               apparaît aussi sur les vrais programmes malveillants. Ne
               prenez donc pas l'habitude de passer outre sans réfléchir :
               vérifiez d'abord l'empreinte ci-dessous, puis cliquez
               "Informations complémentaires" et "Exécuter quand même".
             </p>
             ${blocEmpreinte}`
          : `<p style="color:#a4262c">Le fichier n'est pas encore disponible. Réessayez plus tard.</p>`
      }
    </div>`,
    { connecte: false }
  );
}

// ─────────────────────────────── Routage ────────────────────────────────

/// Le tableau de bord n'utilise aucun JavaScript : on peut donc l'interdire
/// complètement. Même si une valeur mal échappée se glissait dans une page,
/// le navigateur refuserait d'exécuter quoi que ce soit.
const EN_TETES_SECURITE = {
  "content-security-policy":
    "default-src 'none'; style-src 'unsafe-inline'; img-src 'self' data:; form-action 'self'; base-uri 'none'; frame-ancestors 'none'",
  "x-content-type-options": "nosniff",
  "referrer-policy": "no-referrer",
  "x-frame-options": "DENY",
};

function avecEnTetesSecurite(reponse) {
  const entetes = new Headers(reponse.headers);
  for (const [nom, valeur] of Object.entries(EN_TETES_SECURITE)) entetes.set(nom, valeur);
  return new Response(reponse.body, {
    status: reponse.status,
    statusText: reponse.statusText,
    headers: entetes,
  });
}

/// Bornes sur tout ce qu'un inconnu peut écrire dans la base depuis le
/// formulaire public : sans elles, un seul envoi peut y déposer des
/// mégaoctets de texte, autant de fois qu'il le souhaite.
function borner(valeur, maximum = 200) {
  return String(valeur ?? "").trim().slice(0, maximum);
}

export default {
  async fetch(request, env) {
    return avecEnTetesSecurite(await router(request, env));
  },
};

async function router(request, env) {
  {
    const url = new URL(request.url);
    const { pathname } = url;
    const method = request.method;

    try {
      if (pathname === "/connexion" && method === "GET") {
        return new Response(await pageConnexion(), { headers: { "content-type": "text/html; charset=utf-8" } });
      }
      if (pathname === "/connexion" && method === "POST") {
        const ip = adresseAppelant(request);
        if ((await tentativesRecentes(env, ip)) >= TENTATIVES_MAX) {
          return new Response(
            await pageConnexion("Trop d'essais. Réessayez dans un quart d'heure."),
            { status: 429, headers: { "content-type": "text/html; charset=utf-8" } }
          );
        }
        const donnees = await request.formData();
        if (!(await motDePasseValide(env, donnees.get("mot_de_passe")))) {
          await enregistrerEchec(env, ip);
          return new Response(await pageConnexion("Mot de passe incorrect."), {
            status: 401,
            headers: { "content-type": "text/html; charset=utf-8" },
          });
        }
        await oublierTentatives(env, ip);
        const cookie = await creerCookieSession(env);
        return new Response(null, {
          status: 302,
          headers: {
            Location: "/",
            "Set-Cookie": `session=${cookie}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=2592000`,
          },
        });
      }
      if (pathname === "/deconnexion") {
        return new Response(null, {
          status: 302,
          headers: { Location: "/connexion", "Set-Cookie": "session=; Path=/; Max-Age=0" },
        });
      }

      // Page publique : n'importe quel gérant peut y accéder pour signaler
      // un paiement, sans avoir de compte ni de mot de passe.
      if (pathname === "/renouveler" && method === "GET") {
        return new Response(await pageRenouveler(env), { headers: { "content-type": "text/html; charset=utf-8" } });
      }
      if (pathname === "/renouveler" && method === "POST") {
        // Formulaire ouvert à tous : sans limite, une seule personne peut
        // remplir la base de fausses demandes et rendre le tableau de bord
        // inutilisable.
        const ip = adresseAppelant(request);
        if ((await tentativesRecentes(env, `demande:${ip}`)) >= TENTATIVES_MAX) {
          return new Response(
            await pageRenouveler(env, {
              erreur: "Trop de demandes envoyées. Réessayez dans un quart d'heure.",
            }),
            { status: 429, headers: { "content-type": "text/html; charset=utf-8" } }
          );
        }
        await enregistrerEchec(env, `demande:${ip}`);

        const donnees = await request.formData();
        const nomBoutique = borner(donnees.get("nom_boutique"), 120);
        const machineId = borner(donnees.get("machine_id"), 120);
        const numeroPaiement = borner(donnees.get("numero_paiement"), 40);
        if (!nomBoutique || !machineId || !numeroPaiement) {
          return new Response(await pageRenouveler(env, { erreur: "Merci de remplir tous les champs obligatoires." }), {
            status: 400,
            headers: { "content-type": "text/html; charset=utf-8" },
          });
        }
        const jours = Math.max(1, Math.min(3650, parseInt(donnees.get("jours_demandes"), 10) || 30));
        await env.DB.prepare(
          `INSERT INTO demandes (machine_id, nom_boutique, telephone, numero_paiement, jours_demandes, commentaire)
           VALUES (?, ?, ?, ?, ?, ?)`
        )
          .bind(
            machineId,
            nomBoutique,
            borner(donnees.get("telephone"), 40) || null,
            numeroPaiement,
            jours,
            borner(donnees.get("commentaire"), 500) || null
          )
          .run();
        return new Response(await pageRenouveler(env, { envoye: true }), {
          headers: { "content-type": "text/html; charset=utf-8" },
        });
      }

      // Page publique de téléchargement — pour n'importe quel gérant, sans
      // compte ni mot de passe, sans avoir besoin d'accéder au dépôt GitHub
      // (privé) où est développé le code.
      if (pathname === "/telecharger" && method === "GET") {
        const objet = await env.TELECHARGEMENTS.head(CLE_INSTALLATEUR);
        // Petit fichier texte déposé à côté de l'installateur, produit par la
        // compilation. Absent tant qu'il n'a pas été téléversé : la page
        // fonctionne quand même, sans le bloc de vérification.
        let empreinte = null;
        try {
          const fichier = await env.TELECHARGEMENTS.get(CLE_EMPREINTE);
          if (fichier) empreinte = borner(await fichier.text(), 64);
        } catch {
          empreinte = null;
        }
        return new Response(await pageTelecharger(!!objet, empreinte), {
          headers: { "content-type": "text/html; charset=utf-8" },
        });
      }
      if (pathname === "/telecharger/exe" && method === "GET") {
        const objet = await env.TELECHARGEMENTS.get(CLE_INSTALLATEUR);
        if (!objet) return new Response("Fichier indisponible pour l'instant.", { status: 404 });
        return new Response(objet.body, {
          headers: {
            "content-type": "application/octet-stream",
            "content-disposition": `attachment; filename="${CLE_INSTALLATEUR}"`,
          },
        });
      }

      // Tout le reste exige d'être connecté.
      if (!(await estConnecte(request, env))) {
        return new Response(null, { status: 302, headers: { Location: "/connexion" } });
      }

      if (pathname === "/" && method === "GET") {
        return new Response(await pageAccueil(env), { headers: { "content-type": "text/html; charset=utf-8" } });
      }

      if (pathname === "/boutiques/nouvelle" && method === "GET") {
        return new Response(await pageNouvelleBoutique(), { headers: { "content-type": "text/html; charset=utf-8" } });
      }
      if (pathname === "/boutiques/nouvelle" && method === "POST") {
        const donnees = await request.formData();
        const nom = (donnees.get("nom") || "").trim();
        const machineId = (donnees.get("machine_id") || "").trim();
        if (!nom || !machineId) {
          return new Response(await pageNouvelleBoutique("Le nom et l'identifiant machine sont obligatoires."), {
            status: 400,
            headers: { "content-type": "text/html; charset=utf-8" },
          });
        }
        try {
          const resultat = await env.DB.prepare(
            `INSERT INTO boutiques (nom, gerant_nom, telephone, machine_id, notes) VALUES (?, ?, ?, ?, ?)`
          )
            .bind(nom, donnees.get("gerant_nom") || null, donnees.get("telephone") || null, machineId, donnees.get("notes") || null)
            .run();
          return new Response(null, {
            status: 302,
            headers: { Location: `/boutiques/${resultat.meta.last_row_id}` },
          });
        } catch (e) {
          return new Response(
            await pageNouvelleBoutique("Cet identifiant machine est déjà enregistré pour une autre boutique."),
            { status: 400, headers: { "content-type": "text/html; charset=utf-8" } }
          );
        }
      }

      const matchBoutique = pathname.match(/^\/boutiques\/(\d+)$/);
      if (matchBoutique && method === "GET") {
        const html = await pageBoutique(env, matchBoutique[1]);
        if (!html) return new Response("Boutique introuvable.", { status: 404 });
        return new Response(html, { headers: { "content-type": "text/html; charset=utf-8" } });
      }

      const matchLicence = pathname.match(/^\/boutiques\/(\d+)\/licence$/);
      if (matchLicence && method === "POST") {
        const id = matchLicence[1];
        const boutique = await env.DB.prepare(`SELECT machine_id FROM boutiques WHERE id = ?`).bind(id).first();
        if (!boutique) return new Response("Boutique introuvable.", { status: 404 });
        const donnees = await request.formData();
        const jours = Math.max(1, Math.min(3650, parseInt(donnees.get("jours"), 10) || 30));
        const { cle, dateExpiration } = await genererCle(env, boutique.machine_id, jours);
        await env.DB.prepare(
          `INSERT INTO licences (boutique_id, cle, jours, date_expiration) VALUES (?, ?, ?, ?)`
        )
          .bind(id, cle, jours, dateExpiration)
          .run();
        const html = await pageBoutique(env, id, { cleGeneree: { cle, dateExpiration } });
        return new Response(html, { headers: { "content-type": "text/html; charset=utf-8" } });
      }

      const matchRapport = pathname.match(/^\/boutiques\/(\d+)\/rapport$/);
      if (matchRapport && method === "POST") {
        const id = matchRapport[1];
        const donnees = await request.formData();
        const contenuJson = borner(donnees.get("contenu_json"), 20000);
        try {
          JSON.parse(contenuJson);
        } catch {
          const html = await pageBoutique(env, id);
          return new Response(html || "Boutique introuvable.", {
            status: html ? 400 : 404,
            headers: { "content-type": "text/html; charset=utf-8" },
          });
        }
        await env.DB.prepare(`INSERT INTO rapports (boutique_id, contenu_json) VALUES (?, ?)`)
          .bind(id, contenuJson)
          .run();
        return new Response(null, { status: 302, headers: { Location: `/boutiques/${id}` } });
      }

      const matchDesactiver = pathname.match(/^\/boutiques\/(\d+)\/desactiver$/);
      if (matchDesactiver && method === "POST") {
        const id = matchDesactiver[1];
        await env.DB.prepare(
          `UPDATE boutiques SET abonnement_desactive = 1 - abonnement_desactive WHERE id = ?`
        )
          .bind(id)
          .run();
        return new Response(null, { status: 302, headers: { Location: `/boutiques/${id}` } });
      }

      if (pathname === "/nouveautes" && method === "GET") {
        return new Response(await pageNouveautes(env), { headers: { "content-type": "text/html; charset=utf-8" } });
      }
      if (pathname === "/nouveautes" && method === "POST") {
        const donnees = await request.formData();
        const version = (donnees.get("version") || "").trim();
        const titre = (donnees.get("titre") || "").trim();
        if (!version || !titre) {
          return new Response("Version et titre obligatoires.", { status: 400 });
        }
        await env.DB.prepare(
          `INSERT INTO nouveautes (version, titre, description) VALUES (?, ?, ?)`
        )
          .bind(version, titre, (donnees.get("description") || "").trim() || null)
          .run();
        return new Response(null, { status: 302, headers: { Location: "/nouveautes" } });
      }
      const matchSupprimerNouveaute = pathname.match(/^\/nouveautes\/(\d+)\/supprimer$/);
      if (matchSupprimerNouveaute && method === "POST") {
        await env.DB.prepare(`DELETE FROM nouveautes WHERE id = ?`).bind(matchSupprimerNouveaute[1]).run();
        return new Response(null, { status: 302, headers: { Location: "/nouveautes" } });
      }

      if (pathname === "/parametres" && method === "GET") {
        return new Response(await pageParametres(env), { headers: { "content-type": "text/html; charset=utf-8" } });
      }
      if (pathname === "/parametres" && method === "POST") {
        const donnees = await request.formData();
        await definirParametre(env, "numeros_paiement", (donnees.get("numeros_paiement") || "").trim());
        await definirParametre(env, "montant_indicatif", (donnees.get("montant_indicatif") || "").trim());
        await definirParametre(env, "contact_whatsapp", (donnees.get("contact_whatsapp") || "").trim());
        return new Response(null, { status: 302, headers: { Location: "/parametres" } });
      }

      const matchConfirmer = pathname.match(/^\/demandes\/(\d+)\/confirmer$/);
      if (matchConfirmer && method === "POST") {
        const demande = await env.DB.prepare(`SELECT * FROM demandes WHERE id = ? AND statut = 'en_attente'`)
          .bind(matchConfirmer[1])
          .first();
        if (!demande) return new Response("Demande introuvable ou déjà traitée.", { status: 404 });

        let boutique = await env.DB.prepare(`SELECT * FROM boutiques WHERE machine_id = ?`)
          .bind(demande.machine_id)
          .first();
        if (!boutique) {
          const resultat = await env.DB.prepare(
            `INSERT INTO boutiques (nom, telephone, machine_id) VALUES (?, ?, ?)`
          )
            .bind(demande.nom_boutique, demande.telephone, demande.machine_id)
            .run();
          boutique = { id: resultat.meta.last_row_id };
        }

        const { cle, dateExpiration } = await genererCle(env, demande.machine_id, demande.jours_demandes);
        await env.DB.prepare(
          `INSERT INTO licences (boutique_id, cle, jours, date_expiration) VALUES (?, ?, ?, ?)`
        )
          .bind(boutique.id, cle, demande.jours_demandes, dateExpiration)
          .run();
        await env.DB.prepare(`UPDATE demandes SET statut = 'confirmee' WHERE id = ?`).bind(demande.id).run();

        const html = await pageBoutique(env, boutique.id, { cleGeneree: { cle, dateExpiration } });
        return new Response(html, { headers: { "content-type": "text/html; charset=utf-8" } });
      }

      const matchRejeter = pathname.match(/^\/demandes\/(\d+)\/rejeter$/);
      if (matchRejeter && method === "POST") {
        await env.DB.prepare(`UPDATE demandes SET statut = 'rejetee' WHERE id = ?`).bind(matchRejeter[1]).run();
        return new Response(null, { status: 302, headers: { Location: "/" } });
      }

      return new Response("Introuvable.", { status: 404 });
    } catch (err) {
      console.error("Erreur non gérée:", err);
      return new Response("Erreur serveur.", { status: 500 });
    }
  }
}
