const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const listeEl = document.querySelector("#liste-fichiers");
const etatVideEl = document.querySelector("#etat-vide");
const statutDossierEl = document.querySelector("#statut-dossier");
const cheminDossierEl = document.querySelector("#chemin-dossier");
const tplLigne = document.querySelector("#tpl-ligne-fichier");

const LIBELLES_KIND = {
  imprimable: "Prêt à imprimer",
  editable: "À ouvrir/éditer",
  inconnu: "Format non reconnu",
};

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

function jouerNotification() {
  try {
    const ctx = new (window.AudioContext || window.webkitAudioContext)();
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.frequency.value = 880;
    gain.gain.setValueAtTime(0.08, ctx.currentTime);
    gain.gain.exponentialRampToValueAtTime(0.001, ctx.currentTime + 0.4);
    osc.connect(gain).connect(ctx.destination);
    osc.start();
    osc.stop(ctx.currentTime + 0.4);
  } catch {
    // Pas grave si le son ne peut pas jouer (ex: pas d'interaction utilisateur encore).
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
  ]
    .filter(Boolean)
    .join(" · ");
  noeud.querySelector(".meta-fichier").textContent = meta;

  const actions = noeud.querySelector(".actions-fichier");

  if (item.kind === "imprimable") {
    actions.appendChild(bouton("Imprimer", "btn-primaire", () => imprimer(item.id)));
  } else if (item.kind === "editable") {
    actions.appendChild(bouton("Ouvrir/Éditer", "btn-primaire", () => ouvrir(item.id)));
  } else {
    const avert = document.createElement("span");
    avert.className = "avertissement";
    avert.textContent = "Format non supporté — redemander un format standard au client";
    actions.appendChild(avert);
  }

  actions.appendChild(
    bouton("Marquer traité", "btn-discret", () => marquerTraite(item.id))
  );

  return noeud;
}

function bouton(label, classe, onClick) {
  const b = document.createElement("button");
  b.textContent = label;
  b.className = classe;
  b.addEventListener("click", onClick);
  return b;
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
}

async function chargerFile() {
  const items = await invoke("get_queue");
  listeEl.innerHTML = "";
  if (!items.length) {
    etatVideEl.hidden = false;
    return;
  }
  etatVideEl.hidden = true;
  for (const item of items) ajouterFichier(item);
}

async function imprimer(id) {
  try {
    await invoke("print_file", { id });
  } catch (e) {
    alert(`Impossible d'imprimer ce fichier : ${e}`);
  }
}

async function ouvrir(id) {
  try {
    await invoke("open_file", { id });
  } catch (e) {
    alert(`Impossible d'ouvrir ce fichier : ${e}`);
  }
}

async function marquerTraite(id) {
  try {
    await invoke("mark_processed", { id });
    retirerFichier(id);
  } catch (e) {
    alert(`Impossible de mettre à jour le fichier : ${e}`);
  }
}

async function rafraichirStatutDossier() {
  const dossier = await invoke("get_watched_folder");
  if (dossier) {
    statutDossierEl.textContent = `Dossier surveillé : ${dossier}`;
    cheminDossierEl.textContent = dossier;
  } else {
    statutDossierEl.textContent = "Dossier surveillé : non configuré";
    cheminDossierEl.textContent = "Aucun dossier configuré";
  }
}

function ouvrirPanneauMenu() {
  document.querySelector("#panneau-menu").hidden = false;
}
function fermerPanneauMenu() {
  document.querySelector("#panneau-menu").hidden = true;
}

async function choisirDossier() {
  const dossier = await invoke("choose_watched_folder");
  if (dossier) await rafraichirStatutDossier();
}

window.addEventListener("DOMContentLoaded", async () => {
  document.querySelector("#btn-menu").addEventListener("click", ouvrirPanneauMenu);
  document.querySelector("#btn-fermer-menu").addEventListener("click", fermerPanneauMenu);
  document.querySelector("#btn-choisir-dossier").addEventListener("click", choisirDossier);

  await chargerFile();
  await rafraichirStatutDossier();

  await listen("nouveau-fichier", (event) => {
    ajouterFichier(event.payload, true);
    jouerNotification();
  });
});
