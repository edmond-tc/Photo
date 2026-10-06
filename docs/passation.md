# Passation — reprendre le projet sans repartir de zéro

Ce document est écrit pour deux lecteurs : le porteur du projet (Edmond),
et la personne ou l'assistant qui reprendra le code plus tard. Il ne
contient AUCUN mot de passe ni clé : seulement leurs noms et où ils vivent.

## 1. Ce que le projet contient

| Morceau | Dossier | Rôle |
| --- | --- | --- |
| Logiciel Windows « Gestion Photocopie » (Tauri : Rust + JavaScript simple) | `src/`, `src-tauri/` | Reçoit les documents, gère la caisse, l'impression, le Wi-Fi du guichet |
| Page client (ouverte dans le navigateur du téléphone) | `client-web/` | Le client envoie son document |
| Application Android « Envoyeur Kiosque » | `android/` | Envoi depuis le téléphone, liaison Bluetooth, « main à main » |
| Tableau de bord web (Cloudflare Worker + base D1 + stockage R2) | `admin/` | Boutiques, licences, codes d'installation, téléchargements |
| Documents | `docs/` | Cahier des charges, guide de terrain, confiance, ce document |

## 2. Adresses en service

- Tableau de bord : <https://photocopie-admin.atinzed2.workers.dev/>
- Logiciel complet : <https://photocopie-admin.atinzed2.workers.dev/telecharger>
- Mise à jour légère : <https://photocopie-admin.atinzed2.workers.dev/telecharger/leger>
- Application Android : <https://photocopie-admin.atinzed2.workers.dev/telecharger/apk>
- Secours : <https://github.com/edmond-tc/Photo/releases/latest>
- Assistance affichée dans le logiciel : WhatsApp 0151226741

## 3. Les comptes à ne jamais perdre

Sans eux, le projet est figé : il tourne, mais plus personne ne peut le mettre à jour.

1. **Compte GitHub** propriétaire du dépôt `edmond-tc/Photo`. Il construit
   tout automatiquement.
2. **Compte Cloudflare** (sous-domaine `atinzed2`). Il héberge le tableau de bord,
   la base et les fichiers à télécharger.
3. **Les trois clés de signature**. Chacune est un secret : si elle est perdue, rien
   ne la remplace.

| Nom du secret | Où il est enregistré | À quoi il sert | Si on le perd |
| --- | --- | --- | --- |
| `CLE_SIGNATURE_MAJ` | GitHub → Settings → Secrets → Actions | Signe les mises à jour du logiciel Windows | Les PC déjà installés refusent les nouvelles mises à jour : il faut réinstaller à la main |
| `ENVOYEUR_KEYSTORE_B64` et `ENVOYEUR_KEYSTORE_MDP` | GitHub → Secrets → Actions | Signe l'application Android | Android refuse de mettre à jour : les téléphones doivent désinstaller puis réinstaller |
| `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ACCOUNT_ID` | GitHub → Secrets → Actions | Permet à GitHub de publier le tableau de bord | On en recrée un sur dash.cloudflare.com (sans gravité) |
| `ADMIN_PASSWORD`, `CLE_PRIVEE_LICENCE` | Cloudflare (Worker `photocopie-admin`) | Mot de passe du tableau de bord ; clé qui signe les licences | Plus de nouvelles licences possibles : tous les clients actuels devraient être réactivés avec une nouvelle clé |

**À faire dès aujourd'hui** : garder une copie hors ligne (papier ou clé USB chiffrée,
pas WhatsApp) des clés ci-dessus et du fichier `.jks` d'Android. Je (l'assistant) ne
connais pas ces valeurs et je ne peux pas les retrouver.

## 4. Publier une nouvelle version (aucun terminal nécessaire côté GitHub)

**Logiciel Windows**
1. Modifier le code, puis changer le numéro de version dans `src-tauri/Cargo.toml`,
   `src-tauri/tauri.conf.json`, `package.json` et `src-tauri/Cargo.lock`
   (les quatre doivent porter le même numéro).
2. Pousser sur la branche `main`. Le workflow **Build Windows** construit les deux
   installateurs, vérifie que le logiciel démarre, les signe, et crée la publication
   `vX.Y.Z`. Durée : environ 20 minutes.
3. Le site `/telecharger` sert ensuite la nouvelle version.

**Application Android** : changer `versionCode` et `versionName` dans
`android/app/build.gradle.kts`, pousser sur `main`. Le workflow **Build Android**
publie l'APK.

**Tableau de bord** : toute modification dans `admin/` est déployée par le workflow
**Deploy Admin**. Une nouvelle migration de base doit être ajoutée À LA MAIN dans la
liste du fichier `.github/workflows/deploy-admin.yml` (sinon elle n'est jamais jouée).

## 5. État honnête du produit (au 6 octobre 2026, logiciel 0.5.36, Android 0.3.14)

**Marche (testé sur le terrain au moins une fois)**
- Réception de documents depuis un téléphone via le Wi-Fi créé par le PC, quand
  la carte Wi-Fi du PC le permet.
- Caisse, impression, suivi de la commande côté client, contrôle des impressions.
- Licence, essai gratuit, codes d'installation.

**Écrit mais jamais vérifié sur un vrai PC / un vrai téléphone**
- Démarrage en arrière-plan avec Windows, icône près de l'horloge, tâche de
  surveillance, bip à l'arrivée d'un fichier (0.5.35 et 0.5.36).
- Écran d'état, bouton « Réparer », rapport par QR WhatsApp.
- Mode routeur externe avec un vrai routeur.
- iPhone avec le portail captif.
- « Main à main » après la dernière correction.
- Liaison Bluetooth : fonctionne mais lente (1 à 2 minutes observées).

**Limites connues**
- Le Wi-Fi créé par le PC dépend de la carte Wi-Fi : beaucoup de PC ne savent pas le faire.
  Solutions : routeur (environ 10 000 à 15 000 FCFA), ou Bluetooth (lent), ou clé USB.
- Plusieurs PC dans un même kiosque : aucune coordination. Chaque PC a son QR et sa
  licence.
- Le canal Bluetooth privé n'est pas chiffré.
- La désinstallation ne retire pas la tâche de surveillance ni la clé de démarrage.

## 6. Pour la personne qui reprend

1. Lire dans cet ordre : `docs/cahier-des-charges.md`, `docs/fonctionnalites-confiance.md`,
   `docs/guide-installation-terrain.md`, puis ce document.
2. Les écrans destinés au gérant ne doivent contenir aucun mot technique. Un test
   automatique (`aucun_mot_de_technicien_dans_les_ecrans_du_gerant`, dans
   `src-tauri/src/etat_poste.rs`) fait échouer la construction si on en ajoute.
3. Ne jamais lancer le logiciel de test avec la surveillance active en CI : la
   variable `PHOTOCOPIE_SANS_PERMANENCE=1` existe pour ça (voir `build-windows.yml`).
4. La logique réseau du PC est dans `hotspot.rs`, `server.rs`, `dns.rs`, `dhcp.rs`,
   `routeur_externe.rs`, `bluetooth.rs`, `canal_bt.rs`.
5. Consigne d'Edmond : le logiciel ne doit **jamais** s'ouvrir tout seul. Il démarre
   en arrière-plan avec Windows ; la fenêtre ne s'ouvre que lorsque le gérant la demande.
6. Consigne d'Edmond : les PC des boutiques n'ont pas Internet. Rien de ce qui est
   construit pour eux ne doit en dépendre.

### Phrase à donner à un assistant IA pour reprendre

> Voici le dépôt `edmond-tc/Photo`. Lis `docs/passation.md` puis les documents
> qu'il cite. Le porteur du projet n'est pas développeur : explique en français
> simple, ne casse pas les règles de la section 6, publie par la branche `main`.
