// Le site public : présentation, téléchargement, mode d'emploi,
// confidentialité. Léger (aucune ressource extérieure) : il doit s'ouvrir
// vite sur un téléphone, avec une petite connexion.

export const ADRESSE_ADMIN = "https://photocopie-admin.atinzed2.workers.dev";
const WHATSAPP = "2290151226741";
const TELEPHONE_LISIBLE = "01 51 22 67 41";

/** Couleurs du thème clair, en clair : la valeur de secours de chaque
 * couleur. Opera Mini, UC Browser et les vieux Android, très répandus ici,
 * ignorent les variables CSS : sans secours, les boutons y étaient du texte
 * blanc sur fond transparent — invisibles. */
const CLAIR = { fond: "#f6f7fb", carte: "#fff", encre: "#16213a", doux: "#55617a", accent: "#0b5cad", "accent-doux": "#e3eefb", vert: "#137a3d", trait: "#dfe3ec" };
function avecReplis(css) {
  return css.replace(/([a-z-]+):([^;{}]*?var\(--[a-z-]+\)[^;{}]*)/g, (tout, propriete, valeur) => {
    if (propriete.startsWith("--")) return tout;
    const repli = valeur.replace(/var\(--([a-z-]+)\)/g, (_, nom) => CLAIR[nom] || "inherit");
    return `${propriete}:${repli};${propriete}:${valeur}`;
  });
}

const STYLE = avecReplis(`
:root{--fond:#f6f7fb;--carte:#fff;--encre:#16213a;--doux:#55617a;--accent:#0b5cad;--accent-doux:#e3eefb;--vert:#137a3d;--trait:#dfe3ec}
@media (prefers-color-scheme:dark){:root{--fond:#0f1420;--carte:#171e2e;--encre:#eef2fa;--doux:#a9b4c9;--accent:#5ea2ec;--accent-doux:#1b2a44;--vert:#48c27a;--trait:#2a3348}}
*{box-sizing:border-box}html{-webkit-text-size-adjust:100%}
body{margin:0;background:var(--fond);color:var(--encre);font:16px/1.55 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif}
a{color:var(--accent)}
.page{max-width:980px;margin:0 auto;padding:0 16px}
header.haut{display:flex;align-items:center;justify-content:space-between;gap:12px;padding:14px 0}
.marque{font-weight:800;font-size:1.1rem;text-decoration:none;color:var(--encre)}
nav{display:flex;flex-wrap:wrap;gap:6px 14px;justify-content:flex-end}nav a{font-size:.95rem;text-decoration:none}
.heros{padding:28px 0 8px}
h1{font-size:clamp(1.7rem,5vw,2.6rem);line-height:1.15;margin:.2em 0 .4em}
h2{font-size:1.45rem;margin:1.6em 0 .5em}
h3{font-size:1.1rem;margin:.2em 0 .3em}
.chapeau{font-size:1.15rem;color:var(--doux);max-width:46em}
.boutons{display:flex;flex-wrap:wrap;gap:12px;margin:22px 0}
.btn{display:inline-block;padding:14px 20px;border-radius:12px;background:var(--accent);color:#fff;text-decoration:none;font-weight:700}
.btn.second{background:var(--accent-doux);color:var(--accent)}
.btn small{display:block;font-weight:500;opacity:.85;font-size:.8rem}
.grille{display:grid;grid-template-columns:repeat(auto-fit,minmax(240px,1fr));gap:14px}
.carte{background:var(--carte);border:1px solid var(--trait);border-radius:14px;padding:18px}
.etape{display:flex;gap:12px}.etape b{flex:none;width:34px;height:34px;border-radius:50%;background:var(--accent);color:#fff;display:grid;place-items:center}
.coche{color:var(--vert);font-weight:800}
ul.liste{padding-left:1.1em}ul.liste li{margin:.35em 0}
footer{margin:48px 0 24px;padding-top:18px;border-top:1px solid var(--trait);color:var(--doux);font-size:.9rem}
.petit{font-size:.9rem;color:var(--doux)}
`);

const RACINE = "https://photocopie-benin.atinzed2.workers.dev";

function gabarit({ titre, description, chemin, corps, jsonld = "", lang = "fr", alternes = null, indexer = true }) {
  const url = `${RACINE}${chemin}`;
  const liens = alternes
    ? Object.entries(alternes).map(([l, c]) => `<link rel="alternate" hreflang="${l}" href="${RACINE}${c}">`).join("\n") +
      `\n<link rel="alternate" hreflang="x-default" href="${RACINE}${alternes.fr || chemin}">`
    : "";
  const en = lang === "en";
  return `<!doctype html>
<html lang="${lang}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${titre}</title>
<meta name="description" content="${description}">
<link rel="canonical" href="${url}">
${liens}
${indexer ? "" : '<meta name="robots" content="noindex">'}
<meta property="og:image" content="${RACINE}/apercu.png">
<meta property="og:image:width" content="1200">
<meta property="og:image:height" content="630">
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:image" content="${RACINE}/apercu.png">
<meta property="og:type" content="website">
<meta property="og:title" content="${titre}">
<meta property="og:description" content="${description}">
<meta property="og:url" content="${url}">
<meta property="og:locale" content="${en ? "en_US" : "fr_FR"}">
<meta name="theme-color" content="#0b5cad">
<link rel="icon" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 64 64'%3E%3Crect width='64' height='64' rx='14' fill='%230b5cad'/%3E%3Cpath d='M18 24h28v22H18z' fill='%23fff'/%3E%3Cpath d='M22 14h20v12H22z' fill='%23cfe2fb'/%3E%3C/svg%3E">
${jsonld}
<style>${STYLE}</style>
</head>
<body>
<div class="page">
<header class="haut">
  <a class="marque" href="/">🖨️ Gestion Photocopie</a>
  ${en
    ? `<nav><a href="/telecharger">Download</a><a href="/" hreflang="fr">Français</a></nav>`
    : `<nav><a href="/comment-ca-marche">Comment ça marche</a><a href="/faq">Questions</a><a href="/telecharger">Télécharger</a><a href="/en" hreflang="en">English</a></nav>`}
</header>
${corps}
<footer>
  <p><strong>Gestion Photocopie</strong> — le logiciel des boutiques de photocopie et cybercafés du Bénin.</p>
  <p><a href="/comment-ca-marche">Comment ça marche</a> · <a href="/faq">Questions fréquentes</a> · <a href="/telecharger">Télécharger</a> · <a href="/confidentialite">Confidentialité</a> · <a href="/en">English</a> · <a href="https://wa.me/${WHATSAPP}">WhatsApp ${TELEPHONE_LISIBLE}</a></p>
</footer>
</div>
</body>
</html>`;
}

export function pageAccueil() {
  const jsonld = `<script type="application/ld+json">${JSON.stringify({
    "@context": "https://schema.org",
    "@type": "SoftwareApplication",
    name: "Gestion Photocopie",
    operatingSystem: "Windows, Android",
    applicationCategory: "BusinessApplication",
    description:
      "Logiciel pour boutique de photocopie : les clients envoient leurs documents depuis leur téléphone en scannant un QR code, le gérant les reçoit sur son ordinateur, imprime et encaisse.",
    inLanguage: "fr",
    areaServed: "BJ",
    offers: { "@type": "Offer", price: "0", priceCurrency: "XOF", description: "Un mois d'essai gratuit" },
  })}</script>`;
  return gabarit({
    titre: "Gestion Photocopie — vos clients envoient leurs documents par QR code | Bénin",
    description:
      "Le client scanne un QR code et envoie ses documents depuis son téléphone : le gérant les reçoit sur son ordinateur, sans câble, sans clé USB, avec ou sans Internet. Logiciel pour photocopie et cybercafé au Bénin.",
    chemin: "/",
    alternes: { fr: "/", en: "/en" },
    jsonld,
    corps: `
<section class="heros">
  <h1>Vos clients envoient leurs documents depuis leur téléphone. Vous imprimez.</h1>
  <p class="chapeau">Fini la clé USB, le câble et les « envoie-moi sur WhatsApp ». Le client scanne le QR code du guichet, choisit ses documents, et ils arrivent sur l'ordinateur de la boutique, prêts à imprimer.</p>
  <div class="boutons">
    <a class="btn" href="${ADRESSE_ADMIN}/telecharger/exe">⬇️ Télécharger pour Windows<small>Logiciel de la boutique · un mois d'essai gratuit</small></a>
    <a class="btn second" href="${ADRESSE_ADMIN}/telecharger/apk">📱 Application Android<small>Facultative, pour les clients</small></a>
    <a class="btn second" href="/comment-ca-marche">Voir comment ça marche</a>
  </div>
</section>

<h2>Avec ou sans Internet</h2>
<div class="grille">
  <div class="carte"><h3>📶 Sans Internet</h3><p>L'ordinateur crée le Wi-Fi de la boutique (ou un petit routeur le fait). Le client le rejoint et envoie. Rien ne sort de la boutique.</p></div>
  <div class="carte"><h3>🌍 Par Internet</h3><p>Le client envoie avec ses propres données mobiles, depuis le trottoir ou depuis chez lui. L'ordinateur reçoit dès qu'il a une petite connexion.</p></div>
  <div class="carte"><h3>🔵 Bluetooth et clé USB</h3><p>Toujours possibles en secours : le logiciel range tout dans la même file d'attente.</p></div>
</div>

<h2>Ce que le gérant y gagne</h2>
<div class="grille">
  <div class="carte"><p><span class="coche">✓</span> <strong>Moins d'attente au guichet</strong> : la commande arrive avec le nombre de copies, la couleur, le recto-verso.</p></div>
  <div class="carte"><p><span class="coche">✓</span> <strong>Un numéro pour chaque client</strong>, et le client voit sur son téléphone quand c'est imprimé, puis prêt.</p></div>
  <div class="carte"><p><span class="coche">✓</span> <strong>La caisse tenue toute seule</strong> : prix calculé, encaissement, rapport du jour, rien ne disparaît.</p></div>
  <div class="carte"><p><span class="coche">✓</span> <strong>Les impressions contrôlées</strong> : ce qui est imprimé et ce qui est payé, visibles par le patron.</p></div>
</div>

<h2>Pour le client : trois gestes</h2>
<div class="grille">
  <div class="carte etape"><b>1</b><p>Il scanne le QR code affiché au guichet.</p></div>
  <div class="carte etape"><b>2</b><p>Il choisit ses documents (PDF, Word, photos…).</p></div>
  <div class="carte etape"><b>3</b><p>Il touche « Envoyer » et reçoit son numéro.</p></div>
</div>
<p class="petit">Aucune application à installer pour le client : la page s'ouvre dans son navigateur, sur Android comme sur iPhone.</p>

<h2>Essayer</h2>
<p>Installez le logiciel sur l'ordinateur de la boutique : un mois d'essai gratuit, sans engagement. Une question ? Écrivez-nous sur <a href="https://wa.me/${WHATSAPP}">WhatsApp (${TELEPHONE_LISIBLE})</a>.</p>
<div class="boutons"><a class="btn" href="/telecharger">Télécharger</a></div>
`,
  });
}

export function pageComment() {
  return gabarit({
    titre: "Comment ça marche — Gestion Photocopie",
    description:
      "Installation du logiciel Gestion Photocopie, envoi des documents par QR code, Wi-Fi de la boutique, envoi par Internet, Bluetooth et clé USB : le mode d'emploi en mots simples.",
    chemin: "/comment-ca-marche",
    corps: `
<h1>Comment ça marche</h1>
<h2>1. Installer le logiciel (une fois)</h2>
<ul class="liste">
  <li>Téléchargez le logiciel pour Windows depuis la page <a href="/telecharger">Télécharger</a>, puis ouvrez le fichier.</li>
  <li>Donnez le nom de la boutique : c'est fini. Le logiciel démarre ensuite tout seul avec l'ordinateur, sans fenêtre qui s'ouvre d'elle-même.</li>
  <li>Imprimez l'affiche avec les QR codes depuis le logiciel, et collez-la au guichet.</li>
</ul>
<h2>2. Le client envoie</h2>
<h3>Sans Internet, par le Wi-Fi de la boutique</h3>
<ul class="liste">
  <li>Le client scanne le premier code : son téléphone rejoint le Wi-Fi de la boutique. La page d'envoi s'ouvre (sinon, il scanne le second code).</li>
  <li>Sur Android, il coupe ses <strong>données mobiles</strong> le temps de l'envoi : sinon le téléphone cherche la boutique sur Internet au lieu du Wi-Fi.</li>
  <li>Si l'ordinateur ne sait pas créer de Wi-Fi, un petit routeur Wi-Fi branché à l'ordinateur fait la même chose.</li>
</ul>
<h3>Par Internet</h3>
<ul class="liste">
  <li>Le client scanne le code « Par Internet » : la page de la boutique s'ouvre avec ses données mobiles, où qu'il soit.</li>
  <li>Il envoie ; l'ordinateur de la boutique récupère la commande dès qu'il a une connexion, et le client reçoit son numéro.</li>
</ul>
<h3>En secours</h3>
<ul class="liste">
  <li><strong>Bluetooth</strong> (Android) : choisir le document, « Partager », « Bluetooth », puis le nom de l'ordinateur.</li>
  <li><strong>Câble ou clé USB</strong> : le logiciel repère le téléphone ou la clé et propose les documents.</li>
</ul>
<h2>3. Le gérant imprime et encaisse</h2>
<ul class="liste">
  <li>Chaque commande arrive avec un numéro, les options du client et le prix calculé.</li>
  <li>Le gérant ouvre, imprime, encaisse ; le client voit sur son téléphone « imprimé », puis « prêt ».</li>
</ul>
<p class="petit">Besoin d'aide ? <a href="https://wa.me/${WHATSAPP}">WhatsApp ${TELEPHONE_LISIBLE}</a>.</p>
`,
  });
}

export function pageTelechargerSite() {
  return gabarit({
    titre: "Télécharger Gestion Photocopie — Windows et Android",
    description:
      "Téléchargez le logiciel Gestion Photocopie pour Windows (un mois d'essai gratuit) et l'application Envoyeur Kiosque pour Android.",
    chemin: "/telecharger",
    corps: `
<h1>Télécharger</h1>
<div class="grille">
  <div class="carte">
    <h3>💻 Logiciel de la boutique (Windows)</h3>
    <p>Pour l'ordinateur du guichet. Windows 10 ou 11. Un mois d'essai gratuit.</p>
    <div class="boutons"><a class="btn" href="${ADRESSE_ADMIN}/telecharger/exe">Télécharger pour Windows</a></div>
    <p class="petit">Déjà installé ? La <a href="${ADRESSE_ADMIN}/telecharger/leger">mise à jour légère</a> suffit. Empreintes de vérification : <a href="${ADRESSE_ADMIN}/telecharger">page détaillée</a>.</p>
  </div>
  <div class="carte">
    <h3>📱 Application Envoyeur (Android, facultative)</h3>
    <p>Pour les clients qui veulent envoyer plus vite. Android 10 ou plus récent. Elle n'est pas nécessaire : la page d'envoi marche dans le navigateur.</p>
    <div class="boutons"><a class="btn second" href="${ADRESSE_ADMIN}/telecharger/apk">Télécharger l'application</a></div>
  </div>
</div>
<p class="petit">Windows peut afficher un avertissement au premier lancement : touchez « Informations complémentaires », puis « Exécuter quand même ».</p>
`,
  });
}

export function pageConfidentialite() {
  return gabarit({
    titre: "Confidentialité — Gestion Photocopie",
    description:
      "Ce que Gestion Photocopie fait de vos documents et de vos informations : rien n'est vendu, les documents envoyés par Internet sont effacés dès que la boutique les a reçus.",
    chemin: "/confidentialite",
    corps: `
<h1>Confidentialité</h1>
<p class="petit">Dernière mise à jour : 12 octobre 2026.</p>
<h2>En bref</h2>
<ul class="liste">
  <li>Vos documents servent à une seule chose : être imprimés par la boutique que vous avez choisie.</li>
  <li>Nous ne vendons, ne louons et ne partageons aucune donnée.</li>
  <li>Sans Internet (Wi-Fi de la boutique, Bluetooth, câble), vos documents vont directement à l'ordinateur de la boutique : ils ne passent par aucun serveur.</li>
  <li>Par Internet, vos documents passent par notre relais le temps que l'ordinateur de la boutique les récupère, puis ils y sont <strong>effacés</strong> — au plus tard 24 heures après l'envoi, même s'ils n'ont pas été récupérés.</li>
</ul>
<h2>Ce qui est collecté</h2>
<ul class="liste">
  <li><strong>Les documents</strong> que vous envoyez, et les options choisies (copies, couleur, recto-verso…).</li>
  <li><strong>Votre prénom et votre numéro de téléphone</strong>, seulement si vous les donnez : ils permettent à la boutique de vous appeler quand c'est prêt.</li>
  <li><strong>Un numéro de suivi</strong> de la commande, pour vous dire « imprimé », puis « prêt ». Il est supprimé du relais au bout de 7 jours.</li>
</ul>
<h2>Où vont vos documents</h2>
<ul class="liste">
  <li>Dans la boutique : sur l'ordinateur du gérant, qui les garde le temps nécessaire au travail, puis les efface automatiquement.</li>
  <li>Par Internet : sur le relais du service, hébergé chez Cloudflare, uniquement le temps de la récupération.</li>
  <li>Seul l'ordinateur de la boutique, avec sa clé secrète, peut récupérer les documents qui lui sont envoyés.</li>
</ul>
<h2>Pour les gérants</h2>
<p>Le logiciel garde sur l'ordinateur de la boutique les informations de travail (commandes, caisse, rapports). Elles ne nous sont pas transmises. Pour la licence, nous connaissons le nom de la boutique, le nom et le numéro du gérant, et l'identifiant de l'ordinateur.</p>
<h2>Vos droits</h2>
<p>Vous pouvez demander à savoir ce que nous détenons sur vous, le faire corriger ou effacer : écrivez-nous sur <a href="https://wa.me/${WHATSAPP}">WhatsApp ${TELEPHONE_LISIBLE}</a>. Au Bénin, vous pouvez aussi vous adresser à l'Autorité de Protection des Données à caractère Personnel (APDP).</p>
`,
  });
}

export function robots() {
  return `User-agent: *\nAllow: /\nDisallow: /b/\nDisallow: /api/\nSitemap: https://photocopie-benin.atinzed2.workers.dev/sitemap.xml\n`;
}

export function sitemap() {
  const pages = ["/", "/en", "/comment-ca-marche", "/faq", "/telecharger", "/confidentialite"];
  return `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${pages
    .map((p) => `  <url><loc>https://photocopie-benin.atinzed2.workers.dev${p}</loc><lastmod>2026-10-12</lastmod></url>`)
    .join("\n")}\n</urlset>\n`;
}

/** QR code du site (pour l'affiche), fabriqué une fois : il ne change pas. */
const QR_SITE = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 35 35" shape-rendering="crispEdges"><path fill="#ffffff" d="M0 0h35v35H0z"/><path stroke="#16213a" d="M1 1.5h7m3 0h1m2 0h2m1 0h6m1 0h1m2 0h7M1 2.5h1m5 0h1m2 0h2m1 0h4m1 0h1m2 0h1m1 0h1m3 0h1m5 0h1M1 3.5h1m1 0h3m1 0h1m1 0h1m1 0h2m1 0h1m2 0h3m1 0h2m1 0h1m2 0h1m1 0h3m1 0h1M1 4.5h1m1 0h3m1 0h1m1 0h4m2 0h1m1 0h1m3 0h1m2 0h1m2 0h1m1 0h3m1 0h1M1 5.5h1m1 0h3m1 0h1m1 0h1m2 0h1m1 0h1m1 0h1m1 0h2m2 0h1m1 0h1m2 0h1m1 0h3m1 0h1M1 6.5h1m5 0h1m1 0h2m2 0h2m1 0h3m2 0h1m5 0h1m5 0h1M1 7.5h7m1 0h1m1 0h1m1 0h1m1 0h1m1 0h1m1 0h1m1 0h1m1 0h1m1 0h1m1 0h7M9 8.5h2m1 0h1m3 0h1m2 0h1m2 0h2M1 9.5h1m1 0h5m2 0h1m2 0h4m1 0h1m4 0h1m1 0h1m1 0h5M2 10.5h2m2 0h1m1 0h1m4 0h1m3 0h1m1 0h3m2 0h1m2 0h2m1 0h2m1 0h1M2 11.5h2m2 0h4m1 0h1m1 0h1m2 0h2m1 0h1m3 0h1m1 0h2m2 0h1m1 0h1M3 12.5h2m4 0h5m1 0h2m3 0h1m1 0h2m2 0h1m2 0h3m1 0h1M3 13.5h1m3 0h1m2 0h1m1 0h2m5 0h1m2 0h1m2 0h2m1 0h3M4 14.5h1m1 0h1m3 0h1m1 0h7m2 0h1m2 0h1m1 0h2m2 0h1m1 0h2M1 15.5h2m1 0h1m1 0h3m3 0h1m1 0h1m4 0h2m2 0h1m1 0h3m1 0h1m2 0h1M1 16.5h2m2 0h2m2 0h2m2 0h3m1 0h1m2 0h4m2 0h6M1 17.5h1m1 0h1m2 0h7m1 0h2m2 0h1m1 0h1m2 0h1m2 0h1m1 0h3m2 0h1M2 18.5h1m1 0h1m1 0h1m2 0h1m1 0h1m2 0h1m2 0h5m1 0h2m1 0h3m1 0h4M1 19.5h2m2 0h3m1 0h2m1 0h3m1 0h1m1 0h1m2 0h1m1 0h1m4 0h2m1 0h1M2 20.5h1m1 0h1m1 0h1m1 0h2m3 0h1m2 0h4m1 0h1m2 0h1m2 0h5M3 21.5h1m3 0h1m1 0h5m1 0h3m2 0h7m1 0h3m1 0h1M1 22.5h2m3 0h1m1 0h1m2 0h1m2 0h3m1 0h2m2 0h1m1 0h1m1 0h2m2 0h2m1 0h1M1 23.5h1m2 0h7m1 0h1m1 0h1m1 0h3m2 0h2m2 0h1m2 0h3m1 0h1M1 24.5h1m1 0h1m2 0h1m1 0h3m2 0h1m2 0h1m2 0h1m2 0h1m1 0h1m3 0h1m1 0h2m1 0h1M1 25.5h1m3 0h1m1 0h2m1 0h1m1 0h1m1 0h6m3 0h7m2 0h2M9 26.5h1m1 0h2m2 0h1m1 0h1m1 0h2m1 0h1m2 0h1m3 0h1m1 0h1m1 0h1M1 27.5h7m3 0h1m1 0h1m1 0h3m1 0h1m1 0h2m1 0h2m1 0h1m1 0h1m1 0h2M1 28.5h1m5 0h1m1 0h2m1 0h3m1 0h1m4 0h5m3 0h4M1 29.5h1m1 0h3m1 0h1m1 0h2m1 0h1m1 0h1m1 0h1m2 0h1m2 0h1m2 0h6m1 0h1M1 30.5h1m1 0h3m1 0h1m1 0h4m1 0h5m2 0h1m1 0h1m1 0h2m2 0h2m2 0h1M1 31.5h1m1 0h3m1 0h1m1 0h4m2 0h1m2 0h1m1 0h1m1 0h1m4 0h2m1 0h1M1 32.5h1m5 0h1m3 0h3m1 0h1m1 0h1m1 0h5m5 0h3M1 33.5h7m1 0h1m2 0h1m1 0h2m2 0h1m1 0h1m2 0h1m1 0h4m1 0h1m1 0h1"/></svg>`;

const FAQ = [
  ["Le client doit-il installer une application ?", "Non. Il scanne le QR code du guichet et la page d'envoi s'ouvre dans son navigateur, sur Android comme sur iPhone. L'application Envoyeur (Android) est facultative."],
  ["Faut-il Internet dans la boutique ?", "Non. L'ordinateur crée le Wi-Fi de la boutique et reçoit les documents sans Internet. Si l'ordinateur a une connexion, les clients peuvent aussi envoyer par Internet, avec leurs données mobiles, d'où qu'ils soient."],
  ["Mon ordinateur ne crée pas de Wi-Fi : que faire ?", "Un petit routeur Wi-Fi branché à l'ordinateur par un câble fait la même chose. Le Bluetooth, le câble USB et la clé USB restent possibles en secours."],
  ["Quels documents peut-on envoyer ?", "PDF, Word, Excel, photos, et la plupart des fichiers. Le client choisit le nombre de copies, la couleur, le recto-verso et le format, et peut laisser un message vocal."],
  ["Le client sait-il quand c'est prêt ?", "Oui. Il reçoit un numéro, et sa page affiche « imprimé », puis « prêt », au fur et à mesure que le gérant avance."],
  ["Combien ça coûte ?", "Un mois d'essai gratuit. Ensuite, un petit abonnement mensuel : le prix est donné par l'équipe sur WhatsApp, selon la boutique."],
  ["Mes documents sont-ils gardés ?", "Non. Par le Wi-Fi de la boutique, ils vont directement à l'ordinateur. Par Internet, ils passent par le relais le temps de la récupération et y sont effacés, au plus tard 24 heures après l'envoi."],
  ["Sur quels ordinateurs le logiciel fonctionne-t-il ?", "Sur les ordinateurs Windows 10 et 11. Il démarre avec l'ordinateur et travaille en arrière-plan ; sa fenêtre ne s'ouvre que quand le gérant la demande."],
];

export function pageFaq() {
  const jsonld = `<script type="application/ld+json">${JSON.stringify({
    "@context": "https://schema.org",
    "@type": "FAQPage",
    mainEntity: FAQ.map(([q, r]) => ({ "@type": "Question", name: q, acceptedAnswer: { "@type": "Answer", text: r } })),
  })}</script>`;
  return gabarit({
    titre: "Questions fréquentes — Gestion Photocopie",
    description: "Faut-il Internet ? Le client doit-il installer une application ? Combien ça coûte ? Les réponses sur le logiciel Gestion Photocopie pour boutiques de photocopie au Bénin.",
    chemin: "/faq",
    jsonld,
    corps: `<h1>Questions fréquentes</h1>${FAQ.map(([q, r]) => `<div class="carte" style="margin:12px 0"><h3>${q}</h3><p>${r}</p></div>`).join("")}
<p class="petit">Une autre question ? <a href="https://wa.me/${WHATSAPP}">WhatsApp ${TELEPHONE_LISIBLE}</a>.</p>`,
  });
}

export function pageAccueilEn() {
  return gabarit({
    lang: "en",
    titre: "Gestion Photocopie — customers send their documents with a QR code | Print shop software",
    description: "Customers scan a QR code and send their documents from their phone; the shop receives them on its computer, ready to print — with or without Internet. Software for print shops and cyber cafés.",
    chemin: "/en",
    alternes: { fr: "/", en: "/en" },
    corps: `
<section class="heros">
  <h1>Your customers send their documents from their phone. You print.</h1>
  <p class="chapeau">No more USB sticks, cables or "send it to me on WhatsApp". The customer scans the QR code at the counter, picks the documents, and they arrive on the shop's computer, ready to print.</p>
  <div class="boutons"><a class="btn" href="/telecharger">Download<small>Windows · one month free trial</small></a></div>
</section>
<h2>With or without Internet</h2>
<div class="grille">
  <div class="carte"><h3>📶 Offline</h3><p>The computer (or a small router) creates the shop's Wi-Fi. Customers join it and send. Nothing leaves the shop.</p></div>
  <div class="carte"><h3>🌍 Online</h3><p>Customers send with their own mobile data, from anywhere. The computer collects the order as soon as it has a connection.</p></div>
  <div class="carte"><h3>🔵 Bluetooth &amp; USB</h3><p>Always available as a fallback: everything lands in the same queue.</p></div>
</div>
<h2>What the shop gets</h2>
<div class="grille">
  <div class="carte"><p><span class="coche">✓</span> Orders arrive with copies, colour and duplex settings.</p></div>
  <div class="carte"><p><span class="coche">✓</span> Each customer gets a number and sees "printed", then "ready" on their phone.</p></div>
  <div class="carte"><p><span class="coche">✓</span> Prices, cash and daily reports kept automatically.</p></div>
</div>
<p class="petit">The software is in French, made for print shops in Benin and French-speaking Africa. Questions: <a href="https://wa.me/${WHATSAPP}">WhatsApp +229 ${TELEPHONE_LISIBLE}</a>.</p>`,
  });
}

const TEXTES_KIT = [
  ["Statut WhatsApp", `🖨️ Fini la clé USB et le câble au guichet !
Avec Gestion Photocopie, vos clients scannent un QR code et envoient leurs documents depuis leur téléphone. Vous les recevez sur l'ordinateur, prêts à imprimer.
✅ Avec ou sans Internet
✅ Un numéro pour chaque client
✅ Un mois d'essai gratuit
👉 ${RACINE}`],
  ["Publication Facebook", `Gérants de photocopie et de cyber : vos clients font la queue avec leur clé USB ou vous envoient tout sur WhatsApp ?
Gestion Photocopie change ça : le client scanne le QR code du guichet, choisit ses documents (PDF, Word, photos) et les envoie. Ils arrivent sur votre ordinateur avec le nombre de copies, la couleur et le recto-verso. Le client reçoit un numéro et voit sur son téléphone quand c'est prêt.
Ça marche même sans Internet, par le Wi-Fi de la boutique. Et si vous avez Internet, vos clients peuvent envoyer de chez eux.
Un mois d'essai gratuit : ${RACINE}
Questions : WhatsApp ${TELEPHONE_LISIBLE}`],
  ["Message direct à un gérant", `Bonjour, je vous présente Gestion Photocopie, un logiciel fait au Bénin pour les boutiques de photocopie : vos clients vous envoient leurs documents en scannant un QR code, sans clé USB ni WhatsApp, et vous les recevez directement sur l'ordinateur. Un mois d'essai gratuit. Je peux passer vous le montrer en 5 minutes ? Tout est ici : ${RACINE}`],
  ["SMS court", `Photocopie : vos clients envoient leurs docs par QR code, vous imprimez. Sans Internet. 1 mois gratuit. ${RACINE}`],
  ["WhatsApp status (English)", `🖨️ Print shops: customers scan a QR code and send their documents from their phone — they arrive on your computer, ready to print. Works offline. One month free: ${RACINE}/en`],
];

export function pageKit() {
  const echap = (t) => t.replace(/&/g, "&amp;").replace(/</g, "&lt;");
  return gabarit({
    titre: "Kit de promotion — Gestion Photocopie",
    description: "Textes prêts à publier, image et affiche pour faire connaître Gestion Photocopie.",
    chemin: "/kit",
    indexer: false,
    corps: `<h1>Kit de promotion</h1>
<p class="chapeau">Des textes prêts à copier, l'image à partager et l'affiche à imprimer. Touchez « Copier », puis collez dans WhatsApp, Facebook ou un SMS.</p>
<div class="boutons"><a class="btn" href="/affiche">Affiche à imprimer</a><a class="btn second" href="/apercu.png" download>Image à partager</a></div>
${TEXTES_KIT.map(([titre, texte], i) => `<div class="carte" style="margin:14px 0"><h3>${titre}</h3><p id="t${i}" style="white-space:pre-line">${echap(texte)}</p><button class="btn second" style="border:0;cursor:pointer" onclick="navigator.clipboard.writeText(document.getElementById('t${i}').innerText).then(()=>{this.textContent='Copié ✓'})">Copier</button></div>`).join("")}`,
  });
}

export function pageAffiche() {
  return `<!doctype html><html lang="fr"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><meta name="robots" content="noindex">
<title>Affiche — Gestion Photocopie</title>
<style>
@page{size:A4;margin:12mm}
body{font-family:system-ui,"Segoe UI",Roboto,sans-serif;color:#16213a;margin:0;background:#fff}
.a4{max-width:186mm;margin:0 auto;padding:10mm 0;text-align:center}
h1{font-size:34pt;line-height:1.1;margin:0 0 6mm;color:#0b5cad}
.s{font-size:15pt;margin:0 0 8mm}
.qr{width:70mm;height:70mm;margin:0 auto 6mm}.qr svg{width:100%;height:100%}
.e{display:flex;gap:6mm;justify-content:center;margin:6mm 0;text-align:left}
.e div{flex:1;border:2px solid #0b5cad;border-radius:5mm;padding:4mm;font-size:13pt}
.e b{display:block;font-size:20pt;color:#0b5cad}
.p{font-size:13pt;margin-top:8mm}
.imp{margin:6mm;padding:10px 16px;font-size:14pt}
@media print{.imp{display:none}}
</style></head><body><div class="a4">
<button class="imp" onclick="print()">🖨️ Imprimer cette affiche</button>
<h1>Gérants de photocopie :<br>fini la clé USB au guichet</h1>
<p class="s">Vos clients envoient leurs documents depuis leur téléphone,<br>vous les recevez sur l'ordinateur, prêts à imprimer.</p>
<div class="qr">${QR_SITE}</div>
<div class="e"><div><b>1</b>Le client scanne le QR code du guichet</div><div><b>2</b>Il choisit ses documents</div><div><b>3</b>Vous imprimez, il reçoit son numéro</div></div>
<p class="p"><strong>Avec ou sans Internet · Un mois d'essai gratuit</strong><br>${RACINE.replace("https://", "")}<br>WhatsApp ${TELEPHONE_LISIBLE}</p>
</div></body></html>`;
}
