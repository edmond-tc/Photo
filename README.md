# Gestion Photocopie — Bénin

Logiciel de bureau (Windows, [Tauri](https://tauri.app)) pour les gérants de
boutiques de photocopie au Bénin. 100% fonctionnel hors ligne : réception de
fichiers (dossier surveillé, clé USB, QR/Wi-Fi local), file d'attente unique,
impression/édition via les outils déjà installés sur le PC (dialogue
d'impression Windows natif, Word/LibreOffice), gestion financière et licence.

Le cahier des charges complet, validé avec le porteur du projet, est dans
[`docs/cahier-des-charges.md`](docs/cahier-des-charges.md).

## Récupérer le .exe (sans installer Rust)

Chaque envoi de code sur `main` déclenche une compilation automatique sur un
runner Windows via GitHub Actions (onglet **Actions** du dépôt → dernier
run de *Build Windows* → artifact `photocopie-benin-windows`). C'est la
méthode prévue pour tester sur un vrai PC Windows sans avoir besoin
d'installer la chaîne de compilation Rust localement.

## Développement local (Linux/macOS/Windows)

```bash
npm install
npm run tauri dev
```

Prérequis Tauri (WebKitGTK sur Linux, etc.) : voir
https://tauri.app/start/prerequisites/. Sur Ubuntu 22.04, `webkit2gtk-4.1`
n'existe pas dans les dépôts officiels (limitation connue de Tauri v2) —
voir la section GitHub Codespaces ci-dessous pour tester sans ce problème.

### Tester dans le navigateur, sans rien installer (GitHub Codespaces)

Le dépôt inclut une configuration `.devcontainer/` prête à l'emploi, avec
un bureau graphique accessible dans un onglet du navigateur :

1. Sur GitHub, page du dépôt → bouton **Code** → onglet **Codespaces** →
   **Create codespace on main**.
2. Une fois prêt, ouvrir l'onglet **Ports** en bas de l'éditeur, trouver le
   port **6080** et cliquer sur l'icône du globe pour l'ouvrir dans un
   nouvel onglet — un bureau Linux apparaît (mot de passe : `photocopie`).
3. Dans le terminal du Codespace : `npm run tauri dev`.
4. La fenêtre de l'application s'ouvre **dans l'onglet du bureau
   graphique** (pas dans l'éditeur) — il faut y basculer pour la voir.

Comme sur toute machine Linux, les boutons *Imprimer*/*Ouvrir* afficheront
une erreur (spécifiques à Windows) — tout le reste est testable
normalement.

Vérifications rapides avant de pousser du code Rust :

```bash
cd src-tauri
cargo check
cargo clippy --all-targets --bins
cargo fmt
```

## Générer une clé de licence (porteur du projet)

Le gérant lit son identifiant machine dans Réglages > Licence et vous le
communique. Pour générer sa clé sur le terrain **sans recompiler** (rapide,
comme pour récupérer le `.exe` principal) : onglet **Actions** du dépôt →
dernier run de *Build Windows* → artifact `generer-licence-windows` →
`generer-licence.exe`. À garder sur votre PC une fois téléchargé, réutilisable
pour tous les gérants sans avoir à le retélécharger.

```
generer-licence.exe <ID_MACHINE> [jours_valables=30]
```

(Alternative pour du développement local : `cd src-tauri && cargo run
--release --bin generer-licence -- <ID_MACHINE> [jours_valables=30]` — plus
lent la première fois, le temps de compiler.)

Le gérant colle la clé reçue dans Réglages > Licence pour activer ou
renouveler son abonnement. Le secret de signature (constante `SECRET` dans
`src-tauri/src/license.rs`) a été changé pour une valeur aléatoire avant le
début de la distribution réelle (mécanisme léger décrit en section 7 du
cahier des charges — pas une vérification serveur, mais empêche la
génération de clés sans connaître ce secret).

## État d'avancement

Toutes les étapes du plan de développement (section 11 du cahier des
charges) ont une première implémentation :

- **Étape 1** — Squelette Tauri + SQLite (WAL) + écran unique, réception par
  dossier surveillé, routage impression/édition, style Office.
- **Étape 2** — Aperçu (vignette pour les images ; pour le reste, l'aperçu
  avant impression est déjà fourni par l'application native qui gère le
  clic *Imprimer* — voir "Limites connues" ci-dessous).
- **Réception clé USB** : détection des disques amovibles, scan automatique.
- **Étape 3** — Serveur HTTP local (page de réception mobile-friendly,
  sélection de plusieurs fichiers avec options par fichier — N&B/couleur,
  format, copies, plage de pages — pour que le gérant n'ait qu'à confirmer),
  **un seul QR code** (retour explicite du porteur du projet : pas deux
  codes) qui encode directement les identifiants Wi-Fi (format `WIFI:`
  reconnu nativement par les appareils photo Android/iPhone) — le client
  scanne, rejoint le réseau local en un geste (aucune saisie), puis la page
  d'envoi s'ouvre. Réseau 100% local, sans internet — comme un talkie-walkie
  entre le téléphone et le PC, rien ne sort de la boutique. Le QR est
  imprimable (bouton dédié) pour être collé à l'entrée, et reste valable
  tant que le mot de passe Wi-Fi ne change pas. Le partage de connexion
  Wi-Fi (Mobile Hotspot) lui-même s'active via un raccourci vers les
  paramètres Windows plutôt qu'une automatisation WinRT non testable ici.
- **Étape 4** — L'impression elle-même passe *toujours* directement par la
  boîte de dialogue Windows native (bouton *Imprimer*), qui gère déjà
  copies/couleur/recto-verso/format — pas de double saisie dans l'appli.
  L'écran « Détails » ne sert qu'à ce que Windows ne peut pas savoir : les
  finitions (agrafage, reliure...) et ce qu'il faut facturer, pour le
  calcul de prix. **Aperçu intégré** (bouton *Aperçu*, PDF/image) avant
  d'imprimer : le document s'affiche directement dans l'application (PDF
  via le moteur de rendu intégré de WebView2, image directement), avec le
  bouton *Imprimer* juste en dessous — aucune autre application à ouvrir
  entre les deux, pas de va-et-vient.
- Diagnostics à la réception d'un fichier : PDF probablement protégé par
  mot de passe, format détecté différent de A4 (ex. US Letter), fichier
  volumineux — affichés comme avertissements, jamais bloquants (cf.
  "Limites connues").
- **Étape 5** — Suivi financier (encaissement espèces/Mobile Money/crédit),
  reçu imprimable après chaque encaissement (texte simple, ou HTML avec
  logo si configuré), stock papier/toner avec alerte, dépenses, employés,
  rapport du jour, export CSV des transactions (avec BOM UTF-8 pour un
  affichage correct des accents dans Excel), clôture de caisse (écart
  espèces attendu/compté), rappel des impayés, remise fidélité automatique
  (10% à partir de 5 visites payées du même numéro de téléphone), compteur
  d'entretien imprimante (alerte au-delà d'un seuil de feuilles), mode
  démonstration (exemples factices pour un agent terrain sans client réel).
- **Étape 6** — Menu latéral (⋮) avec Commandes en cours / Historique /
  Recherche client / Rapports / Réglages, dans l'esprit Claude.ai décrit au
  cahier des charges.
- **Étape 7** — Licence liée à la machine avec date d'expiration (statut
  essai/actif/expiré, sans jamais afficher de prix), vérification de mise à
  jour opportuniste (silencieuse hors ligne), sauvegarde automatique
  régulière de la base SQLite.

### Limites connues (choix assumés faute de pouvoir tester sur un vrai PC Windows)

- **Mobile Hotspot** : pas d'automatisation WinRT (`NetworkOperatorTetheringManager`)
  — l'app ouvre directement la page de réglages Windows correspondante à la
  place. Plus robuste qu'un code impossible à valider ici, mais demande un
  clic de plus au gérant la première fois.
- **Décompte de stock** : approximatif (basé sur le nombre de copies
  déclaré, pas sur une lecture réelle des compteurs de l'imprimante via
  SNMP). Le gérant peut corriger le stock manuellement dans Rapports à tout
  moment. Une intégration SNMP réelle nécessiterait un test sur une
  imprimante réseau physique.
- **Export des rapports** : CSV (ouvrable dans Excel) plutôt que `.xlsx`/`.pdf`
  binaires, pour rester simple et robuste sans dépendance lourde
  supplémentaire.
- **Aperçu avant impression** : intégré dans l'app pour PDF/image (bouton
  *Aperçu*) via le moteur de rendu PDF de WebView2 (le composant Chromium
  embarqué par Tauri sur Windows) — pas un moteur de rendu maison, mais pas
  non plus besoin d'ouvrir Edge/Acrobat séparément. Fichiers volumineux
  (> 15 Mo) : l'aperçu intégré est désactivé (le data URI deviendrait trop
  lourd pour la vue web), le bouton *Imprimer* reste utilisable directement.
  Pour les fichiers Word/Excel/PowerPoint, l'aperçu se fait dans l'éditeur
  natif (bouton *Ouvrir/Éditer*), pas dans l'app.
- **Détection PDF protégé / format papier** : heuristiques par lecture
  directe des octets du fichier (recherche de `/Encrypt`, `/MediaBox`), pas
  un vrai analyseur PDF. Peut manquer certains PDF récents dont les objets
  de page sont dans un flux compressé ("object streams", PDF 1.5+). Jamais
  bloquant : juste un avertissement affiché au gérant.
- **Service de saisie/dactylographie** (section 4 du cahier des charges) :
  volontairement pas implémenté pour l'instant — le gérant tape directement
  dans Word comme il le fait déjà. À revoir selon les retours terrain une
  fois que de vrais gérants auront utilisé le logiciel.
- **Vérification des mises à jour** : nécessite que le porteur du projet
  héberge une URL renvoyant `{"version": "x.y.z"}` (réglage "URL de
  vérification des mises à jour"). Vide par défaut = fonctionnalité
  inactive, aucune erreur affichée.
- Non testé sur un vrai PC Windows (7/8/10/11, antivirus/SmartScreen) —
  seule la compilation via GitHub Actions est vérifiée automatiquement. Voir
  [`docs/guide-installation-terrain.md`](docs/guide-installation-terrain.md)
  pour la marche à suivre face à l'écran SmartScreen.
- **Alerte "carte Wi-Fi incapable de faire point d'accès + internet en même
  temps"** : pas de détection automatique (nécessiterait l'API WinRT de
  tethering, non testable ici) — le message d'erreur du QR explique le cas
  et propose une solution de contournement, mais ne le détecte pas
  précisément.
- **Ouverture automatique de la page après connexion Wi-Fi ("portail
  captif")** : tentative best-effort (serveur qui répond sur le port 80 et
  redirige tout vers la page d'envoi, comme les Wi-Fi d'hôtel/café). Ça
  fonctionne sur une partie des téléphones seulement — la détection de
  portail captif dépend du système et de sa version, et nécessiterait
  aussi de contrôler le DNS du réseau local pour être fiable partout, ce
  que Windows Mobile Hotspot ne permet pas facilement. Si l'ouverture
  automatique ne se déclenche pas, le client n'a qu'à ouvrir son navigateur
  une fois connecté — la connexion Wi-Fi elle-même (la partie qui compte
  le plus) reste, elle, automatique dans tous les cas grâce au QR au format
  `WIFI:`.
- **Secret de licence codé en dur** dans `license.rs` (désormais une valeur
  aléatoire, plus le placeholder par défaut) : le dépôt étant privé, il
  n'est pas exposé publiquement, mais reste extractible du binaire compilé
  par quelqu'un de déterminé. Si le dépôt devient public un jour, le
  déplacer vers un secret injecté à la compilation plutôt que le code
  source.
- **Anti-piratage renforcé mais pas absolu** : la date de début d'essai est
  écrite à la fois dans la base SQLite et le registre Windows (la plus
  ancienne des deux fait foi) — supprimer un seul des deux ne suffit plus à
  relancer l'essai, mais supprimer les deux (ou modifier le registre à la
  main) reste possible pour quelqu'un de déterminé. Pas de solution
  totalement étanche sans vérification serveur, ce que le cahier des
  charges exclut explicitement (logiciel 100% hors ligne).

## Structure

```
src/                interface (HTML/CSS/JS vanilla)
src-tauri/src/
  db.rs             connexion SQLite (WAL) + schéma + réglages
  models.rs         structures partagées (file d'attente, tarifs, etc.)
  watcher.rs        surveillance du dossier de réception (notify)
  usb.rs            détection des clés USB et scan automatique
  files.rs          classification des fichiers + ouverture/impression Windows
  server.rs         serveur HTTP local (page de réception QR)
  qr.rs             génération du QR code affiché à l'écran
  gestion.rs        tarifs, encaissement, stock, dépenses, employés, rapports
  license.rs        licence liée à la machine, essai/abonnement
  updates.rs        vérification opportuniste de mise à jour
  backup.rs         sauvegarde automatique régulière de la base
  commands.rs       commandes Tauri (file d'attente, réglages, QR)
  bin/generer-licence.rs   outil CLI pour le porteur du projet
  lib.rs            point d'entrée, câblage des plugins et de l'état
.github/workflows/build-windows.yml   compilation .exe automatique
```
