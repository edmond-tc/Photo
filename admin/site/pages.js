// Le site public : présentation, téléchargement, mode d'emploi,
// confidentialité. Léger (aucune ressource extérieure) : il doit s'ouvrir
// vite sur un téléphone, avec une petite connexion.

export const ADRESSE_ADMIN = "https://photocopie-admin.atinzed2.workers.dev";
const WHATSAPP = "2290151226741";
const TELEPHONE_LISIBLE = "01 51 22 67 41";

const STYLE = `
:root{--fond:#f6f7fb;--carte:#fff;--encre:#16213a;--doux:#55617a;--accent:#0b5cad;--accent-doux:#e3eefb;--vert:#137a3d;--trait:#dfe3ec}
@media (prefers-color-scheme:dark){:root{--fond:#0f1420;--carte:#171e2e;--encre:#eef2fa;--doux:#a9b4c9;--accent:#5ea2ec;--accent-doux:#1b2a44;--vert:#48c27a;--trait:#2a3348}}
*{box-sizing:border-box}html{-webkit-text-size-adjust:100%}
body{margin:0;background:var(--fond);color:var(--encre);font:16px/1.55 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif}
a{color:var(--accent)}
.page{max-width:980px;margin:0 auto;padding:0 16px}
header.haut{display:flex;align-items:center;justify-content:space-between;gap:12px;padding:14px 0}
.marque{font-weight:800;font-size:1.1rem;text-decoration:none;color:var(--encre)}
nav a{margin-left:14px;font-size:.95rem;text-decoration:none}
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
`;

function gabarit({ titre, description, chemin, corps, jsonld = "" }) {
  const url = `https://photocopie-benin.atinzed2.workers.dev${chemin}`;
  return `<!doctype html>
<html lang="fr">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${titre}</title>
<meta name="description" content="${description}">
<link rel="canonical" href="${url}">
<meta property="og:type" content="website">
<meta property="og:title" content="${titre}">
<meta property="og:description" content="${description}">
<meta property="og:url" content="${url}">
<meta property="og:locale" content="fr_FR">
<meta name="theme-color" content="#0b5cad">
<link rel="icon" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 64 64'%3E%3Crect width='64' height='64' rx='14' fill='%230b5cad'/%3E%3Cpath d='M18 24h28v22H18z' fill='%23fff'/%3E%3Cpath d='M22 14h20v12H22z' fill='%23cfe2fb'/%3E%3C/svg%3E">
${jsonld}
<style>${STYLE}</style>
</head>
<body>
<div class="page">
<header class="haut">
  <a class="marque" href="/">🖨️ Gestion Photocopie</a>
  <nav><a href="/comment-ca-marche">Comment ça marche</a><a href="/telecharger">Télécharger</a></nav>
</header>
${corps}
<footer>
  <p><strong>Gestion Photocopie</strong> — le logiciel des boutiques de photocopie et cybercafés du Bénin.</p>
  <p><a href="/comment-ca-marche">Comment ça marche</a> · <a href="/telecharger">Télécharger</a> · <a href="/confidentialite">Confidentialité</a> · <a href="https://wa.me/${WHATSAPP}">WhatsApp ${TELEPHONE_LISIBLE}</a></p>
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
    jsonld,
    corps: `
<section class="heros">
  <h1>Vos clients envoient leurs documents depuis leur téléphone. Vous imprimez.</h1>
  <p class="chapeau">Fini la clé USB, le câble et les « envoie-moi sur WhatsApp ». Le client scanne le QR code du guichet, choisit ses documents, et ils arrivent sur l'ordinateur de la boutique, prêts à imprimer.</p>
  <div class="boutons">
    <a class="btn" href="/telecharger">Télécharger le logiciel<small>Windows · un mois d'essai gratuit</small></a>
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
  const pages = ["/", "/comment-ca-marche", "/telecharger", "/confidentialite"];
  return `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${pages
    .map((p) => `  <url><loc>https://photocopie-benin.atinzed2.workers.dev${p}</loc><lastmod>2026-10-12</lastmod></url>`)
    .join("\n")}\n</urlset>\n`;
}
