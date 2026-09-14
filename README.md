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
https://tauri.app/start/prerequisites/

Vérifications rapides avant de pousser du code Rust :

```bash
cd src-tauri
cargo check
cargo clippy --all-targets --bins
cargo fmt
```

## Générer une clé de licence (porteur du projet)

Le gérant lit son identifiant machine dans Réglages > Licence et vous le
communique. Vous générez ensuite sa clé :

```bash
cd src-tauri
cargo run --release --bin generer-licence -- <ID_MACHINE> [jours_valables=30]
```

Le gérant colle la clé reçue dans Réglages > Licence pour activer ou
renouveler son abonnement. **Important avant toute distribution réelle** :
changez la constante `SECRET` dans `src-tauri/src/license.rs` (mécanisme
léger décrit en section 7 du cahier des charges — pas une vérification
serveur, mais empêche la génération de clés sans connaître ce secret).

## État d'avancement

Toutes les étapes du plan de développement (section 11 du cahier des
charges) ont une première implémentation :

- **Étape 1** — Squelette Tauri + SQLite (WAL) + écran unique, réception par
  dossier surveillé, routage impression/édition, style Office.
- **Étape 2** — Aperçu (vignette pour les images ; pour le reste, l'aperçu
  avant impression est déjà fourni par l'application native qui gère le
  clic *Imprimer* — voir "Limites connues" ci-dessous).
- **Réception clé USB** : détection des disques amovibles, scan automatique.
- **Étape 3** — Serveur HTTP local (page de réception mobile-friendly, bouton
  d'envoi direct en avant, WhatsApp/Bluetooth en liens secondaires) + QR code
  affiché à l'écran (bouton 📶 dans la barre du haut). Le partage de
  connexion Wi-Fi (Mobile Hotspot) s'active via un raccourci vers les
  paramètres Windows plutôt qu'une automatisation WinRT non testable ici.
- **Étape 4** — L'impression elle-même passe *toujours* directement par la
  boîte de dialogue Windows native (bouton *Imprimer*), qui gère déjà
  copies/couleur/recto-verso/format — pas de double saisie dans l'appli.
  L'écran « Détails » ne sert qu'à ce que Windows ne peut pas savoir : les
  finitions (agrafage, reliure...) et ce qu'il faut facturer, pour le
  calcul de prix.
- Diagnostics à la réception d'un fichier : PDF probablement protégé par
  mot de passe, format détecté différent de A4 (ex. US Letter), fichier
  volumineux — affichés comme avertissements, jamais bloquants (cf.
  "Limites connues").
- **Étape 5** — Suivi financier (encaissement espèces/Mobile Money/crédit),
  stock papier/toner avec alerte, dépenses, employés, rapport du jour,
  export CSV des transactions.
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
- **Aperçu avant impression** : pas de moteur de rendu PDF maison — le clic
  *Imprimer* route vers l'application déjà installée sur le PC (Edge, Acrobat,
  Photos…), qui affiche elle-même un aperçu avant impression.
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
  seule la compilation via GitHub Actions est vérifiée automatiquement.

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
