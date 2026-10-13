// Faux SebPay pour le banc d'essai (admin/src/sebpay.js) : mêmes adresses et
// mêmes formes que la documentation de l'API v1 (new.sebpay.bj).
import http from "node:http";
import crypto from "node:crypto";

export function demarrerFauxSebpay(port, { publique = "pk_test_banc", secrete = "sk_test_banc", relectureAbsente = false } = {}) {
  const collectes = new Map(); // external_reference → collecte
  const etat = { relectureAbsente };
  let suivant = 1;
  const serveur = http.createServer(async (req, res) => {
    let corps = "";
    for await (const morceau of req) corps += morceau;
    const json = (code, o) => { res.writeHead(code, { "content-type": "application/json" }); res.end(JSON.stringify(o)); };
    const url = new URL(req.url, "http://x");
    if (req.headers["x-public-key"] !== publique || req.headers["x-secret-key"] !== secrete) return json(401, { message: "Unauthenticated." });
    if (req.method === "GET" && url.pathname === "/api/v1/operators") {
      const pays = url.searchParams.get("country");
      const tous = [
        { name: "MTN Mobile Money", slug: "mtn", country: "BJ", otp_required: false, ussd_code: null },
        { name: "Moov Money", slug: "moov", country: "BJ", otp_required: false, ussd_code: null },
        { name: "Orange Money", slug: "orange", country: "CI", otp_required: true, ussd_code: "#144*82#" },
        { name: "MTN Mobile Money", slug: "mtn", country: "CI", otp_required: false, ussd_code: null },
      ];
      return json(200, { success: true, data: tous.filter((o) => !pays || o.country === pays) });
    }
    if (req.method === "POST" && url.pathname === "/api/v1/collections") {
      const d = JSON.parse(corps);
      for (const champ of ["amount", "currency", "phone", "operator", "country", "external_reference"]) {
        if (!d[champ]) return json(422, { message: `The ${champ} field is required.` });
      }
      if (d.operator === "orange" && !d.otp_code) return json(422, { message: "otp_code required" });
      if (collectes.has(d.external_reference)) return json(200, { success: true, data: collectes.get(d.external_reference) });
      const c = {
        transaction_id: String(20261013000000 + suivant++),
        external_reference: d.external_reference,
        status: "pending",
        amount: d.amount,
        currency: d.currency,
        customer_phone: d.phone,
        operator: d.operator,
        callback_url: d.callback_url,
        created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
      };
      collectes.set(c.external_reference, c);
      return json(201, { success: true, data: c });
    }
    const m = url.pathname.match(/^\/api\/v1\/collections\/([^/]+)$/);
    if (req.method === "GET" && m) {
      if (etat.relectureAbsente) return json(404, { message: "Not Found" });
      const ref = decodeURIComponent(m[1]);
      const c = collectes.get(ref) || [...collectes.values()].find((x) => x.transaction_id === ref);
      return c ? json(200, { success: true, data: c }) : json(404, { message: "Not Found" });
    }
    json(404, { message: "Not Found" });
  });

  /** Le gérant valide (ou refuse) sur son téléphone ; SebPay prévient (ou pas). */
  async function conclure(reference, { statut = "approved", montant, avis = true, secret = secrete } = {}) {
    const c = collectes.get(reference);
    c.status = statut;
    if (montant) c.amount = montant;
    c.updated_at = new Date().toISOString();
    if (!avis) return null;
    const { callback_url, operator, ...charge } = c;
    const corpsAvis = JSON.stringify(charge);
    const signature = crypto.createHmac("sha256", secret).update(corpsAvis).digest("hex");
    return fetch(callback_url, { method: "POST", headers: { "content-type": "application/json", "x-sebpay-signature": signature }, body: corpsAvis });
  }

  return new Promise((ok) => serveur.listen(port, "127.0.0.1", () => ok({ collectes, conclure, etat, fermer: () => serveur.close() })));
}
