// Faux FedaPay pour le banc d'essai du paiement en ligne (paiement.js) :
// mêmes adresses et mêmes formes de réponse que l'API v1.
import http from "node:http";

export function demarrerFauxFedapay(port, { cle = "sk_test_banc" } = {}) {
  const transactions = new Map();
  let suivant = 1000;
  const serveur = http.createServer(async (req, res) => {
    let corps = "";
    for await (const morceau of req) corps += morceau;
    const json = (code, o) => { res.writeHead(code, { "content-type": "application/json" }); res.end(JSON.stringify(o)); };
    const url = new URL(req.url, "http://x");
    if (url.pathname.startsWith("/v1/") && req.headers.authorization !== `Bearer ${cle}`) return json(401, { message: "Unauthorized" });
    if (req.method === "POST" && url.pathname === "/v1/transactions") {
      const d = JSON.parse(corps);
      const t = { id: ++suivant, amount: d.amount, status: "pending", callback_url: d.callback_url, custom_metadata: d.custom_metadata, currency: d.currency };
      transactions.set(t.id, t);
      return json(200, { "v1/transaction": t });
    }
    let m = url.pathname.match(/^\/v1\/transactions\/(\d+)\/token$/);
    // Adresse de paiement : celle du faux FedaPay, ou (essai dans un vrai
    // navigateur) une adresse « fedapay.com » que le navigateur intercepte.
    if (req.method === "POST" && m) return json(200, { token: "tok" + m[1], url: process.env.ADRESSE_PAGE_PAIEMENT ? `${process.env.ADRESSE_PAGE_PAIEMENT}/${m[1]}` : `http://127.0.0.1:${port}/payer/${m[1]}` });
    m = url.pathname.match(/^\/v1\/transactions\/(\d+)$/);
    if (req.method === "GET" && m) {
      const t = transactions.get(Number(m[1]));
      return t ? json(200, { "v1/transaction": t }) : json(404, {});
    }
    // La page de paiement : le client « paie », puis revient.
    m = url.pathname.match(/^\/payer\/(\d+)$/);
    if (m) {
      const t = transactions.get(Number(m[1]));
      t.status = "approved";
      if (url.searchParams.get("montant")) t.amount = Number(url.searchParams.get("montant"));
      res.writeHead(302, { location: t.callback_url });
      return res.end();
    }
    json(404, {});
  });
  return new Promise((ok) => serveur.listen(port, "127.0.0.1", () => ok({ transactions, fermer: () => serveur.close() })));
}
