// Faux PC de boutique : sert la VRAIE page client (client-web/index.html) et
// répond comme server.rs aux adresses qu'elle appelle. Le mode « trou »
// imite un téléphone parti sur ses données mobiles : les requêtes n'arrivent
// plus (elles restent sans réponse, comme des paquets partis ailleurs).
import http from "node:http";
import fs from "node:fs";
import { alleger } from "../admin/site/alleger.js";

const PAGE = process.env.PAGE || new URL("../client-web/index.html", import.meta.url);

export function demarrerFauxPc(port) {
  const etat = { mode: "normal", morceaux: new Map(), commandes: 0, recus: [], suspendues: [] };
  const serveur = http.createServer((req, res) => {
    const url = new URL(req.url, "http://x");
    const chemin = url.pathname;
    // Panne : on ne répond pas, on garde la connexion ouverte.
    const enPanne =
      etat.mode === "trou" || (etat.mode === "infos-trou" && chemin === "/infos");
    if (enPanne && chemin !== "/") {
      etat.suspendues.push(res);
      req.resume();
      return;
    }
    const json = (code, objet) => {
      res.writeHead(code, { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" });
      res.end(JSON.stringify(objet));
    };
    if (req.method === "GET" && chemin === "/langue-en.json") {
      res.writeHead(200, { "Content-Type": "application/json; charset=utf-8" });
      res.end(fs.readFileSync(new URL("../client-web/langue-en.json", import.meta.url)));
      return;
    }
    if (req.method === "GET" && (chemin === "/" || chemin === "/index.html")) {
      res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
      // Allégée comme par le vrai PC (build.rs) et le relais.
      res.end(alleger(fs.readFileSync(PAGE, "utf8")));
      return;
    }
    if (req.method === "GET" && chemin === "/infos") {
      return json(200, { boutique: "Boutique d'essai", tarifs: { nb_a4: 25, couleur_a4: 100 } });
    }
    let m = chemin.match(/^\/morceau\/([0-9a-f]+)$/);
    if (m && req.method === "GET") return json(200, { recu: etat.morceaux.get(m[1]) || 0 });
    if (m && req.method === "POST") {
      const id = m[1];
      const debut = Number(url.searchParams.get("debut") || 0);
      const deja = etat.morceaux.get(id) || 0;
      if (debut !== deja) {
        req.resume();
        return json(409, { recu: deja });
      }
      let n = 0;
      req.on("data", (c) => (n += c.length));
      req.on("end", () => {
        // Un vrai Wi-Fi n'est pas instantané : ~4 Mo/s.
        setTimeout(() => {
          if (etat.mode === "trou") { etat.suspendues.push(res); return; }
          etat.morceaux.set(id, deja + n);
          json(200, { recu: deja + n });
        }, 250);
      });
      return;
    }
    if (req.method === "POST" && chemin === "/envoyer") {
      let n = 0;
      req.on("data", (c) => (n += c.length));
      req.on("end", () => {
        if (etat.mode === "trou" || etat.mode === "trou-commande") { etat.suspendues.push(res); return; }
        etat.commandes += 1;
        etat.recus.push(n);
        json(200, { numero: "A-" + etat.commandes, jetons: ["jeton" + etat.commandes] });
      });
      return;
    }
    if (req.method === "GET" && chemin.startsWith("/statut/")) return json(200, { etape: "recu" });
    json(404, {});
  });
  return new Promise((ok) =>
    serveur.listen(port, "127.0.0.1", () =>
      ok({
        etat,
        panne(mode) {
          etat.mode = mode;
          if (mode === "normal") {
            // Les requêtes restées en route échouent, comme au retour du Wi-Fi.
            for (const r of etat.suspendues.splice(0)) r.destroy();
          }
        },
        totalRecu() { return [...etat.morceaux.values()].reduce((a, b) => a + b, 0); },
        fermer() { for (const r of etat.suspendues.splice(0)) r.destroy(); serveur.closeAllConnections?.(); serveur.close(); },
      }),
    ),
  );
}
