// Amphi en ligne : publier sur le Wi-Fi de l'amphi depuis Internet.
//
// Le portail de l'amphi tourne sur un téléphone, sans Internet pour les
// étudiants. Avec ce lien (partagé dans le groupe Facebook ou WhatsApp de la
// promo), chacun publie de chez lui : dédicace, objet perdu, marché, binôme,
// question, conseil, document, star du jour. Le téléphone du portail, avec
// ses données mobiles, vient chercher toutes les quelques minutes et affiche
// sur le Wi-Fi de l'amphi. Rien n'est gardé ici après récupération (7 jours
// au plus si le téléphone ne passe pas).

const CODE_VALIDE = /^[0-9A-HJKMNP-TV-Z]{6}$/;
const ID_VALIDE = /^[0-9a-f]{16,40}$/;
const SORTES = ["dedicace", "perdu", "trouve", "vente", "achat", "binome", "conseil", "question", "document", "star"];
const FICHIER_MAX = 20 * 1024 * 1024;
const ATTENTE_MAX = 300;
const ENVOIS_PAR_HEURE = 15;
const DUREE_DE_VIE_MS = 7 * 24 * 3600 * 1000;

const json = (o, statut = 200) => new Response(JSON.stringify(o), { status: statut, headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store" } });
function hasard(octets) { const t = new Uint8Array(octets); crypto.getRandomValues(t); return [...t].map((o) => o.toString(16).padStart(2, "0")).join(""); }
async function empreinte(texte) { const b = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(texte)); return [...new Uint8Array(b)].map((o) => o.toString(16).padStart(2, "0")).join(""); }
function egal(a, b) { if (a.length !== b.length) return false; let d = 0; for (let i = 0; i < a.length; i++) d |= a.charCodeAt(i) ^ b.charCodeAt(i); return d === 0; }
const texte = (v, max) => String(v || "").replace(/[\u0000-\u0008\u000B-\u001F]/g, "").trim().slice(0, max);
const echapper = (t) => String(t).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));

function nouveauCode() {
  const a = "0123456789ABCDEFGHJKMNPQRSTVWXYZ", t = new Uint8Array(6);
  crypto.getRandomValues(t);
  return [...t].map((o) => a[o % a.length]).join("");
}

async function compte(env, code) {
  if (!CODE_VALIDE.test(code)) return null;
  return env.DB.prepare("SELECT * FROM amphi_comptes WHERE code = ?").bind(code).first();
}

/** Le téléphone du portail prouve qu'il est le propriétaire du code. */
async function autorise(request, env, code) {
  const c = await compte(env, code);
  if (!c) return null;
  const secret = (request.headers.get("authorization") || "").replace(/^Bearer\s+/i, "");
  if (!secret || !egal(await empreinte(secret), c.empreinte_secret)) return null;
  return c;
}

async function nettoyer(env, code) {
  const limite = new Date(Date.now() - DUREE_DE_VIE_MS).toISOString();
  const vieux = await env.DB.prepare("SELECT id, fichier FROM amphi_envois WHERE code = ? AND cree < ?").bind(code, limite).all();
  for (const v of vieux.results || []) {
    if (v.fichier) await env.TELECHARGEMENTS.delete(v.fichier);
    await env.DB.prepare("DELETE FROM amphi_envois WHERE id = ?").bind(v.id).run();
  }
}

export async function amphi(request, env) {
  const url = new URL(request.url);
  const p = url.pathname, m = request.method;

  // Inscription du téléphone du portail : un code public, un secret gardé par le téléphone.
  if (p === "/api/amphi/inscrire" && m === "POST") {
    const q = await request.json().catch(() => ({}));
    let code = "";
    for (let i = 0; i < 5 && !code; i++) { const c = nouveauCode(); if (!(await compte(env, c))) code = c; }
    if (!code) return json({ erreur: "Réessayez." }, 503);
    const secret = hasard(24);
    await env.DB.prepare("INSERT INTO amphi_comptes (code, empreinte_secret, titre, cree) VALUES (?, ?, ?, ?)")
      .bind(code, await empreinte(secret), texte(q.titre, 80), new Date().toISOString()).run();
    return json({ ok: true, code, secret });
  }

  let r = p.match(/^\/api\/amphi\/([0-9A-Z]{6})\/(titre|attente|recu)$/);
  if (r) {
    const c = await autorise(request, env, r[1]);
    if (!c) return json({ erreur: "Refusé." }, 403);
    await env.DB.prepare("UPDATE amphi_comptes SET vu = ? WHERE code = ?").bind(new Date().toISOString(), c.code).run();
    if (r[2] === "titre" && m === "POST") {
      const q = await request.json().catch(() => ({}));
      await env.DB.prepare("UPDATE amphi_comptes SET titre = ? WHERE code = ?").bind(texte(q.titre, 80), c.code).run();
      return json({ ok: true });
    }
    if (r[2] === "attente" && m === "GET") {
      await nettoyer(env, c.code);
      const l = await env.DB.prepare("SELECT id, sorte, champs, nom, type, taille, fichier, cree FROM amphi_envois WHERE code = ? ORDER BY cree LIMIT 50").bind(c.code).all();
      return json({ envois: (l.results || []).map((e) => ({ id: e.id, sorte: e.sorte, champs: JSON.parse(e.champs), nom: e.nom, type: e.type, taille: e.taille, fichier: !!e.fichier, cree: e.cree })) });
    }
    if (r[2] === "recu" && m === "POST") {
      const q = await request.json().catch(() => ({}));
      for (const id of (q.ids || []).slice(0, 100)) {
        if (!ID_VALIDE.test(id)) continue;
        const e = await env.DB.prepare("SELECT fichier FROM amphi_envois WHERE id = ? AND code = ?").bind(id, c.code).first();
        if (!e) continue;
        if (e.fichier) await env.TELECHARGEMENTS.delete(e.fichier);
        await env.DB.prepare("DELETE FROM amphi_envois WHERE id = ?").bind(id).run();
      }
      return json({ ok: true });
    }
    return json({ erreur: "Méthode non permise." }, 405);
  }

  r = p.match(/^\/api\/amphi\/([0-9A-Z]{6})\/fichier\/([0-9a-f]{16,40})$/);
  if (r && m === "GET") {
    const c = await autorise(request, env, r[1]);
    if (!c) return json({ erreur: "Refusé." }, 403);
    const e = await env.DB.prepare("SELECT fichier, type FROM amphi_envois WHERE id = ? AND code = ?").bind(r[2], c.code).first();
    const o = e && e.fichier ? await env.TELECHARGEMENTS.get(e.fichier) : null;
    if (!o) return json({ erreur: "Introuvable." }, 404);
    return new Response(o.body, { headers: { "content-type": e.type || "application/octet-stream", "content-length": String(o.size), "cache-control": "no-store" } });
  }

  // Un étudiant publie (texte dans l'adresse, fichier éventuel dans le corps).
  r = p.match(/^\/api\/amphi\/([0-9A-Za-z]{6})\/publier$/);
  if (r && m === "POST") {
    const c = await compte(env, r[1].toUpperCase());
    if (!c) return json({ erreur: "Ce lien n'existe plus." }, 404);
    const sorte = url.searchParams.get("sorte");
    if (!SORTES.includes(sorte)) return json({ erreur: "Type inconnu." }, 400);
    const ip = request.headers.get("cf-connecting-ip") || "local";
    const depuis = new Date(Date.now() - 3600 * 1000).toISOString();
    const n = await env.DB.prepare("SELECT COUNT(*) AS n FROM amphi_envois WHERE code = ? AND ip = ? AND cree > ?").bind(c.code, ip, depuis).first();
    if ((n && n.n) >= ENVOIS_PAR_HEURE) return json({ erreur: "Trop d'envois depuis ce téléphone : réessayez dans une heure." }, 429);
    const total = await env.DB.prepare("SELECT COUNT(*) AS n FROM amphi_envois WHERE code = ?").bind(c.code).first();
    if ((total && total.n) >= ATTENTE_MAX) return json({ erreur: "Beaucoup d'envois en attente : réessayez plus tard." }, 503);
    const champs = {};
    for (const k of ["texte", "contact", "prix", "ue", "quartier", "titre", "matiere", "auteur", "nom", "message"]) {
      const v = texte(url.searchParams.get(k), k === "texte" || k === "message" ? 400 : 120);
      if (v) champs[k] = v;
    }
    if (sorte === "document" || sorte === "star") { /* fichier attendu */ }
    else if (!champs.texte || champs.texte.length < 3) return json({ erreur: "Écrivez un message." }, 400);
    if (sorte === "star" && (!champs.nom || champs.nom.length < 2)) return json({ erreur: "Écrivez le prénom de la star." }, 400);
    const id = hasard(10);
    let fichier = null, taille = 0;
    const longueur = Number(request.headers.get("content-length") || 0);
    if (longueur > 0) {
      if (longueur > FICHIER_MAX) return json({ erreur: "Fichier trop gros (20 Mo au plus)." }, 413);
      fichier = `amphi/${c.code}/${id}`;
      const corps = await request.arrayBuffer();
      if (corps.byteLength > FICHIER_MAX) return json({ erreur: "Fichier trop gros (20 Mo au plus)." }, 413);
      if (corps.byteLength) { await env.TELECHARGEMENTS.put(fichier, corps); taille = corps.byteLength; } else fichier = null;
    }
    if ((sorte === "document" || sorte === "star") && !fichier) return json({ erreur: sorte === "star" ? "Choisissez une photo." : "Choisissez le document." }, 400);
    await env.DB.prepare("INSERT INTO amphi_envois (id, code, sorte, champs, fichier, nom, type, taille, ip, cree) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
      .bind(id, c.code, sorte, JSON.stringify(champs), fichier, texte(url.searchParams.get("fichier"), 120) || null, texte(url.searchParams.get("type"), 80) || null, taille, ip, new Date().toISOString()).run();
    return json({ ok: true });
  }

  r = p.match(/^\/a\/([0-9A-Za-z]{6})\/?$/);
  if (r && m === "GET") {
    const c = await compte(env, r[1].toUpperCase());
    if (!c) return new Response("<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width'><title>Lien introuvable</title><body style='font-family:sans-serif;padding:2em;text-align:center'><h1>Lien introuvable</h1><p>Demandez au délégué le lien à jour.</p>", { status: 404, headers: { "content-type": "text/html; charset=utf-8" } });
    return new Response(pagePublier(c), { headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-store", "x-content-type-options": "nosniff" } });
  }
  return null;
}

function pagePublier(c) {
  const titre = echapper(c.titre || "Wi-Fi de l'amphi");
  const vu = c.vu ? Math.round((Date.now() - Date.parse(c.vu)) / 60000) : null;
  const etat = vu == null ? "Le téléphone de l'amphi ne s'est pas encore connecté." : vu < 15 ? "🟢 Le téléphone de l'amphi est en ligne : vos publications arrivent en quelques minutes." : "🟠 Le téléphone de l'amphi est hors ligne pour l'instant : vos publications arriveront dès son retour.";
  return `<!doctype html>
<html lang="fr"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Publier · ${titre}</title>
<meta property="og:title" content="Publier sur le Wi-Fi de l'amphi · ${titre}">
<meta property="og:description" content="Dédicaces, objets perdus, binômes, documents, questions : publiez de chez vous, c'est affiché sur le Wi-Fi de l'amphi.">
<style>
  :root { --ecran:#06110d; --vert:#3dffa2; --bleu:#2b579a; --nuit:#16306f; --fond:#eef3f7; --discret:#5d6475; }
  * { box-sizing: border-box; } body { margin:0; background:var(--fond); color:#1f2430; font:15px/1.45 Roboto,"Segoe UI",Arial,sans-serif; }
  header { background:var(--ecran); color:#fff; padding:22px 16px; } header h1 { margin:6px 0 2px; font-size:1.35rem; } header p { margin:0; color:#bfe9d6; font-size:.92rem; }
  .pilule { display:inline-block; border:1px solid rgba(61,255,162,.4); background:rgba(61,255,162,.1); color:#d9ffee; border-radius:999px; padding:2px 10px; font-size:.78rem; font-weight:700; }
  main { max-width:560px; margin:0 auto; padding:14px 16px 40px; display:flex; flex-direction:column; gap:12px; }
  .etat { font-size:.88rem; color:var(--nuit); background:#fff; border-radius:12px; padding:10px 12px; }
  .choix { display:flex; flex-wrap:wrap; gap:6px; } .choix button { border:1px solid #cfd6e4; background:#fff; color:var(--nuit); border-radius:999px; padding:7px 12px; font-family:inherit; font-weight:600; font-size:.88rem; cursor:pointer; }
  .choix button.on { background:var(--nuit); color:#fff; border-color:var(--nuit); }
  form { background:#fff; border-radius:16px; padding:16px; display:flex; flex-direction:column; gap:8px; box-shadow:0 2px 10px rgba(16,24,40,.08); }
  label { font-weight:700; font-size:.86rem; color:var(--nuit); } input, textarea, select { width:100%; font:inherit; padding:10px 12px; border:1px solid #cfd6e4; border-radius:10px; background:#fff; }
  textarea { min-height:90px; } .envoyer { border:0; border-radius:12px; padding:13px; background:var(--bleu); color:#fff; font-family:inherit; font-weight:800; font-size:1rem; cursor:pointer; }
  .aide { color:var(--discret); font-size:.82rem; } #resultat { font-weight:700; } [hidden] { display:none !important; }
</style></head>
<body>
<header><span class="pilule">● Wi-Fi de l'amphi</span><h1>${titre}</h1><p>Publiez de chez vous : c'est affiché sur le Wi-Fi de l'amphi.</p></header>
<main>
  <p class="etat">${etat}</p>
  <div class="choix" id="choix"></div>
  <form id="f" onsubmit="return false">
    <div data-pour="star"><label>Prénom de la star</label><input name="nom" maxlength="30" placeholder="Ex. : Koffi"></div>
    <div data-pour="document"><label>Titre du document</label><input name="titre" maxlength="120" placeholder="Ex. : Épreuve de Mécanique quantique 2024"></div>
    <div data-pour="document"><label>Matière</label><input name="matiere" maxlength="60" placeholder="Ex. : Physique quantique"></div>
    <div data-pour="binome"><label>UE</label><input name="ue" maxlength="60" placeholder="Ex. : PHY1520 Physique quantique"></div>
    <div data-pour="binome"><label>Quartier</label><input name="quartier" maxlength="40" placeholder="Ex. : Calavi"></div>
    <div data-sauf="document"><label id="l-texte">Message</label><textarea name="texte" maxlength="400"></textarea></div>
    <div data-pour="vente achat"><label>Prix</label><input name="prix" maxlength="30" placeholder="Ex. : 5 000 F"></div>
    <div data-pour="perdu trouve vente achat binome"><label>Comment vous joindre</label><input name="contact" maxlength="60" placeholder="Ex. : rang 6, ou votre numéro"></div>
    <div data-pour="conseil"><label>Votre prénom et parcours</label><input name="contact" maxlength="60" placeholder="Ex. : Rodrigue, Master 2 Physique"></div>
    <div data-pour="document star perdu trouve vente achat"><label id="l-fichier">Photo (facultatif)</label><input type="file" id="fichier"></div>
    <div data-pour="document"><label>Votre prénom (facultatif)</label><input name="auteur" maxlength="40"></div>
    <button class="envoyer" id="envoyer" type="button">Publier</button>
    <p id="resultat"></p>
    <p class="aide">Restez correct : pas de nom complet, pas d'insulte. Les contenus signalés par la promo sont retirés.</p>
  </form>
</main>
<script>
(function () {
  var CODE = ${JSON.stringify(c.code)};
  var SORTES = [["dedicace","💌 Dédicace","Votre dédicace"],["star","🎂 Star du jour","Un mot de la star (facultatif)"],["question","💬 Question","Votre question (anonyme)"],["document","📚 Document",""],["binome","👥 Binôme","Ce que vous cherchez"],["perdu","🔎 Perdu","Ce que vous avez perdu"],["trouve","🔎 Trouvé","Ce que vous avez trouvé"],["vente","🛒 Je vends","Ce que vous vendez"],["achat","🛒 Je cherche","Ce que vous cherchez"],["conseil","🧭 Conseil d'aîné","Votre conseil aux L3"]];
  var sorte = "dedicace", z = document.getElementById("choix"), f = document.getElementById("f");
  function montrer() {
    Array.prototype.forEach.call(z.children, function (b) { b.className = b.dataset.s === sorte ? "on" : ""; });
    Array.prototype.forEach.call(f.querySelectorAll("[data-pour]"), function (d) { d.hidden = d.dataset.pour.split(" ").indexOf(sorte) < 0; });
    Array.prototype.forEach.call(f.querySelectorAll("[data-sauf]"), function (d) { d.hidden = d.dataset.sauf === sorte; });
    var s = SORTES.filter(function (x) { return x[0] === sorte; })[0];
    document.getElementById("l-texte").textContent = s[2] || "Message";
    document.getElementById("l-fichier").textContent = sorte === "document" ? "Le document (PDF, photo… 20 Mo au plus)" : sorte === "star" ? "Votre plus belle photo" : "Photo (facultatif)";
    document.getElementById("resultat").textContent = "";
  }
  SORTES.forEach(function (s) { var b = document.createElement("button"); b.type = "button"; b.textContent = s[1]; b.dataset.s = s[0]; b.onclick = function () { sorte = s[0]; montrer(); }; z.appendChild(b); });
  montrer();
  document.getElementById("envoyer").onclick = function () {
    var b = this, res = document.getElementById("resultat"), q = "sorte=" + sorte;
    Array.prototype.forEach.call(f.querySelectorAll("[data-pour]:not([hidden]) [name], [data-sauf]:not([hidden]) [name]"), function (i) { if (i.value.trim()) q += "&" + i.name + "=" + encodeURIComponent(i.value.trim()); });
    var fi = document.getElementById("fichier"), fichier = fi.closest("[hidden]") ? null : fi.files && fi.files[0];
    if (fichier) q += "&fichier=" + encodeURIComponent(fichier.name) + "&type=" + encodeURIComponent(fichier.type || "");
    b.disabled = true; res.textContent = "Envoi…";
    fetch("/api/amphi/" + CODE + "/publier?" + q, { method: "POST", body: fichier || null })
      .then(function (r) { return r.json(); })
      .then(function (r) {
        if (!r.ok) { res.textContent = "❌ " + (r.erreur || "Échec."); return; }
        res.textContent = "✅ Envoyé ! Ce sera affiché sur le Wi-Fi de l'amphi dès que le téléphone du portail le récupère.";
        Array.prototype.forEach.call(f.querySelectorAll("textarea, input"), function (i) { if (i.name !== "contact" && i.name !== "auteur") i.value = ""; });
      })
      .catch(function () { res.textContent = "❌ Pas de connexion Internet."; })
      .then(function () { b.disabled = false; });
  };
})();
</script>
</body></html>`;
}
