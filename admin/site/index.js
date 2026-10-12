// Le site public de Gestion Photocopie et le relais Internet des boutiques.
// Séparé du tableau de bord (photocopie-admin) : une adresse publique
// propre, et rien de ce qui est réservé au porteur du projet.

import { relais, nettoyer, nettoyerSiBesoin } from "./relais.js";
import { pageAccueil, pageComment, pageConfidentialite, pageTelechargerSite, robots, sitemap } from "./pages.js";

const html = (corps) =>
  new Response(corps, {
    headers: {
      "content-type": "text/html; charset=utf-8",
      "cache-control": "public, max-age=600",
      "x-content-type-options": "nosniff",
      "referrer-policy": "strict-origin-when-cross-origin",
    },
  });

export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url);
    // La visite d'un PC déclenche, au plus une fois par heure, le nettoyage
    // du relais — après la réponse, sans la retarder.
    if (/^\/api\/relais\/[0-9A-Z]{6,10}\/attente$/.test(url.pathname)) {
      ctx.waitUntil(nettoyerSiBesoin(env).catch(() => {}));
    }
    // Filet de sécurité appelé chaque heure par GitHub (nettoyage-relais.yml),
    // pour les heures où aucune boutique n'est allumée. Sans danger : au plus
    // un nettoyage par heure, quoi qu'il arrive.
    if (url.pathname === "/api/relais/nettoyer" && request.method === "POST") {
      ctx.waitUntil(nettoyerSiBesoin(env).catch(() => {}));
      return new Response("ok", { headers: { "cache-control": "no-store" } });
    }
    try {
      const r = await relais(request, env);
      if (r) return r;
      if (request.method !== "GET" && request.method !== "HEAD") return new Response("Méthode non permise", { status: 405 });
      switch (url.pathname) {
        case "/": return html(pageAccueil());
        case "/comment-ca-marche": return html(pageComment());
        case "/telecharger": return html(pageTelechargerSite());
        case "/confidentialite": return html(pageConfidentialite());
        case "/robots.txt": return new Response(robots(), { headers: { "content-type": "text/plain; charset=utf-8" } });
        case "/sitemap.xml": return new Response(sitemap(), { headers: { "content-type": "application/xml; charset=utf-8" } });
        default: return new Response("Page introuvable", { status: 404, headers: { "content-type": "text/plain; charset=utf-8" } });
      }
    } catch (e) {
      return new Response("Erreur passagère du service. Réessayez dans un instant.", { status: 500 });
    }
  },
  async scheduled(_evenement, env, ctx) {
    ctx.waitUntil(nettoyer(env));
  },
};
