const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const listeEl = document.querySelector("#liste-fichiers");
const etatVideEl = document.querySelector("#etat-vide");
const tplLigne = document.querySelector("#tpl-ligne-fichier");

const LIBELLES_KIND = {
  imprimable: "Prêt à imprimer",
  editable: "À ouvrir/éditer",
  installateur: "Mise à jour du logiciel",
  inconnu: "Format non reconnu",
};

const FINITIONS = [
  ["agrafage", "Agrafage"],
  ["reliure_spirale", "Reliure spirale"],
  ["reliure_dos_carre", "Reliure dos carré collé"],
  ["plastification", "Plastification"],
  ["decoupe", "Découpe / massicotage"],
  ["perforation", "Perforation"],
  ["pliage", "Pliage"],
];

let idOptionsEnCours = null;
let idEncaissementEnCours = null;

// ───────────────────────────── Utilitaires ─────────────────────────────

// Pour toute valeur interpolée dans un gabarit HTML (innerHTML) plutôt que
// posée via .textContent/.value — sinon un guillemet ou un chevron dans une
// valeur saisie (nom de boutique, SSID Wi-Fi, raison personnalisée...) peut
// casser un attribut ou injecter du HTML dans la page des réglages.
function echapperHtml(valeur) {
  return String(valeur ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function formatHeure(isoString) {
  try {
    return new Date(isoString).toLocaleTimeString("fr-FR", {
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return "";
  }
}

function formatFcfa(montant) {
  return `${Number(montant ?? 0).toLocaleString("fr-FR")} FCFA`;
}

function jouerTonalite(ctx, { freq, debut, duree, gainMax }) {
  const osc = ctx.createOscillator();
  const gain = ctx.createGain();
  osc.type = "sine";
  osc.frequency.value = freq;
  gain.gain.setValueAtTime(0.0001, ctx.currentTime + debut);
  gain.gain.exponentialRampToValueAtTime(gainMax, ctx.currentTime + debut + 0.02);
  gain.gain.exponentialRampToValueAtTime(0.0001, ctx.currentTime + debut + duree);
  osc.connect(gain).connect(ctx.destination);
  osc.start(ctx.currentTime + debut);
  osc.stop(ctx.currentTime + debut + duree + 0.02);
}

// Carillon "ding-dong" : signale l'arrivée d'un nouveau fichier (USB, dossier
// surveillé ou Wi-Fi — les trois passent par le même événement côté Rust).
function jouerNotification() {
  try {
    const ctx = new (window.AudioContext || window.webkitAudioContext)();
    jouerTonalite(ctx, { freq: 988, debut: 0, duree: 0.35, gainMax: 0.22 });
    jouerTonalite(ctx, { freq: 740, debut: 0.28, duree: 0.5, gainMax: 0.2 });
  } catch {
    // Pas grave si le son ne peut pas jouer (ex: pas d'interaction utilisateur encore).
  }
}

// Petit "tic" — confirme que l'impression a bien été envoyée.
function jouerSonImpression() {
  try {
    const ctx = new (window.AudioContext || window.webkitAudioContext)();
    jouerTonalite(ctx, { freq: 659, debut: 0, duree: 0.13, gainMax: 0.14 });
  } catch {
    // Pas grave si le son ne peut pas jouer.
  }
}

// Deux notes montantes — confirme qu'un encaissement vient d'être validé.
function jouerSonEncaissement() {
  try {
    const ctx = new (window.AudioContext || window.webkitAudioContext)();
    jouerTonalite(ctx, { freq: 523, debut: 0, duree: 0.14, gainMax: 0.16 });
    jouerTonalite(ctx, { freq: 659, debut: 0.13, duree: 0.22, gainMax: 0.16 });
  } catch {
    // Pas grave si le son ne peut pas jouer.
  }
}

// ── Confirmation discrète qui disparaît seule, pour ne pas interrompre le
// gérant à chaque action routinière (contrairement à alert(), qui bloque
// tant qu'on ne clique pas "OK"). type: "succes" (vert) ou "attention" (jaune).
let toastMinuteur = null;
function toast(message, type = "succes", dureeMs = 3200) {
  const el = document.querySelector("#toast");
  el.textContent = message;
  el.classList.toggle("toast-attention", type === "attention");
  el.classList.add("visible");
  clearTimeout(toastMinuteur);
  toastMinuteur = setTimeout(() => el.classList.remove("visible"), dureeMs);
}

function bouton(label, classe, onClick) {
  const b = document.createElement("button");
  b.type = "button";
  b.textContent = label;
  b.className = classe;
  b.addEventListener("click", onClick);
  return b;
}

function ouvrirModal(id) {
  // Une seule fenêtre à la fois — sinon elles s'empilent et deviennent
  // illisibles (bug remonté lors des tests).
  document.querySelectorAll(".modal").forEach((m) => {
    if (m.id !== id) m.hidden = true;
  });
  document.querySelector("#panneau-menu").hidden = true;
  document.querySelector(`#${id}`).hidden = false;
}
function fermerModal(id) {
  document.querySelector(`#${id}`).hidden = true;
}

// ───────────────────────────── File d'attente ─────────────────────────────

// Résultats de confirmation d'impression déjà reçus, par identifiant de
// commande — le spouleur Windows peut mettre jusqu'à 45 secondes à
// répondre (voir impression.rs), largement après le premier affichage de
// la ligne. Permet de retrouver l'info si la ligne est reconstruite entre
// temps (nouveau fichier arrivé, changement de section...).
const confirmationsImpression = new Map();

function texteStatutImpression(payload) {
  if (payload.erreur) return `⚠️ Impression non confirmée : ${payload.erreur}`;
  if (payload.confirmee) {
    const pages = payload.pages_imprimees ?? payload.pages;
    return pages ? `✓ Impression confirmée (${pages} page${pages > 1 ? "s" : ""})` : "✓ Impression confirmée";
  }
  return null;
}

function appliquerStatutImpression(id, payload) {
  const texte = texteStatutImpression(payload);
  if (!texte) return;
  document.querySelectorAll(`[data-id="${id}"] .statut-impression`).forEach((el) => {
    el.textContent = texte;
    el.hidden = false;
    el.style.color = payload.erreur ? "var(--rouge-alerte)" : "var(--vert-succes)";
  });
}

function creerLigne(item) {
  const noeud = tplLigne.content.cloneNode(true);
  const li = noeud.querySelector(".ligne-fichier");
  li.dataset.id = item.id;

  noeud.querySelector(".nom-fichier").textContent = item.original_name;
  const meta = [
    item.client_name ? `Client : ${item.client_name}` : null,
    formatHeure(item.received_at),
    LIBELLES_KIND[item.kind] ?? item.kind,
    item.couleur ? "Couleur" : null,
    item.format_papier && item.format_papier !== "A4" ? item.format_papier : null,
    item.copies > 1 ? `${item.copies} copies` : null,
    item.plage_pages ? `pages ${item.plage_pages}` : null,
    item.finitions?.length ? item.finitions.length + " finition(s)" : null,
    item.taille_octets > 20 * 1024 * 1024
      ? `${(item.taille_octets / 1024 / 1024).toFixed(1)} Mo — fichier volumineux`
      : null,
  ]
    .filter(Boolean)
    .join(" · ");
  noeud.querySelector(".meta-fichier").textContent = meta;

  if (item.protege || item.format_detecte) {
    const alertes = document.createElement("span");
    alertes.className = "meta-fichier";
    alertes.style.color = "var(--rouge-alerte)";
    alertes.textContent = [
      item.protege ? "🔒 PDF probablement protégé par mot de passe — le demander au client" : null,
      item.format_detecte ? `Format détecté : ${item.format_detecte} (pas A4) — vérifier l'ajustement à l'impression` : null,
    ]
      .filter(Boolean)
      .join(" · ");
    noeud.querySelector(".info-fichier").appendChild(alertes);
  }

  const actions = noeud.querySelector(".actions-fichier");

  // Le bouton Imprimer/Ouvrir passe toujours en premier et va droit au but
  // (boîte de dialogue Windows native) — les détails de facturation sont
  // secondaires et n'empêchent jamais d'imprimer ou d'éditer directement.
  if (item.kind === "imprimable") {
    actions.appendChild(bouton("Aperçu", "btn-secondaire", () => ouvrirApercu(item)));
    actions.appendChild(bouton("Imprimer", "btn-primaire", () => imprimer(item.id)));
    actions.appendChild(bouton("Encaisser", "btn-secondaire", () => ouvrirEncaissement(item)));
    actions.appendChild(bouton("Détails", "btn-discret", () => ouvrirOptions(item)));
  } else if (item.kind === "editable") {
    actions.appendChild(bouton("Ouvrir/Éditer", "btn-primaire", () => ouvrir(item.id)));
    actions.appendChild(bouton("Encaisser", "btn-secondaire", () => ouvrirEncaissement(item)));
    actions.appendChild(bouton("Détails", "btn-discret", () => ouvrirOptions(item)));
  } else if (item.kind === "installateur") {
    // Un exécutable ne peut arriver ici que par clé USB branchée sur le PC
    // (voir watcher.rs). Une clé USB peut malgré tout être infectée : on ne
    // lance jamais un exécutable sans que le gérant confirme qu'il vient
    // bien de la personne qui lui fournit le logiciel.
    actions.appendChild(
      bouton("Installer la mise à jour", "btn-primaire", () => {
        const ok = confirm(
          `Installer "${item.original_name}" ?\n\n` +
            "N'installez ce fichier QUE s'il vient d'une clé USB donnée en main propre " +
            "par la personne qui vous fournit ce logiciel (ou transférée depuis SON lien " +
            "WhatsApp à elle).\n\n" +
            "Un fichier trouvé sur la clé USB d'un client peut contenir un virus.\n\n" +
            "L'installateur va s'ouvrir. Si Gestion Photocopie ne se ferme pas " +
            "automatiquement, fermez-la vous-même (croix en haut), puis suivez " +
            "les instructions à l'écran jusqu'au bout."
        );
        if (ok) ouvrir(item.id);
      })
    );
    actions.appendChild(bouton("Ignorer", "btn-discret", () => ignorer(item.id)));
  } else {
    const estExecutable = /\.(exe|msi)$/i.test(item.original_name || "");
    const avert = document.createElement("span");
    avert.className = "avertissement";
    avert.textContent = estExecutable
      ? "⚠️ Fichier exécutable reçu d'un client — ne pas l'ouvrir, ce n'est pas une mise à jour officielle"
      : "Format non supporté — redemander un format standard au client";
    actions.appendChild(avert);
    actions.appendChild(bouton("Ignorer", "btn-discret", () => ignorer(item.id)));
  }

  if (item.kind === "imprimable") {
    // Rempli plus tard, en arrière-plan, une fois que le spouleur Windows
    // confirme (ou pas) que l'impression a vraiment eu lieu — voir
    // impression.rs et l'écouteur "impression-confirmee" plus bas.
    const statutImpression = document.createElement("span");
    statutImpression.className = "meta-fichier statut-impression";
    statutImpression.hidden = true;
    noeud.querySelector(".info-fichier").appendChild(statutImpression);
    if (confirmationsImpression.has(item.id)) {
      // Appliqué après l'insertion dans le document (voir ajouterFichier) :
      // querySelector sur `noeud`, un DocumentFragment, ne verrait pas ses
      // propres enfants une fois déplacés dans le DOM.
      queueMicrotask(() => appliquerStatutImpression(item.id, confirmationsImpression.get(item.id)));
    }

    const img = noeud.querySelector(".vignette-fichier");
    invoke("get_thumbnail", { id: item.id }).then((dataUri) => {
      if (dataUri) {
        img.src = dataUri;
        img.hidden = false;
      }
    });
  }

  return noeud;
}

function ajouterFichier(item, enTete = false) {
  etatVideEl.hidden = true;
  const noeud = creerLigne(item);
  if (enTete) {
    listeEl.prepend(noeud);
  } else {
    listeEl.appendChild(noeud);
  }
}

function retirerFichier(id) {
  const li = listeEl.querySelector(`.ligne-fichier[data-id="${id}"]`);
  if (li) li.remove();
  if (!listeEl.children.length) etatVideEl.hidden = false;
  mettreAJourBadgeOublies();
}

async function chargerFile() {
  const items = await invoke("get_queue");
  listeEl.innerHTML = "";
  if (!items.length) {
    etatVideEl.hidden = false;
    mettreAJourBadgeOublies();
    return;
  }
  etatVideEl.hidden = true;
  for (const item of items) ajouterFichier(item);
  mettreAJourBadgeOublies();
}

async function imprimer(id) {
  try {
    await invoke("print_file", { id });
    jouerSonImpression();
  } catch (e) {
    alert(`⚠️ L'impression n'a pas pu démarrer. Vérifiez que l'imprimante est allumée et connectée, puis réessayez.\n\nDétail : ${e}`);
  }
}

let idApercuEnCours = null;

async function ouvrirApercu(item) {
  idApercuEnCours = item.id;
  document.querySelector("#apercu-titre").textContent = item.original_name;
  const conteneur = document.querySelector("#apercu-contenu");
  conteneur.innerHTML = "Chargement de l'aperçu…";
  ouvrirModal("modal-apercu");
  try {
    const dataUri = await invoke("get_apercu", { id: item.id });
    conteneur.innerHTML = "";
    if (dataUri.startsWith("data:application/pdf")) {
      const iframe = document.createElement("iframe");
      iframe.src = dataUri;
      conteneur.appendChild(iframe);
    } else {
      const img = document.createElement("img");
      img.src = dataUri;
      img.alt = item.original_name;
      conteneur.appendChild(img);
    }
  } catch (e) {
    conteneur.innerHTML = `<p class="avertissement">${echapperHtml(e)}</p>`;
  }
}

document.querySelector("#btn-imprimer-depuis-apercu").addEventListener("click", () => {
  if (idApercuEnCours != null) imprimer(idApercuEnCours);
});

async function ouvrir(id) {
  try {
    await invoke("open_file", { id });
  } catch (e) {
    alert(`⚠️ Ce fichier n'a pas pu s'ouvrir — il est peut-être corrompu ou dans un format non pris en charge.\n\nDétail : ${e}`);
  }
}

let idIgnorerEnCours = null;

function ignorer(id) {
  idIgnorerEnCours = id;
  document.querySelector("#raison-ignore-select").value = "Format non supporté";
  document.querySelector("#raison-ignore-texte").value = "";
  ouvrirModal("modal-raison-ignore");
}

document.querySelector("#form-raison-ignore").addEventListener("submit", async (e) => {
  e.preventDefault();
  const select = document.querySelector("#raison-ignore-select").value;
  const texte = document.querySelector("#raison-ignore-texte").value.trim();
  const raison = select === "Autre" ? texte : select;
  if (!raison) {
    toast("Merci de préciser une raison avant de continuer.", "attention");
    return;
  }
  try {
    await invoke("ignorer_fichier", { id: idIgnorerEnCours, raison });
    retirerFichier(idIgnorerEnCours);
    fermerModal("modal-raison-ignore");
  } catch (e) {
    alert(`⚠️ Ce fichier n'a pas pu être mis à jour. Réessayez dans un instant.\n\nDétail : ${e}`);
  }
});

// ───────────── Détails de facturation (pas des options d'impression) ─────────────
// L'impression elle-même passe toujours directement par la boîte de dialogue
// Windows native (bouton Imprimer) — elle gère déjà copies/couleur/recto-verso.
// Ceci ne sert qu'à noter ce qu'il faut facturer : copies, N&B/couleur, format
// papier et finitions.

function majTotalFeuilles() {
  const pages = Number(document.querySelector("#opt-pages").value) || 1;
  const copies = Number(document.querySelector("#opt-nb-copies").value) || 1;
  const total = pages * copies;
  document.querySelector("#opt-total-feuilles").textContent =
    `= ${total} feuille${total > 1 ? "s" : ""} au total`;
}
document.querySelector("#opt-pages").addEventListener("input", majTotalFeuilles);
document.querySelector("#opt-nb-copies").addEventListener("input", majTotalFeuilles);

function ouvrirOptions(item) {
  idOptionsEnCours = item.id;
  // On ne connaît que le total de feuilles déjà enregistré (pas le détail
  // pages × copies) : on repart d'une copie pour ce total, modifiable.
  document.querySelector("#opt-pages").value = 1;
  document.querySelector("#opt-nb-copies").value = item.copies ?? 1;
  majTotalFeuilles();
  document.querySelector("#opt-couleur").checked = !!item.couleur;
  document.querySelector("#opt-format").value = item.format_papier ?? "A4";

  const grille = document.querySelector("#opt-finitions");
  grille.innerHTML = "";
  const actives = new Set(item.finitions ?? []);
  for (const [cle, libelle] of FINITIONS) {
    const label = document.createElement("label");
    label.className = "champ-inline";
    const input = document.createElement("input");
    input.type = "checkbox";
    input.value = cle;
    input.checked = actives.has(cle);
    label.appendChild(input);
    label.append(" " + libelle);
    grille.appendChild(label);
  }

  ouvrirModal("modal-options");
}

document.querySelector("#form-options").addEventListener("submit", async (e) => {
  e.preventDefault();
  const finitions = [...document.querySelectorAll("#opt-finitions input:checked")].map(
    (i) => i.value
  );
  const pages = Number(document.querySelector("#opt-pages").value) || 1;
  const nbCopies = Number(document.querySelector("#opt-nb-copies").value) || 1;
  try {
    await invoke("set_print_options", {
      id: idOptionsEnCours,
      copies: pages * nbCopies,
      couleur: document.querySelector("#opt-couleur").checked,
      formatPapier: document.querySelector("#opt-format").value,
      finitions,
    });
    fermerModal("modal-options");
    await chargerFile();
    toast("✓ Détails enregistrés");
  } catch (err) {
    alert(`⚠️ Ces détails n'ont pas pu être enregistrés. Réessayez dans un instant.\n\nDétail : ${err}`);
  }
});

// ───────────────────────────── Encaissement ─────────────────────────────

async function remplirEmployes(selectEl) {
  const employes = await invoke("list_employes");
  selectEl.innerHTML = '<option value="">—</option>';
  for (const emp of employes.filter((e) => e.actif)) {
    const opt = document.createElement("option");
    opt.value = emp.nom;
    opt.textContent = emp.nom;
    selectEl.appendChild(opt);
  }
}

let montantCalculeEnCours = 0;

async function ouvrirEncaissement(item) {
  idEncaissementEnCours = item.id;
  const indiceFidelite = document.querySelector("#enc-fidelite");
  indiceFidelite.hidden = true;
  document.querySelector("#enc-raison-champ").hidden = true;
  document.querySelector("#enc-raison-ecart").value = "";
  try {
    const resultat = await invoke("calculer_prix", { id: item.id });
    montantCalculeEnCours = resultat.total;
    document.querySelector("#enc-montant-calcule").textContent = formatFcfa(resultat.total);
    document.querySelector("#enc-montant").value = resultat.total;
    if (resultat.remise_fidelite_appliquee) {
      indiceFidelite.hidden = false;
    }
  } catch {
    montantCalculeEnCours = 0;
    document.querySelector("#enc-montant-calcule").textContent = formatFcfa(0);
    document.querySelector("#enc-montant").value = 0;
  }
  await remplirEmployes(document.querySelector("#enc-employe"));
  ouvrirModal("modal-encaissement");
}

document.querySelector("#enc-montant").addEventListener("input", (e) => {
  const diffère = Number(e.target.value) !== montantCalculeEnCours;
  document.querySelector("#enc-raison-champ").hidden = !diffère;
});

document.querySelector("#form-encaissement").addEventListener("submit", async (e) => {
  e.preventDefault();
  const moyen = document.querySelector("#enc-moyen").value;
  const montant = Number(document.querySelector("#enc-montant").value) || 0;
  const raisonEcart = document.querySelector("#enc-raison-ecart").value.trim() || null;
  if (montant !== montantCalculeEnCours && !raisonEcart) {
    toast("Le montant diffère du prix calculé — précisez la raison pour continuer.", "attention");
    return;
  }
  try {
    const resultat = await invoke("finaliser_commande", {
      id: idEncaissementEnCours,
      montantCalcule: montantCalculeEnCours,
      montant,
      raisonEcart,
      moyenPaiement: moyen,
      statut: moyen === "credit" ? "impaye" : "paye",
      employe: document.querySelector("#enc-employe").value || null,
    });
    fermerModal("modal-encaissement");
    retirerFichier(idEncaissementEnCours);
    jouerSonEncaissement();
    if (confirm("Encaissement enregistré. Imprimer le reçu ?")) {
      try {
        await invoke("imprimer_recu", { transactionId: resultat.transaction_id });
      } catch (err) {
        alert(`⚠️ Le reçu n'a pas pu s'imprimer. Vérifiez l'imprimante et réessayez depuis l'historique.\n\nDétail : ${err}`);
      }
    }
    if (resultat.alerte_entretien_imprimante) {
      alert(
        "🔧 Petit rappel : l'imprimante approche du seuil d'entretien préventif " +
          "(nettoyage, pièces d'usure). Rien d'urgent, mais pensez à la faire vérifier bientôt. " +
          "Vous pourrez réinitialiser ce compteur dans Rapports une fois l'entretien fait."
      );
    }
  } catch (err) {
    alert(`⚠️ L'encaissement n'a pas pu être enregistré. Réessayez dans un instant — rien n'a été perdu.\n\nDétail : ${err}`);
  }
});

// ───────────────────────────── QR / réception client ─────────────────────────────

async function afficherQr() {
  const conteneur = document.querySelector("#qr-conteneur");
  const urlEl = document.querySelector("#qr-url");
  conteneur.innerHTML = "Chargement…";
  urlEl.textContent = "";
  try {
    const info = await invoke("get_server_info");
    conteneur.innerHTML = "";
    const img = document.createElement("img");
    img.src = info.qr_data_uri;
    img.alt = "QR code de réception";
    img.width = 220;
    img.height = 220;
    conteneur.appendChild(img);
    if (info.wifi_configure) {
      // L'ouverture automatique de la page après connexion au Wi-Fi dépend
      // de la détection "portail captif" du téléphone du client — fiable
      // sur beaucoup d'appareils, mais pas garantie sur tous. Sans cette
      // adresse affichée en clair, le gérant n'aurait aucun moyen de guider
      // un client bloqué après la connexion Wi-Fi.
      urlEl.innerHTML =
        `Le client scanne, rejoint le Wi-Fi automatiquement, et la page d'envoi s'ouvre — sur la plupart des téléphones. ` +
        `Si rien ne s'ouvre après quelques secondes : dites-lui d'ouvrir son navigateur et de taper ` +
        `<strong>${echapperHtml(info.url)}</strong>.`;
    } else {
      urlEl.innerHTML =
        `Wi-Fi non configuré — ce QR n'ouvre que la page (${echapperHtml(info.url)}), le client doit déjà être connecté. ` +
        `Configurez le nom et le mot de passe du Wi-Fi dans Réglages pour un QR unique tout-en-un.`;
    }
  } catch (e) {
    conteneur.innerHTML = `<p class="avertissement">${echapperHtml(e)}</p>`;
  }
  ouvrirModal("modal-qr");
}

document.querySelector("#btn-parametres-partage").addEventListener("click", async () => {
  try {
    await invoke("ouvrir_parametres_partage_connexion");
  } catch (e) {
    alert(`⚠️ Impossible d'ouvrir les paramètres Windows automatiquement. Ouvrez-les vous-même : Paramètres → Réseau et Internet → Partage de connexion.\n\nDétail : ${e}`);
  }
});

document.querySelector("#btn-imprimer-qr").addEventListener("click", () => {
  window.print();
});

// ───────────────────────────── Panneau latéral ─────────────────────────────

function ouvrirPanneauMenu() {
  document.querySelectorAll(".modal").forEach((m) => (m.hidden = true));
  document.querySelector("#panneau-menu").hidden = false;
  document.querySelector("#panneau-nav").hidden = false;
  document.querySelector("#panneau-contenu").hidden = true;
  document.querySelector("#panneau-titre").textContent = "Menu";
}
function fermerPanneauMenu() {
  document.querySelector("#panneau-menu").hidden = true;
}

const TITRES_SECTION = {
  commandes: "Commandes en cours",
  historique: "Historique",
  recherche: "Documents reçus / Recherche client",
  rapports: "Rapports",
  reglages: "Réglages",
};

async function ouvrirSection(section) {
  document.querySelector("#panneau-nav").hidden = true;
  document.querySelector("#panneau-contenu").hidden = false;
  document.querySelector("#panneau-titre").textContent = TITRES_SECTION[section] ?? "Menu";
  const corps = document.querySelector("#section-corps");
  corps.innerHTML = "Chargement…";

  const rendus = {
    commandes: rendreCommandesEnCours,
    historique: rendreHistorique,
    recherche: rendreRecherche,
    rapports: rendreRapports,
    reglages: rendreReglages,
  };
  await rendus[section]?.(corps);
}

function ligneListe(texte, sousTexte) {
  const div = document.createElement("div");
  div.className = "ligne-liste";
  const t = document.createElement("div");
  t.textContent = texte;
  div.appendChild(t);
  if (sousTexte) {
    const s = document.createElement("div");
    s.className = "meta-fichier";
    s.textContent = sousTexte;
    div.appendChild(s);
  }
  return div;
}

async function rendreCommandesEnCours(corps) {
  const items = await invoke("get_queue");
  corps.innerHTML = "";
  if (!items.length) {
    corps.appendChild(ligneListe("Aucune commande en cours."));
    return;
  }
  for (const item of items) {
    corps.appendChild(
      ligneListe(
        item.original_name,
        [item.client_name, formatHeure(item.received_at), LIBELLES_KIND[item.kind]]
          .filter(Boolean)
          .join(" · ")
      )
    );
  }
}

async function rendreHistorique(corps) {
  const items = await invoke("get_historique", { limite: 100 });
  corps.innerHTML = "";
  if (!items.length) {
    corps.appendChild(ligneListe("Aucun historique pour l'instant."));
    return;
  }
  for (const item of items) {
    const ligne = document.createElement("div");
    ligne.className = "ligne-liste";
    ligne.dataset.id = item.id;
    const info = document.createElement("div");
    info.textContent = item.original_name;
    const sousTexte = document.createElement("div");
    sousTexte.className = "meta-fichier";
    sousTexte.textContent = [
      item.client_name,
      formatHeure(item.received_at),
      item.prix ? formatFcfa(item.prix) : null,
      item.document_supprime ? "🗑️ Document supprimé" : null,
    ]
      .filter(Boolean)
      .join(" · ");
    ligne.append(info, sousTexte);

    // Confirmation d'impression : d'abord ce que la base sait déjà (un
    // gérant qui rouvre l'historique plus tard), sinon un résultat reçu
    // entre-temps pendant que cette commande était encore en attente.
    if (item.kind === "imprimable") {
      const statutImpression = document.createElement("span");
      statutImpression.className = "meta-fichier statut-impression";
      statutImpression.hidden = true;
      ligne.appendChild(statutImpression);

      const dejaConnu = confirmationsImpression.get(item.id) ?? {
        confirmee: item.impression_confirmee,
        pages_imprimees: item.pages_imprimees,
        erreur: item.impression_erreur,
      };
      const texte = texteStatutImpression(dejaConnu);
      if (texte) {
        statutImpression.textContent = texte;
        statutImpression.hidden = false;
        statutImpression.style.color = dejaConnu.erreur ? "var(--rouge-alerte)" : "var(--vert-succes)";
      }
    }

    // Le client peut demander la suppression de son document à tout
    // moment après le passage en caisse — n'importe quel gérant doit
    // pouvoir le faire lui-même, sans mot de passe technique (contrairement
    // aux outils de dépannage réservés au porteur du projet).
    if (!item.document_supprime) {
      ligne.appendChild(
        bouton("Supprimer le document", "btn-discret", async () => {
          const ok = confirm(
            `Supprimer définitivement "${item.original_name}" ?\n\n` +
              "Le document sera effacé de cet ordinateur. La ligne de comptabilité " +
              "(montant, date) reste, elle ne contient jamais le document lui-même."
          );
          if (!ok) return;
          try {
            await invoke("supprimer_document", { id: item.id });
            toast("✓ Document supprimé");
            await ouvrirSection("historique");
          } catch (e) {
            alert(`⚠️ La suppression a échoué.\n\nDétail : ${e}`);
          }
        })
      );
    }
    corps.appendChild(ligne);
  }
}

async function rendreRecherche(corps) {
  corps.innerHTML = "";
  const champ = document.createElement("input");
  champ.type = "text";
  champ.placeholder = "Nom, téléphone ou nom de fichier…";
  corps.appendChild(champ);
  const resultats = document.createElement("div");
  corps.appendChild(resultats);

  champ.addEventListener("input", async () => {
    if (champ.value.trim().length < 2) {
      resultats.innerHTML = "";
      return;
    }
    const items = await invoke("rechercher_client", { terme: champ.value.trim() });
    resultats.innerHTML = "";
    for (const item of items) {
      resultats.appendChild(
        ligneListe(
          item.original_name,
          [item.client_name, item.client_telephone, formatHeure(item.received_at)]
            .filter(Boolean)
            .join(" · ")
        )
      );
    }
  });
}

async function rendreRapports(corps) {
  corps.innerHTML = "";

  const rapport = await invoke("rapport_du_jour");
  const resume = document.createElement("div");
  resume.className = "carte-rapport";
  resume.innerHTML = `
    <h3>Aujourd'hui (${rapport.date})</h3>
    <p>${rapport.nombre_commandes} commande(s) · Encaissé : <strong>${formatFcfa(rapport.total_encaisse)}</strong></p>
    <p>Impayés : ${formatFcfa(rapport.total_impaye)}</p>
    <p>Dépenses : ${formatFcfa(rapport.total_depenses)}</p>
    <p>Bénéfice net : <strong>${formatFcfa(rapport.benefice_net)}</strong></p>
  `;
  corps.appendChild(resume);

  const reconciliation = await invoke("rapport_reconciliation");
  const carteReconciliation = document.createElement("div");
  carteReconciliation.className = "carte-rapport";
  const raisonsHtml = reconciliation.ignores_par_raison
    .map((r) => `<li>${echapperHtml(r.raison)} : ${r.nombre}</li>`)
    .join("");
  carteReconciliation.innerHTML = `
    <h3>Réconciliation — tous les fichiers reçus aujourd'hui</h3>
    <p>Reçus : <strong>${reconciliation.recus}</strong> · Payés : ${reconciliation.payes} ·
       Ignorés : ${reconciliation.ignores} · En attente : ${reconciliation.en_attente}</p>
    ${raisonsHtml ? `<ul style="margin:0.3rem 0 0; padding-left:1.2rem; font-size:0.85rem">${raisonsHtml}</ul>` : ""}
    <p style="font-size:0.8rem; color:var(--gris-texte-discret); margin-top:0.5rem">
      Chaque fichier reçu (dossier surveillé, USB, QR) est compté automatiquement dès son
      arrivée — aucun ne peut disparaître sans une raison enregistrée.
    </p>
  `;
  corps.appendChild(carteReconciliation);

  const impayes = await invoke("list_impayes");
  if (impayes.length) {
    const sectionImpayes = document.createElement("section");
    sectionImpayes.innerHTML = "<h3>Impayés à relancer</h3>";
    for (const imp of impayes) {
      const ligne = document.createElement("div");
      ligne.className = "ligne-liste";
      const info = document.createElement("div");
      info.textContent = `${imp.client_name ?? "Client sans nom"} — ${formatFcfa(imp.montant)}`;
      const sousTexte = document.createElement("div");
      sousTexte.className = "meta-fichier";
      sousTexte.textContent = [
        imp.client_telephone ? `Tél. ${imp.client_telephone} — à relancer` : "Pas de téléphone enregistré",
        imp.description,
        formatHeure(imp.created_at),
      ]
        .filter(Boolean)
        .join(" · ");
      ligne.append(info, sousTexte);
      ligne.appendChild(
        bouton("Marquer réglé", "btn-discret", async () => {
          await invoke("marquer_impaye_regle", { transactionId: imp.transaction_id });
          await ouvrirSection("rapports");
        })
      );
      sectionImpayes.appendChild(ligne);
    }
    corps.appendChild(sectionImpayes);
  }

  const stock = await invoke("list_stock");
  const sectionStock = document.createElement("section");
  sectionStock.innerHTML = "<h3>Stock</h3>";
  for (const s of stock) {
    const ligne = document.createElement("div");
    ligne.className = "ligne-stock" + (s.alerte ? " alerte" : "");
    const input = document.createElement("input");
    input.type = "number";
    input.value = s.quantite;
    input.step = "0.1";
    input.addEventListener("change", () =>
      invoke("ajuster_stock", { item: s.item, quantite: Number(input.value) })
    );
    ligne.append(`${s.libelle} (${s.unite}) : `, input);
    if (s.alerte) ligne.append(" ⚠ stock bas");
    sectionStock.appendChild(ligne);
  }
  corps.appendChild(sectionStock);

  const sectionDepense = document.createElement("section");
  sectionDepense.innerHTML = "<h3>Ajouter une dépense</h3>";
  const formDepense = document.createElement("form");
  formDepense.innerHTML = `
    <input type="text" placeholder="Description" id="dep-description" required />
    <input type="number" placeholder="Montant FCFA" id="dep-montant" min="0" required />
    <select id="dep-categorie">
      <option value="papier">Papier</option>
      <option value="encre">Encre / toner</option>
      <option value="electricite">Électricité</option>
      <option value="autre">Autre</option>
    </select>
    <button type="submit" class="btn-secondaire">Ajouter</button>
  `;
  formDepense.addEventListener("submit", async (e) => {
    e.preventDefault();
    await invoke("ajouter_depense", {
      description: document.querySelector("#dep-description").value,
      montant: Number(document.querySelector("#dep-montant").value) || 0,
      categorie: document.querySelector("#dep-categorie").value,
    });
    await ouvrirSection("rapports");
  });
  sectionDepense.appendChild(formDepense);
  corps.appendChild(sectionDepense);

  const btnExport = bouton("Exporter les transactions (CSV)", "btn-secondaire", async () => {
    const csv = await invoke("exporter_transactions_csv");
    const blob = new Blob([csv], { type: "text/csv;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "transactions.csv";
    a.click();
    URL.revokeObjectURL(url);
  });
  corps.appendChild(btnExport);

  const sectionCloture = document.createElement("section");
  sectionCloture.innerHTML = `
    <h3>Clôture de caisse (espèces)</h3>
    <p style="font-size:0.8rem; color:var(--gris-texte-discret)">
      Compte le cash réellement dans le tiroir et compare au total encaissé
      en espèces aujourd'hui d'après l'application.
    </p>
  `;
  const formCloture = document.createElement("form");
  formCloture.innerHTML = `
    <input type="number" placeholder="Montant réellement compté (FCFA)" id="clot-reel" min="0" required />
    <button type="submit" class="btn-secondaire">Clôturer</button>
  `;
  const resultatCloture = document.createElement("p");
  resultatCloture.className = "chemin-dossier";
  formCloture.addEventListener("submit", async (e) => {
    e.preventDefault();
    const totalReel = Number(document.querySelector("#clot-reel").value) || 0;
    const r = await invoke("cloturer_caisse", { totalReel });
    resultatCloture.textContent =
      r.ecart === 0
        ? `Tout correspond : ${formatFcfa(r.total_attendu)}.`
        : `Attendu ${formatFcfa(r.total_attendu)}, compté ${formatFcfa(r.total_reel)} — écart de ${formatFcfa(r.ecart)}.`;
  });
  sectionCloture.appendChild(formCloture);
  sectionCloture.appendChild(resultatCloture);
  corps.appendChild(sectionCloture);

  const sectionImprimante = document.createElement("section");
  sectionImprimante.innerHTML = "<h3>Entretien imprimante</h3>";
  sectionImprimante.appendChild(
    bouton("Entretien effectué — réinitialiser le compteur", "btn-discret", async () => {
      await invoke("reinitialiser_compteur_imprimante");
      toast("✓ Compteur d'entretien réinitialisé");
    })
  );
  corps.appendChild(sectionImprimante);

  const sectionDemo = document.createElement("section");
  sectionDemo.innerHTML = "<h3>Démonstration</h3><p style=\"font-size:0.8rem; color:var(--gris-texte-discret)\">Pour montrer le logiciel sans client réel présent.</p>";
  sectionDemo.appendChild(
    bouton("Ajouter des exemples de démonstration", "btn-secondaire", async () => {
      await invoke("activer_mode_demo");
      toast("✓ Exemples ajoutés à la file d'attente");
    })
  );
  sectionDemo.appendChild(
    bouton("Retirer les exemples de démonstration", "btn-discret", async () => {
      await invoke("desactiver_mode_demo");
      await chargerFile();
      toast("✓ Exemples retirés");
    })
  );
  corps.appendChild(sectionDemo);
}

async function rendreReglages(corps) {
  corps.innerHTML = "";

  // Dossier surveillé
  const dossier = await invoke("get_watched_folder");
  const secDossier = document.createElement("section");
  secDossier.innerHTML = `<h3>Dossier surveillé</h3><p class="chemin-dossier">${echapperHtml(dossier ?? "Aucun dossier configuré")}</p>`;
  secDossier.appendChild(
    bouton("Choisir un dossier…", "btn-secondaire", async () => {
      const nouveauDossier = await invoke("choose_watched_folder");
      if (nouveauDossier) await ouvrirSection("reglages");
    })
  );
  corps.appendChild(secDossier);

  // Infos boutique
  const params = await invoke("get_boutique_settings");
  const secBoutique = document.createElement("section");
  secBoutique.innerHTML = "<h3>Boutique</h3>";
  const formBoutique = document.createElement("form");
  formBoutique.innerHTML = `
    <label>Nom de la boutique <input type="text" id="reg-nom" value="${echapperHtml(params.nom)}" /></label>
    <label>Numéro WhatsApp <input type="text" id="reg-whatsapp" value="${echapperHtml(params.whatsapp)}" /></label>
    <p style="font-size:0.8rem; color:var(--gris-texte-discret); margin:-0.5rem 0 0.75rem">
      À quoi sert ce numéro : sur l'écran d'envoi (celui que le client voit en
      scannant le QR), un bouton "Envoyer par WhatsApp à la place" apparaît
      pour ceux qui préfèrent ou ne peuvent pas utiliser le Wi-Fi de la
      boutique. Sans numéro renseigné ici, ce bouton n'apparaît tout
      simplement pas — rien ne casse, mais vous perdez cette option pour vos
      clients.
    </p>
    <label>Dossier de sauvegarde <input type="text" id="reg-sauvegarde" value="${echapperHtml(params.dossier_sauvegarde)}" /></label>
    <label>URL de vérification des mises à jour <input type="text" id="reg-url-maj" value="${echapperHtml(params.url_verification_maj)}" /></label>
    <button type="submit" class="btn-secondaire">Enregistrer</button>
  `;
  formBoutique.addEventListener("submit", async (e) => {
    e.preventDefault();
    await invoke("set_boutique_setting", { cle: "boutique_nom", valeur: document.querySelector("#reg-nom").value });
    await invoke("set_boutique_setting", { cle: "boutique_whatsapp", valeur: document.querySelector("#reg-whatsapp").value });
    await invoke("set_boutique_setting", { cle: "dossier_sauvegarde", valeur: document.querySelector("#reg-sauvegarde").value });
    await invoke("set_boutique_setting", { cle: "url_verification_maj", valeur: document.querySelector("#reg-url-maj").value });
    toast("✓ Réglages enregistrés");
  });
  secBoutique.appendChild(formBoutique);

  const logoActuel = document.createElement("p");
  logoActuel.className = "chemin-dossier";
  logoActuel.textContent = params.logo_chemin
    ? `Logo actuel : ${params.logo_chemin}`
    : "Aucun logo — les reçus seront en texte simple.";
  secBoutique.appendChild(logoActuel);
  secBoutique.appendChild(
    bouton("Choisir un logo (pour les reçus)…", "btn-secondaire", async () => {
      const chemin = await invoke("choisir_logo_boutique");
      if (chemin) await ouvrirSection("reglages");
    })
  );
  corps.appendChild(secBoutique);

  const secWifi = document.createElement("section");
  secWifi.innerHTML = `
    <h3>Wi-Fi local (pour le QR unique)</h3>
    <p style="font-size:0.8rem; color:var(--gris-texte-discret)">
      Recopiez ici le nom et le mot de passe affichés sur l'écran des
      paramètres Windows (bouton "Ouvrir le partage de connexion" dans la
      fenêtre QR) — permet au QR de connecter le client automatiquement.
    </p>
  `;
  const formWifi = document.createElement("form");
  formWifi.innerHTML = `
    <label>Nom du réseau (SSID) <input type="text" id="reg-wifi-ssid" value="${echapperHtml(params.wifi_ssid)}" /></label>
    <label>Mot de passe <input type="text" id="reg-wifi-mdp" value="${echapperHtml(params.wifi_mot_de_passe)}" /></label>
    <button type="submit" class="btn-secondaire">Enregistrer</button>
  `;
  formWifi.addEventListener("submit", async (e) => {
    e.preventDefault();
    await invoke("set_boutique_setting", { cle: "wifi_ssid", valeur: document.querySelector("#reg-wifi-ssid").value });
    await invoke("set_boutique_setting", { cle: "wifi_mot_de_passe", valeur: document.querySelector("#reg-wifi-mdp").value });
    toast("✓ Wi-Fi enregistré — le QR l'utilisera dès maintenant");
  });
  secWifi.appendChild(formWifi);
  corps.appendChild(secWifi);

  const secBluetooth = document.createElement("section");
  secBluetooth.innerHTML = `
    <h3>Bluetooth (pour les clients sans Wi-Fi)</h3>
    <p style="font-size:0.8rem; color:var(--gris-texte-discret)">
      Le nom que Windows affiche pour cet ordinateur quand on le cherche en
      Bluetooth — trouvez-le (ou changez-le) dans Windows : Paramètres >
      Bluetooth et appareils > Renommer cet appareil. Recopiez-le ici pour
      que le client sache exactement quel appareil chercher.
    </p>
  `;
  const formBluetooth = document.createElement("form");
  formBluetooth.innerHTML = `
    <label>Nom Bluetooth de cet ordinateur <input type="text" id="reg-bluetooth-nom" value="${echapperHtml(params.bluetooth_nom)}" placeholder="Ex: PC-Photocopie-Rapide" /></label>
    <button type="submit" class="btn-secondaire">Enregistrer</button>
  `;
  formBluetooth.addEventListener("submit", async (e) => {
    e.preventDefault();
    await invoke("set_boutique_setting", { cle: "bluetooth_nom", valeur: document.querySelector("#reg-bluetooth-nom").value });
    toast("✓ Nom Bluetooth enregistré — visible par vos clients dès maintenant");
  });
  secBluetooth.appendChild(formBluetooth);
  corps.appendChild(secBluetooth);

  // Fidélité — c'est au gérant de décider, pas à nous : seuil et
  // pourcentage sont réglables ici, jamais imposés dans le code.
  const secFidelite = document.createElement("section");
  secFidelite.innerHTML = `
    <h3>Fidélité client</h3>
    <p style="font-size:0.8rem; color:var(--gris-texte-discret)">
      Une réduction automatique s'applique quand un client (identifié par
      son numéro de téléphone) a déjà réglé plusieurs commandes. Mettez le
      pourcentage à 0 pour désactiver complètement la réduction.
    </p>
  `;
  const formFidelite = document.createElement("form");
  formFidelite.innerHTML = `
    <label>Nombre de visites payées avant réduction <input type="number" id="reg-fidelite-seuil" min="1" value="${echapperHtml(params.fidelite_seuil_visites || "5")}" /></label>
    <label>Pourcentage de réduction <input type="number" id="reg-fidelite-pourcent" min="0" max="100" value="${echapperHtml(params.fidelite_remise_pourcent || "10")}" /></label>
    <button type="submit" class="btn-secondaire">Enregistrer</button>
  `;
  formFidelite.addEventListener("submit", async (e) => {
    e.preventDefault();
    await invoke("set_boutique_setting", { cle: "fidelite_seuil_visites", valeur: document.querySelector("#reg-fidelite-seuil").value || "5" });
    await invoke("set_boutique_setting", { cle: "fidelite_remise_pourcent", valeur: document.querySelector("#reg-fidelite-pourcent").value || "0" });
    toast("✓ Réglages de fidélité enregistrés");
  });
  secFidelite.appendChild(formFidelite);
  corps.appendChild(secFidelite);

  // Conservation des documents clients
  const secRetention = document.createElement("section");
  secRetention.innerHTML = `
    <h3>Conservation des documents clients</h3>
    <p style="font-size:0.8rem; color:var(--gris-texte-discret)">
      Les documents envoyés par vos clients (CV, relevés, pièces...) sont
      effacés automatiquement de ce PC passé ce délai, une fois la commande
      terminée. Votre comptabilité, elle, n'est jamais effacée : seuls les
      documents partent. Mettez 0 pour tout garder — mais le disque finira
      par se remplir, et garder longtemps les papiers personnels de vos
      clients vous engage.
    </p>
  `;
  const formRetention = document.createElement("form");
  formRetention.innerHTML = `
    <label>Effacer les documents après (jours) <input type="number" id="reg-retention" min="0" max="3650" value="${echapperHtml(params.retention_jours || "30")}" /></label>
    <button type="submit" class="btn-secondaire">Enregistrer</button>
  `;
  formRetention.addEventListener("submit", async (e) => {
    e.preventDefault();
    try {
      await invoke("set_boutique_setting", { cle: "retention_jours", valeur: document.querySelector("#reg-retention").value || "30" });
      toast("✓ Durée de conservation enregistrée");
    } catch (err) {
      toast(String(err), "attention");
    }
  });
  secRetention.appendChild(formRetention);
  corps.appendChild(secRetention);

  // Rapport de diagnostic pour le porteur du projet
  const secRapport = document.createElement("section");
  secRapport.innerHTML = `
    <h3>Rapport pour le porteur du projet</h3>
    <p style="font-size:0.8rem; color:var(--gris-texte-discret)">
      À donner au porteur du projet lors d'une visite (clé USB, ou envoyé par
      vous-même si vous avez du réseau) — il l'utilise pour suivre l'état de
      votre licence et de votre sauvegarde, sans que ce PC soit connecté à
      internet.
    </p>
  `;
  secRapport.appendChild(
    bouton("Générer le rapport", "btn-secondaire", async () => {
      const rapport = await invoke("generer_rapport_diagnostic");
      const texte = JSON.stringify(rapport, null, 2);
      const zone = document.createElement("textarea");
      zone.readOnly = true;
      zone.rows = 10;
      zone.style.fontFamily = "Consolas, monospace";
      zone.style.fontSize = "0.8rem";
      zone.value = texte;
      const btnCopier = bouton("Copier", "btn-secondaire", async () => {
        try {
          await navigator.clipboard.writeText(texte);
          toast("✓ Rapport copié — colle-le dans un message au porteur du projet");
        } catch {
          zone.select();
          toast("Sélectionné — copiez avec Ctrl+C, le copier automatique n'a pas fonctionné ici.", "attention");
        }
      });
      const ancienResultat = document.querySelector("#rapport-resultat");
      if (ancienResultat) ancienResultat.remove();
      const conteneur = document.createElement("div");
      conteneur.id = "rapport-resultat";
      conteneur.style.marginTop = "0.5rem";
      conteneur.appendChild(zone);
      conteneur.appendChild(btnCopier);
      secRapport.appendChild(conteneur);
    })
  );
  corps.appendChild(secRapport);

  // Grille tarifaire
  const tarifs = await invoke("list_tarifs");
  const secTarifs = document.createElement("section");
  secTarifs.innerHTML = "<h3>Grille tarifaire (FCFA)</h3>";
  for (const t of tarifs) {
    const ligne = document.createElement("div");
    ligne.className = "ligne-stock";
    const input = document.createElement("input");
    input.type = "number";
    input.value = t.prix_unitaire;
    input.addEventListener("change", () =>
      invoke("update_tarif", { id: t.id, prixUnitaire: Number(input.value) })
    );
    ligne.append(`${t.libelle} (par ${t.unite}) : `, input);
    secTarifs.appendChild(ligne);
  }
  corps.appendChild(secTarifs);

  const historiqueTarifs = await invoke("list_historique_tarifs");
  if (historiqueTarifs.length) {
    const secHistoriqueTarifs = document.createElement("section");
    secHistoriqueTarifs.innerHTML = "<h3>Historique des changements de tarifs</h3>";
    for (const h of historiqueTarifs) {
      secHistoriqueTarifs.appendChild(
        ligneListe(
          `${h.libelle} : ${h.ancien_prix} → ${h.nouveau_prix} FCFA`,
          formatHeure(h.changed_at) + " · " + new Date(h.changed_at).toLocaleDateString("fr-FR")
        )
      );
    }
    corps.appendChild(secHistoriqueTarifs);
  }

  // Employés
  const employes = await invoke("list_employes");
  const secEmployes = document.createElement("section");
  secEmployes.innerHTML = "<h3>Employés</h3>";
  for (const emp of employes) {
    secEmployes.appendChild(ligneListe(emp.nom, emp.actif ? "Actif" : "Inactif"));
  }
  const formEmploye = document.createElement("form");
  formEmploye.innerHTML = `<input type="text" id="nouvel-employe" placeholder="Nom de l'employé" /><button type="submit" class="btn-secondaire">Ajouter</button>`;
  formEmploye.addEventListener("submit", async (e) => {
    e.preventDefault();
    const nom = document.querySelector("#nouvel-employe").value.trim();
    if (!nom) return;
    await invoke("ajouter_employe", { nom });
    await ouvrirSection("reglages");
  });
  secEmployes.appendChild(formEmploye);
  corps.appendChild(secEmployes);

  // Licence
  const licence = await invoke("get_license_status");
  const secLicence = document.createElement("section");
  const libellesStatut = {
    essai: `Essai gratuit — ${licence.jours_restants} jour(s) restant(s)`,
    actif: `Abonnement actif (jusqu'au ${licence.date_expiration})`,
    expire: "Abonnement expiré — contactez le porteur du projet",
    invalide: "Clé de licence invalide",
  };
  secLicence.innerHTML = `
    <h3>Licence</h3>
    <p>${libellesStatut[licence.statut] ?? licence.statut}</p>
    <p class="chemin-dossier">Identifiant machine (à communiquer au porteur du projet) : ${licence.machine_id}</p>
  `;
  const formLicence = document.createElement("form");
  formLicence.innerHTML = `<input type="text" id="cle-licence" placeholder="Coller la clé de licence reçue" /><button type="submit" class="btn-secondaire">Activer</button>`;
  formLicence.addEventListener("submit", async (e) => {
    e.preventDefault();
    const ok = await invoke("set_license_key", { cle: document.querySelector("#cle-licence").value });
    if (ok) {
      await ouvrirSection("reglages");
      await rafraichirBadgeAbonnement();
      await verifierBlocageLicence();
      toast("✓ Licence activée — merci !");
    } else {
      toast("Cette clé n'est pas reconnue. Vérifiez qu'elle est copiée en entier, sans espace avant ni après.", "attention");
    }
  });
  secLicence.appendChild(formLicence);
  corps.appendChild(secLicence);

  const version = await invoke("version_actuelle");
  const versionEl = document.createElement("p");
  versionEl.className = "version";
  versionEl.textContent = `Version ${version}`;
  corps.appendChild(versionEl);

  // Écran technique caché — triple-clic sur le numéro de version, pour le
  // porteur du projet en visite de dépannage. Pas une vraie barrière de
  // sécurité (l'appli tourne déjà en local avec les mêmes droits que le
  // gérant), juste pour éviter qu'un gérant curieux tombe dessus par hasard.
  let clicsVersion = 0;
  let minuteurClicsVersion = null;
  versionEl.addEventListener("click", () => {
    clicsVersion++;
    clearTimeout(minuteurClicsVersion);
    minuteurClicsVersion = setTimeout(() => {
      clicsVersion = 0;
    }, 1000);
    if (clicsVersion >= 3) {
      clicsVersion = 0;
      ouvrirEcranTechnique(corps);
    }
  });
}

const MOT_DE_PASSE_TECHNIQUE = "ATINZ-TECH-2026";

async function ouvrirEcranTechnique(corps) {
  const ancien = document.querySelector("#section-technique");
  if (ancien) {
    ancien.remove();
    return;
  }
  const mdp = prompt("Mot de passe technique :");
  if (mdp === null) return;
  if (mdp !== MOT_DE_PASSE_TECHNIQUE) {
    alert("Mot de passe incorrect.");
    return;
  }
  const rapport = await invoke("generer_rapport_diagnostic");
  const sec = document.createElement("section");
  sec.id = "section-technique";
  sec.innerHTML = `
    <h3>🔧 Outils techniques</h3>
    <pre style="font-size:0.75rem; background:var(--gris-clair); padding:0.5rem; border-radius:4px; overflow-x:auto">${echapperHtml(JSON.stringify(rapport, null, 2))}</pre>
  `;
  sec.appendChild(
    bouton("Forcer une sauvegarde maintenant", "btn-secondaire", async () => {
      await invoke("sauvegarder_maintenant");
      toast("✓ Sauvegarde effectuée");
    })
  );
  sec.appendChild(
    bouton("Ouvrir le dossier de données", "btn-secondaire", async () => {
      await invoke("ouvrir_dossier_donnees");
    })
  );

  // Restauration — le filet de sécurité qui manquait : sans lui, les
  // sauvegardes automatiques ne servaient à rien tant que personne ne
  // venait copier un fichier à la main sur place.
  sec.appendChild(
    bouton("Restaurer une sauvegarde…", "btn-secondaire", async () => {
      const ancienne = document.querySelector("#liste-sauvegardes");
      if (ancienne) {
        ancienne.remove();
        return;
      }
      const sauvegardes = await invoke("lister_sauvegardes");
      const bloc = document.createElement("div");
      bloc.id = "liste-sauvegardes";
      if (!sauvegardes.length) {
        bloc.innerHTML = `<p class="avertissement">Aucune sauvegarde disponible pour l'instant.</p>`;
        sec.appendChild(bloc);
        return;
      }
      bloc.innerHTML = `<p style="font-size:0.8rem; color:var(--gris-texte-discret)">
        Remplace les données actuelles par celles de la sauvegarde choisie.
        L'état actuel est d'abord mis de côté, donc rien n'est perdu
        définitivement même en cas d'erreur.</p>`;
      for (const s of sauvegardes) {
        const ligne = document.createElement("div");
        ligne.className = "ligne-liste";
        const info = document.createElement("div");
        info.textContent = `${s.date_lisible} — ${(s.taille_octets / 1024).toFixed(0)} Ko`;
        ligne.appendChild(info);
        ligne.appendChild(
          bouton("Restaurer", "btn-discret", async () => {
            if (!confirm(`Remplacer les données actuelles par la sauvegarde du ${s.date_lisible} ?`)) return;
            try {
              await invoke("restaurer_sauvegarde", { chemin: s.chemin });
              alert("Sauvegarde restaurée. L'application va se recharger.");
              location.reload();
            } catch (err) {
              alert(`⚠️ La restauration a échoué.\n\nDétail : ${err}`);
            }
          })
        );
        bloc.appendChild(ligne);
      }
      sec.appendChild(bloc);
    })
  );
  corps.appendChild(sec);
}

// ───────────────────────────── Statut abonnement / mise à jour ─────────────────────────────

async function rafraichirBadgeAbonnement() {
  const badge = document.querySelector("#badge-abonnement");
  try {
    const licence = await invoke("get_license_status");
    const libelles = {
      essai: `Essai — ${licence.jours_restants}j`,
      actif: "Abonnement actif",
      expire: "Abonnement expiré",
      invalide: "Licence invalide",
    };
    badge.textContent = libelles[licence.statut] ?? "";
    // Calme tant qu'il reste largement le temps de renouveler — le jaune
    // n'apparaît que dans les 7 derniers jours, pour ne pas donner une
    // impression d'urgence permanente dès le premier jour d'essai.
    const classeStatut =
      licence.statut === "essai" && licence.jours_restants > 7 ? "essai-calme" : licence.statut;
    badge.className = "badge-abonnement badge-" + classeStatut;
    badge.hidden = false;
  } catch {
    badge.hidden = true;
  }
}

// Blocage réel à expiration — pas un simple badge. Couvre tout l'écran, rien
// n'est cliquable derrière tant que l'abonnement n'est pas renouvelé. Le
// formulaire d'activation reste accessible DANS ce même écran, pour ne
// jamais enfermer le gérant sans porte de sortie une fois qu'il a sa
// nouvelle clé.
/// Nombre de jours pendant lesquels le gérant est prévenu, avant que le
/// blocage ne tombe. Le blocage lui-même reste sec — c'est la découverte du
/// blocage qui doit cesser d'être une surprise, pas le blocage.
const JOURS_AVERTISSEMENT = 3;

/// Avertissement des derniers jours : visible en permanence, impossible à
/// fermer, mais ne gêne aucun geste de travail.
function majBandeauGrace(licence) {
  const bandeau = document.querySelector("#bandeau-grace");
  const concerne = licence.statut === "essai" || licence.statut === "actif";
  if (!concerne || licence.jours_restants > JOURS_AVERTISSEMENT) {
    bandeau.hidden = true;
    return;
  }

  const quoi = licence.statut === "essai" ? "Votre essai gratuit" : "Votre abonnement";
  const quand =
    licence.jours_restants <= 0
      ? "se termine aujourd'hui"
      : licence.jours_restants === 1
        ? "se termine demain"
        : `se termine dans ${licence.jours_restants} jours`;
  document.querySelector("#bandeau-grace-titre").textContent =
    `⚠️ ${quoi} ${quand} — ensuite, l'application ne pourra plus être utilisée.`;
  document.querySelector("#machine-id-bandeau").textContent = licence.machine_id;
  bandeau.hidden = false;
}

async function verifierBlocageLicence() {
  const overlay = document.querySelector("#overlay-licence");
  try {
    const licence = await invoke("get_license_status");
    const bloque = licence.statut === "expire" || licence.statut === "invalide";
    majBandeauGrace(licence);
    if (!bloque) {
      overlay.hidden = true;
      return;
    }
    document.querySelector("#titre-blocage").textContent =
      licence.statut === "invalide"
        ? "Votre clé de licence n'est plus valide"
        : "Votre période d'essai ou votre abonnement est terminé";
    document.querySelector("#machine-id-blocage").textContent = licence.machine_id;
    overlay.hidden = false;
  } catch {
    // Impossible de vérifier le statut : on ne bloque jamais sur un doute,
    // seulement sur une expiration confirmée. Même raisonnement pour
    // l'avertissement — pas d'alarme rouge sur une lecture ratée.
    overlay.hidden = true;
    document.querySelector("#bandeau-grace").hidden = true;
  }
}

// Affichée une seule fois par nouvelle version, juste après une activation
// de licence réussie — pour que le gérant voie concrètement ce qu'il gagne
// à renouveler, pas seulement qu'il a payé.
async function afficherNouveautesSiBesoin() {
  let nouveautes = [];
  try {
    nouveautes = await invoke("recuperer_nouveautes_et_marquer_vues");
  } catch {
    return;
  }
  if (!nouveautes.length) return;

  const liste = document.querySelector("#liste-nouveautes");
  liste.innerHTML = "";
  for (const n of nouveautes) {
    const bloc = document.createElement("div");
    bloc.className = "bloc-nouveaute";
    bloc.innerHTML = `
      <h3>${echapperHtml(n.titre)}</h3>
      <p>${echapperHtml(n.description)}</p>
      <p class="ou-trouver">📍 ${echapperHtml(n.ou_trouver)}</p>
    `;
    liste.appendChild(bloc);
  }
  document.querySelector("#overlay-nouveautes").hidden = false;
}

const SEUIL_OUBLI_MS = 30 * 60 * 1000; // 30 minutes

// Badge doux (pas une alerte bruyante) qui signale les commandes prêtes à
// imprimer/encaisser mais laissées de côté trop longtemps — le genre de
// chose qu'on oublie facilement dans le rush.
async function mettreAJourBadgeOublies() {
  const badge = document.querySelector("#badge-oublies");
  try {
    const items = await invoke("get_queue");
    const maintenant = Date.now();
    const oublies = items.filter((item) => {
      if (item.kind !== "imprimable" && item.kind !== "editable") return false;
      const recu = new Date(item.received_at).getTime();
      return !Number.isNaN(recu) && maintenant - recu > SEUIL_OUBLI_MS;
    });
    if (oublies.length) {
      badge.textContent = `⏳ ${oublies.length} en attente depuis plus de 30 min`;
      badge.hidden = false;
    } else {
      badge.hidden = true;
    }
  } catch {
    badge.hidden = true;
  }
}

async function verifierMiseAJour() {
  try {
    const nouvelleVersion = await invoke("verifier_mise_a_jour");
    const badge = document.querySelector("#badge-maj");
    if (nouvelleVersion) {
      badge.textContent = `Mise à jour disponible (${nouvelleVersion})`;
      badge.hidden = false;
    }
  } catch {
    // silencieux : pas de connexion, ce n'est pas une erreur.
  }
}

// Message chaleureux au tout premier chargement du jour — ne se répète pas
// à chaque réouverture de l'appli le même jour (voir verifier_et_marquer_-
// affichage_du_jour côté Rust). Seulement le matin/début d'après-midi, pour
// ne pas dire "Bonjour" en pleine soirée si la boutique ouvre tard.
async function afficherAccueilDuJour() {
  if (new Date().getHours() >= 14) return;
  try {
    const doitAfficher = await invoke("verifier_et_marquer_affichage_du_jour", { cle: "accueil" });
    if (!doitAfficher) return;
    const hier = await invoke("rapport_hier");
    const message =
      hier.nombre_commandes > 0
        ? `👋 Bonjour ! Hier : ${hier.nombre_commandes} commande(s), ${formatFcfa(hier.total_encaisse)} encaissés.`
        : "👋 Bonjour ! Prêt pour une nouvelle journée.";
    toast(message, "succes", 6000);
  } catch {
    // Pas grave si ça échoue — ce n'est qu'un message de bienvenue.
  }
}

const PHRASES_ENCOURAGEMENT = [
  "Belle journée de travail !",
  "Continuez comme ça !",
  "Une bonne journée pour la boutique.",
  "Bravo pour le travail accompli aujourd'hui !",
];

// Se propose tout seul en fin de journée, une seule fois — jamais à la
// demande pour ne pas être intrusif, jamais deux fois le même jour.
async function proposerResumeFinDeJournee() {
  if (new Date().getHours() < 18) return;
  try {
    const doitAfficher = await invoke("verifier_et_marquer_affichage_du_jour", { cle: "resume_jour" });
    if (!doitAfficher) return;
    const rapport = await invoke("rapport_du_jour");
    if (rapport.nombre_commandes === 0) return; // rien à résumer
    let message = `🌙 Résumé du jour : ${rapport.nombre_commandes} commande(s), ${formatFcfa(rapport.total_encaisse)} encaissés. Bénéfice net : ${formatFcfa(rapport.benefice_net)}.`;
    if (rapport.nombre_commandes >= 3 && Math.random() < 0.6) {
      message += " " + PHRASES_ENCOURAGEMENT[Math.floor(Math.random() * PHRASES_ENCOURAGEMENT.length)];
    }
    toast(message, "succes", 7000);
  } catch {
    // Pas grave si ça échoue — ce n'est qu'un résumé de confort.
  }
}

// ───────────────────────────── Assistant de premier lancement ─────────────────────────────

function afficherEtapeBienvenue(id) {
  for (const el of document.querySelectorAll('[id^="etape-bienvenue-"]')) {
    el.hidden = el.id !== id;
  }
}

/// Renvoie true si l'assistant vient de s'ouvrir pour un tout premier
/// démarrage — sert à savoir si le démarrage doit vérifier les nouveautés
/// (jamais lors d'un premier lancement : tout est "nouveau" pour ce gérant,
/// ce n'est pas ça qu'on veut annoncer).
async function lancerAssistantPremierDemarrage() {
  const params = await invoke("get_boutique_settings");
  if (params.nom) return false; // déjà configuré, pas besoin de l'assistant

  ouvrirModal("modal-bienvenue");
  afficherEtapeBienvenue("etape-bienvenue-1");

  document
    .querySelector('[data-suivant="etape-bienvenue-2"]')
    .addEventListener("click", () => afficherEtapeBienvenue("etape-bienvenue-2"));

  document.querySelector("#bv-choisir-dossier").addEventListener("click", async () => {
    const dossier = await invoke("choose_watched_folder");
    if (dossier) document.querySelector("#bv-dossier").textContent = dossier;
  });

  document.querySelector("#bv-suivant-2").addEventListener("click", async () => {
    const nom = document.querySelector("#bv-nom").value.trim();
    if (!nom) {
      toast("Il nous faut au moins le nom de votre boutique — c'est ce qui apparaîtra sur vos reçus.", "attention");
      return;
    }
    await invoke("set_boutique_setting", { cle: "boutique_nom", valeur: nom });
    const whatsapp = document.querySelector("#bv-whatsapp").value.trim();
    if (whatsapp) {
      await invoke("set_boutique_setting", { cle: "boutique_whatsapp", valeur: whatsapp });
    }
    const licence = await invoke("get_license_status");
    document.querySelector("#bv-machine-id").textContent = licence.machine_id;
    afficherEtapeBienvenue("etape-bienvenue-3");
  });

  document.querySelector("#bv-terminer").addEventListener("click", async () => {
    // Marque la version installée comme "déjà vue" — sinon, au prochain
    // démarrage, ce gérant tout neuf se verrait présenter les fonctionnalités
    // qu'il utilise depuis le premier jour comme une mise à jour fraîche.
    await invoke("marquer_version_actuelle_vue");
    fermerModal("modal-bienvenue");
  });
  return true;
}

// ───────────────────────────── Démarrage ─────────────────────────────

window.addEventListener("DOMContentLoaded", async () => {
  document.querySelector("#btn-menu").addEventListener("click", ouvrirPanneauMenu);
  document.querySelector("#btn-fermer-menu").addEventListener("click", fermerPanneauMenu);
  document.querySelector("#btn-retour-nav").addEventListener("click", () => {
    document.querySelector("#panneau-nav").hidden = false;
    document.querySelector("#panneau-contenu").hidden = true;
    document.querySelector("#panneau-titre").textContent = "Menu";
  });
  document.querySelectorAll(".item-nav").forEach((btn) => {
    btn.addEventListener("click", () => ouvrirSection(btn.dataset.section));
  });
  document.querySelectorAll(".btn-fermer-modal").forEach((btn) => {
    btn.addEventListener("click", () => fermerModal(btn.dataset.cible));
  });
  document.querySelector("#btn-recevoir-qr").addEventListener("click", afficherQr);
  document.querySelector("#form-licence-blocage").addEventListener("submit", async (e) => {
    e.preventDefault();
    const champ = document.querySelector("#cle-licence-blocage");
    const ok = await invoke("set_license_key", { cle: champ.value });
    if (ok) {
      champ.value = "";
      toast("✓ Licence activée — merci !");
      await verifierBlocageLicence();
      await rafraichirBadgeAbonnement();
    } else {
      toast("Cette clé n'est pas reconnue. Vérifiez qu'elle est copiée en entier, sans espace avant ni après.", "attention");
    }
  });
  document.querySelector("#btn-fermer-nouveautes").addEventListener("click", () => {
    document.querySelector("#overlay-nouveautes").hidden = true;
  });

  const premierLancement = await lancerAssistantPremierDemarrage();
  await chargerFile();
  await rafraichirBadgeAbonnement();
  await verifierBlocageLicence();
  // Jamais au tout premier lancement : ce gérant n'a encore rien "gagné",
  // il découvre juste l'application pour la première fois.
  if (!premierLancement) {
    await afficherNouveautesSiBesoin();
  }
  await verifierMiseAJour();
  await afficherAccueilDuJour();
  await proposerResumeFinDeJournee();
  setInterval(mettreAJourBadgeOublies, 60000);
  // Toutes les 10 minutes : si l'appli reste ouverte pendant que l'essai
  // expire, le blocage doit apparaître sans attendre un redémarrage.
  setInterval(verifierBlocageLicence, 10 * 60 * 1000);

  await listen("nouveau-fichier", (event) => {
    ajouterFichier(event.payload, true);
    jouerNotification();
  });

  // Confirmation d'impression, reçue en arrière-plan jusqu'à 45 secondes
  // après le clic sur "Imprimer" (voir impression.rs). Peut arriver que la
  // commande soit encore dans la file, ou déjà passée en caisse — on met
  // donc à jour partout où sa ligne pourrait exister à cet instant.
  await listen("impression-confirmee", (event) => {
    confirmationsImpression.set(event.payload.id, event.payload);
    appliquerStatutImpression(event.payload.id, event.payload);
  });

  // Clé USB contenant un fichier "licence.txt" : évite au gérant de retaper
  // à la main une clé signée de plus de 100 caractères.
  await listen("licence-usb", async (event) => {
    if (event.payload.reussi) {
      toast("✓ Licence activée depuis la clé USB — merci !");
      await verifierBlocageLicence();
      await rafraichirBadgeAbonnement();
    } else {
      toast("Le fichier licence.txt trouvé sur la clé USB n'est pas reconnu.", "attention");
    }
  });
});
