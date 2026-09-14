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

function bouton(label, classe, onClick) {
  const b = document.createElement("button");
  b.type = "button";
  b.textContent = label;
  b.className = classe;
  b.addEventListener("click", onClick);
  return b;
}

function ouvrirModal(id) {
  document.querySelector(`#${id}`).hidden = false;
}
function fermerModal(id) {
  document.querySelector(`#${id}`).hidden = true;
}

// ───────────────────────────── File d'attente ─────────────────────────────

function creerLigne(item) {
  const noeud = tplLigne.content.cloneNode(true);
  const li = noeud.querySelector(".ligne-fichier");
  li.dataset.id = item.id;

  noeud.querySelector(".nom-fichier").textContent = item.original_name;
  const meta = [
    item.client_name ? `Client : ${item.client_name}` : null,
    formatHeure(item.received_at),
    LIBELLES_KIND[item.kind] ?? item.kind,
    item.copies > 1 ? `${item.copies} copies` : null,
    item.finitions?.length ? item.finitions.length + " finition(s)" : null,
  ]
    .filter(Boolean)
    .join(" · ");
  noeud.querySelector(".meta-fichier").textContent = meta;

  const actions = noeud.querySelector(".actions-fichier");

  if (item.kind === "imprimable") {
    actions.appendChild(bouton("Options", "btn-discret", () => ouvrirOptions(item)));
    actions.appendChild(bouton("Imprimer", "btn-primaire", () => imprimer(item.id)));
    actions.appendChild(bouton("Encaisser", "btn-secondaire", () => ouvrirEncaissement(item)));
  } else if (item.kind === "editable") {
    actions.appendChild(bouton("Options", "btn-discret", () => ouvrirOptions(item)));
    actions.appendChild(bouton("Ouvrir/Éditer", "btn-primaire", () => ouvrir(item.id)));
    actions.appendChild(bouton("Encaisser", "btn-secondaire", () => ouvrirEncaissement(item)));
  } else if (item.kind === "installateur") {
    actions.appendChild(
      bouton("Installer la mise à jour", "btn-primaire", () => ouvrir(item.id))
    );
    actions.appendChild(bouton("Ignorer", "btn-discret", () => ignorer(item.id)));
  } else {
    const avert = document.createElement("span");
    avert.className = "avertissement";
    avert.textContent = "Format non supporté — redemander un format standard au client";
    actions.appendChild(avert);
    actions.appendChild(bouton("Ignorer", "btn-discret", () => ignorer(item.id)));
  }

  if (item.kind === "imprimable") {
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

async function ignorer(id) {
  try {
    await invoke("ignorer_fichier", { id });
    retirerFichier(id);
  } catch (e) {
    alert(`Impossible de mettre à jour le fichier : ${e}`);
  }
}

// ───────────────────────────── Options d'impression ─────────────────────────────

function ouvrirOptions(item) {
  idOptionsEnCours = item.id;
  document.querySelector("#opt-copies").value = item.copies ?? 1;
  document.querySelector("#opt-couleur").checked = !!item.couleur;
  document.querySelector("#opt-format").value = item.format_papier ?? "A4";
  document.querySelector("#opt-recto-verso").checked = !!item.recto_verso;
  document.querySelector("#opt-orientation").value = item.orientation ?? "portrait";

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
  try {
    await invoke("set_print_options", {
      id: idOptionsEnCours,
      copies: Number(document.querySelector("#opt-copies").value) || 1,
      couleur: document.querySelector("#opt-couleur").checked,
      formatPapier: document.querySelector("#opt-format").value,
      rectoVerso: document.querySelector("#opt-recto-verso").checked,
      orientation: document.querySelector("#opt-orientation").value,
      finitions,
    });
    fermerModal("modal-options");
    await chargerFile();
  } catch (err) {
    alert(`Impossible d'enregistrer les options : ${err}`);
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

async function ouvrirEncaissement(item) {
  idEncaissementEnCours = item.id;
  try {
    const prix = await invoke("calculer_prix", { id: item.id });
    document.querySelector("#enc-montant").value = prix;
  } catch {
    document.querySelector("#enc-montant").value = 0;
  }
  await remplirEmployes(document.querySelector("#enc-employe"));
  ouvrirModal("modal-encaissement");
}

document.querySelector("#form-encaissement").addEventListener("submit", async (e) => {
  e.preventDefault();
  const moyen = document.querySelector("#enc-moyen").value;
  try {
    await invoke("finaliser_commande", {
      id: idEncaissementEnCours,
      montant: Number(document.querySelector("#enc-montant").value) || 0,
      moyenPaiement: moyen,
      statut: moyen === "credit" ? "impaye" : "paye",
      employe: document.querySelector("#enc-employe").value || null,
    });
    fermerModal("modal-encaissement");
    retirerFichier(idEncaissementEnCours);
  } catch (err) {
    alert(`Impossible d'encaisser : ${err}`);
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
    urlEl.textContent = info.url;
  } catch (e) {
    conteneur.innerHTML = `<p class="avertissement">${e}</p>`;
  }
  ouvrirModal("modal-qr");
}

document.querySelector("#btn-parametres-partage").addEventListener("click", async () => {
  try {
    await invoke("ouvrir_parametres_partage_connexion");
  } catch (e) {
    alert(e);
  }
});

// ───────────────────────────── Panneau latéral ─────────────────────────────

function ouvrirPanneauMenu() {
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
    corps.appendChild(
      ligneListe(
        item.original_name,
        [item.client_name, formatHeure(item.received_at), item.prix ? formatFcfa(item.prix) : null]
          .filter(Boolean)
          .join(" · ")
      )
    );
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
}

async function rendreReglages(corps) {
  corps.innerHTML = "";

  // Dossier surveillé
  const dossier = await invoke("get_watched_folder");
  const secDossier = document.createElement("section");
  secDossier.innerHTML = `<h3>Dossier surveillé</h3><p class="chemin-dossier">${dossier ?? "Aucun dossier configuré"}</p>`;
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
    <label>Nom de la boutique <input type="text" id="reg-nom" value="${params.nom ?? ""}" /></label>
    <label>Numéro WhatsApp <input type="text" id="reg-whatsapp" value="${params.whatsapp ?? ""}" /></label>
    <label>Dossier de sauvegarde <input type="text" id="reg-sauvegarde" value="${params.dossier_sauvegarde ?? ""}" /></label>
    <label>URL de vérification des mises à jour <input type="text" id="reg-url-maj" value="${params.url_verification_maj ?? ""}" /></label>
    <button type="submit" class="btn-secondaire">Enregistrer</button>
  `;
  formBoutique.addEventListener("submit", async (e) => {
    e.preventDefault();
    await invoke("set_boutique_setting", { cle: "boutique_nom", valeur: document.querySelector("#reg-nom").value });
    await invoke("set_boutique_setting", { cle: "boutique_whatsapp", valeur: document.querySelector("#reg-whatsapp").value });
    await invoke("set_boutique_setting", { cle: "dossier_sauvegarde", valeur: document.querySelector("#reg-sauvegarde").value });
    await invoke("set_boutique_setting", { cle: "url_verification_maj", valeur: document.querySelector("#reg-url-maj").value });
    alert("Réglages enregistrés.");
  });
  secBoutique.appendChild(formBoutique);
  corps.appendChild(secBoutique);

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
      alert("Licence activée.");
      await ouvrirSection("reglages");
      await rafraichirBadgeAbonnement();
    } else {
      alert("Clé invalide.");
    }
  });
  secLicence.appendChild(formLicence);
  corps.appendChild(secLicence);

  const version = await invoke("version_actuelle");
  const versionEl = document.createElement("p");
  versionEl.className = "version";
  versionEl.textContent = `Version ${version}`;
  corps.appendChild(versionEl);
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
    badge.className = "badge-abonnement badge-" + licence.statut;
    badge.hidden = false;
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

  await chargerFile();
  await rafraichirBadgeAbonnement();
  await verifierMiseAJour();

  await listen("nouveau-fichier", (event) => {
    ajouterFichier(event.payload, true);
    jouerNotification();
  });
});
