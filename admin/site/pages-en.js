// Le site en anglais (Ghana, Nigeria, Kenya… et tous les anglophones).
// Mêmes pages qu'en français, mêmes informations.

import { gabarit, WHATSAPP, TELEPHONE_LISIBLE, ADRESSE_ADMIN } from "./pages.js";

export function pageHowItWorks() {
  return gabarit({
    lang: "en",
    titre: "How it works — Gestion Photocopie",
    description: "Installing Gestion Photocopie, sending documents with a QR code, shop Wi-Fi, sending over the Internet, Bluetooth and USB: the guide in plain words.",
    chemin: "/en/how-it-works",
    alternes: { fr: "/comment-ca-marche", en: "/en/how-it-works" },
    corps: `
<h1>How it works</h1>
<h2>1. Install the software (once)</h2>
<ul class="liste">
  <li>Download the Windows software from the <a href="/en/download">Download</a> page, then open the file.</li>
  <li>Tap "English" on the first screen (or later in Settings), enter the shop name and your country: that's it. The software then starts on its own with the computer.</li>
  <li>Print the poster with the QR codes from the software and put it up at the counter.</li>
</ul>
<h2>2. The customer sends</h2>
<h3>Without Internet, over the shop's Wi-Fi</h3>
<ul class="liste">
  <li>The customer scans the first code: the phone joins the shop's Wi-Fi and the sending page opens (otherwise, scan the second code).</li>
  <li>On Android, the customer turns off <strong>mobile data</strong> while sending: otherwise the phone looks for the shop on the Internet instead of the Wi-Fi.</li>
  <li>If the computer cannot create a Wi-Fi network, a small Wi-Fi router plugged into the computer does the same job.</li>
</ul>
<h3>Over the Internet</h3>
<ul class="liste">
  <li>The customer scans the "Over the Internet" code: the shop's page opens with mobile data, from anywhere — at home, at work.</li>
  <li>The page shows the shop's address, opening hours, phone and map. The customer gives a first name and WhatsApp number, chooses when to collect, and sends.</li>
  <li>The shop's computer collects the order as soon as it has a connection; the customer receives a number and finds the order in "My orders".</li>
  <li>When it is ready, the shop owner taps "Notify": WhatsApp opens with the message already written.</li>
</ul>
<h3>Fallbacks</h3>
<ul class="liste">
  <li><strong>Bluetooth</strong> (Android): pick the document, "Share", "Bluetooth", then the computer's name.</li>
  <li><strong>USB cable or stick</strong>: the software detects the phone or stick and offers the documents.</li>
</ul>
<h2>3. The shop prints and takes payment</h2>
<ul class="liste">
  <li>Each order arrives with a number, the customer's options and the calculated price.</li>
  <li>The shop opens, prints, takes payment; the customer sees "printed", then "ready" on their phone.</li>
</ul>
<p class="petit">Need help? <a href="https://wa.me/${WHATSAPP}">WhatsApp ${TELEPHONE_LISIBLE}</a>.</p>
`,
  });
}

const FAQ_EN = [
  ["Does the customer need to install an app?", "No. They scan the QR code at the counter and the sending page opens in their browser, on Android and iPhone. The Android app is optional."],
  ["Does the shop need Internet?", "No. The computer creates the shop's Wi-Fi and receives documents without Internet. If the computer has a connection, customers can also send over the Internet with their mobile data, from anywhere."],
  ["Is it available in English and outside Benin?", "Yes. The shop software and the customer page are in English and French (the page follows the phone's language). The shop chooses its country: currency and phone code follow (cedi, naira, shilling, CFA franc…)."],
  ["Does a customer sending over the Internet have to give a number?", "Yes. They are not at the counter, so the shop must be able to tell them when the order is ready. The shop does it in one tap on WhatsApp. Fake orders can be blocked by number."],
  ["Which documents can be sent?", "PDF, Word, Excel, photos and most files. The customer chooses copies, colour, double-sided and size, and can leave a voice message."],
  ["How much does it cost?", "One month free trial. Then a small monthly subscription; the team gives the price on WhatsApp."],
  ["Are my documents kept?", "No. Over the shop's Wi-Fi they go straight to the computer. Over the Internet they pass through the relay while the shop collects them and are deleted there within 24 hours of sending at the latest."],
  ["Which computers does it run on?", "Windows 10 and 11. It starts with the computer and works in the background; its window only opens when the shop asks."],
];

export function pageFaqEn() {
  const jsonld = `<script type="application/ld+json">${JSON.stringify({
    "@context": "https://schema.org",
    "@type": "FAQPage",
    mainEntity: FAQ_EN.map(([q, r]) => ({ "@type": "Question", name: q, acceptedAnswer: { "@type": "Answer", text: r } })),
  })}</script>`;
  return gabarit({
    lang: "en",
    titre: "FAQ — Gestion Photocopie",
    description: "Do you need Internet? Does the customer need an app? Is it in English? How much does it cost? Answers about Gestion Photocopie, software for print shops.",
    chemin: "/en/faq",
    alternes: { fr: "/faq", en: "/en/faq" },
    jsonld,
    corps: `<h1>Frequently asked questions</h1>${FAQ_EN.map(([q, r]) => `<div class="carte" style="margin:12px 0"><h3>${q}</h3><p>${r}</p></div>`).join("")}
<p class="petit">Another question? <a href="https://wa.me/${WHATSAPP}">WhatsApp ${TELEPHONE_LISIBLE}</a>.</p>`,
  });
}

export function pageDownload() {
  return gabarit({
    lang: "en",
    titre: "Download Gestion Photocopie — Windows and Android",
    description: "Download the Gestion Photocopie shop software for Windows (one month free trial) and the optional Android app for customers.",
    chemin: "/en/download",
    alternes: { fr: "/telecharger", en: "/en/download" },
    corps: `
<h1>Download</h1>
<div class="grille">
  <div class="carte">
    <h3>💻 Shop software (Windows)</h3>
    <p>For the counter computer. Windows 10 or 11. One month free trial. Choose English in Settings.</p>
    <div class="boutons"><a class="btn" href="${ADRESSE_ADMIN}/telecharger/exe">Download for Windows</a></div>
    <p class="petit">Already installed? The <a href="${ADRESSE_ADMIN}/telecharger/leger">light update</a> is enough. Checksums: <a href="${ADRESSE_ADMIN}/telecharger">detailed page</a>.</p>
  </div>
  <div class="carte">
    <h3>📱 Sender app (Android, optional)</h3>
    <p>For customers who want to send faster. Android 10 or later. Not required: the sending page works in the browser.</p>
    <div class="boutons"><a class="btn second" href="${ADRESSE_ADMIN}/telecharger/apk">Download the app</a></div>
  </div>
</div>
<p class="petit">Windows may show a warning the first time: click "More info", then "Run anyway".</p>
`,
  });
}

export function pagePrivacy() {
  return gabarit({
    lang: "en",
    titre: "Privacy — Gestion Photocopie",
    description: "What Gestion Photocopie does with your documents and information: nothing is sold, documents sent over the Internet are deleted as soon as the shop has received them.",
    chemin: "/en/privacy",
    alternes: { fr: "/confidentialite", en: "/en/privacy" },
    corps: `
<h1>Privacy</h1>
<p class="petit">Last updated: 13 October 2026.</p>
<h2>In short</h2>
<ul class="liste">
  <li>Your documents are used for one thing only: being printed by the shop you chose.</li>
  <li>We do not sell, rent or share any data.</li>
  <li>Without Internet (shop Wi-Fi, Bluetooth, cable), your documents go straight to the shop's computer: they pass through no server.</li>
  <li>Over the Internet, your documents pass through our relay while the shop's computer collects them, then they are <strong>deleted</strong> — within 24 hours of sending at the latest, even if not collected.</li>
</ul>
<h2>What is collected</h2>
<ul class="liste">
  <li><strong>The documents</strong> you send, and the options chosen (copies, colour, double-sided…).</li>
  <li><strong>Your first name and phone number</strong>: optional in the shop; requested when sending over the Internet, because you are not at the counter and the shop must be able to tell you (on WhatsApp) when it is ready. A shop may refuse orders from a number that sends fake orders.</li>
  <li><strong>On your phone</strong>, the page keeps your recent orders for 7 days ("My orders") so you can find your number. None of this is sent anywhere else.</li>
  <li><strong>A tracking number</strong> for the order, to tell you "printed", then "ready". It is deleted from the relay after 7 days.</li>
</ul>
<h2>Where your documents go</h2>
<ul class="liste">
  <li>In the shop: on the shop's computer, which keeps them as long as needed for the work, then deletes them automatically.</li>
  <li>Over the Internet: on the service's relay, hosted by Cloudflare, only while the shop collects them. Cloudflare has servers worldwide, including in the United States: your documents may therefore travel outside Benin, encrypted in transit.</li>
  <li>Only the shop's computer, with its secret key, can collect the documents sent to it.</li>
</ul>
<h2>For shops</h2>
<p>The software keeps working information (orders, till, reports) on the shop's computer. It is not sent to us. For the licence, we know the shop name, the manager's name and number, and the computer's identifier.</p>
<h2>Your rights</h2>
<p>You can ask what we hold about you, have it corrected or deleted: write to us on <a href="https://wa.me/${WHATSAPP}">WhatsApp ${TELEPHONE_LISIBLE}</a>. You can also contact the data protection authority of your country (for example the Data Protection Commission in Ghana, the NDPC in Nigeria, the ODPC in Kenya, the APDP in Benin).</p>
<h2>Fraudulent pages</h2>
<p>A shop page that pretends to be a bank or an authority, or asks for ID documents or money, can be reported with the "Report this page" link at the bottom of the page. We close it if needed.</p>
<p>See also the <a href="/en/terms">terms of use</a> and the <a href="/en/legal">legal notice</a>.</p>
`,
  });
}
