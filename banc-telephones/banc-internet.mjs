// Banc d'essai « Par Internet » : la vraie page, servie par le vrai relais
// (wrangler dev en local, ou le relais en ligne), sur dix téléphones imités.
// Un PC simulé fait ce que fait relais.rs : il vient chercher les commandes,
// télécharge les fichiers et accuse réception avec un numéro de guichet.
//
//   RELAIS=http://127.0.0.1:8787 node banc-telephones/banc-internet.mjs
import { execSync } from "node:child_process";
import crypto from "node:crypto";

const RELAIS = process.env.RELAIS || "http://127.0.0.1:8787";
const racine = execSync("npm root -g").toString().trim();
const { chromium } = await import(`${racine}/playwright/index.mjs`);

const chrome = (android, modele) =>
  `Mozilla/5.0 (Linux; Android ${android}; ${modele}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36`;
const TELEPHONES = [
  { nom: "Tecno Spark 10", android: "13", modele: "TECNO KI5q", w: 360, h: 804 },
  { nom: "Infinix Hot 30", android: "13", modele: "Infinix X6831", w: 393, h: 873 },
  { nom: "itel A70", android: "13", modele: "itel A665L", w: 360, h: 760 },
  { nom: "Samsung Galaxy A05 (anglais)", android: "14", modele: "SM-A055F", w: 360, h: 800, langue: "en-GB" },
  { nom: "Samsung Galaxy A10", android: "11", modele: "SM-A105F", w: 360, h: 760 },
  { nom: "Redmi Note 12", android: "13", modele: "23021RAA2Y", w: 393, h: 873 },
  { nom: "Redmi 9A", android: "10", modele: "M2006C3LG", w: 360, h: 720 },
  { nom: "Huawei Y7", android: "9", modele: "DUB-LX1", w: 360, h: 720 },
  { nom: "Oppo A17 (anglais)", android: "12", modele: "CPH2477", w: 360, h: 800, langue: "en-US" },
  { nom: "Petit écran Android Go", android: "10", modele: "Go", w: 320, h: 640 },
];
const attendre = (ms) => new Promise((r) => setTimeout(r, ms));

// ── La boutique d'essai et son PC simulé ──
const code = "E" + Array.from(crypto.randomBytes(5), (o) => "0123456789ABCDEFGHJKMNPQRSTVWXYZ"[o % 32]).join("");
const secret = crypto.randomBytes(32).toString("hex");
const r = await fetch(`${RELAIS}/api/relais/inscrire`, {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({
    code, secret, nom: "Boutique d'essai Internet", tarifs: { nb_a4: 25 },
    fiche: { adresse: "Carrefour Vèdoko, Cotonou", horaires: "Lun-Sam 8h-20h", telephone: "+229 01 00 00 00 01", plan: "https://maps.app.goo.gl/essai", devise: "FCFA" },
    bloques: ["+229 01 97 00 00 99"],
  }),
});
if (!r.ok) throw new Error(`inscription refusée : ${r.status} ${await r.text()}`);

// Un numéro bloqué par la boutique est refusé, sans rien stocker.
{
  const f = new FormData();
  f.append("telephone", "0197000099");
  f.append("nombre_fichiers", "1");
  f.append("fichier_0", new Blob(["x"]), "x.pdf");
  const b = await fetch(`${RELAIS}/b/${code}/envoyer`, { method: "POST", body: f });
  if (b.status !== 403) throw new Error(`numéro bloqué accepté (${b.status})`);
  const sans = new FormData();
  sans.append("nombre_fichiers", "1");
  sans.append("fichier_0", new Blob(["x"]), "x.pdf");
  const s = await fetch(`${RELAIS}/b/${code}/envoyer`, { method: "POST", body: sans });
  if (s.status !== 400) throw new Error(`commande sans numéro acceptée (${s.status})`);
  console.log("✅ Numéro bloqué refusé, commande sans numéro refusée.");
}

const recus = new Map(); // nom du document → octets reçus par le PC
let numero = 0;
let pcEnMarche = true;
(async function pc() {
  while (pcEnMarche) {
    try {
      const a = await (await fetch(`${RELAIS}/api/relais/${code}/attente`, { headers: { "x-secret": secret } })).json();
      for (const c of a.commandes) {
        for (const f of c.fichiers) {
          const corps = await (await fetch(`${RELAIS}/api/relais/${code}/fichier/${c.id}/${f.champ}`, { headers: { "x-secret": secret } })).arrayBuffer();
          recus.set(f.nom, corps.byteLength);
        }
        numero += 1;
        await fetch(`${RELAIS}/api/relais/${code}/accuse/${c.id}`, {
          method: "POST",
          headers: { "x-secret": secret, "content-type": "application/json" },
          body: JSON.stringify({ numero: `I-${numero}` }),
        });
      }
    } catch {}
    await attendre(1000);
  }
})();

function document(nom, taille) {
  return { name: nom, mimeType: "application/pdf", buffer: crypto.randomBytes(taille) };
}

async function essai(navigateur, t, coupure) {
  const ctx = await navigateur.newContext({
    userAgent: chrome(t.android, t.modele),
    viewport: { width: t.w, height: t.h },
    isMobile: true,
    hasTouch: true,
    locale: t.langue || "fr-FR",
  });
  const page = await ctx.newPage();
  const erreurs = [];
  page.on("pageerror", (e) => erreurs.push(e.message));
  const nom = `${t.nom}-${coupure ? "coupure" : "normal"}.pdf`.replace(/\s+/g, "_");
  const taille = coupure ? 5 * 1024 * 1024 : 3 * 1024 * 1024;
  try {
    await page.goto(`${RELAIS}/b/${code}/`);
    await page.locator('[data-ecran="accueil"]').waitFor({ state: "visible", timeout: 15000 });
    const boutique = (await page.locator("[data-boutique]").first().textContent()).trim();
    await page.setInputFiles("#fichiers", document(nom, taille));
    await page.locator('[data-ecran="documents"]').waitFor({ state: "visible", timeout: 5000 });
    if (coupure) {
      // Le réseau mobile saute 8 s en plein envoi, puis revient.
      let premier = true;
      await page.route("**/morceau/**", async (route) => {
        if (route.request().method() === "POST" && premier) {
          premier = false;
          setTimeout(() => page.unroute("**/morceau/**").catch(() => {}), 8000);
          return route.abort("internetdisconnected");
        }
        return route.continue();
      });
    }
    await page.click("#btn-vers-quand");
    // Par Internet : « Pour quand ? », prénom et numéro WhatsApp obligatoires.
    await page.locator('[data-ecran="quand"]').waitFor({ state: "visible", timeout: 5000 });
    await page.click("#btn-envoyer");
    if (!(await page.locator("#contact-erreur").isVisible())) throw new Error("envoi parti sans numéro");
    await page.fill("#prenom-en-ligne", "Awa");
    await page.fill("#numero-en-ligne", "+229 01 97 00 00 " + String(10 + Math.floor(Math.random() * 89)));
    await page.click("#btn-envoyer");
    await page.locator('[data-ecran="recu"]').waitFor({ state: "visible", timeout: 120000 });
    const consigneDonnees = await page.locator("[data-conseil-donnees]:visible").count();
    // Le numéro du guichet arrive quand le PC a récupéré la commande.
    const debut = Date.now();
    let affiche = "";
    while (Date.now() - debut < 30000) {
      affiche = (await page.textContent("#numero-commande")).trim();
      if (/^I-\d+$/.test(affiche)) break;
      await attendre(500);
    }
    // Mes commandes : de retour à l'accueil, la commande y est, avec son
    // numéro ; le lien de suivi (#c=…) la rouvre dans une page neuve.
    const lien = page.url();
    await page.click("#btn-nouvelle");
    await page.locator("#liste-commandes .commande").first().waitFor({ state: "visible", timeout: 5000 });
    const dansMes = (await page.textContent("#liste-commandes")).includes(affiche);
    const fiche = (await page.textContent("#fiche-boutique")) || "";
    const ficheOk = fiche.includes("Vèdoko") && fiche.includes("8h-20h") && (await page.locator("#fiche-boutique a[href^='tel:']").count()) === 1;
    const autre = await ctx.newPage();
    await autre.goto(lien);
    await autre.locator('[data-ecran="recu"]').waitFor({ state: "visible", timeout: 10000 });
    const rouvert = /c=[0-9a-f]{32}$/.test(lien) && (await autre.textContent("#numero-commande")).trim() === affiche;
    // Téléphone en anglais : la page l'est aussi, boutons compris.
    const langueOk = !t.langue || ((await page.getAttribute("html", "lang")) === "en" && (await page.textContent("#btn-envoyer")).trim() === "Send to the shop");
    const ok = langueOk && recus.get(nom) === taille && /^I-\d+$/.test(affiche) && consigneDonnees === 0 && erreurs.length === 0 && dansMes && ficheOk && rouvert;
    return {
      ok,
      detail: `boutique « ${boutique} », ${(taille / 1048576).toFixed(0)} Mo reçus par le PC : ${recus.get(nom) === taille ? "entier" : "INCOMPLET"}, numéro affiché « ${affiche} » en ${((Date.now() - debut) / 1000).toFixed(1)} s, consigne « données » ${consigneDonnees ? "AFFICHÉE À TORT" : "absente"}, Mes commandes ${dansMes ? "ok" : "ABSENTE"}, fiche ${ficheOk ? "ok" : "ABSENTE"}, lien de suivi ${rouvert ? "ok" : "CASSÉ"}${t.langue ? `, anglais ${langueOk ? "ok" : "ABSENT"}` : ""}${erreurs.length ? ", erreurs : " + erreurs.join(" | ") : ""}`,
    };
  } catch (e) {
    return { ok: false, detail: `bloqué : ${e.message.split("\n")[0]}` };
  } finally {
    await ctx.close();
  }
}

const navigateur = await chromium.launch();
const resultats = [];
const taches = [];
for (const t of TELEPHONES) for (const coupure of [false, true]) taches.push({ t, coupure });
const enCours = new Set();
for (const { t, coupure } of taches) {
  const p = essai(navigateur, t, coupure).then((r) => {
    resultats.push(r);
    console.log(`${r.ok ? "✅" : "❌"} ${t.nom} — ${coupure ? "réseau coupé 8 s en plein envoi" : "envoi par Internet"} : ${r.detail}`);
  });
  enCours.add(p);
  p.finally(() => enCours.delete(p));
  if (enCours.size >= 5) await Promise.race(enCours);
}
await Promise.all(enCours);
pcEnMarche = false;
await navigateur.close();
console.log(`\nBILAN INTERNET : ${resultats.filter((r) => r.ok).length}/${resultats.length} essais réussis (boutique ${code}).`);
