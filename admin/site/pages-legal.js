// Conditions d'utilisation et mentions légales, en français et en anglais.
// Rédigées en mots simples. À faire relire par un juriste avant une
// diffusion à grande échelle (voir docs/passation.md).

import { gabarit, WHATSAPP, TELEPHONE_LISIBLE } from "./pages.js";

/** Éditeur du service : une personne physique, à titre personnel. Le jour
 * où il y a un IFU ou un RCCM, l'écrire dans `immatriculation` (par ex.
 * « IFU : 0201… ») : il s'affiche alors tout seul. */
const EDITEUR = {
  nom: "Gestion Photocopie",
  personne: "Edmond Atinzoun TCHOKPON",
  statut_fr: "Service édité à titre personnel par Edmond Atinzoun TCHOKPON, personne physique résidant au Bénin.",
  statut_en: "Service published in a personal capacity by Edmond Atinzoun TCHOKPON, an individual living in Benin.",
  immatriculation: "",
};

const MAJ_FR = "13 octobre 2026";
const MAJ_EN = "13 October 2026";

export function pageConditions() {
  return gabarit({
    titre: "Conditions d'utilisation — Gestion Photocopie",
    description: "Les règles d'utilisation de Gestion Photocopie pour les clients qui envoient des documents et pour les boutiques qui les reçoivent.",
    chemin: "/conditions",
    alternes: { fr: "/conditions", en: "/en/terms" },
    corps: `
<h1>Conditions d'utilisation</h1>
<p class="petit">Dernière mise à jour : ${MAJ_FR}.</p>
<h2>1. Ce que fait le service</h2>
<p>Gestion Photocopie permet à un client d'envoyer des documents depuis son téléphone à une boutique (photocopie, impression, cybercafé), sur place ou par Internet, et à la boutique de les recevoir, de les imprimer et de tenir sa caisse.</p>
<h2>2. Qui fait quoi</h2>
<ul class="liste">
  <li><strong>La boutique</strong> fixe ses prix, imprime, encaisse et remet les documents. Elle seule répond de son travail, de ses prix et de la remise des documents.</li>
  <li><strong>Gestion Photocopie</strong> fournit l'outil : le logiciel de la boutique, la page d'envoi et le relais Internet. Nous ne vendons pas d'impression et n'encaissons rien pour le compte des boutiques.</li>
</ul>
<h2>3. Pour le client</h2>
<ul class="liste">
  <li>N'envoyez que des documents que vous avez le droit de faire reproduire, et aucun contenu illégal.</li>
  <li>Pour un envoi par Internet, donnez un prénom et un numéro exacts : la boutique s'en sert pour vous prévenir. Une boutique peut refuser les commandes d'un numéro qui envoie de fausses commandes.</li>
  <li>Le prix affiché est une estimation ; la boutique confirme le prix au guichet.</li>
  <li>Par Internet, vos documents attendent la boutique 24 heures au plus. Passé ce délai, ils sont effacés et la commande doit être renvoyée.</li>
  <li>Gardez toujours l'original de vos documents.</li>
</ul>
<h2>4. Pour la boutique</h2>
<ul class="liste">
  <li>Le logiciel s'utilise avec une licence : un mois d'essai gratuit, puis un abonnement payé à l'avance. Le prix est communiqué avant le paiement.</li>
  <li>Une période payée n'est pas remboursée, sauf si le service a été indisponible par notre faute pendant une grande partie de cette période.</li>
  <li>La boutique traite les documents de ses clients avec discrétion, ne les utilise pour rien d'autre que la commande, et les laisse s'effacer (le logiciel le fait tout seul).</li>
  <li>La boutique répond des informations qu'elle publie sur sa page (adresse, horaires, prix).</li>
  <li>Il est interdit de contourner la licence, de copier le logiciel pour le revendre ou d'utiliser le relais pour autre chose que des commandes de clients.</li>
</ul>
<h2>4 bis. Pages frauduleuses et contenus illicites</h2>
<ul class="liste">
  <li>Il est interdit d'utiliser une page de boutique pour tromper les clients : se faire passer pour une banque, une administration ou un opérateur, réclamer des pièces d'identité, des codes ou de l'argent.</li>
  <li>Toute page peut être signalée par le lien « Signaler cette page ». Nous pouvons fermer sans préavis une page frauduleuse ou utilisée pour des contenus illicites, et transmettre les informations utiles aux autorités qui les demandent légalement.</li>
</ul>
<h2>5. Disponibilité</h2>
<p>Nous faisons notre possible pour que le service marche en continu. L'envoi par Internet dépend des réseaux mobiles et de notre hébergeur ; une coupure peut retarder une commande. L'envoi sans Internet, par le Wi-Fi de la boutique, ne dépend d'aucun serveur.</p>
<h2>6. Responsabilité</h2>
<p>Le service est fourni tel qu'il est. Dans la limite permise par la loi, nous ne répondons pas des pertes indirectes (commande manquée, retard, perte de chiffre d'affaires). Notre responsabilité envers une boutique est limitée au montant qu'elle a payé pour les trois derniers mois.</p>
<h2>7. Données personnelles</h2>
<p>Voir la page <a href="/confidentialite">Confidentialité</a>.</p>
<h2>8. Changements</h2>
<p>Ces conditions peuvent évoluer. La date en haut de la page indique la dernière version. Continuer à utiliser le service vaut acceptation de la nouvelle version.</p>
<h2>9. Droit applicable</h2>
<p>Ces conditions relèvent du droit béninois. En cas de désaccord, nous cherchons d'abord une solution à l'amiable ; à défaut, les tribunaux de Cotonou sont compétents, sauf règle obligatoire contraire du pays du client.</p>
<h2>10. Contact</h2>
<p><a href="https://wa.me/${WHATSAPP}">WhatsApp ${TELEPHONE_LISIBLE}</a></p>
`,
  });
}

export function pageTerms() {
  return gabarit({
    lang: "en",
    titre: "Terms of use — Gestion Photocopie",
    description: "The rules for using Gestion Photocopie, for customers who send documents and for the shops that receive them.",
    chemin: "/en/terms",
    alternes: { fr: "/conditions", en: "/en/terms" },
    corps: `
<h1>Terms of use</h1>
<p class="petit">Last updated: ${MAJ_EN}.</p>
<h2>1. What the service does</h2>
<p>Gestion Photocopie lets customers send documents from their phone to a shop (photocopying, printing, cyber café), in the shop or over the Internet, and lets the shop receive, print and keep its till.</p>
<h2>2. Who does what</h2>
<ul class="liste">
  <li><strong>The shop</strong> sets its prices, prints, takes payment and hands over the documents. The shop alone is responsible for its work, its prices and handing over the documents.</li>
  <li><strong>Gestion Photocopie</strong> provides the tool: the shop software, the sending page and the Internet relay. We do not sell printing and do not take payments on behalf of shops.</li>
</ul>
<h2>3. For customers</h2>
<ul class="liste">
  <li>Only send documents you have the right to have copied, and nothing illegal.</li>
  <li>When sending over the Internet, give a real first name and phone number: the shop uses them to tell you when your order is ready. A shop may refuse orders from a number that sends fake orders.</li>
  <li>The price shown is an estimate; the shop confirms it at the counter.</li>
  <li>Over the Internet, your documents wait for the shop for 24 hours at most. After that they are deleted and the order must be sent again.</li>
  <li>Always keep the original of your documents.</li>
</ul>
<h2>4. For shops</h2>
<ul class="liste">
  <li>The software is used with a licence: one month free trial, then a subscription paid in advance. The price is given before payment.</li>
  <li>A paid period is not refunded, unless the service was unavailable through our fault for a large part of that period.</li>
  <li>The shop handles its customers' documents discreetly, uses them only for the order, and lets them be deleted (the software does this automatically).</li>
  <li>The shop is responsible for the information it publishes on its page (address, opening hours, prices).</li>
  <li>Circumventing the licence, copying the software to resell it, or using the relay for anything other than customer orders is forbidden.</li>
</ul>
<h2>4a. Fraudulent pages and illegal content</h2>
<ul class="liste">
  <li>Using a shop page to deceive customers is forbidden: pretending to be a bank, an authority or an operator, asking for ID documents, codes or money.</li>
  <li>Any page can be reported with the "Report this page" link. We may close a fraudulent page, or one used for illegal content, without notice, and pass relevant information to authorities that lawfully request it.</li>
</ul>
<h2>5. Availability</h2>
<p>We do our best to keep the service running. Sending over the Internet depends on mobile networks and our hosting provider; an outage may delay an order. Sending without Internet, over the shop's Wi-Fi, does not depend on any server.</p>
<h2>6. Liability</h2>
<p>The service is provided as is. To the extent permitted by law, we are not liable for indirect losses (missed order, delay, lost revenue). Our liability towards a shop is limited to the amount it paid for the last three months.</p>
<h2>7. Personal data</h2>
<p>See the <a href="/en/privacy">Privacy</a> page.</p>
<h2>8. Changes</h2>
<p>These terms may change. The date at the top shows the latest version. Continuing to use the service means accepting the new version.</p>
<h2>9. Governing law</h2>
<p>These terms are governed by the law of Benin. In case of disagreement, we first look for an amicable solution; failing that, the courts of Cotonou have jurisdiction, unless a mandatory rule of the customer's country says otherwise.</p>
<h2>10. Contact</h2>
<p><a href="https://wa.me/${WHATSAPP}">WhatsApp ${TELEPHONE_LISIBLE}</a></p>
`,
  });
}

const HEBERGEUR = "Cloudflare, Inc., 101 Townsend Street, San Francisco, CA 94107, États-Unis — cloudflare.com";
const HOST = "Cloudflare, Inc., 101 Townsend Street, San Francisco, CA 94107, United States — cloudflare.com";

export function pageMentions() {
  return gabarit({
    titre: "Mentions légales — Gestion Photocopie",
    description: "Éditeur, hébergeur et contact du site et du service Gestion Photocopie.",
    chemin: "/mentions-legales",
    alternes: { fr: "/mentions-legales", en: "/en/legal" },
    corps: `
<h1>Mentions légales</h1>
<h2>Éditeur</h2>
<p><strong>${EDITEUR.nom}</strong><br>${EDITEUR.statut_fr}${EDITEUR.immatriculation ? `<br>${EDITEUR.immatriculation}` : ""}<br>Contact : <a href="https://wa.me/${WHATSAPP}">WhatsApp ${TELEPHONE_LISIBLE}</a></p>
<h2>Directeur de la publication</h2>
<p>${EDITEUR.personne}</p>
<h2>Hébergement</h2>
<p>Le site et le relais Internet sont hébergés par ${HEBERGEUR}.</p>
<h2>Propriété</h2>
<p>Le logiciel, les pages et les textes de Gestion Photocopie sont protégés. Les documents envoyés par les clients restent la propriété de leurs auteurs.</p>
<p>Voir aussi : <a href="/conditions">conditions d'utilisation</a>, <a href="/confidentialite">confidentialité</a>.</p>
`,
  });
}

export function pageLegal() {
  return gabarit({
    lang: "en",
    titre: "Legal notice — Gestion Photocopie",
    description: "Publisher, host and contact for the Gestion Photocopie website and service.",
    chemin: "/en/legal",
    alternes: { fr: "/mentions-legales", en: "/en/legal" },
    corps: `
<h1>Legal notice</h1>
<h2>Publisher</h2>
<p><strong>${EDITEUR.nom}</strong><br>${EDITEUR.statut_en}${EDITEUR.immatriculation ? `<br>${EDITEUR.immatriculation}` : ""}<br>Contact: <a href="https://wa.me/${WHATSAPP}">WhatsApp ${TELEPHONE_LISIBLE}</a></p>
<h2>Publication director</h2>
<p>${EDITEUR.personne}</p>
<h2>Hosting</h2>
<p>The website and the Internet relay are hosted by ${HOST}.</p>
<h2>Ownership</h2>
<p>The Gestion Photocopie software, pages and texts are protected. Documents sent by customers remain the property of their authors.</p>
<p>See also: <a href="/en/terms">terms of use</a>, <a href="/en/privacy">privacy</a>.</p>
`,
  });
}
