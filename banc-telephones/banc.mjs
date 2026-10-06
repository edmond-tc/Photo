// Banc d'essai sans téléphone : la vraie page, dans le vrai moteur Chrome
// d'Android, imitant chaque modèle courant au Bénin. Pour chacun :
//  1. envoi normal d'un document ;
//  2. la petite fenêtre « Se connecter au réseau » d'Android ;
//  3. le téléphone part sur ses données mobiles en plein envoi, puis revient ;
//  4. le PC injoignable à l'ouverture de la page, puis joignable.
import { execSync } from "node:child_process";
import { demarrerFauxPc } from "./faux-pc.mjs";

const racine = execSync("npm root -g").toString().trim();
const { chromium } = await import(`${racine}/playwright/index.mjs`);

const chrome = (android, modele, version = "130.0.0.0") =>
  `Mozilla/5.0 (Linux; Android ${android}; ${modele}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/${version} Mobile Safari/537.36`;
const fenetrePortail = (android, modele) =>
  `Mozilla/5.0 (Linux; Android ${android}; ${modele} Build/TP1A; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/116.0.0.0 Mobile Safari/537.36`;

const TELEPHONES = [
  { nom: "Tecno Spark 10 (Android 13)", android: "13", modele: "TECNO KI5q", w: 360, h: 804, ratio: 2 },
  { nom: "Infinix Hot 30 (Android 13)", android: "13", modele: "Infinix X6831", w: 393, h: 873, ratio: 2.75 },
  { nom: "itel A70 (Android 13 Go)", android: "13", modele: "itel A665L", w: 360, h: 760, ratio: 2 },
  { nom: "Samsung Galaxy A05 (Android 14)", android: "14", modele: "SM-A055F", w: 360, h: 800, ratio: 2 },
  { nom: "Samsung Galaxy A10 (Android 11)", android: "11", modele: "SM-A105F", w: 360, h: 760, ratio: 2 },
  { nom: "Redmi Note 12 (Android 13)", android: "13", modele: "23021RAA2Y", w: 393, h: 873, ratio: 2.75 },
  { nom: "Redmi 9A (Android 10)", android: "10", modele: "M2006C3LG", w: 360, h: 720, ratio: 2 },
  { nom: "Huawei Y7 (Android 9)", android: "9", modele: "DUB-LX1", w: 360, h: 720, ratio: 2 },
  { nom: "Oppo A17 (Android 12)", android: "12", modele: "CPH2477", w: 360, h: 800, ratio: 2 },
  { nom: "Petit écran (Android 10 Go, 320×640)", android: "10", modele: "Go", w: 320, h: 640, ratio: 1.5 },
];

const attendre = (ms) => new Promise((r) => setTimeout(r, ms));
function document(taille) {
  return { name: "memoire.pdf", mimeType: "application/pdf", buffer: Buffer.alloc(taille, 0x41) };
}

async function contexte(navigateur, t, ua) {
  return navigateur.newContext({
    userAgent: ua,
    viewport: { width: t.w, height: t.h },
    deviceScaleFactor: t.ratio,
    isMobile: true,
    hasTouch: true,
    locale: "fr-FR",
  });
}

async function dansLEcran(page, selecteur) {
  const b = await page.locator(selecteur).first().boundingBox();
  const v = page.viewportSize();
  return !!b && b.y >= 0 && b.y + b.height <= v.height + 1 && b.x >= 0 && b.x + b.width <= v.width + 1;
}

async function scenarioEnvoi(navigateur, t, port) {
  const pc = await demarrerFauxPc(port);
  const ctx = await contexte(navigateur, t, chrome(t.android, t.modele));
  const page = await ctx.newPage();
  const erreurs = [];
  page.on("pageerror", (e) => erreurs.push(e.message));
  try {
    await page.goto(`http://127.0.0.1:${port}/`);
    await page.locator('[data-ecran="accueil"]').waitFor({ state: "visible", timeout: 8000 });
    await page.setInputFiles("#fichiers", document(3 * 1024 * 1024));
    await page.locator('[data-ecran="documents"]').waitFor({ state: "visible", timeout: 5000 });
    const boutonVisible = await dansLEcran(page, "#btn-vers-quand");
    await page.click("#btn-vers-quand");
    await page.locator('[data-ecran="recu"]').waitFor({ state: "visible", timeout: 30000 });
    const numero = (await page.textContent("#numero-commande")).trim();
    const recu = pc.totalRecu();
    const ok = recu === 3 * 1024 * 1024 && pc.etat.commandes === 1 && erreurs.length === 0;
    return { ok, detail: `numéro ${numero}, ${(recu / 1048576).toFixed(1)} Mo reçus, bouton d'envoi ${boutonVisible ? "visible sans défiler" : "HORS de l'écran"}${erreurs.length ? ", erreurs : " + erreurs.join(" | ") : ""}`, alerte: !boutonVisible };
  } catch (e) {
    return { ok: false, detail: `bloqué : ${e.message.split("\n")[0]}${erreurs.length ? " | " + erreurs.join(" | ") : ""}` };
  } finally {
    await ctx.close();
    pc.fermer();
  }
}

async function scenarioPortail(navigateur, t, port) {
  const pc = await demarrerFauxPc(port);
  const ctx = await contexte(navigateur, t, fenetrePortail(t.android, t.modele));
  const page = await ctx.newPage();
  try {
    await page.goto(`http://127.0.0.1:${port}/`);
    await page.locator('[data-ecran="navigateur"]').waitFor({ state: "visible", timeout: 8000 });
    await attendre(3000);
    // Plus aucun départ automatique vers le navigateur.
    const cadres = await page.locator('iframe[src^="intent:"]').count();
    const consigne = await page.locator('[data-ecran="navigateur"]').innerText();
    const dit = /Coupez vos données mobiles/.test(consigne);
    const lien = await page.getAttribute("#lien-navigateur-direct", "href");
    const boutonVisible = await dansLEcran(page, "#lien-navigateur-direct");
    const ok = cadres === 0 && dit && /^intent:\/\//.test(lien || "");
    return { ok, detail: `consigne « données » ${dit ? "affichée" : "ABSENTE"}, départ auto ${cadres ? "PRÉSENT" : "absent"}, bouton ${boutonVisible ? "visible sans défiler" : "à faire défiler"}`, alerte: !boutonVisible };
  } catch (e) {
    return { ok: false, detail: `bloqué : ${e.message.split("\n")[0]}` };
  } finally {
    await ctx.close();
    pc.fermer();
  }
}

async function scenarioDonneesEnPleinEnvoi(navigateur, t, port) {
  const pc = await demarrerFauxPc(port);
  const ctx = await contexte(navigateur, t, chrome(t.android, t.modele));
  const page = await ctx.newPage();
  try {
    await page.goto(`http://127.0.0.1:${port}/`);
    await page.locator('[data-ecran="accueil"]').waitFor({ state: "visible", timeout: 8000 });
    const taille = 8 * 1024 * 1024;
    await page.setInputFiles("#fichiers", document(taille));
    await page.locator('[data-ecran="documents"]').waitFor({ state: "visible", timeout: 5000 });
    await page.click("#btn-vers-quand");
    // Le premier morceau arrive, puis le téléphone part sur ses données.
    const debut = Date.now();
    while (pc.totalRecu() < 1024 * 1024 && Date.now() - debut < 15000) await attendre(100);
    pc.panne("trou");
    const coupe = Date.now();
    const bandeau = page.locator('[data-ecran="envoi"] [data-conseil-donnees]');
    await bandeau.waitFor({ state: "visible", timeout: 30000 });
    const delai = ((Date.now() - coupe) / 1000).toFixed(1);
    // Le client coupe ses données : le Wi-Fi reprend.
    pc.panne("normal");
    await finirCommeUnClient(page);
    const recu = pc.totalRecu();
    const ok = recu === taille && pc.etat.commandes === 1;
    return { ok, detail: `consigne affichée ${delai} s après la coupure, envoi repris et terminé (${(recu / 1048576).toFixed(1)} Mo, rien en double)`, alerte: Number(delai) > 20 };
  } catch (e) {
    return { ok: false, detail: `bloqué : ${e.message.split("\n")[0]} (reçu ${(pc.totalRecu() / 1048576).toFixed(1)} Mo)` };
  } finally {
    await ctx.close();
    pc.fermer();
  }
}

// Attend « C'est envoyé » ; si la page dit d'abord que l'envoi n'a pas
// abouti, touche « Réessayer », comme le ferait le client.
async function finirCommeUnClient(page) {
  const recu = page.locator('[data-ecran="recu"]');
  const reessayer = page.locator("#btn-reessayer");
  const fin = Date.now() + 120000;
  while (Date.now() < fin) {
    if (await recu.isVisible()) return;
    if (await reessayer.isVisible()) await reessayer.click();
    await attendre(500);
  }
  throw new Error("« C'est envoyé » n'est jamais apparu");
}

async function scenarioCoupureALaValidation(navigateur, t, port) {
  const pc = await demarrerFauxPc(port);
  const ctx = await contexte(navigateur, t, chrome(t.android, t.modele));
  const page = await ctx.newPage();
  try {
    await page.goto(`http://127.0.0.1:${port}/`);
    await page.locator('[data-ecran="accueil"]').waitFor({ state: "visible", timeout: 8000 });
    const taille = 2 * 1024 * 1024;
    // La commande ne recevra pas de réponse : la liaison saute pile à la fin.
    pc.panne("trou-commande");
    await page.setInputFiles("#fichiers", document(taille));
    await page.locator('[data-ecran="documents"]').waitFor({ state: "visible", timeout: 5000 });
    await page.click("#btn-vers-quand");
    const debut = Date.now();
    await page.locator("#btn-reessayer").waitFor({ state: "visible", timeout: 120000 });
    const delai = ((Date.now() - debut) / 1000).toFixed(0);
    const consigne = await page.locator('[data-ecran="envoi"] [data-conseil-donnees]').isVisible();
    pc.panne("normal");
    await finirCommeUnClient(page);
    const ok = pc.etat.commandes === 1 && consigne;
    return { ok, detail: `erreur et consigne affichées après ${delai} s (au lieu d'une attente sans fin), « Réessayer » a terminé l'envoi, ${pc.etat.commandes} commande enregistrée` };
  } catch (e) {
    return { ok: false, detail: `bloqué : ${e.message.split("\n")[0]}` };
  } finally {
    await ctx.close();
    pc.fermer();
  }
}

async function scenarioPcInjoignable(navigateur, t, port) {
  const pc = await demarrerFauxPc(port);
  pc.panne("infos-trou");
  const ctx = await contexte(navigateur, t, chrome(t.android, t.modele));
  const page = await ctx.newPage();
  try {
    const debut = Date.now();
    await page.goto(`http://127.0.0.1:${port}/`);
    const bandeau = page.locator('[data-ecran="accueil"] [data-conseil-donnees]');
    await bandeau.waitFor({ state: "visible", timeout: 15000 });
    const delai = ((Date.now() - debut) / 1000).toFixed(1);
    pc.panne("normal");
    await bandeau.waitFor({ state: "hidden", timeout: 15000 });
    const boutique = (await page.locator("[data-boutique]").first().textContent()).trim();
    return { ok: boutique === "Boutique d'essai", detail: `consigne affichée en ${delai} s, disparue toute seule au retour du Wi-Fi, boutique « ${boutique} »` };
  } catch (e) {
    return { ok: false, detail: `bloqué : ${e.message.split("\n")[0]}` };
  } finally {
    await ctx.close();
    pc.fermer();
  }
}

const SCENARIOS = [
  ["Envoi normal", scenarioEnvoi],
  ["Petite fenêtre « Se connecter »", scenarioPortail],
  ["Données mobiles en plein envoi", scenarioDonneesEnPleinEnvoi],
  ["PC injoignable à l'ouverture", scenarioPcInjoignable],
  ["Coupure pile à la validation", scenarioCoupureALaValidation],
];

const navigateur = await chromium.launch();
const resultats = [];
let port = 18100;
const taches = [];
for (const t of TELEPHONES) {
  for (const [nomScenario, f] of SCENARIOS) {
    const p = port++;
    taches.push(async () => {
      const r = await f(navigateur, t, p);
      resultats.push({ telephone: t.nom, scenario: nomScenario, ...r });
      console.log(`${r.ok ? (r.alerte ? "⚠️" : "✅") : "❌"} ${t.nom} — ${nomScenario} : ${r.detail}`);
    });
  }
}
// Six à la fois : la machine a 4 cœurs.
const enCours = new Set();
for (const tache of taches) {
  const p = tache().finally(() => enCours.delete(p));
  enCours.add(p);
  if (enCours.size >= 6) await Promise.race(enCours);
}
await Promise.all(enCours);
await navigateur.close();

const ok = resultats.filter((r) => r.ok).length;
console.log(`\nBILAN : ${ok}/${resultats.length} essais réussis.`);
process.stdout.write(JSON.stringify(resultats) + "\n");
