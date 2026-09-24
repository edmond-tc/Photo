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

// ── Sons ──────────────────────────────────────────────────────────────
//
// Signalé du terrain : « je ne sens rien ». Deux causes, toutes deux
// invisibles parce que chaque échec était avalé sans un mot.
//
// 1. Un NOUVEL appareil audio était créé à chaque son. Le navigateur en
//    limite le nombre à six par page : au septième, la création échoue, et
//    l'application devenait définitivement muette pour le reste de la
//    journée sans que rien ne le dise.
//
// 2. Un appareil audio fraîchement créé naît ENDORMI, et ne se réveille
//    qu'à la demande, après un geste de l'utilisateur. Le son le plus utile
//    — l'arrivée d'un document — se déclenche justement sans geste : il
//    n'avait donc aucune chance de s'entendre.
//
// Un seul appareil est désormais créé, puis réveillé au premier clic ou à
// la première touche. Une fois réveillé, il le reste : les sons déclenchés
// en arrière-plan passent alors normalement.
let appareilAudio = null;

function contexteAudio() {
  if (!appareilAudio) {
    try {
      appareilAudio = new (window.AudioContext || window.webkitAudioContext)();
    } catch {
      return null;
    }
  }
  if (appareilAudio.state === "suspended") {
    appareilAudio.resume().catch(() => {});
  }
  return appareilAudio.state === "closed" ? null : appareilAudio;
}

function debloquerSon() {
  const ctx = contexteAudio();
  if (ctx && ctx.state === "running") {
    window.removeEventListener("pointerdown", debloquerSon);
    window.removeEventListener("keydown", debloquerSon);
  }
}
window.addEventListener("pointerdown", debloquerSon, { passive: true });
window.addEventListener("keydown", debloquerSon, { passive: true });

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
  const ctx = contexteAudio();
  if (!ctx) return;
  try {
    jouerTonalite(ctx, { freq: 988, debut: 0, duree: 0.35, gainMax: 0.22 });
    jouerTonalite(ctx, { freq: 740, debut: 0.28, duree: 0.5, gainMax: 0.2 });
  } catch {
    // Un son manqué ne doit jamais interrompre la réception d'un document.
  }
}

// Petit "tic" — confirme que l'impression a bien été envoyée.
function jouerSonImpression() {
  const ctx = contexteAudio();
  if (!ctx) return;
  try {
    jouerTonalite(ctx, { freq: 659, debut: 0, duree: 0.13, gainMax: 0.14 });
  } catch {
    // Pas grave si le son ne peut pas jouer.
  }
}

// Deux notes montantes — confirme qu'un encaissement vient d'être validé.
function jouerSonEncaissement() {
  const ctx = contexteAudio();
  if (!ctx) return;
  try {
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

// Deux choses différentes s'impriment depuis cette application : le rapport
// (ou le guide) et l'affiche QR. La classe posée ici dit à la feuille de
// style laquelle des deux part sur le papier — sans elle, les deux règles
// « tout cacher sauf X » s'annulaient et la page sortait blanche.
function imprimerPage(classe) {
  document.body.classList.add(classe);
  const nettoyer = () => document.body.classList.remove(classe);
  window.addEventListener("afterprint", nettoyer, { once: true });
  try {
    window.print();
  } finally {
    // Filet de sécurité : certaines fenêtres n'émettent jamais "afterprint"
    // (impression annulée, moteur embarqué). La classe ne doit jamais rester
    // collée au <body>, sinon l'impression suivante sortirait la mauvaise page.
    setTimeout(nettoyer, 1000);
  }
}

// ───────────────────────────── File d'attente ─────────────────────────────

// Résultats de confirmation d'impression déjà reçus, par identifiant de
// commande — le spouleur Windows peut mettre jusqu'à 45 secondes à
// répondre (voir impression.rs), largement après le premier affichage de
// la ligne. Permet de retrouver l'info si la ligne est reconstruite entre
// temps (nouveau fichier arrivé, changement de section...).
const confirmationsImpression = new Map();

// Libellés lisibles pour chaque champ pouvant différer entre ce qui a été
// facturé et ce que l'imprimante a réellement reçu (voir
// impression::comparer_a_la_facturation, côté Rust).
const LIBELLES_CHAMP_ECART = {
  feuilles: "nombre de feuilles",
  couleur: "couleur",
  recto_verso: "recto-verso",
  format_papier: "format papier",
};

function texteEcarts(ecarts) {
  if (!ecarts?.length) return null;
  const detail = ecarts
    .map((e) => `${LIBELLES_CHAMP_ECART[e.champ] ?? e.champ} : facturé ${e.facture}, imprimé ${e.imprime}`)
    .join(" · ");
  return `⚠️ Écart avec la facturation — ${detail}`;
}

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
  if (texte) {
    document.querySelectorAll(`[data-id="${id}"] .statut-impression`).forEach((el) => {
      el.textContent = texte;
      el.hidden = false;
      el.style.color = payload.erreur ? "var(--rouge-alerte)" : "var(--vert-succes)";
    });
  }

  // Jamais bloquant, jamais mélangé avec le statut de réussite : un écart
  // constaté après une impression confirmée reste une impression réussie,
  // juste avec un détail à signaler au propriétaire.
  const texteEcart = texteEcarts(payload.ecarts);
  if (texteEcart) {
    document.querySelectorAll(`[data-id="${id}"] .ecarts-impression`).forEach((el) => {
      el.textContent = texteEcart;
      el.hidden = false;
    });
  }
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
    item.recto_verso ? "Recto-verso" : null,
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

    const ecartsImpression = document.createElement("span");
    ecartsImpression.className = "meta-fichier ecarts-impression";
    ecartsImpression.style.color = "var(--jaune-alerte)";
    ecartsImpression.hidden = true;
    noeud.querySelector(".info-fichier").appendChild(ecartsImpression);

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

// Liste déroulante d'imprimante : "" veut dire "imprimante par défaut du
// PC" (comportement inchangé) — voir chargerImprimantes().
function imprimanteChoisie() {
  const valeur = document.querySelector("#imprimante-choisie")?.value;
  return valeur ? valeur : undefined;
}

async function imprimer(id) {
  try {
    await invoke("print_file", { id, imprimante: imprimanteChoisie() });
    jouerSonImpression();
  } catch (e) {
    alert(`⚠️ L'impression n'a pas pu démarrer. Vérifiez que l'imprimante est allumée et connectée, puis réessayez.\n\nDétail : ${e}`);
  }
}

// Rempli une seule fois au démarrage : la liste des imprimantes installées
// ne change pas pendant qu'on utilise l'application. Masqué s'il n'y a
// qu'une seule imprimante (ou aucune détectée) — rien à choisir.
async function chargerImprimantes() {
  let imprimantes = [];
  try {
    imprimantes = await invoke("lister_imprimantes");
  } catch {
    return;
  }
  if (imprimantes.length < 2) return;

  const select = document.querySelector("#imprimante-choisie");
  select.innerHTML = "";
  const optionDefaut = document.createElement("option");
  optionDefaut.value = "";
  optionDefaut.textContent = "Imprimante par défaut";
  select.appendChild(optionDefaut);
  for (const nom of imprimantes) {
    const option = document.createElement("option");
    option.value = nom;
    option.textContent = nom;
    select.appendChild(option);
  }
  document.querySelector("#selecteur-imprimante").hidden = false;
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
  if (idApercuEnCours == null) return;
  // Refermé avant d'imprimer : sinon l'aperçu reste ouvert par-dessus la file
  // une fois la fenêtre d'impression de Windows passée, et le gérant croit
  // que l'application est figée.
  fermerModal("modal-apercu");
  imprimer(idApercuEnCours);
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
  // Les deux chiffres réellement saisis sont relus tels quels : le nombre de
  // pages est mémorisé à part (pages_document), le total de feuilles restant
  // dans `copies`. Rouvrir cette fenêtre réaffiche donc exactement ce que le
  // gérant avait entré — sinon, corriger le nombre de pages remultipliait le
  // total déjà multiplié.
  const pages = Math.max(1, item.pages_document ?? 1);
  const total = Math.max(1, item.copies ?? 1);
  document.querySelector("#opt-pages").value = pages;
  document.querySelector("#opt-nb-copies").value = Math.max(1, Math.round(total / pages));
  majTotalFeuilles();
  document.querySelector("#opt-couleur").checked = !!item.couleur;
  document.querySelector("#opt-recto-verso").checked = !!item.recto_verso;
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
      pagesDocument: pages,
      copies: pages * nbCopies,
      couleur: document.querySelector("#opt-couleur").checked,
      rectoVerso: document.querySelector("#opt-recto-verso").checked,
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
  } catch (e) {
    // Le prix n'a pas pu être calculé (tarif manquant, base occupée...) :
    // on le dit. Sans ce message, l'écran affichait « 0 FCFA » comme si
    // c'était le vrai prix, puis réclamait au gérant de justifier un
    // « écart » par rapport à ce zéro dès qu'il tapait le montant réel.
    montantCalculeEnCours = null;
    document.querySelector("#enc-montant-calcule").textContent = "à saisir à la main";
    document.querySelector("#enc-montant").value = "";
    toast(
      `Le prix n'a pas pu être calculé automatiquement (${e}) — saisissez le montant vous-même.`,
      "attention",
      6000
    );
  }
  await remplirEmployes(document.querySelector("#enc-employe"));
  ouvrirModal("modal-encaissement");
}

document.querySelector("#enc-montant").addEventListener("input", (e) => {
  // `null` = aucun prix n'a pu être calculé : il n'y a alors pas d'écart
  // possible, donc rien à justifier.
  const diffère = montantCalculeEnCours != null && Number(e.target.value) !== montantCalculeEnCours;
  document.querySelector("#enc-raison-champ").hidden = !diffère;
});

document.querySelector("#form-encaissement").addEventListener("submit", async (e) => {
  e.preventDefault();
  const moyen = document.querySelector("#enc-moyen").value;
  const montant = Number(document.querySelector("#enc-montant").value) || 0;
  const raisonEcart = document.querySelector("#enc-raison-ecart").value.trim() || null;
  if (montantCalculeEnCours != null && montant !== montantCalculeEnCours && !raisonEcart) {
    toast("Le montant diffère du prix calculé — précisez la raison pour continuer.", "attention");
    return;
  }
  try {
    const resultat = await invoke("finaliser_commande", {
      id: idEncaissementEnCours,
      // Prix non calculable : le montant saisi fait foi, et la comptabilité
      // n'enregistre pas un écart imaginaire contre un prix qui n'a jamais
      // existé.
      montantCalcule: montantCalculeEnCours ?? montant,
      montant,
      raisonEcart,
      moyenPaiement: moyen,
      statut: moyen === "credit" ? "impaye" : "paye",
      employe: document.querySelector("#enc-employe").value || null,
    });
    fermerModal("modal-encaissement");
    retirerFichier(idEncaissementEnCours);
    jouerSonEncaissement();
    // Le reçu n'est plus proposé à chaque encaissement : dans les boutiques
    // visées, l'usage est de ne PAS donner de reçu, et une question posée à
    // chaque vente fait perdre un geste à chaque fois pour une réponse qui
    // est presque toujours "non". Le gérant qui en délivre active l'option
    // une fois dans Réglages, et le reçu sort alors tout seul.
    //
    // Dans les deux cas, il reste imprimable à la demande depuis
    // l'historique : désactiver l'automatisme ne retire aucune possibilité.
    if (await recuAutomatiqueActif()) {
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

/// Le gérant a-t-il demandé qu'un reçu s'imprime après chaque encaissement ?
///
/// Répond non par défaut, y compris si le réglage est illisible : un reçu
/// qui sort sans qu'on l'ait demandé gâche du papier et surprend le gérant,
/// alors qu'un reçu manquant s'imprime en deux clics depuis l'historique.
async function recuAutomatiqueActif() {
  try {
    const params = await invoke("get_boutique_settings");
    return params.recu_automatique === "oui";
  } catch {
    return false;
  }
}

// ───────────────────────────── QR / réception client ─────────────────────────────

async function afficherQr() {
  const conteneur = document.querySelector("#qr-conteneur");
  const urlEl = document.querySelector("#qr-url");

  // La fenêtre s'ouvre AVANT la préparation du QR, pas après.
  //
  // Jusqu'ici elle ne s'affichait qu'une fois tout prêt : entre le clic et
  // l'apparition, l'écran ne bougeait pas d'un pixel. Le gérant croyait que
  // l'application avait planté, et recliquait — ce que personne ne devrait
  // avoir à deviner. On ouvre donc tout de suite, avec de quoi patienter,
  // et le QR remplace l'attente quand il est prêt.
  conteneur.innerHTML = `
    <p class="chargement">
      <span class="chargement-rond" aria-hidden="true"></span>
      Préparation du QR, veuillez patienter…
    </p>`;
  urlEl.textContent = "";
  ouvrirModal("modal-qr");

  try {
    const info = await invoke("get_server_info");
    conteneur.innerHTML = "";
    conteneur.classList.toggle("deux-qr", Boolean(info.qr_page_data_uri));

    // Cette phrase part à l'impression sur l'affiche collée à l'entrée :
    // c'est souvent la seule consigne que le client lira. Elle annonce UN
    // seul geste, parce que c'est le cas normal — le second code n'est qu'un
    // filet, et l'annoncer d'emblée ferait croire qu'il faut deux scans.
    document.querySelector("#qr-intro").textContent = info.qr_page_data_uri
      ? "Avec l'appareil photo de votre téléphone : le 1, puis le 2."
      : "Scannez ce code avec l'appareil photo de votre téléphone";

    // DEUX scans, annoncés comme tels, et de même taille.
    //
    // On a longtemps présenté le second comme un filet discret, en 110 px
    // sous un grand code de 220, en promettant que la page s'ouvrirait
    // toute seule. Sur le terrain, elle ne s'est jamais ouverte toute
    // seule : jamais une fois, sur aucun des deux téléphones d'essai, et le
    // seul chemin qui ait marché passait par les réglages Wi-Fi du
    // téléphone — ce qu'aucun client ne fera.
    //
    // L'ouverture automatique dépend d'un mécanisme que les téléphones
    // appliquent chacun à leur façon, que les fabricants changent sans
    // prévenir, et que des portails d'entreprise bien plus dotés que nous
    // ne maîtrisent pas davantage. Le second QR, lui, ne dépend de rien :
    // il porte l'adresse de la page, le téléphone l'ouvre, c'est tout.
    //
    // Deux gestes qui marchent valent mieux qu'un seul qui échoue, et la
    // promesse faite aux gérants reste tenue : le client ne tape JAMAIS
    // rien. C'est l'ouverture automatique qu'on abandonne, pas elle.
    const ajouterQr = (source, numero, legende) => {
      const bloc = document.createElement("figure");
      bloc.className = "qr-bloc";
      if (numero) {
        const rang = document.createElement("span");
        rang.className = "qr-numero";
        rang.textContent = numero;
        bloc.appendChild(rang);
      }
      const img = document.createElement("img");
      img.src = source;
      img.alt = legende;
      img.width = 180;
      img.height = 180;
      bloc.appendChild(img);
      const texte = document.createElement("figcaption");
      texte.textContent = legende;
      bloc.appendChild(texte);
      conteneur.appendChild(bloc);
      return bloc;
    };

    if (info.qr_page_data_uri) {
      // Les légendes disent le GESTE, pas le concept. « Rejoindre le Wi-Fi »
      // décrit un état ; « appuyez sur Rejoindre » décrit ce que le client
      // doit faire de son pouce — et c'est précisément le moment où il
      // s'arrête, parce que le téléphone lui demande de confirmer.
      ajouterQr(info.qr_data_uri, "1", "Scannez, puis appuyez sur « Rejoindre »");
      ajouterQr(info.qr_page_data_uri, "2", "Scannez pour ouvrir la page d'envoi");
    } else {
      // Pas de point d'accès local : le premier code ouvre déjà la page.
      ajouterQr(info.qr_data_uri, null, "Scannez pour envoyer vos documents");
    }
    // Chaque installation appelle une consigne différente : dire au gérant
    // une phrase qui ne correspond pas à son poste le laisserait sans réponse
    // devant un client bloqué.
    if (info.mode === "point_acces_actif") {
      // Sur iPhone la page s'ouvre seule ; sur Android le système affiche
      // une notification à toucher. L'adresse reste affichée pour que le
      // gérant puisse guider un client dont le téléphone ne réagit pas.
      // Ce que le gérant doit pouvoir dire à un client bloqué, sans
      // réfléchir — et rien de plus, car le reste varie.
      //
      // Une version précédente affirmait ici que « sur iPhone l'appareil
      // photo reste ouvert ». C'était tiré d'un seul essai, fait avec
      // l'application Scanner et non avec l'appareil photo : rien ne dit
      // que tous les iPhone se comportent ainsi. Une consigne fausse dans
      // la main d'un gérant est pire que pas de consigne — il la répète à
      // chaque client et se décrédibilise. Ce qui suit est vrai quel que
      // soit le téléphone et quelle que soit l'application de scan.
      //
      // Le secours tient en une phrase, et j'avais écrit trois lignes de
      // navigation inutile. Le scan du premier code OUVRE DÉJÀ l'écran des
      // réseaux Wi-Fi : le client n'a aucun menu à chercher, le réseau est
      // devant lui, il appuie sur le bouton à côté. Décrire un chemin
      // « Réglages → Wi-Fi → … » envoyait le client ouvrir ce qui était
      // déjà ouvert — la consigne la plus sûre est celle qui décrit le
      // geste, pas l'itinéraire.
      urlEl.innerHTML =
        `<strong>Deux scans, rien à taper.</strong> Le client scanne le <strong>1</strong> et accepte de rejoindre le Wi-Fi, ` +
        `puis il scanne le <strong>2</strong> : la page d'envoi s'ouvre.` +
        `<br><br>Selon le téléphone, l'écran revient ou non à l'appareil photo après le 1. ` +
        `Ce n'est pas un échec : il suffit de rouvrir le scanner et de viser le <strong>2</strong>.` +
        `<br><br><strong>Si le 2 ne donne rien non plus :</strong> le scan a déjà ouvert ` +
        `l'écran des réseaux Wi-Fi. Il suffit d'appuyer sur le bouton à côté du nom du réseau — ` +
        `aucun menu à chercher.`;
    } else if (info.mode === "point_acces_inactif") {
      urlEl.innerHTML =
        `⚠️ Ce QR fait rejoindre le Wi-Fi <strong>${echapperHtml(info.url)}</strong>… mais le Wi-Fi local n'est pas allumé. ` +
        `Appuyez d'abord sur « 📶 Activer le Wi-Fi local de la boutique », sinon le client se connectera à un réseau qui n'existe pas.`;
    } else if (info.mode === "routeur_actif") {
      // Le routeur fait tourner le Wi-Fi lui-même — indépendamment de
      // l'application — donc pas d'avertissement "réseau éteint" ici : ce
      // mode signifie seulement que l'ouverture automatique fonctionne.
      urlEl.innerHTML =
        `<strong>Un seul scan.</strong> Le téléphone rejoint le Wi-Fi du routeur et la page d'envoi s'ouvre ` +
        `toute seule. Le petit code en dessous ne sert que si elle ne s'ouvre pas : rien à taper, jamais.`;
    } else if (info.mode === "routeur_inactif") {
      urlEl.innerHTML =
        `Le grand code fait rejoindre le Wi-Fi, le petit ouvre la page d'envoi : les deux fonctionnent déjà, ` +
        `le client n'a rien à taper. Mais l'ouverture AUTOMATIQUE de la page n'est pas activée : appuyez une ` +
        `fois sur « 📶 Activer le Wi-Fi local de la boutique » pour que le grand code suffise à lui seul.`;
    } else {
      urlEl.innerHTML =
        `Ce QR ouvre directement la page d'envoi (${echapperHtml(info.url)}) dans le navigateur du client. ` +
        `Il faut que son téléphone soit sur le même réseau que ce PC — c'est le cas s'il est connecté au Wi-Fi de la box ou du routeur de la boutique. ` +
        `C'est le parcours le plus simple : un seul scan, rien à taper.`;
    }
  } catch (e) {
    conteneur.innerHTML = `<p class="avertissement">${echapperHtml(e)}</p>`;
  }
}

// Le "Point d'accès mobile" des paramètres Windows refuse de s'activer sans
// connexion internet/Ethernet à partager — précisément le cas normal d'une
// boutique hors ligne (vérifié sur le terrain). Ce bouton crée un point
// d'accès autonome à la place (voir hotspot.rs), sans cette exigence.
/// Affiche le compte rendu d'une activation Wi-Fi sous le bouton, dans une
/// zone que le gérant peut relire et copier.
///
/// Remplace les fenêtres d'alerte utilisées jusqu'ici. La raison vient du
/// terrain : quand l'activation échoue, le texte affiché contient le message
/// EXACT de Windows — la seule information qui permette de réparer. Dans une
/// alerte, il disparaît au premier clic, ne se copie pas, et se retrouve
/// tronqué dès qu'il dépasse quelques lignes.
function afficherCompteRenduWifi(titre, texte, ton) {
  const zone = document.querySelector("#wifi-compte-rendu");
  zone.hidden = false;
  zone.innerHTML = "";

  const entete = document.createElement("p");
  entete.style.cssText = "font-weight:600; margin:0.75rem 0 0.25rem";
  entete.textContent = titre;
  if (ton === "attention") entete.style.color = "var(--rouge, #b00020)";
  zone.appendChild(entete);

  const detail = document.createElement("textarea");
  detail.readOnly = true;
  // Assez haut pour montrer le compte rendu ENTIER sans défiler : sur le
  // terrain, la partie la plus décisive — la liste des programmes à
  // l'écoute — se trouvait sous le bord du cadre, invisible sans le savoir.
  detail.rows = Math.min(26, Math.max(4, texte.split("\n").length + 1));
  detail.style.cssText =
    "width:100%; font-family:Consolas, monospace; font-size:0.75rem; line-height:1.4";
  detail.value = texte;
  zone.appendChild(detail);

  zone.appendChild(
    bouton("Copier ce message", "btn-secondaire", async () => {
      try {
        await navigator.clipboard.writeText(`${titre}\n\n${texte}`);
        toast("✓ Message copié");
      } catch {
        detail.select();
        toast("Sélectionnez et copiez le texte à la main.");
      }
    })
  );
}

document.querySelector("#btn-activer-wifi-local").addEventListener("click", async (e) => {
  const bouton = e.currentTarget;
  bouton.disabled = true;
  const texteInitial = bouton.textContent;
  bouton.textContent = "Activation en cours… (une fenêtre Windows va demander une autorisation)";
  try {
    const resultat = await invoke("activer_point_acces_local");
    const recap = (resultat.recapitulatif || []).join("\n");
    if (resultat.avertissements.length > 0) {
      // Le Wi-Fi lui-même a démarré, mais DHCP et/ou DNS n'ont pas pu
      // s'installer (souvent : Windows fait déjà tourner son propre service
      // sur ce port). Sans ça, les téléphones se connectent au réseau mais
      // n'obtiennent jamais d'adresse ou n'ouvrent jamais la page tout
      // seuls — un souci invisible si on ne le montre pas explicitement ici.
      afficherCompteRenduWifi(
        `⚠️ Wi-Fi local activé (${resultat.methode}), mais avec un problème :`,
        `${recap}\n\n` +
          resultat.avertissements.join("\n\n") +
          `\n\nSi les clients n'arrivent pas à se connecter ou si la page ne s'ouvre pas ` +
          `toute seule, c'est probablement la cause.`,
        "attention"
      );
    } else {
      // Affiché même quand tout va bien : c'est la seule façon de savoir ce
      // qui tourne vraiment, et une photo de cet écran situe une panne sans
      // avoir à tout réessayer à l'aveugle.
      afficherCompteRenduWifi(`✓ Wi-Fi local activé (${resultat.methode})`, recap, "ok");
      toast("✓ Wi-Fi local activé — rafraîchissement du QR…");
    }
    await afficherQr();
  } catch (err) {
    afficherCompteRenduWifi(
      "⚠️ Le Wi-Fi local n'a pas pu être activé.",
      `${err}\n\nSi rien ci-dessus ne débloque la situation, copiez ce message et ` +
        `transmettez-le : il contient le message exact de Windows.`,
      "attention"
    );
  } finally {
    bouton.disabled = false;
    bouton.textContent = texteInitial;
  }
});

document.querySelector("#btn-parametres-partage").addEventListener("click", async () => {
  try {
    await invoke("ouvrir_parametres_partage_connexion");
  } catch (e) {
    alert(`⚠️ Impossible d'ouvrir les paramètres Windows automatiquement. Ouvrez-les vous-même : Paramètres → Réseau et Internet → Partage de connexion.\n\nDétail : ${e}`);
  }
});

// La preuve, au lieu de la déduction. Le gérant connecte un téléphone au
// Wi-Fi, appuie ici, et voit ce que ce PC a RÉELLEMENT reçu de lui.
document.querySelector("#btn-journal-telephones").addEventListener("click", async () => {
  try {
    const lignes = await invoke("journal_des_telephones");
    afficherCompteRenduWifi(
      "📋 Ce que les téléphones ont demandé à ce PC",
      lignes.join("\n"),
      "ok"
    );
  } catch (err) {
    afficherCompteRenduWifi("⚠️ Journal illisible", String(err), "attention");
  }
});

document.querySelector("#btn-imprimer-qr").addEventListener("click", () => {
  imprimerPage("impression-qr");
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
  sectionOuverte = null;
}

const TITRES_SECTION = {
  commandes: "Commandes en cours",
  historique: "Historique",
  recherche: "Documents reçus / Recherche client",
  rapports: "Rapports",
  reglages: "Réglages",
};

// Une phrase en haut de chaque écran : un gérant qui découvre l'application
// ne doit jamais avoir à deviner à quoi sert l'onglet sur lequel il vient
// de cliquer. Court exprès — le détail bouton par bouton est dans l'aide
// (bouton "?" de la barre du haut).
const AIDES_SECTION = {
  commandes: "Les documents reçus qui attendent encore d'être servis. Même liste que l'écran principal, en plus court.",
  historique: "Les commandes déjà servies et encaissées. C'est ici qu'on retrouve une commande d'hier, et qu'on supprime le document d'un client qui le demande.",
  recherche: "Retrouver un document par nom de client, numéro de téléphone ou nom de fichier — même vieux de plusieurs semaines.",
  rapports: "L'argent du jour, les impayés à relancer, le stock, et le rapport imprimable à garder ou à montrer au propriétaire.",
  reglages: "Le nom de la boutique, le dossier surveillé, les tarifs, les employés et la sauvegarde. À régler une fois, rarement retouché ensuite.",
};

// Écran actuellement affiché, pour que le bouton "?" parle du bon écran.
// `null` = l'écran principal (file d'attente), panneau fermé.
let sectionOuverte = null;

async function ouvrirSection(section) {
  document.querySelector("#panneau-nav").hidden = true;
  document.querySelector("#panneau-contenu").hidden = false;
  document.querySelector("#panneau-titre").textContent = TITRES_SECTION[section] ?? "Menu";
  const corps = document.querySelector("#section-corps");
  corps.innerHTML = "Chargement…";
  sectionOuverte = section;

  const rendus = {
    commandes: rendreCommandesEnCours,
    historique: rendreHistorique,
    recherche: rendreRecherche,
    rapports: rendreRapports,
    reglages: rendreReglages,
  };
  await rendus[section]?.(corps);

  // Ajouté APRÈS le rendu : chaque fonction de rendu commence par vider le
  // corps, un bandeau posé avant serait effacé aussitôt.
  const texteAide = AIDES_SECTION[section];
  if (texteAide) {
    const bandeau = document.createElement("p");
    bandeau.className = "aide-section";
    bandeau.textContent = texteAide;
    corps.prepend(bandeau);
  }
}

// ───────────────────────────── Aide intégrée ─────────────────────────────
// Trois niveaux, du plus léger au plus complet : le bandeau d'une phrase en
// haut de chaque onglet (ci-dessus), le bouton "?" qui détaille chaque
// bouton de l'écran en cours, et la visite guidée du premier lancement —
// rejouable, parce que le gérant qui ouvre l'application le premier jour
// n'est pas toujours celui qui s'en servira tous les jours.

const AIDES_ECRAN = {
  accueil: {
    titre: "L'écran principal",
    intro:
      "C'est l'écran de travail de la journée : les documents des clients arrivent ici tout seuls, du haut vers le bas. " +
      "Vous pouvez aussi GLISSER des fichiers directement sur cette fenêtre — pratique quand un client vient avec un câble : " +
      "ouvrez son téléphone dans l'Explorateur, sélectionnez, et lâchez ici.",
    boutons: [
      ["Aperçu", "Regarder le document avant de l'imprimer, sans ouvrir un autre programme."],
      ["Imprimer", "Envoie le document à l'imprimante. Windows ouvre sa fenêtre d'impression habituelle."],
      ["Encaisser", "Enregistre le paiement du client et retire la commande de la liste."],
      ["Détails", "Ce qu'on facture : nombre de pages, couleur, recto-verso, format, finitions."],
      ["Imprimer sur", "Change d'imprimante sans passer par les réglages de Windows. N'apparaît que si le PC en a plusieurs."],
      ["📶 en haut", "Affiche le QR code que le client scanne pour envoyer son document depuis son téléphone."],
      ["⋮ en haut", "Le menu : historique, recherche, rapports et réglages."],
    ],
  },
  commandes: {
    titre: "Commandes en cours",
    intro: "La même liste que l'écran principal, en version courte — pratique pour compter ce qui reste à faire.",
    boutons: [],
  },
  historique: {
    titre: "Historique",
    intro: "Les commandes déjà servies, de la plus récente à la plus ancienne.",
    boutons: [
      ["✓ Impression confirmée", "L'imprimante a confirmé elle-même que le papier est sorti. Sans cette mention, on ne sait pas."],
      ["⚠️ Écart", "Ce qui a été facturé ne correspond pas à ce que l'imprimante a reçu (couleur, recto-verso, format...). À vérifier, jamais bloquant."],
      ["Supprimer le document", "Efface le fichier du client à sa demande. La ligne de comptabilité reste, le document disparaît."],
    ],
  },
  recherche: {
    titre: "Recherche",
    intro: "Tapez au moins 2 lettres : nom du client, numéro de téléphone ou nom de fichier.",
    boutons: [],
  },
  rapports: {
    titre: "Rapports",
    intro: "L'état de la journée, et le rapport à imprimer pour le propriétaire.",
    boutons: [
      ["Réconciliation", "Vérifie qu'aucun fichier reçu n'a disparu sans explication : reçus = payés + ignorés + en attente."],
      ["Impayés à relancer", "Les commandes prises à crédit. « Marquer réglé » quand le client a payé."],
      ["Rapport imprimable", "Sur papier, ou en PDF avec « Microsoft Print to PDF » dans la fenêtre d'impression."],
      ["Stock", "Papier et encre restants. Chiffres à corriger à la main quand vous rachetez."],
    ],
  },
  reglages: {
    titre: "Réglages",
    intro: "À régler une fois à l'installation, rarement retouché ensuite.",
    boutons: [
      ["Dossier surveillé", "Tout fichier déposé dans ce dossier entre automatiquement dans la file d'attente."],
      ["Tarifs", "Le prix de la page noir & blanc, de la couleur, des finitions. Sert à calculer le prix proposé."],
      ["Employés", "Qui a encaissé quoi. Utile dès qu'une deuxième personne tient la caisse."],
      ["Sauvegarde", "Copie de sécurité de toute la comptabilité, automatique toutes les 15 minutes."],
    ],
  },
};

function ouvrirAide() {
  const aide = AIDES_ECRAN[sectionOuverte] ?? AIDES_ECRAN.accueil;
  document.querySelector("#aide-titre").textContent = `Aide — ${aide.titre}`;
  document.querySelector("#aide-intro").textContent = aide.intro;

  const contenu = document.querySelector("#aide-contenu");
  contenu.innerHTML = "";
  for (const [nom, explication] of aide.boutons) {
    const terme = document.createElement("dt");
    terme.textContent = nom;
    const definition = document.createElement("dd");
    definition.textContent = explication;
    contenu.append(terme, definition);
  }

  ouvrirModal("modal-aide");
}

// ───────────────────────────── Visite guidée ─────────────────────────────

const ETAPES_VISITE = [
  {
    titre: "Bienvenue",
    texte:
      "En trois minutes, voici comment se passe une journée normale. Vous pouvez arrêter à tout moment — et revoir cette visite plus tard avec le bouton « ? » en haut de l'écran.",
  },
  {
    titre: "1. Le document arrive tout seul",
    texte:
      "Trois façons : le client scanne le QR code (bouton 📶) et envoie depuis son téléphone ; vous branchez sa clé USB ; ou vous déposez le fichier dans le dossier surveillé. Dans les trois cas, il apparaît dans la liste sans rien faire de plus.",
  },
  {
    titre: "2. Vous imprimez",
    texte:
      "« Aperçu » pour vérifier le document, puis « Imprimer ». L'application surveille ensuite l'imprimante et affiche « ✓ Impression confirmée » quand le papier est vraiment sorti — ou l'erreur si l'imprimante bourre ou n'a plus de papier.",
  },
  {
    titre: "3. Vous encaissez",
    texte:
      "« Détails » pour dire ce qu'on facture (pages, couleur, recto-verso), puis « Encaisser ». Le prix est calculé tout seul à partir de vos tarifs ; vous pouvez toujours le modifier, en disant pourquoi.",
  },
  {
    titre: "4. Le soir",
    texte:
      "Menu ⋮ → Rapports : ce que vous avez encaissé, les impayés à relancer, et le rapport imprimable à garder. Rien ne se perd : chaque fichier reçu est compté dès son arrivée.",
  },
  {
    titre: "Une dernière chose",
    texte:
      "Le bouton « ? » en haut à droite explique les boutons de l'écran sur lequel vous êtes, à tout moment. Vous ne pouvez rien casser en cliquant dessus. Bonne journée de travail.",
  },
];

let etapeVisite = 0;

function afficherEtapeVisite() {
  const etape = ETAPES_VISITE[etapeVisite];
  document.querySelector("#visite-titre").textContent = etape.titre;
  document.querySelector("#visite-compteur").textContent = `Étape ${etapeVisite + 1} sur ${ETAPES_VISITE.length}`;
  document.querySelector("#visite-texte").textContent = etape.texte;
  document.querySelector("#btn-visite-suivant").textContent =
    etapeVisite === ETAPES_VISITE.length - 1 ? "Terminer" : "Suivant";
}

function demarrerVisite() {
  etapeVisite = 0;
  afficherEtapeVisite();
  ouvrirModal("modal-visite");
}

async function terminerVisite() {
  fermerModal("modal-visite");
  // Enregistré une fois pour toutes : un gérant qui a déjà vu la visite ne
  // doit pas la revoir à chaque démarrage. Le bouton "?" permet de la
  // relancer quand il le décide.
  try {
    await invoke("set_boutique_setting", { cle: "visite_guidee_vue", valeur: "oui" });
  } catch {
    // Sans importance : au pire la visite se reproposera au prochain
    // démarrage, ce qui n'empêche personne de travailler.
  }
}

// ──────────────────── Guide du gérant, imprimable ────────────────────
// Réutilise le même mécanisme que le rapport (voir imprimerRapport) : une
// page dédiée, puis l'impression native de Windows. Pensé pour être
// affiché à côté du PC, pour la personne qui tient la caisse.

function imprimerGuideGerant() {
  document.querySelector("#rapport-imprimable").innerHTML = `
    <h1>Guide du gérant — Gestion Photocopie</h1>
    <h2>À garder près de l'ordinateur</h2>

    <h3>Une journée normale, en 3 gestes</h3>
    <table class="table-rapport-imprimable">
      <tbody>
        <tr><td><strong>1. Le document arrive</strong></td><td>Le client scanne le QR code (bouton 📶) et envoie depuis son téléphone,
          ou vous branchez sa clé USB, ou vous déposez le fichier dans le dossier surveillé.
          Il apparaît tout seul dans la liste.</td></tr>
        <tr><td><strong>2. Vous imprimez</strong></td><td>« Aperçu » pour vérifier, puis « Imprimer ».
          L'application affiche « ✓ Impression confirmée » quand le papier est vraiment sorti.</td></tr>
        <tr><td><strong>3. Vous encaissez</strong></td><td>« Détails » pour dire ce qu'on facture, puis « Encaisser ».
          Le prix est calculé tout seul ; il reste modifiable en disant pourquoi.</td></tr>
      </tbody>
    </table>

    <h3>Quand ça ne va pas</h3>
    <table class="table-rapport-imprimable">
      <tbody>
        <tr><td>Le document du client n'apparaît pas</td><td>Vérifiez que son téléphone est bien connecté au Wi-Fi de la boutique,
          puis faites-lui rescanner le QR code.</td></tr>
        <tr><td>« Impression non confirmée »</td><td>Regardez l'imprimante : papier, bourrage, câble, allumée ou non.
          Le message dit lequel de ces problèmes Windows a signalé.</td></tr>
        <tr><td>« PDF protégé par mot de passe »</td><td>Demandez le mot de passe au client : sans lui, personne ne peut imprimer le fichier.</td></tr>
        <tr><td>Un fichier ne doit pas être servi</td><td>« Ignorer », puis choisissez la raison. Rien ne disparaît sans explication.</td></tr>
        <tr><td>L'écran demande une clé d'abonnement</td><td>Appelez le <strong>0151226741</strong> depuis votre téléphone,
          et donnez l'identifiant affiché à l'écran.</td></tr>
      </tbody>
    </table>

    <h3>À faire une fois par semaine</h3>
    <p>Menu ⋮ → Rapports : vérifier les impayés à relancer, et le stock de papier et d'encre.</p>

    <p class="pied-rapport-imprimable">Le bouton « ? » en haut de l'application explique les boutons de chaque écran.</p>
  `;
  imprimerPage("impression-rapport");
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
        ecarts: item.impression_ecarts ? JSON.parse(item.impression_ecarts) : [],
      };
      const texte = texteStatutImpression(dejaConnu);
      if (texte) {
        statutImpression.textContent = texte;
        statutImpression.hidden = false;
        statutImpression.style.color = dejaConnu.erreur ? "var(--rouge-alerte)" : "var(--vert-succes)";
      }

      const texteEcart = texteEcarts(dejaConnu.ecarts);
      if (texteEcart) {
        const ecartsImpression = document.createElement("span");
        ecartsImpression.className = "meta-fichier ecarts-impression";
        ecartsImpression.style.color = "var(--jaune-alerte)";
        ecartsImpression.textContent = texteEcart;
        ligne.appendChild(ecartsImpression);
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
            alert(
              `⚠️ Le document n'a pas pu être supprimé — il est peut-être ouvert dans un autre ` +
                `programme. Fermez-le, puis réessayez.\n\nDétail : ${e}`
            );
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

// ───────────────────── Rapport imprimable (jour/semaine/mois) ─────────────────────
// Utilise l'impression native de Windows (papier, ou "Microsoft Print to
// PDF" comme n'importe quelle autre impression) plutôt qu'une bibliothèque
// de génération PDF — cohérent avec le reste de l'application, qui délègue
// déjà l'impression des documents clients à Windows.

function dateIso(d) {
  const annee = d.getFullYear();
  const mois = String(d.getMonth() + 1).padStart(2, "0");
  const jour = String(d.getDate()).padStart(2, "0");
  return `${annee}-${mois}-${jour}`;
}

function debutDeSemaine(d) {
  // getDay() : 0 = dimanche … 6 = samedi. La semaine commence le lundi.
  const jourSemaine = d.getDay();
  const decalage = jourSemaine === 0 ? 6 : jourSemaine - 1;
  const lundi = new Date(d);
  lundi.setDate(d.getDate() - decalage);
  return lundi;
}

function periodeVersDates(periode) {
  const aujourdhui = new Date();
  const fin = dateIso(aujourdhui);
  if (periode === "semaine") {
    return { debut: dateIso(debutDeSemaine(aujourdhui)), fin };
  }
  if (periode === "mois") {
    return { debut: dateIso(new Date(aujourdhui.getFullYear(), aujourdhui.getMonth(), 1)), fin };
  }
  return { debut: fin, fin }; // "jour"
}

const LIBELLES_PERIODE = { jour: "Aujourd'hui", semaine: "Cette semaine", mois: "Ce mois-ci" };

const LIBELLES_SOURCE = {
  dossier_surveille: "dossier surveillé",
  usb: "clé USB",
  qr: "QR client",
  glisser: "glissé sur la fenêtre",
  bluetooth: "Bluetooth",
};

// Une ligne = une impression réussie ; les erreurs (bourrage, hors ligne...)
// n'apparaissent jamais ici — décision explicite : ce rapport rend compte
// du travail fait, les incidents restent visibles ailleurs, en direct.
function detailsLigneImpression(ligne) {
  return [
    ligne.pages ? `${ligne.pages} page${ligne.pages > 1 ? "s" : ""}` : null,
    ligne.couleur === true ? "Couleur" : ligne.couleur === false ? "Noir & Blanc" : null,
    ligne.recto_verso === true ? "Recto-verso" : ligne.recto_verso === false ? "Recto simple" : null,
    ligne.format_papier,
    `reçu par ${LIBELLES_SOURCE[ligne.source] ?? ligne.source}${
      ligne.attente_minutes != null ? `, attente ${ligne.attente_minutes} min` : ""
    }`,
  ]
    .filter(Boolean)
    .join(" · ");
}

function ligneEcartHtml(ecarts) {
  if (!ecarts?.length) return "";
  const detail = ecarts
    .map((e) => `${LIBELLES_CHAMP_ECART[e.champ] ?? e.champ} : facturé ${e.facture}, imprimé ${e.imprime}`)
    .join(" · ");
  return `<span class="ecart-ligne">⚠️ Écart — ${echapperHtml(detail)}</span>`;
}

async function imprimerRapport(periode) {
  const { debut, fin } = periodeVersDates(periode);
  const [rapport, impressions, boutique] = await Promise.all([
    invoke("rapport_periode", { debut, fin }),
    invoke("rapport_periode_impressions", { debut, fin }),
    invoke("get_boutique_settings"),
  ]);

  const ecartsHtml = rapport.ecarts_par_champ.length
    ? `<table class="table-rapport-imprimable">
        <thead><tr><th>Écart constaté</th><th>Nombre de documents</th></tr></thead>
        <tbody>${rapport.ecarts_par_champ
          .map((e) => `<tr><td>${echapperHtml(LIBELLES_CHAMP_ECART[e.champ] ?? e.champ)}</td><td>${e.nombre}</td></tr>`)
          .join("")}</tbody>
      </table>`
    : "<p>Aucun écart constaté entre facturation et impression réelle sur cette période.</p>";

  const repartitionImprimanteTexte = rapport.repartition_imprimante.length
    ? rapport.repartition_imprimante.map((s) => `${echapperHtml(s.imprimante)} : ${s.nombre}`).join(" · ")
    : null;

  const detailImpressionsHtml = impressions.length
    ? `<table class="table-rapport-imprimable">
        <thead><tr><th>Heure</th><th>Document</th><th>Détails</th><th>Poste → Imprimante</th></tr></thead>
        <tbody>${impressions
          .map(
            (ligne) => `<tr>
              <td class="col-heure">${formatHeure(ligne.termine_le)}</td>
              <td>${echapperHtml(ligne.original_name)}${
                ligne.client_name ? `<span class="nom-client">${echapperHtml(ligne.client_name)}</span>` : ""
              }</td>
              <td>${detailsLigneImpression(ligne)}${ligneEcartHtml(ligne.ecarts)}</td>
              <td>${echapperHtml([ligne.poste_utilisateur, ligne.imprimante].filter(Boolean).join(" → ") || "—")}</td>
            </tr>`
          )
          .join("")}</tbody>
      </table>`
    : "<p>Aucune impression confirmée sur cette période.</p>";

  document.querySelector("#rapport-imprimable").innerHTML = `
    <h1>${echapperHtml(boutique.nom ?? "Gestion Photocopie")}</h1>
    <h2>${LIBELLES_PERIODE[periode]} — du ${rapport.debut} au ${rapport.fin}</h2>
    <table class="table-rapport-imprimable">
      <tbody>
        <tr><td>Commandes réglées</td><td>${rapport.nombre_commandes}</td></tr>
        <tr><td>Total encaissé</td><td>${formatFcfa(rapport.total_encaisse)}</td></tr>
        <tr><td>Impayés</td><td>${formatFcfa(rapport.total_impaye)}</td></tr>
        <tr><td>Dépenses</td><td>${formatFcfa(rapport.total_depenses)}</td></tr>
        <tr><td><strong>Bénéfice net</strong></td><td><strong>${formatFcfa(rapport.benefice_net)}</strong></td></tr>
      </tbody>
    </table>
    <h3>Résumé des impressions</h3>
    <p>${rapport.documents_imprimes_confirmes} document(s) imprimé(s) et confirmé(s) par l'imprimante ·
       ${rapport.documents_avec_ecart} avec un écart par rapport à la facturation.</p>
    <p>Couleur : ${rapport.documents_couleur} · Noir & Blanc : ${rapport.documents_noir_et_blanc}</p>
    <p>Recto-verso : ${rapport.documents_recto_verso} · Recto simple : ${rapport.documents_recto_simple}</p>
    ${repartitionImprimanteTexte ? `<p>Par imprimante : ${repartitionImprimanteTexte}</p>` : ""}
    ${ecartsHtml}
    <h3>Détail des impressions</h3>
    ${detailImpressionsHtml}
    <p class="pied-rapport-imprimable">Généré le ${new Date().toLocaleString("fr-FR")}</p>
  `;
  // Le contenu n'est visible qu'en impression (voir styles.css, règle
  // @media print) — pas besoin d'une fenêtre à part.
  imprimerPage("impression-rapport");
}

function carteRapportImprimable() {
  const carte = document.createElement("div");
  carte.className = "carte-rapport";
  carte.innerHTML = `
    <h3>🖨️ Rapport imprimable</h3>
    <p style="font-size:0.85rem; color:var(--gris-texte-discret); margin-top:0">
      Sur papier, ou en PDF via "Microsoft Print to PDF" dans la fenêtre d'impression de Windows.
    </p>
  `;
  const actions = document.createElement("div");
  actions.className = "actions-rapport-imprimable";
  for (const periode of ["jour", "semaine", "mois"]) {
    actions.appendChild(bouton(LIBELLES_PERIODE[periode], "btn-secondaire", () => imprimerRapport(periode)));
  }
  carte.appendChild(actions);
  return carte;
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
  corps.appendChild(carteRapportImprimable());

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
          // Une dette effacée par erreur ne se retrouve plus dans cette
          // liste : on demande confirmation, avec le nom et le montant sous
          // les yeux.
          const ok = confirm(
            `Confirmer que ${imp.client_name ?? "ce client"} a bien payé ${formatFcfa(imp.montant)} ?\n\n` +
              "La commande quittera la liste des impayés à relancer."
          );
          if (!ok) return;
          try {
            await invoke("marquer_impaye_regle", { transactionId: imp.transaction_id });
            toast("✓ Impayé marqué comme réglé");
            await ouvrirSection("rapports");
          } catch (err) {
            toast(`Cet impayé n'a pas pu être mis à jour : ${err}`, "attention", 6000);
          }
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
    input.addEventListener("change", async () => {
      try {
        await invoke("ajuster_stock", { item: s.item, quantite: Number(input.value) });
        toast(`✓ ${s.libelle} : ${input.value} ${s.unite}`);
      } catch (err) {
        toast(`Ce stock n'a pas été enregistré : ${err}`, "attention", 6000);
      }
    });
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
    try {
      await invoke("ajouter_depense", {
        description: document.querySelector("#dep-description").value,
        montant: Number(document.querySelector("#dep-montant").value) || 0,
        categorie: document.querySelector("#dep-categorie").value,
      });
      toast("✓ Dépense enregistrée");
      await ouvrirSection("rapports");
    } catch (err) {
      toast(`Cette dépense n'a pas été enregistrée : ${err}`, "attention", 6000);
    }
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
    try {
      const r = await invoke("cloturer_caisse", { totalReel });
      resultatCloture.textContent =
        r.ecart === 0
          ? `Tout correspond : ${formatFcfa(r.total_attendu)}.`
          : `Attendu ${formatFcfa(r.total_attendu)}, compté ${formatFcfa(r.total_reel)} — écart de ${formatFcfa(r.ecart)}.`;
    } catch (err) {
      resultatCloture.textContent = `La clôture n'a pas pu être enregistrée : ${err}`;
    }
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
      try {
        await invoke("activer_mode_demo");
        // Sans ce rechargement, les exemples n'étaient visibles qu'après un
        // redémarrage : on annonçait « ✓ Exemples ajoutés » devant une file
        // restée vide — exactement au moment d'une démonstration.
        await chargerFile();
        fermerPanneauMenu();
        toast("✓ Exemples ajoutés à la file d'attente");
      } catch (e) {
        toast(`Les exemples n'ont pas pu être ajoutés : ${e}`, "attention");
      }
    })
  );
  sectionDemo.appendChild(
    bouton("Retirer les exemples de démonstration", "btn-discret", async () => {
      try {
        await invoke("desactiver_mode_demo");
        await chargerFile();
        toast("✓ Exemples retirés");
      } catch (e) {
        toast(`Les exemples n'ont pas pu être retirés : ${e}`, "attention");
      }
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
    <h3>Wi-Fi local (pour le QR)</h3>
    <p style="font-size:0.8rem; color:var(--gris-texte-discret)">
      Deux installations possibles.
    </p>
    <p style="font-size:0.8rem; color:var(--gris-texte-discret)">
      <strong>1. Ce PC crée le réseau.</strong> Le plus simple quand ça
      marche : appuyez sur « 📶 Activer le Wi-Fi local de la boutique » dans
      la fenêtre du QR. Mais toutes les cartes Wi-Fi n'en sont pas capables —
      « Vérifier ce PC », plus bas, le dit en une phrase.
    </p>
    <p style="font-size:0.8rem; color:var(--gris-texte-discret)">
      <strong>2. Le réseau vient d'ailleurs.</strong> Une box, un routeur —
      ou, sans rien acheter, <strong>le partage de connexion d'un
      téléphone</strong> : celui du gérant, laissé allumé et en charge sous
      le comptoir. Ça marche sur n'importe quel PC ayant une carte Wi-Fi,
      même très ancienne, parce que REJOINDRE un réseau est à la portée de
      toutes les cartes — c'est en CRÉER un qui ne l'est pas. Aucune donnée
      mobile n'est nécessaire : le partage crée le réseau même sans internet.
      <br />
      À faire une seule fois : allumez le partage de connexion du téléphone
      et donnez-lui un nom et un mot de passe simples ; connectez ce PC à ce
      réseau (en cochant « se connecter automatiquement ») ; recopiez ce nom
      et ce mot de passe ci-dessous. Le client scanne alors le QR 1 pour
      rejoindre le réseau, puis le QR 2 pour ouvrir la page — sans jamais
      rien taper, et un seul scan dès sa deuxième visite.
      <br />
      Si vous imprimez l'affiche : réimprimez-la si un jour la page cesse de
      s'ouvrir, car l'adresse du PC sur ce réseau peut changer. Rouvrez la
      fenêtre du QR pour voir l'affiche à jour.
    </p>
  `;
  const formWifi = document.createElement("form");
  formWifi.innerHTML = `
    <label>Type de réseau
      <select id="reg-wifi-type">
        <option value="pc" ${params.wifi_type_reseau === "routeur_externe" ? "" : "selected"}>Ce PC crée le réseau</option>
        <option value="routeur_externe" ${params.wifi_type_reseau === "routeur_externe" ? "selected" : ""}>Le réseau vient d'ailleurs (box, routeur, ou partage de connexion d'un téléphone)</option>
      </select>
    </label>
    <label>Nom du réseau (SSID) <input type="text" id="reg-wifi-ssid" value="${echapperHtml(params.wifi_ssid)}" /></label>
    <label>Mot de passe <input type="text" id="reg-wifi-mdp" value="${echapperHtml(params.wifi_mot_de_passe)}" /></label>
    <button type="submit" class="btn-secondaire">Enregistrer</button>
  `;
  formWifi.addEventListener("submit", async (e) => {
    e.preventDefault();
    await invoke("set_boutique_setting", { cle: "wifi_type_reseau", valeur: document.querySelector("#reg-wifi-type").value });
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

  // Reçus : désactivés d'office. Dans les boutiques visées, l'usage est de
  // ne pas en délivrer — poser la question à chaque encaissement coûtait un
  // geste à chaque vente pour une réponse presque toujours négative.
  const secRecus = document.createElement("section");
  secRecus.innerHTML = `
    <h3>Reçus</h3>
    <p style="font-size:0.8rem; color:var(--gris-texte-discret)">
      Par défaut, aucun reçu n'est imprimé après un encaissement. Si vous en
      délivrez, cochez la case : le reçu sortira alors tout seul à chaque
      encaissement. Dans les deux cas, vous pouvez toujours en imprimer un à
      la demande depuis l'historique.
    </p>
  `;
  const formRecus = document.createElement("form");
  formRecus.innerHTML = `
    <label style="display:flex; align-items:center; gap:0.5rem">
      <input type="checkbox" id="reg-recu-auto" ${params.recu_automatique === "oui" ? "checked" : ""} />
      Imprimer un reçu après chaque encaissement
    </label>
    <button type="submit" class="btn-secondaire">Enregistrer</button>
  `;
  formRecus.addEventListener("submit", async (e) => {
    e.preventDefault();
    try {
      await invoke("set_boutique_setting", {
        cle: "recu_automatique",
        valeur: document.querySelector("#reg-recu-auto").checked ? "oui" : "non",
      });
      toast("✓ Réglage des reçus enregistré");
    } catch (err) {
      toast(String(err), "attention");
    }
  });
  secRecus.appendChild(formRecus);
  corps.appendChild(secRecus);

  // « Je ne sens rien » : sans moyen d'essayer, impossible de distinguer
  // une application muette d'un haut-parleur coupé. Ce bouton tranche en
  // une seconde, et dit ce qu'il a trouvé.
  const secSons = document.createElement("section");
  secSons.innerHTML = `
    <h3>Sons</h3>
    <p style="font-size:0.8rem; color:var(--gris-texte-discret)">
      L'application émet trois sons : un carillon à l'arrivée d'un document,
      un petit « tic » à l'envoi d'une impression, et deux notes à
      l'encaissement. Appuyez pour les entendre.
    </p>
  `;
  secSons.appendChild(
    bouton("🔔 Tester les sons", "btn-secondaire", async () => {
      const ctx = contexteAudio();
      if (!ctx) {
        toast("Ce PC ne fournit aucune sortie audio à l'application.", "attention");
        return;
      }
      if (ctx.state !== "running") {
        toast("Le son est encore endormi — réappuyez une fois.", "attention");
        return;
      }
      jouerNotification();
      setTimeout(jouerSonImpression, 900);
      setTimeout(jouerSonEncaissement, 1400);
      toast("🔔 Trois sons joués — si vous n'entendez rien, vérifiez le volume de Windows");
    })
  );
  corps.appendChild(secSons);

  // Répond à la seule question qui se pose en installant l'application sur
  // un poste inconnu : sur CE PC, comment les clients envoient-ils leurs
  // fichiers ? Sans rien activer, sans autorisation Windows, en un clic.
  const secPoste = document.createElement("section");
  secPoste.innerHTML = `
    <h3>Vérifier ce PC</h3>
    <p style="font-size:0.8rem; color:var(--gris-texte-discret)">
      Indique en une phrase ce qu'il faut faire sur ce poste pour recevoir les
      fichiers des clients. À lancer dès l'installation, avant le premier client.
    </p>
  `;
  secPoste.appendChild(
    bouton("Vérifier ce PC", "btn-secondaire", async () => {
      const diag = await invoke("diagnostiquer_poste");

      const verdict = document.createElement("p");
      verdict.style.cssText =
        "font-size:0.95rem; line-height:1.5; padding:0.75rem; border-radius:6px; background:var(--gris-clair); margin:0.5rem 0";
      verdict.textContent = diag.verdict;
      secPoste.appendChild(verdict);

      // Le verdict automatique peut rester indécis : la sortie brute de
      // Windows, elle, fait foi. On la garde accessible pour le support,
      // sans l'imposer à l'écran du gérant.
      if (diag.details_bruts) {
        const details = document.createElement("details");
        details.innerHTML = `<summary style="cursor:pointer; font-size:0.8rem">Détails techniques (à envoyer en cas de problème)</summary>`;
        const zone = document.createElement("textarea");
        zone.readOnly = true;
        zone.rows = 12;
        zone.style.cssText = "width:100%; font-family:Consolas, monospace; font-size:0.75rem";
        zone.value = diag.details_bruts;
        details.appendChild(zone);
        details.appendChild(
          bouton("Copier les détails", "btn-secondaire", async () => {
            try {
              await navigator.clipboard.writeText(diag.details_bruts);
              toast("✓ Détails copiés");
            } catch {
              zone.select();
              toast("Sélectionnez et copiez le texte à la main.");
            }
          })
        );
        secPoste.appendChild(details);
      }
    })
  );
  corps.appendChild(secPoste);

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

    // Bouton explicite plutôt que l'événement "change" du champ : celui-ci se
    // déclenche à CHAQUE clic sur les flèches ↑↓ du champ numérique, pas
    // seulement une fois le prix choisi — un gérant qui ajustait un prix en
    // cliquant plusieurs fois enregistrait chaque valeur intermédiaire (et
    // remplissait l'historique de prix qu'il n'avait jamais voulu valider).
    const btnValider = document.createElement("button");
    btnValider.type = "button";
    btnValider.className = "btn-secondaire";
    btnValider.textContent = "Valider";
    btnValider.disabled = true;

    input.addEventListener("input", () => {
      btnValider.disabled = Number(input.value) === t.prix_unitaire;
    });

    btnValider.addEventListener("click", async () => {
      try {
        await invoke("update_tarif", { id: t.id, prixUnitaire: Number(input.value) });
        toast(`✓ ${t.libelle} : ${formatFcfa(Number(input.value))}`);
        t.prix_unitaire = Number(input.value);
        btnValider.disabled = true;
      } catch (err) {
        toast(`Ce tarif n'a pas été enregistré : ${err}`, "attention", 6000);
      }
    });

    ligne.append(`${t.libelle} (par ${t.unite}) : `, input, btnValider);
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
    try {
      const ok = await invoke("set_license_key", { cle: document.querySelector("#cle-licence").value });
      if (ok) {
        await ouvrirSection("reglages");
        await rafraichirBadgeAbonnement();
        await verifierBlocageLicence();
        toast("✓ Licence activée — merci !");
      } else {
        toast("Cette clé n'est pas reconnue. Vérifiez qu'elle est copiée en entier, sans espace avant ni après.", "attention");
      }
    } catch (err) {
      // Sans ce filet, une erreur d'enregistrement ne provoquait AUCUNE
      // réaction à l'écran : le gérant cliquait « Activer » et ne voyait
      // rien du tout — au moment précis où il vient de payer.
      toast(`La clé n'a pas pu être enregistrée : ${err}`, "attention", 6000);
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
              // Rassurer d'abord : une restauration qui échoue ne touche
              // pas aux données en service (la sauvegarde est vérifiée
              // avant, voir backup.rs). Sans cette phrase, un gérant croit
              // qu'il vient de tout perdre.
              alert(
                `⚠️ La restauration n'a pas eu lieu — vos données actuelles sont intactes.\n\n` +
                  `Essayez une sauvegarde plus ancienne dans la liste. Si aucune ne fonctionne, ` +
                  `appelez le 0151226741 avant de toucher à autre chose.\n\nDétail : ${err}`
              );
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
    // Enchaîne sur la visite guidée : la configuration est faite, le gérant
    // a maintenant besoin de savoir comment travailler avec.
    demarrerVisite();
  });
  return true;
}

// ───────────────────────────── Démarrage ─────────────────────────────

/// Tout ce qui suppose une installation déjà validée (voir
/// #overlay-installation) : charger la file, proposer l'assistant, écouter
/// les événements... Regroupé pour ne s'exécuter ni avant la validation du
/// code d'installation, ni deux fois si un premier essai avait échoué.
async function demarrerApplication() {
  const premierLancement = await lancerAssistantPremierDemarrage();
  await chargerFile();
  await chargerImprimantes();
  await rafraichirBadgeAbonnement();
  await verifierBlocageLicence();
  // Jamais au tout premier lancement : ce gérant n'a encore rien "gagné",
  // il découvre juste l'application pour la première fois.
  if (!premierLancement) {
    await afficherNouveautesSiBesoin();
    // Les boutiques déjà installées avant l'arrivée de la visite guidée ne
    // l'ont jamais vue : on la propose une fois, puis plus jamais.
    const params = await invoke("get_boutique_settings");
    if (!params.visite_guidee_vue) {
      demarrerVisite();
    }
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
      toast(
        "Le fichier licence.txt de cette clé USB ne correspond pas à cet ordinateur. " +
          "Appelez le 0151226741 en donnant l'identifiant affiché à l'écran.",
        "attention"
      );
    }
  });
}

window.addEventListener("DOMContentLoaded", async () => {
  document.querySelector("#btn-menu").addEventListener("click", ouvrirPanneauMenu);
  document.querySelector("#btn-fermer-menu").addEventListener("click", fermerPanneauMenu);
  document.querySelector("#btn-aide").addEventListener("click", ouvrirAide);
  document.querySelector("#btn-revoir-visite").addEventListener("click", () => {
    fermerModal("modal-aide");
    demarrerVisite();
  });
  document.querySelector("#btn-imprimer-guide").addEventListener("click", () => {
    fermerModal("modal-aide");
    imprimerGuideGerant();
  });
  document.querySelector("#btn-visite-passer").addEventListener("click", terminerVisite);
  document.querySelector("#btn-visite-suivant").addEventListener("click", () => {
    if (etapeVisite >= ETAPES_VISITE.length - 1) {
      terminerVisite();
      return;
    }
    etapeVisite++;
    afficherEtapeVisite();
  });

  document.querySelector("#btn-retour-nav").addEventListener("click", () => {
    sectionOuverte = null;
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
    try {
      const ok = await invoke("set_license_key", { cle: champ.value });
      if (ok) {
        champ.value = "";
        toast("✓ Licence activée — merci !");
        await verifierBlocageLicence();
        await rafraichirBadgeAbonnement();
      } else {
        toast("Cette clé n'est pas reconnue. Vérifiez qu'elle est copiée en entier, sans espace avant ni après.", "attention");
      }
    } catch (err) {
      // Écran de blocage : c'est le seul endroit où le gérant peut encore
      // agir. Une erreur silencieuse ici le laisse devant un bouton qui ne
      // répond pas, sans savoir s'il doit rappeler ou réessayer.
      toast(
        `La clé n'a pas pu être enregistrée (${err}). Réessayez, puis appelez le 0151226741 si cela persiste.`,
        "attention",
        7000
      );
    }
  });
  document.querySelector("#btn-fermer-nouveautes").addEventListener("click", () => {
    document.querySelector("#overlay-nouveautes").hidden = true;
  });

  // Tout premier lancement : rien d'autre ne démarre tant que ce code n'est
  // pas validé (voir license.rs) — c'est ce qui garantit que le porteur du
  // projet est au courant de CETTE installation avant qu'elle serve.
  document.querySelector("#form-code-installation").addEventListener("submit", async (e) => {
    e.preventDefault();
    const champ = document.querySelector("#code-installation");
    try {
      const ok = await invoke("valider_code_installation", { code: champ.value });
      if (ok) {
        document.querySelector("#overlay-installation").hidden = true;
        toast("✓ Installation validée — merci !");
        await demarrerApplication();
      } else {
        toast(
          "Ce code n'est pas reconnu. Vérifiez qu'il est copié en entier, sans espace avant ni après.",
          "attention"
        );
      }
    } catch (err) {
      // Écran de blocage : c'est le seul endroit où l'on peut encore agir.
      toast(
        `Le code n'a pas pu être vérifié (${err}). Réessayez, puis appelez le 0151226741 si cela persiste.`,
        "attention",
        7000
      );
    }
  });

  if (await invoke("code_installation_deja_valide")) {
    await demarrerApplication();
  } else {
    const licence = await invoke("get_license_status");
    document.querySelector("#machine-id-installation").textContent = licence.machine_id;
    document.querySelector("#overlay-installation").hidden = false;
  }
});
