# Gestion Photocopie — Bénin

Logiciel de bureau (Windows, [Tauri](https://tauri.app)) pour les gérants de
boutiques de photocopie au Bénin. 100% fonctionnel hors ligne : réception de
fichiers (dossier surveillé, clé USB, QR/Wi-Fi local), file d'attente unique,
impression/édition via les outils déjà installés sur le PC (dialogue
d'impression Windows natif, Word/LibreOffice).

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
cargo clippy --all-targets
cargo fmt
```

## État d'avancement

Réalisé (étape 1 du plan de développement, section 11 du cahier des
charges) :

- Squelette Tauri + SQLite (mode WAL) + écran unique.
- Réception de fichiers via un **dossier surveillé** (le canal le plus
  simple, sans réseau) : tout fichier déposé apparaît automatiquement dans
  la file d'attente, sans rechargement de page (événement Tauri).
- Routage automatique : PDF/image → bouton **Imprimer** (dialogue
  d'impression Windows natif via `ShellExecuteW`) ; fichier bureautique
  (Word/Excel/PowerPoint/texte) → bouton **Ouvrir/Éditer** (ouvre le
  programme associé par défaut sur le PC du gérant) ; format non reconnu →
  message d'avertissement clair, pas de plantage.
- Menu latéral (⋮) minimal avec le réglage du dossier surveillé.
- Style sobre inspiré de Microsoft Office (police Segoe UI/Calibri,
  couleurs neutres).
- Workflow GitHub Actions qui compile un `.exe` Windows à chaque push.

Pas encore fait (étapes suivantes du plan) :

- Réception clé USB.
- Serveur local + QR code + Wi-Fi local (point d'accès) + WebSocket.
- Options d'impression complètes (copies, N&B/couleur, format, recto-verso)
  et métadonnées de finition (agrafage, reliure, etc.).
- Couche Gestion (suivi financier, stock, rapports, encaissement Mobile
  Money).
- Licence liée au PC, statut d'abonnement (essai/payant/expiré).
- Tests multi-versions Windows, robustesse coupures de courant au-delà du
  mode WAL (sauvegardes automatiques).

## Structure

```
src/               interface (HTML/CSS/JS vanilla)
src-tauri/src/
  db.rs            connexion SQLite (WAL) + schéma + réglages
  watcher.rs       surveillance du dossier de réception (notify)
  files.rs         classification des fichiers + ouverture/impression Windows
  commands.rs       commandes Tauri appelées depuis le frontend
  lib.rs           point d'entrée, câblage des plugins et de l'état
.github/workflows/build-windows.yml   compilation .exe automatique
```
